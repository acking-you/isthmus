//! `isthmus status`: what is on disk and what macOS says, without starting
//! anything.
//!
//! Each part is read on its own and reported as found, an unreadable file
//! included: a diagnostic that stops at the first broken file hides the
//! rest. Only a missing data directory fails the command, since nothing
//! else can be read without it.
//!
//! Whether an Isthmus is running is read from the instance lock: taking it
//! and dropping it at once. A process starting in that instant would find
//! the lock held and run without changing anything, so the probe is kept as
//! short as it can be; `flock` offers no way to ask without taking it.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use isthmus_core::instance::InstanceLock;
use isthmus_core::{boot, paths};
use isthmus_input::journal::Journal;
use isthmus_input::settings::ProfileStyle;
use serde::Serialize;
use snafu::ResultExt;

use crate::error::{CliError, DataDirSnafu, chain};
use crate::output::{on_off, print_json};
use crate::settings_file;

/// Everything `status` reports; `--json` prints it as is.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusReport {
    version: &'static str,
    data_dir: PathBuf,
    settings_file: PathBuf,
    settings: SettingsReport,
    instance_lock: LockReport,
    journal: JournalReport,
    accessibility: AccessibilityReport,
}

/// The settings file, summarized.
#[derive(Debug, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "file"
)]
enum SettingsReport {
    /// No file yet: the defaults apply.
    Missing(SettingsSummary),
    Present(SettingsSummary),
    Unreadable {
        error: String,
    },
}

/// The master switch and each section, which is what decides whether
/// Isthmus changes anything.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsSummary {
    enabled: bool,
    keyboard: SectionSummary,
    mouse: SectionSummary,
}

/// One section's switch and which preset its values match.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SectionSummary {
    enabled: bool,
    style: ProfileStyle,
}

/// Whether another Isthmus owns the machine-wide input state.
#[derive(Debug, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "state"
)]
enum LockReport {
    /// No Isthmus holds it.
    Free,
    /// An Isthmus (the app, another build or `isthmus run`) is running and
    /// owns the keyboard and mouse.
    Held,
    Unknown {
        error: String,
    },
}

/// What a previous run left to put back, if anything.
#[derive(Debug, PartialEq, Serialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "state"
)]
enum JournalReport {
    /// No journal: nothing to put back.
    None,
    Present {
        entries: usize,
        boot_session: String,
        /// Written during this boot; `None` when this boot's session is not
        /// known. A journal from an earlier boot is already undone.
        same_boot: Option<bool>,
    },
    Unreadable {
        error: String,
    },
}

/// `AXIsProcessTrusted()` for this process, which never prompts.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum AccessibilityReport {
    #[cfg_attr(
        not(target_os = "macos"),
        expect(dead_code, reason = "only macOS can ask")
    )]
    Granted,
    #[cfg_attr(
        not(target_os = "macos"),
        expect(dead_code, reason = "only macOS can ask")
    )]
    NotGranted,
    /// Not macOS.
    #[cfg_attr(
        target_os = "macos",
        expect(dead_code, reason = "only other systems cannot ask")
    )]
    Unknown,
}

/// Prints the status, as text or as JSON.
pub(crate) fn run(json: bool) -> Result<(), CliError> {
    let dir = paths::data_dir().context(DataDirSnafu)?;
    let report = StatusReport {
        version: env!("CARGO_PKG_VERSION"),
        settings_file: paths::settings_file(&dir),
        settings: settings_report(&dir),
        instance_lock: lock_report(&dir),
        journal: journal_report(read_journal(&dir), boot::boot_session_id().as_deref()),
        accessibility: accessibility(),
        data_dir: dir,
    };
    if json {
        print_json(&report)
    } else {
        print!("{}", render(&report));
        Ok(())
    }
}

fn settings_report(dir: &Path) -> SettingsReport {
    match settings_file::load(dir) {
        Ok(file) => {
            let settings = &file.settings;
            let summary = SettingsSummary {
                enabled: settings.enabled,
                keyboard: SectionSummary {
                    enabled: settings.keyboard.enabled,
                    style: settings.keyboard.style(),
                },
                mouse: SectionSummary {
                    enabled: settings.mouse.enabled,
                    style: settings.mouse.style(),
                },
            };
            if file.exists {
                SettingsReport::Present(summary)
            } else {
                SettingsReport::Missing(summary)
            }
        }
        Err(error) => SettingsReport::Unreadable {
            error: chain(&error),
        },
    }
}

