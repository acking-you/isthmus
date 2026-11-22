//! Windows: the environment for the computer name and the profile folder,
//! Explorer for showing files, and a source address to pick the interface
//! (Windows sends from the interface that owns the bound address).

use std::net::{IpAddr, SocketAddr};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::path::LocalInterface;
use crate::store::PeerOs;

pub(super) const OS: PeerOs = PeerOs::Windows;

/// `CREATE_NO_WINDOW`: Explorer is a GUI process, but the flag keeps any
/// console from flashing up.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(super) fn computer_name() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
}

/// `%USERPROFILE%\Downloads`. A user who moved the folder elsewhere gets
/// the default location; the receive folder can be set in `link.json`.
pub(super) fn downloads_dir() -> Option<PathBuf> {
    let profile = std::env::var_os("USERPROFILE").filter(|home| !home.is_empty())?;
    Some(PathBuf::from(profile).join("Downloads"))
}

pub(super) fn reveal(path: &Path) -> std::io::Result<()> {
    // Explorer parses its own command line: `/select,"path"` must reach it
    // as one argument, unquoted by Rust's own rules.
    Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .spawn()
        .map(drop)
}

pub(super) fn open_folder(path: &Path) -> std::io::Result<()> {
    Command::new("explorer.exe")
        .arg(path)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .spawn()
        .map(drop)
}

pub(super) fn bind_to_interface(
    socket: &socket2::Socket,
    interface: &LocalInterface,
    ipv6: bool,
) -> std::io::Result<()> {
    let local = if ipv6 {
        interface
            .v6
            .first()
            .map(|(ip, _)| SocketAddr::new(IpAddr::V6(*ip), 0))
    } else {
        interface
            .first_v4()
            .map(|ip| SocketAddr::new(IpAddr::V4(ip), 0))
    };
    match local {
        Some(local) => socket.bind(&local.into()),
        None => Ok(()),
    }
}
