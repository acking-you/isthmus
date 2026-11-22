//! Turning the engine's answers into `CGEvent`s (tap half).
//!
//! Every event Isthmus creates is built here, so every one carries
//! [`ISTHMUS_TAG`] in `kCGEventSourceUserData` (field 42) and the tap passes
//! it through instead of translating its own output. Key events come from an
//! `HIDSystemState` source so they look like hardware to consumers that track
//! modifier state, and their flags are always set explicitly: a created event
//! otherwise inherits whatever modifiers the source's state table holds
//! (eventtap.md §3.4). What each event should be is decided by plain values
//! ([`KeyPlan`], [`SwipePlan`]) that the tests pin; the `CGEvent` calls around
//! them stay a thin shell.

use objc2_core_foundation::{CFRetained, CGPoint};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation,
    CGEventType, CGMouseButton, CGScrollEventUnit,
};

use super::ISTHMUS_TAG;
use crate::engine::Synth;
use crate::keys::{Flags, KeyCode};

/// The `CGEventField`s Isthmus reads and writes. Several have no name in the
/// SDK (87, 110, 115, 132, 137), so all of them are numbered here in one
/// place, where they can be checked against the research notes.
pub(crate) mod field {
    use objc2_core_graphics::CGEventField;

    /// `kCGMouseEventButtonNumber`: 3 is back, 4 is forward.
    pub(crate) const MOUSE_BUTTON_NUMBER: CGEventField = CGEventField(3);
    /// `kCGKeyboardEventAutorepeat`: non-zero on a held key's repeats.
    pub(crate) const KEY_AUTOREPEAT: CGEventField = CGEventField(8);
    /// `kCGKeyboardEventKeycode`: the virtual keycode.
    pub(crate) const KEYCODE: CGEventField = CGEventField(9);
    /// `kCGKeyboardEventKeyboardType`: the keyboard's layout type, which
    /// a replacement event copies so consumers translate it the same way.
    pub(crate) const KEYBOARD_TYPE: CGEventField = CGEventField(10);
    /// `kCGScrollWheelEventDeltaAxis1`: vertical lines. Setting it makes
    /// macOS recompute fields 93 and 96.
    pub(crate) const SCROLL_LINE_1: CGEventField = CGEventField(11);
    /// `kCGScrollWheelEventDeltaAxis2`: horizontal lines.
    pub(crate) const SCROLL_LINE_2: CGEventField = CGEventField(12);
    /// `kCGEventSourceUserData`: where Isthmus puts [`super::ISTHMUS_TAG`].
    pub(crate) const SOURCE_USER_DATA: CGEventField = CGEventField(42);
    /// Undocumented: the registry id of the HID service that sent the
    /// event, 0 for software (Mac Mouse Fix's `kMFCGEventFieldSenderID`).
    pub(crate) const SENDER_ID: CGEventField = CGEventField(87);
    /// `kCGScrollWheelEventIsContinuous`: 0 for a notched wheel.
    pub(crate) const SCROLL_IS_CONTINUOUS: CGEventField = CGEventField(88);
    /// `kCGScrollWheelEventFixedPtDeltaAxis1`, 16.16 fixed point: read and
    /// written as a double.
    pub(crate) const SCROLL_FIXED_1: CGEventField = CGEventField(93);
    /// `kCGScrollWheelEventFixedPtDeltaAxis2`.
    pub(crate) const SCROLL_FIXED_2: CGEventField = CGEventField(94);
    /// `kCGScrollWheelEventPointDeltaAxis1`: pixels, an integer.
    pub(crate) const SCROLL_POINT_1: CGEventField = CGEventField(96);
    /// `kCGScrollWheelEventPointDeltaAxis2`.
    pub(crate) const SCROLL_POINT_2: CGEventField = CGEventField(97);
    /// `kCGScrollWheelEventScrollPhase`: non-zero for trackpad gestures.
    pub(crate) const SCROLL_PHASE: CGEventField = CGEventField(99);
    /// Undocumented: the IOHID event type of a gesture event.
    pub(crate) const GESTURE_HID_TYPE: CGEventField = CGEventField(110);
    /// Undocumented: the swipe direction mask of a gesture event.
    pub(crate) const GESTURE_SWIPE_VALUE: CGEventField = CGEventField(115);
    /// `kCGScrollWheelEventMomentumPhase`: non-zero for momentum scrolling.
    pub(crate) const SCROLL_MOMENTUM_PHASE: CGEventField = CGEventField(123);
    /// Undocumented: the phase of a gesture event (`CGSGesturePhase`).
    pub(crate) const GESTURE_PHASE: CGEventField = CGEventField(132);
    /// Undocumented: natural scrolling already inverted this event's deltas
    /// (backs `NSEvent.isDirectionInvertedFromDevice`).
    pub(crate) const SCROLL_INVERTED: CGEventField = CGEventField(137);
    /// `kCGScrollWheelEventAcceleratedDeltaAxis2` (a double). The SDK
    /// numbers the pair in reverse: axis 2 is 175, axis 1 is 176.
    pub(crate) const SCROLL_ACCELERATED_2: CGEventField = CGEventField(175);
    /// `kCGScrollWheelEventAcceleratedDeltaAxis1` (a double).
    pub(crate) const SCROLL_ACCELERATED_1: CGEventField = CGEventField(176);
    /// `kCGScrollWheelEventRawDeltaAxis2` (a double).
    pub(crate) const SCROLL_RAW_2: CGEventField = CGEventField(177);
    /// `kCGScrollWheelEventRawDeltaAxis1` (a double).
    pub(crate) const SCROLL_RAW_1: CGEventField = CGEventField(178);
}

