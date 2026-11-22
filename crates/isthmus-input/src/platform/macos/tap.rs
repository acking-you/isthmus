//! The event-tap thread (tap half). See docs/architecture.md, Threads.
//!
//! One active tap at `kCGHIDEventTap`, head-inserted, runs on its own
//! `isthmus-tap` thread with its own `CFRunLoop`, so a slow UI or HID thread
//! can never make the window server wait on a key. The design decisions:
//!
//! - **One tap, two masks.** Keys, modifiers, the wheel and side buttons share
//!   one tap. When macOS refuses the keyboard events (no Accessibility), the
//!   tap is retried with the wheel and side buttons only, so the mouse half
//!   still works where macOS allows it. Pointer motion is never in the mask:
//!   an active tap gates every event it sees.
//! - **The engines live only on this thread.** [`KeyboardEngine`] and
//!   [`MouseEngine`] sit in a `TapCore` that is leaked into the callback's
//!   `user_info` and freed after the tap is invalidated and the run loop has
//!   stopped. Other threads reach it only by queueing blocks on the run loop
//!   ([`TapThread::retry`], [`TapThread::stop`],
//!   [`TapThread::secure_input_changed`]), which find it through a
//!   thread-local, so the core needs no lock and no `Send`.
//! - **The callback fails open.** Its body runs under `catch_unwind`; a
//!   panic, a busy core or an unreadable event passes the event unchanged.
//!   It reads the snapshot (one `Arc` clone), runs the engine and posts what
//!   the engine asks for: no Objective-C, file or blocking work. Blocking
//!   side effects go to the `isthmus-actions` worker without waiting.
//! - **Disabled taps come back, within limits.** When macOS disables the tap
//!   (a slow callback, or user input), it is re-enabled and every synthetic
//!   key Isthmus holds is released. Re-enables and re-creations share one
//!   budget of three per minute (Mos's rule): past it the tap stays off and
//!   reports `Failed`, because a tap that keeps timing out is what makes
//!   macOS take input back from it. A 5 s health check re-creates a tap
//!   whose port died, but never probes for a tap that never worked: on macOS
//!   27 creating one can prompt, so that waits for [`TapThread::retry`].
//! - **Events Isthmus posts carry [`super::ISTHMUS_TAG`]** and pass straight
//!   through. Inside the callback they are posted through the callback's
//!   proxy, so they enter just before the event being answered; outside it
//!   (the switcher watchdog, releases on disable and stop) they are posted at
//!   the HID location and come back through this tap tagged.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::ptr::{self, NonNull};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use block2::RcBlock;
use objc2_application_services::AXIsProcessTrusted;
use objc2_core_foundation::{
    CFAbsoluteTimeGetCurrent, CFMachPort, CFRetained, CFRunLoop, CFRunLoopMode, CFRunLoopSource,
    CFRunLoopTimer, CFType, kCFRunLoopCommonModes,
};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventMask, CGEventSource, CGEventSourceStateID, CGEventTapLocation,
    CGEventTapOptions, CGEventTapPlacement, CGEventTapProxy, CGEventType,
};
use tracing::{error, info, warn};

use super::actions::{Actions, Submitted};
use super::synth::{self, field};
use super::{TapShared, TapSnapshot, TapStates};
use crate::engine::keyboard::KeyboardEngine;
use crate::engine::mouse::MouseEngine;
use crate::engine::{
    Action, ButtonEvent, ButtonOutcome, Fate, KeyContext, KeyEvent, KeyOutcome, KeyPhase,
    MouseContext, ScrollEvent, ScrollOutcome, Synth,
};
use crate::keys::{Flags, KeyCode, kc};
use crate::settings::MouseProfile;
use crate::status::TapState;

/// The thread's name, as `ps`, Instruments and crash reports show it.
const THREAD_NAME: &str = "isthmus-tap";

/// How often the Alt+Tab watchdog looks at the physical Alt key while a
/// switcher session is open, in seconds.
const TICK_SECONDS: f64 = 0.15;

/// How often the health check looks at the tap, in seconds.
const HEALTH_SECONDS: f64 = 1.0;

/// A fire date offset that means "not scheduled" for a repeating
/// `CFRunLoopTimer` (about 31 years).
const IDLE_SECONDS: f64 = 1.0e9;

/// How long [`TapThread::start`] waits for each step of the first attempt.
/// Creating a tap takes microseconds; the bound only matters if macOS hangs.
const START_TIMEOUT: Duration = Duration::from_secs(2);

/// How long [`TapThread::stop`] waits for the thread to finish.
const STOP_TIMEOUT: Duration = Duration::from_secs(1);

/// What [`TapThread::states`] says before the first attempt has finished:
/// wanted, but not running.
const NOT_STARTED: TapStates = TapStates {
    keyboard: TapState::Failed,
    mouse: TapState::Failed,
};

/// Re-enables and re-creations allowed in [`RESTART_WINDOW_NS`].
const MAX_RESTARTS: usize = 3;

/// The window of the restart budget: one minute.
const RESTART_WINDOW_NS: u64 = 60_000_000_000;

/// `CGEventMaskBit`, which is a C macro with no binding.
const fn bit(ty: CGEventType) -> CGEventMask {
    1 << ty.0
}

/// The wheel and the side buttons: what an active tap may still get without
/// Accessibility's keyboard access.
const MOUSE_MASK: CGEventMask = bit(CGEventType::ScrollWheel)
    | bit(CGEventType::OtherMouseDown)
    | bit(CGEventType::OtherMouseUp);

/// Everything the tap handles. Left and right button downs are here only to
/// tell the keyboard engine that a Win press was not a tap; they always pass.
const FULL_MASK: CGEventMask = MOUSE_MASK
    | bit(CGEventType::KeyDown)
    | bit(CGEventType::KeyUp)
    | bit(CGEventType::FlagsChanged)
    | bit(CGEventType::LeftMouseDown)
    | bit(CGEventType::RightMouseDown);

/// Which events an installed tap receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TapMask {
    /// Keys and mouse: Accessibility is granted.
    Full,
    /// Wheel and side buttons only.
    MouseOnly,
}

impl TapMask {
    /// The event mask for `CGEventTapCreate`.
    const fn bits(self) -> CGEventMask {
        match self {
            TapMask::Full => FULL_MASK,
            TapMask::MouseOnly => MOUSE_MASK,
        }
    }
}

/// The states after an install attempt. A part macOS refused needs
/// Accessibility, unless Accessibility already reads as granted: then the
/// grant has not reached this process (macOS applies it at the next launch)
/// or something else failed, and the UI should say so rather than ask again.
fn installed_states(installed: Option<TapMask>, ax_trusted: bool) -> TapStates {
    let refused = if ax_trusted {
        TapState::Failed
    } else {
        TapState::NeedsPermission
    };
    match installed {
        Some(TapMask::Full) => TapStates {
            keyboard: TapState::Running,
            mouse: TapState::Running,
        },
        Some(TapMask::MouseOnly) => TapStates {
            keyboard: refused,
            mouse: TapState::Running,
        },
        None => TapStates {
            keyboard: refused,
            mouse: refused,
        },
    }
}

