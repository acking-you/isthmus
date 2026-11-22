//! What each HID service should carry, and getting it there safely.
//!
//! Reconciling is three steps, so the decisions are tested without IOKit:
//!
//! 1. [`intents`]: from the settings, the app profile and the devices, the
//!    values each service should carry (key mapping, F-key mode, pointer
//!    gain). A key a service should not carry is simply not listed.
//! 2. [`diff`]: against what the services hold now and what the journal
//!    says Isthmus wrote, the writes and restores still needed. Only
//!    differences are written, so reconciling is idempotent and may run on
//!    every hint (hot-plug, wake, a global property change).
//! 3. [`execute`]: each write is journalled, durably, before it is made and
//!    read back after; each restore puts back the first original (of a key
//!    mapping, only the modifier entries: the others stay as they are now),
//!    or the system-wide value for keys macOS keeps one of, and only over a
//!    value still as Isthmus left it.
//!
//! Reading a property back is not always possible (the keyboard filter does
//! not return `UserKeyMapping`), so an unreadable value is taken to be what
//! the journal says Isthmus wrote, or absent when Isthmus never wrote it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use tracing::{debug, warn};

use crate::device::{self, Device, DeviceKind, HidService};
use crate::error::Error;
use crate::hidmap::{self, HidProfile, KeyMapping};
use crate::journal::{Journal, JournalEntry, PropertyValue, Recovery};
use crate::pointer::{self, PointerPlan};
use crate::service::effective_layout;
use crate::settings::{FunctionKeys, KeyboardStyle, LayoutChoice, Settings};
use crate::status::Issue;

/// The acceleration type under which linear pointer scaling works.
const MOUSE_ACCELERATION_TYPE: &str = "HIDMouseAcceleration";

/// The fixed-point value that switches pointer acceleration off (-1.0).
const RAW_ACCELERATION: i64 = -65_536;

/// The acceleration macOS ships with (0.6875), written back only when
/// neither the system-wide value nor a journalled original is known.
const FALLBACK_ACCELERATION: i64 = 45_056;

/// A property Isthmus writes. The order is the order of writes to one
/// service: the linear flag before the acceleration, because the pointer
/// filter rebuilds its accelerator on each and must see the flag first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum PropKey {
    /// The per-keyboard modifier mapping.
    UserKeyMapping,
    /// F1–F12 as F-keys (1) or media keys (0) on Apple-vendor keyboards.
    FKeyMode,
    /// Linear pointer scaling (macOS 14+): acceleration off, gain as speed.
    LinearScaling,
    /// The pointer gain (linear) or curve index (otherwise), 16.16.
    MouseAcceleration,
}

impl PropKey {
    /// Every key, in write order.
    pub(crate) const ALL: [PropKey; 4] = [
        PropKey::UserKeyMapping,
        PropKey::FKeyMode,
        PropKey::LinearScaling,
        PropKey::MouseAcceleration,
    ];

    /// The HID property name.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            PropKey::UserKeyMapping => "UserKeyMapping",
            PropKey::FKeyMode => "HIDFKeyMode",
            PropKey::LinearScaling => "HIDUseLinearScalingMouseAcceleration",
            PropKey::MouseAcceleration => "HIDMouseAcceleration",
        }
    }

    /// The key a journal entry names; `None` for one a newer Isthmus wrote.
    pub(crate) fn from_name(name: &str) -> Option<PropKey> {
        PropKey::ALL.into_iter().find(|key| key.name() == name)
    }

    /// Whether macOS keeps a system-wide value of this key (System Settings
    /// pushes it to every service). A restore prefers that value to the
    /// journalled original, which goes stale when the user moves the
    /// System Settings slider while Isthmus runs.
    pub(crate) const fn has_global(self) -> bool {
        match self {
            PropKey::UserKeyMapping => false,
            PropKey::FKeyMode | PropKey::LinearScaling | PropKey::MouseAcceleration => true,
        }
    }
}

/// What the HID thread should make true.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Desired {
    /// Isthmus is on, owns the machine (instance lock), and the user's
    /// session is the active one. Off restores everything.
    pub active: bool,
    /// The keyboard and mouse sections, normalized.
    pub settings: Settings,
    /// The mapping Windows-style keyboards get now.
    pub hid_profile: HidProfile,
    /// The main display's panel pixels per point, which divides the Windows
    /// pointer gain (see [`crate::pointer::plan`]).
    pub display_scale: f64,
}

impl Desired {
    /// Nothing applied: what the HID thread starts from and what a restore
    /// leaves.
    pub(crate) fn inactive() -> Desired {
        Desired {
            active: false,
            settings: Settings::default(),
            hid_profile: HidProfile::Positions,
            display_scale: 1.0,
        }
    }
}

/// One service as reconciling sees it now.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Observed {
    /// Identity and kind, read when the service appeared.
    pub service: HidService,
    /// Values read back, by key; a key missing here could not be read.
    pub values: BTreeMap<PropKey, PropertyValue>,
    /// The service itself publishes `HIDUseLinearScalingMouseAcceleration`
    /// (macOS 14+, and a client that may see it).
    pub publishes_linear: bool,
    /// System Settings › Modifier Keys has a mapping on this service.
    pub system_modifier_keys: bool,
}

/// Why a service counts in the status, when its intent is in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// It carries a Windows key mapping.
    MappedKeyboard,
    /// It carries a Windows pointer speed, or acceleration off.
    TunedPointer,
}

/// The values one service should carry.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ServiceIntent {
    /// The service, as the event system and the journal name it.
    pub registry_id: u64,
    /// Values by key, in write order. A key not listed goes back to what it
    /// was before Isthmus wrote it.
    pub targets: Vec<(PropKey, PropertyValue)>,
    /// How the service counts in the status once its targets are in place.
    pub role: Option<Role>,
}

