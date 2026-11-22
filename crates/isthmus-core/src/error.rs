//! Failures of the foundations.
//!
//! One enum for the whole crate: its callers (the service, the CLI, the FFI)
//! map each variant to their own code, so a variant names what failed and
//! carries the path the user would look at; the cause stays in `source`.

use std::path::PathBuf;

use snafu::Snafu;

/// What went wrong, by the step that failed.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    /// Neither `ISTHMUS_DATA_DIR` nor `HOME` (`APPDATA` on Windows) says
    /// where the data directory is, so there is nowhere to keep settings or
    /// the journal.
    #[snafu(display("the home directory is not known"))]
    NoHome,

    /// The data directory could not be created.
    #[snafu(display("could not create {}", path.display()))]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },

    /// A state file could not be replaced; `path` is the file that was to be
    /// replaced, which still holds its old contents (or does not exist).
    #[snafu(display("could not write {}", path.display()))]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },

    /// The instance lock file could not be opened or locked for a reason
    /// other than another process holding it.
    #[snafu(display("could not open the lock file {}", path.display()))]
    Lock {
        path: PathBuf,
        source: std::io::Error,
    },
}
