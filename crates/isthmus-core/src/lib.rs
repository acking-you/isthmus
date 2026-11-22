//! Foundations every Isthmus feature shares: where its files live, how a
//! state file is replaced without ever being half written, the lock that
//! lets one process own the machine-wide input state, and the boot session
//! that tells a journal from a previous boot.

pub mod boot;
pub mod durable_file;
pub mod error;
pub mod instance;
pub mod paths;

#[cfg(test)]
mod test_support;

pub use error::Error;
