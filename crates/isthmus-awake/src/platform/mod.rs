//! The system's power request. Only `macos.rs` and `windows.rs` name an OS
//! API; any other system gets `other.rs`, which says it cannot.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", windows)))]
mod other;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub(crate) use macos::{SUPPORTED, hold};
#[cfg(not(any(target_os = "macos", windows)))]
pub(crate) use other::{SUPPORTED, hold};
#[cfg(windows)]
pub(crate) use windows::{SUPPORTED, hold};
