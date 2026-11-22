//! The keyboard-and-mouse service: settings in, system changes out.
//!
//! One per process, and one process per Mac: the instance lock decides
//! which. The service owns the settings file and the journal, runs the
//! event-tap thread, the HID thread (devices, mappings, pointer speed,
//! hot-plug, wake) and the frontmost-app observer, and puts every change
//! back on [`InputService::shutdown`]. Every method is safe to call from
//! any thread; none blocks on the event tap.
//!
//! **Decisions are plain functions.** Reading and writing the settings
//! file, deciding what a journal left at start means, assembling the
//! status, listing devices and building the tap's snapshot take values and
//! return values, so they are tested on any host. The rest is a thin shell
//! that moves those values between threads.
//!
//! **Threads never wait on each other under the service's lock.** The tap
//! and HID threads never take it: they post tap states and HID summaries
//! to a coordinator thread (`isthmus-utility`), which also polls
//! Accessibility (2 s), Secure Event Input (1 s) and the system hotkeys
//! (60 s). The main thread's observers take the lock only to swap values.
//! A synchronous restore waits on the HID thread with the lock released.
//!
//! **A service that does not own the machine changes nothing.** Without the
//! instance lock (another build, or the CLI, is running) the service starts
//! no thread, reports `another_instance`, and refuses every change.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use tracing::{info, warn};

use crate::context::{self, FrontApp};
use crate::device::{self, Confidence, Device, DeviceId, DeviceKind, HidService, Layout};
use crate::engine::SystemHotkeys;
use crate::error::Error;
use crate::hidmap::HidProfile;
use crate::journal::Journal;
use crate::settings::{KeyboardStyle, LayoutChoice, Preset, PresetTarget, Settings};
use crate::status::{
    Change, DeviceEntry, HidBackend, HidStatus, Issue, LoginItem, PermissionKind, PermissionState,
    Permissions, SettingsPane, Status, TapState,
};

/// Receives change hints, on an arbitrary thread. It must not block.
pub type ChangeSink = Arc<dyn Fn(Change) + Send + Sync>;

/// How to start the service.
pub struct ServiceOptions {
    /// Where `settings.json`, `journal.json` and `instance.lock` live:
    /// normally [`isthmus_core::paths::data_dir`].
    pub data_dir: PathBuf,
    pub on_change: Option<ChangeSink>,
}

/// The running service. Cheap to clone; every clone is the same service.
#[derive(Clone)]
pub struct InputService {
    inner: Arc<Shared>,
}

/// Whether this build can apply anything at all.
const SUPPORTED: bool = cfg!(target_os = "macos");

/// The longest a layout probe leaves a keyboard unmapped, should the app
/// never end it (closed mid-question, say).
pub const LAYOUT_PROBE_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);

/// `settings` with keyboard `id` in Mac style, which gets no mapping: what
/// the HID side applies while that keyboard is probed.
#[cfg_attr(
    all(not(target_os = "macos"), not(test)),
    expect(dead_code, reason = "no mapping to lift off macOS")
)]
fn with_keyboard_unmapped(mut settings: Settings, id: &str) -> Settings {
    match settings.keyboard.devices.iter_mut().find(|d| d.id == id) {
        Some(device) => device.style = KeyboardStyle::Mac,
        None => settings
            .keyboard
            .devices
            .push(crate::settings::KeyboardDeviceSettings {
                id: id.to_owned(),
                style: KeyboardStyle::Mac,
                ..Default::default()
            }),
    }
    settings
}

/// What the HID thread last made true, for the status and the device
/// lists.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HidSummary {
    /// Which client the HID thread has, if any.
    pub backend: HidBackend,
    /// Keyboard and pointer services, as read.
    pub services: Vec<HidService>,
    /// The services grouped into devices.
    pub devices: Vec<Device>,
    /// Registry ids of the services carrying Isthmus's mapping or speed.
    pub applied: BTreeSet<u64>,
    /// Keyboard services carrying a Windows mapping.
    pub keyboards_mapped: u32,
    /// Pointer services carrying a Windows speed.
    pub mice_tuned: u32,
    /// Karabiner, System Settings modifier keys.
    pub issues: Vec<Issue>,
    /// Journalled changes that could not be put back yet.
    pub restore_pending: u32,
}

impl Default for HidSummary {
    /// Nothing known yet.
    fn default() -> Self {
        HidSummary {
            backend: HidBackend::Unavailable,
            services: Vec::new(),
            devices: Vec::new(),
            applied: BTreeSet::new(),
            keyboards_mapped: 0,
            mice_tuned: 0,
            issues: Vec::new(),
            restore_pending: 0,
        }
    }
}

/// Everything the status and the tap snapshot are made from.
struct State {
    settings: Settings,
    /// The settings file could not be read at start, so the defaults are in
    /// effect: the status says so until a save works. The file is left as
    /// it was until the user saves, and then set aside rather than
    /// overwritten.
    settings_unreadable: bool,
    /// The app in front, classified under the user's rules.
    front: Option<FrontApp>,
    hid: HidSummary,
    /// The mapping Windows-style keyboards should have now.
    hid_profile: HidProfile,
    keyboard_tap: TapState,
    mouse_tap: TapState,
    /// Accessibility, as last polled.
    permission: PermissionState,
    secure_input: bool,
    /// A grant that macOS will apply only after a relaunch.
    relaunch_needed: bool,
    /// This user's session is the one on the console.
    #[cfg_attr(
        all(not(target_os = "macos"), not(test)),
        expect(dead_code, reason = "only macOS follows session switches")
    )]
    session_active: bool,
    /// The user's system hotkeys, for the tap.
    #[cfg_attr(
        not(target_os = "macos"),
        expect(dead_code, reason = "only the macOS tap posts hotkeys")
    )]
    hotkeys: SystemHotkeys,
    /// The user has an input method, so a Shift tap has somewhere to go.
    #[cfg_attr(
        not(target_os = "macos"),
        expect(dead_code, reason = "only the macOS tap switches input sources")
    )]
    input_method: bool,
    /// [`InputService::shutdown`] ran.
    stopped: bool,
    /// A keyboard whose mapping is lifted while the app asks the user to
    /// press the key beside the space bar, and when that began.
    layout_probe: Option<(String, std::time::Instant)>,
}

impl State {
    /// Before anything is known: no app, no devices, taps off.
    fn new(settings: Settings) -> State {
        State {
            settings,
            settings_unreadable: false,
            front: None,
            hid: HidSummary::default(),
            hid_profile: HidProfile::Positions,
            keyboard_tap: TapState::Off,
            mouse_tap: TapState::Off,
            permission: PermissionState::Unknown,
            secure_input: false,
            relaunch_needed: false,
            session_active: true,
            hotkeys: SystemHotkeys::default(),
            input_method: false,
            stopped: false,
            layout_probe: None,
        }
    }

    /// Isthmus should be changing the system now.
    #[cfg_attr(
        all(not(target_os = "macos"), not(test)),
        expect(dead_code, reason = "nothing is applied off macOS")
    )]
    fn active(&self, owner: bool) -> bool {
        owner && self.settings.enabled && self.session_active && !self.stopped
    }
}

/// Reads the settings file: `None` when there is none yet. Anything
/// unreadable is an error the caller reports; it never silently becomes
/// the defaults here.
pub(crate) fn load_settings(path: &Path) -> Result<Option<Settings>, Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::ReadSettings {
                path: path.to_owned(),
                source,
            });
        }
    };
    Settings::from_json(&text)
        .map(Some)
        .map_err(|source| Error::ParseSettings {
            path: path.to_owned(),
            source,
        })
}

/// The settings file's bytes.
pub(crate) fn encode_settings(settings: &Settings) -> Result<Vec<u8>, Error> {
    let mut bytes = settings
        .to_json_pretty()
        .map_err(|source| Error::Encode {
            what: "the settings",
            source,
        })?
        .into_bytes();
    bytes.push(b'\n');
    Ok(bytes)
}

/// Writes the settings file atomically: never half written.
fn save_settings(path: &Path, settings: &Settings) -> Result<(), Error> {
    let bytes = encode_settings(settings)?;
    isthmus_core::durable_file::replace_contents(path, &bytes).map_err(|source| Error::WriteFile {
        path: path.to_owned(),
        source,
    })
}

/// Moves an unreadable state file out of the way rather than overwriting
/// it, so whatever it held can still be recovered by hand.
pub(crate) fn set_aside(path: &Path) -> Option<PathBuf> {
    let aside = path.with_extension("unreadable.json");
    match std::fs::rename(path, &aside) {
        Ok(()) => Some(aside),
        // Nothing there: an earlier save already set it aside, then failed.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            warn!(?error, path = %path.display(), "could not set an unreadable file aside");
            None
        }
    }
}

/// What a journal found at start means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    all(not(target_os = "macos"), not(test)),
    expect(dead_code, reason = "only macOS writes a journal")
)]
pub(crate) enum JournalStart {
    /// Nothing journalled.
    Fresh,
    /// A previous run of this boot left changes: put them back.
    Recover,
    /// Written in another boot (or the boot is unknown): the restart
    /// already undid every change, and a registry id from then may name a
    /// different service now.
    Discard,
}

