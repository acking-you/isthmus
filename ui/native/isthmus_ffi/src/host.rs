//! The one service per process, and its life: not started, running, shut
//! down.
//!
//! The service starts on the first `start` op and never twice: later
//! `start`s return the running service. Shutdown (Swift's quit hook through
//! `isthmus_shutdown`, or the `shutdown` op) is final for the process: a
//! `start` racing the quit must not put the mappings back on while the app
//! exits. Start and shutdown hold the lifecycle lock while they run, so a
//! shutdown arriving during a start waits for it and then undoes it, and a
//! second shutdown returns only once the first has put everything back.
//! Every other op holds the lock only to clone the service, so a slow op
//! never holds up another.
//!
//! The ops reach the service through the [`Service`] trait, which
//! [`InputService`] implements and the tests implement with a fake, so the
//! lifecycle and the op table are tested without touching the system.
//!
//! File sharing ([`LinkService`]) starts with it, through the [`Link`]
//! trait, and lives and dies with it, but its failure to start never stops
//! the keyboard and mouse: the link ops answer with that failure instead.
//! A link that could not start because another Isthmus holds its lock is
//! tried again on the next link op, so quitting the other copy is enough.
//! Keeping the computer awake ([`KeepAwake`], through the [`Awake`] trait)
//! starts and stops with them the same way.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use isthmus_awake::{AwakeStatus, Error as AwakeError, KeepAwake};
use isthmus_input::context::FrontApp;
use isthmus_input::status::{DeviceEntry, LoginItem, PermissionKind, SettingsPane};
use isthmus_input::{
    ChangeSink, Error as ServiceError, InputService, Preset, PresetTarget, ServiceOptions,
    Settings, Status,
};
use isthmus_link::{DeviceId, Error as LinkError, LinkOptions, LinkService, LinkStatus};
use parking_lot::Mutex;

use crate::error::{CtlError, ErrorCode};
use crate::events;
use crate::response::panic_text;

/// What the ops need from the service: [`InputService`]'s methods, as a
/// trait so the tests can stand in a fake.
pub(crate) trait Service: Send + Sync {
    fn status(&self) -> Status;
    fn settings(&self) -> Settings;
    fn update_settings(&self, settings: Settings) -> Result<Settings, ServiceError>;
    fn apply_preset(&self, preset: Preset, target: PresetTarget) -> Result<Settings, ServiceError>;
    fn devices(&self) -> Vec<DeviceEntry>;
    fn running_apps(&self) -> Vec<FrontApp>;
    fn request_permission(&self, kind: PermissionKind) -> Status;
    fn open_settings_pane(&self, pane: SettingsPane) -> Result<(), ServiceError>;
    fn restore(&self) -> Result<Settings, ServiceError>;
    fn begin_layout_probe(&self, device_id: &str) -> Result<(), ServiceError>;
    fn end_layout_probe(&self);
    fn login_item(&self) -> LoginItem;
    fn set_login_item(&self, enabled: bool) -> Result<LoginItem, ServiceError>;
    fn shutdown(&self);
}

impl Service for InputService {
    fn status(&self) -> Status {
        InputService::status(self)
    }

    fn settings(&self) -> Settings {
        InputService::settings(self)
    }

    fn update_settings(&self, settings: Settings) -> Result<Settings, ServiceError> {
        InputService::update_settings(self, settings)
    }

    fn apply_preset(&self, preset: Preset, target: PresetTarget) -> Result<Settings, ServiceError> {
        InputService::apply_preset(self, preset, target)
    }

    fn devices(&self) -> Vec<DeviceEntry> {
        InputService::devices(self)
    }

    fn running_apps(&self) -> Vec<FrontApp> {
        InputService::running_apps(self)
    }

    fn request_permission(&self, kind: PermissionKind) -> Status {
        InputService::request_permission(self, kind)
    }

