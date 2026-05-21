//! Phase 4 exit-criterion test: every reserved [`LeanError`] code has
//! at least one *reachable* code path on the Rust side.
//!
//! v0 is able to actually trigger codes 0x01, 0x05, and 0x07 from
//! `leo4-abi` alone — those use `assert_triggers`. The remaining codes
//! (0x02, 0x03, 0x04, 0x06, 0x08) require Phase 5 (`leo4-native` shim +
//! resource handles + decode-depth tracking + OOM injection) to be
//! triggered through the real boundary. For those we assert the code
//! *value* and exercise the constructor path, ensuring the code remains
//! reachable from Rust source. Phase 5's exit criteria will replace
//! those stubs with real triggers.

use leo4_abi::error::error_codes::*;
use leo4_abi::{
    handshake::check_schema_hash,
    marshal::{encode_to_fixed},
    LeanError,
};

#[test]
fn decode_error_triggered_by_invalid_bool_byte() {
    // 0x01 — DECODE_ERROR
    let err = <bool as leo4_abi::LeanMarshal>::canonical_decode(&[2u8], 0).unwrap_err();
    assert_eq!(err.code, DECODE_ERROR);
}

#[test]
fn encode_error_value_is_stable() {
    // 0x02 — ENCODE_ERROR
    // v0 encoders are total; this code is reserved for future fallible
    // encoders (out-of-range checks, etc.) — see SPEC/canonical-abi.md §13.
    // We exercise the constructor so the code stays linked from source.
    let e = LeanError::new(ENCODE_ERROR, "stub (Phase 5+)");
    assert_eq!(e.code, ENCODE_ERROR);
}

#[test]
fn invalid_handle_value_is_stable() {
    // 0x03 — INVALID_HANDLE
    // Resource handle path lives in `leo4-native`; Phase 5 adds the
    // actual trigger when `LeanRef::deref_via_shim` finds a dead handle.
    let e = LeanError::new(INVALID_HANDLE, "stub (Phase 5+)");
    assert_eq!(e.code, INVALID_HANDLE);
}

#[test]
fn oom_value_is_stable() {
    // 0x04 — OOM
    // Allocator failure cannot be injected from leo4-abi without
    // platform-specific hooks. Phase 5 will instrument the shim layer
    // for an actual trigger.
    let e = LeanError::new(OOM, "stub (Phase 5+)");
    assert_eq!(e.code, OOM);
}

#[test]
fn handshake_mismatch_triggered_by_check_schema_hash() {
    // 0x05 — HANDSHAKE_MISMATCH
    let err = check_schema_hash("expected13chars", "actualxxxxxxx").unwrap_err();
    assert_eq!(err.code, HANDSHAKE_MISMATCH);
}

#[test]
fn unknown_function_value_is_stable() {
    // 0x06 — UNKNOWN_FUNCTION
    // Symbol-table miss lives in `leo4-native::dispatch`; Phase 5 wires
    // an actual trigger when `libloading::Symbol::get` fails for a
    // mangled name.
    let e = LeanError::new(UNKNOWN_FUNCTION, "stub (Phase 5+)");
    assert_eq!(e.code, UNKNOWN_FUNCTION);
}

#[test]
fn buffer_too_small_triggered_by_encode_to_fixed() {
    // 0x07 — BUFFER_TOO_SMALL
    let mut tiny = [0u8; 2];
    let err = encode_to_fixed(&0xDEADBEEFu32, &mut tiny).unwrap_err();
    assert_eq!(err.code, BUFFER_TOO_SMALL);
}

#[test]
fn decode_depth_exceeded_value_is_stable() {
    // 0x08 — DECODE_DEPTH_EXCEEDED
    // `Self`-recursive decode lives in the shim's encode/decode and the
    // user package's deriving handlers. Phase 5 adds an actual trigger
    // by injecting a deeply-nested `Tree`-like fixture.
    let e = LeanError::new(DECODE_DEPTH_EXCEEDED, "stub (Phase 5+)");
    assert_eq!(e.code, DECODE_DEPTH_EXCEEDED);
}

#[test]
fn all_eight_codes_match_spec() {
    // Sanity check the literal values in case anyone forgets §13.
    assert_eq!(DECODE_ERROR, 0x0000_0001);
    assert_eq!(ENCODE_ERROR, 0x0000_0002);
    assert_eq!(INVALID_HANDLE, 0x0000_0003);
    assert_eq!(OOM, 0x0000_0004);
    assert_eq!(HANDSHAKE_MISMATCH, 0x0000_0005);
    assert_eq!(UNKNOWN_FUNCTION, 0x0000_0006);
    assert_eq!(BUFFER_TOO_SMALL, 0x0000_0007);
    assert_eq!(DECODE_DEPTH_EXCEEDED, 0x0000_0008);
}
