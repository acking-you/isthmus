//! What the link service reports failing to its callers.
//!
//! One enum for the crate's public operations, each variant naming what the
//! caller acts on (a device id, a path). Failures inside a connection are
//! [`crate::wire::WireError`] and end up as a transfer's or a pairing's
//! failure kind in the status, never as an error of an operation.

use std::path::PathBuf;

use snafu::Snafu;

/// What went wrong, by the step that failed.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    /// A state file (`link.json`, `peers.json`, the identity) could not be
    /// read.
    #[snafu(display("could not read {}", path.display()))]
    ReadState {
        path: PathBuf,
        source: std::io::Error,
    },

    /// A state file is not what this version can read; it is left alone.
    #[snafu(display("{} is not valid", path.display()))]
    ParseState {
        path: PathBuf,
        source: serde_json::Error,
    },

    /// A state file could not be replaced.
    #[snafu(display("could not save the file sharing state"))]
    WriteState { source: isthmus_core::Error },

    /// The identity key could not be made.
    #[snafu(display("could not create this device's key"))]
    Key { source: snow::Error },

    /// The link's own lock file could not be opened.
    #[snafu(display("could not take the file sharing lock"))]
    Lock { source: isthmus_core::Error },

    /// Another process on this computer (another build of the app, say)
    /// already announces this device and listens for it.
    #[snafu(display("another Isthmus on this computer is already sharing files"))]
    AnotherInstance,

    /// The async runtime could not start.
    #[snafu(display("could not start the network runtime"))]
    Runtime { source: std::io::Error },

    /// No port could be listened on.
    #[snafu(display("could not listen for other devices"))]
    Listen { source: std::io::Error },

    /// mDNS could not start.
    #[snafu(display("could not start looking for other devices"))]
    Discovery { source: mdns_sd::Error },

    /// The device id is neither paired nor seen on the network.
    #[snafu(display("no device {id} is known"))]
    UnknownPeer { id: String },

    /// Files go only to paired devices.
    #[snafu(display("{id} is not paired"))]
    NotPaired { id: String },

    /// The device is known but has no address to try.
    #[snafu(display("{id} cannot be reached right now"))]
    PeerOffline { id: String },

    /// One pairing at a time, in either direction.
    #[snafu(display("another pairing is in progress"))]
    PairingBusy,

    /// A code was entered with no pairing waiting for one.
    #[snafu(display("no pairing is waiting for a code"))]
    NoPairing,

    /// A pairing code is six digits.
    #[snafu(display("the code must be six digits"))]
    InvalidCode,

    /// The list of files was empty, or held only things that are skipped.
    #[snafu(display("there is nothing to send"))]
    NothingToSend,

    /// A file or folder to send could not be read.
    #[snafu(display("could not read {}", path.display()))]
    Source {
        path: PathBuf,
        source: std::io::Error,
    },

    /// A send is capped so the offer stays a bounded message.
    #[snafu(display("too many files to send at once (at most {limit})"))]
    TooManyFiles { limit: usize },

    /// No transfer has that id.
    #[snafu(display("no transfer {id}"))]
    UnknownTransfer { id: String },

    /// A device name is one line of text.
    #[snafu(display("the device name must be 1 to {max} characters on one line"))]
    InvalidName { max: usize },

    /// The file manager could not be opened at a path.
    #[snafu(display("could not show {}", path.display()))]
    Reveal {
        path: PathBuf,
        source: std::io::Error,
    },

    /// The service has shut down in this process.
    #[snafu(display("file sharing has shut down"))]
    ShutDown,
}
