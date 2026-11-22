//! Failures the service reports. Callers branch on the variant, never on
//! the message.

use std::path::PathBuf;

use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("could not read the settings file {}", path.display()))]
    ReadSettings {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display("the settings file {} is not valid", path.display()))]
    ParseSettings {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[snafu(display("could not save {}", path.display()))]
    WriteFile {
        path: PathBuf,
        source: isthmus_core::Error,
    },

    #[snafu(display("could not encode {what}"))]
    Encode {
        what: &'static str,
        source: serde_json::Error,
    },

    #[snafu(display("the data directory is not available"))]
    DataDir { source: isthmus_core::Error },

    #[snafu(display("another Isthmus is running and owns the keyboard and mouse"))]
    AnotherInstance,

    #[snafu(display("Isthmus is not running"))]
    NotRunning,

    #[snafu(display("{what} is not supported on this system"))]
    Unsupported { what: &'static str },

    #[snafu(display("macOS has not granted {what}"))]
    PermissionMissing { what: &'static str },

    #[snafu(display("macOS refused {what} (status {status})"))]
    System { what: &'static str, status: i64 },
}
