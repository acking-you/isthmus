//! This device's long-term key and the id derived from it.
//!
//! The key is the Noise static key every connection proves. The id is the
//! first eight bytes of the key's SHA-256, so a device cannot answer to
//! another's id without that device's private key, and pairing pins the
//! whole key, not the id. The private key lives in `link-identity.json` in
//! the data directory, readable by the user alone; it never leaves the
//! device.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use snafu::ResultExt;

use crate::error::{Error, KeySnafu, ParseStateSnafu, WriteStateSnafu};
use crate::hex;
use crate::wire::NOISE_PARAMS;

/// The length of an X25519 key.
pub const KEY_LEN: usize = 32;

/// How many bytes of the key's hash make an id: 64 bits keeps ids apart on
/// any network, and an id is only a name; the pinned key is the trust.
const ID_BYTES: usize = 8;

/// A device's public key, hex in JSON.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey([u8; KEY_LEN]);

impl PublicKey {
    /// The key from raw bytes, which must be exactly [`KEY_LEN`] long.
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        bytes.try_into().ok().map(Self)
    }

    /// The raw key.
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", hex::encode(&self.0))
    }
}

impl Serialize for PublicKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let mut key = [0; KEY_LEN];
        if hex::decode_into(&text, &mut key) {
            Ok(Self(key))
        } else {
            Err(serde::de::Error::custom("a public key is 64 hex digits"))
        }
    }
}

/// A device id: 16 lowercase hex digits, derived from the device's key.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DeviceId(String);

impl DeviceId {
    /// The id of the device holding `key`.
    pub fn of(key: &PublicKey) -> Self {
        let digest = Sha256::digest(key.as_bytes());
        Self(hex::encode(&digest[..ID_BYTES]))
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for DeviceId {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let valid = text.len() == ID_BYTES * 2
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if valid {
            Ok(Self(text))
        } else {
            Err(format!("`{text}` is not a device id"))
        }
    }
}

impl From<DeviceId> for String {
    fn from(id: DeviceId) -> Self {
        id.0
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceId({})", self.0)
    }
}

/// This device's key pair and id.
pub struct Identity {
    private: [u8; KEY_LEN],
    public: PublicKey,
    id: DeviceId,
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The private key never reaches a log.
        f.debug_struct("Identity")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// The identity file's contents.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdentityFile {
    private_key: String,
    public_key: PublicKey,
}

impl Identity {
    /// The identity in `path`, or a new one written there when the file does
    /// not exist. A file that exists but cannot be read is an error, never
    /// a silent new identity: that would unpair every peer.
    pub fn load_or_create(path: &Path) -> Result<Self, Error> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let file: IdentityFile =
                    serde_json::from_slice(&bytes).context(ParseStateSnafu { path })?;
                let mut private = [0; KEY_LEN];
                if !hex::decode_into(&file.private_key, &mut private) {
                    let source = serde::de::Error::custom("the private key is not 64 hex digits");
                    return Err(Error::ParseState {
                        path: path.to_path_buf(),
                        source,
                    });
                }
                Ok(Self::from_parts(private, file.public_key))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let identity = Self::generate()?;
                let file = IdentityFile {
                    private_key: hex::encode(&identity.private),
                    public_key: identity.public,
                };
                let bytes = serde_json::to_vec_pretty(&file).context(ParseStateSnafu { path })?;
                isthmus_core::durable_file::replace_contents(path, &bytes)
                    .context(WriteStateSnafu)?;
                Ok(identity)
            }
            Err(source) => Err(Error::ReadState {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// A fresh key pair from the system's random source.
    pub fn generate() -> Result<Self, Error> {
        let params = NOISE_PARAMS.parse().context(KeySnafu)?;
        let keypair = snow::Builder::new(params)
            .generate_keypair()
            .context(KeySnafu)?;
        let (Ok(private), Some(public)) = (
            <[u8; KEY_LEN]>::try_from(keypair.private.as_slice()),
            PublicKey::from_slice(&keypair.public),
        ) else {
            return Err(Error::Key {
                source: snow::Error::Input,
            });
        };
        Ok(Self::from_parts(private, public))
    }

    fn from_parts(private: [u8; KEY_LEN], public: PublicKey) -> Self {
        Self {
            private,
            id: DeviceId::of(&public),
            public,
        }
    }

    /// The private key, for the Noise handshake alone.
    pub(crate) fn private_key(&self) -> &[u8; KEY_LEN] {
        &self.private
    }

    /// The public key peers pin.
    pub fn public_key(&self) -> &PublicKey {
        &self.public
    }

    /// This device's id.
    pub fn id(&self) -> &DeviceId {
        &self.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScratchDir;

    /// The identity is made once and read back the same on every start:
    /// a new key would make every paired peer refuse this device.
    #[test]
    fn the_identity_is_created_once_and_then_read_back() {
        let scratch = ScratchDir::new("identity");
        let path = scratch.path().join("link-identity.json");
        let first = Identity::load_or_create(&path).unwrap();
        let second = Identity::load_or_create(&path).unwrap();
        assert_eq!(first.id(), second.id());
        assert_eq!(first.public_key(), second.public_key());
        assert_eq!(first.private_key(), second.private_key());
        assert_eq!(first.id(), &DeviceId::of(first.public_key()));
    }

    /// A damaged identity file is an error, not a quiet new identity.
    #[test]
    fn a_damaged_identity_file_is_an_error() {
        let scratch = ScratchDir::new("identity-damaged");
        let path = scratch.path().join("link-identity.json");
        std::fs::write(&path, b"{\"privateKey\": \"xyz\"}").unwrap();
        assert!(matches!(
            Identity::load_or_create(&path),
            Err(Error::ParseState { .. })
        ));
    }

    /// Ids are 16 lowercase hex digits; anything else is refused when read.
    #[test]
    fn device_ids_are_validated_when_read() {
        assert!(DeviceId::try_from("0123456789abcdef".to_owned()).is_ok());
        assert!(DeviceId::try_from("0123456789ABCDEF".to_owned()).is_err());
        assert!(DeviceId::try_from("0123".to_owned()).is_err());
        assert!(serde_json::from_str::<DeviceId>("\"../../etc\"").is_err());
    }
}
