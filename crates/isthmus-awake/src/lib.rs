//! Keeping the computer awake while its screen sleeps.
//!
//! The user's one choice, `enabled`, lives in `awake.json` in the data
//! directory. While it is on, the app holds one power request of the kind
//! that stops the system sleeping when nobody touches it, and no other: the
//! display still turns off on its own schedule, the screen saver and the
//! lock still come, and the computer still sleeps when it is told to (the
//! lid closed, the Sleep menu item). Everything else keeps running: downloads,
//! builds, remote sessions, file sharing.
//!
//! The request belongs to this process, and the system drops it when the
//! process ends, however it ends. So nothing is journalled and nothing needs
//! putting back after a crash: the next launch holds it again if the choice
//! is still on.
//!
//! Holding it is a change to the live system, so the decisions here run
//! against a [`Power`] the tests replace; only `platform` names an OS API
//! (macOS's IOKit power assertions, Windows' power requests).

mod error;
mod platform;
mod settings;

#[cfg(test)]
mod test_support;

use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::Serialize;

pub use crate::error::Error;
pub use crate::settings::{AwakeSettings, SETTINGS_FILE};

/// What the system is asked for, behind a trait so the tests hold nothing.
pub trait Power: Send + Sync {
    /// Whether this system can keep itself awake on request at all.
    fn supported(&self) -> bool;

    /// Asks the system to stay awake until the returned value is dropped.
    /// The error is the system's own sentence for why it refused.
    fn hold(&self) -> Result<Box<dyn Held>, String>;
}

/// A request the system is honouring; dropping it lets the system sleep
/// again.
pub trait Held: Send {}

/// This build's operating system.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPower;

impl Power for SystemPower {
    fn supported(&self) -> bool {
        platform::SUPPORTED
    }

    fn hold(&self) -> Result<Box<dyn Held>, String> {
        platform::hold(REASON).map(|held| Box::new(held) as Box<dyn Held>)
    }
}

/// How the request is named where the system lists them (`pmset -g
/// assertions`, `powercfg /requests`), so whoever finds the computer awake
/// can tell why.
pub const REASON: &str = "Isthmus keeps this computer awake (the screen may still turn off)";

/// What the app shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AwakeStatus {
    /// What the user chose.
    pub enabled: bool,
    /// Whether the system is holding the request right now.
    pub active: bool,
    /// Whether this system can do it at all.
    pub supported: bool,
    /// The system's reason for refusing the request, while it is chosen but
    /// not held.
    pub error: Option<String>,
}

/// The choice and the request it holds.
pub struct KeepAwake {
    file: PathBuf,
    power: Box<dyn Power>,
    state: Mutex<State>,
}

struct State {
    settings: AwakeSettings,
    held: Option<Box<dyn Held>>,
    error: Option<String>,
    /// Released for good: the process is quitting.
    stopped: bool,
}

impl KeepAwake {
    /// Reads the choice in `data_dir` and, when it is on, holds the request
    /// on this system.
    pub fn start(data_dir: &Path) -> Result<KeepAwake, Error> {
        Self::with_power(data_dir, Box::new(SystemPower))
    }

    /// [`KeepAwake::start`] against any [`Power`]. A settings file that
    /// cannot be read is an error; a system that refuses the request is not,
    /// since the choice still stands, and the status says why it is not
    /// held.
    pub fn with_power(data_dir: &Path, power: Box<dyn Power>) -> Result<KeepAwake, Error> {
        let file = data_dir.join(SETTINGS_FILE);
        let settings = settings::load(&file)?;
        let keep = KeepAwake {
            file,
            power,
            state: Mutex::new(State {
                settings,
                held: None,
                error: None,
                stopped: false,
            }),
        };
        {
            let mut state = keep.state.lock();
            keep.apply(&mut state);
        }
        Ok(keep)
    }

    /// The choice and whether it is in effect.
    pub fn status(&self) -> AwakeStatus {
        self.status_of(&self.state.lock())
    }

    /// Saves the choice, then holds or lets go of the request to match it.
    /// Fails without saving on a system that cannot do it, and after
    /// saving when the system refuses: the choice stands and the next
    /// launch tries again.
    pub fn set(&self, enabled: bool) -> Result<AwakeStatus, Error> {
        if enabled && !self.power.supported() {
            return Err(Error::Unsupported);
        }
        let mut state = self.state.lock();
        if state.stopped {
            return Err(Error::ShutDown);
        }
        if state.settings.enabled != enabled {
            let mut next = state.settings.clone();
            next.enabled = enabled;
            settings::save(&self.file, &next)?;
            state.settings = next;
        }
        self.apply(&mut state);
        match &state.error {
            Some(reason) => Err(Error::Refused {
                reason: reason.clone(),
            }),
            None => Ok(self.status_of(&state)),
        }
    }

    /// Lets the system sleep again and holds nothing more, for the quit.
    /// The choice is kept for the next launch.
    pub fn shutdown(&self) {
        let mut state = self.state.lock();
        state.stopped = true;
        state.held = None;
    }