/// The source state every synthetic key event is created with. Hardware-like
/// sources give events a keyboard type and a hardware state table, which
/// Wine-class consumers need (LinearMouse `CGEvent+HardwareLikeKeyEvent`).
/// One constant, because whether posting from it hides the physical Alt from
/// `CGEventSourceKeyState(HIDSystemState, ..)` is still unverified; the
/// alternative is `Private`.
pub(crate) const SYNTH_SOURCE_STATE: CGEventSourceStateID = CGEventSourceStateID::HIDSystemState;

/// Creates the event source the tap thread keeps for its synthetic keys.
/// `None` still works (events are then sourceless), so callers tolerate it.
pub(crate) fn event_source() -> Option<CFRetained<CGEventSource>> {
    CGEventSource::new(SYNTH_SOURCE_STATE)
}

/// What one [`Synth`] becomes, decided without the OS so it can be tested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyPlan {
    pub keycode: KeyCode,
    /// The `keyDown` argument of `CGEventCreateKeyboardEvent`. A modifier is
    /// created as a key-down and then retyped, because a `flagsChanged`
    /// event's direction lives in its flags, not in its type.
    pub key_down: bool,
    /// Retype the event to `kCGEventFlagsChanged` after creating it.
    pub flags_changed: bool,
    /// The complete flags, set explicitly over whatever the source holds.
    pub flags: Flags,
}

impl KeyPlan {
    /// The plan for `synth`. Every variant gets a decision of its own.
    pub(crate) const fn of(synth: Synth) -> KeyPlan {
        match synth {
            Synth::Key {
                keycode,
                flags,
                down,
            } => KeyPlan {
                keycode,
                key_down: down,
                flags_changed: false,
                flags,
            },
            Synth::Modifier { keycode, flags } => KeyPlan {
                keycode,
                key_down: true,
                flags_changed: true,
                flags,
            },
        }
    }
}

/// The engine's flag bits as CoreGraphics takes them. Both are the raw
/// `CGEventFlags` word, so this is a relabelling, not a conversion.
pub(crate) const fn cg_flags(flags: Flags) -> CGEventFlags {
    CGEventFlags(flags.0)
}

/// Marks `event` as Isthmus's own, so the tap passes it through.
pub(crate) fn tag(event: &CGEvent) {
    CGEvent::set_integer_value_field(Some(event), field::SOURCE_USER_DATA, ISTHMUS_TAG);
}

/// Whether `event` is one Isthmus made. Checked first in the callback, so
/// the tap never translates its own output.
pub(crate) fn is_tagged(event: &CGEvent) -> bool {
    CGEvent::integer_value_field(Some(event), field::SOURCE_USER_DATA) == ISTHMUS_TAG
}

