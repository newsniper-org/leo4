//! leo4-abi — canonical-ABI encode/decode on the Rust side.
//!
//! Mirrors `lake/Leo4/Leo4/Marshal.lean` + `lake/Leo4/Leo4/Builtins.lean`.
//! Two implementations of [`LeanMarshal`] for the same logical type MUST
//! produce identical bytes on the wire — pinned by `tests/conformance/`.
//!
//! Normative wire format: `SPEC/canonical-abi.md`.
//! Reserved error codes: `SPEC/canonical-abi.md` §13.

#![cfg_attr(feature = "nightly-floats", feature(f16, f128))]

pub mod error;
pub mod marshal;
pub mod scalars;
pub mod composites;
pub mod bignat;
pub mod bigint;
pub mod rat;
pub mod complex;
#[cfg(feature = "nightly-floats")]
pub mod floats_nightly;
pub mod handshake;
pub mod callback;
#[cfg(feature = "rust-exports")]
pub mod rust_exports;
pub mod rust_native;

pub use callback::{
    CallbackInvoker, ErasedRustCallback, LeanCallback, RegistrationGuard,
    RustCallbackRegistry,
};
pub use error::{error_codes, LeanError};
pub use marshal::LeanMarshal;
pub use rust_native::{LeanProc, LeanProcInvoker};
