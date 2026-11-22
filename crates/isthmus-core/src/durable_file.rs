//! Replacing a state file so it is never half written: a temporary file in
//! the same directory, written and synced, then renamed over the old one.
//!
//! A rename within one directory is atomic, so a reader (or the next start
//! after a crash or a power cut) sees the old file or the new one, never a
//! torn mix. Syncing the file before the rename makes the new contents
//! durable before they become visible; syncing the directory after it makes
//! the rename itself durable. The journal depends on this: a journal entry
//! must be on disk before the HID property it describes is written.

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::Error;

/// The mode every state file gets: settings and the journal describe the
/// user's input devices and belong to the user alone.
#[cfg(unix)]
const STATE_FILE_MODE: u32 = 0o600;

/// How many temporary names to try before giving up. A name is taken only if
/// another writer in this process picked the same nanosecond, so one retry
/// with a fresh clock reading practically always succeeds.
const TEMPORARY_NAME_ATTEMPTS: u32 = 8;

/// Replaces `path`'s contents with `bytes` atomically, file mode 0600.
///
/// The bytes go to `.{file name}.tmp-{pid}-{nanos}` beside `path`, created
/// new (never an existing file), synced, renamed over `path`, and then the
/// directory is synced. On failure the temporary file is removed and `path`
/// keeps its old contents, unless only the final directory sync failed, by
/// which time the new contents are in place but may not survive a power cut.
/// The directory must exist. Every failure is [`Error::Write`] naming `path`.
pub fn replace_contents(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let failed = |source| Error::Write {
        path: path.to_path_buf(),
        source,
    };
    let Some(name) = path.file_name() else {
        return Err(failed(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the path names no file",
        )));
    };
    let (temporary, file) = create_temporary(path, name).map_err(failed)?;
    let result = write_and_replace(file, &temporary, path, bytes);
    if result.is_err() {
        // Best effort: after a successful rename this names nothing, and no
        // later call would clean it up either way.
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(failed)
}

/// Creates the temporary file beside `path`, retrying with a fresh name when
/// one is already taken.
fn create_temporary(path: &Path, name: &OsStr) -> std::io::Result<(PathBuf, File)> {
    let pid = std::process::id();
    let mut taken = None;
    for attempt in 0..TEMPORARY_NAME_ATTEMPTS {
        // A clock before 1970 only makes the name less unique; `create_new`
        // still refuses to reuse a file.
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos())
            + u128::from(attempt);
        let temporary =
            path.with_file_name(format!(".{}.tmp-{pid}-{nanos}", name.to_string_lossy()));
        match open_new(&temporary) {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                taken = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    Err(taken.unwrap_or_else(|| std::io::Error::from(std::io::ErrorKind::AlreadyExists)))
}

/// Opens a file that must not exist yet, created with the state-file mode so
/// it is never readable by others, not even before its first byte.
fn open_new(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(STATE_FILE_MODE);
    }
    options.open(path)
}

/// Writes and syncs the temporary file, renames it over `path`, and syncs
/// the directory so the rename survives a crash.
fn write_and_replace(
    mut file: File,
    temporary: &Path,
    path: &Path,
    bytes: &[u8],
) -> std::io::Result<()> {
    set_state_file_mode(&file)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, path)?;
    sync_parent_directory(path)
}

/// Sets the mode exactly: the mode given at creation is reduced by the
/// umask, which must not decide who can read the journal.
#[cfg(unix)]
fn set_state_file_mode(file: &File) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(STATE_FILE_MODE))
}

#[cfg(not(unix))]
fn set_state_file_mode(_: &File) -> std::io::Result<()> {
    Ok(())
}

/// Fsyncs the directory holding `path`. A bare file name's parent is the
/// empty path, which means the current directory.
#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    File::open(parent)?.sync_all()
}

/// Nothing to do: Windows cannot open a directory as a file (that takes
/// `FILE_FLAG_BACKUP_SEMANTICS`, and flushing it write access the user may
/// not have), and NTFS journals the rename's metadata itself. Opening it
/// anyway failed every state-file write after the file was already in
/// place, which kept file sharing from starting on Windows.
#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;
    use crate::test_support::ScratchDir;

    /// The first write creates the file, the second replaces it, and no
    /// temporary file is left beside it.
    #[test]
    fn replace_creates_then_replaces_and_leaves_only_the_target() {
        let scratch = ScratchDir::new("replace");
        let path = scratch.path().join("journal.json");

        replace_contents(&path, b"first").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        replace_contents(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        assert_eq!(scratch.entries(), ["journal.json"]);
    }

    /// The old file is never written in place: a reader that opened it before
    /// the replace still reads the whole old contents, and a reader opening
    /// the path afterwards reads the whole new contents.
    #[test]
    fn a_reader_of_the_old_file_keeps_reading_the_old_contents() {
        let scratch = ScratchDir::new("replace-reader");
        let path = scratch.path().join("settings.json");
        replace_contents(&path, b"old contents").unwrap();
        let mut old_reader = File::open(&path).unwrap();

        replace_contents(&path, b"new").unwrap();

        let mut old = String::new();
        old_reader.read_to_string(&mut old).unwrap();
        assert_eq!(old, "old contents");
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }

    /// A replace that cannot rename keeps what was at the path, removes its
    /// temporary file and names the path it was replacing.
    #[test]
    fn a_failed_replace_keeps_the_old_contents_and_removes_the_temporary() {
        let scratch = ScratchDir::new("replace-fails");
        let path = scratch.path().join("occupied");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("kept"), b"kept").unwrap();

        match replace_contents(&path, b"new") {
            Err(Error::Write { path: reported, .. }) => assert_eq!(reported, path),
            other => panic!("expected Write, got {other:?}"),
        }
        assert_eq!(scratch.entries(), ["occupied"]);
        assert_eq!(std::fs::read(path.join("kept")).unwrap(), b"kept");
    }

    /// The directory is the caller's to create: a missing one is a write
    /// failure, not a directory created behind the caller's back.
    #[test]
    fn replacing_in_a_missing_directory_fails_without_creating_it() {
        let scratch = ScratchDir::new("replace-nodir");
        let path = scratch.path().join("missing").join("journal.json");
        assert!(matches!(
            replace_contents(&path, b"x"),
            Err(Error::Write { .. })
        ));
        assert!(!scratch.path().join("missing").exists());
    }

    /// The new file is the user's alone, whatever mode the old file had and
    /// whatever the umask allows.
    #[cfg(unix)]
    #[test]
    fn the_replaced_file_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = ScratchDir::new("replace-mode");
        let path = scratch.path().join("journal.json");
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        replace_contents(&path, b"new").unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
