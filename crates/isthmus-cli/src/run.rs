//! `isthmus run`: the service in this process, without the app.
//!
//! The service watches the frontmost app through `NSWorkspace`
//! notifications, which AppKit delivers only on a running main loop, so the
//! main thread becomes an AppKit accessory app (no Dock icon, no menu bar)
//! and runs `NSApplication::run` for the rest of the process. The service
//! starts on the main thread before that loop runs: it registers its
//! observers asynchronously, and they take effect once the loop turns.
//!
//! Every way out puts the keyboard and mouse back first. SIGINT, SIGTERM
//! and SIGHUP (the terminal closing) reach a thread through [`Signals`],
//! which shuts the service down and only then exits. AppKit's own quit (a
//! logout or restart, an AppleScript `quit`, Activity Monitor's Quit) calls
//! C's `exit` on the main thread with no unwinding, so an observer of its
//! will-terminate notification shuts the service down before that. An error
//! after the start shuts it down on the way out. The first of these to come
//! does the restore and the others wait for it ([`Exit`]). Only SIGKILL or
//! a crash skips all of them, and then the journal lets the next start (or
//! `isthmus restore`) put things back.

use std::io::Write as _;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Once};

use isthmus_core::paths;
use isthmus_input::context::AppClass;
use isthmus_input::status::{Issue, TapState};
use isthmus_input::{
    Change, ChangeSink, InputService, Preset, PresetTarget, ServiceOptions, Status,
};
use snafu::ResultExt;

use crate::error::{ApplyPresetSnafu, CliError, DataDirSnafu, SignalsSnafu, StartSnafu};
use crate::signals::Signals;

/// How many change hints may wait for the printer; more are dropped, which
/// costs nothing, since the printer re-reads the whole status anyway.
const HINT_QUEUE: usize = 64;

/// Runs the service until a signal or AppKit ends it; does not return on
/// success.
pub(crate) fn run(preset: Option<Preset>) -> Result<(), CliError> {
    let mut main_loop = MainLoop::new()?;
    let dir = paths::data_dir().context(DataDirSnafu)?;
    if std::env::var_os("ISTHMUS_DATA_DIR").is_some_and(|dir| !dir.is_empty()) {
        eprintln!(
            "isthmus: warning: ISTHMUS_DATA_DIR is set, so the instance lock lives there too \
             and does not keep out an Isthmus using the default directory"
        );
    }
    // Installed before the start, so a Ctrl-C during it is kept and acted
    // on as soon as there is a service to shut down.
    let mut signals = Signals::install().context(SignalsSnafu)?;

    let (hints, hint_queue) = mpsc::sync_channel::<Change>(HINT_QUEUE);
    let on_change: ChangeSink = Arc::new(move |change| {
        // Must not block: a full queue drops the hint.
        let _ = hints.try_send(change);
    });
    let service = InputService::start(ServiceOptions {
        data_dir: dir,
        on_change: Some(on_change),
    })
    .context(StartSnafu)?;
    let exit = Exit::new({
        let service = service.clone();
        move || {
            say("Putting the keyboard and mouse back...");
            service.shutdown();
            say("Done.");
        }
    });
    let running = ShutdownOnDrop(exit.clone());

    if let Some(preset) = preset {
        service
            .apply_preset(preset, PresetTarget::All)
            .context(ApplyPresetSnafu)?;
    }
    let status = service.status();
    say(&summary(&status));
    if !status.enabled {
        say("Isthmus is switched off in the settings: `--preset windows` switches it on.");
    }
    say("Running. Ctrl-C puts the keyboard and mouse back and quits.");

    let printer = service.clone();
    // Without the printer the service still works; say so and go on.
    if let Err(error) = spawn("isthmus-cli-print", move || {
        print_changes(&printer, hint_queue)
    }) {
        tracing::warn!(%error, "status changes will not be printed");
    }
    // Without this thread Ctrl-C would do nothing at all, so it is an error,
    // and the guard puts everything back on the way out.
    let on_signal = exit.clone();
    spawn("isthmus-cli-signals", move || {
        if let Err(error) = signals.wait() {
            // Ctrl-C can no longer be seen, so stop now rather than run on
            // with no way to stop cleanly.
            tracing::error!(%error, "lost the signal pipe; stopping");
        }
        on_signal.put_back_before_exit();
        std::process::exit(0);
    })
    .context(SignalsSnafu)?;

    main_loop.on_terminate(move || exit.put_back_before_exit());
    main_loop.run();
    drop(running);
    Ok(())
}