/// Creates the tagged event for `synth`. `timestamp` is the triggering
/// event's, when there is one, so the synthetic keys sort with it; without
/// one the event keeps its creation time.
pub(crate) fn key_event(
    source: Option<&CGEventSource>,
    synth: Synth,
    timestamp: Option<u64>,
) -> Option<CFRetained<CGEvent>> {
    let plan = KeyPlan::of(synth);
    let event = CGEvent::new_keyboard_event(source, plan.keycode, plan.key_down)?;
    if plan.flags_changed {
        CGEvent::set_type(Some(&event), CGEventType::FlagsChanged);
    }
    CGEvent::set_flags(Some(&event), cg_flags(plan.flags));
    if let Some(timestamp) = timestamp {
        CGEvent::set_timestamp(Some(&event), timestamp);
    }
    tag(&event);
    Some(event)
}

/// Creates the event that replaces `original` when the engine changes its
/// keycode. A new event, not an edited one: an event keeps the characters of
/// the key it was created for, so F4 relabelled as Q would still type F4's
/// private-use character and miss Command+Q (eventtap.md §4.2). The
/// replacement keeps the original's type (a modifier stays `flagsChanged`),
/// timestamp, autorepeat and keyboard type.
pub(crate) fn replacement(
    source: Option<&CGEventSource>,
    original: &CGEvent,
    original_type: CGEventType,
    keycode: KeyCode,
    flags: Flags,
) -> Option<CFRetained<CGEvent>> {
    let key_down = original_type != CGEventType::KeyUp;
    let event = CGEvent::new_keyboard_event(source, keycode, key_down)?;
    if original_type == CGEventType::FlagsChanged {
        CGEvent::set_type(Some(&event), CGEventType::FlagsChanged);
    }
    CGEvent::set_timestamp(Some(&event), CGEvent::timestamp(Some(original)));
    for copied in [field::KEY_AUTOREPEAT, field::KEYBOARD_TYPE] {
        let value = CGEvent::integer_value_field(Some(original), copied);
        CGEvent::set_integer_value_field(Some(&event), copied, value);
    }
    CGEvent::set_flags(Some(&event), cg_flags(flags));
    tag(&event);
    Some(event)
}

/// `NSEventTypeGesture`: the CoreGraphics type of a trackpad gesture event.
pub(crate) const GESTURE_EVENT_TYPE: CGEventType = CGEventType(29);
/// `kIOHIDEventTypeNavigationSwipe`, in field 110.
const NAVIGATION_SWIPE: i64 = 16;
/// `CGSGesturePhase` began, in field 132: the swipe starts.
const PHASE_BEGAN: i64 = 1;
/// `CGSGesturePhase` ended, in field 132: the swipe completes, which is what
/// makes the app navigate.
const PHASE_ENDED: i64 = 4;
/// `IOHIDSwipeMask` left, in field 115: goes back, as a two-finger trackpad
/// swipe does in Safari and Finder.
const SWIPE_LEFT: i64 = 0x04;
/// `IOHIDSwipeMask` right, in field 115: goes forward.
const SWIPE_RIGHT: i64 = 0x08;

/// The two events of a navigation swipe, as field writes. LinearMouse
/// (`GestureKit/GestureEvent+NavigationSwipe.swift`) sends exactly this pair
/// for buttons 4 and 5: the direction goes only on the began event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SwipePlan {
    pub began: [(CGEventField, i64); 3],
    pub ended: [(CGEventField, i64); 2],
}

/// The swipe that goes back (`back`) or forward.
pub(crate) const fn swipe_plan(back: bool) -> SwipePlan {
    let direction = if back { SWIPE_LEFT } else { SWIPE_RIGHT };
    SwipePlan {
        began: [
            (field::GESTURE_HID_TYPE, NAVIGATION_SWIPE),
            (field::GESTURE_PHASE, PHASE_BEGAN),
            (field::GESTURE_SWIPE_VALUE, direction),
        ],
        ended: [
            (field::GESTURE_HID_TYPE, NAVIGATION_SWIPE),
            (field::GESTURE_PHASE, PHASE_ENDED),
        ],
    }
}

/// Creates one tagged gesture event with `writes` applied. Sourceless, as
/// LinearMouse creates them.
fn gesture_event(writes: &[(CGEventField, i64)]) -> Option<CFRetained<CGEvent>> {
    let event = CGEvent::new(None)?;
    CGEvent::set_type(Some(&event), GESTURE_EVENT_TYPE);
    for &(written, value) in writes {
        CGEvent::set_integer_value_field(Some(&event), written, value);
    }
    tag(&event);
    Some(event)
}

