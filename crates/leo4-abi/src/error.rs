//! `LeanError` — the wire-level error type returned by encode/decode
//! operations and by handshake checks.
//!
//! Mirrors `lake/Leo4/Leo4/Marshal.lean.LeanError`. Reserved codes per
//! `SPEC/canonical-abi.md` §13.

/// Wire-level error (`SPEC/canonical-abi.md` §13).
///
/// Carried in a serialized buffer by the C shim on a non-zero status.
/// On the Rust side this is the natural error type for `Result<T, _>`
/// across the boundary.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LeanError {
    pub code: u32,
    pub message: String,
    pub backtrace: Option<String>,
}

impl LeanError {
    /// Construct a `LeanError` with a code and a message (no backtrace).
    #[must_use]
    pub fn new(code: u32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            backtrace: None,
        }
    }
}

impl std::fmt::Display for LeanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "leo4 error 0x{:08x}: {}", self.code, self.message)
    }
}

impl std::error::Error for LeanError {}

/// Reserved error codes mirroring `Leo4.LeanError` in Lean.
pub mod error_codes {
    pub const DECODE_ERROR: u32           = 0x0000_0001;
    pub const ENCODE_ERROR: u32           = 0x0000_0002;
    pub const INVALID_HANDLE: u32         = 0x0000_0003;
    pub const OOM: u32                    = 0x0000_0004;
    pub const HANDSHAKE_MISMATCH: u32     = 0x0000_0005;
    pub const UNKNOWN_FUNCTION: u32       = 0x0000_0006;
    pub const BUFFER_TOO_SMALL: u32       = 0x0000_0007;
    pub const DECODE_DEPTH_EXCEEDED: u32  = 0x0000_0008;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_codes_match_spec() {
        // Spec / canonical-abi.md §13 — pinned literal values.
        assert_eq!(error_codes::DECODE_ERROR, 0x0000_0001);
        assert_eq!(error_codes::ENCODE_ERROR, 0x0000_0002);
        assert_eq!(error_codes::INVALID_HANDLE, 0x0000_0003);
        assert_eq!(error_codes::OOM, 0x0000_0004);
        assert_eq!(error_codes::HANDSHAKE_MISMATCH, 0x0000_0005);
        assert_eq!(error_codes::UNKNOWN_FUNCTION, 0x0000_0006);
        assert_eq!(error_codes::BUFFER_TOO_SMALL, 0x0000_0007);
        assert_eq!(error_codes::DECODE_DEPTH_EXCEEDED, 0x0000_0008);
    }
}