fn lock_report(dir: &Path) -> LockReport {
    match InstanceLock::try_acquire(&paths::lock_file(dir)) {
        // Dropped at once: this command must not keep an Isthmus out.
        Ok(Some(lock)) => {
            drop(lock);
            LockReport::Free
        }
        Ok(None) => LockReport::Held,
        Err(error) => LockReport::Unknown {
            error: chain(&error),
        },
    }
}

/// The journal file's text, or why there is none.
enum JournalFile {
    Missing,
    Text(String),
    Unreadable(String),
}

fn read_journal(dir: &Path) -> JournalFile {
    match std::fs::read_to_string(paths::journal_file(dir)) {
        Ok(text) => JournalFile::Text(text),
        Err(error) if error.kind() == ErrorKind::NotFound => JournalFile::Missing,
        Err(error) => JournalFile::Unreadable(error.to_string()),
    }
}

/// What the journal holds, against this boot's session.
fn journal_report(file: JournalFile, current_boot: Option<&str>) -> JournalReport {
    let text = match file {
        JournalFile::Missing => return JournalReport::None,
        JournalFile::Unreadable(error) => return JournalReport::Unreadable { error },
        JournalFile::Text(text) => text,
    };
    match serde_json::from_str::<Journal>(&text) {
        Ok(journal) => JournalReport::Present {
            entries: journal.entries.len(),
            same_boot: current_boot.map(|boot| boot == journal.boot_session),
            boot_session: journal.boot_session,
        },
        Err(error) => JournalReport::Unreadable {
            error: error.to_string(),
        },
    }
}

#[cfg(target_os = "macos")]
fn accessibility() -> AccessibilityReport {
    // SAFETY: takes no arguments and only asks TCC whether this process is
    // trusted; unlike `AXIsProcessTrustedWithOptions` it never prompts.
    if unsafe { objc2_application_services::AXIsProcessTrusted() } {
        AccessibilityReport::Granted
    } else {
        AccessibilityReport::NotGranted
    }
}

#[cfg(not(target_os = "macos"))]
fn accessibility() -> AccessibilityReport {
    AccessibilityReport::Unknown
}

/// The report as aligned `label  value` lines.
fn render(report: &StatusReport) -> String {
    let settings = match &report.settings {
        SettingsReport::Missing(summary) => {
            format!("{} (no file yet: the defaults)", summarize(summary))
        }
        SettingsReport::Present(summary) => summarize(summary),
        SettingsReport::Unreadable { error } => format!("unreadable: {error}"),
    };
    let lock = match &report.instance_lock {
        LockReport::Free => "free: no Isthmus is running".to_owned(),
        LockReport::Held => {
            "held: an Isthmus is running and owns the keyboard and mouse".to_owned()
        }
        LockReport::Unknown { error } => format!("unknown: {error}"),
    };
    let journal = match &report.journal {
        JournalReport::None => "none: nothing to put back".to_owned(),
        JournalReport::Present { entries: 0, .. } => "empty: nothing to put back".to_owned(),
        JournalReport::Present {
            entries,
            same_boot: Some(true),
            ..
        } => format!(
            "{} from this boot: the Isthmus that made them puts them back when it \
             quits; after a crash, its next start or `isthmus restore` does",
            changes(*entries)
        ),
        JournalReport::Present {
            entries,
            same_boot: Some(false),
            ..
        } => format!(
            "{} from an earlier boot: the restart already undid them, and the next \
             start discards the journal",
            changes(*entries)
        ),
        JournalReport::Present {
            entries,
            same_boot: None,
            ..
        } => format!("{} (this boot's session is not known)", changes(*entries)),
        JournalReport::Unreadable { error } => format!("unreadable: {error}"),
    };
    let accessibility = match report.accessibility {
        AccessibilityReport::Granted => "granted",
        AccessibilityReport::NotGranted => "not granted",
        AccessibilityReport::Unknown => "unknown (not macOS)",
    };
    let lines = [
        ("version", report.version.to_owned()),
        ("data directory", report.data_dir.display().to_string()),
        ("settings", settings),
        ("instance lock", lock),
        ("journal", journal),
        (
            "accessibility",
            format!(
                "{accessibility} for this process (run from a terminal, the terminal's \
                 grant is what counts)"
            ),
        ),
    ];
    lines
        .iter()
        .map(|(label, value)| format!("{label:<16}{value}\n"))
        .collect()
}

