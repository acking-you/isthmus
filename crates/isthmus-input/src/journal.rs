//! Every system change Isthmus makes, written down before it is made.
//!
//! HID properties live in the HID event system, not in this process: a
//! crash or a forced quit leaves the keyboard remapped and the mouse linear
//! until the device is replugged or the Mac restarts. So each change is
//! journalled first (the original value, and what Isthmus wrote), and the
//! next start puts back whatever is still as Isthmus left it. A value that
//! someone else changed meanwhile is never overwritten. This module is the
//! pure part: the file format and the decisions. The platform reads and
//! writes the properties.

use serde::{Deserialize, Serialize};

/// The journal file.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Journal {
    /// [`Journal::VERSION`] when this build wrote it.
    pub version: u32,
    /// `kern.bootsessionuuid` when the entries were written. A different
    /// boot means every entry is already gone.
    pub boot_session: String,
    pub entries: Vec<JournalEntry>,
}

/// One property on one service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    /// The service, valid for this boot only (hence the boot session).
    pub registry_id: u64,
    /// With `product_id` and `location_id`, lets recovery check that the
    /// registry id still names the same device, and tells a person reading
    /// the file which device the entry was for.
    pub vendor_id: u32,
    /// See `vendor_id`.
    pub product_id: u32,
    /// See `vendor_id`.
    pub location_id: u32,
    /// The property key, such as `UserKeyMapping`.
    pub key: String,
    /// What the property held before Isthmus first wrote it. The first
    /// original wins: a later write never replaces it.
    pub original: PropertyValue,
    /// What Isthmus wrote last.
    pub applied: PropertyValue,
}

/// A property value, as far as Isthmus writes them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "value")]
pub enum PropertyValue {
    /// The property was not set.
    Absent,
    /// A CFNumber.
    Number(i64),
    /// A `UserKeyMapping` array of (src, dst).
    KeyMapping(Vec<(u64, u64)>),
}

/// What to do with one entry at start, given the property's current value.
#[derive(Debug, Clone, PartialEq)]
pub enum Recovery {
    /// Still as Isthmus left it: write `original` back (an absent mapping is
    /// written as an empty array).
    Restore(PropertyValue),
    /// Someone else changed it: leave it, drop the entry.
    Leave,
}

impl Journal {
    /// The format this build writes.
    pub const VERSION: u32 = 1;

    /// Records that `key` on `entry`'s service is about to become `applied`,
    /// keeping the first original. Returns whether the journal changed.
    ///
    /// Called before every write, also when Isthmus rewrites its own value:
    /// at that point the property holds what Isthmus wrote last, and taking
    /// that as the original would make a later restore put back Isthmus's
    /// own mapping. So an existing entry only gets the new `applied`.
    pub fn record(&mut self, entry: JournalEntry) -> bool {
        let existing = self
            .entries
            .iter_mut()
            .find(|known| known.registry_id == entry.registry_id && known.key == entry.key);
        match existing {
            Some(known) if known.applied == entry.applied => false,
            Some(known) => {
                known.applied = entry.applied;
                true
            }
            None => {
                self.entries.push(entry);
                true
            }
        }
    }

    /// Forgets the entry for (`registry_id`, `key`) after a restore.
    /// Returns whether there was one.
    pub fn forget(&mut self, registry_id: u64, key: &str) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|entry| !(entry.registry_id == registry_id && entry.key == key));
        self.entries.len() != before
    }

    /// Forgets every entry of a service that no longer exists: its
    /// properties went with it, and its registry id is never reused within a
    /// boot. Returns whether there were any.
    pub fn forget_service(&mut self, registry_id: u64) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|entry| entry.registry_id != registry_id);
        self.entries.len() != before
    }
}

/// The property that holds a keyboard's key mapping, whose "not set" is
/// written back as an empty array: the HID filter takes no "remove".
pub const USER_KEY_MAPPING: &str = "UserKeyMapping";

/// The decision for one entry: restore only what is still as Isthmus left
/// it. `current` is `None` when the service no longer exists (it took its
/// properties with it).
///
/// `None` means there is nothing to do: drop the entry without a word. An
/// original that was not set comes back as an empty `UserKeyMapping` (no
/// remapping), or as [`PropertyValue::Absent`] for any other key, which tells
/// the platform to write the system's own value. A `UserKeyMapping` counts
/// as unchanged when it holds the same pairs in another order, or reads as
/// absent where Isthmus wrote an empty array: both mean the same mapping.
pub fn recover(entry: &JournalEntry, current: Option<&PropertyValue>) -> Option<Recovery> {
    let current = current?;
    if !same_value(&entry.key, current, &entry.applied) {
        return Some(Recovery::Leave);
    }
    let original = match (&entry.original, entry.key.as_str()) {
        (PropertyValue::Absent, USER_KEY_MAPPING) => PropertyValue::KeyMapping(Vec::new()),
        (original, _) => original.clone(),
    };
    Some(Recovery::Restore(original))
}

