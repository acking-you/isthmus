//! `isthmus restore`: puts back what a previous run journalled and did not
//! restore (a crash, a forced quit), without starting anything. A value
//! someone else changed since is left alone, as at a normal start. Refused
//! while a running Isthmus holds the instance lock: that one owns the
//! journal and restores it itself when it quits.

use isthmus_core::paths;
use isthmus_input::platform::{self, RecoveryReport};
use snafu::ResultExt;

use crate::error::{CliError, DataDirSnafu, RestoreIncompleteSnafu, RestoreSnafu};

/// Recovers the journal and prints what happened.
pub(crate) fn run() -> Result<(), CliError> {
    let dir = paths::data_dir().context(DataDirSnafu)?;
    let report = platform::recover_journal(&dir).context(RestoreSnafu)?;
    println!("{}", describe(&report));
    if report.failed > 0 {
        return RestoreIncompleteSnafu {
            failed: report.failed,
        }
        .fail();
    }
    Ok(())
}

fn describe(report: &RecoveryReport) -> String {
    let RecoveryReport {
        restored,
        left,
        failed,
    } = *report;
    if restored == 0 && left == 0 && failed == 0 {
        return "Nothing to put back.".to_owned();
    }
    format!(
        "Put back {restored} changes; left {left} that something else changed since; \
         {failed} could not be written."
    )
}
