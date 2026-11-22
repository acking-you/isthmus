//! The handle Dart holds: a view of the one service in this process.
//!
//! HID mappings, pointer properties and the event tap are machine-wide, and
//! Swift's quit hook must reach the service without any Dart handle, so the
//! service lives in a process-wide [`Host`], not in the handle. A handle is
//! only a pointer to that host: Dart's hot restart drops its handles and
//! creates new ones, and finds the same running service. Destroying a handle
//! never stops the service; `isthmus_shutdown` does.

use crate::host::{self, Host};
use crate::logging;

/// Opaque to Dart, which passes its address between isolates.
pub struct IsthmusHandle {
    pub(crate) host: &'static Host,
}

impl IsthmusHandle {
    /// A handle on `host`; the exports use the process's own host, tests a
    /// host with a fake service.
    pub(crate) fn new(host: &'static Host) -> Self {
        Self { host }
    }
}

/// A new handle on the process's service, freed with `isthmus_destroy`.
/// Also sets up logging to stderr, once per process.
#[unsafe(no_mangle)]
pub extern "C" fn isthmus_create() -> *mut IsthmusHandle {
    logging::init();
    Box::into_raw(Box::new(IsthmusHandle::new(host::global())))
}

/// Frees a handle. The service keeps running.
///
/// # Safety
/// `handle` must be null or come from `isthmus_create`, destroyed once, with
/// every call borrowing it returned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn isthmus_destroy(handle: *mut IsthmusHandle) {
    if !handle.is_null() {
        // SAFETY: from `Box::into_raw` in `isthmus_create`, destroyed once
        // with no call in flight, as the caller guarantees.
        unsafe { drop(Box::from_raw(handle)) };
    }
}
