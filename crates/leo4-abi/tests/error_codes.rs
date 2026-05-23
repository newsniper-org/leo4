//! Phase 4 / Phase 10-F1 exit-criterion test: every reserved
//! [`LeanError`] code from `SPEC/canonical-abi.md` §13 has at least one
//! *real trigger* on the Rust side.
//!
//! Phase 4 shipped stubs (constructor-path coverage) for codes that
//! couldn't be triggered without shim infrastructure: 0x02 / 0x03 /
//! 0x04 / 0x06 / 0x08. Phase 10-F1 (2026-05-21) replaces those stubs
//! with code-path triggers using helpers exposed by `leo4-abi` itself
//! (`MAX_DECODE_DEPTH`, `check_decode_depth`,
//! `encode_resource_handle`, `decode_resource_handle`, and the
//! `LeanError::*` convenience constructors). End-to-end shim-level
//! triggers (libloading `dlsym` miss, allocator OOM under a real
//! workload) remain Phase 5+ / leo4-mslean4 concerns.

use std::collections::HashMap;

use leo4_abi::error::error_codes::*;
use leo4_abi::marshal::{
    check_decode_depth, decode_resource_handle, encode_resource_handle, encode_to_fixed,
    INVALID_RESOURCE_HANDLE, MAX_DECODE_DEPTH,
};
use leo4_abi::{handshake::check_schema_hash, LeanError};

// ─── 0x01 DECODE_ERROR — preserved Phase 4 trigger ────────────────────

#[test]
fn decode_error_triggered_by_invalid_bool_byte() {
    let err = <bool as leo4_abi::LeanMarshal>::canonical_decode(&[2u8], 0).unwrap_err();
    assert_eq!(err.code, DECODE_ERROR);
}

// ─── 0x02 ENCODE_ERROR — Phase 10-F1 real trigger ────────────────────

#[test]
fn encode_error_triggered_by_null_resource_handle() {
    // The null-sentinel handle (`INVALID_RESOURCE_HANDLE == 0`) is
    // forbidden at encode time — encoders reject it as out-of-domain.
    let mut buf = Vec::new();
    let err = encode_resource_handle(INVALID_RESOURCE_HANDLE, &mut buf).unwrap_err();
    assert_eq!(err.code, ENCODE_ERROR);
    assert!(buf.is_empty(), "encode must not append bytes on error");
}

#[test]
fn encode_resource_handle_succeeds_for_non_zero() {
    let mut buf = Vec::new();
    encode_resource_handle(0xdead_beef_cafe_babe, &mut buf).unwrap();
    assert_eq!(buf.len(), 8);
}

// ─── 0x03 INVALID_HANDLE — Phase 10-F1 real trigger ──────────────────

#[test]
fn invalid_handle_triggered_by_null_resource_decode() {
    // Wire blob carrying handle `0` triggers INVALID_HANDLE on decode.
    let bytes = [0u8; 8];
    let err = decode_resource_handle(&bytes, 0).unwrap_err();
    assert_eq!(err.code, INVALID_HANDLE);
}

#[test]
fn decode_resource_handle_round_trips_non_zero() {
    let mut buf = Vec::new();
    encode_resource_handle(0x42, &mut buf).unwrap();
    let (h, off) = decode_resource_handle(&buf, 0).unwrap();
    assert_eq!(h, 0x42);
    assert_eq!(off, 8);
}

// ─── 0x04 OOM — Phase 10-F1 real trigger ─────────────────────────────

#[test]
fn oom_triggered_by_try_reserve_overflow() {
    // `Vec::try_reserve` rejects requests larger than `isize::MAX`
    // with a `TryReserveError`, which the canonical-ABI path maps
    // to `LeanError::oom`.
    #[allow(clippy::cast_sign_loss)]
    let request = isize::MAX as usize;
    let mut buf: Vec<u8> = Vec::new();
    let alloc_err = buf
        .try_reserve(request)
        .expect_err("isize::MAX must overflow Vec capacity");
    let err = LeanError::oom(format!("Vec::try_reserve({request}): {alloc_err}"));
    assert_eq!(err.code, OOM);
}

