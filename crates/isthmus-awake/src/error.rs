//! What keeping the computer awake reports failing to its callers.

use std::path::PathBuf;

use snafu::Snafu;

/// What went wrong, by the step that failed.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    /// `awake.json` could not be read.
    #[snafu(display("could not read {}", path.display()))]
    ReadSettings {
        path: PathBuf,
        source: std::io::Error,
    },

    /// `awake.json` is not what this version can read; it is left alone.
    #[snafu(display("{} is not valid", path.display()))]
    ParseSettings {
        path: PathBuf,
        source: serde_json::Error,
    },

    /// `awake.json` could not be replaced.
    #[snafu(display("could not save the keep-awake choice"))]
    WriteSettings { source: isthmus_core::Error },

    /// This system has no way to keep itself awake on request.
    #[snafu(display("keeping the computer awake is not available on this system"))]
    Unsupported,

    /// The system refused the request; the choice is saved and tried again
    /// at the next launch.
    #[snafu(display("the system refused to stay awake: {reason}"))]
    Refused { reason: String },

    /// The app is quitting and holds nothing more.
    #[snafu(display("Isthmus is quitting"))]
    ShutDown,
}