    /// Makes the request match the choice.
    fn apply(&self, state: &mut State) {
        state.error = None;
        if !state.settings.enabled || !self.power.supported() || state.stopped {
            if state.held.take().is_some() {
                tracing::info!("the computer may sleep on its own again");
            }
            return;
        }
        if state.held.is_some() {
            return;
        }
        match self.power.hold() {
            Ok(held) => {
                tracing::info!("keeping the computer awake; the screen may still turn off");
                state.held = Some(held);
            }
            Err(reason) => {
                tracing::warn!(%reason, "the system refused to stay awake");
                state.error = Some(reason);
            }
        }
    }

    fn status_of(&self, state: &State) -> AwakeStatus {
        AwakeStatus {
            enabled: state.settings.enabled,
            active: state.held.is_some(),
            supported: self.power.supported(),
            error: state.error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;
    use crate::test_support::ScratchDir;

    /// A system that counts the requests it holds, and refuses or lacks
    /// the feature on command.
    #[derive(Default)]
    struct FakePower {
        held: Arc<AtomicUsize>,
        refuse: AtomicBool,
        unsupported: bool,
    }

    struct FakeHeld(Arc<AtomicUsize>);

    impl Held for FakeHeld {}

    impl Drop for FakeHeld {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl Power for Arc<FakePower> {
        fn supported(&self) -> bool {
            !self.unsupported
        }

        fn hold(&self) -> Result<Box<dyn Held>, String> {
            if self.refuse.load(Ordering::SeqCst) {
                return Err("no".into());
            }
            self.held.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeHeld(self.held.clone())))
        }
    }

    fn held(power: &Arc<FakePower>) -> usize {
        power.held.load(Ordering::SeqCst)
    }

    /// Off by default; turned on, exactly one request is held and the
    /// choice is saved; off again, it is let go and that is saved too.
    #[test]
    fn the_switch_holds_one_request_and_is_remembered() {
        let scratch = ScratchDir::new("switch");
        let power = Arc::new(FakePower::default());
        let keep = KeepAwake::with_power(scratch.path(), Box::new(power.clone())).unwrap();
        assert_eq!(
            keep.status(),
            AwakeStatus {
                enabled: false,
                active: false,
                supported: true,
                error: None
            }
        );
        assert_eq!(held(&power), 0);

        assert!(keep.set(true).unwrap().active);
        keep.set(true).unwrap();
        assert_eq!(held(&power), 1, "a second `on` holds nothing more");

        let again = KeepAwake::with_power(scratch.path(), Box::new(power.clone())).unwrap();
        assert!(again.status().enabled, "the choice survives a restart");
        drop(again);

        let off = keep.set(false).unwrap();
        assert!(!off.enabled && !off.active);
        assert_eq!(held(&power), 0);
    }

    /// A launch with the choice on holds the request at once, and the quit
    /// lets go of it without forgetting the choice.
    #[test]
    fn a_launch_holds_it_and_the_quit_lets_go() {
        let scratch = ScratchDir::new("launch");
        let power = Arc::new(FakePower::default());
        KeepAwake::with_power(scratch.path(), Box::new(power.clone()))
            .unwrap()
            .set(true)
            .unwrap();
        assert_eq!(held(&power), 0, "dropping the owner lets go");

        let keep = KeepAwake::with_power(scratch.path(), Box::new(power.clone())).unwrap();
        assert_eq!(held(&power), 1);
        keep.shutdown();
        assert_eq!(held(&power), 0);
        assert!(keep.status().enabled);
        assert!(matches!(keep.set(true), Err(Error::ShutDown)));
        assert_eq!(held(&power), 0, "nothing is held after the quit");
    }

    /// A refusal keeps the choice, reports the system's reason, and the
    /// next attempt holds the request once the system agrees.
    #[test]
    fn a_refusal_keeps_the_choice_and_says_why() {
        let scratch = ScratchDir::new("refusal");
        let power = Arc::new(FakePower::default());
        power.refuse.store(true, Ordering::SeqCst);
        let keep = KeepAwake::with_power(scratch.path(), Box::new(power.clone())).unwrap();
        assert!(matches!(keep.set(true), Err(Error::Refused { .. })));
        let status = keep.status();
        assert!(status.enabled && !status.active);
        assert_eq!(status.error.as_deref(), Some("no"));

        power.refuse.store(false, Ordering::SeqCst);
        assert!(keep.set(true).unwrap().active);
        assert_eq!(held(&power), 1);
    }

    /// Where the system cannot do it, turning it on fails and saves
    /// nothing.
    #[test]
    fn an_unsupported_system_refuses_without_saving() {
        let scratch = ScratchDir::new("unsupported");
        let power = Arc::new(FakePower {
            unsupported: true,
            ..FakePower::default()
        });
        let keep = KeepAwake::with_power(scratch.path(), Box::new(power.clone())).unwrap();
        assert!(!keep.status().supported);
        assert!(matches!(keep.set(true), Err(Error::Unsupported)));
        assert!(!scratch.path().join(SETTINGS_FILE).exists());
        assert!(keep.set(false).is_ok());
    }
}
