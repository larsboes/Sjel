//! Canonical, reversible Inventory item references.
//!
//! `lamp` is `inv:6c616d70`; `a:b` is `inv:613a62`; `é` is `inv:c3a9`.

/// Maximum UTF-8 byte length accepted in a typed reference.
pub const MAX_ITEM_ID_BYTES: usize = 256;

/// Encode a non-empty raw item ID without changing it.
pub fn encode_item_ref(id: &str) -> Result<String, &'static str> {
    if id.is_empty() || id.len() > MAX_ITEM_ID_BYTES {
        return Err("item ID must contain 1 to 256 UTF-8 bytes");
    }
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(4 + id.len() * 2);
    encoded.push_str("inv:");
    for byte in id.bytes() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 15) as usize] as char);
    }
    Ok(encoded)
}

/// Decode only the canonical lowercase UTF-8 hex representation.
pub fn decode_item_ref(reference: &str) -> Result<String, &'static str> {
    let hex = reference
        .strip_prefix("inv:")
        .ok_or("invalid inventory reference prefix")?
        .as_bytes();
    if hex.is_empty() || hex.len() > MAX_ITEM_ID_BYTES * 2 || hex.len() % 2 != 0 {
        return Err("invalid inventory reference length");
    }
    fn digit(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        }
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_chunks::<2>().0 {
        let high = digit(pair[0]).ok_or("inventory reference must use lowercase hex")?;
        let low = digit(pair[1]).ok_or("inventory reference must use lowercase hex")?;
        bytes.push(high * 16 + low);
    }
    String::from_utf8(bytes).map_err(|_| "inventory reference is not UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_and_round_trips() {
        for (raw, encoded) in [
            ("lamp", "inv:6c616d70"),
            ("a:b", "inv:613a62"),
            ("é", "inv:c3a9"),
            ("\u{1fa91}", "inv:f09faa91"),
        ] {
            assert_eq!(encode_item_ref(raw).unwrap(), encoded);
            assert_eq!(decode_item_ref(encoded).unwrap(), raw);
        }
        let long = "x".repeat(MAX_ITEM_ID_BYTES);
        assert_eq!(
            decode_item_ref(&encode_item_ref(&long).unwrap()).unwrap(),
            long
        );
    }

    #[test]
    fn invalid_and_noncanonical_refs_are_rejected() {
        for value in [
            "",
            "inv:",
            "INV:61",
            "inv:a",
            "inv:GG",
            "inv:6A",
            "inv:ff",
            "inv:61:62",
            "inv:é",
        ] {
            assert!(decode_item_ref(value).is_err(), "{value}");
        }
        assert!(decode_item_ref(&format!("inv:{}", "61".repeat(MAX_ITEM_ID_BYTES + 1))).is_err());
        assert!(encode_item_ref("").is_err());
        assert!(encode_item_ref(&"x".repeat(MAX_ITEM_ID_BYTES + 1)).is_err());
    }
}