/// Do two values of `key` mean the same thing to the HID event system?
fn same_value(key: &str, a: &PropertyValue, b: &PropertyValue) -> bool {
    if key == USER_KEY_MAPPING
        && let (Some(a), Some(b)) = (mapping_pairs(a), mapping_pairs(b))
    {
        return a == b;
    }
    a == b
}

/// A key mapping's pairs in a canonical order, with "not set" as no pairs;
/// `None` for a value that is not a key mapping at all.
fn mapping_pairs(value: &PropertyValue) -> Option<Vec<(u64, u64)>> {
    match value {
        PropertyValue::Absent => Some(Vec::new()),
        PropertyValue::KeyMapping(pairs) => {
            let mut pairs = pairs.clone();
            pairs.sort_unstable();
            Some(pairs)
        }
        PropertyValue::Number(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Left Control to right Command, one pair of the Windows profile.
    const CTRL_TO_RCMD: (u64, u64) = (0x7_0000_00E0, 0x7_0000_00E7);
    /// Left Command to left Control, a pair someone else might write.
    const CMD_TO_CTRL: (u64, u64) = (0x7_0000_00E3, 0x7_0000_00E0);

    /// An entry for the Keychron's keyboard service.
    fn entry(key: &str, original: PropertyValue, applied: PropertyValue) -> JournalEntry {
        JournalEntry {
            registry_id: 0x1_0000_13e4,
            vendor_id: 0x05ac,
            product_id: 0x024f,
            location_id: 0x0011_2000,
            key: key.to_owned(),
            original,
            applied,
        }
    }

    /// A `UserKeyMapping` value of `pairs`.
    fn mapping(pairs: &[(u64, u64)]) -> PropertyValue {
        PropertyValue::KeyMapping(pairs.to_vec())
    }

    /// Recording again for the same service and key keeps the very first
    /// original, so a restore never puts back a value Isthmus wrote itself;
    /// only `applied` follows the latest write.
    #[test]
    fn the_first_original_wins() {
        let mut journal = Journal::default();
        assert!(journal.record(entry(
            USER_KEY_MAPPING,
            PropertyValue::Absent,
            mapping(&[CTRL_TO_RCMD]),
        )));
        assert!(journal.record(entry(
            USER_KEY_MAPPING,
            mapping(&[CTRL_TO_RCMD]),
            mapping(&[]),
        )));
        assert_eq!(journal.entries.len(), 1);
        assert_eq!(journal.entries[0].original, PropertyValue::Absent);
        assert_eq!(journal.entries[0].applied, mapping(&[]));
    }

    /// Recording what is already recorded changes nothing, so the caller can
    /// skip rewriting the file; another key or service is a new entry.
    #[test]
    fn recording_reports_whether_the_journal_changed() {
        let mut journal = Journal::default();
        let first = entry(
            "HIDFKeyMode",
            PropertyValue::Number(0),
            PropertyValue::Number(1),
        );
        assert!(journal.record(first.clone()));
        assert!(!journal.record(first.clone()));
        assert!(journal.record(entry(
            USER_KEY_MAPPING,
            PropertyValue::Absent,
            mapping(&[CTRL_TO_RCMD]),
        )));
        assert!(journal.record(JournalEntry {
            registry_id: 0x1_0000_13dd,
            ..first
        }));
        assert_eq!(journal.entries.len(), 3);
    }

    /// `forget` removes one property of one service, `forget_service` every
    /// property of one service, and both say whether anything went.
    #[test]
    fn forgetting_removes_exactly_the_named_entries() {
        let mut journal = Journal::default();
        journal.record(entry(
            "HIDFKeyMode",
            PropertyValue::Number(0),
            PropertyValue::Number(1),
        ));
        journal.record(entry(
            USER_KEY_MAPPING,
            PropertyValue::Absent,
            mapping(&[CTRL_TO_RCMD]),
        ));
        journal.record(JournalEntry {
            registry_id: 7,
            ..entry(
                USER_KEY_MAPPING,
                PropertyValue::Absent,
                mapping(&[CTRL_TO_RCMD]),
            )
        });

        assert!(journal.forget(0x1_0000_13e4, "HIDFKeyMode"));
        assert!(!journal.forget(0x1_0000_13e4, "HIDFKeyMode"));
        assert_eq!(journal.entries.len(), 2);
        assert!(journal.forget_service(0x1_0000_13e4));
        assert!(!journal.forget_service(0x1_0000_13e4));
        assert_eq!(journal.entries.len(), 1);
        assert_eq!(journal.entries[0].registry_id, 7);
    }

    /// A service that is gone took its properties with it: nothing to
    /// restore and nothing to report.
    #[test]
    fn a_vanished_service_needs_nothing() {
        let entry = entry(
            USER_KEY_MAPPING,
            PropertyValue::Absent,
            mapping(&[CTRL_TO_RCMD]),
        );
        assert_eq!(recover(&entry, None), None);
    }

    /// A value still as Isthmus left it gets its original back.
    #[test]
    fn a_value_still_as_isthmus_left_it_is_restored() {
        let foreign = entry(
            USER_KEY_MAPPING,
            mapping(&[CMD_TO_CTRL]),
            mapping(&[CMD_TO_CTRL, CTRL_TO_RCMD]),
        );
        assert_eq!(
            recover(&foreign, Some(&mapping(&[CMD_TO_CTRL, CTRL_TO_RCMD]))),
            Some(Recovery::Restore(mapping(&[CMD_TO_CTRL])))
        );
        let speed = entry(
            "HIDMouseAcceleration",
            PropertyValue::Number(45_056),
            PropertyValue::Number(65_536),
        );
        assert_eq!(
            recover(&speed, Some(&PropertyValue::Number(65_536))),
            Some(Recovery::Restore(PropertyValue::Number(45_056)))
        );
    }

    /// A value someone else changed since Isthmus wrote it is never
    /// overwritten.
    #[test]
    fn a_value_someone_else_changed_is_never_overwritten() {
        let ours = entry(
            USER_KEY_MAPPING,
            PropertyValue::Absent,
            mapping(&[CTRL_TO_RCMD]),
        );
        assert_eq!(
            recover(&ours, Some(&mapping(&[CMD_TO_CTRL]))),
            Some(Recovery::Leave)
        );
        let speed = entry(
            "HIDMouseAcceleration",
            PropertyValue::Number(45_056),
            PropertyValue::Number(65_536),
        );
        assert_eq!(
            recover(&speed, Some(&PropertyValue::Number(131_072))),
            Some(Recovery::Leave)
        );
        assert_eq!(
            recover(&speed, Some(&PropertyValue::Absent)),
            Some(Recovery::Leave)
        );
    }

    /// An original that was not set comes back as an empty mapping for
    /// `UserKeyMapping`, which the HID filter accepts as "no remapping", and
    /// as `Absent` for any other key, so the platform writes the system's
    /// value.
    #[test]
    fn an_absent_original_is_an_empty_mapping_or_the_system_default() {
        let keys = entry(
            USER_KEY_MAPPING,
            PropertyValue::Absent,
            mapping(&[CTRL_TO_RCMD]),
        );
        assert_eq!(
            recover(&keys, Some(&mapping(&[CTRL_TO_RCMD]))),
            Some(Recovery::Restore(mapping(&[])))
        );
        let fkeys = entry(
            "HIDFKeyMode",
            PropertyValue::Absent,
            PropertyValue::Number(1),
        );
        assert_eq!(
            recover(&fkeys, Some(&PropertyValue::Number(1))),
            Some(Recovery::Restore(PropertyValue::Absent))
        );
    }

    /// A key mapping is compared as the set of pairs it maps: the same pairs
    /// in another order, or "not set" where Isthmus wrote an empty array,
    /// is still Isthmus's value. Numbers compare exactly.
    #[test]
    fn key_mappings_compare_by_what_they_map() {
        let both = entry(
            USER_KEY_MAPPING,
            mapping(&[CMD_TO_CTRL]),
            mapping(&[CTRL_TO_RCMD, CMD_TO_CTRL]),
        );
        assert_eq!(
            recover(&both, Some(&mapping(&[CMD_TO_CTRL, CTRL_TO_RCMD]))),
            Some(Recovery::Restore(mapping(&[CMD_TO_CTRL])))
        );
        let emptied = entry(USER_KEY_MAPPING, mapping(&[CMD_TO_CTRL]), mapping(&[]));
        assert_eq!(
            recover(&emptied, Some(&PropertyValue::Absent)),
            Some(Recovery::Restore(mapping(&[CMD_TO_CTRL])))
        );
        let zero = entry(
            "HIDFKeyMode",
            PropertyValue::Number(1),
            PropertyValue::Number(0),
        );
        assert_eq!(
            recover(&zero, Some(&PropertyValue::Absent)),
            Some(Recovery::Leave)
        );
    }

    /// The file keeps its shape: camelCase keys and tagged values, and an
    /// empty file reads as an empty journal.
    #[test]
    fn the_journal_file_round_trips() {
        let mut journal = Journal {
            version: Journal::VERSION,
            boot_session: "12F8E6DD-5134-4B64-9578-EB937B801FA6".into(),
            entries: Vec::new(),
        };
        journal.record(entry(
            USER_KEY_MAPPING,
            PropertyValue::Absent,
            mapping(&[CTRL_TO_RCMD]),
        ));
        let text = serde_json::to_string(&journal).unwrap();
        assert!(text.contains(r#""bootSession""#), "{text}");
        assert!(text.contains(r#""original":{"type":"absent"}"#), "{text}");
        assert!(
            text.contains(r#""applied":{"type":"keyMapping","value":[[30064771296,30064771303]]}"#),
            "{text}"
        );
        assert_eq!(serde_json::from_str::<Journal>(&text).unwrap(), journal);
        assert_eq!(
            serde_json::from_str::<Journal>("{}").unwrap(),
            Journal::default()
        );
    }
}