/// What to do with a journal found at start, given this boot's session.
#[cfg_attr(
    all(not(target_os = "macos"), not(test)),
    expect(dead_code, reason = "only macOS writes a journal")
)]
pub(crate) fn journal_start(journal: &Journal, boot_session: Option<&str>) -> JournalStart {
    if journal.entries.is_empty() {
        return JournalStart::Fresh;
    }
    match boot_session {
        Some(boot) if !boot.is_empty() && boot == journal.boot_session => JournalStart::Recover,
        Some(_) | None => JournalStart::Discard,
    }
}

/// Reads the journal; a missing one is empty, an unreadable one is set
/// aside and read as empty (nothing it says can be trusted).
#[cfg_attr(
    all(not(target_os = "macos"), not(test)),
    expect(dead_code, reason = "only macOS writes a journal")
)]
pub(crate) fn load_journal(path: &Path) -> Journal {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|error| {
            warn!(?error, "the journal is unreadable; setting it aside");
            set_aside(path);
            Journal::default()
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Journal::default(),
        Err(error) => {
            warn!(?error, "could not read the journal");
            Journal::default()
        }
    }
}

/// Deletes a journal whose changes are already gone.
#[cfg(target_os = "macos")]
pub(crate) fn discard_journal(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => info!("discarded a journal from another boot"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => warn!(?error, "could not delete a stale journal"),
    }
}

/// The status as the UI shows it.
fn assemble_status(state: &State, owner: bool, supported: bool) -> Status {
    let running = state.settings.enabled && owner && supported && !state.stopped;
    let tap = |s: TapState| if running { s } else { TapState::Off };
    let mut issues = Vec::new();
    if state.settings_unreadable {
        // First: it explains why everything else is off.
        issues.push(Issue::SettingsUnreadable);
    }
    issues.extend(state.hid.issues.iter().cloned());
    if state.relaunch_needed {
        issues.push(Issue::RelaunchNeeded);
    }
    if state.hid.restore_pending > 0 {
        issues.push(Issue::RestorePending {
            count: state.hid.restore_pending,
        });
    }
    Status {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        enabled: state.settings.enabled,
        running,
        another_instance: !owner,
        permissions: Permissions {
            // A tap macOS let Isthmus install is the grant, whatever tccd's
            // answers lag behind at.
            accessibility: if state.keyboard_tap == TapState::Running
                || state.mouse_tap == TapState::Running
            {
                PermissionState::Granted
            } else {
                state.permission
            },
        },
        keyboard_tap: tap(state.keyboard_tap),
        mouse_tap: tap(state.mouse_tap),
        secure_input: state.secure_input,
        hid: HidStatus {
            backend: state.hid.backend,
            keyboards_mapped: state.hid.keyboards_mapped,
            mice_tuned: state.hid.mice_tuned,
        },
        front_app: state.front.clone(),
        hid_profile: state.hid_profile,
        issues,
    }
}

/// A device the settings remember that is not connected now.
fn remembered(id: &str, name: &str, kind: DeviceKind, layout: Layout) -> Device {
    Device {
        id: DeviceId(id.to_owned()),
        kind,
        name: name.to_owned(),
        manufacturer: String::new(),
        vendor_id: 0,
        product_id: 0,
        transport: String::new(),
        built_in: false,
        services: Vec::new(),
        detected_layout: layout,
        layout_confidence: Confidence::Low,
        via_kvm: false,
        has_mode_switch: false,
    }
}

/// The layout a keyboard is treated as: the user's choice, else detection.
pub(crate) fn effective_layout(choice: LayoutChoice, detected: Layout) -> Layout {
    match choice {
        LayoutChoice::Mac => Layout::Mac,
        LayoutChoice::Pc => Layout::Pc,
        LayoutChoice::Auto => detected,
    }
}

/// The device lists: connected devices first, as the HID thread grouped
/// them, then the remembered ones that have settings of their own.
fn device_entries(hid: &HidSummary, settings: &Settings) -> Vec<DeviceEntry> {
    let layout_of = |device: &Device| match device.kind {
        DeviceKind::Keyboard => {
            let choice = settings
                .keyboard
                .device(device.id.as_str())
                .map_or(LayoutChoice::Auto, |d| d.layout);
            effective_layout(choice, device.detected_layout)
        }
        DeviceKind::Mouse | DeviceKind::Trackpad => device.detected_layout,
    };
    let connected: BTreeSet<&str> = hid.devices.iter().map(|d| d.id.as_str()).collect();
    let mut entries: Vec<DeviceEntry> = hid
        .devices
        .iter()
        .map(|device| DeviceEntry {
            device: device.clone(),
            connected: true,
            layout: layout_of(device),
            applied: device.services.iter().any(|s| hid.applied.contains(s)),
        })
        .collect();
    let mut away: Vec<Device> = settings
        .keyboard
        .devices
        .iter()
        .filter(|d| !connected.contains(d.id.as_str()))
        .map(|d| {
            let layout = if d.layout == LayoutChoice::Mac {
                Layout::Mac
            } else {
                Layout::Pc
            };
            remembered(&d.id, &d.name, DeviceKind::Keyboard, layout)
        })
        .chain(
            settings
                .mouse
                .devices
                .iter()
                .filter(|d| !connected.contains(d.id.as_str()))
                .map(|d| remembered(&d.id, &d.name, DeviceKind::Mouse, Layout::Pc)),
        )
        .collect();
    away.sort_by_key(|d| d.name.to_lowercase());
    entries.extend(away.into_iter().map(|device| DeviceEntry {
        layout: layout_of(&device),
        device,
        connected: false,
        applied: false,
    }));
    entries
}

/// The frontmost app with its class under the user's rules.
fn classified(bundle_id: String, name: String, pid: i32, settings: &Settings) -> FrontApp {
    let profile = context::classify(&bundle_id, &settings.keyboard.app_rules);
    FrontApp {
        bundle_id,
        name,
        pid,
        profile,
    }
}

/// The System Settings deep link of a pane.
pub(crate) const fn pane_url(pane: SettingsPane) -> &'static str {
    match pane {
        SettingsPane::Accessibility => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        SettingsPane::LoginItems => {
            "x-apple.systempreferences:com.apple.LoginItems-Settings.extension"
        }
        SettingsPane::Keyboard => "x-apple.systempreferences:com.apple.preference.keyboard",
        SettingsPane::Mouse => "x-apple.systempreferences:com.apple.preference.mouse",
        SettingsPane::Trackpad => "x-apple.systempreferences:com.apple.preference.trackpad",
    }
}

/// The service's shared half: what every clone of [`InputService`] and
/// every thread of the service refer to.
struct Shared {
    /// `settings.json` in the data directory.
    settings_path: PathBuf,
    /// This process holds the instance lock.
    owner: bool,
    /// Held for the service's life; the kernel releases it if the process
    /// dies, so a crash never leaves a stale lock.
    _lock: Option<isthmus_core::instance::InstanceLock>,
    sink: Option<ChangeSink>,
    state: Mutex<State>,
    /// Serializes changing the settings (read, modify, save, apply), so the
    /// file and the state never disagree about which of two concurrent
    /// changes won, and neither is lost.
    save_lock: Mutex<()>,
    /// [`InputService::shutdown`] has begun; it runs once.
    shut_down: AtomicBool,
    /// The threads (macOS), or nothing.
    #[cfg_attr(
        not(target_os = "macos"),
        expect(dead_code, reason = "no threads off macOS")
    )]
    runtime: runtime::Runtime,
}

impl Shared {
    /// Sends change hints to the UI's sink, if there is one. Called with no
    /// lock held, since the sink runs on this thread.
    fn emit(&self, changes: &[Change]) {
        if let Some(sink) = &self.sink {
            for change in changes {
                sink(*change);
            }
        }
    }

    /// Changes are refused without the lock, and after shutdown.
    fn check_writable(&self) -> Result<(), Error> {
        if !self.owner {
            return Err(Error::AnotherInstance);
        }
        if self.shut_down.load(Ordering::Acquire) {
            return Err(Error::NotRunning);
        }
        Ok(())
    }

    /// Saves `settings` (normalized) and makes them the service's, without
    /// applying them. An unreadable file from start is set aside first, and
    /// reported until a save works. Callers hold `save_lock`.
    fn store(&self, settings: Settings) -> Result<Settings, Error> {
        let settings = settings.normalized();
        let unreadable = self.state.lock().settings_unreadable;
        if unreadable && let Some(aside) = set_aside(&self.settings_path) {
            info!(path = %aside.display(), "set the unreadable settings file aside");
        }
        save_settings(&self.settings_path, &settings)?;
        let mut state = self.state.lock();
        state.settings_unreadable = false;
        state.front = state
            .front
            .take()
            .map(|f| classified(f.bundle_id, f.name, f.pid, &settings));
        state.settings = settings.clone();
        Ok(settings)
    }

