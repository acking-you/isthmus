//! A listen-only tap that prints what devices send (tap half).
//!
//! Several facts the tap relies on can only be checked against real hardware:
//! how a wheel's line, fixed-point and pixel deltas grow, what the raw and
//! accelerated deltas (175 to 178) hold, whether natural scrolling marks
//! wheel events (137), and which HID service sends what (87). This prints
//! every one of those fields for each key, flag, button and wheel event. It is
//! listen-only at the session location so it changes nothing and runs beside
//! a running Isthmus; it needs Input Monitoring or Accessibility, which macOS
//! asks for on first use. Reading an event into a [`Record`] is separate from
//! printing it, so the format is tested without a tap.

use std::ffi::c_void;
use std::panic::{self, AssertUnwindSafe};
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2_core_foundation::{CFMachPort, CFRetained, CFRunLoop, kCFRunLoopDefaultMode};
use objc2_core_graphics::{
    CGEvent, CGEventMask, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType,
};

use super::synth::field;
use crate::error::Error;

/// Keys, modifiers, side buttons and the wheel: what the tap handles. Never
/// pointer motion, which would only flood the output.
const MASK: CGEventMask = (1 << CGEventType::KeyDown.0)
    | (1 << CGEventType::KeyUp.0)
    | (1 << CGEventType::FlagsChanged.0)
    | (1 << CGEventType::ScrollWheel.0)
    | (1 << CGEventType::OtherMouseDown.0)
    | (1 << CGEventType::OtherMouseUp.0);

/// How often the loop checks `stop`, in seconds.
const POLL_SECONDS: f64 = 0.1;

/// One event's fields, read once, in the order they are printed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Record {
    Key {
        kind: &'static str,
        keycode: i64,
        flags: u64,
        autorepeat: i64,
        keyboard_type: i64,
        sender: i64,
    },
    Wheel {
        flags: u64,
        continuous: i64,
        scroll_phase: i64,
        momentum_phase: i64,
        line: [i64; 2],
        fixed: [f64; 2],
        point: [i64; 2],
        inverted: i64,
        sender: i64,
        accelerated: [f64; 2],
        raw: [f64; 2],
    },
    Button {
        kind: &'static str,
        button: i64,
        flags: u64,
        sender: i64,
    },
    /// Anything else the mask lets through (a disabled-tap notice).
    Other { kind: u32 },
}

/// The name `ty` is printed with.
fn kind_name(ty: CGEventType) -> &'static str {
    match ty {
        CGEventType::KeyDown => "keyDown",
        CGEventType::KeyUp => "keyUp",
        CGEventType::FlagsChanged => "flagsChanged",
        CGEventType::OtherMouseDown => "otherMouseDown",
        CGEventType::OtherMouseUp => "otherMouseUp",
        CGEventType::ScrollWheel => "scrollWheel",
        _ => "other",
    }
}

/// Reads the fields Isthmus uses from `event`.
pub(crate) fn record(ty: CGEventType, event: &CGEvent) -> Record {
    let int = |read| CGEvent::integer_value_field(Some(event), read);
    let double = |read| CGEvent::double_value_field(Some(event), read);
    let flags = CGEvent::flags(Some(event)).0;
    match ty {
        CGEventType::KeyDown | CGEventType::KeyUp | CGEventType::FlagsChanged => Record::Key {
            kind: kind_name(ty),
            keycode: int(field::KEYCODE),
            flags,
            autorepeat: int(field::KEY_AUTOREPEAT),
            keyboard_type: int(field::KEYBOARD_TYPE),
            sender: int(field::SENDER_ID),
        },
        CGEventType::ScrollWheel => Record::Wheel {
            flags,
            continuous: int(field::SCROLL_IS_CONTINUOUS),
            scroll_phase: int(field::SCROLL_PHASE),
            momentum_phase: int(field::SCROLL_MOMENTUM_PHASE),
            line: [int(field::SCROLL_LINE_1), int(field::SCROLL_LINE_2)],
            fixed: [double(field::SCROLL_FIXED_1), double(field::SCROLL_FIXED_2)],
            point: [int(field::SCROLL_POINT_1), int(field::SCROLL_POINT_2)],
            inverted: int(field::SCROLL_INVERTED),
            sender: int(field::SENDER_ID),
            accelerated: [
                double(field::SCROLL_ACCELERATED_1),
                double(field::SCROLL_ACCELERATED_2),
            ],
            raw: [double(field::SCROLL_RAW_1), double(field::SCROLL_RAW_2)],
        },
        CGEventType::OtherMouseDown | CGEventType::OtherMouseUp => Record::Button {
            kind: kind_name(ty),
            button: int(field::MOUSE_BUTTON_NUMBER),
            flags,
            sender: int(field::SENDER_ID),
        },
        _ => Record::Other { kind: ty.0 },
    }
}

