//! Schema-handshake check.
//!
//! Mirrors the contract specified in `SPEC/canonical-abi.md` §15:
//! the caller compares its compiled-in expected schema hash against the
//! shim's reported value; mismatch returns
//! [`error_codes::HANDSHAKE_MISMATCH`] (`0x0000_0005`).
//!
//! In v0 we don't yet have a native shim that publishes a hash via a
//! C entry point. This helper exists so that:
//!   * the error code is reachable through a documented code path;
//!   * Phase 5 (`leo4-mslean4`) can plug in by replacing the
//!     "`shim_hash`" source with a `libloading` symbol lookup.

use crate::error::{error_codes, LeanError};

/// Compare an expected schema hash against the one a shim (or any
/// other source) reports. Both arguments are the 13-character lowercase
/// base32 form emitted by `leo4-idl::Hash::to_base32lc`.
///
/// # Errors
/// Returns a `LeanError` carrying [`error_codes::HANDSHAKE_MISMATCH`]
/// when the hashes differ.  The message includes both values so the
/// caller can surface them.
pub fn check_schema_hash(expected: &str, actual: &str) -> Result<(), LeanError> {
    if expected == actual {
        Ok(())
    } else {
        Err(LeanError::new(
            error_codes::HANDSHAKE_MISMATCH,
            format!("expected schema_hash {expected}, shim reports {actual}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_returns_ok() {
        assert!(check_schema_hash("7vi56qcxzb3xw", "7vi56qcxzb3xw").is_ok());
    }

    #[test]
    fn mismatch_returns_0x05() {
        let err = check_schema_hash("aaaaaaaaaaaaa", "bbbbbbbbbbbbb").unwrap_err();
        assert_eq!(err.code, error_codes::HANDSHAKE_MISMATCH);
        assert!(err.message.contains("aaaaaaaaaaaaa"));
        assert!(err.message.contains("bbbbbbbbbbbbb"));
    }
}
