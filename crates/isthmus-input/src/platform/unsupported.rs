//! Every OS but macOS: nothing to list, recover or watch. The settings and
//! the pure model still work there; the service runs without applying.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::RecoveryReport;
use crate::device::HidService;
use crate::error::Error;

pub(super) fn list_services() -> Result<Vec<HidService>, Error> {
    Err(Error::Unsupported {
        what: "listing HID devices",
    })
}

pub(super) fn recover_journal(_data_dir: &Path) -> Result<RecoveryReport, Error> {
    Err(Error::Unsupported {
        what: "restoring HID properties",
    })
}

pub(super) fn inspect_events(
    _stop: Arc<AtomicBool>,
    _sink: impl FnMut(String),
) -> Result<(), Error> {
    Err(Error::Unsupported {
        what: "watching input events",
    })
}
