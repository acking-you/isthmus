//! What the UI shows about Isthmus right now. Every type here crosses the
//! FFI as camelCase JSON; enums tolerate values a newer library sends.

use serde::{Deserialize, Serialize};

use crate::context::FrontApp;
use crate::device::{Device, Layout};
use crate::hidmap::HidProfile;

/// The state of everything, for the overview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// The library's version.
    pub version: String,
    /// The settings' master switch.
    pub enabled: bool,
    /// Isthmus is applying the settings now: enabled, this process holds the
    /// instance lock, and the platform is supported.
    pub running: bool,
    /// Another Isthmus process (another build, or the CLI) holds the lock,
    /// so this one changes nothing.
    pub another_instance: bool,
    pub permissions: Permissions,
    /// The keyboard event tap: Windows shortcuts, and with them the HID
    /// mapping that makes Ctrl act as Command.
    pub keyboard_tap: TapState,
    /// The mouse event tap: wheel, side buttons, Ctrl+wheel.
    pub mouse_tap: TapState,
    /// Some app holds Secure Event Input (a password field, a terminal's
    /// secure keyboard entry): shortcut translation pauses until it ends.
    pub secure_input: bool,
    pub hid: HidStatus,
    /// The app in front, and how it is treated.
    pub front_app: Option<FrontApp>,
    /// The mapping Windows-style keyboards have now.
    pub hid_profile: HidProfile,
    pub issues: Vec<Issue>,
}

/// macOS privacy permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Permissions {
    /// Accessibility (macOS 27: "Device Control and Data Access"), which an
    /// event tap that changes events needs.
    pub accessibility: PermissionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionState {
    Granted,
    Denied,
    /// Not asked yet, or macOS will not say.
    #[serde(other)]
    Unknown,
}

/// One event tap's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TapState {
    /// Installed and enabled.
    Running,
    /// Not wanted: Isthmus or this part of it is switched off.
    Off,
    /// Wanted, but macOS refused it: Accessibility is missing.
    NeedsPermission,
    /// Wanted and permitted, but it failed or keeps getting disabled.
    #[serde(other)]
    Failed,
}

/// The HID event-system side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HidStatus {
    pub backend: HidBackend,
    /// Keyboard services carrying Isthmus's mapping now.
    pub keyboards_mapped: u32,
    /// Pointer services with Isthmus's speed now.
    pub mice_tuned: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HidBackend {
    /// Every property readable and hot-plug notifications working.
    Full,
    /// The public client only: key mappings work, pointer speed and
    /// hot-plug do not.
    Limited,
    #[serde(other)]
    Unavailable,
}

/// Something the user should know about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Issue {
    /// Karabiner-Elements has seized keyboards: Isthmus's key mapping does
    /// not reach them.
    KarabinerActive,
    /// System Settings › Keyboard › Modifier Keys changes this keyboard too;
    /// Isthmus composes with it, but two tools moving the same keys confuse.
    SystemModifierKeys { device: String },
    /// Another app writes pointer properties and may undo Isthmus's speed.
    ForeignPointerTool { name: String },
    /// Accessibility reads as granted but the event tap still fails; macOS
    /// applies a new grant to a running app only after a relaunch.
    RelaunchNeeded,
    /// Some changes from a previous run could not be put back yet.
    RestorePending { count: u32 },
    /// `settings.json` could not be read at start, so Isthmus runs with the
    /// defaults, switched off, rather than guess. The file is kept as it
    /// was; the next saved change sets it aside as `settings.unreadable.json`
    /// and clears this.
    SettingsUnreadable,
    /// A kind this build does not know.
    #[serde(other)]
    Unknown,
}

/// A device for the keyboard and mouse pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceEntry {
    #[serde(flatten)]
    pub device: Device,
    /// Connected now; a remembered device with settings of its own is listed
    /// while disconnected too.
    pub connected: bool,
    /// The layout in effect: the user's choice, else the detected one.
    pub layout: Layout,
    /// Isthmus's mapping or pointer speed is on its services now.
    pub applied: bool,
}

/// What a permission request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionKind {
    Accessibility,
}

/// System Settings panes the UI links to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingsPane {
    Accessibility,
    LoginItems,
    Keyboard,
    Mouse,
    Trackpad,
}

/// Launch at login, as `SMAppService.mainApp` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginItem {
    Enabled,
    Disabled,
    /// The user turned it off in System Settings › Login Items.
    RequiresApproval,
    /// Not available: not running from an app bundle, or not macOS 13+.
    #[serde(other)]
    Unavailable,
}

/// A change the UI should re-read. A hint, not state: the receiver asks for
/// the new value through the normal calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Change {
    Status,
    Devices,
    Settings,
    FrontApp,
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The app matches an unreadable settings file by this exact kind, with
    /// no fields.
    #[test]
    fn an_unreadable_settings_file_crosses_the_ffi_as_its_own_kind() {
        let json = serde_json::to_value(Issue::SettingsUnreadable).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "settingsUnreadable" }));
        let back: Issue = serde_json::from_value(json).unwrap();
        assert_eq!(back, Issue::SettingsUnreadable);
    }
}
