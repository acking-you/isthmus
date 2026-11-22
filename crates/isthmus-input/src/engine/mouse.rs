//! The mouse engine: wheel arithmetic and side buttons, on the tap thread.
//!
//! Only a notched wheel is ever changed: an event that is continuous, or in
//! a scroll or momentum phase, comes from a trackpad, a Magic Mouse or
//! software, which already scroll the macOS way. For a notched wheel:
//!
//! - **Direction.** macOS bakes natural scrolling into the deltas before any
//!   tap sees them and says so in field 137 (`inverted`). Windows direction
//!   is therefore "flip both axes when the event says it was inverted",
//!   which follows a live change of the system toggle without reading it.
//! - **Distance.** Each event is one notch (its sign is all that counts, so
//!   macOS's acceleration is thrown away), and scrolls N lines: a line delta
//!   of ±1 for apps that count ticks, a fixed-point delta of ±N for AppKit
//!   and browsers, and a pixel delta of ±10·N for apps that read pixels.
//! - **Ctrl+wheel.** One zoom step per notch, as Command+= or Command+-,
//!   because no macOS app zooms on Control+wheel and Chromium compiles its
//!   Ctrl+wheel zoom out on the Mac.
//!
//! Side buttons become a back or forward swipe in apps that ignore them,
//! and pass to the apps that handle them natively. A button's release
//! follows its press, whatever changed in between.

use super::{ButtonEvent, ButtonOutcome, Chord, MouseContext, ScrollEvent, ScrollOutcome, Synth};
use crate::context::AppClass;
use crate::hidmap;
use crate::keys::{Mods, kc};
use crate::settings::MouseProfile;

/// `kCGMouseEventButtonNumber` of the back button (mouse button 4).
pub const BACK_BUTTON: i64 = 3;
/// `kCGMouseEventButtonNumber` of the forward button (mouse button 5).
pub const FORWARD_BUTTON: i64 = 4;

/// How many pixels one line scrolls in AppKit (`NSScrollView`'s default
/// line scroll): the pixel delta of N lines is N times this.
const PIXELS_PER_LINE: i32 = 10;

/// Mouse state that outlives one event: which side buttons were turned
/// into navigation on the way down, so their release is swallowed too.
#[derive(Debug, Default)]
pub struct MouseEngine {
    back_swiped: bool,
    forward_swiped: bool,
}

impl MouseEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decides one wheel event. Only notched-wheel events from a mouse
    /// change; trackpads, momentum, pixel scrolling and software scrolling
    /// pass.
    pub fn scroll(&mut self, event: &ScrollEvent, cx: &MouseContext<'_>) -> ScrollOutcome {
        if event.continuous || event.scroll_phase != 0 || event.momentum_phase != 0 {
            return ScrollOutcome::Pass;
        }
        let [vertical, horizontal] = axis_signs(event);
        if vertical == 0 && horizontal == 0 {
            return ScrollOutcome::Pass;
        }
        let profile = cx.profile.normalized();
        if let Some(keys) = zoom_keys(event, cx, &profile, vertical) {
            return ScrollOutcome::Keys(keys);
        }
        let reverse = profile.reverse_wheel && event.inverted;
        if profile.linear_wheel {
            let lines = i32::from(profile.lines_per_notch);
            let signs = if reverse {
                [-vertical, -horizontal]
            } else {
                [vertical, horizontal]
            };
            ScrollOutcome::Rewrite {
                line: signs.map(i64::from),
                fixed: signs.map(|sign| f64::from(sign * lines)),
                point: signs.map(|sign| i64::from(sign * lines * PIXELS_PER_LINE)),
            }
        } else if reverse {
            ScrollOutcome::Rewrite {
                line: event.line.map(i64::saturating_neg),
                fixed: event.fixed.map(|delta| -delta),
                point: event.point.map(i64::saturating_neg),
            }
        } else {
            ScrollOutcome::Pass
        }
    }

    /// Decides one button 4 or 5 event.
    pub fn button(&mut self, event: &ButtonEvent, cx: &MouseContext<'_>) -> ButtonOutcome {
        let (swiped, back) = match event.button {
            BACK_BUTTON => (&mut self.back_swiped, true),
            FORWARD_BUTTON => (&mut self.forward_swiped, false),
            _ => return ButtonOutcome::Pass,
        };
        if event.down {
            *swiped = cx.profile.side_buttons
                && cx.app.class != AppClass::Passthrough
                && !cx.app.traits.native_side_buttons;
            if *swiped {
                ButtonOutcome::Swipe { back }
            } else {
                ButtonOutcome::Pass
            }
        } else if std::mem::take(swiped) {
            ButtonOutcome::Drop
        } else {
            ButtonOutcome::Pass
        }
    }
}