/// The pointer's position in global points (origin top left), read from a
/// fresh event; `None` if none could be made.
pub(crate) fn pointer_location() -> Option<[f64; 2]> {
    let event = CGEvent::new(None)?;
    let point = CGEvent::location(Some(&event));
    Some([point.x, point.y])
}

/// The press and the release of a middle click at `at`, tagged: the click
/// a held middle press turned out to be.
pub(crate) fn middle_click(
    source: Option<&CGEventSource>,
    at: [f64; 2],
) -> Vec<CFRetained<CGEvent>> {
    let point = CGPoint::new(at[0], at[1]);
    [CGEventType::OtherMouseDown, CGEventType::OtherMouseUp]
        .into_iter()
        .filter_map(|ty| {
            let event = CGEvent::new_mouse_event(source, ty, point, CGMouseButton::Center)?;
            tag(&event);
            Some(event)
        })
        .collect()
}

/// Posts a tagged pixel scroll, vertical then horizontal, at the HID
/// location: a step of a middle-button drag scroll. Answers whether it was
/// posted.
pub(crate) fn post_pixel_scroll(source: Option<&CGEventSource>, pixels: [i32; 2]) -> bool {
    let Some(event) = CGEvent::new_scroll_wheel_event2(
        source,
        CGScrollEventUnit::Pixel,
        2,
        pixels[0],
        pixels[1],
        0,
    ) else {
        return false;
    };
    tag(&event);
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
    true
}

/// Creates the began and ended events of a navigation swipe.
pub(crate) fn navigation_swipe(back: bool) -> Option<[CFRetained<CGEvent>; 2]> {
    let plan = swipe_plan(back);
    Some([gesture_event(&plan.began)?, gesture_event(&plan.ended)?])
}

/// Posts a navigation swipe at the session location, where LinearMouse
/// posts it: downstream of the HID tap, so Isthmus's own tap never sees it.
/// Returns whether it was posted, for the log.
pub(crate) fn post_navigation_swipe(back: bool) -> bool {
    let Some(events) = navigation_swipe(back) else {
        return false;
    };
    for event in &events {
        CGEvent::post(CGEventTapLocation::SessionEventTap, Some(event));
    }
    true
}

#[cfg(test)]
mod tests {
    use objc2_core_graphics::CGEventFlags;

    use super::*;
    use crate::keys::kc;

    /// A modifier synth becomes a key-down retyped to `flagsChanged`, with
    /// exactly the engine's flags: the direction of a modifier event is in
    /// its flags, and an inherited Control would stick.
    #[test]
    fn modifier_synths_are_retyped_key_downs_with_exact_flags() {
        let flags = Flags::COMMAND | Flags::DEV_LCMD | Flags::NON_COALESCED;
        let plan = KeyPlan::of(Synth::Modifier {
            keycode: kc::COMMAND,
            flags,
        });
        assert_eq!(
            plan,
            KeyPlan {
                keycode: kc::COMMAND,
                key_down: true,
                flags_changed: true,
                flags,
            }
        );
    }

    /// A key synth keeps its direction and is never retyped.
    #[test]
    fn key_synths_keep_their_direction() {
        let up = KeyPlan::of(Synth::Key {
            keycode: kc::TAB,
            flags: Flags::COMMAND,
            down: false,
        });
        assert!(!up.key_down);
        assert!(!up.flags_changed);
    }

    /// The flag word reaches CoreGraphics unchanged, side bits included,
    /// since the tap reads the side bits to tell keyboards apart.
    #[test]
    fn flags_cross_to_core_graphics_bit_for_bit() {
        let flags = Flags::CONTROL | Flags::DEV_RCTL | Flags::FN | Flags::NUMERIC_PAD;
        assert_eq!(cg_flags(flags).0, flags.0);
        assert!(cg_flags(Flags::SHIFT).contains(CGEventFlags::MaskShift));
    }