/// "1 change", "3 changes".
fn changes(count: usize) -> String {
    if count == 1 {
        "1 change".to_owned()
    } else {
        format!("{count} changes")
    }
}

fn summarize(summary: &SettingsSummary) -> String {
    format!(
        "Isthmus {}; keyboard {}; mouse {}",
        on_off(summary.enabled),
        section(&summary.keyboard),
        section(&summary.mouse)
    )
}

/// A section's preset name, or "off" when its switch is.
fn section(summary: &SectionSummary) -> &'static str {
    if !summary.enabled {
        return "off";
    }
    match summary.style {
        ProfileStyle::Windows => "Windows",
        ProfileStyle::Mac => "Mac",
        ProfileStyle::Custom => "custom",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    const JOURNAL: &str = r#"{"version": 1, "bootSession": "boot-a", "entries": [
        {"registryId": 7, "vendorId": 1, "productId": 2, "locationId": 3,
         "key": "UserKeyMapping", "original": {"type": "absent"},
         "applied": {"type": "keyMapping", "value": [[1, 2]]}}]}"#;

    /// A journal of this boot still has changes to put back; one from an
    /// earlier boot was undone by the restart, and the report says which.
    #[test]
    fn a_journal_is_judged_against_this_boot() {
        let this_boot = journal_report(JournalFile::Text(JOURNAL.into()), Some("boot-a"));
        assert_eq!(
            this_boot,
            JournalReport::Present {
                entries: 1,
                boot_session: "boot-a".into(),
                same_boot: Some(true)
            }
        );
        let earlier = journal_report(JournalFile::Text(JOURNAL.into()), Some("boot-b"));
        assert!(matches!(
            earlier,
            JournalReport::Present {
                same_boot: Some(false),
                ..
            }
        ));
        let unknown = journal_report(JournalFile::Text(JOURNAL.into()), None);
        assert!(matches!(
            unknown,
            JournalReport::Present {
                same_boot: None,
                ..
            }
        ));
    }

    /// `--json` uses the app's camelCase everywhere, fields of the tagged
    /// variants included, so a script reads the same names as the app.
    #[test]
    fn json_fields_are_camel_case() {
        let report = journal_report(JournalFile::Text(JOURNAL.into()), Some("boot-a"));
        let json = serde_json::to_value(report).unwrap();
        assert_eq!(json["state"], "present");
        assert_eq!(json["bootSession"], "boot-a");
        assert_eq!(json["sameBoot"], true);
    }

    /// No journal is nothing to do; a broken one is reported, not fatal.
    #[test]
    fn a_missing_or_broken_journal_is_reported_not_fatal() {
        assert_eq!(
            journal_report(JournalFile::Missing, Some("boot-a")),
            JournalReport::None
        );
        assert!(matches!(
            journal_report(JournalFile::Text("[".into()), Some("boot-a")),
            JournalReport::Unreadable { .. }
        ));
    }

    /// A broken settings file shows up in the report instead of stopping
    /// it, so the rest of the status is still visible.
    #[test]
    fn a_broken_settings_file_is_part_of_the_report() {
        let dir = TempDir::new("status-settings");
        assert!(matches!(
            settings_report(dir.path()),
            SettingsReport::Missing(SettingsSummary { enabled: false, .. })
        ));
        std::fs::write(paths::settings_file(dir.path()), "nope").unwrap();
        assert!(matches!(
            settings_report(dir.path()),
            SettingsReport::Unreadable { .. }
        ));
    }
}