/// Step 1: what every observed service should carry.
pub(crate) fn intents(
    devices: &[Device],
    observed: &BTreeMap<u64, Observed>,
    journal: &Journal,
    desired: &Desired,
) -> Vec<ServiceIntent> {
    let keyboard_on = desired.active && desired.settings.keyboard.enabled;
    let mouse_on = desired.active && desired.settings.mouse.enabled;
    let mut out = Vec::new();
    for device in devices {
        let id = device.id.as_str();
        match device.kind {
            DeviceKind::Keyboard => {
                let keyboard = &desired.settings.keyboard;
                let style = if keyboard_on {
                    keyboard.style_of(id)
                } else {
                    KeyboardStyle::Mac
                };
                let own = keyboard.device(id);
                let layout = effective_layout(
                    own.map_or(LayoutChoice::Auto, |d| d.layout),
                    device.detected_layout,
                );
                let function_keys = own.map_or(FunctionKeys::Standard, |d| d.function_keys);
                let ours = hidmap::key_mapping(style, layout, desired.hid_profile);
                for rid in &device.services {
                    let Some(seen) = observed.get(rid).filter(|o| o.service.is_keyboard) else {
                        continue;
                    };
                    let mut targets = Vec::new();
                    if style == KeyboardStyle::Windows {
                        let current = current_mapping(seen, journal);
                        let merged = hidmap::merge(&current, &ours);
                        targets.push((PropKey::UserKeyMapping, mapping_value(&merged)));
                        // Apple's keyboards (and the clones that borrow its
                        // vendor id) send media keys on F1–F12 unless told
                        // not to, over USB and Bluetooth alike.
                        if function_keys == FunctionKeys::Standard
                            && device::is_apple_vendor(seen.service.vendor_id)
                        {
                            targets.push((PropKey::FKeyMode, PropertyValue::Number(1)));
                        }
                    }
                    let role = (style == KeyboardStyle::Windows && !ours.is_empty())
                        .then_some(Role::MappedKeyboard);
                    out.push(ServiceIntent {
                        registry_id: *rid,
                        targets,
                        role,
                    });
                }
            }
            DeviceKind::Mouse => {
                for rid in &device.services {
                    let Some(seen) = observed.get(rid).filter(|o| o.service.is_pointer) else {
                        continue;
                    };
                    let plan = if mouse_on {
                        let profile = desired.settings.mouse.profile_of(id);
                        pointer::plan(&profile, supports_linear(seen), desired.display_scale)
                    } else {
                        PointerPlan::System
                    };
                    out.push(pointer_intent(*rid, plan));
                }
            }
            // Trackpads and the Magic Mouse are shown, never changed; any
            // journalled value on them goes back.
            DeviceKind::Trackpad => {}
        }
    }
    out
}

/// Linear pointer scaling works only on a service that publishes the flag
/// and accelerates as a mouse; trackpads use another acceleration key.
pub(crate) fn supports_linear(seen: &Observed) -> bool {
    seen.publishes_linear && seen.service.pointer_acceleration_type == MOUSE_ACCELERATION_TYPE
}

/// The targets of a pointer plan, linear flag first.
pub(crate) fn pointer_intent(registry_id: u64, plan: PointerPlan) -> ServiceIntent {
    let (targets, role) = match plan {
        PointerPlan::System => (Vec::new(), None),
        PointerPlan::Linear { gain } => (
            vec![
                (PropKey::LinearScaling, PropertyValue::Number(1)),
                (
                    PropKey::MouseAcceleration,
                    PropertyValue::Number(i64::from(pointer::to_fixed(gain))),
                ),
            ],
            Some(Role::TunedPointer),
        ),
        PointerPlan::Raw => (
            vec![(
                PropKey::MouseAcceleration,
                PropertyValue::Number(RAW_ACCELERATION),
            )],
            Some(Role::TunedPointer),
        ),
    };
    ServiceIntent {
        registry_id,
        targets,
        role,
    }
}

/// The key mapping a keyboard service holds now, as far as Isthmus can
/// tell: read back, else what Isthmus last wrote, else nothing.
fn current_mapping(seen: &Observed, journal: &Journal) -> Vec<KeyMapping> {
    seen.values
        .get(&PropKey::UserKeyMapping)
        .or_else(|| {
            journal_entry(
                journal,
                seen.service.registry_id,
                PropKey::UserKeyMapping.name(),
            )
            .map(|e| &e.applied)
        })
        .map_or_else(Vec::new, key_mappings)
}

/// The entries of a `UserKeyMapping` value; none for an absent one, or for a
/// value that is not a key mapping at all.
fn key_mappings(value: &PropertyValue) -> Vec<KeyMapping> {
    match value {
        PropertyValue::KeyMapping(pairs) => pairs
            .iter()
            .map(|&(src, dst)| KeyMapping { src, dst })
            .collect(),
        PropertyValue::Absent | PropertyValue::Number(_) => Vec::new(),
    }
}

/// `mappings` as the `UserKeyMapping` value to write.
fn mapping_value(mappings: &[KeyMapping]) -> PropertyValue {
    PropertyValue::KeyMapping(mappings.iter().map(|m| (m.src, m.dst)).collect())
}

fn journal_entry<'a>(
    journal: &'a Journal,
    registry_id: u64,
    key: &str,
) -> Option<&'a JournalEntry> {
    journal
        .entries
        .iter()
        .find(|e| e.registry_id == registry_id && e.key == key)
}

/// Whether two values of `key` are the same setting. An absent key mapping
/// and an empty one are the same (the filter maps nothing either way), and
/// mapping order does not matter, so a reordering read-back never causes a
/// rewrite.
pub(crate) fn same_value(key: PropKey, a: &PropertyValue, b: &PropertyValue) -> bool {
    match key {
        PropKey::UserKeyMapping => mapping_set(a) == mapping_set(b),
        PropKey::FKeyMode | PropKey::LinearScaling | PropKey::MouseAcceleration => a == b,
    }
}

fn mapping_set(value: &PropertyValue) -> Option<BTreeSet<(u64, u64)>> {
    match value {
        PropertyValue::Absent => Some(BTreeSet::new()),
        PropertyValue::KeyMapping(pairs) => Some(pairs.iter().copied().collect()),
        PropertyValue::Number(_) => None,
    }
}

/// One change reconciling still needs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Step {
    /// Journal, then write `value`. `original` is what the property holds
    /// now (absent when unreadable); the journal keeps the first original.
    Apply {
        service: HidService,
        key: PropKey,
        value: PropertyValue,
        original: PropertyValue,
    },
    /// Put back what the journal entry says, if the property is still as
    /// Isthmus left it. `current` is the read-back, `None` if unreadable.
    Restore {
        entry: JournalEntry,
        current: Option<PropertyValue>,
    },
    /// The service is gone and took its properties with it.
    ForgetService { registry_id: u64 },
}

