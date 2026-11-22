//! Side effects that may block, done off the tap thread (tap half).
//!
//! The tap callback must answer the window server within milliseconds or
//! macOS disables the tap, so anything that waits (launching an app, opening
//! a Finder window) goes to the `isthmus-actions` worker through a bounded
//! queue. The callback never waits on the queue: when it is full the action
//! is dropped and logged, which costs one missed shortcut instead of a frozen
//! keyboard. The worker runs `/usr/bin/open`, which needs no AppKit and no
//! main thread, with stdin closed, and reaps every child.

use std::ffi::{OsStr, OsString};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};

use tracing::{debug, error, warn};

use crate::engine::Action;

/// How many actions may wait. A person presses a few shortcuts a second at
/// most; more than this queued means the worker is stuck, and waiting longer
/// would not help.
pub(crate) const QUEUE_CAPACITY: usize = 16;

/// The worker thread's name, as `ps` and crash reports show it.
const THREAD_NAME: &str = "isthmus-actions";

/// The program every action runs. LaunchServices does the work; `open`
/// returns as soon as the request is made.
const OPEN: &str = "/usr/bin/open";

/// What [`Actions::submit`] did with an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Submitted {
    Queued,
    /// The queue was full; the action was dropped.
    Full,
    /// The worker is gone; the action was dropped.
    Closed,
}

/// The sending end of the worker's queue. Dropping it ends the worker once
/// the action in hand is done.
#[derive(Debug)]
pub(crate) struct Actions {
    queue: SyncSender<Action>,
    /// Kept so the worker is not detached silently; never joined, because
    /// the worker may be waiting on a child and nobody should wait for it.
    _worker: JoinHandle<()>,
}

impl Actions {
    /// Starts the worker that runs actions with `/usr/bin/open`.
    pub(crate) fn start() -> std::io::Result<Actions> {
        Actions::start_with(QUEUE_CAPACITY, run)
    }

    /// Starts a worker that hands each action to `perform`, in order. The
    /// seam that lets the tests check queueing without launching anything.
    fn start_with(
        capacity: usize,
        perform: impl FnMut(Action) + Send + 'static,
    ) -> std::io::Result<Actions> {
        let (queue, inbox) = sync_channel(capacity);
        let worker = thread::Builder::new()
            .name(THREAD_NAME.to_owned())
            .spawn(move || work(&inbox, perform))?;
        Ok(Actions {
            queue,
            _worker: worker,
        })
    }

    /// Queues `action` without waiting: safe to call from the tap callback.
    pub(crate) fn submit(&self, action: Action) -> Submitted {
        match self.queue.try_send(action) {
            Ok(()) => Submitted::Queued,
            Err(TrySendError::Full(_)) => Submitted::Full,
            Err(TrySendError::Disconnected(_)) => Submitted::Closed,
        }
    }
}

/// The worker's loop: ends when every sender is gone.
fn work(inbox: &Receiver<Action>, mut perform: impl FnMut(Action)) {
    for action in inbox {
        perform(action);
    }
    debug!("the actions worker stopped");
}

/// A command line, as data, so the tests can check it without running it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Invocation {
    program: &'static str,
    args: Vec<OsString>,
}

/// The command for `action`. `home` is the user's home folder; without one
/// Win+E has nowhere to go.
fn invocation(action: Action, home: Option<&OsStr>) -> Option<Invocation> {
    match action {
        Action::OpenApp { bundle_id } => Some(Invocation {
            program: OPEN,
            args: vec![OsString::from("-b"), OsString::from(bundle_id)],
        }),
        Action::OpenHomeInFinder => home.map(|home| Invocation {
            program: OPEN,
            args: vec![home.to_owned()],
        }),
    }
}

/// Runs one action and waits for `open` to exit, so no zombie is left.
/// Failures are logged: there is nobody to return them to.
fn run(action: Action) {
    let home = std::env::var_os("HOME");
    let Some(Invocation { program, args }) = invocation(action, home.as_deref()) else {
        warn!(?action, "no home folder to open");
        return;
    };
    let child = Command::new(program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let status = match child {
        Ok(mut child) => child.wait(),
        Err(source) => {
            error!(?action, error = %source, "could not start {program}");
            return;
        }
    };
    match status {
        Ok(status) if status.success() => debug!(?action, "done"),
        Ok(status) => warn!(?action, %status, "{program} failed"),
        Err(source) => error!(?action, error = %source, "could not wait for {program}"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use super::*;

    /// Opening an app goes by bundle id, so it works wherever the app is
    /// installed and brings a running one forward.
    #[test]
    fn opening_an_app_uses_its_bundle_id() {
        let command = invocation(
            Action::OpenApp {
                bundle_id: "com.apple.ActivityMonitor",
            },
            None,
        );
        assert_eq!(
            command,
            Some(Invocation {
                program: "/usr/bin/open",
                args: vec!["-b".into(), "com.apple.ActivityMonitor".into()],
            })
        );
    }

    /// Win+E opens the home folder, and does nothing without one.
    #[test]
    fn win_e_opens_the_home_folder_when_there_is_one() {
        let command = invocation(Action::OpenHomeInFinder, Some(OsStr::new("/Users/someone")));
        assert_eq!(
            command.map(|command| command.args),
            Some(vec![OsString::from("/Users/someone")])
        );
        assert_eq!(invocation(Action::OpenHomeInFinder, None), None);
    }

    /// The worker runs actions in the order they were submitted.
    #[test]
    fn the_worker_runs_actions_in_order() {
        let (seen, done) = channel();
        let actions = Actions::start_with(4, move |action| {
            seen.send(action).expect("the test is listening");
        })
        .expect("a worker thread");
        let first = Action::OpenHomeInFinder;
        let second = Action::OpenApp { bundle_id: "a.b" };
        assert_eq!(actions.submit(first), Submitted::Queued);
        assert_eq!(actions.submit(second), Submitted::Queued);
        let timeout = Duration::from_secs(5);
        assert_eq!(done.recv_timeout(timeout), Ok(first));
        assert_eq!(done.recv_timeout(timeout), Ok(second));
    }

    /// A full queue drops the action at once instead of blocking the tap
    /// callback.
    #[test]
    fn a_full_queue_drops_instead_of_waiting() {
        let (started, worker_busy) = channel();
        let (release, held) = channel::<()>();
        let actions = Actions::start_with(1, move |_| {
            let _ = started.send(());
            let _ = held.recv();
        })
        .expect("a worker thread");
        let action = Action::OpenHomeInFinder;
        assert_eq!(actions.submit(action), Submitted::Queued);
        worker_busy
            .recv_timeout(Duration::from_secs(5))
            .expect("the worker took the first action");
        assert_eq!(actions.submit(action), Submitted::Queued, "one slot waits");
        assert_eq!(actions.submit(action), Submitted::Full);
        drop(release);
    }
}