/// `current`, with the parts an installed tap of `mask` covers set to
/// `state`. A mouse-only tap going down leaves the keyboard's state alone.
fn with_parts(mask: TapMask, current: TapStates, state: TapState) -> TapStates {
    match mask {
        TapMask::Full => TapStates {
            keyboard: state,
            mouse: state,
        },
        TapMask::MouseOnly => TapStates {
            keyboard: current.keyboard,
            mouse: state,
        },
    }
}

/// The budget shared by re-enables and re-creations: at most
/// [`MAX_RESTARTS`] in any [`RESTART_WINDOW_NS`]. A fixed array, because it
/// is touched from the callback.
#[derive(Debug, Default)]
struct RestartBudget {
    recent: [Option<u64>; MAX_RESTARTS],
}

impl RestartBudget {
    /// Takes one restart at `now_ns` if the window has room.
    fn try_take(&mut self, now_ns: u64) -> bool {
        let expired = |taken: Option<u64>| {
            taken.is_none_or(|taken| now_ns.saturating_sub(taken) >= RESTART_WINDOW_NS)
        };
        match self.recent.iter_mut().find(|slot| expired(**slot)) {
            Some(slot) => {
                *slot = Some(now_ns);
                true
            }
            None => false,
        }
    }
}

/// [`TapStates`] packed into one atomic, so [`TapThread::states`] reads both
/// parts consistently without a lock the callback could wait on.
#[derive(Debug)]
struct StateCell(AtomicU16);

impl StateCell {
    /// A cell holding `states`.
    fn new(states: TapStates) -> StateCell {
        StateCell(AtomicU16::new(pack(states)))
    }

    /// The last stored states.
    fn load(&self) -> TapStates {
        unpack(self.0.load(Ordering::Acquire))
    }

    /// Replaces the states.
    fn store(&self, states: TapStates) {
        self.0.store(pack(states), Ordering::Release);
    }
}

/// One state as a byte.
const fn encode(state: TapState) -> u16 {
    match state {
        TapState::Running => 0,
        TapState::Off => 1,
        TapState::NeedsPermission => 2,
        TapState::Failed => 3,
    }
}

/// The inverse of [`encode`]; anything else reads as `Failed`.
const fn decode(code: u16) -> TapState {
    match code {
        0 => TapState::Running,
        1 => TapState::Off,
        2 => TapState::NeedsPermission,
        _ => TapState::Failed,
    }
}

/// Both states in one word: keyboard high, mouse low.
const fn pack(states: TapStates) -> u16 {
    (encode(states.keyboard) << 8) | encode(states.mouse)
}

/// The inverse of [`pack`].
const fn unpack(word: u16) -> TapStates {
    TapStates {
        keyboard: decode(word >> 8),
        mouse: decode(word & 0xFF),
    }
}

/// What happened to a key, from the event's type.
fn key_phase(ty: CGEventType) -> Option<KeyPhase> {
    match ty {
        CGEventType::KeyDown => Some(KeyPhase::Down),
        CGEventType::KeyUp => Some(KeyPhase::Up),
        CGEventType::FlagsChanged => Some(KeyPhase::FlagsChanged),
        _ => None,
    }
}

/// Buttons 4 and 5 (`kCGMouseEventButtonNumber` 3 and 4): the only other
/// buttons the mouse engine decides. The middle button and beyond pass.
const fn is_side_button(button: i64) -> bool {
    matches!(button, 3 | 4)
}

/// What becomes of the key event the callback holds, before any OS call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disposition {
    /// Returned as it is.
    Original,
    /// Returned with these flags: a modifier change edits the event in
    /// place, which keeps its characters right.
    OriginalWithFlags(Flags),
    /// Replaced by a new key event: a changed keycode needs a new event,
    /// because an event keeps the characters of the key it was made for.
    Replacement { keycode: KeyCode, flags: Flags },
    /// Swallowed.
    Nothing,
}

/// The disposition of a key event with `keycode` whose fate is `fate`.
fn disposition(keycode: KeyCode, fate: Fate) -> Disposition {
    match fate {
        Fate::Pass => Disposition::Original,
        Fate::Rewrite {
            keycode: new,
            flags,
        } if new == keycode => Disposition::OriginalWithFlags(flags),
        Fate::Rewrite {
            keycode: new,
            flags,
        } => Disposition::Replacement {
            keycode: new,
            flags,
        },
        Fate::Drop => Disposition::Nothing,
    }
}

/// One field write on a wheel event.
#[derive(Debug, Clone, Copy, PartialEq)]
enum FieldWrite {
    Int(CGEventField, i64),
    /// The fixed-point deltas, which are 16.16 numbers that CoreGraphics
    /// converts from a double.
    Double(CGEventField, f64),
}

/// The writes that give a wheel event new deltas, in the only order that
/// works: continuous off first, then the line deltas (setting one makes
/// macOS recompute the fixed-point and pixel deltas from it), then the
/// fixed-point and pixel deltas over what macOS computed.
fn scroll_writes(line: [i64; 2], fixed: [f64; 2], point: [i64; 2]) -> [FieldWrite; 7] {
    let [line_1, line_2] = line;
    let [fixed_1, fixed_2] = fixed;
    let [point_1, point_2] = point;
    [
        FieldWrite::Int(field::SCROLL_IS_CONTINUOUS, 0),
        FieldWrite::Int(field::SCROLL_LINE_1, line_1),
        FieldWrite::Int(field::SCROLL_LINE_2, line_2),
        FieldWrite::Double(field::SCROLL_FIXED_1, fixed_1),
        FieldWrite::Double(field::SCROLL_FIXED_2, fixed_2),
        FieldWrite::Int(field::SCROLL_POINT_1, point_1),
        FieldWrite::Int(field::SCROLL_POINT_2, point_2),
    ]
}

/// Applies `writes` to `event`, in order.
fn apply_writes(event: &CGEvent, writes: &[FieldWrite]) {
    for write in writes {
        match *write {
            FieldWrite::Int(written, value) => {
                CGEvent::set_integer_value_field(Some(event), written, value);
            }
            FieldWrite::Double(written, value) => {
                CGEvent::set_double_value_field(Some(event), written, value);
            }
        }
    }
}

/// The registry id of the HID service that sent `event` (field 87), 0 for
/// software. The field is a `uint64` that CoreGraphics hands out as an
/// `int64`; the cast takes the bits back.
fn sender_of(event: &CGEvent) -> u64 {
    CGEvent::integer_value_field(Some(event), field::SENDER_ID) as u64
}

