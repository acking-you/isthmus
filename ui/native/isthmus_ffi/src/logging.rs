//! Logs go to stderr, which Xcode's console and `flutter run` show and
//! Console.app collects for a launched app. The level comes from `RUST_LOG`,
//! else `info`.

use std::io::IsTerminal;
use std::sync::Once;

use tracing_subscriber::EnvFilter;

static INIT: Once = Once::new();

/// Installs the subscriber once per process; later calls do nothing, so
/// every `isthmus_create` (one per hot restart) may call it.
pub(crate) fn init() {
    INIT.call_once(|| {
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
        // `try_init` fails only when the host process already installed a
        // global subscriber; that one then gets the events, which is fine.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_ansi(std::io::stderr().is_terminal())
            .try_init();
    });
}
