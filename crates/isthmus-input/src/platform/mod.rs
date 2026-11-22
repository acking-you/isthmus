//! Operating-system access. Only `macos/` names an OS API; every other OS
//! gets `unsupported`, which reports [`crate::Error::Unsupported`].
//!
//! The public items here serve the CLI's diagnostics without starting the
//! service: listing HID services, recovering a journal left by a crash, and
//! watching raw events.

#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(not(target_os = "macos"))]
mod unsupported;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::device::HidService;
use crate::error::Error;

/// Every keyboard, mouse and trackpad HID service, read once. Needs no
/// permission.
pub fn list_services() -> Result<Vec<HidService>, Error> {
    #[cfg(target_os = "macos")]
    {
        macos::hid::list_services()
    }
    #[cfg(not(target_os = "macos"))]
    {
        unsupported::list_services()
    }
}

/// What [`recover_journal`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Properties put back to their original values.
    pub restored: u32,
    /// Properties someone else had changed meanwhile, left alone.
    pub left: u32,
    /// Properties that could not be written; still journalled.
    pub failed: u32,
}

/// Puts back what a previous run journalled and did not restore, without
/// starting the service. Fails with [`Error::AnotherInstance`] while a
/// running Isthmus holds the lock.
pub fn recover_journal(data_dir: &Path) -> Result<RecoveryReport, Error> {
    #[cfg(target_os = "macos")]
    {
        recover_journal_macos(data_dir)
    }
    #[cfg(not(target_os = "macos"))]
    {
        unsupported::recover_journal(data_dir)
    }
}

/// The journal belongs to whoever holds the instance lock, so recovery
/// takes it first; a journal from another boot is deleted unread, since the
/// restart already undid it.
#[cfg(target_os = "macos")]
fn recover_journal_macos(data_dir: &Path) -> Result<RecoveryReport, Error> {
    use crate::service::{JournalStart, discard_journal, journal_start, load_journal};
    use isthmus_core::{boot, instance::InstanceLock, paths};

    if !data_dir.exists() {
        return Ok(RecoveryReport::default());
    }
    let _lock = InstanceLock::try_acquire(&paths::lock_file(data_dir))
        .map_err(|source| Error::DataDir { source })?
        .ok_or(Error::AnotherInstance)?;
    let path = paths::journal_file(data_dir);
    let journal = load_journal(&path);
    let boot = boot::boot_session_id();
    match journal_start(&journal, boot.as_deref()) {
        JournalStart::Fresh => Ok(RecoveryReport::default()),
        JournalStart::Discard => {
            discard_journal(&path);
            Ok(RecoveryReport::default())
        }
        JournalStart::Recover => {
            let mut store =
                macos::devices::JournalStore::new(path, journal, boot.unwrap_or_default());
            let report = macos::hid::recover_once(&mut store)?;
            Ok(RecoveryReport {
                restored: report.restored,
                left: report.left,
                failed: report.restore_failed,
            })
        }
    }
}

/// Watches raw key, flag, button and wheel events with a listen-only tap
/// and hands each one, formatted with the fields Isthmus reads, to `sink`
/// until `stop` is set. For checking what a device actually sends.
pub fn inspect_events(stop: Arc<AtomicBool>, sink: impl FnMut(String)) -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    {
        macos::inspect::run(stop, sink)
    }
    #[cfg(not(target_os = "macos"))]
    {
        unsupported::inspect_events(stop, sink)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// Listing HID services only reads (a Passive client, property reads),
    /// needs no permission, and returns nothing but keyboards and pointers.
    /// Run with `--nocapture` to see this Mac's devices.
    #[test]
    fn listing_services_reads_keyboards_and_pointers_without_permission() {
        let services = list_services().expect("the HID event system answers a read-only client");
        for s in &services {
            println!(
                "{:#x} {:04x}:{:04x} loc={:#010x} kb={} ptr={} builtIn={} {:?} / {:?} transport={:?} serial={:?} usage={}:{} scroll={:?} pointer={:?}",
                s.registry_id,
                s.vendor_id,
                s.product_id,
                s.location_id,
                s.is_keyboard,
                s.is_pointer,
                s.built_in,
                s.product,
                s.manufacturer,
                s.transport,
                s.serial_number,
                s.primary_usage_page,
                s.primary_usage,
                s.scroll_acceleration_type,
                s.pointer_acceleration_type,
            );
        }
        assert!(services.iter().all(|s| s.is_keyboard || s.is_pointer));
    }
}
