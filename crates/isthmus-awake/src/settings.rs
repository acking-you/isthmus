//! `awake.json`: the user's one choice, read by every later version.
//!
//! Like every Isthmus settings file, fields are added with defaults and keys
//! this version does not know survive a rewrite (the `unknown` map); a file
//! that cannot be read is an error, never silently the default.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use snafu::ResultExt;

use crate::error::{Error, ParseSettingsSnafu, ReadSettingsSnafu, WriteSettingsSnafu};

/// The file in the data directory.
pub const SETTINGS_FILE: &str = "awake.json";

/// What the user chose.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwakeSettings {
    /// Keep the computer awake while the app runs.
    #[serde(default)]
    pub enabled: bool,
    /// Keys a later version wrote.
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

/// The settings at `path`, or the defaults when there is no file yet.
pub(crate) fn load(path: &Path) -> Result<AwakeSettings, Error> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AwakeSettings::default());
        }
        Err(source) => return Err(source).context(ReadSettingsSnafu { path }),
    };
    serde_json::from_slice(&bytes).context(ParseSettingsSnafu { path })
}

/// Replaces `path` with `settings`, atomically.
pub(crate) fn save(path: &Path, settings: &AwakeSettings) -> Result<(), Error> {
    let mut bytes = serde_json::to_vec_pretty(settings).unwrap_or_default();
    bytes.push(b'\n');
    if let Some(dir) = path.parent() {
        // The data directory exists once anything ran; a fresh one is made.
        let _ = std::fs::create_dir_all(dir);
    }
    isthmus_core::durable_file::replace_contents(path, &bytes).context(WriteSettingsSnafu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScratchDir;

    /// A later version's keys survive a rewrite, and a broken file is an
    /// error rather than the defaults.
    #[test]
    fn unknown_keys_survive_and_a_broken_file_is_an_error() {
        let scratch = ScratchDir::new("settings");
        let path = scratch.path().join(SETTINGS_FILE);
        std::fs::write(
            &path,
            r#"{"enabled": true, "schedule": {"until": "18:00"}}"#,
        )
        .unwrap();
        let mut settings = load(&path).unwrap();
        assert!(settings.enabled);
        settings.enabled = false;
        save(&path, &settings).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"until\""), "{text}");
        assert!(!load(&path).unwrap().enabled);

        std::fs::write(&path, "{not json").unwrap();
        assert!(matches!(load(&path), Err(Error::ParseSettings { .. })));
    }
}
