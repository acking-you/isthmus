//! The boot session, which tells a journal written before a restart (whose
//! HID changes the restart already undid) from one of this boot.
//!
//! macOS gives every boot a fresh UUID in `kern.bootsessionuuid`. Comparing
//! it is exact where comparing boot times is not (the clock can be set), and
//! it is readable without any privilege.

/// `kern.bootsessionuuid` on macOS; `None` elsewhere or if unreadable.
pub fn boot_session_id() -> Option<String> {
    imp::boot_session_id()
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::CStr;

    /// The sysctl name, NUL-terminated for `sysctlbyname`.
    const NAME: &CStr = c"kern.bootsessionuuid";

    /// Enough for the 36-character UUID and its NUL; the value is asked for
    /// at its real size first, so this only bounds a kernel that misreports.
    const MAX_LEN: usize = 256;

    /// Reads the string sysctl in two steps: its size, then its bytes.
    pub(super) fn boot_session_id() -> Option<String> {
        let mut len: libc::size_t = 0;
        // SAFETY: `NAME` is NUL-terminated; a null `oldp` asks only for the
        // size, which the kernel writes to `len`, a valid `size_t`; no new
        // value is set (`newp` null, `newlen` 0).
        let status = unsafe {
            libc::sysctlbyname(
                NAME.as_ptr(),
                std::ptr::null_mut(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 || len == 0 || len > MAX_LEN {
            return None;
        }
        let mut buffer = vec![0u8; len];
        // SAFETY: `buffer` is valid for writes of `len` bytes and `len` says
        // so; the kernel writes at most `len` bytes and stores the count it
        // wrote back into `len`. No new value is set.
        let status = unsafe {
            libc::sysctlbyname(
                NAME.as_ptr(),
                buffer.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 {
            return None;
        }
        buffer.truncate(len);
        while buffer.last() == Some(&0) {
            buffer.pop();
        }
        String::from_utf8(buffer).ok().filter(|id| !id.is_empty())
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    /// No boot session elsewhere: a journal is then never matched to a boot,
    /// which the caller treats as "cannot tell".
    pub(super) fn boot_session_id() -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// On macOS the id is the boot's UUID, `8-4-4-4-12` hex digits with no
    /// trailing NUL, and reading it twice gives the same value.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_boot_session_is_a_stable_uuid_on_macos() {
        let id = boot_session_id().unwrap();
        let groups: Vec<&str> = id.split('-').collect();
        let lengths: Vec<usize> = groups.iter().map(|group| group.len()).collect();
        assert_eq!(lengths, [8, 4, 4, 4, 12], "{id:?}");
        assert!(
            groups
                .iter()
                .all(|group| group.chars().all(|c| c.is_ascii_hexdigit())),
            "{id:?}"
        );
        assert_eq!(boot_session_id(), Some(id));
    }

    /// Without the macOS sysctl there is no boot session to report.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn there_is_no_boot_session_elsewhere() {
        assert_eq!(boot_session_id(), None);
    }
}
