//! The cryptography of pairing, in two ways.
//!
//! **By confirmation** (both devices on version 2): one click on the device
//! being added. The answerer commits to a random nonce (a SHA-256 of it),
//! the dialler sends its own nonce, the answerer reveals its nonce, and both
//! devices show six digits derived from the Noise handshake hash and the two
//! nonces; the answerer's user accepts or declines. A man in the middle runs
//! two handshakes, and on each side must fix its nonce (by commitment, or by
//! sending it) before it learns the honest side's, so the two screens show
//! the same digits by chance alone: one in a million. Comparing them is what
//! catches it, as with Bluetooth's numeric comparison; a user who accepts
//! without looking trusts the network on first use.
//!
//! **By code** (a version 1 device on either side): the device being added
//! shows a fresh random code; the user types it on the device that asked.
//! Both run SPAKE2 with the code as the password and the two device ids as
//! identities, then prove the key with an HMAC over the Noise handshake
//! hash. That hash covers both static keys, so a man in the middle, who runs
//! two handshakes with different hashes, fails the confirmation unless they
//! guessed the code, one guess per attempt; an eavesdropper learns nothing
//! to test codes against. Each code serves one attempt.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity as SpakeIdentity, Password, Spake2};

use crate::identity::DeviceId;

/// How many digits a code has.
pub const CODE_DIGITS: usize = 6;

/// A pairing code: six ASCII digits.
#[derive(Clone, PartialEq, Eq)]
pub struct PairCode(String);

impl std::fmt::Debug for PairCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A code is a secret while its pairing runs.
        f.write_str("PairCode(..)")
    }
}

impl PairCode {
    /// A uniformly random code from the system's random source.
    pub fn random() -> Result<Self, getrandom::Error> {
        // Rejection sampling keeps every code equally likely: values at or
        // above the largest multiple of 10^6 below 2^32 are drawn again.
        const LIMIT: u32 = u32::MAX - (u32::MAX % 1_000_000);
        loop {
            let mut bytes = [0; 4];
            getrandom::fill(&mut bytes)?;
            let value = u32::from_le_bytes(bytes);
            if value < LIMIT {
                return Ok(Self(format!("{:06}", value % 1_000_000)));
            }
        }
    }

    /// The code the user typed: digits, with any spaces or dashes ignored;
    /// `None` unless exactly six digits remain.
    pub fn parse(text: &str) -> Option<Self> {
        let digits: String = text
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .collect();
        (digits.len() == CODE_DIGITS && digits.bytes().all(|b| b.is_ascii_digit()))
            .then_some(Self(digits))
    }

    /// The digits.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A confirmation pairing's random value, one per side per attempt.
#[derive(Clone, PartialEq, Eq)]
pub struct Nonce([u8; 32]);

impl std::fmt::Debug for Nonce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Nonce(..)")
    }
}

impl Nonce {
    /// A nonce from the system's random source.
    pub fn random() -> Result<Self, getrandom::Error> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes)?;
        Ok(Self(bytes))
    }

    /// A nonce the other side sent: exactly 32 bytes.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        <[u8; 32]>::try_from(bytes).ok().map(Self)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The commitment the answerer sends before it sees the dialler's
    /// nonce.
    pub fn commitment(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"isthmus pair v2 commit");
        hash.update(self.0);
        hash.finalize().into()
    }

    /// Whether this nonce is the one `commitment` was made to.
    pub fn opens(&self, commitment: &[u8]) -> bool {
        self.commitment().as_slice() == commitment
    }
}

/// The six digits both devices show in a confirmation pairing: from the
/// connection's handshake hash and both nonces, dialler's first.
pub fn comparison_code(handshake_hash: &[u8], dialler: &Nonce, answerer: &Nonce) -> PairCode {
    let mut hash = Sha256::new();
    hash.update(b"isthmus pair v2 compare");
    hash.update(handshake_hash);
    hash.update(dialler.0);
    hash.update(answerer.0);
    let digest: [u8; 32] = hash.finalize().into();
    let mut first = [0; 8];
    first.copy_from_slice(&digest[..8]);
    // 2^64 is so much larger than 10^6 that the remainder's bias is
    // invisible.
    PairCode(format!("{:06}", u64::from_le_bytes(first) % 1_000_000))
}

/// Which side of a pairing this device is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The device that asked and types the code.
    Initiator,
    /// The device being added, which shows the code.
    Responder,
}

impl Role {
    fn label(self) -> &'static [u8] {
        match self {
            Role::Initiator => b"isthmus pair v1 initiator",
            Role::Responder => b"isthmus pair v1 responder",
        }
    }
}

/// One side's SPAKE2 exchange in progress.
pub struct Exchange {
    spake: Spake2<Ed25519Group>,
}

/// SPAKE2 failed: the other side's message was malformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadMessage;

/// Starts `role`'s side of the exchange for `code`, between `initiator` and
/// `responder`; the returned bytes go to the other side.
pub fn start(
    role: Role,
    code: &PairCode,
    initiator: &DeviceId,
    responder: &DeviceId,
) -> (Exchange, Vec<u8>) {
    let password = Password::new(code.as_str().as_bytes());
    let a = SpakeIdentity::new(initiator.as_str().as_bytes());
    let b = SpakeIdentity::new(responder.as_str().as_bytes());
    let (spake, message) = match role {
        Role::Initiator => Spake2::<Ed25519Group>::start_a(&password, &a, &b),
        Role::Responder => Spake2::<Ed25519Group>::start_b(&password, &a, &b),
    };
    (Exchange { spake }, message)
}

