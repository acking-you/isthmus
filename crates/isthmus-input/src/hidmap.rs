//! The per-keyboard HID mapping (`UserKeyMapping`).
//!
//! Remapping modifiers at the HID level, rather than in the event tap, is
//! what makes Ctrl act as Command everywhere a real Command would: in
//! password fields (where Secure Input blinds every event tap), in menus, in
//! apps that track modifier state themselves. Each Windows-style keyboard
//! gets one of three mappings, chosen by the frontmost app's class:
//!
//! | Profile | Windows Ctrl | Win | Alt | Used for |
//! |---|---|---|---|---|
//! | [`HidProfile::Windows`] | Right Command | Right Control | Right Option | standard apps |
//! | [`HidProfile::NativeCtrl`] | Right Control | Right Command | Left Option | terminals, JetBrains |
//! | [`HidProfile::Positions`] | Left Control | Left Command | Left Option | VMs, remote desktops |
//!
//! The right-hand destinations are markers: a keyboard left in Mac style
//! sends left-hand modifiers, so the event tap can tell, from the
//! device-dependent flag bits alone, whether a Command came from a
//! Windows-style Ctrl or from a real Command key. Apps see ordinary
//! modifiers either way; macOS does not distinguish sides for shortcuts.
//! `Positions` keeps the left side because a remote Windows reads Right Alt
//! as AltGr.
//!
//! A keyboard in Mac style gets no mapping at all, in every profile.
//!
//! Everything here is plain arithmetic on usages and flag bits, so the
//! tables in docs/keyboard-and-mouse.md are pinned by tests on any host.

use serde::{Deserialize, Serialize};

use crate::context::AppClass;
use crate::device::Layout;
use crate::keys::usage::{self, HidUsage};
use crate::keys::{Flags, KeyCode, kc};
use crate::settings::KeyboardStyle;

/// The mapping a Windows-style keyboard has while some app is frontmost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HidProfile {
    Windows,
    NativeCtrl,
    Positions,
}

/// One `UserKeyMapping` entry: `HIDKeyboardModifierMappingSrc` →
/// `HIDKeyboardModifierMappingDst`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyMapping {
    pub src: HidUsage,
    pub dst: HidUsage,
}

impl KeyMapping {
    /// Shorthand for the tables below, which read as "source to destination".
    const fn new(src: HidUsage, dst: HidUsage) -> Self {
        KeyMapping { src, dst }
    }
}

/// The profile the keyboards get while an app of `class` is in front.
/// Without a working keyboard tap the Windows profile is unsafe (nothing
/// would stop Ctrl+Q quitting or Ctrl+Tab opening the app switcher), so it
/// falls back to Positions. Terminals and NativeCtrl apps keep Ctrl as
/// Control, which is safe with or without the tap, so they need no fallback.
pub fn profile_for(class: AppClass, keyboard_tap_active: bool) -> HidProfile {
    match class {
        AppClass::Standard if keyboard_tap_active => HidProfile::Windows,
        AppClass::Standard => HidProfile::Positions,
        AppClass::Terminal | AppClass::NativeCtrl => HidProfile::NativeCtrl,
        AppClass::Passthrough => HidProfile::Positions,
    }
}

/// The PC keyboard under the Windows profile: Ctrl to Right Command, Win to
/// Right Control, Alt to Right Option (Right Alt already is).
const PC_WINDOWS: &[KeyMapping] = &[
    KeyMapping::new(usage::LEFT_CONTROL, usage::RIGHT_GUI),
    KeyMapping::new(usage::RIGHT_CONTROL, usage::RIGHT_GUI),
    KeyMapping::new(usage::LEFT_GUI, usage::RIGHT_CONTROL),
    KeyMapping::new(usage::RIGHT_GUI, usage::RIGHT_CONTROL),
    KeyMapping::new(usage::LEFT_ALT, usage::RIGHT_ALT),
];

/// The PC keyboard under NativeCtrl: Ctrl stays Control but moves to the
/// right-hand marker, Win becomes Right Command, Alt stays Left Option.
const PC_NATIVE_CTRL: &[KeyMapping] = &[
    KeyMapping::new(usage::LEFT_CONTROL, usage::RIGHT_CONTROL),
    KeyMapping::new(usage::LEFT_GUI, usage::RIGHT_GUI),
];

