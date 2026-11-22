//! `isthmus inspect`: raw key, modifier, button and wheel events, with the
//! fields Isthmus reads, until Ctrl-C. For checking what a keyboard or
//! mouse actually sends (which keycode a key has, whether a wheel is
//! notched, which service an event names) before blaming a rule.
//!
//! The listen-only tap runs on the main thread's run loop and stops when a
//! flag is set: Ctrl-C sets it through [`Signals`], and so does a closed
//! stdout (`isthmus inspect | head`), so the command ends instead of
//! panicking on a broken pipe.

use std::io::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use isthmus_input::platform;
use snafu::ResultExt;

use crate::error::{CliError, InspectSnafu, SignalsSnafu};
use crate::signals::Signals;

/// Prints events until Ctrl-C.
pub(crate) fn run() -> Result<(), CliError> {
    let mut signals = Signals::install().context(SignalsSnafu)?;
    let stop = Arc::new(AtomicBool::new(false));
    let on_signal = stop.clone();
    std::thread::Builder::new()
        .name("isthmus-cli-signals".to_owned())
        .spawn(move || {
            // An error means Ctrl-C can no longer be seen: stop either way.
            let _ = signals.wait();
            on_signal.store(true, Ordering::SeqCst);
        })
        .map_err(|source| CliError::Signals { source })?;

    eprintln!("Watching key, modifier, button and wheel events. Ctrl-C stops.");
    let on_closed = stop.clone();
    let mut stdout = std::io::stdout();
    platform::inspect_events(stop, move |line| {
        if writeln!(stdout, "{line}").is_err() {
            on_closed.store(true, Ordering::SeqCst);
        }
    })
    .context(InspectSnafu)
}
