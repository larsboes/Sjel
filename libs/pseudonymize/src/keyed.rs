//! Keyed tokens: the same value gets the same token in every process that holds the key.
//!
//! A counted token (`<TRAVELER_01>`) depends on what one session has already seen, so two
//! capabilities serving one agent would give one person two names. A keyed token is a
//! function of the key, the entity type and the exact value, so they agree without sharing
//! state (ISA F9, ISC-41).
//!
//! HMAC, not a bare hash: without the key, a party that sees `<TRAVELER_k3x9qa>` cannot
//! hash a guessed name and compare. HMAC-SHA256 per RFC 2104, over the workspace's `sha2`.

use sha2::{Digest, Sha256};

const BLOCK: usize = 64;

/// HMAC-SHA256 (RFC 2104). Checked against RFC 4231 test case 2 below.
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|b| b ^ 0x36));
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(block.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// The key for one agent session: the machine secret and the session id, and nothing else.
pub fn session_key(machine_secret: &[u8], session_id: &str) -> [u8; 32] {
    let mut message = b"sjel-agent-session\0".to_vec();
    message.extend_from_slice(session_id.as_bytes());
    hmac_sha256(machine_secret, &message)
}

/// The token suffix for `value` of family `prefix`, `chars` long, lowercase base32.
///
/// Lowercase letters and digits only, so the result still has the shape
/// `session::next_token_shape` recognises and rehydration can find it.
pub(crate) fn suffix(key: &[u8; 32], prefix: &str, value: &str, chars: usize) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut message = prefix.as_bytes().to_vec();
    message.push(0);
    message.extend_from_slice(value.as_bytes());
    let mac = hmac_sha256(key, &message);
    let mut bits: u64 = 0;
    let mut have = 0;
    let mut out = String::with_capacity(chars);
    for byte in mac {
        bits = (bits << 8) | u64::from(byte);
        have += 8;
        while have >= 5 && out.len() < chars {
            have -= 5;
            out.push(ALPHABET[((bits >> have) & 31) as usize] as char);
        }
        if out.len() == chars {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_4231_test_case_2() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn a_long_key_is_hashed_first() {
        // RFC 4231 test case 6: a 131-byte key.
        let mac = hmac_sha256(
            &[0xaa; 131],
            b"Test Using Larger Than Block-Size Key - Hash Key First",
        );
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn the_suffix_is_stable_per_key_and_differs_across_keys() {
        let a = session_key(b"machine", "s1");
        let b = session_key(b"machine", "s2");
        assert_eq!(
            suffix(&a, "TRAVELER", "Anna", 6),
            suffix(&a, "TRAVELER", "Anna", 6)
        );
        assert_ne!(
            suffix(&a, "TRAVELER", "Anna", 6),
            suffix(&b, "TRAVELER", "Anna", 6)
        );
        assert_ne!(
            suffix(&a, "TRAVELER", "Anna", 6),
            suffix(&a, "EMAIL", "Anna", 6)
        );
        assert!(suffix(&a, "TRAVELER", "Anna", 6)
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }
}
