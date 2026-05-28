//! Composite `LeanMarshal` impls: String, Vec/list, Option, Result, tuples.
//!
//! Wire formats: SPEC/canonical-abi.md §§3-7.

use crate::error::{error_codes, LeanError};
use crate::marshal::LeanMarshal;

/// Bounded `read_n` helper.
#[inline]
fn need(buf: &[u8], off: usize, n: usize, what: &str) -> Result<(), LeanError> {
    if off + n > buf.len() {
        Err(LeanError::new(
            error_codes::DECODE_ERROR,
            format!("{what}: out of bounds at offset {off}"),
        ))
    } else {
        Ok(())
    }
}

// ── String — `u32 len + utf-8 bytes` ──────────────────────────────────

impl LeanMarshal for String {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        let bytes = self.as_bytes();
        buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(bytes);
    }

    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 4, "string len")?;
        let len = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
        let body = off + 4;
        need(buf, body, len, "string body")?;
        let slice = &buf[body..body + len];
        match std::str::from_utf8(slice) {
            Ok(s) => Ok((s.to_string(), body + len)),
            Err(_) => Err(LeanError::new(
                error_codes::DECODE_ERROR,
                "string: invalid UTF-8",
            )),
        }
    }
}

// ── Vec<T> — `u32 len + T elements` ──────────────────────────────────

impl<T: LeanMarshal> LeanMarshal for Vec<T> {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&(self.len() as u32).to_le_bytes());
        for elem in self {
            elem.canonical_encode(buf);
        }
    }

    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 4, "list len")?;
        let len = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
        let mut out = Vec::with_capacity(len);
        let mut cur = off + 4;
        for _ in 0..len {
            let (v, next) = T::canonical_decode(buf, cur)?;
            out.push(v);
            cur = next;
        }
        Ok((out, cur))
    }
}

// ── Option<T> — `disc:u8 (0=none, 1=some) + payload` ─────────────────

impl<T: LeanMarshal> LeanMarshal for Option<T> {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        match self {
            None => buf.push(0),
            Some(v) => {
                buf.push(1);
                v.canonical_encode(buf);
            }
        }
    }

    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 1, "option disc")?;
        match buf[off] {
            0 => Ok((None, off + 1)),
            1 => {
                let (v, end) = T::canonical_decode(buf, off + 1)?;
                Ok((Some(v), end))
            }
            b => Err(LeanError::new(
                error_codes::DECODE_ERROR,
                format!("option: invalid tag 0x{b:02x}"),
            )),
        }
    }
}

// ── Result<T, E> — `disc:u8 (0=ok, 1=err) + payload` ─────────────────

impl<T: LeanMarshal, E: LeanMarshal> LeanMarshal for Result<T, E> {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        match self {
            Ok(t) => {
                buf.push(0);
                t.canonical_encode(buf);
            }
            Err(e) => {
                buf.push(1);
                e.canonical_encode(buf);
            }
        }
    }

    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        need(buf, off, 1, "result disc")?;
        match buf[off] {
            0 => {
                let (t, end) = T::canonical_decode(buf, off + 1)?;
                Ok((Ok(t), end))
            }
            1 => {
                let (e, end) = E::canonical_decode(buf, off + 1)?;
                Ok((Err(e), end))
            }
            b => Err(LeanError::new(
                error_codes::DECODE_ERROR,
                format!("result: invalid tag 0x{b:02x}"),
            )),
        }
    }
}

// ── Tuple impls — SPEC/canonical-abi.md §7 ───────────────────────────

macro_rules! tuple_impl {
    ($($name:ident),+) => {
        #[allow(non_snake_case)]
        impl<$($name: LeanMarshal),+> LeanMarshal for ($($name,)+) {
            fn canonical_encode(&self, buf: &mut Vec<u8>) {
                let ($(ref $name,)+) = *self;
                $($name.canonical_encode(buf);)+
            }
            fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
                let mut off = off;
                $(
                    let ($name, next) = $name::canonical_decode(buf, off)?;
                    off = next;
                )+
                Ok((($($name,)+), off))
            }
        }
    };
}

