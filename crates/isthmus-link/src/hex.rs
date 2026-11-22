//! Lowercase hex for keys, ids and tags in JSON: small enough to write here
//! rather than take a crate for.

/// `bytes` as lowercase hex.
pub(crate) fn encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

/// Fills `out` from `text`, which must be exactly twice as long and hex
/// (either case); false otherwise, with `out` partly written.
pub(crate) fn decode_into(text: &str, out: &mut [u8]) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != out.len() * 2 {
        return false;
    }
    let (pairs, _) = bytes.as_chunks::<2>();
    for (slot, [high, low]) in out.iter_mut().zip(pairs) {
        let (Some(high), Some(low)) = (nibble(*high), nibble(*low)) else {
            return false;
        };
        *slot = (high << 4) | low;
    }
    true
}

/// `text` decoded, or `None` when it is not hex of an even length.
pub(crate) fn decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let mut out = vec![0; text.len() / 2];
    decode_into(text, &mut out).then_some(out)
}

fn nibble(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encoding and decoding round-trip, and anything that is not hex of
    /// the right length is refused rather than half read.
    #[test]
    fn hex_round_trips_and_refuses_malformed_text() {
        let bytes = [0x00, 0x7f, 0xa5, 0xff];
        assert_eq!(encode(&bytes), "007fa5ff");
        assert_eq!(decode("007FA5ff").unwrap(), bytes);
        assert!(decode("abc").is_none());
        assert!(decode("zz").is_none());
        let mut out = [0u8; 2];
        assert!(!decode_into("00", &mut out));
    }
}
