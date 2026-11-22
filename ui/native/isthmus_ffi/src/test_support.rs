//! What this crate's tests share: a fake service, hosts built around it, and
//! reading an export's answer. The real service touches the keyboard and
//! mouse of the machine running the tests, so no test ever starts it.

use std::ffi::{CStr, c_char};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use isthmus_awake::{AwakeStatus, Error as AwakeError};
use isthmus_input::context::FrontApp;
use isthmus_input::hidmap::HidProfile;
use isthmus_input::status::{
    DeviceEntry, HidBackend, HidStatus, LoginItem, PermissionKind, PermissionState, Permissions,
    SettingsPane, TapState,
};
use isthmus_input::{Error as ServiceError, Preset, PresetTarget, Settings, Status};
use isthmus_link::status::Me;
use isthmus_link::store::PeerOs;
use isthmus_link::{DeviceId, Error as LinkError, LinkStatus};
use parking_lot::Mutex;

use crate::error::CtlError;
use crate::host::{Awake, Host, Link, Service};

/// A service that keeps its settings in memory and records what it was
/// asked, with switches for the failures a test needs.
#[derive(Default)]
pub(crate) struct FakeService {
    /// The next start fails as an unreadable settings file would.
    pub fail_start: AtomicBool,
    /// `status` panics.
    pub panic_on_status: AtomicBool,
    /// `set_login_item` fails as outside an app bundle.
    pub fail_login_item: AtomicBool,
    pub settings: Mutex<Settings>,
    pub permissions: Mutex<Vec<PermissionKind>>,
    pub panes: Mutex<Vec<SettingsPane>>,
    pub login_item: Mutex<Option<LoginItem>>,
    pub shutdowns: AtomicUsize,
    /// Runs inside `shutdown`, for a test that holds a shutdown open.
    pub on_shutdown: Mutex<Option<Box<dyn Fn() + Send>>>,
}

impl Service for FakeService {
    fn status(&self) -> Status {
        assert!(
            !self.panic_on_status.load(Ordering::SeqCst),
            "the fake was told to panic"
        );
        Status {
            version: "fake".into(),
            enabled: self.settings.lock().enabled,
            running: false,
            another_instance: false,
            permissions: Permissions {
                accessibility: PermissionState::Unknown,
            },
            keyboard_tap: TapState::Off,
            mouse_tap: TapState::Off,
            secure_input: false,
            hid: HidStatus {
                backend: HidBackend::Unavailable,
                keyboards_mapped: 0,
                mice_tuned: 0,
            },
            front_app: None,
            hid_profile: HidProfile::Positions,
            issues: Vec::new(),
        }
    }

    fn settings(&self) -> Settings {
        self.settings.lock().clone()
    }

    fn update_settings(&self, settings: Settings) -> Result<Settings, ServiceError> {
        let saved = settings.normalized();
        *self.settings.lock() = saved.clone();
        Ok(saved)
    }

    fn apply_preset(&self, preset: Preset, target: PresetTarget) -> Result<Settings, ServiceError> {
        let mut settings = self.settings.lock();
        settings.apply_preset(preset, target);
        Ok(settings.clone())
    }

    fn devices(&self) -> Vec<DeviceEntry> {
        Vec::new()
    }

    fn running_apps(&self) -> Vec<FrontApp> {
        vec![FrontApp {
            bundle_id: "com.apple.finder".into(),
            name: "Finder".into(),
            pid: 1,
            profile: Default::default(),
        }]
    }

    fn request_permission(&self, kind: PermissionKind) -> Status {
        self.permissions.lock().push(kind);
        self.status()
    }

    fn open_settings_pane(&self, pane: SettingsPane) -> Result<(), ServiceError> {
        self.panes.lock().push(pane);
        Ok(())
    }

    fn restore(&self) -> Result<Settings, ServiceError> {
        let mut settings = self.settings.lock();
        settings.enabled = false;
        Ok(settings.clone())
    }

    fn begin_layout_probe(&self, _device_id: &str) -> Result<(), ServiceError> {
        Ok(())
    }

    fn end_layout_probe(&self) {}

    fn login_item(&self) -> LoginItem {
        self.login_item.lock().unwrap_or(LoginItem::Disabled)
    }

    fn set_login_item(&self, enabled: bool) -> Result<LoginItem, ServiceError> {
        if self.fail_login_item.load(Ordering::SeqCst) {
            return Err(ServiceError::Unsupported {
                what: "launch at login",
            });
        }
        let state = if enabled {
            LoginItem::Enabled
        } else {
            LoginItem::Disabled
        };
        *self.login_item.lock() = Some(state);
        Ok(state)
    }

    fn shutdown(&self) {
        if let Some(hook) = &*self.on_shutdown.lock() {
            hook();
        }
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
    }
}

/// File sharing that records each call as a line of text.
#[derive(Default)]
pub(crate) struct FakeLink {
    pub calls: Mutex<Vec<String>>,
    pub shutdowns: AtomicUsize,
}

impl FakeLink {
    fn record(&self, call: String) {
        self.calls.lock().push(call);
    }
}