    fn open_settings_pane(&self, pane: SettingsPane) -> Result<(), ServiceError> {
        InputService::open_settings_pane(self, pane)
    }

    fn restore(&self) -> Result<Settings, ServiceError> {
        InputService::restore(self)
    }

    fn begin_layout_probe(&self, device_id: &str) -> Result<(), ServiceError> {
        InputService::begin_layout_probe(self, device_id)
    }

    fn end_layout_probe(&self) {
        InputService::end_layout_probe(self)
    }

    fn login_item(&self) -> LoginItem {
        InputService::login_item(self)
    }

    fn set_login_item(&self, enabled: bool) -> Result<LoginItem, ServiceError> {
        InputService::set_login_item(self, enabled)
    }

    fn shutdown(&self) {
        InputService::shutdown(self)
    }
}

/// What the link ops need from file sharing: [`LinkService`]'s methods, as
/// a trait so the tests can stand in a fake.
pub(crate) trait Link: Send + Sync {
    fn status(&self) -> LinkStatus;
    fn pair(&self, peer: &DeviceId) -> Result<(), LinkError>;
    fn enter_code(&self, code: &str) -> Result<(), LinkError>;
    fn cancel_pairing(&self);
    fn unpair(&self, peer: &DeviceId) -> Result<(), LinkError>;
    fn send(&self, peer: &DeviceId, paths: Vec<PathBuf>) -> Result<String, LinkError>;
    fn cancel_transfer(&self, id: &str) -> Result<(), LinkError>;
    fn clear_transfers(&self);
    fn rename(&self, name: &str) -> Result<(), LinkError>;
    fn reveal(&self, id: Option<&str>) -> Result<(), LinkError>;
    fn shutdown(&self);
}

impl Link for LinkService {
    fn status(&self) -> LinkStatus {
        LinkService::status(self)
    }

    fn pair(&self, peer: &DeviceId) -> Result<(), LinkError> {
        LinkService::pair(self, peer)
    }

    fn enter_code(&self, code: &str) -> Result<(), LinkError> {
        LinkService::enter_code(self, code)
    }

    fn cancel_pairing(&self) {
        LinkService::cancel_pairing(self)
    }

    fn unpair(&self, peer: &DeviceId) -> Result<(), LinkError> {
        LinkService::unpair(self, peer)
    }

    fn send(&self, peer: &DeviceId, paths: Vec<PathBuf>) -> Result<String, LinkError> {
        LinkService::send(self, peer, paths)
    }

    fn cancel_transfer(&self, id: &str) -> Result<(), LinkError> {
        LinkService::cancel_transfer(self, id)
    }

    fn clear_transfers(&self) {
        LinkService::clear_transfers(self)
    }

    fn rename(&self, name: &str) -> Result<(), LinkError> {
        LinkService::rename(self, name)
    }

    fn reveal(&self, id: Option<&str>) -> Result<(), LinkError> {
        LinkService::reveal(self, id)
    }

    fn shutdown(&self) {
        LinkService::shutdown(self)
    }
}

/// What the awake ops need: [`KeepAwake`]'s methods, as a trait so the
/// tests never hold a real power request.
pub(crate) trait Awake: Send + Sync {
    fn status(&self) -> AwakeStatus;
    fn set(&self, enabled: bool) -> Result<AwakeStatus, AwakeError>;
    fn shutdown(&self);
}

impl Awake for KeepAwake {
    fn status(&self) -> AwakeStatus {
        KeepAwake::status(self)
    }

    fn set(&self, enabled: bool) -> Result<AwakeStatus, AwakeError> {
        KeepAwake::set(self, enabled)
    }

    fn shutdown(&self) {
        KeepAwake::shutdown(self)
    }
}

/// Starts the service; the host calls it at most once per process.
pub(crate) type Starter = Box<dyn Fn() -> Result<Arc<dyn Service>, ServiceError> + Send + Sync>;

