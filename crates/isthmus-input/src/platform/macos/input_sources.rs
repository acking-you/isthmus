//! Whether this user types with an input method (Pinyin, WeType, Kotoeri,
//! Rime …), which decides whether a lone Shift tap switches input sources.
//!
//! On Windows a Shift tap switches Chinese and English inside the IME, and
//! does nothing for someone who types with keyboard layouts alone. Isthmus
//! switches to the previous input source instead, so for someone with ABC
//! and German only that would swap layouts: the tap is used only when an
//! input method is among the user's input sources.
//!
//! The answer comes from `com.apple.HIToolbox` through
//! `CFPreferencesCopyAppValue`, read-only, rather than from the Text Input
//! Sources API, which must run on the main thread. Its enabled list does
//! not always name a third-party input method that is in use, so the
//! selected source and the recent history count too. Reading is split like
//! [`super::hotkeys`]: [`read_kinds`] copies the kinds into plain data and
//! [`any_input_method`] decides on them.

use objc2_core_foundation::{CFArray, CFDictionary, CFPreferencesCopyAppValue, CFString, CFType};

/// The `InputSourceKind` of an input method with modes (Pinyin's "Input
/// Mode" entries) or without (Rime's "Keyboard Input Method"). Keyboard
/// layouts are "Keyboard Layout", and the emoji palette is a "Non Keyboard
/// Input Method".
const METHOD_KINDS: [&str; 2] = ["Input Mode", "Keyboard Input Method"];

/// The lists in `com.apple.HIToolbox` that name input sources.
const LISTS: [&str; 3] = [
    "AppleEnabledInputSources",
    "AppleSelectedInputSources",
    "AppleInputSourceHistory",
];

/// Whether any of the kinds is an input method's.
pub(crate) fn any_input_method(kinds: &[String]) -> bool {
    kinds
        .iter()
        .any(|kind| METHOD_KINDS.contains(&kind.as_str()))
}

/// Whether this user has an input method; false when the preferences
/// cannot be read.
pub(crate) fn has_input_method() -> bool {
    any_input_method(&read_kinds())
}

/// Every `InputSourceKind` in the three lists, in no particular order.
pub(crate) fn read_kinds() -> Vec<String> {
    let domain = CFString::from_static_str("com.apple.HIToolbox");
    let kind_key = CFString::from_static_str("InputSourceKind");
    let mut kinds = Vec::new();
    for list in LISTS {
        let key = CFString::from_str(list);
        let Some(value) = CFPreferencesCopyAppValue(&key, &domain) else {
            continue;
        };
        let Some(array) = value.downcast_ref::<CFArray>() else {
            continue;
        };
        // SAFETY: every element of a CFArray read from preferences is a CF
        // object; each is downcast with a type check before use.
        let array: &CFArray<CFType> = unsafe { array.cast_unchecked() };
        for entry in array.to_vec() {
            let Some(entry) = entry.downcast_ref::<CFDictionary>() else {
                continue;
            };
            // SAFETY: as above; the key looked up is a CFString and the
            // value is downcast before use.
            let entry: &CFDictionary<CFString, CFType> = unsafe { entry.cast_unchecked() };
            if let Some(kind) = entry
                .get(&kind_key)
                .and_then(|kind| kind.downcast_ref::<CFString>().map(CFString::to_string))
            {
                kinds.push(kind);
            }
        }
    }
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(list: &[&str]) -> Vec<String> {
        list.iter().map(|kind| (*kind).to_owned()).collect()
    }

    /// Pinyin's modes and a mode-less method such as Rime count; keyboard
    /// layouts and the emoji palette alone do not.
    #[test]
    fn only_input_methods_count() {
        assert!(any_input_method(&kinds(&["Keyboard Layout", "Input Mode"])));
        assert!(any_input_method(&kinds(&["Keyboard Input Method"])));
        assert!(!any_input_method(&kinds(&[
            "Keyboard Layout",
            "Keyboard Layout",
            "Non Keyboard Input Method"
        ])));
        assert!(!any_input_method(&[]));
    }

    /// Prints what this Mac's preferences say, for checking by hand
    /// (read-only): `cargo test -p isthmus-input input_sources -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore = "reads this Mac's preferences"]
    fn prints_this_macs_input_source_kinds() {
        let kinds = read_kinds();
        println!("{kinds:?} -> input method: {}", any_input_method(&kinds));
    }
}
