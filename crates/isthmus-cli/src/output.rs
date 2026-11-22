//! Printing for every command: `--json` is the serde form of the library's
//! own types (camelCase, as the app sees them), pretty-printed, so scripts
//! and the app read the same shapes.

use serde::Serialize;
use snafu::ResultExt;

use crate::error::{CliError, EncodeOutputSnafu};

/// Prints `value` as pretty JSON on stdout.
pub(crate) fn print_json(value: &impl Serialize) -> Result<(), CliError> {
    let text = serde_json::to_string_pretty(value).context(EncodeOutputSnafu)?;
    println!("{text}");
    Ok(())
}

/// "on" or "off", for the human-readable summaries.
pub(crate) fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}