/// The Mac keyboard under the Windows profile. Its corner Control is Windows
/// Ctrl, its Option sits where Win does, its Command where Alt does. Apple's
/// own keyboards have no right Control, but many Mac-layout boards (Keychron
/// in Mac mode) do; it is Ctrl too, because left unmapped it would arrive as
/// Right Control, the Win marker, and Right Ctrl+C would read as Win+C.
const MAC_WINDOWS: &[KeyMapping] = &[
    KeyMapping::new(usage::LEFT_CONTROL, usage::RIGHT_GUI),
    KeyMapping::new(usage::RIGHT_CONTROL, usage::RIGHT_GUI),
    KeyMapping::new(usage::LEFT_ALT, usage::RIGHT_CONTROL),
    KeyMapping::new(usage::RIGHT_ALT, usage::RIGHT_CONTROL),
    KeyMapping::new(usage::LEFT_GUI, usage::RIGHT_ALT),
    KeyMapping::new(usage::RIGHT_GUI, usage::RIGHT_ALT),
];

/// The Mac keyboard under NativeCtrl.
const MAC_NATIVE_CTRL: &[KeyMapping] = &[
    KeyMapping::new(usage::LEFT_CONTROL, usage::RIGHT_CONTROL),
    KeyMapping::new(usage::LEFT_ALT, usage::RIGHT_GUI),
    KeyMapping::new(usage::RIGHT_ALT, usage::RIGHT_GUI),
    KeyMapping::new(usage::LEFT_GUI, usage::LEFT_ALT),
    KeyMapping::new(usage::RIGHT_GUI, usage::RIGHT_ALT),
];

/// The Mac keyboard under Positions: Option and Command swap, so the keys
/// next to the space bar read as a PC keyboard's Win and Alt.
const MAC_POSITIONS: &[KeyMapping] = &[
    KeyMapping::new(usage::LEFT_ALT, usage::LEFT_GUI),
    KeyMapping::new(usage::LEFT_GUI, usage::LEFT_ALT),
    KeyMapping::new(usage::RIGHT_ALT, usage::RIGHT_GUI),
    KeyMapping::new(usage::RIGHT_GUI, usage::RIGHT_ALT),
];

/// The mapping for one keyboard: exactly the table in
/// docs/keyboard-and-mouse.md. Mac style, and a PC keyboard under
/// Positions, give an empty list. Identity entries are never listed:
/// [`merge`] drops every managed source before adding these, so a previous
/// profile's entries never survive.
pub fn key_mapping(style: KeyboardStyle, layout: Layout, profile: HidProfile) -> Vec<KeyMapping> {
    let table: &[KeyMapping] = match (style, layout, profile) {
        (KeyboardStyle::Mac, _, _) => &[],
        (KeyboardStyle::Windows, Layout::Pc, HidProfile::Windows) => PC_WINDOWS,
        (KeyboardStyle::Windows, Layout::Pc, HidProfile::NativeCtrl) => PC_NATIVE_CTRL,
        (KeyboardStyle::Windows, Layout::Pc, HidProfile::Positions) => &[],
        (KeyboardStyle::Windows, Layout::Mac, HidProfile::Windows) => MAC_WINDOWS,
        (KeyboardStyle::Windows, Layout::Mac, HidProfile::NativeCtrl) => MAC_NATIVE_CTRL,
        (KeyboardStyle::Windows, Layout::Mac, HidProfile::Positions) => MAC_POSITIONS,
    };
    table.to_vec()
}

/// The sources Isthmus owns in a keyboard's `UserKeyMapping`: the eight
/// modifier usages. Entries for any other source belong to someone else
/// (the user's own `hidutil`) and are kept when Isthmus rewrites the array.
pub fn is_managed_source(src: HidUsage) -> bool {
    (usage::LEFT_CONTROL..=usage::RIGHT_GUI).contains(&src)
}

