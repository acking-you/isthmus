//! The macOS symbolic hotkeys as this user has them bound (tap half).
//!
//! Win+D, Win+Tab, the Start-menu tap, the input-source and Space switches
//! and tiling post macOS's own hotkey chords rather than calling private Dock
//! or SkyLight functions. Posting the chord the user actually bound keeps
//! them working after a rebind, and a hotkey the user switched off then does
//! nothing, as it would on that Mac.
//!
//! Reading is split in two. [`read_stored`] copies the preference into plain
//! Rust data; it is the only OS call, and it only reads.
//! [`resolve`] turns that data into [`SystemHotkeys`], filling in the macOS
//! defaults for every hotkey the user never touched (macOS stores only
//! customised ids). The tests pin `resolve` on hand-built maps.

use std::collections::BTreeMap;

use objc2_core_foundation::{
    CFArray, CFBoolean, CFDictionary, CFNumber, CFPreferencesCopyAppValue, CFRetained, CFString,
    CFType,
};

use crate::engine::{Chord, SystemHotkeys};
use crate::keys::{KeyCode, Mods, kc};

/// The symbolic-hotkey ids Isthmus posts, as `com.apple.symbolichotkeys`
/// numbers them (`DefaultShortcutsTable.xml` on macOS 27).
pub(crate) mod id {
    /// Mission Control ("All windows").
    pub(crate) const MISSION_CONTROL: u32 = 32;
    /// Show Desktop.
    pub(crate) const SHOW_DESKTOP: u32 = 36;
    /// Select the previous input source.
    pub(crate) const PREVIOUS_INPUT_SOURCE: u32 = 60;
    /// Select the next source in the Input menu.
    pub(crate) const NEXT_INPUT_SOURCE: u32 = 61;
    /// Show Spotlight search.
    pub(crate) const SPOTLIGHT: u32 = 64;
    /// Move left a Space.
    pub(crate) const SPACE_LEFT: u32 = 79;
    /// Move right a Space.
    pub(crate) const SPACE_RIGHT: u32 = 81;
    /// Window › Fill (macOS 15+ tiling).
    pub(crate) const TILE_FILL: u32 = 237;
    /// Window › Return to Previous Size.
    pub(crate) const TILE_RESTORE: u32 = 239;
    /// Window › Move & Resize › Left.
    pub(crate) const TILE_LEFT: u32 = 240;
    /// Window › Move & Resize › Right.
    pub(crate) const TILE_RIGHT: u32 = 241;

    /// The tiling hotkeys, which macOS stores as Fn+Ctrl+key. On these
    /// alone a stored Fn on an arrow means Fn held, so the Fn layer's key
    /// (Fn+Left is Home): read as the arrow, Tile Left would be the Space
    /// switch's Ctrl+Left, since hardware sends every arrow with Fn.
    pub(crate) const fn on_fn_layer(hotkey: u32) -> bool {
        matches!(hotkey, TILE_FILL | TILE_RESTORE | TILE_LEFT | TILE_RIGHT)
    }
}

/// The modifier bits of a stored hotkey. They are `CGEventFlags` bits, so a
/// stored arrow or F-key hotkey usually carries the Fn bit too.
mod mask {
    pub(super) const SHIFT: i64 = 0x2_0000;
    pub(super) const CONTROL: i64 = 0x4_0000;
    pub(super) const OPTION: i64 = 0x8_0000;
    pub(super) const COMMAND: i64 = 0x10_0000;
    pub(super) const FN: i64 = 0x80_0000;
}

/// The highest virtual keycode (`kVK_*` stops at 0x7E). A stored keycode
/// above it, such as the 65535 macOS writes for a cleared shortcut, binds no
/// key.
const MAX_KEYCODE: i64 = 0x7F;

/// One hotkey as `com.apple.symbolichotkeys` stores it, copied into plain
/// data: `{enabled, value: {parameters: [char, keycode, modifiers]}}`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct StoredHotkey {
    /// `enabled`; missing means enabled, as System Settings treats it.
    pub enabled: Option<bool>,
    /// `value.parameters`; missing means the default chord is in effect
    /// (ids 79 to 82 on this Mac are stored enabled with no value).
    pub parameters: Option<Vec<i64>>,
}

/// What a stored hotkey's parameters bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Binding {
    Bound(Chord),
    /// A cleared shortcut: there is no chord to post.
    Unbound,
    /// Too short to read; the default is the best guess.
    Malformed,
}

