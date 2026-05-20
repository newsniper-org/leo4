//! `LeanRat` — Lean-core `Rat` (rational) marshalling, ROADMAP Phase 8.
//!
//! Wire format mirrors `lake/Leo4/Leo4/MathlibSubset.lean`'s
//! `instance : LeanMarshal Rat`:
//!
//! ```text
//!   bigint num
//!   bignat den
//! ```
//!
//! The Lean side reconstructs the rational via `mkRat num den` on
//! decode, which normalises (gcd division) and degenerates to `0/1`
//! when `den == 0`. The Rust side keeps the raw `(num, den)` pair —
//! callers who need a normalised form can reduce it themselves; for
//! "send this Rat to Lean, get a Rat back" the unnormalised in-Rust
//! form is enough because the Lean side renormalises on receive.

use crate::bigint::BigInt;
use crate::bignat::BigNat;
use crate::error::LeanError;
use crate::marshal::LeanMarshal;

/// Rust mirror of Lean core's `Rat`. The `num` carries the signed
/// numerator; `den` is the unsigned denominator (Lean uses `Nat` for
/// `den`). Both fields are arbitrary precision.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LeanRat {
    pub num: BigInt,
    pub den: BigNat,
}

impl LeanRat {
    /// Build a `LeanRat` from machine-sized signed numerator and unsigned
    /// denominator. Used in tests / example code; production callers
    /// constructing `LeanRat` from larger magnitudes should populate the
    /// `BigInt` / `BigNat` fields directly.
    #[must_use]
    pub fn from_i64_u64(num: i64, den: u64) -> Self {
        Self {
            num: BigInt::from_i64(num),
            den: BigNat::from_u64(den),
        }
    }
}

impl LeanMarshal for LeanRat {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        self.num.canonical_encode(buf);
        self.den.canonical_encode(buf);
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        let (num, off) = BigInt::canonical_decode(buf, off)?;
        let (den, off) = BigNat::canonical_decode(buf, off)?;
        Ok((LeanRat { num, den }, off))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marshal::{decode_from_slice, encode_to_vec};

    #[test]
    fn rat_round_trip_basic() {
        for (n, d) in [(0i64, 1u64), (1, 1), (-1, 1), (3, 4), (-7, 5), (i64::MAX, u64::MAX)] {
            let r = LeanRat::from_i64_u64(n, d);
            let bytes = encode_to_vec(&r);
            let decoded: LeanRat = decode_from_slice(&bytes).unwrap();
            assert_eq!(decoded, r, "round-trip {n}/{d}");
        }
    }

    #[test]
    fn rat_round_trip_zero() {
        let r = LeanRat::default();
        let bytes = encode_to_vec(&r);
        let decoded: LeanRat = decode_from_slice(&bytes).unwrap();
        assert_eq!(decoded, r);
    }
}