    /// Changes the settings: `change` gets the current ones and returns the
    /// new ones, which are saved and then applied. Reading and saving
    /// happen under one lock, so concurrent changes never undo each other.
    /// New settings that switch Isthmus off switch it off even when they
    /// cannot be saved (see [`Shared::store_or_switch_off`]).
    fn update(&self, change: impl FnOnce(Settings) -> Settings) -> Result<Settings, Error> {
        self.check_writable()?;
        let _serial = self.save_lock.lock();
        let current = self.state.lock().settings.clone();
        let wanted = change(current);
        let saved = if wanted.enabled {
            self.store(wanted)?
        } else {
            match self.store_or_switch_off(wanted) {
                Ok(settings) => settings,
                Err(error) => {
                    self.apply();
                    self.emit(&[Change::Settings, Change::Status]);
                    return Err(error);
                }
            }
        };
        self.apply();
        self.emit(&[Change::Settings, Change::Status, Change::FrontApp]);
        Ok(saved)
    }

    /// Saves `settings`, which switch Isthmus off. When the file cannot be
    /// written, Isthmus is switched off in memory anyway and the write error
    /// returned: the control that puts the system back must never fail
    /// closed for a reason that has nothing to do with the system. The file
    /// keeps what it said, which the next start reads again.
    fn store_or_switch_off(&self, settings: Settings) -> Result<Settings, Error> {
        let saved = self.store(settings);
        if saved.is_err() {
            self.state.lock().settings.enabled = false;
        }
        saved
    }

    /// Stops applying and puts everything back, once.
    fn shutdown(&self) {
        if self.shut_down.swap(true, Ordering::AcqRel) {
            return;
        }
        self.state.lock().stopped = true;
        self.stop_runtime();
        self.emit(&[Change::Status]);
    }
}

impl Drop for Shared {
    /// A service dropped without [`InputService::shutdown`] still puts the
    /// system back: HID changes outlive the process otherwise.
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl InputService {
    /// Loads the settings, takes the instance lock (without it the service
    /// runs but changes nothing), recovers the journal of a previous run
    /// that did not finish, and applies the settings. Must be called on a
    /// thread other than the one running the AppKit main loop only if that
    /// loop is running (the frontmost-app observer is registered on it
    /// asynchronously).
    ///
    /// A settings file that cannot be read does not fail the start: the
    /// service runs with the defaults (switched off), logs
    /// [`Error::ParseSettings`] or [`Error::ReadSettings`], reports
    /// [`Issue::SettingsUnreadable`] in the status so the user learns why,
    /// and leaves the file untouched until the user saves.
    pub fn start(options: ServiceOptions) -> Result<InputService, Error> {
        let dir = options.data_dir;
        std::fs::create_dir_all(&dir).map_err(|source| Error::DataDir {
            source: isthmus_core::Error::CreateDir {
                path: dir.clone(),
                source,
            },
        })?;
        let settings_path = isthmus_core::paths::settings_file(&dir);
        let lock = isthmus_core::instance::InstanceLock::try_acquire(
            &isthmus_core::paths::lock_file(&dir),
        )
        .map_err(|source| Error::DataDir { source })?;
        let owner = lock.is_some();
        if !owner {
            warn!("another Isthmus owns the keyboard and mouse; this one changes nothing");
        }
        let (settings, unreadable) = match load_settings(&settings_path) {
            Ok(Some(settings)) => (settings, false),
            Ok(None) => {
                let settings = Settings::default();
                if owner && let Err(error) = save_settings(&settings_path, &settings) {
                    warn!(?error, "could not write the default settings");
                }
                (settings, false)
            }
            Err(error) => {
                warn!(
                    ?error,
                    "the settings file is unreadable; running with the defaults, switched off"
                );
                (Settings::default(), true)
            }
        };
        let mut state = State::new(settings);
        state.settings_unreadable = unreadable;
        let shared = Arc::new(Shared {
            settings_path,
            owner,
            _lock: lock,
            sink: options.on_change,
            state: Mutex::new(state),
            save_lock: Mutex::new(()),
            shut_down: AtomicBool::new(false),
            runtime: runtime::Runtime::new(),
        });
        if owner {
            runtime::launch(&shared, &dir);
        }
        Ok(InputService { inner: shared })
    }

    /// The state of everything, for the overview.
    pub fn status(&self) -> Status {
        // The UI asks while its window is up, which is when a grant is
        // likely: check at once rather than at the next poll.
        self.inner.check_permission_soon();
        assemble_status(&self.inner.state.lock(), self.inner.owner, SUPPORTED)
    }

    /// The settings in effect (as saved, normalized).
    pub fn settings(&self) -> Settings {
        self.inner.state.lock().settings.clone()
    }

    /// Normalizes, saves and applies `settings`; returns what was saved.
    pub fn update_settings(&self, settings: Settings) -> Result<Settings, Error> {
        self.inner.update(|_| settings)
    }

    /// Applies a one-click preset, then saves and applies.
    pub fn apply_preset(&self, preset: Preset, target: PresetTarget) -> Result<Settings, Error> {
        self.inner.update(|mut settings| {
            settings.apply_preset(preset, target);
            settings
        })
    }

    /// Connected keyboards, mice and trackpads, plus remembered ones that
    /// have settings of their own.
    pub fn devices(&self) -> Vec<DeviceEntry> {
        let (hid, settings) = {
            let state = self.inner.state.lock();
            (state.hid.clone(), state.settings.clone())
        };
        let hid = if self.inner.owner {
            hid
        } else {
            // No HID thread runs without the lock; a one-off read still
            // shows what is connected.
            let services = crate::platform::list_services().unwrap_or_default();
            HidSummary {
                devices: device::group_services(&services),
                services,
                ..HidSummary::default()
            }
        };
        device_entries(&hid, &settings)
    }

    /// Running apps with a bundle id and a window, for the app-rule picker.
    pub fn running_apps(&self) -> Vec<FrontApp> {
        let settings = self.settings();
        let mut apps: Vec<FrontApp> = runtime::running_apps()
            .into_iter()
            .map(|(bundle_id, name, pid)| classified(bundle_id, name, pid, &settings))
            .collect();
        apps.sort_by_key(|app| app.name.to_lowercase());
        let mut seen = BTreeSet::new();
        apps.retain(|app| seen.insert(app.bundle_id.clone()));
        apps
    }

    /// Opens the permission's pane in System Settings, where the app's
    /// drag-to-authorize card adds Isthmus to the list. The system's own
    /// prompt is not raised: it would stack a second dialog on the same
    /// pane, and it appears only once per app anyway.
    pub fn request_permission(&self, kind: PermissionKind) -> Status {
        match kind {
            PermissionKind::Accessibility => {
                if let Err(error) = self.open_settings_pane(SettingsPane::Accessibility) {
                    warn!(?error, "could not open the Accessibility pane");
                }
                self.inner.check_permission_soon();
            }
        }
        self.status()
    }

    /// Lifts `device_id`'s key mapping for at most [`LAYOUT_PROBE_LIMIT`],
    /// so the key beside the space bar arrives as the keyboard sends it:
    /// Command in Mac mode, Option (Alt) in Windows mode. The app asks the
    /// user to press it and saves the layout that follows; firmware mode
    /// switches tell macOS nothing, so this one key press is how a
    /// keyboard's mode is found. Starting a probe ends any other.
    pub fn begin_layout_probe(&self, device_id: &str) -> Result<(), Error> {
        self.inner.check_writable()?;
        self.inner.state.lock().layout_probe =
            Some((device_id.to_owned(), std::time::Instant::now()));
        self.inner.apply();
        Ok(())
    }

    /// Puts the probed keyboard's mapping back. Harmless with no probe.
    pub fn end_layout_probe(&self) {
        let ended = self.inner.state.lock().layout_probe.take().is_some();
        if ended {
            self.inner.apply();
        }
    }

    /// Opens a System Settings pane the UI links to.
    pub fn open_settings_pane(&self, pane: SettingsPane) -> Result<(), Error> {
        runtime::open_url(pane_url(pane))
    }

    /// Switches Isthmus off and puts everything back now; the settings keep
    /// `enabled = false`. When they cannot be saved, everything is put back
    /// all the same and the write error returned.
    pub fn restore(&self) -> Result<Settings, Error> {
        let shared = &self.inner;
        shared.check_writable()?;
        let _serial = shared.save_lock.lock();
        let mut settings = shared.state.lock().settings.clone();
        settings.enabled = false;
        let saved = shared.store_or_switch_off(settings);
        shared.restore_system();
        shared.emit(&[Change::Settings, Change::Status]);
        saved
    }

    /// Whether Isthmus launches at login.
    pub fn login_item(&self) -> LoginItem {
        runtime::login_item()
    }

    /// Registering a login item is a change like any other: refused without
    /// the instance lock, so a second build never points it at itself.
    pub fn set_login_item(&self, enabled: bool) -> Result<LoginItem, Error> {
        self.inner.check_writable()?;
        runtime::set_login_item(enabled)
    }

    /// Puts every system change back and stops the threads, without
    /// touching the settings. Idempotent; bounded (a few property writes).
    /// The app calls it from `applicationWillTerminate`.
    pub fn shutdown(&self) {
        self.inner.shutdown();
    }
}

/// The macOS shell: the threads and what moves between them.
#[cfg(target_os = "macos")]
mod runtime {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
    use std::sync::{Arc, OnceLock, Weak};
    use std::time::{Duration, Instant};

