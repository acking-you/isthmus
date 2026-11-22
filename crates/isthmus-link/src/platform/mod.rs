//! Operating-system access: the computer's name, the Downloads folder,
//! showing a file in the file manager and pinning a socket to an
//! interface. Only `macos.rs` and `windows.rs` name an OS API; any other
//! system gets `other.rs`, which does the least that still works.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", windows)))]
mod other;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(not(any(target_os = "macos", windows)))]
use other as imp;
#[cfg(windows)]
use windows as imp;

use std::path::{Path, PathBuf};

use crate::path::LocalInterface;
use crate::store::PeerOs;

/// The system this build runs on, as other devices are told.
pub(crate) fn this_os() -> PeerOs {
    imp::OS
}

/// The name the user gave this computer, if the system says.
pub(crate) fn computer_name() -> Option<String> {
    imp::computer_name().filter(|name| !name.trim().is_empty())
}

/// The user's Downloads folder, if the system says where it is.
pub(crate) fn downloads_dir() -> Option<PathBuf> {
    imp::downloads_dir()
}

/// Shows `path` selected in the file manager.
pub(crate) fn reveal(path: &Path) -> std::io::Result<()> {
    imp::reveal(path)
}

/// Opens the folder `path` in the file manager.
pub(crate) fn open_folder(path: &Path) -> std::io::Result<()> {
    imp::open_folder(path)
}

/// Makes `socket` leave through `interface`, so a self-assigned address
/// (which every cable has in the same 169.254/16) goes down the cable it
/// was seen on rather than whichever the routing table names first.
pub(crate) fn bind_to_interface(
    socket: &socket2::Socket,
    interface: &LocalInterface,
    ipv6: bool,
) -> std::io::Result<()> {
    imp::bind_to_interface(socket, interface, ipv6)
}
