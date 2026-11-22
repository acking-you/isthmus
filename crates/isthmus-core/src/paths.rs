//! Where Isthmus keeps its files.
//!
//! One directory holds everything (settings, the journal, the instance lock),
//! so a single override moves all of it: `ISTHMUS_DATA_DIR` lets a test run
//! or a second copy keep its state apart without touching the real one. The
//! environment is read once, here, and never written.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use snafu::{OptionExt, ResultExt};

use crate::error::{CreateDirSnafu, Error, NoHomeSnafu};

/// The directory name under Application Support. Shared by every build
/// (release and `-local`), so the instance lock keeps them apart.
pub const APP_DIR_NAME: &str = "Isthmus";

/// The variable that, set and non-empty, replaces the data directory.
pub const DATA_DIR_ENV: &str = "ISTHMUS_DATA_DIR";

/// `~/Library/Application Support/Isthmus`, or `$ISTHMUS_DATA_DIR` when that
/// is set and non-empty; created if missing. Fails with [`Error::NoHome`]
/// when neither says where it is.
pub fn data_dir() -> Result<PathBuf, Error> {
    let dir = resolve_data_dir(std::env::var_os(DATA_DIR_ENV), std::env::var_os("HOME"))?;
    ensure_dir(&dir)?;
    Ok(dir)
}

/// The data directory from the two variables, without touching the disk or
/// the environment, so the choice is testable on its own. An empty value
/// counts as unset: an empty `HOME` would otherwise put the files in a
/// relative path under whatever directory the process runs in.
fn resolve_data_dir(
    override_dir: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, Error> {
    if let Some(dir) = override_dir.filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = home.filter(|home| !home.is_empty()).context(NoHomeSnafu)?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join(APP_DIR_NAME))
}

/// Creates `dir` and its parents if they are missing; an existing directory
/// is fine.
fn ensure_dir(dir: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(dir).context(CreateDirSnafu { path: dir })
}

/// The settings file in `dir`.
pub fn settings_file(dir: &std::path::Path) -> PathBuf {
    dir.join("settings.json")
}

/// The journal of system changes in `dir`.
pub fn journal_file(dir: &std::path::Path) -> PathBuf {
    dir.join("journal.json")
}

/// The single-instance lock in `dir`.
pub fn lock_file(dir: &std::path::Path) -> PathBuf {
    dir.join("instance.lock")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScratchDir;

    /// Without an override the files live in the user's Application Support,
    /// under the one name every build shares.
    #[test]
    fn the_data_dir_is_under_application_support_by_default() {
        let dir = resolve_data_dir(None, Some("/Users/someone".into())).unwrap();
        assert_eq!(
            dir,
            Path::new("/Users/someone/Library/Application Support/Isthmus")
        );
    }

    /// A set override wins over `HOME`, and needs no `HOME` at all.
    #[test]
    fn a_non_empty_override_replaces_the_data_dir() {
        let dir = resolve_data_dir(Some("/tmp/isthmus-test".into()), None).unwrap();
        assert_eq!(dir, Path::new("/tmp/isthmus-test"));
    }

    /// An empty override is treated as unset, and an unknown or empty home is
    /// an error rather than a path relative to wherever the process runs.
    #[test]
    fn an_empty_override_falls_back_and_a_missing_home_is_no_home() {
        let dir = resolve_data_dir(Some("".into()), Some("/h".into())).unwrap();
        assert_eq!(dir, Path::new("/h/Library/Application Support/Isthmus"));
        assert!(matches!(resolve_data_dir(None, None), Err(Error::NoHome)));
        assert!(matches!(
            resolve_data_dir(Some("".into()), Some("".into())),
            Err(Error::NoHome)
        ));
    }

    /// The directory is created with its parents, and creating it again is
    /// not an error.
    #[test]
    fn ensure_dir_creates_missing_parents_and_accepts_an_existing_dir() {
        let scratch = ScratchDir::new("paths");
        let dir = scratch.path().join("a").join("b");
        ensure_dir(&dir).unwrap();
        assert!(dir.is_dir());
        ensure_dir(&dir).unwrap();
    }

    /// A file where the directory should be is reported as a create failure
    /// naming that path.
    #[test]
    fn ensure_dir_reports_the_path_it_could_not_create() {
        let scratch = ScratchDir::new("paths-blocked");
        let blocker = scratch.path().join("file");
        std::fs::write(&blocker, b"x").unwrap();
        let dir = blocker.join("below");
        match ensure_dir(&dir) {
            Err(Error::CreateDir { path, .. }) => assert_eq!(path, dir),
            other => panic!("expected CreateDir, got {other:?}"),
        }
    }
}
