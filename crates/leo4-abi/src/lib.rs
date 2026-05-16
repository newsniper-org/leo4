//! leo4-abi — canonical-ABI encode/decode on the Rust side.
//!
//! Mirrors `lake/Leo4/Leo4/Marshal.lean` + `lake/Leo4/Leo4/Builtins.lean`.
//! Two implementations of [`LeanMarshal`] for the same logical type MUST
//! produce identical bytes on the wire — pinned by `tests/conformance/`.
//!
//! Normative wire format: `SPEC/canonical-abi.md`.
//! Reserved error codes: `SPEC/canonical-abi.md` §13.

pub mod error;
pub mod marshal;
pub mod scalars;
pub mod composites;
pub mod bignat;
pub mod bigint;
pub mod handshake;

pub use error::{error_codes, LeanError};
pub use marshal::LeanMarshal;