impl Exchange {
    /// The shared key from the other side's message. Both sides get the
    /// same key only if they used the same code.
    pub fn finish(self, theirs: &[u8]) -> Result<SharedKey, BadMessage> {
        self.spake
            .finish(theirs)
            .map(SharedKey)
            .map_err(|_| BadMessage)
    }
}

/// The key SPAKE2 agreed.
pub struct SharedKey(Vec<u8>);

impl SharedKey {
    fn mac(&self) -> Hmac<Sha256> {
        // HMAC takes a key of any length; `new_from_slice` cannot fail for it.
        match Hmac::<Sha256>::new_from_slice(&self.0) {
            Ok(mac) => mac,
            Err(_) => unreachable!("HMAC accepts keys of every length"),
        }
    }

    /// `role`'s confirmation for the connection with `handshake_hash`.
    pub fn confirmation(&self, role: Role, handshake_hash: &[u8]) -> Vec<u8> {
        let mut mac = self.mac();
        mac.update(role.label());
        mac.update(handshake_hash);
        mac.finalize().into_bytes().to_vec()
    }

    /// Whether `tag` is `role`'s confirmation, compared in constant time.
    pub fn verify(&self, role: Role, handshake_hash: &[u8], tag: &[u8]) -> bool {
        let mut mac = self.mac();
        mac.update(role.label());
        mac.update(handshake_hash);
        mac.verify_slice(tag).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (DeviceId, DeviceId) {
        (
            DeviceId::try_from("1111111111111111".to_owned()).unwrap(),
            DeviceId::try_from("2222222222222222".to_owned()).unwrap(),
        )
    }

    fn run(
        initiator_code: &str,
        responder_code: &str,
        hash_a: &[u8],
        hash_b: &[u8],
    ) -> (bool, bool) {
        let (a, b) = ids();
        let (ea, ma) = start(
            Role::Initiator,
            &PairCode::parse(initiator_code).unwrap(),
            &a,
            &b,
        );
        let (eb, mb) = start(
            Role::Responder,
            &PairCode::parse(responder_code).unwrap(),
            &a,
            &b,
        );
        let ka = ea.finish(&mb).unwrap();
        let kb = eb.finish(&ma).unwrap();
        let a_ok = kb.verify(
            Role::Initiator,
            hash_b,
            &ka.confirmation(Role::Initiator, hash_a),
        );
        let b_ok = ka.verify(
            Role::Responder,
            hash_a,
            &kb.confirmation(Role::Responder, hash_b),
        );
        (a_ok, b_ok)
    }

    /// The same code on both sides confirms both ways.
    #[test]
    fn the_same_code_confirms_both_ways() {
        assert_eq!(run("123456", "123 456", b"hash", b"hash"), (true, true));
    }

    /// A wrong code fails both confirmations.
    #[test]
    fn a_wrong_code_fails() {
        assert_eq!(run("123456", "123457", b"hash", b"hash"), (false, false));
    }

    /// The right code over two different connections (a man in the middle
    /// relaying between two handshakes) still fails: the confirmation is
    /// bound to the handshake hash.
    #[test]
    fn a_relayed_exchange_fails_even_with_the_right_code() {
        assert_eq!(
            run("123456", "123456", b"hash one", b"hash two"),
            (false, false)
        );
    }

    /// One side's confirmation is not accepted as the other's.
    #[test]
    fn confirmations_are_not_interchangeable() {
        let (a, b) = ids();
        let code = PairCode::parse("000042").unwrap();
        let (ea, ma) = start(Role::Initiator, &code, &a, &b);
        let (eb, mb) = start(Role::Responder, &code, &a, &b);
        let ka = ea.finish(&mb).unwrap();
        let kb = eb.finish(&ma).unwrap();
        let tag = ka.confirmation(Role::Initiator, b"h");
        assert!(!kb.verify(Role::Responder, b"h", &tag));
    }

    /// Codes are six digits, typed with or without separators, and random
    /// codes are always well formed.
    #[test]
    fn codes_are_six_digits() {
        assert!(PairCode::parse("12-34 56").is_some());
        assert!(PairCode::parse("12345").is_none());
        assert!(PairCode::parse("1234567").is_none());
        assert!(PairCode::parse("12345a").is_none());
        assert!(
            PairCode::parse("１２３４５６").is_none(),
            "full-width digits"
        );
        for _ in 0..100 {
            let code = PairCode::random().unwrap();
            assert!(PairCode::parse(code.as_str()).is_some());
        }
    }

    /// Both sides of a confirmation pairing show the same digits; another
    /// connection (a man in the middle's) or another nonce shows others.
    #[test]
    fn both_sides_show_the_same_comparison_code_and_a_relay_does_not() {
        let dialler = Nonce::random().unwrap();
        let answerer = Nonce::random().unwrap();
        let code = comparison_code(b"hash", &dialler, &answerer);
        assert_eq!(comparison_code(b"hash", &dialler, &answerer), code);
        assert!(PairCode::parse(code.as_str()).is_some());
        assert_ne!(comparison_code(b"other hash", &dialler, &answerer), code);
        let other = Nonce::random().unwrap();
        assert_ne!(comparison_code(b"hash", &dialler, &other), code);
    }

    /// A commitment opens with its own nonce and no other, and a nonce
    /// must be 32 bytes.
    #[test]
    fn a_commitment_opens_only_with_its_nonce() {
        let nonce = Nonce::random().unwrap();
        let commitment = nonce.commitment();
        assert!(nonce.opens(&commitment));
        assert!(!Nonce::random().unwrap().opens(&commitment));
        assert!(Nonce::from_bytes(&[0; 31]).is_none());
        assert_eq!(Nonce::from_bytes(nonce.as_bytes()).unwrap(), nonce);
    }
}
