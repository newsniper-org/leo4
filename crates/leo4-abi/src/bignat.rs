//! `BigNat` — arbitrary-precision unsigned integer marshalling.
//!
//! Wire format: SPEC/canonical-abi.md §2.
//!   `len:u32  limbs:[u64; len]` (little-endian limbs, LSB first).
//! Most significant limb is non-zero unless `len == 0` (the value 0).
//!
//! On the Rust side we represent the value as a `num_bigint`-like
//! sequence of u64 limbs but, without an external dependency, we expose
//! a thin wrapper around `Vec<u64>`. Conversion helpers to/from
//! primitive integer types are provided for convenience.

use crate::error::{error_codes, LeanError};
use crate::marshal::LeanMarshal;

/// Arbitrary-precision non-negative integer, little-endian u64 limbs.
/// `limbs[0]` is the least-significant 64 bits.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BigNat {
    pub limbs: Vec<u64>,
}

impl BigNat {
    /// Construct from a sequence of little-endian limbs. The most
    /// significant zero limbs are stripped — `Vec![0]` and `Vec![]`
    /// both canonicalise to `Vec![]` (value `0`).
    #[must_use]
    pub fn from_limbs(mut limbs: Vec<u64>) -> Self {
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        Self { limbs }
    }

    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    /// Construct from a `u64`.
    #[must_use]
    pub fn from_u64(n: u64) -> Self {
        if n == 0 {
            Self::default()
        } else {
            Self { limbs: vec![n] }
        }
    }

    /// Convert to `u64`, returning `None` if the value does not fit.
    #[must_use]
    pub fn to_u64(&self) -> Option<u64> {
        match self.limbs.len() {
            0 => Some(0),
            1 => Some(self.limbs[0]),
            _ => None,
        }
    }
}

impl LeanMarshal for BigNat {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        let len = self.limbs.len() as u32;
        buf.extend_from_slice(&len.to_le_bytes());
        for limb in &self.limbs {
            buf.extend_from_slice(&limb.to_le_bytes());
        }
    }

    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        if off + 4 > buf.len() {
            return Err(LeanError::new(
                error_codes::DECODE_ERROR,
                "bignat: missing length prefix",
            ));
        }
        let len = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
        let mut cur = off + 4;
        if cur + len * 8 > buf.len() {
            return Err(LeanError::new(
                error_codes::DECODE_ERROR,
                format!("bignat: truncated limb stream (need {} bytes, have {})", len * 8, buf.len() - cur),
            ));
        }
        let mut limbs = Vec::with_capacity(len);
        for _ in 0..len {
            let limb = u64::from_le_bytes(buf[cur..cur + 8].try_into().unwrap());
            limbs.push(limb);
            cur += 8;
        }
        // Decoder MUST reject a non-zero `len` whose MSB limb is 0
        // (SPEC/canonical-abi.md §2).
        if len > 0 && *limbs.last().unwrap() == 0 {
            return Err(LeanError::new(
                error_codes::DECODE_ERROR,
                "bignat: leading-zero limb violates canonical form",
            ));
        }
        Ok((BigNat { limbs }, cur))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marshal::{decode_from_slice, encode_to_vec};

    #[test]
    fn round_trip_zero() {
        let v = BigNat::default();
        let buf = encode_to_vec(&v);
        assert_eq!(buf, vec![0u8, 0, 0, 0]);
        let decoded: BigNat = decode_from_slice(&buf).unwrap();
        assert_eq!(decoded, v);
    }

    #[test]
    fn round_trip_one_limb() {
        let v = BigNat::from_u64(0xDEAD_BEEF_CAFE_BABE);
        let buf = encode_to_vec(&v);
        assert_eq!(buf.len(), 4 + 8);
        let decoded: BigNat = decode_from_slice(&buf).unwrap();
        assert_eq!(decoded, v);
    }

    #[test]
    fn round_trip_three_limbs() {
        let v = BigNat { limbs: vec![1, 2, 3] };
        let buf = encode_to_vec(&v);
        let decoded: BigNat = decode_from_slice(&buf).unwrap();
        assert_eq!(decoded, v);
    }

    #[test]
    fn rejects_leading_zero_limb() {
        // len=2 limb=[0xaa, 0x00] → MSB limb 0 violates canonical form.
        let mut buf = vec![];
        buf.extend_from_slice(&2u32.to_le_bytes());
        buf.extend_from_slice(&0xaau64.to_le_bytes());
        buf.extend_from_slice(&0u64.to_le_bytes());
        let err = BigNat::canonical_decode(&buf, 0).unwrap_err();
        assert_eq!(err.code, error_codes::DECODE_ERROR);
    }
}