/// The direction of each axis, vertical then horizontal: -1, 0 or 1, from
/// the line delta, else the fixed-point delta, else the pixel delta (a slow
/// notch can carry less than a line).
fn axis_signs(event: &ScrollEvent) -> [i32; 2] {
    let [line_v, line_h] = event.line;
    let [fixed_v, fixed_h] = event.fixed;
    let [point_v, point_h] = event.point;
    [
        sign_of(line_v, fixed_v, point_v),
        sign_of(line_h, fixed_h, point_h),
    ]
}

/// One axis's direction from its three deltas, first non-zero wins.
fn sign_of(line: i64, fixed: f64, point: i64) -> i32 {
    if line != 0 {
        return signum(line);
    }
    if fixed > 0.0 {
        return 1;
    }
    if fixed < 0.0 {
        return -1;
    }
    signum(point)
}

/// -1, 0 or 1, small enough to scale by lines without overflow.
fn signum(value: i64) -> i32 {
    match value.cmp(&0) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// Command+= for a notch away from the user, Command+- for one toward,
/// while Windows Ctrl alone is held; `None` when this wheel event is not a
/// zoom. Office zooms on Command+wheel itself (which is what Ctrl+wheel
/// arrives as there), and passthrough apps get Ctrl+wheel untouched.
fn zoom_keys(
    event: &ScrollEvent,
    cx: &MouseContext<'_>,
    profile: &MouseProfile,
    vertical: i32,
) -> Option<Vec<Synth>> {
    if !profile.ctrl_wheel_zoom
        || cx.app.class == AppClass::Passthrough
        || cx.app.traits.office
        || vertical == 0
    {
        return None;
    }
    let mods = hidmap::decode(event.flags, cx.hid_profile);
    if !mods.ctrl || mods.win || mods.alt || mods.shift || mods.foreign {
        return None;
    }
    // The physical direction: natural scrolling inverted the delta.
    let away = if event.inverted {
        vertical < 0
    } else {
        vertical > 0
    };
    let key = if away { kc::EQUAL } else { kc::MINUS };
    Some(Chord::new(key, Mods::COMMAND).stroke().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{AppProfile, AppTraits};
    use crate::keys::Flags;

    /// Windows Ctrl as it arrives under the Windows profile.
    const CTRL: Flags = Flags::COMMAND.union(Flags::DEV_RCMD);

    fn app(class: AppClass) -> AppProfile {
        AppProfile {
            class,
            traits: AppTraits::default(),
        }
    }

    fn cx<'a>(profile: &'a MouseProfile, app: AppProfile) -> MouseContext<'a> {
        MouseContext {
            profile,
            app,
            hid_profile: hidmap::profile_for(app.class, true),
        }
    }

    /// One notch of a wheel as macOS sends it, accelerated: up (away from
    /// the user) is positive before natural scrolling inverts it.
    fn notch(lines: i64, inverted: bool) -> ScrollEvent {
        let sign = if inverted { -lines } else { lines };
        ScrollEvent {
            continuous: false,
            scroll_phase: 0,
            momentum_phase: 0,
            line: [sign, 0],
            fixed: [sign as f64 * 1.5, 0.0],
            point: [sign * 12, 0],
            inverted,
            sender: 0x1_0000_6091,
            flags: Flags::NON_COALESCED,
        }
    }

    fn windows() -> MouseProfile {
        MouseProfile::WINDOWS
    }

    /// Trackpads, the Magic Mouse, momentum and software scrolling are
    /// never changed, whatever the profile says.
    #[test]
    fn only_notched_wheel_events_change() {
        let profile = windows();
        let cx = cx(&profile, app(AppClass::Standard));
        let mut engine = MouseEngine::new();
        let base = notch(1, true);
        for event in [
            ScrollEvent {
                continuous: true,
                ..base
            },
            ScrollEvent {
                scroll_phase: 2,
                ..base
            },
            ScrollEvent {
                momentum_phase: 1,
                ..base
            },
            ScrollEvent {
                line: [0, 0],
                fixed: [0.0, 0.0],
                point: [0, 0],
                ..base
            },
        ] {
            assert_eq!(engine.scroll(&event, &cx), ScrollOutcome::Pass, "{event:?}");
        }
    }

    /// With linear scrolling each notch scrolls N lines, whatever macOS's
    /// acceleration made of it: line ±1, fixed ±N, pixels ±10·N, and the
    /// direction is flipped back when natural scrolling inverted it.
    #[test]
    fn a_linear_notch_scrolls_n_lines_in_windows_direction() {
        let mut profile = windows();
        profile.lines_per_notch = 5;
        let cx = cx(&profile, app(AppClass::Standard));
        let mut engine = MouseEngine::new();
        let down_rewrite = ScrollOutcome::Rewrite {
            line: [-1, 0],
            fixed: [-5.0, 0.0],
            point: [-50, 0],
        };
        // Toward the user, inverted by natural scrolling: scrolls down.
        assert_eq!(engine.scroll(&notch(-4, true), &cx), down_rewrite);
        // Toward the user, not inverted: also down.
        assert_eq!(engine.scroll(&notch(-4, false), &cx), down_rewrite);
        // Away from the user: up.
        assert_eq!(
            engine.scroll(&notch(1, true), &cx),
            ScrollOutcome::Rewrite {
                line: [1, 0],
                fixed: [5.0, 0.0],
                point: [50, 0],
            }
        );
    }

    /// Linear scrolling without the Windows direction keeps the system's
    /// direction.
    #[test]
    fn linear_without_reverse_keeps_the_systems_direction() {
        let mut profile = windows();
        profile.reverse_wheel = false;
        let cx = cx(&profile, app(AppClass::Standard));
        let mut engine = MouseEngine::new();
        assert_eq!(
            engine.scroll(&notch(2, true), &cx),
            ScrollOutcome::Rewrite {
                line: [-1, 0],
                fixed: [-3.0, 0.0],
                point: [-30, 0],
            }
        );
    }

    /// Each axis is scaled on its own; an axis that did not move stays at
    /// zero, and a notch with less than a line takes its direction from the
    /// fixed-point delta.
    #[test]
    fn each_axis_is_scaled_alone_and_zero_axes_stay_zero() {
        let profile = windows();
        let cx = cx(&profile, app(AppClass::Standard));
        let mut engine = MouseEngine::new();
        let tilt = ScrollEvent {
            line: [0, 0],
            fixed: [0.0, -0.25],
            point: [0, -2],
            inverted: false,
            ..notch(1, false)
        };
        assert_eq!(
            engine.scroll(&tilt, &cx),
            ScrollOutcome::Rewrite {
                line: [0, -1],
                fixed: [0.0, -3.0],
                point: [0, -30],
            }
        );
        let pixels_only = ScrollEvent {
            line: [0, 0],
            fixed: [0.0, 0.0],
            point: [7, 0],
            ..notch(1, false)
        };
        assert_eq!(
            engine.scroll(&pixels_only, &cx),
            ScrollOutcome::Rewrite {
                line: [1, 0],
                fixed: [3.0, 0.0],
                point: [30, 0],
            }
        );
    }

    /// The Windows direction alone flips every delta of both axes and
    /// keeps macOS's distance.
    #[test]
    fn reverse_alone_negates_every_delta() {
        let mut profile = windows();
        profile.linear_wheel = false;
        let cx = cx(&profile, app(AppClass::Standard));
        let mut engine = MouseEngine::new();
        let event = ScrollEvent {
            line: [-3, 1],
            fixed: [-3.5, 1.25],
            point: [-35, 12],
            ..notch(1, true)
        };
        assert_eq!(
            engine.scroll(&event, &cx),
            ScrollOutcome::Rewrite {
                line: [3, -1],
                fixed: [3.5, -1.25],
                point: [35, -12],
            }
        );
        // Not inverted: already the Windows direction.
        assert_eq!(engine.scroll(&notch(-3, false), &cx), ScrollOutcome::Pass);
    }

    /// The Mac preset changes nothing.
    #[test]
    fn the_mac_profile_passes_every_wheel_event() {
        let profile = MouseProfile::MAC;
        let cx = cx(&profile, app(AppClass::Standard));
        let mut engine = MouseEngine::new();
        assert_eq!(engine.scroll(&notch(-2, true), &cx), ScrollOutcome::Pass);
        let ctrl = ScrollEvent {
            flags: CTRL,
            ..notch(1, false)
        };
        assert_eq!(engine.scroll(&ctrl, &cx), ScrollOutcome::Pass);
    }

    /// A lines-per-notch value out of range is clamped, not trusted.
    #[test]
    fn lines_per_notch_is_clamped_into_range() {
        let mut profile = windows();
        profile.lines_per_notch = 0;
        let cx = cx(&profile, app(AppClass::Standard));
        let mut engine = MouseEngine::new();
        assert_eq!(
            engine.scroll(&notch(1, false), &cx),
            ScrollOutcome::Rewrite {
                line: [1, 0],
                fixed: [1.0, 0.0],
                point: [10, 0],
            }
        );
    }

    /// Ctrl+wheel zooms one step per notch, by the physical direction:
    /// away from the user zooms in, whether or not natural scrolling
    /// inverted the event.
    #[test]
    fn ctrl_wheel_zooms_by_the_physical_direction() {
        let profile = windows();
        let mut engine = MouseEngine::new();
        let zoom_in = ScrollOutcome::Keys(Chord::new(kc::EQUAL, Mods::COMMAND).stroke().to_vec());
        let zoom_out = ScrollOutcome::Keys(Chord::new(kc::MINUS, Mods::COMMAND).stroke().to_vec());
        let standard = cx(&profile, app(AppClass::Standard));
        let with = |event: ScrollEvent, flags| ScrollEvent { flags, ..event };
        assert_eq!(
            engine.scroll(&with(notch(1, true), CTRL), &standard),
            zoom_in
        );
        assert_eq!(
            engine.scroll(&with(notch(1, false), CTRL), &standard),
            zoom_in
        );
        assert_eq!(
            engine.scroll(&with(notch(-3, true), CTRL), &standard),
            zoom_out
        );
        // In a terminal Ctrl arrives as Control.
        let terminal = cx(&profile, app(AppClass::Terminal));
        let control = Flags::CONTROL | Flags::DEV_RCTL;
        assert_eq!(
            engine.scroll(&with(notch(-1, false), control), &terminal),
            zoom_out
        );
    }

    /// No zoom where it would be wrong: Office zooms on Command+wheel
    /// itself, passthrough apps get Ctrl+wheel as it is, a Mac keyboard's
    /// Command is not Ctrl, Ctrl+Shift is not Ctrl, a horizontal wheel is
    /// not a zoom, and the setting can be off. Such events still get the
    /// wheel settings.
    #[test]
    fn ctrl_wheel_zoom_applies_only_to_ctrl_alone_on_the_vertical_wheel() {
        let profile = windows();
        let mut engine = MouseEngine::new();
        let scrolled_up = ScrollOutcome::Rewrite {
            line: [1, 0],
            fixed: [3.0, 0.0],
            point: [30, 0],
        };
        let up_with = |flags| ScrollEvent {
            flags,
            ..notch(1, false)
        };

        let office = AppProfile {
            class: AppClass::Standard,
            traits: AppTraits {
                office: true,
                ..AppTraits::default()
            },
        };
        assert_eq!(
            engine.scroll(&up_with(CTRL), &cx(&profile, office)),
            scrolled_up
        );

        let passthrough = cx(&profile, app(AppClass::Passthrough));
        let vm_ctrl = Flags::CONTROL | Flags::DEV_LCTL;
        assert_eq!(engine.scroll(&up_with(vm_ctrl), &passthrough), scrolled_up);

        let standard = cx(&profile, app(AppClass::Standard));
        let mac_command = Flags::COMMAND | Flags::DEV_LCMD;
        assert_eq!(engine.scroll(&up_with(mac_command), &standard), scrolled_up);
        let ctrl_shift = CTRL | Flags::SHIFT | Flags::DEV_LSHIFT;
        assert_eq!(engine.scroll(&up_with(ctrl_shift), &standard), scrolled_up);

        let tilt = ScrollEvent {
            line: [0, 1],
            fixed: [0.0, 1.0],
            point: [0, 10],
            flags: CTRL,
            ..notch(1, false)
        };
        assert_eq!(
            engine.scroll(&tilt, &standard),
            ScrollOutcome::Rewrite {
                line: [0, 1],
                fixed: [0.0, 3.0],
                point: [0, 30],
            }
        );

        let mut off = windows();
        off.ctrl_wheel_zoom = false;
        assert_eq!(
            engine.scroll(&up_with(CTRL), &cx(&off, app(AppClass::Standard))),
            scrolled_up
        );
    }

    /// Passthrough apps still get the wheel's direction and distance: those
    /// are mouse settings, not keyboard translation.
    #[test]
    fn passthrough_apps_still_get_wheel_direction_and_distance() {
        let profile = windows();
        let cx = cx(&profile, app(AppClass::Passthrough));
        let mut engine = MouseEngine::new();
        assert_eq!(
            engine.scroll(&notch(-2, true), &cx),
            ScrollOutcome::Rewrite {
                line: [-1, 0],
                fixed: [-3.0, 0.0],
                point: [-30, 0],
            }
        );
    }

    fn press(button: i64, down: bool) -> ButtonEvent {
        ButtonEvent {
            button,
            down,
            flags: Flags::NONE,
        }
    }

    /// Back and forward become swipes in apps that ignore them, and the
    /// matching release is swallowed.
    #[test]
    fn side_buttons_swipe_and_swallow_their_release() {
        let profile = windows();
        let cx = cx(&profile, app(AppClass::Standard));
        let mut engine = MouseEngine::new();
        assert_eq!(
            engine.button(&press(BACK_BUTTON, true), &cx),
            ButtonOutcome::Swipe { back: true }
        );
        assert_eq!(
            engine.button(&press(FORWARD_BUTTON, true), &cx),
            ButtonOutcome::Swipe { back: false }
        );
        assert_eq!(
            engine.button(&press(BACK_BUTTON, false), &cx),
            ButtonOutcome::Drop
        );
        assert_eq!(
            engine.button(&press(FORWARD_BUTTON, false), &cx),
            ButtonOutcome::Drop
        );
        // A release without a swallowed press passes.
        assert_eq!(
            engine.button(&press(BACK_BUTTON, false), &cx),
            ButtonOutcome::Pass
        );
    }

    /// Apps that handle the side buttons natively (Chromium, VS Code), VMs,
    /// and a mouse with the setting off get them untouched, and the release
    /// matches the press.
    #[test]
    fn side_buttons_pass_where_they_already_work() {
        let profile = windows();
        let chrome = AppProfile {
            class: AppClass::Standard,
            traits: AppTraits {
                native_side_buttons: true,
                browser: true,
                ..AppTraits::default()
            },
        };
        let mut off = windows();
        off.side_buttons = false;
        let contexts = [
            cx(&profile, chrome),
            cx(&profile, app(AppClass::Passthrough)),
            cx(&off, app(AppClass::Standard)),
        ];
        for context in contexts {
            let mut engine = MouseEngine::new();
            for event in [press(BACK_BUTTON, true), press(BACK_BUTTON, false)] {
                assert_eq!(engine.button(&event, &context), ButtonOutcome::Pass);
            }
        }
    }

    /// The release follows the press, even if the front app changed in
    /// between: a swallowed press never leaks its release, and a passed
    /// press never loses it.
    #[test]
    fn a_side_button_release_follows_its_press() {
        let profile = windows();
        let standard = cx(&profile, app(AppClass::Standard));
        let vm = cx(&profile, app(AppClass::Passthrough));
        let mut engine = MouseEngine::new();
        engine.button(&press(BACK_BUTTON, true), &standard);
        assert_eq!(
            engine.button(&press(BACK_BUTTON, false), &vm),
            ButtonOutcome::Drop
        );
        engine.button(&press(FORWARD_BUTTON, true), &vm);
        assert_eq!(
            engine.button(&press(FORWARD_BUTTON, false), &standard),
            ButtonOutcome::Pass
        );
    }

    /// Only buttons 4 and 5 are side buttons.
    #[test]
    fn other_buttons_pass() {
        let profile = windows();
        let cx = cx(&profile, app(AppClass::Standard));
        let mut engine = MouseEngine::new();
        for button in [0, 1, 2, 5, -1] {
            assert_eq!(
                engine.button(&press(button, true), &cx),
                ButtonOutcome::Pass
            );
            assert_eq!(
                engine.button(&press(button, false), &cx),
                ButtonOutcome::Pass
            );
        }
    }
}
