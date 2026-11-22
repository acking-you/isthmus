//! Windows: a power request of type `PowerRequestSystemRequired`, which
//! keeps the system from sleeping when idle and leaves the display alone
//! (that would be `PowerRequestDisplayRequired`, which is not set). Unlike
//! `SetThreadExecutionState` it belongs to a handle rather than a thread,
//! so it holds whichever thread the app calls from, and `powercfg
//! /requests` lists it under its reason. Windows clears it when the process
//! exits.

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Power::{
    PowerClearRequest, PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest,
};
use windows_sys::Win32::System::Threading::{
    POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
};

use crate::Held;

pub(crate) const SUPPORTED: bool = true;

/// `POWER_REQUEST_CONTEXT_VERSION`, which windows-sys keeps in a module this
/// crate otherwise has no use for.
const POWER_REQUEST_CONTEXT_VERSION: u32 = 0;

/// A set power request, cleared and closed on drop.
pub(crate) struct Request {
    handle: HANDLE,
    /// The reason's UTF-16, kept alive as long as the request that names it.
    _reason: Vec<u16>,
}

// SAFETY: a power request handle is a kernel object handle, usable and
// closable from any thread.
unsafe impl Send for Request {}

impl Held for Request {}

pub(crate) fn hold(reason: &str) -> Result<Request, String> {
    let mut wide: Vec<u16> = reason.encode_utf16().chain(Some(0)).collect();
    let context = REASON_CONTEXT {
        Version: POWER_REQUEST_CONTEXT_VERSION,
        Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
        Reason: REASON_CONTEXT_0 {
            SimpleReasonString: wide.as_mut_ptr(),
        },
    };
    // SAFETY: `context` is a valid REASON_CONTEXT whose string is
    // NUL-terminated and outlives the call.
    let handle = unsafe { PowerCreateRequest(&context) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(format!(
            "PowerCreateRequest failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: `handle` is the power request just created.
    if unsafe { PowerSetRequest(handle, PowerRequestSystemRequired) } == 0 {
        let error = std::io::Error::last_os_error();
        // SAFETY: as above; it is closed once, here.
        unsafe { CloseHandle(handle) };
        return Err(format!("PowerSetRequest failed: {error}"));
    }
    Ok(Request {
        handle,
        _reason: wide,
    })
}

impl Drop for Request {
    fn drop(&mut self) {
        // SAFETY: the request set in `hold`, cleared and closed once, here.
        unsafe {
            PowerClearRequest(self.handle, PowerRequestSystemRequired);
            CloseHandle(self.handle);
        }
    }
}