    use parking_lot::{Mutex, RwLock};
    use tracing::{info, warn};

    use super::{HidSummary, Shared, State, classified};
    use crate::context::{AppClass, AppProfile};
    use crate::device::{Device, DeviceKind};
    use crate::engine::SystemHotkeys;
    use crate::error::Error;
    use crate::hidmap::{self, HidProfile};
    use crate::platform::macos::devices::{Desired, JournalStore};
    use crate::platform::macos::hid::{AFTER_WAKE, HidStart, HidThread};
    use crate::platform::macos::permissions::{self, RawAccess, RetryStage, TapAction};
    use crate::platform::macos::tap::TapThread;
    use crate::platform::macos::workspace::{self, Observers, WorkspaceEvent};
    use crate::platform::macos::{
        TapShared, TapSnapshot, TapStates, hotkeys, input_sources, login, secure,
    };
    use crate::settings::{KeyboardStyle, MouseProfile, Settings};
    use crate::status::{Change, LoginItem, PermissionState, TapState};

    /// How long start waits for the HID thread's first reconcile.
    const HID_START: Duration = Duration::from_secs(3);
    /// How long a restore waits for the HID thread.
    const RESTORE: Duration = Duration::from_millis(1200);
    /// How long shutdown waits for a thread to finish.
    const JOIN: Duration = Duration::from_millis(300);
    /// Secure Event Input has no notification; once a second is prompt
    /// enough for a badge and to reset the tap's held keys.
    const SECURE_POLL: Duration = Duration::from_secs(1);
    /// Accessibility is read from tccd (blocking IPC) this often.
    const PERMISSION_POLL: Duration = Duration::from_secs(2);
    /// How often Accessibility is read while the keyboard tap waits for it.
    const GRANT_POLL: Duration = Duration::from_millis(500);
    /// How often the utility loop checks tccd's change notification while
    /// the keyboard tap waits for Accessibility.
    const GRANT_WATCH: Duration = Duration::from_millis(250);
    /// The least time between two retries of a refused tap.
    const RETRY_SPACING: Duration = Duration::from_secs(1);
    /// The user rebinds system hotkeys rarely; a minute is soon enough.
    const HOTKEY_RELOAD: Duration = Duration::from_secs(60);
    /// The coordinator thread's name, also how shutdown knows it runs there.
    const COORDINATOR: &str = "isthmus-utility";

    /// What reaches the coordinator from other threads.
    pub(super) enum Coord {
        /// The HID thread's new summary.
        Hid(Box<HidSummary>),
        /// The taps' new states, from the tap thread.
        Taps(TapStates),
        /// Read Accessibility now rather than at the next poll.
        CheckPermission,
        /// Shutdown.
        Stop,
    }

    /// The threads of a running service.
    pub(super) struct Runtime {
        /// What the tap reads; shared with the tap thread.
        tap_shared: Arc<TapShared>,
        /// The tap thread, while a tap is wanted.
        tap: Mutex<Option<TapThread>>,
        /// The HID thread, once started.
        hid: OnceLock<HidThread>,
        /// Taken at shutdown to tell the coordinator to stop.
        coordinator: Mutex<Option<Sender<Coord>>>,
        /// The coordinator's end, until it is spawned.
        inbox: Mutex<Option<Receiver<Coord>>>,
        /// Disconnects when the coordinator has finished.
        coordinator_exited: Mutex<Option<Receiver<()>>>,
        /// The workspace observers, once installed.
        observers: OnceLock<Observers>,
        /// Accessibility went away under a running tap, which was stopped;
        /// it comes back only once access is granted again.
        tap_held: AtomicBool,
        /// A grant has been followed by a tap retry.
        tap_retried: AtomicBool,
    }

    impl Runtime {
        /// The channels and the tap's shared state; no thread yet. The
        /// tap's state callback only queues a message: it runs on the tap
        /// thread, which must never wait.
        pub(super) fn new() -> Runtime {
            let (sender, inbox) = mpsc::channel();
            let to_coordinator = sender.clone();
            let tap_shared = Arc::new(TapShared {
                snapshot: RwLock::new(Arc::new(TapSnapshot::default())),
                on_state: Box::new(move |states| {
                    let _ = to_coordinator.send(Coord::Taps(states));
                }),
            });
            Runtime {
                tap_shared,
                tap: Mutex::new(None),
                hid: OnceLock::new(),
                coordinator: Mutex::new(Some(sender)),
                inbox: Mutex::new(Some(inbox)),
                coordinator_exited: Mutex::new(None),
                observers: OnceLock::new(),
                tap_held: AtomicBool::new(false),
                tap_retried: AtomicBool::new(false),
            }
        }
    }

    /// The tap's view of everything: what to translate, for which app, and
    /// each mouse's profile by the service that sends its events.
    pub(crate) fn build_snapshot(
        active: bool,
        settings: &Settings,
        app: AppProfile,
        hid_profile: HidProfile,
        hotkeys: SystemHotkeys,
        input_method: bool,
        devices: &[Device],
    ) -> TapSnapshot {
        let any_windows_keyboard = devices
            .iter()
            .filter(|d| d.kind == DeviceKind::Keyboard)
            .any(|d| settings.keyboard.style_of(d.id.as_str()) == KeyboardStyle::Windows);
        let mut mouse_by_sender = HashMap::new();
        for device in devices {
            let profile = match device.kind {
                DeviceKind::Mouse => settings.mouse.profile_of(device.id.as_str()),
                // Never changed: an explicit Mac profile, not the defaults.
                DeviceKind::Trackpad => MouseProfile::MAC,
                DeviceKind::Keyboard => continue,
            };
            for service in &device.services {
                mouse_by_sender.insert(*service, profile);
            }
        }
        TapSnapshot {
            keyboard: active && settings.keyboard.enabled && any_windows_keyboard,
            mouse: active && settings.mouse.enabled,
            app,
            hid_profile,
            groups: settings.keyboard.groups,
            hotkeys,
            shift_switches_input: settings.keyboard.shift_switches_input && input_method,
            drag_scroll: settings.mouse.drag_scroll(),
            mouse_default: settings.mouse.defaults,
            mouse_by_sender,
        }
    }

    /// Starts the threads of a service that owns the machine.
    pub(super) fn launch(shared: &Arc<Shared>, dir: &Path) {
        let runtime = &shared.runtime;
        let initial_access = permissions::probe();
        {
            let mut state = shared.state.lock();
            state.permission = permissions::assess(None, initial_access);
            state.secure_input = secure::enabled();
            state.hotkeys = hotkeys::load();
            state.input_method = input_sources::has_input_method();
        }
        spawn_coordinator(shared, initial_access);
        shared.ensure_tap();
        shared.refresh();

        let journal_path = isthmus_core::paths::journal_file(dir);
        let journal = super::load_journal(&journal_path);
        let boot = isthmus_core::boot::boot_session_id();
        let recover = match super::journal_start(&journal, boot.as_deref()) {
            super::JournalStart::Fresh => false,
            super::JournalStart::Recover => true,
            super::JournalStart::Discard => {
                super::discard_journal(&journal_path);
                false
            }
        };
        let journal = if recover {
            journal
        } else {
            crate::journal::Journal::default()
        };
        let to_coordinator = runtime.coordinator.lock().clone();
        let start = HidStart {
            journal: JournalStore::new(journal_path, journal, boot.unwrap_or_default()),
            recover,
            desired: shared.desired(&shared.state.lock()),
            on_summary: Box::new(move |summary| {
                if let Some(sender) = &to_coordinator {
                    let _ = sender.send(Coord::Hid(Box::new(summary)));
                }
            }),
        };
        if let Some((hid, first)) = HidThread::start(start, HID_START) {
            // Every summary also reaches the coordinator, in order; fill in
            // the first only if none has arrived, never over a newer one.
            let mut state = shared.state.lock();
            if state.hid == HidSummary::default() {
                state.hid = first;
            }
            drop(state);
            let _ = runtime.hid.set(hid);
        }
        shared.refresh();

        let weak = Arc::downgrade(shared);
        let observers = Observers::install(Arc::new(move |event| {
            if let Some(shared) = weak.upgrade() {
                shared.on_workspace(event);
            }
        }));
        let _ = runtime.observers.set(observers);
    }

    /// Starts the coordinator, which holds only a weak reference: the
    /// service ends when its last handle is dropped, not when this thread
    /// does.
    fn spawn_coordinator(shared: &Arc<Shared>, initial: RawAccess) {
        let runtime = &shared.runtime;
        let Some(inbox) = runtime.inbox.lock().take() else {
            return;
        };
        let (exited_tx, exited_rx) = mpsc::channel::<()>();
        *runtime.coordinator_exited.lock() = Some(exited_rx);
        let weak = Arc::downgrade(shared);
        let spawned = std::thread::Builder::new()
            .name(COORDINATOR.into())
            .spawn(move || {
                coordinate(&weak, &inbox, initial);
                drop(exited_tx);
            });
        if let Err(error) = spawned {
            warn!(
                ?error,
                "could not start the utility thread; permissions and Secure Input are not watched"
            );
        }
    }

