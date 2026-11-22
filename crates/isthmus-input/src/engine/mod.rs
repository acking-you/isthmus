//! The event tap's decisions, with no OS in sight.
//!
//! The platform turns each `CGEvent` into one of the plain values here,
//! asks the engine what to do, and carries the answer out. Everything that
//! can be decided without the OS is decided here, so it is tested here: the
//! shortcut table, the sticky key-up pairing, the Alt+Tab switcher session,
//! the Win-tap detector, wheel arithmetic and side buttons.
//!
//! The engine runs on the tap thread for every key and wheel event, so the
//! pass-through path allocates nothing and takes no lock.

pub mod keyboard;
pub mod mouse;
mod rules;

use crate::context::AppProfile;
use crate::hidmap::HidProfile;
use crate::keys::{Flags, KeyCode, Mods, kc};
use crate::settings::{MouseProfile, ShortcutGroups};

/// What happened to a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPhase {
    Down,
    Up,
    /// A modifier went down or up (`kCGEventFlagsChanged`); `flags` is the
    /// state after it.
    FlagsChanged,
}

/// A keyboard event as the tap sees it, after the HID mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    pub phase: KeyPhase,
    pub keycode: KeyCode,
    pub flags: Flags,
    /// `kCGKeyboardEventAutorepeat`.
    pub autorepeat: bool,
    /// The event's timestamp, nanoseconds since boot.
    pub timestamp_ns: u64,
}

/// A shortcut as macOS names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub keycode: KeyCode,
    pub mods: Mods,
    /// Send with the Fn flag, as macOS's tiling and Show Desktop hotkeys are
    /// stored. It changes nothing on an arrow, which hardware always sends
    /// with Fn; a hotkey on Fn and an arrow is the Fn layer's key instead
    /// (see [`Chord::on_fn_layer`]).
    pub fn_key: bool,
}

impl Chord {
    pub const fn new(keycode: KeyCode, mods: Mods) -> Self {
        Chord {
            keycode,
            mods,
            fn_key: false,
        }
    }

    pub const fn with_fn(self) -> Self {
        Chord {
            fn_key: true,
            ..self
        }
    }

    /// The chord an Apple keyboard sends with Fn held. Fn turns an arrow
    /// into Home, End, Page Up or Page Down, so Fn+Ctrl+Left, the stored
    /// form of "Tile Left", arrives as Ctrl+Home with Fn and no NumericPad
    /// (which is why Ctrl+Home tiles windows in Excel). The arrow with the
    /// Fn flag would not do: hardware sends every arrow with Fn, so that is
    /// byte for byte a plain Ctrl+Left, "Move left a Space". Only the
    /// keycode tells the Fn layer apart. Any other key just gains the Fn
    /// flag.
    pub const fn on_fn_layer(self) -> Self {
        let keycode = match self.keycode {
            kc::LEFT => kc::HOME,
            kc::RIGHT => kc::END,
            kc::UP => kc::PAGE_UP,
            kc::DOWN => kc::PAGE_DOWN,
            other => other,
        };
        Chord {
            keycode,
            fn_key: true,
            ..self
        }
    }

    /// The flags a hardware event for this chord carries: the modifiers'
    /// left-side bits, plus Fn and NumericPad on arrows and Fn on the
    /// navigation block, as a keyboard sends them.
    pub fn flags(self) -> Flags {
        let mut flags = self.mods.to_flags().union(Flags::NON_COALESCED);
        if self.fn_key || kc::is_fn_navigation(self.keycode) || kc::is_function_key(self.keycode) {
            flags = flags.union(Flags::FN);
        }
        if kc::is_arrow(self.keycode) {
            flags = flags.union(Flags::FN).union(Flags::NUMERIC_PAD);
        }
        flags
    }

    /// Down and up, as two key events.
    ///
    /// Only the key-down carries the chord's modifiers; the key-up carries
    /// none. A posted event updates the HID system's modifier state, so a
    /// key-up that still said Command would leave Command "held" for the
    /// system until a real key cleared it. Shortcuts fire on the key-down.
    pub fn stroke(self) -> [Synth; 2] {
        let flags = self.flags();
        [
            Synth::Key {
                keycode: self.keycode,
                flags,
                down: true,
            },
            Synth::Key {
                keycode: self.keycode,
                flags: flags.non_modifier_bits(),
                down: false,
            },
        ]
    }
}

