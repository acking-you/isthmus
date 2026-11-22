//! The clipboard's system calls. Only `macos.rs` and `windows.rs` name an
//! OS API; any other system gets `other.rs`, which shares nothing.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", windows)))]
mod other;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub(crate) use macos::{SUPPORTED, access, change_count, read, write};
#[cfg(not(any(target_os = "macos", windows)))]
pub(crate) use other::{SUPPORTED, access, change_count, read, write};
#[cfg(windows)]
pub(crate) use windows::{SUPPORTED, access, change_count, read, write};