/// Starts file sharing, answering its failure as the link ops will.
pub(crate) type LinkStarter = Box<dyn Fn() -> Result<Arc<dyn Link>, CtlError> + Send + Sync>;

/// Reads the keep-awake choice and holds the request it asks for,
/// answering its failure as the awake ops will.
pub(crate) type AwakeStarter = Box<dyn Fn() -> Result<Arc<dyn Awake>, CtlError> + Send + Sync>;

/// Where the service is in its life.
enum Lifecycle {
    /// No `start` has succeeded yet.
    Idle,
    /// Started; every op talks to this one.
    Running(Running),
    /// Shut down; this process does not start it again.
    Stopped,
}

/// The running services.
struct Running {
    service: Arc<dyn Service>,
    /// File sharing, or why it did not start.
    link: Result<Arc<dyn Link>, CtlError>,
    /// Keeping the computer awake, or why its choice could not be read.
    awake: Result<Arc<dyn Awake>, CtlError>,
}

/// What a shutdown found, so the op can say what happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shutdown {
    /// The service was running and has now put everything back.
    Stopped,
    /// Nothing had started; nothing will start now.
    NeverStarted,
    /// An earlier shutdown already did it.
    AlreadyStopped,
}

/// The process's service slot and how to fill it.
pub struct Host {
    lifecycle: Mutex<Lifecycle>,
    starter: Starter,
    link_starter: LinkStarter,
    awake_starter: AwakeStarter,
}

impl Host {
    pub(crate) fn new(
        starter: Starter,
        link_starter: LinkStarter,
        awake_starter: AwakeStarter,
    ) -> Self {
        Self {
            lifecycle: Mutex::new(Lifecycle::Idle),
            starter,
            link_starter,
            awake_starter,
        }
    }

    /// The running service, starting it if this is the first `start`. A
    /// failed start leaves the host idle, so the next `start` tries again
    /// (after the user fixed the settings file, say).
    pub(crate) fn start(&self) -> Result<Arc<dyn Service>, CtlError> {
        let mut lifecycle = self.lifecycle.lock();
        match &*lifecycle {
            Lifecycle::Running(running) => Ok(running.service.clone()),
            Lifecycle::Stopped => Err(shut_down()),
            Lifecycle::Idle => {
                let service = (self.starter)().map_err(CtlError::from_service)?;
                let link = (self.link_starter)();
                if let Err(error) = &link {
                    tracing::warn!(code = ?error.code(), %error, "file sharing did not start");
                }
                let awake = (self.awake_starter)();
                if let Err(error) = &awake {
                    tracing::warn!(code = ?error.code(), %error, "keep-awake did not start");
                }
                *lifecycle = Lifecycle::Running(Running {
                    service: service.clone(),
                    link,
                    awake,
                });
                Ok(service)
            }
        }
    }

    /// The running service, for every op but `start` and `shutdown`.
    pub(crate) fn running(&self) -> Result<Arc<dyn Service>, CtlError> {
        match &*self.lifecycle.lock() {
            Lifecycle::Running(running) => Ok(running.service.clone()),
            Lifecycle::Idle => Err(CtlError::not_running("Isthmus has not started yet")),
            Lifecycle::Stopped => Err(shut_down()),
        }
    }

    /// File sharing, for the link ops: its failure to start when it did
    /// not, after one more try if another Isthmus held its lock then.
    pub(crate) fn link(&self) -> Result<Arc<dyn Link>, CtlError> {
        let mut lifecycle = self.lifecycle.lock();
        let running = match &mut *lifecycle {
            Lifecycle::Running(running) => running,
            Lifecycle::Idle => return Err(CtlError::not_running("Isthmus has not started yet")),
            Lifecycle::Stopped => return Err(shut_down()),
        };
        if matches!(&running.link, Err(error) if error.code() == ErrorCode::AnotherInstance) {
            running.link = (self.link_starter)();
        }
        running.link.clone()
    }