    /// The coordinator's loop: messages as they come, polls when due.
    fn coordinate(shared: &Weak<Shared>, inbox: &Receiver<Coord>, initial: RawAccess) {
        let mut access = initial;
        // tccd posts after every permission write: a grant is seen within a
        // turn of this loop instead of at the next poll.
        let tcc = permissions::TccWatch::new();
        if tcc.is_none() {
            warn!("could not watch tccd's changes; a grant is seen at the next poll");
        }
        let mut last_retry: Option<Instant> = None;
        let now = Instant::now();
        let (mut next_secure, mut next_permission, mut next_hotkeys) = (
            now + SECURE_POLL,
            now + PERMISSION_POLL,
            now + HOTKEY_RELOAD,
        );
        loop {
            // While the keyboard tap waits for Accessibility, turn often so
            // the grant takes effect the moment it is given.
            let awaiting = shared
                .upgrade()
                .is_some_and(|shared| shared.awaiting_grant());
            let mut wake = next_secure.min(next_permission).min(next_hotkeys);
            if awaiting {
                wake = wake.min(Instant::now() + GRANT_WATCH);
            }
            let message = inbox.recv_timeout(wake.saturating_duration_since(Instant::now()));
            let Some(shared) = shared.upgrade() else {
                return;
            };
            match message {
                Ok(Coord::Hid(summary)) => shared.on_hid_summary(*summary),
                // A tap thread that was stopped may still report its last
                // attempt; with no thread running, the state is already
                // decided (Off, or held after a revocation) and must stay.
                Ok(Coord::Taps(states)) => {
                    let running = shared.runtime.tap.lock().is_some();
                    if running {
                        shared.set_taps(states);
                    }
                }
                Ok(Coord::CheckPermission) => next_permission = Instant::now(),
                Ok(Coord::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if tcc.as_ref().is_some_and(permissions::TccWatch::changed) {
                next_permission = Instant::now();
            }
            if shared.shut_down.load(Ordering::Acquire) {
                return;
            }
            let now = Instant::now();
            if now >= next_secure {
                shared.poll_secure_input();
                shared.expire_layout_probe();
                next_secure = now + SECURE_POLL;
            }
            if now >= next_permission {
                access = shared.poll_permission(access, &mut last_retry);
                next_permission = now
                    + if shared.awaiting_grant() {
                        GRANT_POLL
                    } else {
                        PERMISSION_POLL
                    };
            }
            if now >= next_hotkeys {
                shared.reload_hotkeys();
                next_hotkeys = now + HOTKEY_RELOAD;
            }
        }
    }

    impl Shared {
        /// What the HID side should make true now.
        fn desired(&self, state: &State) -> Desired {
            let settings = match &state.layout_probe {
                Some((id, _)) => super::with_keyboard_unmapped(state.settings.clone(), id),
                None => state.settings.clone(),
            };
            Desired {
                active: state.active(self.owner),
                settings,
                hid_profile: state.hid_profile,
                display_scale: crate::platform::macos::display::main_display_scale(),
            }
        }

        /// Recomputes the HID profile, sends the HID thread what it should
        /// make true and the tap its snapshot. Everything here is a value
        /// swap or a queued message, so it runs under the lock: two
        /// refreshes can never publish out of order.
        pub(super) fn refresh(&self) {
            let mut state = self.state.lock();
            let class = state
                .front
                .as_ref()
                .map_or(AppClass::Standard, |f| f.profile.class);
            state.hid_profile = hidmap::profile_for(class, state.keyboard_tap == TapState::Running);
            let active = state.active(self.owner);
            let app = state
                .front
                .as_ref()
                .map_or_else(AppProfile::default, |f| f.profile);
            let snapshot = build_snapshot(
                active,
                &state.settings,
                app,
                state.hid_profile,
                state.hotkeys,
                state.input_method,
                &state.hid.devices,
            );
            if let Some(hid) = self.runtime.hid.get() {
                hid.set_desired(self.desired(&state));
            }
            self.runtime.tap_shared.publish(snapshot);
        }

        /// Starts the tap thread when it is wanted and stops it when not:
        /// "off" removes the event tap rather than leaving it to pass
        /// everything through.
        fn ensure_tap(&self) {
            let wanted = {
                let state = self.state.lock();
                state.active(self.owner)
                    && (state.settings.keyboard.enabled || state.settings.mouse.enabled)
                    && !self.runtime.tap_held.load(Ordering::Acquire)
            };
            let mut slot = self.runtime.tap.lock();
            match (wanted, slot.is_some()) {
                (true, false) => {
                    let thread = TapThread::start(Arc::clone(&self.runtime.tap_shared));
                    let states = thread.states();
                    *slot = Some(thread);
                    drop(slot);
                    self.set_taps(states);
                }
                (false, true) => {
                    if let Some(thread) = slot.take() {
                        drop(slot);
                        thread.stop();
                    }
                    self.set_taps(TapStates {
                        keyboard: TapState::Off,
                        mouse: TapState::Off,
                    });
                }
                (true, true) | (false, false) => {}
            }
        }

        /// Records the taps' states; a keyboard tap starting or stopping
        /// changes which mapping the keyboards may have.
        fn set_taps(&self, states: TapStates) {
            let profile_matters = {
                let mut state = self.state.lock();
                let was = state.keyboard_tap == TapState::Running;
                let mut changed =
                    state.keyboard_tap != states.keyboard || state.mouse_tap != states.mouse;
                state.keyboard_tap = states.keyboard;
                state.mouse_tap = states.mouse;
                // A running keyboard tap is the grant reaching this process:
                // no relaunch is needed, and the next permission poll that
                // would say so is up to two seconds away.
                if states.keyboard == TapState::Running
                    && std::mem::take(&mut state.relaunch_needed)
                {
                    changed = true;
                }
                if !changed {
                    return;
                }
                was != (states.keyboard == TapState::Running)
            };
            if profile_matters {
                self.refresh();
            }
            self.emit(&[Change::Status]);
        }

        /// After a settings change: tap on or off, then everything else.
        pub(super) fn apply(&self) {
            self.ensure_tap();
            self.refresh();
        }

        /// Switched off: the tap goes, and the HID side puts everything
        /// back before this returns (or the wait runs out).
        pub(super) fn restore_system(&self) {
            self.ensure_tap();
            self.refresh();
            if let Some(hid) = self.runtime.hid.get()
                && hid.restore_all(RESTORE).is_none()
            {
                warn!("the HID thread did not finish restoring in time");
            }
        }

        /// A new HID summary: new devices change the tap's per-mouse
        /// profiles.
        fn on_hid_summary(&self, summary: HidSummary) {
            let devices_changed = {
                let mut state = self.state.lock();
                let changed =
                    state.hid.devices != summary.devices || state.hid.applied != summary.applied;
                state.hid = summary;
                changed
            };
            if devices_changed {
                self.refresh();
                self.emit(&[Change::Devices, Change::Status]);
            } else {
                self.emit(&[Change::Status]);
            }
        }

        /// An event from the main thread's observers. Quick: it only swaps
        /// values and queues messages.
        fn on_workspace(&self, event: WorkspaceEvent) {
            if self.shut_down.load(Ordering::Acquire) {
                return;
            }
            match event {
                WorkspaceEvent::Activated(app) => {
                    {
                        let mut state = self.state.lock();
                        let same = state
                            .front
                            .as_ref()
                            .is_some_and(|f| f.pid == app.pid && f.bundle_id == app.bundle_id);
                        if same {
                            return;
                        }
                        state.front = Some(classified(
                            app.bundle_id,
                            app.name,
                            app.pid,
                            &state.settings,
                        ));
                    }
                    self.refresh();
                    self.emit(&[Change::FrontApp, Change::Status]);
                }
                WorkspaceEvent::DidWake => {
                    if let Some(hid) = self.runtime.hid.get() {
                        hid.reconcile_after(AFTER_WAKE);
                    }
                }
                WorkspaceEvent::SessionResigned | WorkspaceEvent::SessionActivated => {
                    let active = event == WorkspaceEvent::SessionActivated;
                    self.state.lock().session_active = active;
                    self.refresh();
                    self.emit(&[Change::Status]);
                }
            }
        }

        /// Follows Secure Event Input: the tap releases its held keys when
        /// it starts or ends, since key events stop arriving meanwhile.
        fn poll_secure_input(&self) {
            let on = secure::enabled();
            let changed = {
                let mut state = self.state.lock();
                std::mem::replace(&mut state.secure_input, on) != on
            };
            if changed {
                if let Some(tap) = self.runtime.tap.lock().as_ref() {
                    tap.secure_input_changed(on);
                }
                self.emit(&[Change::Status]);
            }
        }

        /// Asks the tap thread to install whatever is missing, or starts one
        /// when there is none. Returns whether the retry was only queued (its
        /// outcome reaches the state later, as a `Coord::Taps`).
        fn retry_tap(&self) -> bool {
            self.runtime.tap_held.store(false, Ordering::Release);
            let existing = self.runtime.tap.lock().as_ref().map(|tap| {
                tap.retry();
                tap.states()
            });
            let queued = match existing {
                Some(states) => {
                    self.set_taps(states);
                    true
                }
                None => {
                    self.ensure_tap();
                    false
                }
            };
            self.runtime.tap_retried.store(true, Ordering::Release);
            queued
        }

        /// Whether the keyboard tap is wanted and still waits for
        /// Accessibility, which is when permissions are watched closely.
        pub(super) fn awaiting_grant(&self) -> bool {
            let state = self.state.lock();
            state.active(self.owner)
                && state.settings.keyboard.enabled
                && matches!(
                    state.keyboard_tap,
                    TapState::NeedsPermission | TapState::Failed
                )
        }

        /// Ends a layout probe the app left running past its limit.
        fn expire_layout_probe(&self) {
            let expired = {
                let mut state = self.state.lock();
                let late = state
                    .layout_probe
                    .as_ref()
                    .is_some_and(|(_, since)| since.elapsed() >= super::LAYOUT_PROBE_LIMIT);
                if late {
                    state.layout_probe = None;
                }
                late
            };
            if expired {
                warn!("a layout probe ran past its limit; mapping the keyboard again");
                self.apply();
            }
        }

        /// Has the utility thread read Accessibility now: the UI is up, or
        /// the user just asked for the grant.
        pub(super) fn check_permission_soon(&self) {
            if let Some(sender) = self.runtime.coordinator.lock().as_ref() {
                let _ = sender.send(Coord::CheckPermission);
            }
        }

        /// Reads Accessibility; acts on a change (retry the tap, or stop it
        /// at once); flags a needed relaunch. Returns what the next poll is
        /// judged against: the reading, except that a revocation holds while
        /// tccd says "unknown" ([`permissions::carried`]).
        fn poll_permission(
            &self,
            previous: RawAccess,
            last_retry: &mut Option<Instant>,
        ) -> RawAccess {
            let reading = permissions::probe();
            let now = permissions::assess(Some(previous), reading);
            if reading != previous {
                info!(
                    trusted = reading.trusted,
                    post = ?reading.post,
                    assessed = ?now,
                    "Accessibility reading changed"
                );
            }
            let (before, tap_running) = {
                let state = self.state.lock();
                (
                    state.permission,
                    state.keyboard_tap == TapState::Running || state.mouse_tap == TapState::Running,
                )
            };
            let mut changed = before != now;
            // A retry of an existing tap only queues work on the tap thread;
            // its outcome reaches the state later, as a `Coord::Taps`.
            let mut retry_queued = false;
            if changed {
                self.state.lock().permission = now;
                match permissions::on_change(before, now, tap_running) {
                    TapAction::Retry => {
                        retry_queued = self.retry_tap();
                        *last_retry = Some(Instant::now());
                    }
                    TapAction::Stop => {
                        warn!("Accessibility was revoked; stopping the event tap");
                        self.runtime.tap_held.store(true, Ordering::Release);
                        let stopped = self.runtime.tap.lock().take();
                        if let Some(tap) = stopped {
                            tap.stop();
                        }
                        self.set_taps(TapStates {
                            keyboard: TapState::NeedsPermission,
                            mouse: TapState::NeedsPermission,
                        });
                    }
                    TapAction::Nothing => {}
                }
            }
            // A grant does not always read as a change (tccd and the AX cache
            // lag, or disagree with each other): whenever the keyboard tap is
            // refused and a no-prompt check says it may now be allowed, try
            // it again, at most once a second. Its creation is the answer.
            if !retry_queued && last_retry.is_none_or(|at| at.elapsed() >= RETRY_SPACING) {
                let (wanted, keyboard) = {
                    let state = self.state.lock();
                    (
                        state.active(self.owner) && state.settings.keyboard.enabled,
                        state.keyboard_tap,
                    )
                };
                let may_be_allowed = now == PermissionState::Granted || permissions::preflight();
                if permissions::should_retry(wanted, keyboard, may_be_allowed) {
                    info!("Accessibility may be granted; trying the event tap again");
                    retry_queued = self.retry_tap();
                    *last_retry = Some(Instant::now());
                }
            }
            {
                let mut state = self.state.lock();
                let wanted = state.active(self.owner) && state.settings.keyboard.enabled;
                let retry = if retry_queued {
                    RetryStage::Queued
                } else if self.runtime.tap_retried.load(Ordering::Acquire) {
                    RetryStage::Done
                } else {
                    RetryStage::NotYet
                };
                if let Some(relaunch) =
                    permissions::relaunch_needed(now, wanted, retry, state.keyboard_tap)
                    && relaunch != state.relaunch_needed
                {
                    state.relaunch_needed = relaunch;
                    changed = true;
                }
            }
            if changed {
                self.emit(&[Change::Status]);
            }
            permissions::carried(previous, reading)
        }

        /// Re-reads the user's system hotkeys (the tap posts what they
        /// bind) and whether they have an input method (the Shift tap's
        /// condition).
        fn reload_hotkeys(&self) {
            let hotkeys = hotkeys::load();
            let input_method = input_sources::has_input_method();
            let changed = {
                let mut state = self.state.lock();
                let hotkeys_changed = std::mem::replace(&mut state.hotkeys, hotkeys) != hotkeys;
                let method_changed =
                    std::mem::replace(&mut state.input_method, input_method) != input_method;
                hotkeys_changed || method_changed
            };
            if changed {
                self.refresh();
            }
        }

        /// Stops the observers, the coordinator and the tap, puts back every
        /// HID change, and stops the HID thread. Bounded: each wait is short
        /// and nothing waits on the main thread (this may run on it).
        pub(super) fn stop_runtime(&self) {
            let runtime = &self.runtime;
            if let Some(observers) = runtime.observers.get() {
                observers.remove();
            }
            if let Some(sender) = runtime.coordinator.lock().take() {
                let _ = sender.send(Coord::Stop);
            }
            // The coordinator itself may be dropping the last handle; it
            // must not wait for its own exit.
            let on_coordinator = std::thread::current().name() == Some(COORDINATOR);
            let exited = runtime.coordinator_exited.lock().take();
            if let Some(exited) = exited
                && !on_coordinator
            {
                let _ = exited.recv_timeout(JOIN);
            }
            let tap = runtime.tap.lock().take();
            if let Some(tap) = tap {
                tap.stop();
            }
            runtime.tap_shared.publish(TapSnapshot::default());
            if let Some(hid) = runtime.hid.get() {
                hid.set_desired(Desired::inactive());
                if hid.restore_all(RESTORE).is_none() {
                    warn!(
                        "the HID thread did not finish restoring before shutdown; the journal keeps the rest"
                    );
                }
                hid.stop(JOIN);
            }
            let mut state = self.state.lock();
            state.keyboard_tap = TapState::Off;
            state.mouse_tap = TapState::Off;
        }
    }

    /// Running apps as (bundle id, name, pid).
    pub(super) fn running_apps() -> Vec<(String, String, i32)> {
        workspace::running_apps()
            .into_iter()
            .map(|app| (app.bundle_id, app.name, app.pid))
            .collect()
    }

    /// Opens a URL with `/usr/bin/open`, which hands System Settings deep
    /// links to the right pane without AppKit.
    pub(super) fn open_url(url: &str) -> Result<(), Error> {
        let status = std::process::Command::new("/usr/bin/open")
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match status {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(Error::System {
                what: "opening System Settings",
                status: i64::from(status.code().unwrap_or(-1)),
            }),
            Err(error) => {
                warn!(?error, "could not run /usr/bin/open");
                Err(Error::System {
                    what: "opening System Settings",
                    status: i64::from(error.raw_os_error().unwrap_or(-1)),
                })
            }
        }
    }

    /// The login item's state.
    pub(super) fn login_item() -> LoginItem {
        login::state()
    }

    /// Registers or removes the login item.
    pub(super) fn set_login_item(enabled: bool) -> Result<LoginItem, Error> {
        login::set(enabled)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::device::{Confidence, DeviceId, Layout};
        use crate::settings::{KeyboardDeviceSettings, MouseDeviceSettings};

        fn device(id: &str, kind: DeviceKind, services: Vec<u64>) -> Device {
            Device {
                id: DeviceId(id.into()),
                kind,
                name: id.into(),
                manufacturer: String::new(),
                vendor_id: 0,
                product_id: 0,
                transport: "USB".into(),
                built_in: false,
                services,
                detected_layout: Layout::Pc,
                layout_confidence: Confidence::Medium,
                via_kvm: false,
                has_mode_switch: false,
            }
        }

        fn snapshot(active: bool, settings: &Settings, devices: &[Device]) -> TapSnapshot {
            build_snapshot(
                active,
                settings,
                AppProfile::default(),
                HidProfile::Windows,
                SystemHotkeys::default(),
                true,
                devices,
            )
        }

        /// Every service of a wheel mouse maps to that mouse's own profile,
        /// a trackpad's services to the Mac profile, and keyboards to
        /// nothing; an unknown sender falls back to the defaults.
        #[test]
        fn the_snapshot_maps_each_pointer_service_to_its_devices_profile() {
            let mut settings = Settings {
                enabled: true,
                ..Settings::default()
            };
            let mut slow = MouseProfile::WINDOWS;
            slow.pointer_speed = 4;
            settings.mouse.devices.push(MouseDeviceSettings {
                id: "mouse-a".into(),
                name: "A".into(),
                profile: slow,
                ..MouseDeviceSettings::default()
            });
            let devices = [
                device("mouse-a", DeviceKind::Mouse, vec![1, 2]),
                device("mouse-b", DeviceKind::Mouse, vec![3]),
                device("pad", DeviceKind::Trackpad, vec![4]),
                device("kbd", DeviceKind::Keyboard, vec![5]),
            ];
            let snap = snapshot(true, &settings, &devices);
            assert_eq!(snap.mouse_by_sender.get(&1), Some(&slow));
            assert_eq!(snap.mouse_by_sender.get(&2), Some(&slow));
            assert_eq!(snap.mouse_by_sender.get(&3), Some(&MouseProfile::WINDOWS));
            assert_eq!(snap.mouse_by_sender.get(&4), Some(&MouseProfile::MAC));
            assert_eq!(snap.mouse_by_sender.get(&5), None);
            assert_eq!(snap.mouse_default, settings.mouse.defaults);
            assert!(snap.mouse);
        }

        /// A keyboard tap that comes up running clears a relaunch flagged
        /// earlier at once, rather than at the next permission poll: the
        /// grant has reached this process after all.
        #[test]
        fn a_running_keyboard_tap_clears_a_flagged_relaunch() {
            let dir = std::env::temp_dir();
            let service = crate::service::tests::detached_service(
                dir.join("isthmus-never-written").join("settings.json"),
                Settings {
                    enabled: true,
                    ..Settings::default()
                },
                false,
            );
            let shared = &service.inner;
            {
                let mut state = shared.state.lock();
                state.keyboard_tap = TapState::NeedsPermission;
                state.relaunch_needed = true;
            }
            shared.set_taps(TapStates {
                keyboard: TapState::NeedsPermission,
                mouse: TapState::Running,
            });
            assert!(shared.state.lock().relaunch_needed);
            shared.set_taps(TapStates {
                keyboard: TapState::Running,
                mouse: TapState::Running,
            });
            assert!(!shared.state.lock().relaunch_needed);
        }

        /// Keys are translated only while Isthmus is active, the keyboard
        /// section is on, and some connected keyboard is in Windows style.
        #[test]
        fn keys_are_translated_only_with_a_windows_style_keyboard_while_active() {
            let mut settings = Settings {
                enabled: true,
                ..Settings::default()
            };
            let keyboard = [device("kbd", DeviceKind::Keyboard, vec![5])];
            assert!(snapshot(true, &settings, &keyboard).keyboard);
            assert!(!snapshot(false, &settings, &keyboard).keyboard);
            assert!(!snapshot(true, &settings, &[]).keyboard);
            settings.keyboard.devices.push(KeyboardDeviceSettings {
                id: "kbd".into(),
                style: KeyboardStyle::Mac,
                ..KeyboardDeviceSettings::default()
            });
            assert!(!snapshot(true, &settings, &keyboard).keyboard);
            let off = snapshot(false, &settings, &keyboard);
            assert!(!off.mouse && !off.keyboard);
        }
    }
}

/// Every other OS: the service keeps settings but applies nothing.
#[cfg(not(target_os = "macos"))]
mod runtime {
    use std::path::Path;
    use std::sync::Arc;

    use super::Shared;
    use crate::error::Error;
    use crate::status::LoginItem;

    /// No threads.
    pub(super) struct Runtime;

    impl Runtime {
        pub(super) fn new() -> Runtime {
            Runtime
        }
    }

    /// Nothing to start.
    pub(super) fn launch(_shared: &Arc<Shared>, _dir: &Path) {}

    impl Shared {
        /// Settings are kept; nothing is applied.
        pub(super) fn apply(&self) {}
        /// Nothing was changed, so nothing goes back.
        pub(super) fn restore_system(&self) {}
        /// No threads to stop.
        pub(super) fn stop_runtime(&self) {}
        /// No permissions to watch.
        pub(super) fn check_permission_soon(&self) {}
    }

    /// No app list off macOS.
    pub(super) fn running_apps() -> Vec<(String, String, i32)> {
        Vec::new()
    }

    /// No System Settings.
    pub(super) fn open_url(_url: &str) -> Result<(), Error> {
        Err(Error::Unsupported {
            what: "System Settings",
        })
    }

    /// No login items.
    pub(super) fn login_item() -> LoginItem {
        LoginItem::Unavailable
    }

    /// No login items.
    pub(super) fn set_login_item(_enabled: bool) -> Result<LoginItem, Error> {
        Err(Error::Unsupported {
            what: "launch at login",
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU32;

    use super::*;
    use crate::journal::{JournalEntry, PropertyValue};
    use crate::settings::{KeyboardDeviceSettings, MouseDeviceSettings};

    /// A fresh directory under the system temp dir, unique per test.
    fn scratch(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "isthmus-service-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// No settings file yet is "nothing to load", not an error: start then
    /// writes the defaults.
    #[test]
    fn a_missing_settings_file_is_nothing_to_load() {
        let dir = scratch("missing");
        assert!(load_settings(&dir.join("settings.json")).unwrap().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A file that does not parse is an error naming the file, and reading
    /// it leaves its bytes exactly as they were.
    #[test]
    fn an_unreadable_settings_file_is_an_error_and_stays_untouched() {
        let dir = scratch("corrupt");
        let path = dir.join("settings.json");
        std::fs::write(&path, b"{ not json").unwrap();
        let error = load_settings(&path).unwrap_err();
        assert!(matches!(error, Error::ParseSettings { path: ref p, .. } if *p == path));
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
        let aside = set_aside(&path).unwrap();
        assert_eq!(std::fs::read(aside).unwrap(), b"{ not json");
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// What is saved reads back equal, keys a newer version wrote included.
    #[test]
    fn saved_settings_read_back_equal_with_unknown_keys_kept() {
        let dir = scratch("round-trip");
        let path = dir.join("settings.json");
        let mut settings =
            Settings::from_json(r#"{"enabled": true, "fromTheFuture": [1, 2]}"#).unwrap();
        settings.mouse.defaults.lines_per_notch = 5;
        std::fs::write(&path, encode_settings(&settings).unwrap()).unwrap();
        let back = load_settings(&path).unwrap().unwrap();
        assert_eq!(back, settings);
        assert!(back.unknown.contains_key("fromTheFuture"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn journal_with(boot: &str, entries: usize) -> Journal {
        let entry = JournalEntry {
            registry_id: 1,
            vendor_id: 0,
            product_id: 0,
            location_id: 0,
            key: "UserKeyMapping".into(),
            original: PropertyValue::Absent,
            applied: PropertyValue::KeyMapping(vec![(1, 2)]),
        };
        Journal {
            version: Journal::VERSION,
            boot_session: boot.into(),
            entries: vec![entry; entries],
        }
    }

    /// A journal of this boot is recovered; one of another boot, or of a
    /// boot that cannot be told, is discarded (the restart undid it and its
    /// registry ids may name other services now); an empty one is nothing.
    #[test]
    fn only_a_journal_of_this_boot_is_recovered() {
        assert_eq!(
            journal_start(&journal_with("A", 1), Some("A")),
            JournalStart::Recover
        );
        assert_eq!(
            journal_start(&journal_with("A", 1), Some("B")),
            JournalStart::Discard
        );
        assert_eq!(
            journal_start(&journal_with("", 1), None),
            JournalStart::Discard
        );
        assert_eq!(
            journal_start(&journal_with("", 1), Some("")),
            JournalStart::Discard
        );
        assert_eq!(
            journal_start(&journal_with("A", 0), Some("B")),
            JournalStart::Fresh
        );
    }

    /// A journal that does not parse is set aside and read as empty, so the
    /// next record never mixes with what it held.
    #[test]
    fn an_unreadable_journal_is_set_aside_and_read_as_empty() {
        let dir = scratch("journal");
        let path = dir.join("journal.json");
        assert_eq!(load_journal(&path), Journal::default());
        std::fs::write(&path, b"[").unwrap();
        assert_eq!(load_journal(&path), Journal::default());
        assert!(!path.exists());
        assert!(dir.join("journal.unreadable.json").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn running_state() -> State {
        let mut state = State::new(Settings {
            enabled: true,
            ..Settings::default()
        });
        state.keyboard_tap = TapState::Running;
        state.mouse_tap = TapState::Running;
        state
    }

    /// Running means enabled, owning the lock, on a supported platform and
    /// not shut down; while not running both taps read as off.
    #[test]
    fn the_status_runs_only_when_enabled_owned_supported_and_not_stopped() {
        let state = running_state();
        let status = assemble_status(&state, true, true);
        assert!(status.running && !status.another_instance);
        assert_eq!(status.keyboard_tap, TapState::Running);
        assert_eq!(status.version, env!("CARGO_PKG_VERSION"));

        let second = assemble_status(&state, false, true);
        assert!(!second.running && second.another_instance);
        assert_eq!(
            (second.keyboard_tap, second.mouse_tap),
            (TapState::Off, TapState::Off)
        );

        assert!(!assemble_status(&state, true, false).running);
        let mut stopped = running_state();
        stopped.stopped = true;
        assert!(!assemble_status(&stopped, true, true).running);
        let mut off = running_state();
        off.settings.enabled = false;
        assert!(!assemble_status(&off, true, true).running);
    }

    /// An unreadable settings file comes first, since it explains why
    /// Isthmus is off; then the HID side's issues, a needed relaunch and the
    /// count of changes still waiting to be put back.
    #[test]
    fn the_status_lists_unreadable_settings_hid_issues_a_needed_relaunch_and_pending_restores() {
        let mut state = running_state();
        state.settings_unreadable = true;
        state.hid.issues = vec![Issue::KarabinerActive];
        state.hid.restore_pending = 2;
        state.relaunch_needed = true;
        assert_eq!(
            assemble_status(&state, true, true).issues,
            vec![
                Issue::SettingsUnreadable,
                Issue::KarabinerActive,
                Issue::RelaunchNeeded,
                Issue::RestorePending { count: 2 }
            ]
        );
    }

    /// A service over `settings_path` that owns the machine but starts no
    /// thread, and whose console session is inactive, so it never installs
    /// a tap or touches HID even if a change wrongly left it enabled: only
    /// the settings file, the settings and the status move.
    pub(super) fn detached_service(
        settings_path: PathBuf,
        settings: Settings,
        settings_unreadable: bool,
    ) -> InputService {
        let mut state = State::new(settings);
        state.settings_unreadable = settings_unreadable;
        state.session_active = false;
        InputService {
            inner: Arc::new(Shared {
                settings_path,
                owner: true,
                _lock: None,
                sink: None,
                state: Mutex::new(state),
                save_lock: Mutex::new(()),
                shut_down: AtomicBool::new(false),
                runtime: runtime::Runtime::new(),
            }),
        }
    }

    fn enabled() -> Settings {
        Settings {
            enabled: true,
            ..Settings::default()
        }
    }

    /// An unreadable settings file is reported in the status, not silently
    /// replaced by the defaults; a save that fails keeps reporting it, and
    /// the first save that works sets the file aside and clears the issue.
    #[test]
    fn an_unreadable_settings_file_is_reported_until_a_save_works() {
        let dir = scratch("unreadable-issue");
        let unwritable = detached_service(
            dir.join("missing").join("settings.json"),
            Settings::default(),
            true,
        );
        assert!(unwritable.update_settings(Settings::default()).is_err());
        assert!(
            unwritable
                .status()
                .issues
                .contains(&Issue::SettingsUnreadable)
        );

        let path = dir.join("settings.json");
        std::fs::write(&path, b"{ not json").unwrap();
        let service = detached_service(path.clone(), Settings::default(), true);
        assert!(service.status().issues.contains(&Issue::SettingsUnreadable));
        let mut changed = Settings::default();
        changed.mouse.defaults.lines_per_notch = 5;
        service.update_settings(changed.clone()).unwrap();
        assert!(!service.status().issues.contains(&Issue::SettingsUnreadable));
        assert_eq!(
            std::fs::read(dir.join("settings.unreadable.json")).unwrap(),
            b"{ not json"
        );
        assert_eq!(load_settings(&path).unwrap(), Some(changed.normalized()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Restore switches Isthmus off and puts the system back even when the
    /// settings file cannot be written, and still returns the write error:
    /// the control that undoes everything never fails closed over the disk.
    #[test]
    fn restore_switches_off_even_when_the_settings_cannot_be_saved() {
        let dir = scratch("restore-unsaved");
        let service = detached_service(dir.join("missing").join("settings.json"), enabled(), false);
        let error = service.restore().unwrap_err();
        assert!(matches!(error, Error::WriteFile { .. }));
        assert!(!service.settings().enabled);
        assert!(!service.status().enabled);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Switching Isthmus off takes effect even when it cannot be saved;
    /// any other change that cannot be saved changes nothing.
    #[test]
    fn switching_off_works_without_the_disk_but_other_changes_need_it() {
        let dir = scratch("update-unsaved");
        let service = detached_service(dir.join("missing").join("settings.json"), enabled(), false);
        let mut faster = enabled();
        faster.mouse.defaults.lines_per_notch = 5;
        assert!(matches!(
            service.update_settings(faster),
            Err(Error::WriteFile { .. })
        ));
        assert_eq!(service.settings(), enabled().normalized());

        assert!(matches!(
            service.update_settings(Settings::default()),
            Err(Error::WriteFile { .. })
        ));
        assert!(!service.settings().enabled);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn device(id: &str, kind: DeviceKind, services: Vec<u64>, layout: Layout) -> Device {
        Device {
            id: DeviceId(id.into()),
            kind,
            name: id.into(),
            manufacturer: String::new(),
            vendor_id: 0,
            product_id: 0,
            transport: "USB".into(),
            built_in: false,
            services,
            detected_layout: layout,
            layout_confidence: Confidence::Medium,
            via_kvm: false,
            has_mode_switch: false,
        }
    }

    /// Connected devices come first with the layout in effect (the user's
    /// choice over detection) and whether Isthmus's values are on any of
    /// their services; then disconnected devices with settings of their
    /// own, by name.
    #[test]
    fn device_lists_show_connected_then_remembered_devices() {
        let hid = HidSummary {
            devices: vec![
                device("kbd", DeviceKind::Keyboard, vec![10, 11], Layout::Mac),
                device("mouse", DeviceKind::Mouse, vec![20], Layout::Pc),
            ],
            applied: BTreeSet::from([11]),
            ..HidSummary::default()
        };
        let mut settings = Settings::default();
        settings.keyboard.devices = vec![
            KeyboardDeviceSettings {
                id: "kbd".into(),
                layout: LayoutChoice::Pc,
                ..KeyboardDeviceSettings::default()
            },
            KeyboardDeviceSettings {
                id: "away-kbd".into(),
                name: "Zed".into(),
                layout: LayoutChoice::Mac,
                ..KeyboardDeviceSettings::default()
            },
        ];
        settings.mouse.devices = vec![MouseDeviceSettings {
            id: "away-mouse".into(),
            name: "Alpha".into(),
            ..MouseDeviceSettings::default()
        }];
        let entries = device_entries(&hid, &settings);
        let summary: Vec<(&str, bool, Layout, bool)> = entries
            .iter()
            .map(|e| (e.device.id.as_str(), e.connected, e.layout, e.applied))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("kbd", true, Layout::Pc, true),
                ("mouse", true, Layout::Pc, false),
                ("away-mouse", false, Layout::Pc, false),
                ("away-kbd", false, Layout::Mac, false),
            ]
        );
    }

    /// The user's layout choice wins over detection; Auto takes detection.
    #[test]
    fn a_chosen_layout_wins_over_the_detected_one() {
        assert_eq!(effective_layout(LayoutChoice::Pc, Layout::Mac), Layout::Pc);
        assert_eq!(
            effective_layout(LayoutChoice::Auto, Layout::Mac),
            Layout::Mac
        );
    }

    /// Without the instance lock, and after shutdown, active is false: a
    /// second Isthmus and a stopped one change nothing.
    #[test]
    fn only_an_owning_running_session_is_active() {
        let state = running_state();
        assert!(state.active(true));
        assert!(!state.active(false));
        let mut away = running_state();
        away.session_active = false;
        assert!(!away.active(true));
    }

    /// Probing a keyboard puts only that keyboard in Mac style (no
    /// mapping), whether or not it had an entry, and leaves the saved
    /// settings and every other keyboard as they were.
    #[test]
    fn a_probed_keyboard_alone_goes_unmapped() {
        let mut settings = Settings::default();
        settings
            .keyboard
            .devices
            .push(crate::settings::KeyboardDeviceSettings {
                id: "keyboard:usb:05ac:024f:loc=00112000".into(),
                style: KeyboardStyle::Windows,
                layout: LayoutChoice::Mac,
                ..Default::default()
            });
        let probed =
            super::with_keyboard_unmapped(settings.clone(), "keyboard:usb:05ac:024f:loc=00112000");
        assert_eq!(
            probed
                .keyboard
                .style_of("keyboard:usb:05ac:024f:loc=00112000"),
            KeyboardStyle::Mac
        );
        assert_eq!(
            probed
                .keyboard
                .device("keyboard:usb:05ac:024f:loc=00112000")
                .map(|d| d.layout),
            Some(LayoutChoice::Mac)
        );
        assert_eq!(
            probed.keyboard.style_of("keyboard:usb:1a86:e010"),
            KeyboardStyle::Windows
        );

        let fresh = super::with_keyboard_unmapped(Settings::default(), "keyboard:new");
        assert_eq!(fresh.keyboard.style_of("keyboard:new"), KeyboardStyle::Mac);
        assert_eq!(settings.keyboard.devices.len(), 1);
    }
}
