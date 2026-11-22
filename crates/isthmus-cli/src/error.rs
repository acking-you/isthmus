//! What can stop a command. Each variant names the step that failed and
//! keeps the cause as its source; `main` prints the whole chain as
//! `isthmus: <step>: <cause>: …` and exits with 1.

use std::path::PathBuf;

use snafu::Snafu;

/// Why a command failed. The display names the step in words a person can
/// act on; the source says what the system answered.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub(crate) enum CliError {
    #[snafu(display("the data directory is not available"))]
    DataDir { source: isthmus_core::Error },

    #[snafu(display("could not list the keyboards and mice"))]
    ListDevices { source: isthmus_input::Error },

    #[snafu(display("could not read the settings file {}; nothing was changed", path.display()))]
    ReadSettings {
        path: PathBuf,
        source: std::io::Error,
    },

    #[snafu(display(
        "the settings file {} is not valid; fix or remove it, then try again",
        path.display()
    ))]
    ParseSettings {
        path: PathBuf,
        source: serde_json::Error,
    },

    #[snafu(display("could not encode the settings"))]
    EncodeSettings { source: serde_json::Error },

    #[snafu(display("could not save the settings file {}", path.display()))]
    SaveSettings {
        path: PathBuf,
        source: isthmus_core::Error,
    },

    #[snafu(display("could not encode the output"))]
    EncodeOutput { source: serde_json::Error },

    #[snafu(display("could not start Isthmus"))]
    Start { source: isthmus_input::Error },

    #[snafu(display("could not apply the preset"))]
    ApplyPreset { source: isthmus_input::Error },

    #[snafu(display("could not put back the changes a previous run left"))]
    Restore { source: isthmus_input::Error },

    #[snafu(display(
        "{failed} changes could not be put back; they stay in the journal for the next try"
    ))]
    RestoreIncomplete { failed: u32 },

    #[snafu(display("could not watch input events"))]
    Inspect { source: isthmus_input::Error },

    #[snafu(display("could not install the Ctrl-C, SIGTERM and SIGHUP handlers"))]
    Signals { source: std::io::Error },

    #[snafu(display("`isthmus run` must start on the main thread"))]
    NotMainThread,
}

/// `error` and every cause below it, joined with `: `, as `main` prints it.
pub(crate) fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