/// Reads a wheel event into the engine's terms.
fn read_scroll(event: &CGEvent) -> ScrollEvent {
    let int = |read| CGEvent::integer_value_field(Some(event), read);
    let double = |read| CGEvent::double_value_field(Some(event), read);
    ScrollEvent {
        continuous: int(field::SCROLL_IS_CONTINUOUS) != 0,
        scroll_phase: int(field::SCROLL_PHASE),
        momentum_phase: int(field::SCROLL_MOMENTUM_PHASE),
        line: [int(field::SCROLL_LINE_1), int(field::SCROLL_LINE_2)],
        fixed: [double(field::SCROLL_FIXED_1), double(field::SCROLL_FIXED_2)],
        point: [int(field::SCROLL_POINT_1), int(field::SCROLL_POINT_2)],
        inverted: int(field::SCROLL_INVERTED) != 0,
        sender: sender_of(event),
        flags: Flags(CGEvent::flags(Some(event)).0),
    }
}

/// The profile of the mouse that sent an event, or the defaults for a
/// sender the snapshot does not know.
fn profile_of(snapshot: &TapSnapshot, sender: u64) -> &MouseProfile {
    snapshot
        .mouse_by_sender
        .get(&sender)
        .unwrap_or(&snapshot.mouse_default)
}

/// Nanoseconds since boot, not counting sleep: the clock for both key
/// events and watchdog ticks. Isthmus reads its own clock rather than the
/// event's timestamp because the unit of `CGEventTimestamp` on Apple silicon
/// is not settled (eventtap.md §3.4), and the engine compares the two.
fn monotonic_ns() -> u64 {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `now` is a valid, writable timespec for the call to fill.
    if unsafe { libc::clock_gettime(libc::CLOCK_UPTIME_RAW, &mut now) } != 0 {
        return 0;
    }
    let seconds = u64::try_from(now.tv_sec).unwrap_or(0);
    let nanos = u64::try_from(now.tv_nsec).unwrap_or(0);
    seconds.saturating_mul(1_000_000_000).saturating_add(nanos)
}

/// Whether either Alt (Option) key is physically down, for the switcher
/// watchdog: it must not depend on seeing the release, which Secure Input or
/// a disabled tap can hide.
fn physical_alt_down() -> bool {
    let state = CGEventSourceStateID::HIDSystemState;
    CGEventSource::key_state(state, kc::OPTION) || CGEventSource::key_state(state, kc::RIGHT_OPTION)
}

/// Whether Accessibility reads as granted. Only a hint (it is cached and can
/// be stale on macOS 27), used to tell "needs permission" from "failed".
fn ax_trusted() -> bool {
    // SAFETY: takes no arguments and only reads this process's trust state.
    unsafe { AXIsProcessTrusted() }
}

/// The run-loop modes the tap's source and timers are added in, so they keep
/// firing while a menu or a modal loop runs.
fn common_modes() -> Option<&'static CFRunLoopMode> {
    // SAFETY: reading an immutable CoreFoundation constant.
    unsafe { kCFRunLoopCommonModes }
}

/// Raises the calling thread to the interactive QoS class, as LinearMouse's
/// event thread does: every key waits on this thread.
fn set_user_interactive_qos() {
    // SAFETY: changes only the calling thread's QoS class; no pointers.
    let status = unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0)
    };
    if status != 0 {
        warn!(status, "could not raise the tap thread's QoS class");
    }
}

/// Runs `job`, logging instead of unwinding if it panics: blocks and timers
/// are called from CoreFoundation, and a panic must not unwind into it.
fn guarded(what: &'static str, job: impl FnOnce()) {
    if panic::catch_unwind(AssertUnwindSafe(job)).is_err() {
        error!("{what} panicked; the tap thread carries on");
    }
}

/// The tap thread's run loop, as other threads hold it: only to queue blocks
/// on it and wake it.
struct RemoteRunLoop(CFRetained<CFRunLoop>);

// SAFETY: through this wrapper only `CFRunLoopPerformBlock` and
// `CFRunLoopWakeUp` are called, which CoreFoundation documents as safe from
// any thread, and CF reference counting is atomic.
unsafe impl Send for RemoteRunLoop {}
// SAFETY: as above; no method takes `&mut self`.
unsafe impl Sync for RemoteRunLoop {}

impl RemoteRunLoop {
    /// Queues `job` to run on the tap thread and wakes the thread.
    fn perform(&self, job: impl Fn() + Send + 'static) {
        let block = RcBlock::new(move || guarded("a tap command", &job));
        let mode: Option<&CFType> = common_modes().map(|mode| &**mode);
        // SAFETY: `CFRunLoopPerformBlock` may be called from any thread; the
        // mode is the common-modes constant; CF copies the block, which
        // captures only `Send` data.
        unsafe { self.0.perform_block(mode, Some(&block)) };
        self.0.wake_up();
    }
}

/// What the tap thread tells [`TapThread::start`].
enum Ready {
    /// The run loop exists; commands can be queued.
    RunLoop(RemoteRunLoop),
    /// The first install attempt has finished.
    Attempted,
}

/// The thread itself, for [`TapThread::stop`].
struct Worker {
    join: JoinHandle<()>,
    /// Signalled when the thread is about to exit, so the join is bounded.
    done: Receiver<()>,
}

/// The running tap thread. Dropping it without [`TapThread::stop`] leaks
/// nothing but leaves the thread running until the process exits.
pub(crate) struct TapThread {
    run_loop: Option<RemoteRunLoop>,
    states: Arc<StateCell>,
    worker: Option<Worker>,
}

impl TapThread {
    /// Starts the `isthmus-tap` thread and tries to install the tap: the
    /// keyboard-and-mouse mask first, the mouse-only mask when macOS refuses
    /// keyboard events (no Accessibility). Returns once the first attempt
    /// has finished, so [`TapThread::states`] is meaningful at once.
    pub fn start(shared: Arc<TapShared>) -> TapThread {
        let states = Arc::new(StateCell::new(NOT_STARTED));
        let (ready, ready_inbox) = mpsc::sync_channel(2);
        let (done, done_inbox) = mpsc::sync_channel(1);
        let thread_states = Arc::clone(&states);
        let spawned = thread::Builder::new()
            .name(THREAD_NAME.to_owned())
            .spawn(move || thread_main(shared, thread_states, &ready, &done));
        let join = match spawned {
            Ok(join) => join,
            Err(source) => {
                error!(error = %source, "could not start the tap thread");
                return TapThread {
                    run_loop: None,
                    states,
                    worker: None,
                };
            }
        };
        let run_loop = match ready_inbox.recv_timeout(START_TIMEOUT) {
            Ok(Ready::RunLoop(run_loop)) => Some(run_loop),
            Ok(Ready::Attempted) | Err(_) => {
                error!("the tap thread did not come up");
                None
            }
        };
        if run_loop.is_some() {
            match ready_inbox.recv_timeout(START_TIMEOUT) {
                Ok(Ready::Attempted) => {}
                Ok(Ready::RunLoop(_)) | Err(_) => {
                    warn!("the first tap attempt is still running; states follow later");
                }
            }
        }
        TapThread {
            run_loop,
            states,
            worker: Some(Worker {
                join,
                done: done_inbox,
            }),
        }
    }