/// The one restore of a run, shared by every way out: the signal thread,
/// AppKit's terminate and the guard on an error. Each of them ends the
/// process right after, and the service's own shutdown returns at once to
/// a second caller, so without this a second way out would exit while the
/// first was still putting things back. The first caller does the restore;
/// any other waits for it.
#[derive(Clone)]
struct Exit(Arc<ExitOnce>);

struct ExitOnce {
    once: Once,
    put_back: Box<dyn Fn() + Send + Sync>,
}

impl Exit {
    fn new(put_back: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(ExitOnce {
            once: Once::new(),
            put_back: Box::new(put_back),
        }))
    }

    /// Puts everything back unless another way out has, and says whether
    /// this caller did it. A caller that did not returns only once the one
    /// that did has finished. A restore that panicked does not poison the
    /// next caller: it runs again, and the service's shutdown is idempotent.
    fn put_back(&self) -> bool {
        let mut ran = false;
        self.0.once.call_once_force(|_| {
            (self.0.put_back)();
            ran = true;
        });
        ran
    }

    /// [`Self::put_back`] for a caller that ends the process next. If
    /// another way out did the restore, that one is ending the process
    /// already, and C's `exit` must not run twice at once (atexit handlers
    /// and static destructors would race), so this caller waits for it
    /// instead of returning.
    fn put_back_before_exit(&self) {
        if !self.put_back() {
            loop {
                std::thread::park();
            }
        }
    }
}

/// Shuts the service down when dropped: on an error after the start, or a
/// panic unwinding out of `run`, after which the process ends.
struct ShutdownOnDrop(Exit);

impl Drop for ShutdownOnDrop {
    fn drop(&mut self) {
        self.0.put_back_before_exit();
    }
}

/// Starts a named thread, so a sample or a crash report says which is which.
fn spawn(name: &str, body: impl FnOnce() + Send + 'static) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(body)
        .map(drop)
}

/// Prints the status whenever a hint changes what it says, and the device
/// count when devices come or go.
fn print_changes(service: &InputService, hints: Receiver<Change>) {
    let mut last = summary(&service.status());
    while let Ok(first) = hints.recv() {
        let mut devices = first == Change::Devices;
        for more in hints.try_iter() {
            devices |= more == Change::Devices;
        }
        let now = summary(&service.status());
        if now != last {
            say(&now);
            last = now;
        }
        if devices {
            let connected = service.devices().iter().filter(|d| d.connected).count();
            say(&format!("devices: {connected} connected"));
        }
    }
}

/// A line on stdout; a closed stdout is not a reason to stop the service.
fn say(line: &str) {
    let _ = writeln!(std::io::stdout(), "{line}");
}

/// The status as a few lines: whether Isthmus applies anything, the taps,
/// the HID side, the app in front, and any issue.
fn summary(status: &Status) -> String {
    let state = match (status.enabled, status.running, status.another_instance) {
        (false, _, _) => "Isthmus is off",
        (true, true, _) => "Isthmus is on and applying the settings",
        (true, false, true) => {
            "Isthmus is on but changes nothing: another Isthmus owns the keyboard and mouse"
        }
        (true, false, false) => "Isthmus is on but not applying the settings",
    };
    let front = status.front_app.as_ref().map_or_else(
        || "unknown".to_owned(),
        |app| format!("{} ({})", app.bundle_id, class_name(app.profile.class)),
    );
    let mut text = format!(
        "{state}\n  keyboard tap {}, mouse tap {}, {} keyboards mapped, {} mice tuned\n  front app {front}",
        tap_name(status.keyboard_tap),
        tap_name(status.mouse_tap),
        status.hid.keyboards_mapped,
        status.hid.mice_tuned,
    );
    if status.secure_input {
        text.push_str("\n  Secure Input is on: shortcut translation pauses");
    }
    for issue in &status.issues {
        text.push_str("\n  ! ");
        text.push_str(&issue_text(issue));
    }
    text
}

