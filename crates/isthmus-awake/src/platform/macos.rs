//! macOS: an IOKit power assertion of type `PreventUserIdleSystemSleep`,
//! what `caffeinate -i` takes. It stops idle sleep and nothing else: the
//! display sleeps on its own timer (that would be
//! `PreventUserIdleDisplaySleep`, which is not taken), and closing the lid
//! or choosing Sleep still sleeps. The assertion is the process's: macOS
//! releases it when the process exits.

use objc2_core_foundation::CFString;
use objc2_io_kit::{
    IOPMAssertionCreateWithName, IOPMAssertionID, IOPMAssertionRelease, kIOPMAssertionLevelOn,
    kIOReturnSuccess,
};

use crate::Held;

pub(crate) const SUPPORTED: bool = true;

/// `kIOPMAssertPreventUserIdleSystemSleep`, a `CFSTR` macro in the headers
/// and so not among the bindings.
const PREVENT_IDLE_SYSTEM_SLEEP: &str = "PreventUserIdleSystemSleep";

/// A held assertion, released on drop.
pub(crate) struct Assertion(IOPMAssertionID);

impl Held for Assertion {}

pub(crate) fn hold(reason: &str) -> Result<Assertion, String> {
    let kind = CFString::from_static_str(PREVENT_IDLE_SYSTEM_SLEEP);
    let name = CFString::from_str(reason);
    let mut id: IOPMAssertionID = 0;
    // SAFETY: both strings are live CFStrings for the call, and `id` is a
    // valid place for the new assertion's id.
    let result = unsafe {
        IOPMAssertionCreateWithName(Some(&kind), kIOPMAssertionLevelOn, Some(&name), &mut id)
    };
    if result == kIOReturnSuccess {
        Ok(Assertion(id))
    } else {
        Err(format!("IOPMAssertionCreateWithName returned {result:#x}"))
    }
}

impl Drop for Assertion {
    fn drop(&mut self) {
        let result = IOPMAssertionRelease(self.0);
        if result != kIOReturnSuccess {
            tracing::warn!(result, "the power assertion was not released");
        }
    }
}
