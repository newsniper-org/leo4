//! RFC 4648 lowercase base32 of an 8-byte big-endian payload.
//!
//! Mirrors `Hash.toBase32lc` in `lake/Leo4Plugin/Leo4Plugin/Mangling.lean`.
//! 8 bytes = 64 bits; emit 13 base32 characters, MSB first. The 13th
//! character carries the lowest 4 bits left-aligned in a 5-bit slot (its
//! low bit is therefore zero — by construction, not by accident).

const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Encode 8 big-endian bytes of `value` as 13 base32 characters.
#[must_use]
pub fn encode8be(value: u64) -> String {
    let mut out = String::with_capacity(13);
    // First 12 characters: 5 bits each from MSB down (60 bits total).
    for i in 0..12_u32 {
        let shift = 59 - 5 * i;
        let idx = ((value >> shift) & 0x1f) as usize;
        out.push(ALPHABET[idx] as char);
    }
    // 13th character: bottom 4 bits of `value`, shifted up into 5-bit slot.
    let last4 = (value & 0x0f) as usize;
    out.push(ALPHABET[last4 << 1] as char);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_is_always_13() {
        for &v in &[0u64, 1, u64::MAX, 0xdead_beef_cafe_babe] {
            assert_eq!(encode8be(v).len(), 13, "len mismatch on {v:#x}");
        }
    }

    #[test]
    fn zero_is_all_first_letter() {
        assert_eq!(encode8be(0), "aaaaaaaaaaaaa");
    }

    #[test]
    fn max_packs_all_high() {
        // value = 0xFFFF_FFFF_FFFF_FFFF
        //  → 12 chars from MSB: each chunk is 0x1f (= '7'),
        //  → 13th char: low 4 bits = 0xf, shifted left to slot 30 ('6'),
        let s = encode8be(u64::MAX);
        assert_eq!(&s[..12], "777777777777");
        assert_eq!(s.chars().nth(12).unwrap(), '6');
    }
}
