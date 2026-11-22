//! One Isthmus per Mac. HID mappings and pointer properties are global to
//! the machine, so two processes would fight over them and over the
//! journal. The lock is an `flock` on a file: the kernel releases it when
//! the process dies, so a crash never leaves a stale lock.
//!
//! The lock belongs to the open file, not to the process, so the file stays
//! open inside [`InstanceLock`] for as long as the lock is held, and a second
//! open of the same file, even in this process, cannot take it.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

use crate::error::Error;

/// The held lock; dropping it releases it.
#[derive(Debug)]
pub struct InstanceLock {
    /// The open lock file. Closing it is what releases the `flock`.
    _file: File,
}

impl InstanceLock {
    /// Takes the lock at `path`, or `Ok(None)` while another process holds
    /// it. The file is created if missing and never truncated or removed:
    /// removing it would let a second process lock a new file while the first
    /// still holds the old one. Any other failure is [`Error::Lock`].
    pub fn try_acquire(path: &Path) -> Result<Option<InstanceLock>, Error> {
        let failed = |source| Error::Lock {
            path: path.to_path_buf(),
            source,
        };
        let file = open_lock_file(path).map_err(failed)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(InstanceLock { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(source)) => Err(failed(source)),
        }
    }
}

/// Opens the lock file for writing (an exclusive `flock` does not need it,
/// but some file systems refuse locks on read-only descriptors), creating it
/// private to the user.
fn open_lock_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScratchDir;

    /// While one lock is held, another attempt on the same file (here from
    /// the same process, which `flock` treats like another one) gets `None`;
    /// once the first is dropped, the next attempt succeeds.
    #[test]
    fn a_second_lock_waits_until_the_first_is_dropped() {
        let scratch = ScratchDir::new("lock");
        let path = scratch.path().join("instance.lock");

        let first = InstanceLock::try_acquire(&path).unwrap();
        assert!(first.is_some());
        assert!(InstanceLock::try_acquire(&path).unwrap().is_none());

        drop(first);
        assert!(InstanceLock::try_acquire(&path).unwrap().is_some());
    }

    /// The lock file is left in place after the lock is released, so every
    /// process always locks the same file.
    #[test]
    fn releasing_the_lock_keeps_the_file() {
        let scratch = ScratchDir::new("lock-file");
        let path = scratch.path().join("instance.lock");
        drop(InstanceLock::try_acquire(&path).unwrap());
        assert!(path.is_file());
    }

    /// A lock file that cannot be opened is an error naming it, not a
    /// silent "another instance".
    #[test]
    fn an_unopenable_lock_file_is_an_error() {
        let scratch = ScratchDir::new("lock-missing");
        let path = scratch.path().join("missing").join("instance.lock");
        match InstanceLock::try_acquire(&path) {
            Err(Error::Lock { path: reported, .. }) => assert_eq!(reported, path),
            other => panic!("expected Lock, got {other:?}"),
        }
    }
}
