//! FNV-1a-64 schema hash, mirroring `lake/Leo4Plugin/Leo4Plugin/Mangling.lean`
//! `Hash`.
//!
//! Both sides MUST agree byte-for-byte: see `SPEC/mangling.md` §3.

/// FNV-1a-64 offset basis.
pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a-64 prime.
pub const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// An 8-byte schema digest stored big-endian inside a `u64`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hash {
    pub value: u64,
}

impl Hash {
    /// FNV-1a-64 over the byte stream.
    #[must_use]
    pub fn fnv1a64(bytes: &[u8]) -> Self {
        let mut h: u64 = FNV_OFFSET;
        for &b in bytes {
            h = (h ^ u64::from(b)).wrapping_mul(FNV_PRIME);
        }
        Self { value: h }
    }

    /// FNV-1a-64 over the UTF-8 encoding of `s`.
    #[must_use]
    pub fn of_str(s: &str) -> Self {
        Self::fnv1a64(s.as_bytes())
    }

    /// 8 hash bytes, big-endian (MSB first).
    #[must_use]
    pub fn to_be_bytes(self) -> [u8; 8] {
        self.value.to_be_bytes()
    }

    /// 16-char lowercase hex, big-endian (matches `Hash.toHex` on the Lean side).
    #[must_use]
    pub fn to_hex(self) -> String {
        format!("{:016x}", self.value)
    }

    /// 13-char RFC 4648 lowercase base32 of the 8 big-endian bytes, no padding.
    /// Matches `Hash.toBase32lc` on the Lean side.
    #[must_use]
    pub fn to_base32lc(self) -> String {
        crate::base32::encode8be(self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cross-impl anchors. Each entry was produced by `Hash.ofString`
    // on the Lean side (`lake/Leo4Plugin/Leo4Plugin/Mangling.lean`)
    // and committed here verbatim. If the algorithm ever changes,
    // regenerate by re-running the same input through the Lean
    // implementation — never by tweaking the Rust numbers to match
    // local drift.
    const ANCHORS: &[(&str, &str, &str)] = &[
        // (input, expected_hex, expected_base32lc)
        ("",        "cbf29ce484222325", "zpzjzzeeeirsk"),
        ("u8",      "08c48207b56753d8", "bdcieb5vm5j5q"),
        ("leo4",    "2498c6ada1f01cf1", "esmmnlnb6aopc"),
    ];

    #[test]
    fn fnv1a64_matches_lean_for_known_inputs() {
        for (input, hex, b32) in ANCHORS {
            let h = Hash::of_str(input);
            assert_eq!(&h.to_hex(), hex, "hex mismatch on {input:?}");
            assert_eq!(&h.to_base32lc(), b32, "base32lc mismatch on {input:?}");
        }
    }

    #[test]
    fn hex_and_be_bytes_agree() {
        for (input, _, _) in ANCHORS {
            let h = Hash::of_str(input);
            let be = h.to_be_bytes();
            let hex_from_be: String =
                be.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(h.to_hex(), hex_from_be);
        }
    }
}
