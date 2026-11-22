//! Launch at login, through `SMAppService.mainAppService` (macOS 13+).
//!
//! Only an app bundle can be a login item: run from a bare binary (the
//! CLI, `cargo test`) the item would point at a build directory, so it
//! reads as unavailable there and is never registered.

use std::path::Path;

use objc2::runtime::AnyClass;
use objc2_service_management::{SMAppService, SMAppServiceStatus};

use crate::error::Error;
use crate::status::LoginItem;

/// The login item's state as the UI shows it.
pub(crate) fn from_status(status: SMAppServiceStatus) -> LoginItem {
    match status {
        SMAppServiceStatus::Enabled => LoginItem::Enabled,
        SMAppServiceStatus::NotRegistered => LoginItem::Disabled,
        SMAppServiceStatus::RequiresApproval => LoginItem::RequiresApproval,
        _ => LoginItem::Unavailable,
    }
}

/// Whether `executable` is the main executable of an app bundle.
pub(crate) fn is_bundle_executable(executable: &Path) -> bool {
    let mut parts = executable.ancestors().skip(1);
    let macos = parts.next().and_then(Path::file_name);
    let contents = parts.next().and_then(Path::file_name);
    let bundle = parts.next().and_then(Path::extension);
    macos.is_some_and(|n| n == "MacOS")
        && contents.is_some_and(|n| n == "Contents")
        && bundle.is_some_and(|e| e == "app")
}

/// The main app's service, when this process is an app bundle on a system
/// that has `SMAppService`.
fn main_app() -> Option<objc2::rc::Retained<SMAppService>> {
    let bundled = std::env::current_exe().is_ok_and(|exe| is_bundle_executable(&exe));
    if !bundled || AnyClass::get(c"SMAppService").is_none() {
        return None;
    }
    // SAFETY: the class exists (checked above); the call has no arguments.
    Some(unsafe { SMAppService::mainAppService() })
}

pub(crate) fn state() -> LoginItem {
    // SAFETY: a status query on a valid service.
    main_app().map_or(LoginItem::Unavailable, |service| {
        from_status(unsafe { service.status() })
    })
}

/// Registers or unregisters the app as a login item, then reports the new
/// state. Asking for what is already true changes nothing.
pub(crate) fn set(enabled: bool) -> Result<LoginItem, Error> {
    let service = main_app().ok_or(Error::Unsupported {
        what: "launch at login outside an app bundle",
    })?;
    // SAFETY: status, register and unregister on a valid main-app service.
    unsafe {
        let status = service.status();
        let result = match (enabled, status) {
            (true, SMAppServiceStatus::Enabled) => Ok(()),
            (true, _) => service.registerAndReturnError(),
            (false, SMAppServiceStatus::NotRegistered | SMAppServiceStatus::NotFound) => Ok(()),
            (false, _) => service.unregisterAndReturnError(),
        };
        result.map_err(|error| Error::System {
            what: if enabled {
                "registering the login item"
            } else {
                "removing the login item"
            },
            status: i64::try_from(error.code()).unwrap_or(i64::MIN),
        })?;
        Ok(from_status(service.status()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only `X.app/Contents/MacOS/X` counts as an app bundle's executable.
    #[test]
    fn only_an_app_bundle_executable_can_be_a_login_item() {
        assert!(is_bundle_executable(Path::new(
            "/Applications/Isthmus.app/Contents/MacOS/Isthmus"
        )));
        assert!(!is_bundle_executable(Path::new("/usr/local/bin/isthmus")));
        assert!(!is_bundle_executable(Path::new(
            "/tmp/x.app/Contents/Resources/x"
        )));
    }

    /// `SMAppService`'s statuses map onto what the UI shows; "not found"
    /// (no bundle to register) is unavailable.
    #[test]
    fn service_statuses_map_to_login_item_states() {
        assert_eq!(
            from_status(SMAppServiceStatus::NotRegistered),
            LoginItem::Disabled
        );
        assert_eq!(from_status(SMAppServiceStatus::Enabled), LoginItem::Enabled);
        assert_eq!(
            from_status(SMAppServiceStatus::RequiresApproval),
            LoginItem::RequiresApproval
        );
        assert_eq!(
            from_status(SMAppServiceStatus::NotFound),
            LoginItem::Unavailable
        );
    }
}