/// Step 2: the writes and restores needed to make the observed services
/// carry their intents. `force` names keys whose unreadable values must not
/// be trusted to be what Isthmus wrote: a system-wide push (System Settings,
/// another tool) has just overwritten them on every service. `retry` names
/// the (service, key) writes that failed last time ([`ExecReport::failed`]):
/// the journal may say Isthmus wrote them although the property never
/// changed, so an unreadable one is written again rather than trusted.
pub(crate) fn diff(
    intents: &[ServiceIntent],
    observed: &BTreeMap<u64, Observed>,
    journal: &Journal,
    force: &BTreeSet<PropKey>,
    retry: &BTreeSet<(u64, &'static str)>,
) -> Vec<Step> {
    let mut steps = Vec::new();
    for (rid, seen) in observed {
        // A service that is both a keyboard and a pointer (a combo receiver,
        // a Bluetooth keyboard with a touchpad, a mouse with a keyboard
        // collection) belongs to two devices and has an intent from each: it
        // carries the targets of both. Their keys never overlap.
        let targets: Vec<&(PropKey, PropertyValue)> = intents
            .iter()
            .filter(|i| i.registry_id == *rid)
            .flat_map(|i| &i.targets)
            .collect();
        for key in PropKey::ALL {
            let entry = journal_entry(journal, *rid, key.name());
            let read = seen.values.get(&key);
            match targets.iter().find(|(k, _)| *k == key) {
                Some((_, target)) => {
                    let in_place = match (read, entry) {
                        (Some(now), _) => same_value(key, now, target),
                        (None, _) if force.contains(&key) => false,
                        (None, _) if retry.contains(&(*rid, key.name())) => false,
                        (None, Some(entry)) => same_value(key, &entry.applied, target),
                        (None, None) => same_value(key, &PropertyValue::Absent, target),
                    };
                    if !in_place {
                        steps.push(Step::Apply {
                            service: seen.service.clone(),
                            key,
                            value: target.clone(),
                            original: read.cloned().unwrap_or(PropertyValue::Absent),
                        });
                    }
                }
                None => {
                    if let Some(entry) = entry {
                        steps.push(Step::Restore {
                            entry: entry.clone(),
                            current: read.cloned(),
                        });
                    }
                }
            }
        }
        // Keys a newer Isthmus journalled go back too: this one writes none
        // of them.
        for entry in journal.entries.iter().filter(|e| e.registry_id == *rid) {
            if PropKey::from_name(&entry.key).is_none() {
                steps.push(Step::Restore {
                    entry: entry.clone(),
                    current: None,
                });
            }
        }
    }
    let gone: BTreeSet<u64> = journal
        .entries
        .iter()
        .map(|e| e.registry_id)
        .filter(|rid| !observed.contains_key(rid))
        .collect();
    steps.extend(
        gone.into_iter()
            .map(|registry_id| Step::ForgetService { registry_id }),
    );
    steps
}

/// Restores for every journalled entry: what a restore-all and a crash
/// recovery do. Entries of services that are gone are forgotten.
pub(crate) fn restore_everything(
    observed: &BTreeMap<u64, Observed>,
    journal: &Journal,
) -> Vec<Step> {
    diff(&[], observed, journal, &BTreeSet::new(), &BTreeSet::new())
}

/// The value a restore writes for `key`, given its journalled original and
/// its value now (`current`, what Isthmus wrote when it cannot be read).
///
/// A key mapping gets the original's entries for the modifier sources back,
/// and keeps every other source's entries as they are now: those belong to
/// someone else, who may have changed them while Isthmus ran, and putting
/// back the original whole would undo that. It is always an array (an
/// empty one for nothing: a property cannot be unset). A key macOS keeps a
/// system-wide value of goes back to that value when it can be read, else
/// to the original, else to macOS's default.
pub(crate) fn restore_value(
    key: &str,
    original: &PropertyValue,
    current: &PropertyValue,
    global: Option<i64>,
) -> Option<PropertyValue> {
    let fallback = |prop: PropKey| match prop {
        PropKey::MouseAcceleration => FALLBACK_ACCELERATION,
        // Linear scaling and the F-key mode are off by default.
        PropKey::UserKeyMapping | PropKey::FKeyMode | PropKey::LinearScaling => 0,
    };
    match PropKey::from_name(key) {
        Some(PropKey::UserKeyMapping) => {
            let ours_before: Vec<KeyMapping> = key_mappings(original)
                .into_iter()
                .filter(|m| hidmap::is_managed_source(m.src))
                .collect();
            let restored = hidmap::merge(&key_mappings(current), &ours_before);
            Some(mapping_value(&restored))
        }
        Some(prop @ (PropKey::FKeyMode | PropKey::LinearScaling | PropKey::MouseAcceleration)) => {
            let value = match (global, original) {
                (Some(global), _) => global,
                (None, PropertyValue::Number(n)) => *n,
                (None, PropertyValue::Absent | PropertyValue::KeyMapping(_)) => fallback(prop),
            };
            Some(PropertyValue::Number(value))
        }
        // A key a newer Isthmus wrote goes back to its original, when there
        // is one to write.
        None => match original {
            PropertyValue::Absent => None,
            other => Some(other.clone()),
        },
    }
}

/// Reads and writes HID properties of live services.
pub(crate) trait PropertyIo {
    /// The current value, `None` when it cannot be read.
    fn read(&mut self, registry_id: u64, key: &str) -> Option<PropertyValue>;
    /// Writes `value`; whether the event system accepted it.
    fn write(&mut self, registry_id: u64, key: &str, value: &PropertyValue) -> bool;
    /// The system-wide value of `key`, if any.
    fn global(&mut self, key: &str) -> Option<i64>;
}

/// Where journal changes go. Recording must be durable before it returns
/// `Ok`: the property write follows only then.
pub(crate) trait JournalSink {
    /// Notes that `entry.key` is about to become `entry.applied`, on disk.
    fn record(&mut self, entry: JournalEntry) -> Result<(), Error>;
    /// The property is back (or someone else owns it now).
    fn forget(&mut self, registry_id: u64, key: &str);
    /// The service is gone, with every property Isthmus set on it.
    fn forget_service(&mut self, registry_id: u64);
}

/// The decision a restore follows (normally [`crate::journal::recover`]):
/// passed in so the execution order is tested on its own.
pub(crate) type Decide = fn(&JournalEntry, Option<&PropertyValue>) -> Option<Recovery>;

/// What [`execute`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ExecReport {
    /// Writes made and, where readable, verified.
    pub applied: u32,
    /// Properties put back.
    pub restored: u32,
    /// Restores skipped because someone else changed the value meanwhile.
    pub left: u32,
    /// (registry id, key) of writes that did not take, or were not made
    /// because the journal could not be saved first.
    pub failed: BTreeSet<(u64, &'static str)>,
    /// Restores that could not be written; their entries stay journalled.
    pub restore_failed: u32,
}

/// Step 3: carries out `steps` in order.
pub(crate) fn execute(
    steps: Vec<Step>,
    io: &mut impl PropertyIo,
    journal: &mut impl JournalSink,
    decide: Decide,
) -> ExecReport {
    let mut report = ExecReport::default();
    for step in steps {
        match step {
            Step::Apply {
                service,
                key,
                value,
                original,
            } => {
                let entry = JournalEntry {
                    registry_id: service.registry_id,
                    vendor_id: service.vendor_id,
                    product_id: service.product_id,
                    location_id: service.location_id,
                    key: key.name().to_owned(),
                    original,
                    applied: value.clone(),
                };
                if let Err(error) = journal.record(entry) {
                    warn!(
                        ?error,
                        key = key.name(),
                        "not writing a property the journal could not record"
                    );
                    report.failed.insert((service.registry_id, key.name()));
                    continue;
                }
                if !io.write(service.registry_id, key.name(), &value) {
                    warn!(
                        registry_id = service.registry_id,
                        key = key.name(),
                        "the HID event system refused a property"
                    );
                    report.failed.insert((service.registry_id, key.name()));
                    continue;
                }
                match io.read(service.registry_id, key.name()) {
                    Some(now) if !same_value(key, &now, &value) => {
                        warn!(
                            registry_id = service.registry_id,
                            key = key.name(),
                            "a written property reads back different"
                        );
                        report.failed.insert((service.registry_id, key.name()));
                    }
                    Some(_) | None => report.applied += 1,
                }
            }
            Step::Restore { entry, current } => {
                // Unreadable means "as Isthmus left it": recovery must not
                // depend on read-back, which some drivers never offer.
                let current = current.unwrap_or_else(|| entry.applied.clone());
                match decide(&entry, Some(&current)) {
                    Some(Recovery::Restore(original)) => {
                        let global = PropKey::from_name(&entry.key)
                            .filter(|k| k.has_global())
                            .and_then(|_| io.global(&entry.key));
                        match restore_value(&entry.key, &original, &current, global) {
                            // Nothing this build can write back (a newer
                            // key that was unset): keeping it would only
                            // show as pending forever.
                            None => {
                                journal.forget(entry.registry_id, &entry.key);
                                report.left += 1;
                            }
                            Some(value) if io.write(entry.registry_id, &entry.key, &value) => {
                                journal.forget(entry.registry_id, &entry.key);
                                report.restored += 1;
                            }
                            Some(_) => {
                                warn!(registry_id = entry.registry_id, key = %entry.key, "could not restore a property; it stays journalled");
                                report.restore_failed += 1;
                            }
                        }
                    }
                    Some(Recovery::Leave) => {
                        debug!(registry_id = entry.registry_id, key = %entry.key, "someone else changed a property; leaving it");
                        journal.forget(entry.registry_id, &entry.key);
                        report.left += 1;
                    }
                    None => journal.forget(entry.registry_id, &entry.key),
                }
            }
            Step::ForgetService { registry_id } => journal.forget_service(registry_id),
        }
    }
    report
}

/// The registry ids carrying Isthmus's intent now, and the counts the
/// status shows: services whose intent has a role and whose writes all
/// took.
pub(crate) fn in_place(
    intents: &[ServiceIntent],
    report: &ExecReport,
) -> (BTreeSet<u64>, u32, u32) {
    let failed: BTreeSet<u64> = report.failed.iter().map(|(rid, _)| *rid).collect();
    let mut applied = BTreeSet::new();
    let (mut keyboards, mut pointers) = (0, 0);
    for intent in intents {
        let Some(role) = intent.role else { continue };
        if failed.contains(&intent.registry_id) {
            continue;
        }
        applied.insert(intent.registry_id);
        match role {
            Role::MappedKeyboard => keyboards += 1,
            Role::TunedPointer => pointers += 1,
        }
    }
    (applied, keyboards, pointers)
}

/// Karabiner-Elements' virtual keyboard: present when Karabiner has seized
/// keyboards, whose events then no longer pass through their own services.
pub(crate) fn is_karabiner_virtual(service: &HidService) -> bool {
    service.manufacturer == "pqrs.org" || service.product.starts_with("Karabiner")
}

/// The issues the services show: Karabiner's virtual keyboard, and System
/// Settings modifier mappings on a keyboard (named by its device).
pub(crate) fn device_issues(devices: &[Device], observed: &BTreeMap<u64, Observed>) -> Vec<Issue> {
    let mut issues = Vec::new();
    if observed.values().any(|o| is_karabiner_virtual(&o.service)) {
        issues.push(Issue::KarabinerActive);
    }
    for device in devices.iter().filter(|d| d.kind == DeviceKind::Keyboard) {
        let modified = device.services.iter().any(|rid| {
            observed
                .get(rid)
                .is_some_and(|o| o.service.is_keyboard && o.system_modifier_keys)
        });
        if modified {
            issues.push(Issue::SystemModifierKeys {
                device: device.name.clone(),
            });
        }
    }
    issues
}

/// The journal file and its in-memory copy, kept in step: every record is
/// on disk before the property it protects is written.
pub(crate) struct JournalStore {
    path: PathBuf,
    pub(crate) journal: Journal,
    boot_session: String,
}

impl JournalStore {
    pub(crate) fn new(path: PathBuf, journal: Journal, boot_session: String) -> JournalStore {
        JournalStore {
            path,
            journal,
            boot_session,
        }
    }

    /// Writes the journal, or removes the file once nothing is journalled:
    /// an empty journal is the same as none, and no file is less to explain.
    fn save(&mut self) -> Result<(), Error> {
        if self.journal.entries.is_empty() {
            return match std::fs::remove_file(&self.path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(source) => Err(Error::WriteFile {
                    path: self.path.clone(),
                    source: isthmus_core::Error::Write {
                        path: self.path.clone(),
                        source,
                    },
                }),
            };
        }
        self.journal.version = Journal::VERSION;
        self.journal.boot_session.clone_from(&self.boot_session);
        write_journal(&self.path, &self.journal)
    }
}

impl JournalSink for JournalStore {
    /// A record that cannot be saved is taken back in memory too. Kept, it
    /// would make the next record of the same value look like no change,
    /// skip the save and let the write through with nothing on disk; and an
    /// unreadable key would be taken to hold the unsaved `applied` already.
    fn record(&mut self, entry: JournalEntry) -> Result<(), Error> {
        let before = self.journal.entries.clone();
        if self.journal.record(entry)
            && let Err(error) = self.save()
        {
            self.journal.entries = before;
            return Err(error);
        }
        Ok(())
    }

    fn forget(&mut self, registry_id: u64, key: &str) {
        if self.journal.forget(registry_id, key)
            && let Err(error) = self.save()
        {
            warn!(?error, "could not save the journal after a restore");
        }
    }

    fn forget_service(&mut self, registry_id: u64) {
        if self.journal.forget_service(registry_id)
            && let Err(error) = self.save()
        {
            warn!(?error, "could not save the journal after a service left");
        }
    }
}

/// Writes `journal` to `path` atomically.
pub(crate) fn write_journal(path: &Path, journal: &Journal) -> Result<(), Error> {
    let mut bytes = serde_json::to_vec_pretty(journal).map_err(|source| Error::Encode {
        what: "the journal",
        source,
    })?;
    bytes.push(b'\n');
    isthmus_core::durable_file::replace_contents(path, &bytes).map_err(|source| Error::WriteFile {
        path: path.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Layout;
    use crate::keys::usage;

    const KB: u64 = 0x1000_013E4;
    const MOUSE: u64 = 0x1000_06091;

    fn service(registry_id: u64, keyboard: bool) -> HidService {
        HidService {
            registry_id,
            vendor_id: 0x046D,
            product_id: 0xC54D,
            location_id: 0x0013_3000,
            is_keyboard: keyboard,
            is_pointer: !keyboard,
            pointer_acceleration_type: MOUSE_ACCELERATION_TYPE.into(),
            ..HidService::default()
        }
    }

    /// (registry id, is a keyboard, values read back).
    type Seen = (u64, bool, Vec<(PropKey, PropertyValue)>);

    fn observed(entries: Vec<Seen>) -> BTreeMap<u64, Observed> {
        entries
            .into_iter()
            .map(|(rid, keyboard, values)| {
                (
                    rid,
                    Observed {
                        service: service(rid, keyboard),
                        values: values.into_iter().collect(),
                        publishes_linear: !keyboard,
                        system_modifier_keys: false,
                    },
                )
            })
            .collect()
    }

    fn entry(
        rid: u64,
        key: PropKey,
        original: PropertyValue,
        applied: PropertyValue,
    ) -> JournalEntry {
        JournalEntry {
            registry_id: rid,
            vendor_id: 0x046D,
            product_id: 0xC54D,
            location_id: 0x0013_3000,
            key: key.name().into(),
            original,
            applied,
        }
    }

    fn journal(entries: Vec<JournalEntry>) -> Journal {
        Journal {
            version: Journal::VERSION,
            boot_session: "boot".into(),
            entries,
        }
    }

    fn mapping(pairs: &[(u64, u64)]) -> PropertyValue {
        PropertyValue::KeyMapping(pairs.to_vec())
    }

    fn intent(rid: u64, targets: Vec<(PropKey, PropertyValue)>) -> ServiceIntent {
        ServiceIntent {
            registry_id: rid,
            targets,
            role: None,
        }
    }

    /// A value already in place is not written again, whether it was read
    /// back or, unreadable, is what the journal says Isthmus wrote; so a
    /// reconcile on every hint never rewrites anything.
    #[test]
    fn values_already_in_place_are_not_rewritten() {
        let ours = mapping(&[(0x7_0000_00E0, 0x7_0000_00E7)]);
        let seen = observed(vec![
            (KB, true, vec![]),
            (
                MOUSE,
                false,
                vec![(PropKey::MouseAcceleration, PropertyValue::Number(65_536))],
            ),
        ]);
        let journal = journal(vec![entry(
            KB,
            PropKey::UserKeyMapping,
            PropertyValue::Absent,
            ours.clone(),
        )]);
        let intents = vec![
            intent(KB, vec![(PropKey::UserKeyMapping, ours)]),
            intent(
                MOUSE,
                vec![(PropKey::MouseAcceleration, PropertyValue::Number(65_536))],
            ),
        ];
        assert_eq!(
            diff(
                &intents,
                &seen,
                &journal,
                &BTreeSet::new(),
                &BTreeSet::new()
            ),
            vec![]
        );
    }

    /// An empty target mapping on a keyboard Isthmus never touched needs no
    /// write (absent and empty map nothing alike), and mapping order never
    /// counts as a difference.
    #[test]
    fn absent_and_empty_mappings_and_reordered_ones_are_the_same() {
        let a = mapping(&[(1, 2), (3, 4)]);
        let b = mapping(&[(3, 4), (1, 2)]);
        assert!(same_value(PropKey::UserKeyMapping, &a, &b));
        assert!(same_value(
            PropKey::UserKeyMapping,
            &PropertyValue::Absent,
            &mapping(&[])
        ));
        let seen = observed(vec![(KB, true, vec![])]);
        let intents = vec![intent(KB, vec![(PropKey::UserKeyMapping, mapping(&[]))])];
        assert_eq!(
            diff(
                &intents,
                &seen,
                &journal(vec![]),
                &BTreeSet::new(),
                &BTreeSet::new()
            ),
            vec![]
        );
    }

    /// A pointer service gets the linear flag before the gain, and each
    /// write carries what the property held before as its original.
    #[test]
    fn pointer_writes_come_flag_first_with_their_originals() {
        let seen = observed(vec![(
            MOUSE,
            false,
            vec![
                (PropKey::LinearScaling, PropertyValue::Number(0)),
                (PropKey::MouseAcceleration, PropertyValue::Number(131_072)),
            ],
        )]);
        let intents = vec![intent(
            MOUSE,
            vec![
                (PropKey::MouseAcceleration, PropertyValue::Number(52_429)),
                (PropKey::LinearScaling, PropertyValue::Number(1)),
            ],
        )];
        let steps = diff(
            &intents,
            &seen,
            &journal(vec![]),
            &BTreeSet::new(),
            &BTreeSet::new(),
        );
        let keys: Vec<(PropKey, PropertyValue)> = steps
            .iter()
            .map(|s| match s {
                Step::Apply { key, original, .. } => (*key, original.clone()),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(
            keys,
            vec![
                (PropKey::LinearScaling, PropertyValue::Number(0)),
                (PropKey::MouseAcceleration, PropertyValue::Number(131_072)),
            ]
        );
    }

    /// After a system-wide push (System Settings' tracking speed), an
    /// unreadable value is no longer trusted to be ours and is rewritten.
    #[test]
    fn a_forced_key_is_rewritten_when_it_cannot_be_read_back() {
        let gain = PropertyValue::Number(65_536);
        let seen = observed(vec![(MOUSE, false, vec![])]);
        let journal = journal(vec![entry(
            MOUSE,
            PropKey::MouseAcceleration,
            PropertyValue::Absent,
            gain.clone(),
        )]);
        let intents = vec![intent(MOUSE, vec![(PropKey::MouseAcceleration, gain)])];
        assert!(
            diff(
                &intents,
                &seen,
                &journal,
                &BTreeSet::new(),
                &BTreeSet::new()
            )
            .is_empty()
        );
        let force = BTreeSet::from([PropKey::MouseAcceleration]);
        assert_eq!(
            diff(&intents, &seen, &journal, &force, &BTreeSet::new()).len(),
            1
        );
    }

    /// A journalled key the intent no longer lists goes back (switching a
    /// keyboard to Mac style, a mouse to the system pointer), and a service
    /// that is gone is forgotten rather than written.
    #[test]
    fn untargeted_keys_are_restored_and_vanished_services_forgotten() {
        let seen = observed(vec![(KB, true, vec![])]);
        let journal = journal(vec![
            entry(
                KB,
                PropKey::UserKeyMapping,
                PropertyValue::Absent,
                mapping(&[(1, 2)]),
            ),
            entry(
                KB,
                PropKey::FKeyMode,
                PropertyValue::Number(0),
                PropertyValue::Number(1),
            ),
            entry(
                MOUSE,
                PropKey::MouseAcceleration,
                PropertyValue::Number(131_072),
                PropertyValue::Number(65_536),
            ),
        ]);
        let steps = diff(&[], &seen, &journal, &BTreeSet::new(), &BTreeSet::new());
        assert!(
            matches!(&steps[0], Step::Restore { entry, current: None } if entry.key == "UserKeyMapping")
        );
        assert!(matches!(&steps[1], Step::Restore { entry, .. } if entry.key == "HIDFKeyMode"));
        assert_eq!(steps[2], Step::ForgetService { registry_id: MOUSE });
        assert_eq!(steps.len(), 3);
    }

    /// Restored values: an absent mapping becomes an empty array; a key
    /// with a system-wide value takes that value over a stale original; the
    /// original serves when the system-wide one cannot be read.
    #[test]
    fn restores_prefer_the_system_value_for_keys_macos_keeps_one_of() {
        assert_eq!(
            restore_value(
                "UserKeyMapping",
                &PropertyValue::Absent,
                &PropertyValue::Absent,
                None
            ),
            Some(mapping(&[]))
        );
        assert_eq!(
            restore_value(
                "HIDMouseAcceleration",
                &PropertyValue::Number(131_072),
                &PropertyValue::Absent,
                Some(98_304)
            ),
            Some(PropertyValue::Number(98_304))
        );
        assert_eq!(
            restore_value(
                "HIDMouseAcceleration",
                &PropertyValue::Number(131_072),
                &PropertyValue::Absent,
                None
            ),
            Some(PropertyValue::Number(131_072))
        );
        assert_eq!(
            restore_value(
                "HIDUseLinearScalingMouseAcceleration",
                &PropertyValue::Absent,
                &PropertyValue::Absent,
                None
            ),
            Some(PropertyValue::Number(0))
        );
        assert_eq!(
            restore_value(
                "SomethingNewer",
                &PropertyValue::Absent,
                &PropertyValue::Absent,
                None
            ),
            None
        );
    }

    /// A key mapping goes back to its original's modifier entries, while
    /// every other source keeps what it holds now: an entry the user added
    /// or changed with `hidutil` while Isthmus ran survives the restore, and
    /// one the user removed meanwhile does not come back.
    #[test]
    fn a_restored_key_mapping_keeps_entries_someone_else_set_meanwhile() {
        let caps_to_escape = (usage::CAPS_LOCK, 0x7_0000_0029);
        let caps_to_control = (usage::CAPS_LOCK, usage::LEFT_CONTROL);
        let users_swap = (usage::LEFT_ALT, usage::LEFT_GUI);
        let ours = (usage::LEFT_CONTROL, usage::RIGHT_GUI);
        let key = PropKey::UserKeyMapping.name();
        assert_eq!(
            restore_value(
                key,
                &PropertyValue::Absent,
                &mapping(&[caps_to_escape, ours]),
                None
            ),
            Some(mapping(&[caps_to_escape]))
        );
        assert_eq!(
            restore_value(
                key,
                &mapping(&[caps_to_control, users_swap]),
                &mapping(&[caps_to_escape, ours]),
                None
            ),
            Some(mapping(&[caps_to_escape, users_swap]))
        );
        assert_eq!(
            restore_value(key, &mapping(&[caps_to_control]), &mapping(&[ours]), None),
            Some(mapping(&[]))
        );

        // The whole path: a keyboard with no mapping at start, the user's
        // CapsLock → Escape merged in by a later reconcile, then a quit.
        let e = entry(
            KB,
            PropKey::UserKeyMapping,
            PropertyValue::Absent,
            mapping(&[caps_to_escape, ours]),
        );
        let (mut io, mut sink) = (FakeIo::default(), FakeJournal::default());
        let steps = vec![Step::Restore {
            entry: e,
            current: Some(mapping(&[ours, caps_to_escape])),
        }];
        let report = execute(steps, &mut io, &mut sink, crate::journal::recover);
        assert_eq!(report.restored, 1);
        assert_eq!(
            io.values.get(&(KB, key.to_owned())),
            Some(&mapping(&[caps_to_escape]))
        );
    }

    /// A service that is both a keyboard and a pointer belongs to a keyboard
    /// and a mouse device, and gets the key mapping and the Windows pointer
    /// speed both, not only whichever device sorts first.
    #[test]
    fn a_service_that_is_keyboard_and_pointer_gets_both_intents() {
        let combo = HidService {
            product: "Wireless Touch Keyboard K400 Plus".into(),
            is_keyboard: true,
            is_pointer: true,
            ..service(KB, true)
        };
        let seen = BTreeMap::from([(
            KB,
            Observed {
                service: combo.clone(),
                values: BTreeMap::new(),
                publishes_linear: true,
                system_modifier_keys: false,
            },
        )]);
        let devices = device::group_services(&[combo]);
        let kinds: Vec<DeviceKind> = devices.iter().map(|d| d.kind).collect();
        assert_eq!(kinds, [DeviceKind::Keyboard, DeviceKind::Mouse]);
        let desired = Desired {
            active: true,
            settings: Settings::default(),
            hid_profile: HidProfile::Windows,
            display_scale: 1.0,
        };
        let none = journal(vec![]);
        let wanted = intents(&devices, &seen, &none, &desired);
        let steps = diff(&wanted, &seen, &none, &BTreeSet::new(), &BTreeSet::new());
        let keys: Vec<PropKey> = steps
            .iter()
            .map(|s| match s {
                Step::Apply { key, .. } => *key,
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(
            keys,
            [
                PropKey::UserKeyMapping,
                PropKey::LinearScaling,
                PropKey::MouseAcceleration
            ]
        );
        assert_eq!(
            in_place(&wanted, &ExecReport::default()),
            (BTreeSet::from([KB]), 1, 1)
        );
    }

    /// A Windows-style Apple keyboard gets its F-keys as F-keys over
    /// Bluetooth (Apple's Bluetooth vendor id, 0x004C) as over USB (0x05AC);
    /// another vendor's keyboard, whose F-keys already are, is not touched.
    #[test]
    fn apple_keyboards_get_standard_f_keys_over_bluetooth_and_usb() {
        let desired = Desired {
            active: true,
            settings: Settings::default(),
            hid_profile: HidProfile::Windows,
            display_scale: 1.0,
        };
        for (vendor_id, transport, f_keys) in [
            (0x004C, "Bluetooth", true),
            (0x05AC, "USB", true),
            (0x046D, "USB", false),
        ] {
            let keyboard = HidService {
                vendor_id,
                transport: transport.into(),
                product: "Magic Keyboard".into(),
                ..service(KB, true)
            };
            let seen = BTreeMap::from([(
                KB,
                Observed {
                    service: keyboard.clone(),
                    values: BTreeMap::new(),
                    publishes_linear: false,
                    system_modifier_keys: false,
                },
            )]);
            let devices = device::group_services(&[keyboard]);
            let wanted = intents(&devices, &seen, &journal(vec![]), &desired);
            let has_f_keys = wanted.iter().any(|i| {
                i.targets
                    .contains(&(PropKey::FKeyMode, PropertyValue::Number(1)))
            });
            assert_eq!(has_f_keys, f_keys, "{vendor_id:#06x}");
        }
    }

    /// A write that failed last time is made again even when the value
    /// cannot be read back and the journal says Isthmus wrote it: a refused
    /// write never changed the property, so the journal cannot vouch for it.
    #[test]
    fn a_failed_write_of_an_unreadable_key_is_retried_rather_than_trusted() {
        let ours = mapping(&[(usage::LEFT_CONTROL, usage::RIGHT_GUI)]);
        let seen = observed(vec![(KB, true, vec![])]);
        let journal = journal(vec![entry(
            KB,
            PropKey::UserKeyMapping,
            PropertyValue::Absent,
            ours.clone(),
        )]);
        let intents = vec![intent(KB, vec![(PropKey::UserKeyMapping, ours)])];
        let none = BTreeSet::new();
        assert!(diff(&intents, &seen, &journal, &none, &BTreeSet::new()).is_empty());
        let retry = BTreeSet::from([(KB, PropKey::UserKeyMapping.name())]);
        let steps = diff(&intents, &seen, &journal, &none, &retry);
        assert!(
            matches!(
                &steps[..],
                [Step::Apply {
                    key: PropKey::UserKeyMapping,
                    ..
                }]
            ),
            "{steps:?}"
        );
    }

    /// A record the journal file cannot take is taken back in memory too,
    /// so the same write asked for again fails again rather than finding
    /// "nothing new to save" and going through with nothing on disk.
    #[test]
    fn a_record_that_cannot_be_saved_never_lets_a_later_write_through() {
        let missing = std::env::temp_dir()
            .join(format!("isthmus-no-such-dir-{}", std::process::id()))
            .join("journal.json");
        assert!(missing.parent().is_some_and(|dir| !dir.exists()));
        let before = vec![entry(
            KB,
            PropKey::UserKeyMapping,
            PropertyValue::Absent,
            mapping(&[(usage::LEFT_CONTROL, usage::RIGHT_GUI)]),
        )];
        let mut store = JournalStore::new(missing, journal(before.clone()), "boot".into());
        let mut io = FakeIo::default();
        for _ in 0..2 {
            let report = execute(
                vec![
                    apply(
                        MOUSE,
                        PropKey::MouseAcceleration,
                        PropertyValue::Number(65_536),
                    ),
                    apply(KB, PropKey::UserKeyMapping, mapping(&[])),
                ],
                &mut io,
                &mut store,
                restore_if_unchanged,
            );
            assert_eq!(report.failed.len(), 2, "{report:?}");
        }
        assert!(io.log.is_empty(), "{:?}", io.log);
        assert_eq!(store.journal.entries, before);
    }

    #[derive(Default)]
    struct FakeIo {
        log: Vec<String>,
        values: BTreeMap<(u64, String), PropertyValue>,
        refuse: bool,
        global: Option<i64>,
    }

    impl PropertyIo for FakeIo {
        fn read(&mut self, rid: u64, key: &str) -> Option<PropertyValue> {
            self.values.get(&(rid, key.to_owned())).cloned()
        }
        fn write(&mut self, rid: u64, key: &str, value: &PropertyValue) -> bool {
            self.log.push(format!("write {key}"));
            if !self.refuse {
                self.values.insert((rid, key.to_owned()), value.clone());
            }
            !self.refuse
        }
        fn global(&mut self, _key: &str) -> Option<i64> {
            self.global
        }
    }

    #[derive(Default)]
    struct FakeJournal {
        log: Vec<String>,
        broken: bool,
    }

    impl JournalSink for FakeJournal {
        fn record(&mut self, entry: JournalEntry) -> Result<(), Error> {
            if self.broken {
                return Err(Error::NotRunning);
            }
            self.log.push(format!("record {}", entry.key));
            Ok(())
        }
        fn forget(&mut self, _rid: u64, key: &str) {
            self.log.push(format!("forget {key}"));
        }
        fn forget_service(&mut self, rid: u64) {
            self.log.push(format!("forget service {rid}"));
        }
    }

    fn apply(rid: u64, key: PropKey, value: PropertyValue) -> Step {
        Step::Apply {
            service: service(rid, key == PropKey::UserKeyMapping),
            key,
            value,
            original: PropertyValue::Absent,
        }
    }

    fn restore_if_unchanged(
        entry: &JournalEntry,
        current: Option<&PropertyValue>,
    ) -> Option<Recovery> {
        match current {
            Some(now) if *now == entry.applied => Some(Recovery::Restore(entry.original.clone())),
            Some(_) => Some(Recovery::Leave),
            None => None,
        }
    }

    /// Every write is journalled first; when the journal cannot be saved the
    /// property is not written at all, so a crash can never leave a change
    /// the next start does not know about.
    #[test]
    fn writes_happen_only_after_the_journal_has_them() {
        let (mut io, mut sink) = (FakeIo::default(), FakeJournal::default());
        let report = execute(
            vec![apply(
                MOUSE,
                PropKey::MouseAcceleration,
                PropertyValue::Number(65_536),
            )],
            &mut io,
            &mut sink,
            restore_if_unchanged,
        );
        assert_eq!(sink.log, vec!["record HIDMouseAcceleration"]);
        assert_eq!(io.log, vec!["write HIDMouseAcceleration"]);
        assert_eq!(report.applied, 1);

        let (mut io, mut sink) = (
            FakeIo::default(),
            FakeJournal {
                broken: true,
                ..FakeJournal::default()
            },
        );
        let report = execute(
            vec![apply(
                MOUSE,
                PropKey::MouseAcceleration,
                PropertyValue::Number(65_536),
            )],
            &mut io,
            &mut sink,
            restore_if_unchanged,
        );
        assert!(io.log.is_empty());
        assert!(report.failed.contains(&(MOUSE, "HIDMouseAcceleration")));
    }

    /// A refused write, or one that reads back different, is a failure and
    /// keeps the service out of the applied set and the counts.
    #[test]
    fn refused_or_unverified_writes_do_not_count_as_applied() {
        let (mut io, mut sink) = (
            FakeIo {
                refuse: true,
                ..FakeIo::default()
            },
            FakeJournal::default(),
        );
        let report = execute(
            vec![apply(
                MOUSE,
                PropKey::MouseAcceleration,
                PropertyValue::Number(65_536),
            )],
            &mut io,
            &mut sink,
            restore_if_unchanged,
        );
        let intents = vec![ServiceIntent {
            registry_id: MOUSE,
            targets: vec![],
            role: Some(Role::TunedPointer),
        }];
        assert_eq!(in_place(&intents, &report), (BTreeSet::new(), 0, 0));
        assert_eq!(
            in_place(&intents, &ExecReport::default()),
            (BTreeSet::from([MOUSE]), 0, 1)
        );
    }

    /// A restore writes back only over a value still as Isthmus left it (an
    /// unreadable one counts as that), forgets the entry once written, keeps
    /// it when the write fails, and leaves someone else's value alone.
    #[test]
    fn restores_respect_foreign_changes_and_keep_failures_journalled() {
        let ours = PropertyValue::Number(65_536);
        let e = entry(
            MOUSE,
            PropKey::MouseAcceleration,
            PropertyValue::Number(131_072),
            ours.clone(),
        );

        let (mut io, mut sink) = (FakeIo::default(), FakeJournal::default());
        let steps = vec![Step::Restore {
            entry: e.clone(),
            current: None,
        }];
        let report = execute(steps, &mut io, &mut sink, restore_if_unchanged);
        assert_eq!(report.restored, 1);
        assert_eq!(
            io.values.get(&(MOUSE, "HIDMouseAcceleration".into())),
            Some(&PropertyValue::Number(131_072))
        );
        assert_eq!(sink.log, vec!["forget HIDMouseAcceleration"]);

        let (mut io, mut sink) = (FakeIo::default(), FakeJournal::default());
        let steps = vec![Step::Restore {
            entry: e.clone(),
            current: Some(PropertyValue::Number(7)),
        }];
        let report = execute(steps, &mut io, &mut sink, restore_if_unchanged);
        assert_eq!((report.left, report.restored), (1, 0));
        assert!(io.log.is_empty());

        let (mut io, mut sink) = (
            FakeIo {
                refuse: true,
                ..FakeIo::default()
            },
            FakeJournal::default(),
        );
        let steps = vec![Step::Restore {
            entry: e,
            current: Some(ours),
        }];
        let report = execute(steps, &mut io, &mut sink, restore_if_unchanged);
        assert_eq!(report.restore_failed, 1);
        assert!(sink.log.is_empty());
    }

    /// Karabiner's virtual keyboard and System Settings modifier mappings
    /// show as issues, the latter named by the device's name.
    #[test]
    fn karabiner_and_system_modifier_keys_become_issues() {
        let mut seen = observed(vec![(KB, true, vec![])]);
        if let Some(o) = seen.get_mut(&KB) {
            o.system_modifier_keys = true;
        }
        let mut karabiner = service(7, true);
        karabiner.manufacturer = "pqrs.org".into();
        seen.insert(
            7,
            Observed {
                service: karabiner,
                values: BTreeMap::new(),
                publishes_linear: false,
                system_modifier_keys: false,
            },
        );
        let device = Device {
            id: crate::device::DeviceId("kbd".into()),
            kind: DeviceKind::Keyboard,
            name: "Keychron K8".into(),
            manufacturer: String::new(),
            vendor_id: 0,
            product_id: 0,
            transport: "USB".into(),
            built_in: false,
            services: vec![KB],
            detected_layout: Layout::Mac,
            layout_confidence: crate::device::Confidence::Medium,
            via_kvm: false,
            has_mode_switch: false,
        };
        assert_eq!(
            device_issues(&[device], &seen),
            vec![
                Issue::KarabinerActive,
                Issue::SystemModifierKeys {
                    device: "Keychron K8".into()
                }
            ]
        );
    }
}