    /// Keeping the computer awake, for the awake ops: why its choice could
    /// not be read when it could not.
    pub(crate) fn awake(&self) -> Result<Arc<dyn Awake>, CtlError> {
        match &*self.lifecycle.lock() {
            Lifecycle::Running(running) => running.awake.clone(),
            Lifecycle::Idle => Err(CtlError::not_running("Isthmus has not started yet")),
            Lifecycle::Stopped => Err(shut_down()),
        }
    }

    /// Puts everything back and stops the service, once; later calls wait
    /// for that one and report it.
    pub(crate) fn shutdown(&self) -> Shutdown {
        let mut lifecycle = self.lifecycle.lock();
        match std::mem::replace(&mut *lifecycle, Lifecycle::Stopped) {
            Lifecycle::Running(running) => {
                running.service.shutdown();
                if let Ok(link) = running.link {
                    link.shutdown();
                }
                if let Ok(awake) = running.awake {
                    awake.shutdown();
                }
                Shutdown::Stopped
            }
            Lifecycle::Idle => Shutdown::NeverStarted,
            Lifecycle::Stopped => Shutdown::AlreadyStopped,
        }
    }
}

/// The answer for an op after shutdown.
fn shut_down() -> CtlError {
    CtlError::not_running("Isthmus has shut down in this process")
}

/// The process's host, with the real service behind it.
static HOST: LazyLock<Host> = LazyLock::new(|| {
    Host::new(
        Box::new(start_input_service),
        Box::new(start_link_service),
        Box::new(start_keep_awake),
    )
});

/// The process's host.
pub(crate) fn global() -> &'static Host {
    &HOST
}

/// Starts [`InputService`] on the user's data directory, with its change
/// hints going to Dart's change callback.
fn start_input_service() -> Result<Arc<dyn Service>, ServiceError> {
    let data_dir =
        isthmus_core::paths::data_dir().map_err(|source| ServiceError::DataDir { source })?;
    let on_change: ChangeSink = Arc::new(events::emit);
    let service = InputService::start(ServiceOptions {
        data_dir,
        on_change: Some(on_change),
    })?;
    Ok(Arc::new(service))
}

/// Starts file sharing on the user's data directory, with its change hints
/// going to Dart's change callback.
fn start_link_service() -> Result<Arc<dyn Link>, CtlError> {
    let data_dir = isthmus_core::paths::data_dir()
        .map_err(|error| CtlError::new(ErrorCode::Io, error.to_string()))?;
    let on_change: isthmus_link::ChangeSink = Arc::new(events::emit_link);
    let service = LinkService::start(LinkOptions::new(data_dir, Some(on_change)))
        .map_err(CtlError::from_link)?;
    Ok(Arc::new(service))
}

/// Reads the keep-awake choice in the user's data directory and holds the
/// request when it is on.
fn start_keep_awake() -> Result<Arc<dyn Awake>, CtlError> {
    let data_dir = isthmus_core::paths::data_dir()
        .map_err(|error| CtlError::new(ErrorCode::Io, error.to_string()))?;
    let keep = KeepAwake::start(&data_dir).map_err(CtlError::from_awake)?;
    Ok(Arc::new(keep))
}