impl Link for FakeLink {
    fn status(&self) -> LinkStatus {
        LinkStatus {
            me: Me {
                id: DeviceId::try_from("fedcba9876543210".to_owned()).unwrap(),
                name: "Fake Mac".into(),
                os: PeerOs::Macos,
                port: 47821,
                paths: Vec::new(),
            },
            receive_dir: "/fake/Downloads".into(),
            peers: Vec::new(),
            pairing: None,
            transfers: Vec::new(),
        }
    }

    fn pair(&self, peer: &DeviceId) -> Result<(), LinkError> {
        self.record(format!("pair {peer}"));
        Ok(())
    }

    fn enter_code(&self, code: &str) -> Result<(), LinkError> {
        self.record(format!("code {code}"));
        Ok(())
    }

    fn cancel_pairing(&self) {
        self.record("cancel pairing".into());
    }

    fn unpair(&self, peer: &DeviceId) -> Result<(), LinkError> {
        self.record(format!("unpair {peer}"));
        Ok(())
    }

    fn send(&self, peer: &DeviceId, paths: Vec<PathBuf>) -> Result<String, LinkError> {
        let paths: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
        self.record(format!("send {peer} {}", paths.join("|")));
        Ok("fake-transfer".into())
    }

    fn cancel_transfer(&self, id: &str) -> Result<(), LinkError> {
        self.record(format!("cancel {id}"));
        Ok(())
    }

    fn clear_transfers(&self) {
        self.record("clear".into());
    }

    fn rename(&self, name: &str) -> Result<(), LinkError> {
        self.record(format!("rename {name}"));
        Ok(())
    }

    fn reveal(&self, id: Option<&str>) -> Result<(), LinkError> {
        self.record(format!("reveal {}", id.unwrap_or("-")));
        Ok(())
    }

    fn shutdown(&self) {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
    }
}

/// Keep-awake that only remembers the choice, and how often it was let go.
#[derive(Default)]
pub(crate) struct FakeAwake {
    pub enabled: AtomicBool,
    pub shutdowns: AtomicUsize,
}

impl Awake for FakeAwake {
    fn status(&self) -> AwakeStatus {
        let enabled = self.enabled.load(Ordering::SeqCst);
        AwakeStatus {
            enabled,
            active: enabled,
            supported: true,
            error: None,
        }
    }

    fn set(&self, enabled: bool) -> Result<AwakeStatus, AwakeError> {
        self.enabled.store(enabled, Ordering::SeqCst);
        Ok(self.status())
    }

    fn shutdown(&self) {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
    }
}

/// A host whose starter hands out `fake`, and the number of starts it made.
/// Leaked, as the process's own host lives for the whole process.
pub(crate) fn host_with(fake: Arc<FakeService>) -> (&'static Host, Arc<AtomicUsize>) {
    host_with_link(fake, Ok(Arc::new(FakeLink::default())))
}

/// [`host_with`], with file sharing that starts as `link` says.
pub(crate) fn host_with_link(
    fake: Arc<FakeService>,
    link: Result<Arc<FakeLink>, CtlError>,
) -> (&'static Host, Arc<AtomicUsize>) {
    host_with_parts(fake, link, Arc::new(FakeAwake::default()))
}

/// [`host_with_link`], with `awake` for keeping the computer awake.
pub(crate) fn host_with_parts(
    fake: Arc<FakeService>,
    link: Result<Arc<FakeLink>, CtlError>,
    awake: Arc<FakeAwake>,
) -> (&'static Host, Arc<AtomicUsize>) {
    let starts = Arc::new(AtomicUsize::new(0));
    let counted = starts.clone();
    let link_starter = Box::new(move || {
        link.clone().map(|link| {
            let link: Arc<dyn Link> = link;
            link
        })
    });
    let host = Host::new(
        Box::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            if fake.fail_start.load(Ordering::SeqCst) {
                return Err(ServiceError::ReadSettings {
                    path: PathBuf::from("/fake/settings.json"),
                    source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
                });
            }
            let service: Arc<dyn Service> = fake.clone();
            Ok(service)
        }),
        link_starter,
        Box::new(move || {
            let awake: Arc<dyn Awake> = awake.clone();
            Ok(awake)
        }),
    );
    (Box::leak(Box::new(host)), starts)
}

/// A host for a test that never reaches the service.
pub(crate) fn fake_host() -> &'static Host {
    host_with(Arc::new(FakeService::default())).0
}

/// An export's answer as JSON, freed as Dart frees it.
pub(crate) fn take_reply(text: *mut c_char) -> serde_json::Value {
    assert!(!text.is_null(), "an answer must serialize");
    // SAFETY: a non-null answer is a NUL-terminated string this library
    // allocated, read here before it is freed.
    let value = serde_json::from_slice(unsafe { CStr::from_ptr(text) }.to_bytes())
        .expect("answers are JSON");
    // SAFETY: it came from this library and is freed once.
    unsafe { crate::response::isthmus_free_string(text) };
    value
}