/// An event Isthmus posts. The platform tags every one so the tap passes
/// it through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Synth {
    Key {
        keycode: KeyCode,
        flags: Flags,
        down: bool,
    },
    /// A `flagsChanged` event for a modifier key, with the flags after it.
    Modifier { keycode: KeyCode, flags: Flags },
}

/// What becomes of the event the tap is holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fate {
    /// Unchanged.
    Pass,
    /// The same event with these flags, or a new key event with this keycode
    /// and these flags when the keycode differs (a changed keycode would keep
    /// the old key's characters).
    Rewrite { keycode: KeyCode, flags: Flags },
    /// Swallowed.
    Drop,
}

/// Side effects that may block (opening an app), done off the tap thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Open, or bring forward, the app with this bundle id.
    OpenApp { bundle_id: &'static str },
    /// Open a new Finder window at the home folder (Win+E).
    OpenHomeInFinder,
}

/// The engine's answer for a key event: post `before`, then the event's
/// fate, then `after`; then run `action`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyOutcome {
    pub before: Vec<Synth>,
    pub fate: Fate,
    pub after: Vec<Synth>,
    pub action: Option<Action>,
}

impl KeyOutcome {
    /// Unchanged, with nothing posted. Allocates nothing.
    pub const PASS: KeyOutcome = KeyOutcome {
        before: Vec::new(),
        fate: Fate::Pass,
        after: Vec::new(),
        action: None,
    };

    pub fn is_pass(&self) -> bool {
        *self == KeyOutcome::PASS
    }
}

/// The macOS hotkeys the Windows shortcuts trigger, as this user has them
/// bound. The platform reads `com.apple.symbolichotkeys` and fills in the
/// macOS defaults for anything not customised; `None` is a hotkey the user
/// switched off, which then does nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemHotkeys {
    /// 32, Ctrl+Up.
    pub mission_control: Option<Chord>,
    /// 36, F11.
    pub show_desktop: Option<Chord>,
    /// 64, Cmd+Space.
    pub spotlight: Option<Chord>,
    /// 60, Ctrl+Space.
    pub previous_input_source: Option<Chord>,
    /// 61, Ctrl+Option+Space.
    pub next_input_source: Option<Chord>,
    /// 79 and 81, Ctrl+Left and Ctrl+Right.
    pub space_left: Option<Chord>,
    pub space_right: Option<Chord>,
    /// 237, Fn+Ctrl+F; 240 and 241, Fn+Ctrl+Left/Right; 239, Fn+Ctrl+R.
    /// Fn+Ctrl+Left/Right are held as what the keyboard sends for them,
    /// Ctrl+Home/End with Fn ([`Chord::on_fn_layer`]), so they differ from
    /// the Space switches on the same arrows.
    pub tile_fill: Option<Chord>,
    pub tile_left: Option<Chord>,
    pub tile_right: Option<Chord>,
    pub tile_restore: Option<Chord>,
}

impl Default for SystemHotkeys {
    /// macOS 15+ defaults (`DefaultShortcutsTable.xml`).
    fn default() -> Self {
        let ctrl = Mods::CONTROL;
        SystemHotkeys {
            mission_control: Some(Chord::new(kc::UP, ctrl)),
            show_desktop: Some(Chord::new(kc::F11, Mods::NONE).with_fn()),
            spotlight: Some(Chord::new(kc::SPACE, Mods::COMMAND)),
            previous_input_source: Some(Chord::new(kc::SPACE, ctrl)),
            next_input_source: Some(Chord::new(kc::SPACE, ctrl.with(Mods::OPTION))),
            space_left: Some(Chord::new(kc::LEFT, ctrl)),
            space_right: Some(Chord::new(kc::RIGHT, ctrl)),
            tile_fill: Some(Chord::new(kc::F, ctrl).on_fn_layer()),
            tile_left: Some(Chord::new(kc::LEFT, ctrl).on_fn_layer()),
            tile_right: Some(Chord::new(kc::RIGHT, ctrl).on_fn_layer()),
            tile_restore: Some(Chord::new(kc::R, ctrl).on_fn_layer()),
        }
    }
}

/// What the keyboard engine knows besides the event.
#[derive(Debug, Clone, Copy)]
pub struct KeyContext<'a> {
    pub app: AppProfile,
    /// The mapping the keyboards have now (see [`crate::hidmap`]).
    pub hid_profile: HidProfile,
    pub groups: &'a ShortcutGroups,
    pub hotkeys: &'a SystemHotkeys,
    /// Shift tapped alone selects the previous input source: the setting
    /// is on and this user has an input method to switch to.
    pub shift_switches_input: bool,
}

