//! macOS: `scutil` for the computer name, `open` for Finder, and
//! `IP_BOUND_IF` to pin a socket to an interface.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::path::LocalInterface;
use crate::store::PeerOs;

pub(super) const OS: PeerOs = PeerOs::Macos;

/// The Sharing pane's Computer Name, which is what the user sees in
/// Finder's sidebar on other Macs too.
pub(super) fn computer_name() -> Option<String> {
    let output = Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub(super) fn downloads_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty())?;
    Some(PathBuf::from(home).join("Downloads"))
}

pub(super) fn reveal(path: &Path) -> std::io::Result<()> {
    Command::new("/usr/bin/open")
        .arg("-R")
        .arg(path)
        .stdin(Stdio::null())
        .spawn()
        .map(drop)
}

pub(super) fn open_folder(path: &Path) -> std::io::Result<()> {
    Command::new("/usr/bin/open")
        .arg(path)
        .stdin(Stdio::null())
        .spawn()
        .map(drop)
}

pub(super) fn bind_to_interface(
    socket: &socket2::Socket,
    interface: &LocalInterface,
    ipv6: bool,
) -> std::io::Result<()> {
    let index = NonZeroU32::new(interface.index);
    if ipv6 {
        socket.bind_device_by_index_v6(index)
    } else {
        socket.bind_device_by_index_v4(index)
    }
}