    /// The two parts' states as the tap thread last recorded them. Never
    /// `Off`: whether a part is wanted is the service's call, from the
    /// settings. Lock-free, so any thread may ask at any time.
    pub fn states(&self) -> TapStates {
        self.states.load()
    }

    /// Tries again to install whatever is missing (after Accessibility was
    /// granted). Cheap when nothing is missing.
    pub fn retry(&self) {
        if let Some(run_loop) = &self.run_loop {
            run_loop.perform(|| {
                with_core(TapCore::retry);
            });
        }
    }

    /// Releases whatever synthetic keys are held, disables and invalidates
    /// the tap, stops the run loop and joins the thread. Bounded.
    pub fn stop(self) {
        let TapThread {
            run_loop,
            states: _,
            worker,
        } = self;
        if let Some(run_loop) = &run_loop {
            run_loop.perform(|| {
                with_core(TapCore::shutdown);
            });
        }
        let Some(Worker { join, done }) = worker else {
            return;
        };
        match done.recv_timeout(STOP_TIMEOUT) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                if join.join().is_err() {
                    error!("the tap thread panicked");
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                warn!("the tap thread did not stop in time; leaving it to the process exit");
            }
        }
    }

    /// Secure Event Input started or ended: release synthetic keys and end
    /// any switcher session, since key events stop arriving meanwhile.
    pub fn secure_input_changed(&self, on: bool) {
        if let Some(run_loop) = &self.run_loop {
            run_loop.perform(move || {
                with_core(|core| core.secure_input_changed(on));
            });
        }
    }
}

/// The tap core as the callback, timers and blocks reach it.
type CoreCell = RefCell<TapCore>;

thread_local! {
    /// The tap thread's core, for timers and queued blocks; null elsewhere
    /// and after the core is freed.
    static CORE: Cell<*const CoreCell> = const { Cell::new(ptr::null()) };
}

/// Runs `job` on this thread's core. `None` off the tap thread, after
/// shutdown, or if the core is already borrowed (which a single-threaded run
/// loop never does, so failing quietly is safe).
fn with_core<R>(job: impl FnOnce(&mut TapCore) -> R) -> Option<R> {
    let core = CORE.with(Cell::get);
    // SAFETY: `CORE` is null or the core `thread_main` leaked on this very
    // thread, and it is reset to null before that core is freed.
    let cell = unsafe { core.as_ref() }?;
    let mut core = cell.try_borrow_mut().ok()?;
    Some(job(&mut core))
}

/// The tap thread's body: owns the core from creation to free.
fn thread_main(
    shared: Arc<TapShared>,
    states: Arc<StateCell>,
    ready: &SyncSender<Ready>,
    done: &SyncSender<()>,
) {
    set_user_interactive_qos();
    let Some(run_loop) = CFRunLoop::current() else {
        error!("the tap thread has no run loop");
        return;
    };
    let actions = match Actions::start() {
        Ok(actions) => Some(actions),
        Err(source) => {
            error!(error = %source, "no actions worker: Win+E and app launches do nothing");
            None
        }
    };
    let core = Box::into_raw(Box::new(RefCell::new(TapCore::new(
        shared,
        states,
        run_loop.clone(),
        actions,
    ))));
    CORE.with(|slot| slot.set(core));
    with_core(|this| {
        this.user_info = core.cast();
        this.add_timers();
    });
    if ready.send(Ready::RunLoop(RemoteRunLoop(run_loop))).is_err() {
        // `TapThread::start` gave up waiting and dropped its end, so nobody
        // holds this run loop and nothing could ever stop a tap installed
        // now. Leave without installing one.
        warn!("the tap thread came up after its starter gave up; not installing a tap");
        with_core(TapCore::dismantle);
        CORE.with(|slot| slot.set(ptr::null()));
        // SAFETY: no tap was installed and the run loop never ran, so no
        // callback, timer or block can reach the core; `CORE` is reset and
        // this is the only owner left.
        drop(unsafe { Box::from_raw(core) });
        let _ = done.send(());
        return;
    }
    with_core(TapCore::install_first);
    // Tell the caller before `on_state`, which may wait on a lock the caller
    // holds while it waits here.
    let _ = ready.send(Ready::Attempted);
    with_core(TapCore::announce);
    // Returns when `TapCore::shutdown` stops the loop: the health timer is
    // always installed, so the loop never runs dry.
    CFRunLoop::run();
    with_core(TapCore::dismantle);
    CORE.with(|slot| slot.set(ptr::null()));
    // SAFETY: the tap is invalidated and the run loop has stopped, so no
    // callback, timer or block can reach the core any more; `CORE` is reset
    // and this is the only owner left.
    drop(unsafe { Box::from_raw(core) });
    let _ = done.send(());
}

/// The callback's proxy, valid only while that callback runs.
#[derive(Clone, Copy)]
struct Proxy(CGEventTapProxy);

impl Proxy {
    /// Wraps the proxy the window server passed to the running callback.
    ///
    /// # Safety
    ///
    /// `proxy` must be the one passed to the callback that is running now,
    /// and the value must not be kept past that callback's return.
    unsafe fn new(proxy: CGEventTapProxy) -> Proxy {
        Proxy(proxy)
    }

    /// Posts `event` so it enters just before the event the callback
    /// returns; this tap does not see it again.
    fn post(self, event: &CGEvent) {
        // SAFETY: by `Proxy::new`'s contract the proxy belongs to the
        // callback that is still running.
        unsafe { CGEvent::tap_post_event(self.0, Some(event)) };
    }
}

/// What the callback answers for an event.
enum Answer {
    Pass,
    Drop,
    /// A new event, handed to the window server, which releases it.
    Replace(CFRetained<CGEvent>),
}

/// Set once a panic in the callback has been logged.
static CALLBACK_PANICKED: AtomicBool = AtomicBool::new(false);

/// The tap's callback.
///
/// # Safety
///
/// `user_info` must be the `CoreCell` that `thread_main` leaked for this
/// thread, and the callback must run on that thread's run loop: the tap's
/// source is only ever added there, and the core outlives the tap.
unsafe extern "C-unwind" fn tap_callback(
    proxy: CGEventTapProxy,
    ty: CGEventType,
    event: NonNull<CGEvent>,
    user_info: *mut c_void,
) -> *mut CGEvent {
    let answered = panic::catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the caller's contract; the cell is alive and only this
        // thread touches it.
        let Some(cell) = (unsafe { user_info.cast::<CoreCell>().as_ref() }) else {
            return Answer::Pass;
        };
        let Ok(mut core) = cell.try_borrow_mut() else {
            return Answer::Pass;
        };
        // SAFETY: this is the proxy of the running callback, and `Proxy` is
        // not stored past it.
        let proxy = unsafe { Proxy::new(proxy) };
        core.handle(proxy, ty, event)
    }));
    match answered {
        Ok(Answer::Pass) => event.as_ptr(),
        Ok(Answer::Drop) => ptr::null_mut(),
        Ok(Answer::Replace(replacement)) => CFRetained::into_raw(replacement).as_ptr(),
        Err(_) => {
            if !CALLBACK_PANICKED.swap(true, Ordering::Relaxed) {
                error!("the event tap callback panicked; events pass unchanged");
            }
            event.as_ptr()
        }
    }
}