/// The array to write: `foreign` entries (those whose source Isthmus does not
/// manage) followed by `ours`. Ownership goes by source only, so a user's
/// CapsLock → Control survives although its destination is a modifier.
pub fn merge(current: &[KeyMapping], ours: &[KeyMapping]) -> Vec<KeyMapping> {
    current
        .iter()
        .filter(|entry| !is_managed_source(entry.src))
        .chain(ours)
        .copied()
        .collect()
}

/// The Windows modifiers held in a key event, read back from flags under a
/// profile. A modifier that the profile does not mark (a left Command while
/// the Windows profile is active, say) comes from a Mac-style keyboard and
/// makes `foreign` true: the tap then leaves the event alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct WinMods {
    pub ctrl: bool,
    pub win: bool,
    pub alt: bool,
    pub shift: bool,
    pub foreign: bool,
}

/// One device-independent modifier and its two side bits, as read from an
/// event's flags.
#[derive(Clone, Copy)]
struct Sided {
    held: bool,
    left: bool,
    right: bool,
}

impl Sided {
    fn read(flags: Flags, mask: Flags, left: Flags, right: Flags) -> Self {
        Sided {
            held: flags.contains(mask),
            left: flags.intersects(left),
            right: flags.intersects(right),
        }
    }

    /// Held, and marked as a Windows-style keyboard's key (the right side).
    fn marked(self) -> bool {
        self.held && self.right
    }

    /// Held, and not provably the marked key: a left-side bit, or no side
    /// bit at all (software that posts bare flags), which could be anything.
    fn unmarked(self) -> bool {
        self.held && (self.left || !self.right)
    }
}

/// Decodes `flags` under `profile`. Under [`HidProfile::Positions`] every
/// modifier counts as its Windows key by position (left or right).
///
/// Under the other two profiles a modifier the profile moves must carry the
/// right-hand side bit; the same modifier with the left-hand bit, or with no
/// side bit at all, cannot be told apart from a Mac-style keyboard's and
/// makes the event `foreign`. Option is the exception under NativeCtrl,
/// where Alt stays on the left and both sides read as Alt.
pub fn decode(flags: Flags, profile: HidProfile) -> WinMods {
    let command = Sided::read(flags, Flags::COMMAND, Flags::DEV_LCMD, Flags::DEV_RCMD);
    let control = Sided::read(flags, Flags::CONTROL, Flags::DEV_LCTL, Flags::DEV_RCTL);
    let option = Sided::read(flags, Flags::OPTION, Flags::DEV_LALT, Flags::DEV_RALT);
    let shift = flags.contains(Flags::SHIFT);
    match profile {
        HidProfile::Windows => WinMods {
            ctrl: command.marked(),
            win: control.marked(),
            alt: option.marked(),
            shift,
            foreign: command.unmarked() || control.unmarked() || option.unmarked(),
        },
        HidProfile::NativeCtrl => WinMods {
            ctrl: control.marked(),
            win: command.marked(),
            alt: option.held,
            shift,
            foreign: control.unmarked() || command.unmarked(),
        },
        HidProfile::Positions => WinMods {
            ctrl: control.held,
            win: command.held,
            alt: option.held,
            shift,
            foreign: false,
        },
    }
}

/// The keycode of the modifier key a Windows key arrives as under `profile`
/// (the `flagsChanged` keycode), for the tap's Win-tap and Alt+Tab tracking
/// and for the watchdog that reads a physical key's state.
///
/// Every Windows key has one in every profile: the Windows and NativeCtrl
/// profiles move both sides of a key to the same marker, and Positions
/// leaves the main key on the left. (A PC keyboard's second, right-hand Win
/// or Alt under Positions arrives as the right-hand keycode instead.)
/// Shift is never moved and has no [`WinKey`].
pub fn arrival_keycode(key: WinKey, profile: HidProfile) -> Option<KeyCode> {
    let code = match (profile, key) {
        (HidProfile::Windows, WinKey::Ctrl) => kc::RIGHT_COMMAND,
        (HidProfile::Windows, WinKey::Win) => kc::RIGHT_CONTROL,
        (HidProfile::Windows, WinKey::Alt) => kc::RIGHT_OPTION,
        (HidProfile::NativeCtrl, WinKey::Ctrl) => kc::RIGHT_CONTROL,
        (HidProfile::NativeCtrl, WinKey::Win) => kc::RIGHT_COMMAND,
        (HidProfile::NativeCtrl, WinKey::Alt) => kc::OPTION,
        (HidProfile::Positions, WinKey::Ctrl) => kc::CONTROL,
        (HidProfile::Positions, WinKey::Win) => kc::COMMAND,
        (HidProfile::Positions, WinKey::Alt) => kc::OPTION,
    };
    Some(code)
}