fn tap_name(state: TapState) -> &'static str {
    match state {
        TapState::Running => "running",
        TapState::Off => "off",
        TapState::NeedsPermission => "needs Accessibility",
        TapState::Failed => "failed",
    }
}

fn class_name(class: AppClass) -> &'static str {
    match class {
        AppClass::Terminal => "terminal",
        AppClass::NativeCtrl => "native Ctrl",
        AppClass::Passthrough => "passthrough",
        AppClass::Standard => "standard",
    }
}

fn issue_text(issue: &Issue) -> String {
    match issue {
        Issue::KarabinerActive => {
            "Karabiner-Elements has seized keyboards; Isthmus's mapping does not reach them".into()
        }
        Issue::SystemModifierKeys { device } => {
            format!("System Settings' Modifier Keys also changes {device}")
        }
        Issue::ForeignPointerTool { name } => {
            format!("{name} also writes pointer settings and may undo Isthmus's speed")
        }
        Issue::RelaunchNeeded => {
            "Accessibility reads as granted but the tap still fails; relaunch to apply it".into()
        }
        Issue::RestorePending { count } => {
            format!("{count} changes from a previous run are still to be put back")
        }
        Issue::SettingsUnreadable => {
            "settings.json could not be read; running with the defaults, switched off".into()
        }
        Issue::Unknown => "an issue this build does not know".into(),
    }
}

/// The AppKit main loop, on the main thread.
#[cfg(target_os = "macos")]
struct MainLoop {
    app: objc2::rc::Retained<objc2_app_kit::NSApplication>,
    /// The will-terminate observer, held for the life of the process.
    observers: Vec<appkit::Observer>,
}

#[cfg(target_os = "macos")]
impl MainLoop {
    /// Makes this process an AppKit accessory app, before the service
    /// starts, so its observers have an application to be delivered to.
    fn new() -> Result<Self, CliError> {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
        use snafu::OptionExt;

        use crate::error::NotMainThreadSnafu;

        let mtm = MainThreadMarker::new().context(NotMainThreadSnafu)?;
        let app = NSApplication::sharedApplication(mtm);
        // No Dock icon or menu bar, as the app itself (an LSUIElement).
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        Ok(Self {
            app,
            observers: Vec::new(),
        })
    }

    /// Runs `put_back` on the main thread when AppKit terminates the app,
    /// before it calls `exit`. `run` makes this an AppKit app with the
    /// standard quit Apple Event handler, so a logout, a restart, an
    /// AppleScript `quit` or Activity Monitor's Quit go through
    /// `-[NSApplication terminate:]`, which skips the signal thread and
    /// every destructor. The notification is observed instead of setting a
    /// delegate: the default answer to "should terminate" is "now", and a
    /// delegate could only delay or cancel a logout.
    fn on_terminate(&mut self, put_back: impl Fn() + Send + Sync + 'static) {
        // SAFETY: AppKit's notification name constant, set before main runs.
        let name = unsafe { objc2_app_kit::NSApplicationWillTerminateNotification };
        let center = objc2_foundation::NSNotificationCenter::defaultCenter();
        self.observers
            .push(appkit::observe(&center, name, put_back));
    }

    /// Runs the loop; it never returns, as the signal thread or AppKit's
    /// terminate exits the process.
    fn run(self) {
        self.app.run();
    }
}

/// Notification observers for the main loop, apart from it so a test can
/// drive one through a private center without an `NSApplication`.
#[cfg(target_os = "macos")]
mod appkit {
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{NSObjectProtocol, ProtocolObject};
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSNotificationName};

    /// The center's handle on an observer; dropping it leaves the observer
    /// in place, which is what a process-long observer wants.
    pub(super) type Observer = Retained<ProtocolObject<dyn NSObjectProtocol>>;

    /// Calls `action` whenever `center` posts `name`, on the posting thread
    /// and before the post returns, so the poster (AppKit's terminate)
    /// waits for it.
    pub(super) fn observe(
        center: &NSNotificationCenter,
        name: &NSNotificationName,
        action: impl Fn() + Send + Sync + 'static,
    ) -> Observer {
        let block = RcBlock::new(move |_: NonNull<NSNotification>| action());
        // SAFETY: a NULL queue runs the block synchronously on the posting
        // thread; the block is `Send + Sync` and reads nothing from the
        // notification; a NULL object observes every sender.
        unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block) }
    }
}

