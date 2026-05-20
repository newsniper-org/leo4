//! Machine-complex carrier types for leo4. Each `LeanComplexF<N>x2`
//! pairs Rust's machine float `f<N>` with Lean's `Leo4.LeanComplexF<N>x2`
//! structure on the wire. The carrier is a plain `(re, im)` newtype
//! around two machine floats; the wire form is `re (4/8 B LE) +
//! im (4/8 B LE)` matching the Lean record's field-order encode.
//!
//! Stable Rust path (#56). Nightly-only variants
//! (`LeanComplexF16x2`, `LeanComplexBF16x2`, `LeanComplexF128x2`)
//! land in `complex_nightly.rs` behind the `nightly-floats` cargo
//! feature.

use crate::error::LeanError;
use crate::marshal::LeanMarshal;

/// `(f32, f32)` machine complex. Pairs with Lean
/// `Leo4.LeanComplexF32x2 { re : Float32, im : Float32 }`. Wire: 8 B LE.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct LeanComplexF32x2 {
    pub re: f32,
    pub im: f32,
}

impl LeanComplexF32x2 {
    #[must_use]
    pub fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }
}

impl LeanMarshal for LeanComplexF32x2 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        self.re.canonical_encode(buf);
        self.im.canonical_encode(buf);
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        let (re, off) = f32::canonical_decode(buf, off)?;
        let (im, off) = f32::canonical_decode(buf, off)?;
        Ok((LeanComplexF32x2 { re, im }, off))
    }
}

/// `(f64, f64)` machine complex. Pairs with Lean
/// `Leo4.LeanComplexF64x2 { re : Float, im : Float }`. Wire: 16 B LE.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct LeanComplexF64x2 {
    pub re: f64,
    pub im: f64,
}

impl LeanComplexF64x2 {
    #[must_use]
    pub fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }
}

impl LeanMarshal for LeanComplexF64x2 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        self.re.canonical_encode(buf);
        self.im.canonical_encode(buf);
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        let (re, off) = f64::canonical_decode(buf, off)?;
        let (im, off) = f64::canonical_decode(buf, off)?;
        Ok((LeanComplexF64x2 { re, im }, off))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marshal::{decode_from_slice, encode_to_vec};

    #[test]
    fn f32_complex_round_trip() {
        for (re, im) in [(0.0f32, 0.0), (1.5, -2.5), (f32::MIN, f32::MAX), (-0.0, 0.0)] {
            let c = LeanComplexF32x2::new(re, im);
            let bytes = encode_to_vec(&c);
            let back: LeanComplexF32x2 = decode_from_slice(&bytes).unwrap();
            // f32 NaN handling — we don't have NaN in fixtures, so == is fine here.
            assert_eq!(back, c);
            assert_eq!(bytes.len(), 8);
        }
    }

    #[test]
    fn f64_complex_round_trip() {
        for (re, im) in [
            (0.0f64, 0.0),
            (1.5, -2.5),
            (f64::MIN, f64::MAX),
            (1e308, -1e-308),
        ] {
            let c = LeanComplexF64x2::new(re, im);
            let bytes = encode_to_vec(&c);
            let back: LeanComplexF64x2 = decode_from_slice(&bytes).unwrap();
            assert_eq!(back, c);
            assert_eq!(bytes.len(), 16);
        }
    }
}