/// Reads hotkey `hotkey`'s `[char, keycode, modifiers]`. The character is
/// ignored: macOS matches hotkeys on the keycode, and the character is 65535
/// for keys without one.
///
/// The Fn bit on an arrow needs the id to read. Hardware sends every arrow
/// with Fn, so an arrow hotkey such as Mission Control's Ctrl+Up is often
/// stored with it (0x84_0000) and means the plain arrow; posting anything
/// else would trigger a tiling hotkey instead. On a tiling hotkey it means
/// Fn held, which turns the arrow into Home, End, Page Up or Page Down, and
/// that key is what is posted ([`Chord::on_fn_layer`]); the arrow would be
/// the Space switch's chord.
fn binding(hotkey: u32, parameters: &[i64]) -> Binding {
    let [_, keycode, modifiers, ..] = *parameters else {
        return Binding::Malformed;
    };
    if !(0..=MAX_KEYCODE).contains(&keycode) {
        return Binding::Unbound;
    }
    let Ok(keycode) = KeyCode::try_from(keycode) else {
        return Binding::Unbound;
    };
    let has = |bit: i64| modifiers & bit != 0;
    let mods = Mods {
        control: has(mask::CONTROL),
        option: has(mask::OPTION),
        shift: has(mask::SHIFT),
        command: has(mask::COMMAND),
    };
    let chord = Chord::new(keycode, mods);
    Binding::Bound(if !has(mask::FN) {
        chord
    } else if id::on_fn_layer(hotkey) {
        chord.on_fn_layer()
    } else if kc::is_arrow(keycode) {
        chord
    } else {
        chord.with_fn()
    })
}

/// The chord stored entry `hotkey` gives, or `default` when it stores none.
fn entry_chord(hotkey: u32, entry: &StoredHotkey, default: Option<Chord>) -> Option<Chord> {
    if entry.enabled == Some(false) {
        return None;
    }
    let Some(parameters) = &entry.parameters else {
        return default;
    };
    match binding(hotkey, parameters) {
        Binding::Bound(chord) => Some(chord),
        Binding::Unbound => None,
        Binding::Malformed => default,
    }
}

/// The hotkeys in effect: each stored entry where there is one, else the
/// macOS default.
pub(crate) fn resolve(stored: &BTreeMap<u32, StoredHotkey>) -> SystemHotkeys {
    let pick = |hotkey: u32, default: Option<Chord>| match stored.get(&hotkey) {
        Some(entry) => entry_chord(hotkey, entry, default),
        None => default,
    };
    let SystemHotkeys {
        mission_control,
        show_desktop,
        spotlight,
        previous_input_source,
        next_input_source,
        space_left,
        space_right,
        tile_fill,
        tile_left,
        tile_right,
        tile_restore,
    } = SystemHotkeys::default();
    SystemHotkeys {
        mission_control: pick(id::MISSION_CONTROL, mission_control),
        show_desktop: pick(id::SHOW_DESKTOP, show_desktop),
        spotlight: pick(id::SPOTLIGHT, spotlight),
        previous_input_source: pick(id::PREVIOUS_INPUT_SOURCE, previous_input_source),
        next_input_source: pick(id::NEXT_INPUT_SOURCE, next_input_source),
        space_left: pick(id::SPACE_LEFT, space_left),
        space_right: pick(id::SPACE_RIGHT, space_right),
        tile_fill: pick(id::TILE_FILL, tile_fill),
        tile_left: pick(id::TILE_LEFT, tile_left),
        tile_right: pick(id::TILE_RIGHT, tile_right),
        tile_restore: pick(id::TILE_RESTORE, tile_restore),
    }
}

/// Reads `AppleSymbolicHotKeys` from `com.apple.symbolichotkeys` with
/// `CFPreferencesCopyAppValue` (read-only) and fills in macOS defaults for
/// every hotkey the user has not customised. A hotkey stored with
/// `enabled = false` becomes `None`. Never fails: unreadable preferences
/// give the defaults.
pub(crate) fn load() -> SystemHotkeys {
    let stored = read_stored().unwrap_or_default();
    resolve(&stored)
}

/// Copies the stored hotkeys into plain data. `None` when the preference is
/// missing or not a dictionary; entries that do not parse are skipped, so
/// one odd entry cannot hide the others.
pub(crate) fn read_stored() -> Option<BTreeMap<u32, StoredHotkey>> {
    let key = CFString::from_static_str("AppleSymbolicHotKeys");
    let domain = CFString::from_static_str("com.apple.symbolichotkeys");
    let value = CFPreferencesCopyAppValue(&key, &domain)?;
    let dictionary = value.downcast_ref::<CFDictionary>()?;
    // SAFETY: every key and value of a CFDictionary read from preferences is
    // a CF object, and `CFType` is the root of them all; each one is
    // downcast with a type check before it is used as anything narrower.
    let dictionary: &CFDictionary<CFType, CFType> = unsafe { dictionary.cast_unchecked() };
    let (keys, values) = dictionary.to_vecs();
    let mut stored = BTreeMap::new();
    for (key, value) in keys.iter().zip(&values) {
        let Some(hotkey) = key
            .downcast_ref::<CFString>()
            .and_then(|name| name.to_string().parse::<u32>().ok())
        else {
            continue;
        };
        let Some(entry) = value.downcast_ref::<CFDictionary>() else {
            continue;
        };
        stored.insert(hotkey, stored_entry(entry));
    }
    Some(stored)
}