/// Elsewhere there is no AppKit loop to run; the main thread just waits
/// for the signal thread to exit the process.
#[cfg(not(target_os = "macos"))]
struct MainLoop;

#[cfg(not(target_os = "macos"))]
impl MainLoop {
    fn new() -> Result<Self, CliError> {
        Ok(Self)
    }

    /// No AppKit to quit the process here; the signals are the only way.
    fn on_terminate(&mut self, _put_back: impl Fn() + Send + Sync + 'static) {}

    fn run(self) {
        loop {
            std::thread::park();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use isthmus_input::hidmap::HidProfile;
    use isthmus_input::status::{HidBackend, HidStatus, PermissionState, Permissions};

    use super::*;

    fn status() -> Status {
        Status {
            version: "test".into(),
            enabled: true,
            running: false,
            another_instance: true,
            permissions: Permissions {
                accessibility: PermissionState::Granted,
            },
            keyboard_tap: TapState::Off,
            mouse_tap: TapState::Off,
            secure_input: false,
            hid: HidStatus {
                backend: HidBackend::Full,
                keyboards_mapped: 0,
                mice_tuned: 0,
            },
            front_app: None,
            hid_profile: HidProfile::Positions,
            issues: Vec::new(),
        }
    }

    /// The printer shows a new line only when something the user would see
    /// changed, and the reason a run changes nothing is part of the line.
    #[test]
    fn the_summary_changes_with_what_the_user_should_know() {
        let mut status = status();
        let inert = summary(&status);
        assert!(inert.contains("another Isthmus"), "{inert}");

        status.running = true;
        status.another_instance = false;
        status.keyboard_tap = TapState::Running;
        let running = summary(&status);
        assert_ne!(running, inert);

        status.issues.push(Issue::RestorePending { count: 2 });
        let with_issue = summary(&status);
        assert_ne!(with_issue, running);
        assert_eq!(with_issue.lines().count(), running.lines().count() + 1);
    }

    /// A logout can SIGHUP the run from the terminal and quit it through
    /// AppKit at the same moment. The second way out must wait for the
    /// first restore to finish before the process exits, and the restore
    /// runs once.
    #[test]
    fn a_second_way_out_waits_for_the_first_restore_to_finish() {
        let restores = Arc::new(AtomicUsize::new(0));
        let (entered, restore_entered) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let released = std::sync::Mutex::new(released);
        let exit = Exit::new({
            let restores = restores.clone();
            move || {
                restores.fetch_add(1, Ordering::SeqCst);
                entered.send(()).unwrap();
                released.lock().unwrap().recv().unwrap();
            }
        });

        let first = std::thread::spawn({
            let exit = exit.clone();
            move || exit.put_back()
        });
        restore_entered.recv().unwrap();
        let (done, second_done) = mpsc::channel();
        let second = std::thread::spawn({
            let exit = exit.clone();
            move || done.send(exit.put_back()).unwrap()
        });
        assert!(
            second_done
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "the second way out returned while the first was still putting things back"
        );
        release.send(()).unwrap();
        assert!(first.join().unwrap(), "the first caller did the restore");
        assert!(!second_done.recv().unwrap(), "the second only waited");
        second.join().unwrap();
        assert_eq!(restores.load(Ordering::SeqCst), 1);
    }

    /// The terminate observer runs its action before the post returns,
    /// which is what lets the restore finish before AppKit's `exit`. Driven
    /// through a private center and name, so nothing else in the process
    /// hears it.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_observed_notification_runs_its_action_before_the_post_returns() {
        use objc2_foundation::{NSNotificationCenter, NSString};

        objc2::rc::autoreleasepool(|_| {
            let center = NSNotificationCenter::new();
            let name = NSString::from_str("dev.acking.isthmus.test.terminate");
            let calls = Arc::new(AtomicUsize::new(0));
            let observer = appkit::observe(&center, &name, {
                let calls = calls.clone();
                move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                }
            });
            // SAFETY: a name this test made and no sender object.
            unsafe { center.postNotificationName_object(&name, None) };
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            // SAFETY: the observer came from this center.
            unsafe { center.removeObserver(observer.as_ref()) };
            // SAFETY: as above.
            unsafe { center.postNotificationName_object(&name, None) };
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        });
    }
}
