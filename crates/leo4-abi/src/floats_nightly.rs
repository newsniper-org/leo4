//! Nightly-only float carriers (Phase 8 #57).
//!
//! Active only when `--features nightly-floats` is set. Rust's `f16`
//! and `f128` are nightly primitives (`feature(f16)` / `feature(f128)`
//! enabled at the crate root via `cfg_attr` in `lib.rs`). `bf16` has
//! no native Rust primitive yet; we carry its bit pattern as a `u16`
//! newtype matching the on-wire layout the Lean side uses.
//!
//! Wire formats (LE in every case):
//!
//! | type                 | bytes | composition                       |
//! |----------------------|-------|-----------------------------------|
//! | `f16`                | 2     | raw IEEE-754 binary16             |
//! | `LeanBF16`           | 2     | brain-float16 bit pattern (u16)   |
//! | `f128`               | 16    | raw IEEE-754 binary128            |
//! | `LeanComplexF16x2`   | 4     | re (f16) ‖ im (f16)               |
//! | `LeanComplexBF16x2`  | 4     | re (bf16 u16) ‖ im (bf16 u16)     |
//! | `LeanComplexF128x2`  | 32    | re (f128) ‖ im (f128)             |

use crate::error::{error_codes, LeanError};
use crate::marshal::LeanMarshal;

#[inline]
fn need(buf: &[u8], off: usize, n: usize, what: &str) -> Result<(), LeanError> {
    if buf.len() < off + n {
        Err(LeanError::new(
            error_codes::DECODE_ERROR,
            format!("{what}: out of bounds at offset {off}"),
        ))
    } else {
        Ok(())
    }
}

// ── f16 (IEEE-754 binary16) ────────────────────────────────────────────

impl LeanMarshal for f16 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_le_bytes());
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 2, "f16")?;
        let v = f16::from_le_bytes(buf[off..off + 2].try_into().unwrap());
        Ok((v, off + 2))
    }
}

// ── f128 (IEEE-754 binary128) ──────────────────────────────────────────

impl LeanMarshal for f128 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_le_bytes());
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 16, "f128")?;
        let v = f128::from_le_bytes(buf[off..off + 16].try_into().unwrap());
        Ok((v, off + 16))
    }
}

// ── bf16 (brain-float16) ───────────────────────────────────────────────
//
// Carried as a bit-pattern u16 wrapper. Users who need arithmetic can
// reinterpret via the `half` crate or similar — leo4 just shuttles the
// bits unmodified.

/// Brain-float16 (bfloat16) bit-pattern carrier. `bits` is the
/// raw 16-bit IEEE-style layout (1 sign + 8 exponent + 7 mantissa).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LeanBF16 {
    pub bits: u16,
}

impl LeanMarshal for LeanBF16 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.bits.to_le_bytes());
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 2, "bf16")?;
        let bits = u16::from_le_bytes(buf[off..off + 2].try_into().unwrap());
        Ok((LeanBF16 { bits }, off + 2))
    }
}

// ── complex carriers ───────────────────────────────────────────────────

/// `(f16, f16)` machine complex. Wire: 4 B LE.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct LeanComplexF16x2 {
    pub re: f16,
    pub im: f16,
}

impl LeanMarshal for LeanComplexF16x2 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        self.re.canonical_encode(buf);
        self.im.canonical_encode(buf);
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        let (re, off) = f16::canonical_decode(buf, off)?;
        let (im, off) = f16::canonical_decode(buf, off)?;
        Ok((LeanComplexF16x2 { re, im }, off))
    }
}

/// `(bf16, bf16)` machine complex carrier. Wire: 4 B LE (two bit
/// patterns).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LeanComplexBF16x2 {
    pub re: LeanBF16,
    pub im: LeanBF16,
}

impl LeanMarshal for LeanComplexBF16x2 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        self.re.canonical_encode(buf);
        self.im.canonical_encode(buf);
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        let (re, off) = LeanBF16::canonical_decode(buf, off)?;
        let (im, off) = LeanBF16::canonical_decode(buf, off)?;
        Ok((LeanComplexBF16x2 { re, im }, off))
    }
}

/// `(f128, f128)` machine complex. Wire: 32 B LE.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct LeanComplexF128x2 {
    pub re: f128,
    pub im: f128,
}

impl LeanMarshal for LeanComplexF128x2 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        self.re.canonical_encode(buf);
        self.im.canonical_encode(buf);
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        let (re, off) = f128::canonical_decode(buf, off)?;
        let (im, off) = f128::canonical_decode(buf, off)?;
        Ok((LeanComplexF128x2 { re, im }, off))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marshal::{decode_from_slice, encode_to_vec};

    #[test]
    fn f16_round_trip() {
        for v in [0.0f16, 1.0, -1.5, f16::MIN, f16::MAX] {
            let bytes = encode_to_vec(&v);
            assert_eq!(bytes.len(), 2);
            let back: f16 = decode_from_slice(&bytes).unwrap();
            assert_eq!(back.to_bits(), v.to_bits());
        }
    }

    #[test]
    fn f128_round_trip() {
        for v in [0.0f128, 1.0, -1.5, f128::MIN, f128::MAX] {
            let bytes = encode_to_vec(&v);
            assert_eq!(bytes.len(), 16);
            let back: f128 = decode_from_slice(&bytes).unwrap();
            assert_eq!(back.to_bits(), v.to_bits());
        }
    }

    #[test]
    fn bf16_round_trip() {
        for bits in [0u16, 0x3f80, 0x4000, 0xffff] {
            let v = LeanBF16 { bits };
            let bytes = encode_to_vec(&v);
            assert_eq!(bytes.len(), 2);
            let back: LeanBF16 = decode_from_slice(&bytes).unwrap();
            assert_eq!(back, v);
        }
    }

    #[test]
    fn complex_f16x2_round_trip() {
        let c = LeanComplexF16x2 { re: 1.0, im: -2.5 };
        let bytes = encode_to_vec(&c);
        assert_eq!(bytes.len(), 4);
        let back: LeanComplexF16x2 = decode_from_slice(&bytes).unwrap();
        assert_eq!(back.re.to_bits(), c.re.to_bits());
        assert_eq!(back.im.to_bits(), c.im.to_bits());
    }

    #[test]
    fn complex_bf16x2_round_trip() {
        let c = LeanComplexBF16x2 {
            re: LeanBF16 { bits: 0x3f80 },
            im: LeanBF16 { bits: 0xc000 },
        };
        let bytes = encode_to_vec(&c);
        assert_eq!(bytes.len(), 4);
        let back: LeanComplexBF16x2 = decode_from_slice(&bytes).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn complex_f128x2_round_trip() {
        let c = LeanComplexF128x2 { re: 2.0f128, im: -3.0 };
        let bytes = encode_to_vec(&c);
        assert_eq!(bytes.len(), 32);
        let back: LeanComplexF128x2 = decode_from_slice(&bytes).unwrap();
        assert_eq!(back.re.to_bits(), c.re.to_bits());
        assert_eq!(back.im.to_bits(), c.im.to_bits());
    }
}
