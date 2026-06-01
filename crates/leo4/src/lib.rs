//! leo4 — top-level façade re-exporting the production-side surface.
//!
//! Users typically depend on this crate alone:
//!
//! ```toml
//! [dependencies]
//! leo4 = { path = "..." }
//!
//! [build-dependencies]
//! leo4-build = { path = "..." }
//! ```
//!
//! and pull everything they need out of `leo4::*`:
//!
//! ```ignore
//! use leo4::{Lean, LeanError};
//!
//! fn main() -> Result<(), LeanError> {
//!     let lean = Lean::open(env!("LEO4_SHIM_SO"), env!("LEO4_HANDSHAKE_FILE"))?;
//!     // … leo4_macros wrappers go here once they land (P5-b₂/₃) …
//!     Ok(())
//! }
//! ```
//!
//! The proc-macro (`leo4::import!`) lands in P5-b₂; until then the
//! facade exposes the loader (`leo4_mslean4`) and the canonical-ABI
//! marshalling helpers (`leo4_abi`).

pub use leo4_mslean4::{Arena, Lean, LeanError, LeanRef, LeanResult};

pub use leo4_abi::{bignat, bigint, complex, composites, error_codes, rat, scalars, LeanMarshal};
pub use leo4_abi::bignat::BigNat;
pub use leo4_abi::bigint::BigInt;
pub use leo4_abi::rat::LeanRat;
pub use leo4_abi::complex::{LeanComplexF32x2, LeanComplexF64x2};

/// Nightly-only float carriers. Re-exports the `leo4-abi`
/// `floats_nightly` module gated by the `nightly-floats` feature.
/// Enable via `leo4 = { features = ["nightly-floats"] }` on nightly Rust.
#[cfg(feature = "nightly-floats")]
pub use leo4_abi::floats_nightly;
#[cfg(feature = "nightly-floats")]
pub use leo4_abi::floats_nightly::{
    LeanBF16, LeanComplexBF16x2, LeanComplexF128x2, LeanComplexF16x2,
};

/// Canonical-ABI error type raised by encode / decode (distinct
/// from the loader's [`LeanError`], which carries dispatch /
/// handshake failures). `#[derive(LeanMarshal)]` returns this; the
/// `From<leo4_abi::LeanError> for leo4::LeanError` impl in
/// `leo4_mslean4` lets `?` propagate across both.
pub use leo4_abi::LeanError as AbiError;

/// `leo4::import! { fn add(a: u64, b: u64) -> u64; }` — generate
/// Rust wrappers for `@[leo4_export]` definitions on the Lean side.
/// Supports scalar / composite / multi-instantiation; nominal types
/// arrive in P5-b₃-ii once their `LeanMarshal` is derived.
pub use leo4_macros::import;

/// `#[derive(LeanMarshal)]` — synthesise the canonical-ABI
/// `LeanMarshal` impl for user records / enums / variants /
/// resources. Mirrors `lake/Leo4/Leo4/Deriving.lean` on the Rust
/// side so the same `struct` defined in both languages encodes /
/// decodes byte-identical bytes.
pub use leo4_macros::LeanMarshal;

/// Convenience: encode any `LeanMarshal` value to a `Vec<u8>` in
/// canonical-ABI form. Mirrors `leo4_abi::marshal::encode_to_vec` so
/// downstream code can stay on a single `leo4::*` import path.
#[must_use]
pub fn encode<T: LeanMarshal>(v: &T) -> Vec<u8> {
    leo4_abi::marshal::encode_to_vec(v)
}

/// Convenience: decode a `LeanMarshal` value from a byte slice.
///
/// # Errors
///
/// Surfaces any `leo4_abi::LeanError` raised by the type's
/// `canonical_decode` (malformed wire format, decode-depth exceeded,
/// etc. — `SPEC/canonical-abi.md` §13).
pub fn decode<T: LeanMarshal>(buf: &[u8]) -> Result<T, leo4_abi::LeanError> {
    leo4_abi::marshal::decode_from_slice(buf)
}

/// `#[leo4::export]` — expose a Rust function to Lean (Phase 9
/// reverse direction). Only available when the `rust-exports`
/// feature is enabled on this crate (which in turn enables
/// `leo4-abi/rust-exports` and pulls in `linkme`).
///
/// Build a cdylib with this feature on, then:
///
/// ```ignore
/// #[leo4::export]
/// pub fn solve_smt(formula: String) -> u64 { /* … */ 42 }
///
/// #[leo4::export(isolated)]
/// pub fn run_untrusted(input: Vec<u8>) -> Vec<u8> { /* … */ input }
/// ```
///
/// See `SPEC/reverse-direction.md` for the wire contract.
#[cfg(feature = "rust-exports")]
pub use leo4_macros::export;

/// Implementation-detail re-exports the `#[leo4::export]`
/// proc-macro expands against. Users do not touch this module
/// directly; their hand-written code only sees `leo4::export`.
///
/// The macro emits paths like `::leo4::__private::ExportEntry`
/// and registers entries into `::leo4::__private::EXPORTS`. Both
/// are stable for the macro to refer to, even when the
/// `leo4-abi` re-export shape evolves.
// RC.2 (2026-05-31): unconditional. `#[derive(LeanMarshal)]`
// emits `USER_TYPES` entries at every derive site and
// `#[leo4::export]` emits `EXPORTS` entries; both reach the
// distributed slices via this `__private` re-export. Gating
// on `feature = "rust-exports"` would propagate a
// `unexpected_cfg` lint into every downstream user crate that
// hasn't declared the feature on its own Cargo.toml, so we
// keep the surface always available. The `rust-exports`
// cargo feature itself stays as a no-op alias for backward
// compat (existing user Cargo.toml's `features =
// ["rust-exports"]` lines still resolve).
pub mod __private {
    pub use leo4_abi::rust_exports::{
        CtorEntry, ExportEntry, FieldEntry, UserTypeEntry, UserTypeKind, EXPORTS,
        USER_TYPES,
    };
    // Re-export `linkme` itself so the macro's
    // `#[::linkme::distributed_slice(...)]` path resolves through
    // `leo4` without the user crate adding `linkme` directly.
    pub use linkme;
}
