//! Scratch directories for this crate's tests, removed when a test ends.

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
            std::env::temp_dir().join(format!("isthmus-awake-{label}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        ScratchDir { path }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
