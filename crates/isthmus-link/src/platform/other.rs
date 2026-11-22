//! Any other system: enough to build and to run the tests, no more. The
//! name comes from the environment, nothing is shown in a file manager and
//! sockets are not pinned.

use std::path::{Path, PathBuf};

use crate::path::LocalInterface;
use crate::store::PeerOs;

pub(super) const OS: PeerOs = if cfg!(target_os = "linux") {
    PeerOs::Linux
} else {
    PeerOs::Other
};

pub(super) fn computer_name() -> Option<String> {
    std::env::var("HOSTNAME").ok()
}

pub(super) fn downloads_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty())?;
    Some(PathBuf::from(home).join("Downloads"))
}

pub(super) fn reveal(_: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

pub(super) fn open_folder(_: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

pub(super) fn bind_to_interface(
    _: &socket2::Socket,
    _: &LocalInterface,
    _: bool,
) -> std::io::Result<()> {
    Ok(())
}