/// A wheel event as the tap sees it. Field numbers are `CGEventField`s.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollEvent {
    /// 88: 0 for a notched wheel, 1 for trackpads and pixel scrolling.
    pub continuous: bool,
    /// 99 and 123: non-zero for trackpad gestures and momentum.
    pub scroll_phase: i64,
    pub momentum_phase: i64,
    /// 11 and 12: vertical and horizontal, in lines. Positive is up / left.
    pub line: [i64; 2],
    /// 93 and 94: the same, 16.16 fixed point read as a double.
    pub fixed: [f64; 2],
    /// 96 and 97: pixels.
    pub point: [i64; 2],
    /// 137: natural scrolling already inverted these deltas.
    pub inverted: bool,
    /// 87: the sending HID service's registry id, 0 for software.
    pub sender: u64,
    pub flags: Flags,
}

/// What becomes of a wheel event.
#[derive(Debug, Clone, PartialEq)]
pub enum ScrollOutcome {
    Pass,
    /// Write these deltas, in the order line, fixed, point, with
    /// `continuous` kept at 0.
    Rewrite {
        line: [i64; 2],
        fixed: [f64; 2],
        point: [i64; 2],
    },
    /// Swallow the wheel event and post these keys (Ctrl+wheel zoom).
    Keys(Vec<Synth>),
}

/// Buttons 4 and 5 (`kCGMouseEventButtonNumber` 3 and 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ButtonEvent {
    pub button: i64,
    pub down: bool,
    pub flags: Flags,
}

/// What becomes of a side-button event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonOutcome {
    Pass,
    Drop,
    /// Swallow it and post a navigation swipe, back or forward.
    Swipe {
        back: bool,
    },
}

/// What the mouse engine knows besides the event.
#[derive(Debug, Clone, Copy)]
pub struct MouseContext<'a> {
    /// The profile of the mouse that sent the event (by sender id), or the
    /// defaults when it is not known.
    pub profile: &'a MouseProfile,
    pub app: AppProfile,
    pub hid_profile: HidProfile,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key-down a bound hotkey's stroke posts.
    fn posted(chord: Option<Chord>) -> Synth {
        chord.unwrap().stroke()[0]
    }

    /// Win+Left/Right must tile, not switch Spaces: the default tiling
    /// chords post Ctrl+Home/End with Fn and no NumericPad, never the
    /// arrow that a hardware Ctrl+Left/Right (the Space switches) sends.
    #[test]
    fn default_tiling_never_posts_the_space_switch_chord() {
        let hotkeys = SystemHotkeys::default();
        let ctrl_fn = Mods::CONTROL
            .to_flags()
            .union(Flags::NON_COALESCED)
            .union(Flags::FN);
        assert_eq!(
            posted(hotkeys.tile_left),
            Synth::Key {
                keycode: kc::HOME,
                flags: ctrl_fn,
                down: true,
            }
        );
        assert_eq!(
            posted(hotkeys.tile_right),
            Synth::Key {
                keycode: kc::END,
                flags: ctrl_fn,
                down: true,
            }
        );
        assert_ne!(posted(hotkeys.tile_left), posted(hotkeys.space_left));
        assert_ne!(posted(hotkeys.tile_right), posted(hotkeys.space_right));
    }

    /// Fn turns each arrow into its navigation key, as an Apple keyboard
    /// does; a letter keeps its keycode and gains the Fn flag.
    #[test]
    fn the_fn_layer_of_an_arrow_is_its_navigation_key() {
        let ctrl = Mods::CONTROL;
        for (arrow, layer) in [
            (kc::LEFT, kc::HOME),
            (kc::RIGHT, kc::END),
            (kc::UP, kc::PAGE_UP),
            (kc::DOWN, kc::PAGE_DOWN),
        ] {
            assert_eq!(
                Chord::new(arrow, ctrl).on_fn_layer(),
                Chord::new(layer, ctrl).with_fn()
            );
        }
        assert_eq!(
            Chord::new(kc::F, ctrl).on_fn_layer(),
            Chord::new(kc::F, ctrl).with_fn()
        );
        assert!(
            !Chord::new(kc::HOME, ctrl)
                .flags()
                .contains(Flags::NUMERIC_PAD)
        );
    }
}
