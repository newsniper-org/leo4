//! [`LeanMarshal`] trait — Rust counterpart of `Leo4.LeanMarshal` in
//! `lake/Leo4/Leo4/Marshal.lean`.
//!
//! Encode/decode signatures mirror the Lean side after the `Subarray UInt8`
//! → `ByteArray × Nat` simplification (LEO4-DESIGN.md §10.1). Encoders
//! append to a `Vec<u8>`; decoders read from `&[u8]` starting at a given
//! offset and return the decoded value plus the offset one past its last
//! consumed byte.

use crate::error::{error_codes, LeanError};

/// Canonical-ABI marshalling for boundary types.
///
/// Wire formats are normative — see `SPEC/canonical-abi.md`.  Two
/// implementations of `LeanMarshal` for the same logical type MUST
/// produce identical bytes on the wire.  `tests/conformance/` pins
/// the cross-impl agreement against the Lean side
/// (`lake/Leo4/Leo4/Builtins.lean`).
pub trait LeanMarshal: Sized {
    /// Append `self`'s canonical encoding to `buf`.
    fn canonical_encode(&self, buf: &mut Vec<u8>);

    /// Decode one value of `Self` from `buf` starting at byte `off`.
    /// On success returns `(value, off_after_value)`.
    ///
    /// # Errors
    /// Returns a `LeanError` with one of the reserved codes
    /// (`SPEC/canonical-abi.md` §13) when the bytes do not represent a
    /// valid `Self`.
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError>;
}

/// Convenience: encode to a fresh buffer.
pub fn encode_to_vec<T: LeanMarshal>(v: &T) -> Vec<u8> {
    let mut out = Vec::new();
    v.canonical_encode(&mut out);
    out
}

/// Convenience: decode a value, asserting that the entire buffer was
/// consumed. Useful in tests.
///
/// # Errors
/// Same as `LeanMarshal::canonical_decode`, plus a `DECODE_ERROR` when
/// there are trailing bytes.
pub fn decode_from_slice<T: LeanMarshal>(buf: &[u8]) -> Result<T, LeanError> {
    let (v, off) = T::canonical_decode(buf, 0)?;
    if off != buf.len() {
        return Err(LeanError::new(
            error_codes::DECODE_ERROR,
            format!("trailing bytes after value: {} of {}", off, buf.len()),
        ));
    }
    Ok(v)
}
