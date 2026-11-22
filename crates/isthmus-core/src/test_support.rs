//! Scratch directories for this crate's tests.
//!
//! The crate takes no `tempfile` dependency (state files go through
//! [`crate::durable_file`]), so tests make their own directories under the
//! system temporary directory, one per test, removed when the test ends.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// A fresh, empty directory that is removed, with everything in it, on drop.
pub(crate) struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    /// Creates a directory whose name carries `label`, the process id and a
    /// per-process counter, so parallel tests never share one.
    pub(crate) fn new(label: &str) -> ScratchDir {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("isthmus-core-{label}-{}-{n}", std::process::id()));
        // A leftover from a killed run with the same pid would make the
        // test read stale files.
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        ScratchDir { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The names directly inside the directory, sorted.
    pub(crate) fn entries(&self) -> Vec<String> {
        let mut names = std::fs::read_dir(&self.path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    }
}

impl Drop for ScratchDir {
    /// Best effort: a directory left behind only costs disk space.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
