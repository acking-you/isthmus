//! The link's state files in the data directory: `link.json` (what the user
//! set) and `peers.json` (who is paired).
//!
//! Both follow the settings rules every Isthmus file keeps: fields are added
//! with defaults, unknown keys survive a rewrite (a `#[serde(flatten)]` map
//! on every struct), an unknown enum value reads as the variant that changes
//! nothing, a file that cannot be read is an error and never quietly the
//! defaults, and every write replaces the file atomically.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use snafu::ResultExt;

use crate::error::{Error, ParseStateSnafu, WriteStateSnafu};
use crate::identity::{DeviceId, PublicKey};

/// What the user set, in the data directory.
pub const SETTINGS_FILE: &str = "link.json";
/// The paired devices, in the data directory.
pub const PEERS_FILE: &str = "peers.json";
/// This device's key, in the data directory.
pub const IDENTITY_FILE: &str = "link-identity.json";
/// The lock that keeps a second process from announcing the same device.
pub const LOCK_FILE: &str = "link.lock";

/// How many addresses a peer keeps for reconnecting before mDNS answers.
pub const REMEMBERED_ADDRESSES: usize = 8;

/// The longest device name, in characters.
pub const MAX_NAME_CHARS: usize = 63;

/// The user's choices for file sharing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkSettings {
    /// The name other devices show for this one; the computer's own name
    /// when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Where received files go; the Downloads folder when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receive_dir: Option<PathBuf>,
    /// Whether to share the clipboard with paired devices; on when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<bool>,
    /// Keys a later version wrote, kept on rewrite.
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

impl LinkSettings {
    /// Whether the clipboard is shared with paired devices: on unless
    /// turned off.
    pub fn shares_clipboard(&self) -> bool {
        self.clipboard.unwrap_or(true)
    }
}

/// The operating system a device runs, as it says in its announcement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PeerOs {
    Macos,
    Windows,
    Linux,
    /// Anything this version does not know.
    #[default]
    #[serde(other)]
    Other,
}

/// Every paired device.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PeersFile {
    #[serde(default)]
    pub peers: Vec<PairedPeer>,
    /// Keys a later version wrote, kept on rewrite.
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

/// A device paired with this one: its pinned key and how to find it again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairedPeer {
    pub id: DeviceId,
    /// The key every connection from or to it must prove.
    pub public_key: PublicKey,
    /// Its name when last heard from.
    pub name: String,
    #[serde(default)]
    pub os: PeerOs,
    /// When pairing finished, in seconds since 1970.
    #[serde(default)]
    pub paired_at: u64,
    /// Where it last listened, most recent first, as `ip:port` text so a
    /// form this version cannot parse is skipped rather than failing the
    /// file. Tried before mDNS answers, so a peer reconnects at once.
    #[serde(default)]
    pub addresses: Vec<String>,
    /// Keys a later version wrote, kept on rewrite.
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

impl PairedPeer {
    /// The remembered addresses this version can use.
    pub fn remembered(&self) -> impl Iterator<Item = SocketAddr> + '_ {
        self.addresses.iter().filter_map(|text| text.parse().ok())
    }

    /// Puts `addr` first, dropping a repeat and the oldest past the limit.
    /// Only IPv4 is kept: an IPv6 link-local address needs a scope that
    /// means nothing after a restart, and mDNS finds IPv6 anyway. Returns
    /// whether anything changed.
    pub fn remember(&mut self, addr: SocketAddr) -> bool {
        if !addr.is_ipv4() {
            return false;
        }
        let text = addr.to_string();
        if self.addresses.first() == Some(&text) {
            return false;
        }
        self.addresses.retain(|known| known != &text);
        self.addresses.insert(0, text);
        self.addresses.truncate(REMEMBERED_ADDRESSES);
        true
    }
}

