//! `BigInt` — arbitrary-precision signed integer marshalling.
//!
//! Wire format: SPEC/canonical-abi.md §2.
//!   `sign:u8  len:u32  limbs:[u64; len]`  (little-endian limbs).
//! `sign == 0` for non-negative, `1` for negative.  Magnitude shares
//! `BigNat`'s shape.  Value `0` is required to be `sign=0, len=0` —
//! negative-zero is illegal.

use crate::bignat::BigNat;
use crate::error::{error_codes, LeanError};
use crate::marshal::LeanMarshal;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct BigInt {
    pub negative: bool,
    pub magnitude: BigNat,
}

impl BigInt {
    /// Construct from a signed primitive integer.
    #[must_use]
    pub fn from_i64(n: i64) -> Self {
        if n == 0 {
            Self::default()
        } else if n < 0 {
            // i64::MIN has |n| = 1 << 63 which is exactly fits a u64.
            let mag = (n as i128).unsigned_abs() as u64;
            Self {
                negative: true,
                magnitude: BigNat::from_u64(mag),
            }
        } else {
            Self {
                negative: false,
                magnitude: BigNat::from_u64(n as u64),
            }
        }
    }
}

impl LeanMarshal for BigInt {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.push(u8::from(self.negative));
        self.magnitude.canonical_encode(buf);
    }

    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        if off + 1 > buf.len() {
            return Err(LeanError::new(
                error_codes::DECODE_ERROR,
                "bigint: missing sign byte",
            ));
        }
        let sign = buf[off];
        if sign > 1 {
            return Err(LeanError::new(
                error_codes::DECODE_ERROR,
                format!("bigint: invalid sign byte 0x{sign:02x}"),
            ));
        }
        let (mag, end) = BigNat::canonical_decode(buf, off + 1)?;
        if sign == 1 && mag.is_zero() {
            return Err(LeanError::new(
                error_codes::DECODE_ERROR,
                "bigint: negative zero is illegal",
            ));
        }
        Ok((
            BigInt {
                negative: sign == 1,
                magnitude: mag,
            },
            end,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marshal::{decode_from_slice, encode_to_vec};

    fn rt(n: i64) {
        let v = BigInt::from_i64(n);
        let buf = encode_to_vec(&v);
        let decoded: BigInt = decode_from_slice(&buf).unwrap();
        assert_eq!(decoded, v);
    }

    #[test]
    fn round_trip_zero_and_signed() {
        rt(0);
        rt(1);
        rt(-1);
        rt(i64::MAX);
        rt(i64::MIN);
    }

    #[test]
    fn rejects_negative_zero() {
        let buf = [1u8, 0u8, 0, 0, 0]; // sign=1, len=0 → -0
        let err = BigInt::canonical_decode(&buf, 0).unwrap_err();
        assert_eq!(err.code, error_codes::DECODE_ERROR);
    }

    #[test]
    fn rejects_invalid_sign_byte() {
        let buf = [2u8, 0u8, 0, 0, 0];
        let err = BigInt::canonical_decode(&buf, 0).unwrap_err();
        assert_eq!(err.code, error_codes::DECODE_ERROR);
    }
}