// 1-tuple `(T,)`. The IDL has no `tuple<T>` (a singleton wraps `T`
// directly), but the Rust type `(T,)` shows up in
// `LeanCallback<R, (T,)>` for single-arg function-arrow params
// (Phase 10-B1.x). Wire form is identical to `T` alone — no
// outer framing.
tuple_impl!(A);
tuple_impl!(A, B);
tuple_impl!(A, B, C);
tuple_impl!(A, B, C, D);
tuple_impl!(A, B, C, D, E);

/// Pass-through impl for `Box<T>`. Self-recursive variants (e.g.
/// `enum Tree { Leaf, Node(Box<Tree>, Box<Tree>) }`) need this to
/// satisfy the per-payload `LeanMarshal` bound that derived variant
/// impls emit. The wire form is exactly the inner value's — `Box`
/// only adds heap indirection on the Rust side.
impl<T: LeanMarshal> LeanMarshal for Box<T> {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        <T as LeanMarshal>::canonical_encode(self.as_ref(), buf);
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        let (v, off) = <T as LeanMarshal>::canonical_decode(buf, off)?;
        Ok((Box::new(v), off))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marshal::{decode_from_slice, encode_to_vec};

    #[test]
    fn string_round_trip() {
        for s in ["", "hello", "안녕", "🦀 leo4"] {
            let v = s.to_string();
            let buf = encode_to_vec(&v);
            let decoded: String = decode_from_slice(&buf).unwrap();
            assert_eq!(decoded, v);
        }
    }

    #[test]
    fn string_rejects_invalid_utf8() {
        // len=2, bytes=[0xff, 0xfe] (invalid UTF-8).
        let mut buf = Vec::new();
        buf.extend_from_slice(&2u32.to_le_bytes());
        buf.extend_from_slice(&[0xff, 0xfe]);
        let err = String::canonical_decode(&buf, 0).unwrap_err();
        assert_eq!(err.code, error_codes::DECODE_ERROR);
    }

    #[test]
    fn vec_round_trip_and_nested() {
        let v: Vec<u32> = vec![1, 2, 3, 4];
        let buf = encode_to_vec(&v);
        assert_eq!(buf.len(), 4 + 4 * 4);
        let dec: Vec<u32> = decode_from_slice(&buf).unwrap();
        assert_eq!(dec, v);

        // list<list<u8>> with the inner length prefix on each.
        let nested: Vec<Vec<u8>> = vec![vec![], vec![0], vec![1, 2]];
        let buf2 = encode_to_vec(&nested);
        let dec2: Vec<Vec<u8>> = decode_from_slice(&buf2).unwrap();
        assert_eq!(dec2, nested);
    }

    #[test]
    fn option_round_trip() {
        let v: Option<u64> = None;
        let dec: Option<u64> = decode_from_slice(&encode_to_vec(&v)).unwrap();
        assert_eq!(dec, v);

        let v: Option<u64> = Some(0xDEAD_BEEF_DEAD_BEEF);
        let dec: Option<u64> = decode_from_slice(&encode_to_vec(&v)).unwrap();
        assert_eq!(dec, v);
    }

    #[test]
    fn result_round_trip() {
        let v: Result<u32, String> = Ok(42);
        let dec: Result<u32, String> = decode_from_slice(&encode_to_vec(&v)).unwrap();
        assert_eq!(dec, v);

        let v: Result<u32, String> = Err("nope".into());
        let dec: Result<u32, String> = decode_from_slice(&encode_to_vec(&v)).unwrap();
        assert_eq!(dec, v);
    }

    #[test]
    fn tuple_round_trip() {
        let v: (u8, u16, String) = (1, 2, "three".into());
        let dec: (u8, u16, String) = decode_from_slice(&encode_to_vec(&v)).unwrap();
        assert_eq!(dec, v);
    }
}