// ─── 0x05 HANDSHAKE_MISMATCH — preserved Phase 4 trigger ─────────────

#[test]
fn handshake_mismatch_triggered_by_check_schema_hash() {
    let err = check_schema_hash("expected13chars", "actualxxxxxxx").unwrap_err();
    assert_eq!(err.code, HANDSHAKE_MISMATCH);
}

// ─── 0x06 UNKNOWN_FUNCTION — Phase 10-F1 real trigger ───────────────

#[test]
fn unknown_function_triggered_by_missing_table_entry() {
    // Static lookup-table miss is the in-leo4-abi proxy for
    // libloading's `dlsym` miss (which lives in leo4-mslean4).
    let table: HashMap<&str, fn(u64) -> u64> = HashMap::from([("doubler", (|x| x * 2) as fn(u64) -> u64)]);
    let missing = "halver";
    let err = table
        .get(missing)
        .map_or_else(|| LeanError::unknown_function(missing), |_| unreachable!());
    assert_eq!(err.code, UNKNOWN_FUNCTION);
    assert!(err.message.contains("halver"));
}

// ─── 0x07 BUFFER_TOO_SMALL — preserved Phase 4 trigger ──────────────

#[test]
fn buffer_too_small_triggered_by_encode_to_fixed() {
    let mut tiny = [0u8; 2];
    let err = encode_to_fixed(&0xDEAD_BEEFu32, &mut tiny).unwrap_err();
    assert_eq!(err.code, BUFFER_TOO_SMALL);
}

// ─── 0x08 DECODE_DEPTH_EXCEEDED — Phase 10-F1 real trigger ──────────

/// A toy `Self`-recursive decoder for a unary-counter wire format:
/// each `0x01` byte means "another nesting level", `0x00` terminates.
/// Every recursive call goes through [`check_decode_depth`].
fn decode_unary(buf: &[u8], off: usize, depth: usize) -> Result<usize, LeanError> {
    if off >= buf.len() {
        return Err(LeanError::new(DECODE_ERROR, "decode_unary: short buffer"));
    }
    if buf[off] == 0 {
        return Ok(off + 1);
    }
    let next_depth = check_decode_depth(depth)?;
    decode_unary(buf, off + 1, next_depth)
}

#[test]
fn decode_depth_exceeded_triggered_by_too_deep_recursion() {
    // MAX_DECODE_DEPTH + 1 levels of nesting before the terminator.
    let mut wire = vec![1u8; MAX_DECODE_DEPTH + 1];
    wire.push(0);
    let err = decode_unary(&wire, 0, 0).unwrap_err();
    assert_eq!(err.code, DECODE_DEPTH_EXCEEDED);
}

#[test]
fn decode_depth_stays_clear_below_cap() {
    let mut wire = vec![1u8; MAX_DECODE_DEPTH - 1];
    wire.push(0);
    let off = decode_unary(&wire, 0, 0).expect("MAX_DECODE_DEPTH - 1 must fit");
    assert_eq!(off, wire.len());
}

// ─── Sanity: literals still match SPEC ───────────────────────────────

#[test]
fn all_eight_codes_match_spec() {
    assert_eq!(DECODE_ERROR, 0x0000_0001);
    assert_eq!(ENCODE_ERROR, 0x0000_0002);
    assert_eq!(INVALID_HANDLE, 0x0000_0003);
    assert_eq!(OOM, 0x0000_0004);
    assert_eq!(HANDSHAKE_MISMATCH, 0x0000_0005);
    assert_eq!(UNKNOWN_FUNCTION, 0x0000_0006);
    assert_eq!(BUFFER_TOO_SMALL, 0x0000_0007);
    assert_eq!(DECODE_DEPTH_EXCEEDED, 0x0000_0008);
}
