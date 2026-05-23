//! [`LeanMarshal`] trait — Rust counterpart of `Leo4.LeanMarshal` in
//! `lake/Leo4/Leo4/Marshal.lean`.
//!
//! Encode/decode signatures mirror the Lean side after the `Subarray UInt8`
//! → `ByteArray × Nat` simplification (LEO4-DESIGN.md §10.1). Encoders
//! append to a `Vec<u8>`; decoders read from `&[u8]` starting at a given
//! offset and return the decoded value plus the offset one past its last
//! consumed byte.

use crate::error::{error_codes, LeanError};

/// Maximum nesting depth for `Self`-recursive decoders before they
/// surface `LEO4_ERR_DECODE_DEPTH_EXCEEDED` (`0x08`).
///
/// Pinned at 256, matching the `max_decode_depth` default
/// documented in `SPEC/canonical-abi.md` §8.1. Self-recursive
/// decoders MUST call [`check_decode_depth`] on every recursive site.
/// A future `leo4.toml` knob (Phase 10+) will let users override
/// this for fixtures that legitimately go deeper.
pub const MAX_DECODE_DEPTH: usize = 256;

/// Sentinel handle representing an unbound / null `LeanResource`.
///
/// Decoding a wire blob that carries this value returns
/// [`LeanError::invalid_handle`]; encoding a resource with this
/// handle returns [`LeanError::encode_error`].
pub const INVALID_RESOURCE_HANDLE: u64 = 0;

/// Increment a `Self`-recursive decoder's depth counter; surface
/// [`error_codes::DECODE_DEPTH_EXCEEDED`] if it would exceed
/// [`MAX_DECODE_DEPTH`].
///
/// # Errors
/// `LeanError` with code `0x08` once `depth >= MAX_DECODE_DEPTH`.
#[inline]
pub fn check_decode_depth(depth: usize) -> Result<usize, LeanError> {
    if depth >= MAX_DECODE_DEPTH {
        Err(LeanError::decode_depth_exceeded(depth, MAX_DECODE_DEPTH))
    } else {
        Ok(depth + 1)
    }
}

/// Encode a `LeanResource` handle, rejecting the
/// [`INVALID_RESOURCE_HANDLE`] sentinel with
/// `LEO4_ERR_ENCODE_ERROR` (`0x02`).
///
/// # Errors
/// `LeanError::encode_error` when `handle == INVALID_RESOURCE_HANDLE`.
pub fn encode_resource_handle(handle: u64, buf: &mut Vec<u8>) -> Result<(), LeanError> {
    if handle == INVALID_RESOURCE_HANDLE {
        return Err(LeanError::encode_error(
            "LeanResource handle 0 is reserved as the null sentinel",
        ));
    }
    buf.extend_from_slice(&handle.to_le_bytes());
    Ok(())
}

/// Decode a `LeanResource` handle, rejecting the
/// [`INVALID_RESOURCE_HANDLE`] sentinel with
/// `LEO4_ERR_INVALID_HANDLE` (`0x03`).
///
/// # Errors
/// `LeanError::invalid_handle` when the wire carries
/// `INVALID_RESOURCE_HANDLE`, or `LeanError` with `DECODE_ERROR`
/// (`0x01`) if the buffer is short.
pub fn decode_resource_handle(buf: &[u8], off: usize) -> Result<(u64, usize), LeanError> {
    if off + 8 > buf.len() {
        return Err(LeanError::new(
            error_codes::DECODE_ERROR,
            "decode_resource_handle: short buffer",
        ));
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&buf[off..off + 8]);
    let handle = u64::from_le_bytes(bytes);
    if handle == INVALID_RESOURCE_HANDLE {
        return Err(LeanError::invalid_handle(
            "LeanResource handle 0 received (unbound / null)",
        ));
    }
    Ok((handle, off + 8))
}

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

/// Encode `v` into a fixed-capacity buffer. Returns the number of bytes
/// written.  Matches the shape of the C shim entry point
/// `leo4_call_<mangled>(…, ret_ptr, ret_cap, ret_len)` from
/// `SPEC/canonical-abi.md` §14 — Phase 5 wires this into the actual
/// shim. v0 uses it to surface
/// [`error_codes::BUFFER_TOO_SMALL`](super::error::error_codes::BUFFER_TOO_SMALL)
/// through a documented code path.
///
/// # Errors
/// Returns a `LeanError` with `BUFFER_TOO_SMALL` (0x07) when `buf.len()`
/// is smaller than the encoded value.
pub fn encode_to_fixed<T: LeanMarshal>(v: &T, buf: &mut [u8]) -> Result<usize, LeanError> {
    let mut tmp = Vec::new();
    v.canonical_encode(&mut tmp);
    if tmp.len() > buf.len() {
        return Err(LeanError::new(
            error_codes::BUFFER_TOO_SMALL,
            format!("encode_to_fixed: need {} bytes, have {}", tmp.len(), buf.len()),
        ));
    }
    buf[..tmp.len()].copy_from_slice(&tmp);
    Ok(tmp.len())
}
