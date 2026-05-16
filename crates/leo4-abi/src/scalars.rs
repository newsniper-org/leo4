//! Scalar `LeanMarshal` impls.
//!
//! Wire format: SPEC/canonical-abi.md §1 (one byte for `u8`/`i8`/`bool`,
//! N little-endian bytes for `uN`/`iN`, IEEE-754 LE for `f32`/`f64`,
//! u32 codepoint for `char`).
//! Mirrors `lake/Leo4/Leo4/Builtins.lean`.

use crate::error::{error_codes, LeanError};
use crate::marshal::LeanMarshal;

#[inline]
fn need(buf: &[u8], off: usize, n: usize, what: &str) -> Result<(), LeanError> {
    if off + n > buf.len() {
        Err(LeanError::new(
            error_codes::DECODE_ERROR,
            format!("{what}: out of bounds (need {n} bytes at offset {off}, have {})", buf.len()),
        ))
    } else {
        Ok(())
    }
}

// ── unsigned integers ─────────────────────────────────────────────────

impl LeanMarshal for u8 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.push(*self);
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 1, "u8")?;
        Ok((buf[off], off + 1))
    }
}

impl LeanMarshal for u16 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_le_bytes());
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 2, "u16")?;
        let v = u16::from_le_bytes(buf[off..off + 2].try_into().unwrap());
        Ok((v, off + 2))
    }
}

impl LeanMarshal for u32 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_le_bytes());
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 4, "u32")?;
        let v = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
        Ok((v, off + 4))
    }
}

impl LeanMarshal for u64 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_le_bytes());
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 8, "u64")?;
        let v = u64::from_le_bytes(buf[off..off + 8].try_into().unwrap());
        Ok((v, off + 8))
    }
}

// ── signed integers ───────────────────────────────────────────────────

impl LeanMarshal for i8 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.push(*self as u8);
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 1, "i8")?;
        Ok((buf[off] as i8, off + 1))
    }
}

impl LeanMarshal for i16 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_le_bytes());
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 2, "i16")?;
        let v = i16::from_le_bytes(buf[off..off + 2].try_into().unwrap());
        Ok((v, off + 2))
    }
}

impl LeanMarshal for i32 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_le_bytes());
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 4, "i32")?;
        let v = i32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
        Ok((v, off + 4))
    }
}

impl LeanMarshal for i64 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_le_bytes());
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 8, "i64")?;
        let v = i64::from_le_bytes(buf[off..off + 8].try_into().unwrap());
        Ok((v, off + 8))
    }
}

// ── floats ────────────────────────────────────────────────────────────

impl LeanMarshal for f32 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_bits().to_le_bytes());
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 4, "f32")?;
        let bits = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
        Ok((f32::from_bits(bits), off + 4))
    }
}

impl LeanMarshal for f64 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_bits().to_le_bytes());
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 8, "f64")?;
        let bits = u64::from_le_bytes(buf[off..off + 8].try_into().unwrap());
        Ok((f64::from_bits(bits), off + 8))
    }
}

// ── bool / char ───────────────────────────────────────────────────────

impl LeanMarshal for bool {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.push(if *self { 1 } else { 0 });
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 1, "bool")?;
        match buf[off] {
            0 => Ok((false, off + 1)),
            1 => Ok((true, off + 1)),
            b => Err(LeanError::new(
                error_codes::DECODE_ERROR,
                format!("bool: invalid byte 0x{b:02x}"),
            )),
        }
    }
}

impl LeanMarshal for char {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        let cp: u32 = (*self).into();
        buf.extend_from_slice(&cp.to_le_bytes());
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 4, "char")?;
        let cp = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap());
        char::from_u32(cp)
            .map(|c| (c, off + 4))
            .ok_or_else(|| {
                LeanError::new(
                    error_codes::DECODE_ERROR,
                    format!("char: invalid codepoint 0x{cp:08x}"),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marshal::{decode_from_slice, encode_to_vec};

    fn round_trip<T: LeanMarshal + PartialEq + std::fmt::Debug + Clone>(v: T, expected_len: usize) {
        let buf = encode_to_vec(&v);
        assert_eq!(buf.len(), expected_len, "encoded length for {:?}", v);
        let decoded: T = decode_from_slice(&buf).unwrap();
        assert_eq!(decoded, v);
    }

    #[test]
    fn unsigned_round_trips() {
        round_trip(0u8, 1);
        round_trip(255u8, 1);
        round_trip(0xCAFEu16, 2);
        round_trip(0xDEAD_BEEFu32, 4);
        round_trip(0x1234_5678_9ABC_DEF0u64, 8);
    }

    #[test]
    fn signed_round_trips() {
        round_trip(-1i8, 1);
        round_trip(i16::MIN, 2);
        round_trip(i32::MAX, 4);
        round_trip(i64::MIN, 8);
    }

    #[test]
    fn float_round_trips() {
        round_trip(0.0f32, 4);
        round_trip(-0.0f32, 4);
        round_trip(f32::INFINITY, 4);
        round_trip(std::f64::consts::PI, 8);
    }

    #[test]
    fn bool_round_trip_and_invalid_byte() {
        round_trip(false, 1);
        round_trip(true, 1);
        let err = <bool as LeanMarshal>::canonical_decode(&[2u8], 0).unwrap_err();
        assert_eq!(err.code, error_codes::DECODE_ERROR);
    }

    #[test]
    fn char_round_trip_and_invalid_codepoint() {
        round_trip('a', 4);
        round_trip('한', 4);
        let buf = 0xD800_u32.to_le_bytes(); // surrogate — invalid
        let err = <char as LeanMarshal>::canonical_decode(&buf, 0).unwrap_err();
        assert_eq!(err.code, error_codes::DECODE_ERROR);
    }
}
