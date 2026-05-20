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
//! facade exposes the loader (`leo4_native`) and the canonical-ABI
//! marshalling helpers (`leo4_abi`).

pub use leo4_native::{Arena, Lean, LeanError, LeanRef, LeanResult};

pub use leo4_abi::{bignat, bigint, composites, error_codes, rat, scalars, LeanMarshal};
pub use leo4_abi::bignat::BigNat;
pub use leo4_abi::bigint::BigInt;
pub use leo4_abi::rat::LeanRat;

/// Canonical-ABI error type raised by encode / decode (distinct
/// from the loader's [`LeanError`], which carries dispatch /
/// handshake failures). `#[derive(LeanMarshal)]` returns this; the
/// `From<leo4_abi::LeanError> for leo4::LeanError` impl in
/// `leo4_native` lets `?` propagate across both.
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
