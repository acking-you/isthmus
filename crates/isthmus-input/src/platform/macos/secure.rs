//! Secure Event Input: while any app holds it (a password field, a
//! terminal's Secure Keyboard Entry), no event tap receives key events, so
//! shortcut translation pauses; the HID mapping keeps working.
//!
//! Polled at 1 Hz off the tap thread: there is no notification for it.

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    /// HIToolbox (`CarbonEventsCore.h`); `objc2-carbon` has no binding.
    fn IsSecureEventInputEnabled() -> u8;
}

/// Whether some process has Secure Event Input on now.
pub(crate) fn enabled() -> bool {
    // SAFETY: a query with no arguments and no preconditions.
    unsafe { IsSecureEventInputEnabled() != 0 }
}
