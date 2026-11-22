//! The update check's one question for Rust: is this release manifest
//! signed by the release key?
//!
//! The release workflow signs `release-manifest.json` with Ed25519 through
//! OpenSSL (`scripts/release/ui_release_manifest.py`) and the app installs
//! nothing the manifest does not list, so this check is what an update's
//! trust rests on. It is a pure function of three byte strings, which reach
//! it as base64 in the `update_verify` op, and it needs no service: it
//! answers before `start` and after a quit alike.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, VerifyingKey};

use crate::error::CtlError;

/// The bytes of a base64 argument; anything else is `INVALID_ARGUMENT`.
pub(crate) fn decode(name: &str, text: &str) -> Result<Vec<u8>, CtlError> {
    STANDARD
        .decode(text.trim())
        .map_err(|error| CtlError::invalid_argument(format!("`{name}` is not base64: {error}")))
}

/// A public key: 32 bytes that decode to a point on the curve.
pub(crate) fn public_key(bytes: &[u8]) -> Result<VerifyingKey, CtlError> {
    let raw = <[u8; 32]>::try_from(bytes)
        .map_err(|_| CtlError::invalid_argument("`publicKey` is not 32 bytes"))?;
    VerifyingKey::from_bytes(&raw)
        .map_err(|_| CtlError::invalid_argument("`publicKey` is not an Ed25519 key"))
}

/// Whether `signature` is `key`'s over exactly `message`. Strict
/// verification, so a signature cannot be re-encoded into a second valid
/// one; a signature of the wrong length is simply not valid.
pub(crate) fn verify(key: &VerifyingKey, message: &[u8], signature: &[u8]) -> bool {
    let Ok(raw) = <[u8; 64]>::try_from(signature) else {
        return false;
    };
    key.verify_strict(message, &Signature::from_bytes(&raw))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made with OpenSSL exactly as the release workflow signs
    /// (`openssl pkeyutl -sign -rawin`), with a throwaway key: the manifest
    /// `{"version":"0.1.0","assets":[]}` and a newline.
    const KEY: &str = "LxkcyLr5tYLTchPoLCqIE1Jl3lylRM3/SAZ6wZ+6no8=";
    const MESSAGE: &str = "eyJ2ZXJzaW9uIjoiMC4xLjAiLCJhc3NldHMiOltdfQo=";
    const SIGNATURE: &str =
        "8vQ6kYlaBYhrJTFbeSHGXaaTZctQLJpP2qc45kzdGcPqCSgTi+cyxillCpBek3jEiil7IC6pncii43sIa+XXBg==";

    /// OpenSSL's signature checks out; one changed byte of the manifest, a
    /// different key, or a truncated signature does not.
    #[test]
    fn an_openssl_signature_verifies_and_any_change_breaks_it() {
        let key = public_key(&decode("publicKey", KEY).unwrap()).unwrap();
        let message = decode("message", MESSAGE).unwrap();
        let signature = decode("signature", SIGNATURE).unwrap();
        assert!(verify(&key, &message, &signature));

        let mut tampered = message.clone();
        tampered[12] ^= 1;
        assert!(!verify(&key, &tampered, &signature));
        assert!(!verify(&key, &message, &signature[..63]));

        let release_key = public_key(
            &decode("publicKey", "hdesiJwz0+lsW43ArRyqqYuNubEAPuLw5pa1ZmTzLGU=").unwrap(),
        )
        .unwrap();
        assert!(!verify(&release_key, &message, &signature));
    }

    /// A key that is not 32 bytes, or text that is not base64, is refused
    /// before anything is checked.
    #[test]
    fn malformed_keys_and_text_are_invalid_arguments() {
        assert!(public_key(&[0; 31]).is_err());
        assert!(decode("message", "not base64!").is_err());
    }
}