/// One entry's `enabled` and `value.parameters`.
fn stored_entry(entry: &CFDictionary) -> StoredHotkey {
    // SAFETY: as in `read_stored`, values are only used after a checked
    // downcast; the keys looked up are CFStrings.
    let entry: &CFDictionary<CFString, CFType> = unsafe { entry.cast_unchecked() };
    let enabled = entry
        .get(&CFString::from_static_str("enabled"))
        .and_then(|value| cf_bool(&value));
    let parameters = entry
        .get(&CFString::from_static_str("value"))
        .and_then(|value| parameters_of(&value));
    StoredHotkey {
        enabled,
        parameters,
    }
}

/// A plist boolean, which some writers store as a number.
fn cf_bool(value: &CFType) -> Option<bool> {
    if let Some(boolean) = value.downcast_ref::<CFBoolean>() {
        return Some(boolean.as_bool());
    }
    value
        .downcast_ref::<CFNumber>()
        .and_then(CFNumber::as_i64)
        .map(|number| number != 0)
}

/// `value.parameters` as integers; `None` when any element is not a number.
fn parameters_of(value: &CFType) -> Option<Vec<i64>> {
    let value = value.downcast_ref::<CFDictionary>()?;
    // SAFETY: as in `read_stored`.
    let value: &CFDictionary<CFString, CFType> = unsafe { value.cast_unchecked() };
    let parameters: CFRetained<CFType> = value.get(&CFString::from_static_str("parameters"))?;
    let array = parameters.downcast_ref::<CFArray>()?;
    // SAFETY: every element of a CFArray is a CF object; each is downcast
    // with a type check below.
    let array: &CFArray<CFType> = unsafe { array.cast_unchecked() };
    array
        .to_vec()
        .iter()
        .map(|element| {
            element
                .downcast_ref::<CFNumber>()
                .and_then(CFNumber::as_i64)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An entry as System Settings writes a customised hotkey.
    fn custom(parameters: [i64; 3]) -> StoredHotkey {
        StoredHotkey {
            enabled: Some(true),
            parameters: Some(parameters.to_vec()),
        }
    }

    /// macOS stores only customised ids, so an empty preference means the
    /// macOS defaults throughout.
    #[test]
    fn nothing_stored_gives_the_macos_defaults() {
        assert_eq!(resolve(&BTreeMap::new()), SystemHotkeys::default());
    }

    /// A hotkey switched off does nothing, and only that one changes.
    #[test]
    fn a_disabled_hotkey_resolves_to_none() {
        let stored = BTreeMap::from([(
            id::SHOW_DESKTOP,
            StoredHotkey {
                enabled: Some(false),
                parameters: Some(vec![65535, 103, 0x80_0000]),
            },
        )]);
        let hotkeys = resolve(&stored);
        assert_eq!(hotkeys.show_desktop, None);
        assert_eq!(
            hotkeys.spotlight,
            SystemHotkeys::default().spotlight,
            "other hotkeys keep their defaults"
        );
    }

    /// A rebound hotkey posts the user's chord, with every modifier bit
    /// read from the stored mask. The Fn bit an arrow hotkey is stored with
    /// is the arrow's own, so Mission Control on Ctrl+Up still posts the Up
    /// arrow, not Ctrl+Page Up (Tile Top).
    #[test]
    fn a_rebound_hotkey_posts_the_users_chord() {
        let stored = BTreeMap::from([
            // Spotlight moved to Ctrl+Option+Shift+Command+K.
            (id::SPOTLIGHT, custom([107, i64::from(kc::K), 0x1E_0000])),
            // Mission Control on Ctrl+Up, with the Fn bit arrows are stored
            // with.
            (
                id::MISSION_CONTROL,
                custom([65535, i64::from(kc::UP), 0x84_0000]),
            ),
        ]);
        let hotkeys = resolve(&stored);
        let all = Mods::CONTROL
            .with(Mods::OPTION)
            .with(Mods::SHIFT)
            .with(Mods::COMMAND);
        assert_eq!(hotkeys.spotlight, Some(Chord::new(kc::K, all)));
        assert_eq!(
            hotkeys.mission_control,
            Some(Chord::new(kc::UP, Mods::CONTROL))
        );
    }

    /// Tile Left/Right stored as Fn+Ctrl+arrow (macOS's own form) post the
    /// Fn layer's Ctrl+Home/End, while the Space switches stored on the
    /// same arrows and mask post the arrows, so Win+Left tiles instead of
    /// moving a Space. A tiling hotkey rebound without Fn posts its arrow.
    #[test]
    fn stored_tiling_arrows_post_the_fn_layer_and_space_switches_the_arrow() {
        let fn_ctrl = 0x84_0000;
        let stored = BTreeMap::from([
            (id::TILE_LEFT, custom([65535, i64::from(kc::LEFT), fn_ctrl])),
            (
                id::SPACE_LEFT,
                custom([65535, i64::from(kc::LEFT), fn_ctrl]),
            ),
            (
                id::SPACE_RIGHT,
                custom([65535, i64::from(kc::RIGHT), fn_ctrl]),
            ),
            // Tile Right rebound to Ctrl+Option+Right, no Fn.
            (
                id::TILE_RIGHT,
                custom([65535, i64::from(kc::RIGHT), 0xC_0000]),
            ),
        ]);
        let hotkeys = resolve(&stored);
        assert_eq!(hotkeys.tile_left, SystemHotkeys::default().tile_left);
        assert_eq!(
            hotkeys.tile_left,
            Some(Chord::new(kc::HOME, Mods::CONTROL).with_fn())
        );
        assert_eq!(
            hotkeys.space_left,
            Some(Chord::new(kc::LEFT, Mods::CONTROL))
        );
        assert_eq!(
            hotkeys.space_right,
            Some(Chord::new(kc::RIGHT, Mods::CONTROL))
        );
        assert_eq!(
            hotkeys.tile_right,
            Some(Chord::new(kc::RIGHT, Mods::CONTROL.with(Mods::OPTION)))
        );
    }

    /// Show Desktop stored as F11 with only the Fn bit is F11 with no
    /// modifiers, sent with Fn.
    #[test]
    fn a_function_key_hotkey_keeps_its_fn_flag() {
        let stored = BTreeMap::from([(
            id::SHOW_DESKTOP,
            custom([65535, i64::from(kc::F11), 0x80_0000]),
        )]);
        assert_eq!(
            resolve(&stored).show_desktop,
            Some(Chord::new(kc::F11, Mods::NONE).with_fn())
        );
    }

    /// Enabled with no value (how ids 79 to 82 sit on this Mac) means the
    /// default chord is in effect.
    #[test]
    fn enabled_without_a_value_keeps_the_default() {
        let stored = BTreeMap::from([(
            id::SPACE_LEFT,
            StoredHotkey {
                enabled: Some(true),
                parameters: None,
            },
        )]);
        assert_eq!(
            resolve(&stored).space_left,
            SystemHotkeys::default().space_left
        );
    }

    /// A cleared shortcut (keycode 65535) binds no key, so there is nothing
    /// to post.
    #[test]
    fn a_cleared_shortcut_resolves_to_none() {
        let stored = BTreeMap::from([(id::TILE_LEFT, custom([65535, 65535, 0]))]);
        assert_eq!(resolve(&stored).tile_left, None);
    }

    /// Parameters too short to read fall back to the default rather than
    /// switching the hotkey off.
    #[test]
    fn malformed_parameters_fall_back_to_the_default() {
        let stored = BTreeMap::from([(
            id::NEXT_INPUT_SOURCE,
            StoredHotkey {
                enabled: None,
                parameters: Some(vec![32]),
            },
        )]);
        assert_eq!(
            resolve(&stored).next_input_source,
            SystemHotkeys::default().next_input_source
        );
    }

    /// Reads this Mac's real hotkeys (read-only) and prints them, to check
    /// the parsing against `defaults read com.apple.symbolichotkeys`.
    #[test]
    #[ignore = "reads this Mac's preferences; run by hand with --nocapture"]
    fn prints_this_macs_symbolic_hotkeys() {
        let stored = read_stored();
        let wanted = [
            id::MISSION_CONTROL,
            id::SHOW_DESKTOP,
            id::PREVIOUS_INPUT_SOURCE,
            id::NEXT_INPUT_SOURCE,
            id::SPOTLIGHT,
            id::SPACE_LEFT,
            id::SPACE_RIGHT,
            id::TILE_FILL,
            id::TILE_RESTORE,
            id::TILE_LEFT,
            id::TILE_RIGHT,
        ];
        match &stored {
            Some(stored) => {
                println!("{} ids stored", stored.len());
                for hotkey in wanted {
                    println!("stored {hotkey}: {:?}", stored.get(&hotkey));
                }
            }
            None => println!("AppleSymbolicHotKeys is not readable"),
        }
        println!("{:#?}", load());
    }
}