/// An installed tap.
struct Installed {
    port: CFRetained<CFMachPort>,
    source: CFRetained<CFRunLoopSource>,
    mask: TapMask,
}

impl Installed {
    /// Whether the window server still delivers to it.
    fn is_healthy(&self) -> bool {
        self.port.is_valid() && CGEvent::tap_is_enabled(&self.port)
    }
}

/// Everything the tap thread owns. Lives only on the tap thread.
struct TapCore {
    shared: Arc<TapShared>,
    /// Where [`TapThread::states`] reads the states.
    state_cell: Arc<StateCell>,
    /// The states last reported.
    states: TapStates,
    keyboard: KeyboardEngine,
    mouse: MouseEngine,
    run_loop: CFRetained<CFRunLoop>,
    /// This core's own address, which every tap is created with.
    user_info: *mut c_void,
    installed: Option<Installed>,
    /// The mask that last installed, so the health check re-creates a tap
    /// that died without probing for one that never worked.
    last_mask: Option<TapMask>,
    restarts: RestartBudget,
    /// The switcher watchdog: idle until the keyboard engine needs ticks.
    tick_timer: Option<CFRetained<CFRunLoopTimer>>,
    tick_armed: bool,
    health_timer: Option<CFRetained<CFRunLoopTimer>>,
    /// The source synthetic keys are created from.
    source: Option<CFRetained<CGEventSource>>,
    actions: Option<Actions>,
    /// Whether the last key event found the keyboard section on, so held
    /// synthetic keys are released when it is switched off.
    keyboard_on: bool,
    /// Set by [`TapCore::dismantle`]; nothing is installed after it.
    stopped: bool,
}

impl TapCore {
    /// A core with nothing installed yet.
    fn new(
        shared: Arc<TapShared>,
        state_cell: Arc<StateCell>,
        run_loop: CFRetained<CFRunLoop>,
        actions: Option<Actions>,
    ) -> TapCore {
        TapCore {
            shared,
            state_cell,
            states: NOT_STARTED,
            keyboard: KeyboardEngine::new(),
            mouse: MouseEngine::new(),
            run_loop,
            user_info: ptr::null_mut(),
            installed: None,
            last_mask: None,
            restarts: RestartBudget::default(),
            tick_timer: None,
            tick_armed: false,
            health_timer: None,
            source: synth::event_source(),
            actions,
            keyboard_on: false,
            stopped: false,
        }
    }

    /// Creates the watchdog (idle) and the health timer on this run loop.
    fn add_timers(&mut self) {
        let now = CFAbsoluteTimeGetCurrent();
        let tick = RcBlock::new(|_: *mut CFRunLoopTimer| {
            guarded("the switcher watchdog", || {
                with_core(TapCore::on_tick);
            });
        });
        let health = RcBlock::new(|_: *mut CFRunLoopTimer| {
            guarded("the tap health check", || {
                with_core(TapCore::on_health);
            });
        });
        // SAFETY: the default allocator; CF copies each valid block.
        self.tick_timer = unsafe {
            CFRunLoopTimer::with_handler(None, now + IDLE_SECONDS, TICK_SECONDS, 0, 0, Some(&tick))
        };
        // SAFETY: as above.
        self.health_timer = unsafe {
            CFRunLoopTimer::with_handler(
                None,
                now + HEALTH_SECONDS,
                HEALTH_SECONDS,
                0,
                0,
                Some(&health),
            )
        };
        if self.health_timer.is_none() {
            error!("could not create the tap health timer");
        }
        for timer in [&self.tick_timer, &self.health_timer].into_iter().flatten() {
            self.run_loop.add_timer(Some(timer), common_modes());
        }
    }

    /// The first attempt: stores the states without telling `on_state`,
    /// which [`TapCore::announce`] does once the caller has been released.
    fn install_first(&mut self) {
        let states = self.install(TapMask::Full);
        info!(keyboard = ?states.keyboard, mouse = ?states.mouse, "event tap installed");
        self.states = states;
        self.state_cell.store(states);
    }

    /// Tells the service the first attempt's states.
    fn announce(&mut self) {
        (self.shared.on_state)(self.states);
    }

    /// Records `states` and tells the service, if they changed.
    fn report(&mut self, states: TapStates) {
        if states == self.states {
            return;
        }
        info!(keyboard = ?states.keyboard, mouse = ?states.mouse, "event tap state changed");
        self.states = states;
        self.state_cell.store(states);
        (self.shared.on_state)(states);
    }

    /// Replaces whatever is installed with the widest tap macOS allows, up
    /// to `ceiling`, and returns the resulting states.
    fn install(&mut self, ceiling: TapMask) -> TapStates {
        self.detach();
        let candidates: &[TapMask] = match ceiling {
            TapMask::Full => &[TapMask::Full, TapMask::MouseOnly],
            TapMask::MouseOnly => &[TapMask::MouseOnly],
        };
        let installed = candidates.iter().find_map(|&mask| self.attach(mask));
        let mask = installed.as_ref().map(|installed| installed.mask);
        self.installed = installed;
        if mask.is_some() {
            self.last_mask = mask;
        }
        let ax = mask != Some(TapMask::Full) && ax_trusted();
        installed_states(mask, ax)
    }

    /// Creates a tap with `mask`, adds it to this run loop and enables it.
    fn attach(&self, mask: TapMask) -> Option<Installed> {
        // SAFETY: `tap_callback` has the `CGEventTapCallBack` signature and
        // accepts every event type; `user_info` is this thread's leaked
        // core, which outlives every tap (`dismantle` invalidates them all
        // before the core is freed).
        let port = unsafe {
            CGEvent::tap_create(
                CGEventTapLocation::HIDEventTap,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::Default,
                mask.bits(),
                Some(tap_callback),
                self.user_info,
            )
        }?;
        let Some(source) = CFMachPort::new_run_loop_source(None, Some(&port), 0) else {
            port.invalidate();
            return None;
        };
        self.run_loop.add_source(Some(&source), common_modes());
        CGEvent::tap_enable(&port, true);
        Some(Installed { port, source, mask })
    }

    /// Disables, removes and invalidates the installed tap. Invalidation is
    /// what makes the window server forget it: CoreGraphics keeps its own
    /// reference, and a forgotten zombie tap slows input system-wide.
    fn detach(&mut self) {
        let Some(Installed {
            port,
            source,
            mask: _,
        }) = self.installed.take()
        else {
            return;
        };
        CGEvent::tap_enable(&port, false);
        self.run_loop.remove_source(Some(&source), common_modes());
        port.invalidate();
    }