impl std::fmt::Display for Record {
    /// One line per event, each field labelled with its `CGEventField`
    /// number so a reading can be matched to the research notes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Record::Key {
                kind,
                keycode,
                flags,
                autorepeat,
                keyboard_type,
                sender,
            } => write!(
                f,
                "{kind} keycode={keycode:#04x} flags={flags:#x} autorepeat={autorepeat} \
                 kbtype={keyboard_type} sender(87)={sender:#x}"
            ),
            Record::Wheel {
                flags,
                continuous,
                scroll_phase,
                momentum_phase,
                line,
                fixed,
                point,
                inverted,
                sender,
                accelerated,
                raw,
            } => write!(
                f,
                "scrollWheel flags={flags:#x} continuous(88)={continuous} phase(99)={scroll_phase} \
                 momentum(123)={momentum_phase} line(11,12)={},{} fixed(93,94)={},{} \
                 point(96,97)={},{} inverted(137)={inverted} sender(87)={sender:#x} \
                 accelerated(176,175)={},{} raw(178,177)={},{}",
                line[0],
                line[1],
                fixed[0],
                fixed[1],
                point[0],
                point[1],
                accelerated[0],
                accelerated[1],
                raw[0],
                raw[1],
            ),
            Record::Button {
                kind,
                button,
                flags,
                sender,
            } => write!(
                f,
                "{kind} button(3)={button} flags={flags:#x} sender(87)={sender:#x}"
            ),
            Record::Other { kind } => write!(f, "event type {kind:#x}"),
        }
    }
}

/// What the callback reaches through `user_info`.
struct Inspector<F> {
    sink: F,
    /// The tap, to re-enable it when macOS disables it.
    port: Option<CFRetained<CFMachPort>>,
}

/// The listen-only tap's callback: formats the event for the sink and
/// returns it untouched.
///
/// # Safety
///
/// `user_info` must point to the live `Inspector<F>` that [`run`] created
/// the tap with, and the callback must run only on that thread's run loop.
unsafe extern "C-unwind" fn callback<F: FnMut(String)>(
    _proxy: CGEventTapProxy,
    ty: CGEventType,
    event: NonNull<CGEvent>,
    user_info: *mut c_void,
) -> *mut CGEvent {
    // SAFETY: the caller's contract: `user_info` is the boxed inspector,
    // alive until after the tap is invalidated, and used only here and on
    // this thread.
    let Some(inspector) = (unsafe { user_info.cast::<Inspector<F>>().as_mut() }) else {
        return event.as_ptr();
    };
    if ty == CGEventType::TapDisabledByTimeout || ty == CGEventType::TapDisabledByUserInput {
        if let Some(port) = &inspector.port {
            CGEvent::tap_enable(port, true);
        }
        return event.as_ptr();
    }
    // SAFETY: the window server passes a valid event for the duration of the
    // callback.
    let line = record(ty, unsafe { event.as_ref() }).to_string();
    // A panicking sink must not unwind into CoreGraphics; the event is
    // returned either way.
    let _ = panic::catch_unwind(AssertUnwindSafe(|| (inspector.sink)(line)));
    event.as_ptr()
}