/// A Windows modifier key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WinKey {
    Ctrl,
    Win,
    Alt,
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_PROFILES: [HidProfile; 3] = [
        HidProfile::Windows,
        HidProfile::NativeCtrl,
        HidProfile::Positions,
    ];

    /// Writes a table the way docs/keyboard-and-mouse.md does: `(src, dst)`
    /// as the low byte of page-7 usages.
    fn table(pairs: &[(u64, u64)]) -> Vec<KeyMapping> {
        pairs
            .iter()
            .map(|&(src, dst)| KeyMapping {
                src: 0x7_0000_0000 | src,
                dst: 0x7_0000_0000 | dst,
            })
            .collect()
    }

    /// Standard apps get the Windows profile only while the keyboard tap can
    /// guard Ctrl+Q and Ctrl+Tab; every other class has one fixed profile.
    #[test]
    fn the_profile_follows_the_app_class_and_the_tap() {
        assert_eq!(profile_for(AppClass::Standard, true), HidProfile::Windows);
        assert_eq!(
            profile_for(AppClass::Standard, false),
            HidProfile::Positions
        );
        for tap in [true, false] {
            assert_eq!(profile_for(AppClass::Terminal, tap), HidProfile::NativeCtrl);
            assert_eq!(
                profile_for(AppClass::NativeCtrl, tap),
                HidProfile::NativeCtrl
            );
            assert_eq!(
                profile_for(AppClass::Passthrough, tap),
                HidProfile::Positions
            );
        }
    }

    /// The six arrays are exactly the documented ones, in order, so the
    /// side-bit markers the tap decodes are the ones the HID layer writes.
    #[test]
    fn windows_style_keyboards_get_exactly_the_documented_arrays() {
        let windows = KeyboardStyle::Windows;
        let cases = [
            (
                Layout::Pc,
                HidProfile::Windows,
                table(&[
                    (0xE0, 0xE7),
                    (0xE4, 0xE7),
                    (0xE3, 0xE4),
                    (0xE7, 0xE4),
                    (0xE2, 0xE6),
                ]),
            ),
            (
                Layout::Pc,
                HidProfile::NativeCtrl,
                table(&[(0xE0, 0xE4), (0xE3, 0xE7)]),
            ),
            (Layout::Pc, HidProfile::Positions, Vec::new()),
            (
                Layout::Mac,
                HidProfile::Windows,
                table(&[
                    (0xE0, 0xE7),
                    (0xE4, 0xE7),
                    (0xE2, 0xE4),
                    (0xE6, 0xE4),
                    (0xE3, 0xE6),
                    (0xE7, 0xE6),
                ]),
            ),
            (
                Layout::Mac,
                HidProfile::NativeCtrl,
                table(&[
                    (0xE0, 0xE4),
                    (0xE2, 0xE7),
                    (0xE6, 0xE7),
                    (0xE3, 0xE2),
                    (0xE7, 0xE6),
                ]),
            ),
            (
                Layout::Mac,
                HidProfile::Positions,
                table(&[(0xE2, 0xE3), (0xE3, 0xE2), (0xE6, 0xE7), (0xE7, 0xE6)]),
            ),
        ];
        for (layout, profile, expected) in cases {
            assert_eq!(
                key_mapping(windows, layout, profile),
                expected,
                "{layout:?} {profile:?}"
            );
        }
    }

    /// The flags a held modifier key of this usage sets: its
    /// device-independent bit and its side bit.
    fn held(usage: HidUsage) -> Flags {
        match usage {
            usage::LEFT_CONTROL => Flags::CONTROL | Flags::DEV_LCTL,
            usage::RIGHT_CONTROL => Flags::CONTROL | Flags::DEV_RCTL,
            usage::LEFT_ALT => Flags::OPTION | Flags::DEV_LALT,
            usage::RIGHT_ALT => Flags::OPTION | Flags::DEV_RALT,
            usage::LEFT_GUI => Flags::COMMAND | Flags::DEV_LCMD,
            usage::RIGHT_GUI => Flags::COMMAND | Flags::DEV_RCMD,
            other => panic!("not a modifier this test moves: {other:#x}"),
        }
    }

    /// Either Control key is Windows Ctrl on both layouts and in every
    /// profile: a right Control (which Mac-layout boards such as a Keychron
    /// in Mac mode have) never arrives as the Win marker, so Right Ctrl+C is
    /// never read as Win+C.
    #[test]
    fn either_control_key_reads_as_windows_ctrl_on_both_layouts() {
        for layout in [Layout::Pc, Layout::Mac] {
            for profile in ALL_PROFILES {
                let table = key_mapping(KeyboardStyle::Windows, layout, profile);
                for src in [usage::LEFT_CONTROL, usage::RIGHT_CONTROL] {
                    let arrives = table.iter().find(|m| m.src == src).map_or(src, |m| m.dst);
                    let mods = decode(held(arrives), profile);
                    assert!(
                        mods.ctrl && !mods.win && !mods.foreign,
                        "{layout:?} {profile:?} {src:#x} arrives as {arrives:#x}: {mods:?}"
                    );
                }
            }
        }
    }

    /// A Mac-style keyboard is never touched, whatever its layout and the
    /// front app.
    #[test]
    fn mac_style_keyboards_get_no_mapping_in_any_profile() {
        for layout in [Layout::Pc, Layout::Mac] {
            for profile in ALL_PROFILES {
                assert!(key_mapping(KeyboardStyle::Mac, layout, profile).is_empty());
            }
        }
    }

    /// Every entry Isthmus writes has a managed source and maps a modifier
    /// to a different modifier: no identity entries, no foreign sources that
    /// a later merge would fail to clean up.
    #[test]
    fn every_written_entry_is_a_managed_non_identity_modifier_move() {
        for layout in [Layout::Pc, Layout::Mac] {
            for profile in ALL_PROFILES {
                for entry in key_mapping(KeyboardStyle::Windows, layout, profile) {
                    assert!(is_managed_source(entry.src), "{entry:?}");
                    assert!(is_managed_source(entry.dst), "{entry:?}");
                    assert_ne!(entry.src, entry.dst, "{entry:?}");
                }
            }
        }
    }

    /// Exactly the eight page-7 modifiers are Isthmus's; neighbouring
    /// usages, other pages and Apple's Fn are not.
    #[test]
    fn only_the_eight_modifier_usages_are_managed() {
        for low in 0xE0..=0xE7 {
            assert!(is_managed_source(0x7_0000_0000 | low), "{low:#x}");
        }
        for other in [
            usage::CAPS_LOCK,
            usage::APPLICATION,
            usage::APPLE_FN,
            usage::APPLE_FN_ALT,
            0x7_0000_00DF,
            0x7_0000_00E8,
            0x0000_00E0,
            0xC_0000_00E0,
        ] {
            assert!(!is_managed_source(other), "{other:#x}");
        }
    }

    /// Rewriting the array keeps the user's own entries in their order,
    /// CapsLock → Control included (its destination is a modifier, its
    /// source is not), and replaces every entry of a previous profile.
    #[test]
    fn merging_keeps_the_users_own_capslock_mapping_and_drops_old_profiles() {
        let caps_to_control = KeyMapping {
            src: usage::CAPS_LOCK,
            dst: usage::LEFT_CONTROL,
        };
        let menu_to_gui = KeyMapping {
            src: usage::APPLICATION,
            dst: usage::RIGHT_GUI,
        };
        let mut current = vec![caps_to_control];
        current.extend(key_mapping(
            KeyboardStyle::Windows,
            Layout::Pc,
            HidProfile::Windows,
        ));
        current.push(menu_to_gui);
        let ours = key_mapping(KeyboardStyle::Windows, Layout::Pc, HidProfile::NativeCtrl);

        let merged = merge(&current, &ours);

        let mut expected = vec![caps_to_control, menu_to_gui];
        expected.extend(ours.iter().copied());
        assert_eq!(merged, expected);
    }

    /// A user's own swap of two modifiers has a managed source, so Isthmus
    /// replaces it: only one owner may decide where the modifiers are.
    #[test]
    fn merging_replaces_a_users_own_modifier_swap() {
        let users_swap = KeyMapping {
            src: usage::LEFT_ALT,
            dst: usage::LEFT_GUI,
        };
        assert_eq!(merge(&[users_swap], &[]), Vec::new());
    }

    /// Switching a keyboard back to Mac style writes an array without any
    /// Isthmus entry, but still with the user's own.
    #[test]
    fn merging_nothing_of_ours_leaves_only_the_users_entries() {
        let caps = KeyMapping {
            src: usage::CAPS_LOCK,
            dst: usage::LEFT_CONTROL,
        };
        let mut current = vec![caps];
        current.extend(key_mapping(
            KeyboardStyle::Windows,
            Layout::Mac,
            HidProfile::Windows,
        ));
        assert_eq!(merge(&current, &[]), vec![caps]);
    }

    fn mods(ctrl: bool, win: bool, alt: bool, shift: bool, foreign: bool) -> WinMods {
        WinMods {
            ctrl,
            win,
            alt,
            shift,
            foreign,
        }
    }

    /// Under the Windows profile the right-hand markers are Ctrl, Win and
    /// Alt, and any left-hand Command, Control or Option is a Mac-style
    /// keyboard's.
    #[test]
    fn the_windows_profile_decodes_right_hand_markers_and_flags_left_hand_ones() {
        let p = HidProfile::Windows;
        let cases = [
            (Flags::NONE, mods(false, false, false, false, false)),
            (
                Flags::COMMAND | Flags::DEV_RCMD,
                mods(true, false, false, false, false),
            ),
            (
                Flags::CONTROL | Flags::DEV_RCTL,
                mods(false, true, false, false, false),
            ),
            (
                Flags::OPTION | Flags::DEV_RALT,
                mods(false, false, true, false, false),
            ),
            (
                Flags::SHIFT | Flags::DEV_LSHIFT,
                mods(false, false, false, true, false),
            ),
            (
                Flags::SHIFT | Flags::DEV_RSHIFT,
                mods(false, false, false, true, false),
            ),
            (
                Flags::COMMAND | Flags::DEV_LCMD,
                mods(false, false, false, false, true),
            ),
            (
                Flags::CONTROL | Flags::DEV_LCTL,
                mods(false, false, false, false, true),
            ),
            (
                Flags::OPTION | Flags::DEV_LALT,
                mods(false, false, false, false, true),
            ),
            (
                Flags::COMMAND | Flags::DEV_RCMD | Flags::DEV_LCMD,
                mods(true, false, false, false, true),
            ),
            (
                Flags::COMMAND | Flags::DEV_RCMD | Flags::OPTION | Flags::DEV_LALT,
                mods(true, false, false, false, true),
            ),
            (
                Flags::COMMAND
                    | Flags::DEV_RCMD
                    | Flags::CONTROL
                    | Flags::DEV_RCTL
                    | Flags::OPTION
                    | Flags::DEV_RALT
                    | Flags::SHIFT
                    | Flags::DEV_LSHIFT,
                mods(true, true, true, true, false),
            ),
        ];
        for (flags, expected) in cases {
            assert_eq!(decode(flags, p), expected, "{flags:?}");
        }
    }

    /// Under NativeCtrl, Ctrl and Win swap markers, Option is Alt on either
    /// side, and a left Control or Command is foreign.
    #[test]
    fn the_native_ctrl_profile_decodes_its_markers_and_either_option() {
        let p = HidProfile::NativeCtrl;
        let cases = [
            (
                Flags::CONTROL | Flags::DEV_RCTL,
                mods(true, false, false, false, false),
            ),
            (
                Flags::COMMAND | Flags::DEV_RCMD,
                mods(false, true, false, false, false),
            ),
            (
                Flags::OPTION | Flags::DEV_LALT,
                mods(false, false, true, false, false),
            ),
            (
                Flags::OPTION | Flags::DEV_RALT,
                mods(false, false, true, false, false),
            ),
            (Flags::OPTION, mods(false, false, true, false, false)),
            (
                Flags::CONTROL | Flags::DEV_LCTL,
                mods(false, false, false, false, true),
            ),
            (
                Flags::COMMAND | Flags::DEV_LCMD,
                mods(false, false, false, false, true),
            ),
            (
                Flags::CONTROL | Flags::DEV_RCTL | Flags::SHIFT | Flags::DEV_RSHIFT,
                mods(true, false, false, true, false),
            ),
        ];
        for (flags, expected) in cases {
            assert_eq!(decode(flags, p), expected, "{flags:?}");
        }
    }

    /// Under Positions every modifier is its Windows key by position, and
    /// nothing is foreign: nothing is translated there anyway.
    #[test]
    fn the_positions_profile_decodes_by_position_and_is_never_foreign() {
        let p = HidProfile::Positions;
        let cases = [
            (
                Flags::CONTROL | Flags::DEV_LCTL,
                mods(true, false, false, false, false),
            ),
            (
                Flags::CONTROL | Flags::DEV_RCTL,
                mods(true, false, false, false, false),
            ),
            (
                Flags::COMMAND | Flags::DEV_LCMD,
                mods(false, true, false, false, false),
            ),
            (
                Flags::COMMAND | Flags::DEV_RCMD,
                mods(false, true, false, false, false),
            ),
            (
                Flags::OPTION | Flags::DEV_LALT,
                mods(false, false, true, false, false),
            ),
            (Flags::CONTROL, mods(true, false, false, false, false)),
            (
                Flags::SHIFT | Flags::DEV_LSHIFT,
                mods(false, false, false, true, false),
            ),
        ];
        for (flags, expected) in cases {
            assert_eq!(decode(flags, p), expected, "{flags:?}");
        }
    }

    /// A modifier the profile moves, arriving with no side bit at all, may
    /// come from anywhere (software posting bare flags), so it is foreign
    /// rather than silently ignored.
    #[test]
    fn a_moved_modifier_without_any_side_bit_is_foreign() {
        for flags in [Flags::COMMAND, Flags::CONTROL, Flags::OPTION] {
            assert!(decode(flags, HidProfile::Windows).foreign, "{flags:?}");
        }
        for flags in [Flags::COMMAND, Flags::CONTROL] {
            assert!(decode(flags, HidProfile::NativeCtrl).foreign, "{flags:?}");
        }
    }

    /// Bits that are not modifiers (Fn and NumericPad on arrows, CapsLock,
    /// NonCoalesced) never read as a Windows modifier.
    #[test]
    fn non_modifier_bits_decode_as_nothing() {
        let flags =
            Flags::FN | Flags::NUMERIC_PAD | Flags::CAPS_LOCK | Flags::NON_COALESCED | Flags::HELP;
        for profile in ALL_PROFILES {
            assert_eq!(decode(flags, profile), WinMods::default(), "{profile:?}");
        }
    }

    /// The keycodes the tap watches are the destinations of the HID tables:
    /// what [`decode`] reads as a Windows key arrives as this key.
    #[test]
    fn arrival_keycodes_match_the_mapping_destinations() {
        let cases = [
            (HidProfile::Windows, WinKey::Ctrl, kc::RIGHT_COMMAND),
            (HidProfile::Windows, WinKey::Win, kc::RIGHT_CONTROL),
            (HidProfile::Windows, WinKey::Alt, kc::RIGHT_OPTION),
            (HidProfile::NativeCtrl, WinKey::Ctrl, kc::RIGHT_CONTROL),
            (HidProfile::NativeCtrl, WinKey::Win, kc::RIGHT_COMMAND),
            (HidProfile::NativeCtrl, WinKey::Alt, kc::OPTION),
            (HidProfile::Positions, WinKey::Ctrl, kc::CONTROL),
            (HidProfile::Positions, WinKey::Win, kc::COMMAND),
            (HidProfile::Positions, WinKey::Alt, kc::OPTION),
        ];
        for (profile, key, expected) in cases {
            assert_eq!(
                arrival_keycode(key, profile),
                Some(expected),
                "{profile:?} {key:?}"
            );
        }
    }
}