    /// Decides one event.
    fn handle(&mut self, proxy: Proxy, ty: CGEventType, event: NonNull<CGEvent>) -> Answer {
        if ty == CGEventType::TapDisabledByTimeout || ty == CGEventType::TapDisabledByUserInput {
            self.on_disabled(ty);
            return Answer::Pass;
        }
        // SAFETY: the window server passes a valid event that lives for the
        // whole callback.
        let event = unsafe { event.as_ref() };
        if synth::is_tagged(event) {
            return Answer::Pass;
        }
        let snapshot = self.shared.snapshot();
        match ty {
            CGEventType::KeyDown | CGEventType::KeyUp | CGEventType::FlagsChanged => {
                self.on_key(proxy, ty, event, &snapshot)
            }
            CGEventType::ScrollWheel => self.on_scroll(proxy, event, &snapshot),
            CGEventType::OtherMouseDown | CGEventType::OtherMouseUp => {
                self.on_button(ty, event, &snapshot)
            }
            CGEventType::LeftMouseDown | CGEventType::RightMouseDown => {
                self.keyboard.note_pointer_activity();
                Answer::Pass
            }
            _ => Answer::Pass,
        }
    }

    /// A key, modifier or flags event.
    fn on_key(
        &mut self,
        proxy: Proxy,
        ty: CGEventType,
        event: &CGEvent,
        snapshot: &TapSnapshot,
    ) -> Answer {
        if !snapshot.keyboard {
            self.keyboard_switched_off();
            return Answer::Pass;
        }
        self.keyboard_on = true;
        let keycode = CGEvent::integer_value_field(Some(event), field::KEYCODE);
        let (Some(phase), Ok(keycode)) = (key_phase(ty), KeyCode::try_from(keycode)) else {
            return Answer::Pass;
        };
        let key = KeyEvent {
            phase,
            keycode,
            flags: Flags(CGEvent::flags(Some(event)).0),
            autorepeat: CGEvent::integer_value_field(Some(event), field::KEY_AUTOREPEAT) != 0,
            timestamp_ns: monotonic_ns(),
        };
        let cx = KeyContext {
            app: snapshot.app,
            hid_profile: snapshot.hid_profile,
            groups: &snapshot.groups,
            hotkeys: &snapshot.hotkeys,
        };
        let outcome = self.keyboard.handle(&key, &cx);
        let answer = if outcome.is_pass() {
            Answer::Pass
        } else {
            self.apply_key(proxy, ty, event, keycode, outcome)
        };
        self.sync_tick();
        answer
    }

    /// Carries out a key outcome: `before`, the event's fate, `after`, then
    /// the action. With `after` present the event cannot simply be returned
    /// (it would enter after the posted events), so it is posted in its
    /// place, followed by `after`, and the callback returns nothing.
    fn apply_key(
        &mut self,
        proxy: Proxy,
        ty: CGEventType,
        event: &CGEvent,
        keycode: KeyCode,
        outcome: KeyOutcome,
    ) -> Answer {
        let KeyOutcome {
            before,
            fate,
            after,
            action,
        } = outcome;
        let timestamp = CGEvent::timestamp(Some(event));
        self.post_in_callback(proxy, &before, timestamp);
        let answer = match disposition(keycode, fate) {
            Disposition::Original => Answer::Pass,
            Disposition::OriginalWithFlags(flags) => {
                CGEvent::set_flags(Some(event), synth::cg_flags(flags));
                Answer::Pass
            }
            Disposition::Replacement { keycode, flags } => {
                match synth::replacement(self.source.as_deref(), event, ty, keycode, flags) {
                    Some(replacement) => Answer::Replace(replacement),
                    None => {
                        warn!(keycode, "could not create a replacement key event; passing");
                        Answer::Pass
                    }
                }
            }
            Disposition::Nothing => Answer::Drop,
        };
        let answer = if after.is_empty() {
            answer
        } else {
            match answer {
                Answer::Pass => {
                    if let Some(copy) = CGEvent::new_copy(Some(event)) {
                        synth::tag(&copy);
                        proxy.post(&copy);
                    }
                }
                Answer::Replace(replacement) => proxy.post(&replacement),
                Answer::Drop => {}
            }
            self.post_in_callback(proxy, &after, timestamp);
            Answer::Drop
        };
        if let Some(action) = action {
            self.submit(action);
        }
        answer
    }

    /// A wheel event: the mouse engine's answer, applied in place.
    fn on_scroll(&mut self, proxy: Proxy, event: &CGEvent, snapshot: &TapSnapshot) -> Answer {
        self.keyboard.note_pointer_activity();
        if !snapshot.mouse {
            return Answer::Pass;
        }
        let scroll = read_scroll(event);
        let cx = MouseContext {
            profile: profile_of(snapshot, scroll.sender),
            app: snapshot.app,
            hid_profile: snapshot.hid_profile,
        };
        match self.mouse.scroll(&scroll, &cx) {
            ScrollOutcome::Pass => Answer::Pass,
            ScrollOutcome::Rewrite { line, fixed, point } => {
                apply_writes(event, &scroll_writes(line, fixed, point));
                Answer::Pass
            }
            ScrollOutcome::Keys(keys) => {
                self.post_in_callback(proxy, &keys, CGEvent::timestamp(Some(event)));
                Answer::Drop
            }
        }
    }

    /// A button 3-and-up event: buttons 4 and 5 go to the mouse engine.
    fn on_button(&mut self, ty: CGEventType, event: &CGEvent, snapshot: &TapSnapshot) -> Answer {
        let down = ty == CGEventType::OtherMouseDown;
        if down {
            self.keyboard.note_pointer_activity();
        }
        if !snapshot.mouse {
            return Answer::Pass;
        }
        let button = CGEvent::integer_value_field(Some(event), field::MOUSE_BUTTON_NUMBER);
        if !is_side_button(button) {
            return Answer::Pass;
        }
        let pressed = ButtonEvent {
            button,
            down,
            flags: Flags(CGEvent::flags(Some(event)).0),
        };
        let cx = MouseContext {
            profile: profile_of(snapshot, sender_of(event)),
            app: snapshot.app,
            hid_profile: snapshot.hid_profile,
        };
        match self.mouse.button(&pressed, &cx) {
            ButtonOutcome::Pass => Answer::Pass,
            ButtonOutcome::Drop => Answer::Drop,
            ButtonOutcome::Swipe { back } => {
                if !synth::post_navigation_swipe(back) {
                    warn!(back, "could not create a navigation swipe");
                }
                Answer::Drop
            }
        }
    }

    /// macOS disabled the tap. Everything Isthmus holds is released (the
    /// releases of held keys may have been lost), then the tap is turned
    /// back on while the restart budget lasts.
    fn on_disabled(&mut self, ty: CGEventType) {
        let cause = if ty == CGEventType::TapDisabledByTimeout {
            "timeout"
        } else {
            "user input"
        };
        self.release_held();
        let Some(installed) = &self.installed else {
            return;
        };
        let mask = installed.mask;
        if self.restarts.try_take(monotonic_ns()) {
            CGEvent::tap_enable(&installed.port, true);
            warn!(cause, "macOS disabled the event tap; re-enabled it");
            let states = with_parts(mask, self.states, TapState::Running);
            self.report(states);
        } else {
            warn!(
                cause,
                "macOS keeps disabling the event tap; leaving it off for now"
            );
            let states = with_parts(mask, self.states, TapState::Failed);
            self.report(states);
        }
    }

