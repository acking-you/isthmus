//! macOS: the only place Isthmus names an operating-system API.
//!
//! Two halves meet here. The **tap** half (`tap`, `synth`, `hotkeys`,
//! `actions`, `inspect`) owns the event-tap thread and everything the
//! callback posts. The **system** half (`hid`, `devices`, `workspace`,
//! `permissions`, `login`, `secure`) owns the HID thread, the AppKit
//! observers and the journal, and drives the tap through [`TapShared`].
//! `crate::service` glues them together.

pub(crate) mod actions;
pub(crate) mod cf;
pub(crate) mod devices;
pub(crate) mod display;
pub(crate) mod hid;
pub(crate) mod hotkeys;
pub(crate) mod input_sources;
pub(crate) mod inspect;
pub(crate) mod login;
pub(crate) mod permissions;
pub(crate) mod secure;
pub(crate) mod synth;
pub(crate) mod tap;
pub(crate) mod workspace;

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use crate::context::AppProfile;
use crate::engine::SystemHotkeys;
use crate::hidmap::HidProfile;
use crate::settings::{MouseProfile, ShortcutGroups};
use crate::status::TapState;

/// The marker in `kCGEventSourceUserData` (field 42) on every event Isthmus
/// posts, so its own tap passes them through: "ISTHMUS" plus a version byte.
pub(crate) const ISTHMUS_TAG: i64 = 0x4953_5448_4D55_5301;

/// Everything the tap callback reads, replaced whole when anything changes.
/// The callback takes the read lock only to clone the `Arc`.
#[derive(Debug, Clone)]
pub(crate) struct TapSnapshot {
    /// Translate keys: Isthmus is on, the keyboard section is on, and at
    /// least one keyboard is in Windows style.
    pub keyboard: bool,
    /// Change wheel and side-button events: Isthmus and the mouse section
    /// are on.
    pub mouse: bool,
    pub app: AppProfile,
    pub hid_profile: HidProfile,
    pub groups: ShortcutGroups,
    pub hotkeys: SystemHotkeys,
    /// A lone Shift tap selects the previous input source: the setting is
    /// on and this user has an input method.
    pub shift_switches_input: bool,
    /// Holding the middle button and moving the mouse scrolls.
    pub drag_scroll: bool,
    /// The profile of a wheel event whose sender is not in `mouse_by_sender`.
    pub mouse_default: MouseProfile,
    /// Profiles by sending service registry id (CGEvent field 87).
    pub mouse_by_sender: HashMap<u64, MouseProfile>,
}

impl Default for TapSnapshot {
    /// Off: everything passes.
    fn default() -> Self {
        TapSnapshot {
            keyboard: false,
            mouse: false,
            app: AppProfile::default(),
            hid_profile: HidProfile::Positions,
            groups: ShortcutGroups::all(false),
            hotkeys: SystemHotkeys::default(),
            shift_switches_input: false,
            drag_scroll: false,
            mouse_default: MouseProfile::MAC,
            mouse_by_sender: HashMap::new(),
        }
    }
}

/// What the system half and the tap thread share.
pub(crate) struct TapShared {
    pub snapshot: RwLock<Arc<TapSnapshot>>,
    /// Called (from the tap thread) when either tap's state changes, so the
    /// service can recompute the HID profile and tell the UI. Must not block.
    pub on_state: Box<dyn Fn(TapStates) + Send + Sync>,
}

impl TapShared {
    pub fn snapshot(&self) -> Arc<TapSnapshot> {
        self.snapshot.read().clone()
    }

    pub fn publish(&self, snapshot: TapSnapshot) {
        *self.snapshot.write() = Arc::new(snapshot);
    }
}

/// The two taps' states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TapStates {
    pub keyboard: TapState,
    pub mouse: TapState,
}
