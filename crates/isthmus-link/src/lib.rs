//! Isthmus between computers: other Isthmus computers on the local network,
//! paired once with a short code and trusted on sight after that, and files
//! sent to them over the best path there is.
//!
//! The decisions that shape the crate:
//!
//! - **Discovery finds; it never trusts.** mDNS (`_isthmus._tcp`) on every
//!   interface says where a device id answers. Only a key pinned by pairing
//!   makes that device a peer, and every connection must prove that key.
//! - **One handshake for everything.** Every connection is TCP with a Noise
//!   XX handshake (X25519, ChaChaPoly, BLAKE2s). A device's static key is its
//!   identity and its id is a hash of that key. Noise over TLS because it
//!   needs no certificates and its crates are pure Rust, so the Windows build
//!   compiles anywhere.
//! - **Pairing is a PAKE on a six-digit code** shown on the device being
//!   added (SPAKE2, bound to the Noise handshake hash): a man in the middle
//!   gets one guess per attempt and an eavesdropper none.
//! - **Paths are ranked.** A direct cable (an interface with only
//!   self-assigned addresses, such as a USB Ethernet adapter on a KVM's hub
//!   cabled straight to the other computer) comes before the shared network.
//!   Connects race the candidates in that order, staggered, so the cable wins
//!   whenever it works; a session on the network moves to the cable when it
//!   appears, and a transfer that loses its path resumes on the next one.
//! - **Plain data above `platform/`.** The OS shows up only there: the
//!   computer's name, the Downloads folder, revealing a file, binding a
//!   socket to an interface.

pub mod discovery;
pub mod error;
pub mod files;
mod hex;
pub mod identity;
pub mod pairing;
pub mod path;
mod platform;
pub mod service;
pub mod status;
pub mod store;
pub mod wire;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

pub use error::Error;
pub use identity::DeviceId;
pub use service::{ChangeSink, ClipboardSink, LinkOptions, LinkService};
pub use status::LinkStatus;