    /// The switcher watchdog's tick.
    fn on_tick(&mut self) {
        if self.stopped {
            return;
        }
        if !self.keyboard.needs_ticks() {
            self.disarm_tick();
            return;
        }
        // Switched off mid-session: no key may arrive to notice, so the
        // watchdog releases what the session holds.
        if !self.shared.snapshot().keyboard {
            self.keyboard_switched_off();
            return;
        }
        let releases = self.keyboard.tick(monotonic_ns(), physical_alt_down());
        self.post_outside(&releases);
        if !self.keyboard.needs_ticks() {
            self.disarm_tick();
        }
    }

    /// The health check: re-creates a tap whose port died or that stays
    /// disabled, within the restart budget.
    fn on_health(&mut self) {
        if self.stopped {
            return;
        }
        let ceiling = match &self.installed {
            Some(installed) if installed.is_healthy() => return,
            Some(installed) => installed.mask,
            // Nothing installed: try again once the AX trust cache says
            // Accessibility is granted, which happens the moment the user
            // turns Isthmus on in System Settings. Never without that check,
            // since creating a tap can raise the system prompt on macOS 27.
            None if ax_trusted() => self.last_mask.unwrap_or(TapMask::Full),
            None => return,
        };
        if !self.restarts.try_take(monotonic_ns()) {
            if let Some(installed) = &self.installed {
                let states = with_parts(installed.mask, self.states, TapState::Failed);
                self.report(states);
            }
            return;
        }
        warn!("the event tap stopped working; re-creating it");
        self.release_held();
        let states = self.install(ceiling);
        self.report(states);
    }

    /// Installs whatever is missing. A healthy full tap is left alone.
    fn retry(&mut self) {
        if self.stopped {
            return;
        }
        let complete = self
            .installed
            .as_ref()
            .is_some_and(|installed| installed.mask == TapMask::Full && installed.is_healthy());
        if complete {
            return;
        }
        self.release_held();
        let states = self.install(TapMask::Full);
        self.report(states);
    }

    /// Secure Input toggled: key events stop (or start) arriving, so
    /// whatever was held is released and the switcher session ends.
    fn secure_input_changed(&mut self, on: bool) {
        info!(on, "Secure Input changed; releasing synthetic keys");
        self.release_held();
    }

    /// The keyboard section was switched off: release what it held.
    fn keyboard_switched_off(&mut self) {
        if self.keyboard_on {
            self.keyboard_on = false;
            self.release_held();
        }
    }

    /// Stops everything and then the run loop.
    fn shutdown(&mut self) {
        self.dismantle();
        self.run_loop.stop();
    }

    /// Releases held keys, removes the tap and the timers. Idempotent.
    fn dismantle(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        self.release_held();
        self.detach();
        for timer in [self.tick_timer.take(), self.health_timer.take()]
            .into_iter()
            .flatten()
        {
            timer.invalidate();
        }
    }

    /// Ends every engine session and posts the releases.
    fn release_held(&mut self) {
        let releases = self.keyboard.release_all();
        self.post_outside(&releases);
        self.disarm_tick();
    }

    /// Posts synthetic keys through the running callback's proxy, stamped
    /// with the triggering event's time.
    fn post_in_callback(&self, proxy: Proxy, synths: &[Synth], timestamp: u64) {
        for &synth in synths {
            match synth::key_event(self.source.as_deref(), synth, Some(timestamp)) {
                Some(event) => proxy.post(&event),
                None => warn!(?synth, "could not create a synthetic key event"),
            }
        }
    }

    /// Posts synthetic keys from outside a callback, at the HID location,
    /// where the tag lets them through this tap.
    fn post_outside(&self, synths: &[Synth]) {
        for &synth in synths {
            match synth::key_event(self.source.as_deref(), synth, None) {
                Some(event) => CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event)),
                None => warn!(?synth, "could not create a synthetic key event"),
            }
        }
    }

    /// Hands an action to the worker without waiting.
    fn submit(&self, action: Action) {
        let Some(actions) = &self.actions else {
            warn!(?action, "no actions worker; dropped");
            return;
        };
        match actions.submit(action) {
            Submitted::Queued => {}
            Submitted::Full => warn!(?action, "the actions queue is full; dropped"),
            Submitted::Closed => warn!(?action, "the actions worker is gone; dropped"),
        }
    }

    /// Starts the watchdog when the engine opens a session.
    fn sync_tick(&mut self) {
        if self.tick_armed || !self.keyboard.needs_ticks() {
            return;
        }
        if let Some(timer) = &self.tick_timer {
            timer.set_next_fire_date(CFAbsoluteTimeGetCurrent() + TICK_SECONDS);
            self.tick_armed = true;
        }
    }

    /// Parks the watchdog until the next session.
    fn disarm_tick(&mut self) {
        if !self.tick_armed {
            return;
        }
        if let Some(timer) = &self.tick_timer {
            timer.set_next_fire_date(CFAbsoluteTimeGetCurrent() + IDLE_SECONDS);
        }
        self.tick_armed = false;
    }
}

#[cfg(test)]
mod tests {
    use objc2_core_graphics::CGScrollEventUnit;

    use super::*;

    /// The active tap never gates pointer motion, and the mouse-only mask is
    /// exactly the wheel and the side buttons.
    #[test]
    fn masks_leave_out_pointer_motion_and_split_keyboard_from_mouse() {
        let moved = bit(CGEventType::MouseMoved);
        assert_eq!(FULL_MASK & moved, 0);
        assert_eq!(MOUSE_MASK & moved, 0);
        assert_eq!(MOUSE_MASK, (1 << 22) | (1 << 25) | (1 << 26));
        let keyboard = bit(CGEventType::KeyDown) | bit(CGEventType::KeyUp);
        assert_eq!(MOUSE_MASK & keyboard, 0);
        assert_eq!(FULL_MASK & keyboard, keyboard);
        assert_eq!(FULL_MASK & MOUSE_MASK, MOUSE_MASK);
        assert_eq!(
            FULL_MASK & bit(CGEventType::LeftMouseUp),
            0,
            "only button downs matter to the Win tap"
        );
    }

    /// A full tap runs both parts; a mouse-only tap leaves the keyboard
    /// needing Accessibility, or failed when Accessibility already reads as
    /// granted (the grant has not reached this process).
    #[test]
    fn install_results_map_to_states() {
        let running = TapState::Running;
        assert_eq!(
            installed_states(Some(TapMask::Full), false),
            TapStates {
                keyboard: running,
                mouse: running
            }
        );
        assert_eq!(
            installed_states(Some(TapMask::MouseOnly), false),
            TapStates {
                keyboard: TapState::NeedsPermission,
                mouse: running
            }
        );
        assert_eq!(
            installed_states(Some(TapMask::MouseOnly), true),
            TapStates {
                keyboard: TapState::Failed,
                mouse: running
            }
        );
        assert_eq!(
            installed_states(None, false),
            TapStates {
                keyboard: TapState::NeedsPermission,
                mouse: TapState::NeedsPermission
            }
        );
        assert_eq!(
            installed_states(None, true),
            TapStates {
                keyboard: TapState::Failed,
                mouse: TapState::Failed
            }
        );
    }