/// Runs a listen-only tap for key, flag, button and wheel events on the
/// calling thread's run loop until `stop` is set (checked every 100 ms),
/// handing each event, formatted with the fields Isthmus reads, to `sink`.
pub(crate) fn run<F: FnMut(String)>(stop: Arc<AtomicBool>, sink: F) -> Result<(), Error> {
    let run_loop = CFRunLoop::current().ok_or(Error::System {
        what: "the current run loop",
        status: 0,
    })?;
    // SAFETY: reading an immutable CoreFoundation constant.
    let mode = unsafe { kCFRunLoopDefaultMode };
    let inspector = Box::into_raw(Box::new(Inspector { sink, port: None }));
    // SAFETY: `callback::<F>` matches the inspector's type, and the box
    // outlives the tap: it is freed below, after the tap is invalidated.
    let port = unsafe {
        CGEvent::tap_create(
            CGEventTapLocation::SessionEventTap,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            MASK,
            Some(callback::<F>),
            inspector.cast(),
        )
    };
    let result = match port {
        Some(port) => listen(&run_loop, mode, &port, inspector, &stop),
        None => Err(Error::PermissionMissing {
            what: "Input Monitoring",
        }),
    };
    // SAFETY: the tap, if any, is invalidated, so no callback can still see
    // the inspector; this is the only owner left.
    drop(unsafe { Box::from_raw(inspector) });
    result
}

/// Services the tap until `stop`, then tears it down: disabled, removed from
/// the run loop and invalidated, so the window server forgets it.
fn listen<F>(
    run_loop: &CFRunLoop,
    mode: Option<&'static objc2_core_foundation::CFRunLoopMode>,
    port: &CFRetained<CFMachPort>,
    inspector: *mut Inspector<F>,
    stop: &AtomicBool,
) -> Result<(), Error> {
    let Some(source) = CFMachPort::new_run_loop_source(None, Some(port), 0) else {
        port.invalidate();
        return Err(Error::System {
            what: "a run-loop source for the event tap",
            status: 0,
        });
    };
    // SAFETY: the callback has not run yet (the tap is not on a run loop),
    // so nothing else holds the inspector.
    if let Some(inspector) = unsafe { inspector.as_mut() } {
        inspector.port = Some(port.clone());
    }
    run_loop.add_source(Some(&source), mode);
    CGEvent::tap_enable(port, true);
    while !stop.load(Ordering::Relaxed) {
        CFRunLoop::run_in_mode(mode, POLL_SECONDS, false);
    }
    CGEvent::tap_enable(port, false);
    run_loop.remove_source(Some(&source), mode);
    port.invalidate();
    // SAFETY: the tap is invalidated; drop the inspector's port clone here
    // while nothing else can reach it.
    if let Some(inspector) = unsafe { inspector.as_mut() } {
        inspector.port = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use objc2_core_graphics::CGScrollEventUnit;

    use super::*;

    /// The mask never includes pointer motion, which only floods the output.
    #[test]
    fn the_mask_leaves_out_pointer_motion() {
        assert_eq!(MASK & (1 << CGEventType::MouseMoved.0), 0);
        assert_ne!(MASK & (1 << CGEventType::ScrollWheel.0), 0);
    }

    /// A wheel event is read with every delta field, and the line names each
    /// one by its field number. Built in memory only.
    #[test]
    fn a_wheel_event_prints_every_delta_field() {
        let event = CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Line, 2, 3, -1, 0)
            .expect("an in-memory scroll event");
        let record = record(CGEventType::ScrollWheel, &event);
        let Record::Wheel {
            continuous, line, ..
        } = record
        else {
            panic!("a wheel record, got {record:?}");
        };
        assert_eq!(continuous, 0);
        assert_eq!(line, [3, -1]);
        let printed = record.to_string();
        for label in [
            "continuous(88)=0",
            "line(11,12)=3,-1",
            "fixed(93,94)=",
            "point(96,97)=",
            "inverted(137)=",
            "sender(87)=",
            "raw(178,177)=",
        ] {
            assert!(printed.contains(label), "{label} missing from {printed}");
        }
    }

    /// A key event prints its keycode, flags and autorepeat.
    #[test]
    fn a_key_event_prints_keycode_flags_and_repeat() {
        let printed = Record::Key {
            kind: "keyDown",
            keycode: 0x73,
            flags: 0x80_0100,
            autorepeat: 1,
            keyboard_type: 40,
            sender: 0,
        }
        .to_string();
        assert!(printed.starts_with("keyDown keycode=0x73 flags=0x800100 autorepeat=1"));
    }
}