/// A device name as the user typed it: trimmed, one line, not empty, at most
/// [`MAX_NAME_CHARS`] characters.
pub fn valid_name(name: &str) -> Option<String> {
    let name = name.trim();
    let fits = !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(char::is_control);
    fits.then(|| name.to_owned())
}

/// The JSON file at `path`, or the default when it does not exist yet.
pub fn read<T: DeserializeOwned + Default>(path: &Path) -> Result<T, Error> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).context(ParseStateSnafu { path }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(source) => Err(Error::ReadState {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Replaces the JSON file at `path` with `value`, atomically.
pub fn write<T: Serialize>(path: &Path, value: &T) -> Result<(), Error> {
    let bytes = serde_json::to_vec_pretty(value).context(ParseStateSnafu { path })?;
    isthmus_core::durable_file::replace_contents(path, &bytes).context(WriteStateSnafu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScratchDir;

    fn peer_json() -> Value {
        serde_json::json!({
            "id": "0123456789abcdef",
            "publicKey": "00".repeat(32),
            "name": "Desk PC",
            "os": "haiku",
            "addresses": ["169.254.1.2:47821", "not an address"],
            "trusted": "a later field",
        })
    }

    /// A later version's keys survive a rewrite, an unknown OS reads as
    /// `Other`, and an address this version cannot parse is skipped, not a
    /// failure of the whole file.
    #[test]
    fn peers_keep_unknown_keys_and_skip_what_they_cannot_read() {
        let file: PeersFile =
            serde_json::from_value(serde_json::json!({"peers": [peer_json()], "v": 2})).unwrap();
        let peer = &file.peers[0];
        assert_eq!(peer.os, PeerOs::Other);
        assert_eq!(
            peer.remembered().collect::<Vec<_>>(),
            vec!["169.254.1.2:47821".parse::<SocketAddr>().unwrap()]
        );
        let written = serde_json::to_value(&file).unwrap();
        assert_eq!(written["v"], 2);
        assert_eq!(written["peers"][0]["trusted"], "a later field");
    }

    /// Remembering an address puts it first, once, keeps at most the limit
    /// and ignores IPv6, whose scope would not survive a restart.
    #[test]
    fn remembered_addresses_are_most_recent_first_and_bounded() {
        let mut peer: PairedPeer = serde_json::from_value(peer_json()).unwrap();
        assert!(peer.remember("192.168.1.9:47821".parse().unwrap()));
        assert!(!peer.remember("192.168.1.9:47821".parse().unwrap()));
        assert!(!peer.remember("[fe80::1]:47821".parse().unwrap()));
        assert_eq!(peer.addresses[0], "192.168.1.9:47821");
        for last in 0..20 {
            peer.remember(SocketAddr::from(([10, 0, 0, last], 1)));
        }
        assert_eq!(peer.addresses.len(), REMEMBERED_ADDRESSES);
        assert_eq!(peer.addresses[0], "10.0.0.19:1");
    }

    /// A missing file is the defaults; a damaged one is an error that
    /// leaves it in place.
    #[test]
    fn a_missing_file_is_the_default_and_a_damaged_one_an_error() {
        let scratch = ScratchDir::new("store");
        let path = scratch.path().join(SETTINGS_FILE);
        assert_eq!(
            read::<LinkSettings>(&path).unwrap(),
            LinkSettings::default()
        );
        std::fs::write(&path, b"{").unwrap();
        assert!(matches!(
            read::<LinkSettings>(&path),
            Err(Error::ParseState { .. })
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"{");
    }

    /// Names are trimmed single lines of a bounded length.
    #[test]
    fn device_names_are_one_trimmed_line() {
        assert_eq!(valid_name("  Desk PC ").as_deref(), Some("Desk PC"));
        assert!(valid_name("   ").is_none());
        assert!(valid_name("a\nb").is_none());
        assert!(valid_name(&"x".repeat(MAX_NAME_CHARS + 1)).is_none());
        assert!(valid_name("书房的电脑").is_some());
    }
}