    /// Giving up on a mouse-only tap fails the mouse and leaves the
    /// keyboard's own state; giving up on a full tap fails both.
    #[test]
    fn giving_up_fails_only_the_parts_the_tap_covers() {
        let current = TapStates {
            keyboard: TapState::NeedsPermission,
            mouse: TapState::Running,
        };
        assert_eq!(
            with_parts(TapMask::MouseOnly, current, TapState::Failed),
            TapStates {
                keyboard: TapState::NeedsPermission,
                mouse: TapState::Failed
            }
        );
        assert_eq!(
            with_parts(TapMask::Full, current, TapState::Failed),
            TapStates {
                keyboard: TapState::Failed,
                mouse: TapState::Failed
            }
        );
    }

    /// Three restarts a minute, then none until the oldest is a minute old.
    #[test]
    fn restarts_are_limited_to_three_a_minute() {
        let second = 1_000_000_000;
        let mut budget = RestartBudget::default();
        assert!(budget.try_take(10 * second));
        assert!(budget.try_take(11 * second));
        assert!(budget.try_take(12 * second));
        assert!(!budget.try_take(13 * second));
        assert!(
            !budget.try_take(69 * second),
            "the oldest is not a minute old"
        );
        assert!(budget.try_take(70 * second), "the oldest has expired");
        assert!(!budget.try_take(70 * second));
    }

    /// Both states round-trip through the atomic word, every value.
    #[test]
    fn states_round_trip_through_the_cell() {
        let all = [
            TapState::Running,
            TapState::Off,
            TapState::NeedsPermission,
            TapState::Failed,
        ];
        let cell = StateCell::new(NOT_STARTED);
        for keyboard in all {
            for mouse in all {
                let states = TapStates { keyboard, mouse };
                cell.store(states);
                assert_eq!(cell.load(), states);
            }
        }
    }

    /// A rewrite that keeps the keycode edits the event in place; one that
    /// changes it needs a new event; pass and drop map straight through.
    #[test]
    fn rewrites_edit_in_place_only_when_the_keycode_stays() {
        let flags = Flags::COMMAND | Flags::DEV_LCMD;
        assert_eq!(
            disposition(
                kc::C,
                Fate::Rewrite {
                    keycode: kc::C,
                    flags
                }
            ),
            Disposition::OriginalWithFlags(flags)
        );
        assert_eq!(
            disposition(
                kc::HOME,
                Fate::Rewrite {
                    keycode: kc::LEFT,
                    flags
                }
            ),
            Disposition::Replacement {
                keycode: kc::LEFT,
                flags
            }
        );
        assert_eq!(disposition(kc::A, Fate::Pass), Disposition::Original);
        assert_eq!(disposition(kc::A, Fate::Drop), Disposition::Nothing);
    }

    /// Wheel deltas are written continuous first, then lines, then fixed
    /// point, then pixels: each line write makes macOS recompute the others.
    #[test]
    fn wheel_writes_go_line_then_fixed_then_pixels() {
        let writes = scroll_writes([1, 0], [3.0, 0.0], [30, 0]);
        let order: Vec<u32> = writes
            .iter()
            .map(|write| match *write {
                FieldWrite::Int(written, _) | FieldWrite::Double(written, _) => written.0,
            })
            .collect();
        assert_eq!(order, [88, 11, 12, 93, 94, 96, 97]);
        assert_eq!(writes[0], FieldWrite::Int(field::SCROLL_IS_CONTINUOUS, 0));
        assert_eq!(writes[3], FieldWrite::Double(field::SCROLL_FIXED_1, 3.0));
    }

    /// Why the order matters: setting a line delta overwrites the fixed and
    /// pixel deltas, so they must be written after it. Checked on an
    /// in-memory event; nothing is posted.
    #[test]
    fn setting_a_line_delta_recomputes_the_other_two() {
        let event = CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Line, 1, 1, 0, 0)
            .expect("an in-memory scroll event");
        CGEvent::set_double_value_field(Some(&event), field::SCROLL_FIXED_1, 3.0);
        CGEvent::set_integer_value_field(Some(&event), field::SCROLL_LINE_1, 1);
        assert_eq!(
            CGEvent::double_value_field(Some(&event), field::SCROLL_FIXED_1),
            1.0
        );
    }

    /// Applying the writes gives a notched-wheel event with exactly the
    /// engine's deltas, and reading it back yields them again.
    #[test]
    fn applied_wheel_writes_read_back_as_written() {
        let event = CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Pixel, 2, 7, 5, 0)
            .expect("an in-memory scroll event");
        apply_writes(&event, &scroll_writes([-1, 0], [-3.0, 0.0], [-30, 0]));
        let read = read_scroll(&event);
        assert!(!read.continuous);
        assert_eq!(read.line, [-1, 0]);
        assert_eq!(read.fixed, [-3.0, 0.0]);
        assert_eq!(read.point, [-30, 0]);
        assert_eq!(read.sender, 0, "software events have no sender");
    }

    /// Key event types map to phases; anything else is not a key.
    #[test]
    fn event_types_map_to_key_phases() {
        assert_eq!(key_phase(CGEventType::KeyDown), Some(KeyPhase::Down));
        assert_eq!(key_phase(CGEventType::KeyUp), Some(KeyPhase::Up));
        assert_eq!(
            key_phase(CGEventType::FlagsChanged),
            Some(KeyPhase::FlagsChanged)
        );
        assert_eq!(key_phase(CGEventType::ScrollWheel), None);
    }

    /// Only buttons 4 and 5 reach the mouse engine; the middle button and
    /// higher buttons pass.
    #[test]
    fn only_buttons_four_and_five_are_side_buttons() {
        assert!(is_side_button(3));
        assert!(is_side_button(4));
        assert!(!is_side_button(2));
        assert!(!is_side_button(5));
    }

    /// A wheel event from an unknown sender gets the default profile, a
    /// known one its own.
    #[test]
    fn unknown_senders_get_the_default_mouse_profile() {
        let mut snapshot = TapSnapshot {
            mouse_default: MouseProfile::MAC,
            ..TapSnapshot::default()
        };
        snapshot
            .mouse_by_sender
            .insert(0x1_0000_6091, MouseProfile::WINDOWS);
        assert_eq!(*profile_of(&snapshot, 0x1_0000_6091), MouseProfile::WINDOWS);
        assert_eq!(*profile_of(&snapshot, 7), MouseProfile::MAC);
    }

    /// The clock moves forward and is in nanoseconds since boot.
    #[test]
    fn the_monotonic_clock_moves_forward() {
        let first = monotonic_ns();
        let second = monotonic_ns();
        assert!(first > 0);
        assert!(second >= first);
    }
}