    /// A created key event carries the tag, the keycode, the type and the
    /// exact flags. Built in memory only; nothing is posted.
    #[test]
    fn synthetic_key_events_are_tagged_and_carry_the_plan() {
        let flags = Flags::OPTION | Flags::DEV_LALT | Flags::NON_COALESCED;
        let event = key_event(
            None,
            Synth::Modifier {
                keycode: kc::OPTION,
                flags,
            },
            Some(42),
        )
        .expect("an in-memory keyboard event");
        assert!(is_tagged(&event));
        assert_eq!(CGEvent::r#type(Some(&event)), CGEventType::FlagsChanged);
        assert_eq!(
            CGEvent::integer_value_field(Some(&event), field::KEYCODE),
            i64::from(kc::OPTION)
        );
        assert_eq!(CGEvent::flags(Some(&event)).0, flags.0);
        assert_eq!(CGEvent::timestamp(Some(&event)), 42);
    }

    /// An event Isthmus did not make is not mistaken for its own.
    #[test]
    fn untagged_events_are_not_ours() {
        let event = CGEvent::new_keyboard_event(None, kc::A, true).expect("an in-memory event");
        assert!(!is_tagged(&event));
    }

    /// A replacement keeps what made the original a hardware key event
    /// (autorepeat, keyboard type, timestamp, and `flagsChanged` for a
    /// modifier) while taking the new keycode and flags.
    #[test]
    fn replacements_keep_the_originals_repeat_type_and_time() {
        let original = CGEvent::new_keyboard_event(None, kc::HOME, true).expect("an event");
        CGEvent::set_integer_value_field(Some(&original), field::KEY_AUTOREPEAT, 1);
        CGEvent::set_integer_value_field(Some(&original), field::KEYBOARD_TYPE, 40);
        CGEvent::set_timestamp(Some(&original), 1_234);
        let flags = Flags::COMMAND | Flags::DEV_LCMD | Flags::FN | Flags::NUMERIC_PAD;

        let event = replacement(None, &original, CGEventType::KeyDown, kc::LEFT, flags)
            .expect("a replacement");
        assert_eq!(CGEvent::r#type(Some(&event)), CGEventType::KeyDown);
        assert_eq!(
            CGEvent::integer_value_field(Some(&event), field::KEYCODE),
            i64::from(kc::LEFT)
        );
        assert_eq!(
            CGEvent::integer_value_field(Some(&event), field::KEY_AUTOREPEAT),
            1
        );
        assert_eq!(
            CGEvent::integer_value_field(Some(&event), field::KEYBOARD_TYPE),
            40
        );
        assert_eq!(CGEvent::timestamp(Some(&event)), 1_234);
        assert_eq!(CGEvent::flags(Some(&event)).0, flags.0);
        assert!(is_tagged(&event));

        let modifier = replacement(
            None,
            &original,
            CGEventType::FlagsChanged,
            kc::RIGHT_COMMAND,
            flags,
        )
        .expect("a replacement");
        assert_eq!(CGEvent::r#type(Some(&modifier)), CGEventType::FlagsChanged);
    }

    /// Back is a left swipe and forward a right one; only the began event
    /// carries the direction, as LinearMouse sends them.
    #[test]
    fn navigation_swipes_go_left_for_back_and_right_for_forward() {
        let back = swipe_plan(true);
        assert!(back.began.contains(&(field::GESTURE_SWIPE_VALUE, 0x04)));
        assert!(back.began.contains(&(field::GESTURE_PHASE, 1)));
        assert!(back.ended.contains(&(field::GESTURE_PHASE, 4)));
        assert!(
            !back
                .ended
                .iter()
                .any(|&(written, _)| written == field::GESTURE_SWIPE_VALUE)
        );
        assert!(
            swipe_plan(false)
                .began
                .contains(&(field::GESTURE_SWIPE_VALUE, 0x08))
        );
    }

    /// The swipe events are gesture events (type 29) of the navigation-swipe
    /// kind, tagged. Built in memory only.
    #[test]
    fn navigation_swipe_events_are_tagged_gestures() {
        let [began, ended] = navigation_swipe(true).expect("in-memory gesture events");
        for event in [&began, &ended] {
            assert_eq!(CGEvent::r#type(Some(event)), GESTURE_EVENT_TYPE);
            assert_eq!(
                CGEvent::integer_value_field(Some(event), field::GESTURE_HID_TYPE),
                16
            );
            assert!(is_tagged(event));
        }
        assert_eq!(
            CGEvent::integer_value_field(Some(&began), field::GESTURE_SWIPE_VALUE),
            0x04
        );
    }
}