/// Puts every system change back and stops the service, if it started. For
/// Swift's `applicationWillTerminate`, which has no handle. Idempotent, and
/// returns only when everything is back, so the process may exit right
/// after; a second call waits for a shutdown already in progress.
#[unsafe(no_mangle)]
pub extern "C" fn isthmus_shutdown() {
    // A panic must not unwind into Swift: that aborts, and the quit would
    // then skip whatever the rest of `applicationWillTerminate` does.
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| global().shutdown())) {
        tracing::error!(panic = panic_text(payload.as_ref()), "shutdown panicked");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::error::ErrorCode;
    use crate::test_support::{FakeService, host_with};

    /// The service starts once per process: Dart's hot restart calls
    /// `start` again and must get the running service, not a second one.
    #[test]
    fn start_starts_once_and_then_returns_the_running_service() {
        let fake = Arc::new(FakeService::default());
        let (host, starts) = host_with(fake.clone());
        let first = host.start().unwrap();
        let second = host.start().unwrap();
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&first, &second));
        assert!(host.running().is_ok());
    }

    /// Before a start, the ops have nothing to talk to and say so with the
    /// code the window branches on.
    #[test]
    fn ops_before_start_are_not_running() {
        let (host, starts) = host_with(Arc::new(FakeService::default()));
        let error = host.running().err().unwrap();
        assert_eq!(error.code(), ErrorCode::NotRunning);
        assert_eq!(starts.load(Ordering::SeqCst), 0, "asking never starts it");
    }

    /// A start that fails (an unreadable settings file) leaves the host
    /// idle, so a later start can succeed once the cause is fixed.
    #[test]
    fn a_failed_start_can_be_retried() {
        let fake = Arc::new(FakeService::default());
        fake.fail_start.store(true, Ordering::SeqCst);
        let (host, starts) = host_with(fake.clone());
        assert_eq!(host.start().err().unwrap().code(), ErrorCode::Io);
        assert_eq!(host.running().err().unwrap().code(), ErrorCode::NotRunning);
        fake.fail_start.store(false, Ordering::SeqCst);
        assert!(host.start().is_ok());
        assert_eq!(starts.load(Ordering::SeqCst), 2);
    }

    /// Shutdown puts everything back once, and is final: neither ops nor a
    /// racing `start` bring the service back while the app quits.
    #[test]
    fn shutdown_is_idempotent_and_final() {
        let fake = Arc::new(FakeService::default());
        let (host, starts) = host_with(fake.clone());
        host.start().unwrap();
        assert_eq!(host.shutdown(), Shutdown::Stopped);
        assert_eq!(host.shutdown(), Shutdown::AlreadyStopped);
        assert_eq!(fake.shutdowns.load(Ordering::SeqCst), 1);
        assert_eq!(host.running().err().unwrap().code(), ErrorCode::NotRunning);
        assert_eq!(host.start().err().unwrap().code(), ErrorCode::NotRunning);
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    /// A quit before anything started has nothing to put back, and a
    /// `start` after it does not start the service.
    #[test]
    fn shutdown_before_start_keeps_it_from_starting() {
        let fake = Arc::new(FakeService::default());
        let (host, starts) = host_with(fake.clone());
        assert_eq!(host.shutdown(), Shutdown::NeverStarted);
        assert_eq!(host.start().err().unwrap().code(), ErrorCode::NotRunning);
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        assert_eq!(fake.shutdowns.load(Ordering::SeqCst), 0);
    }

    /// Swift's quit hook may arrive while the `shutdown` op is still putting
    /// things back; it must return only after that finishes, or the process
    /// exits with the keyboard still remapped.
    #[test]
    fn a_second_shutdown_waits_for_the_first_to_finish() {
        let fake = Arc::new(FakeService::default());
        let (entered, shutdown_entered) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        *fake.on_shutdown.lock() = Some(Box::new(move || {
            entered.send(()).unwrap();
            released.recv().unwrap();
        }));
        let (host, _) = host_with(fake.clone());
        host.start().unwrap();

        let first = std::thread::spawn(move || host.shutdown());
        shutdown_entered.recv().unwrap();
        let (done, second_done) = mpsc::channel();
        let second = std::thread::spawn(move || done.send(host.shutdown()).unwrap());
        assert!(
            second_done
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "the second shutdown returned while the first was still running"
        );
        release.send(()).unwrap();
        assert_eq!(first.join().unwrap(), Shutdown::Stopped);
        assert_eq!(second_done.recv().unwrap(), Shutdown::AlreadyStopped);
        second.join().unwrap();
        assert_eq!(fake.shutdowns.load(Ordering::SeqCst), 1);
    }
}
