//! Backend dispatch. Exactly one backend feature is active per
//! build (enforced by the `compile_error!` guards in `lib.rs`):
//!
//! - `backend-wasmtime` (default) — `WasmtimeRuntime`
//! - `backend-wasmi` (opt-in) — `WasmiRuntime`
//!
//! Both backends implement the `runtime::WasmRuntime` trait. The
//! `Default` alias below points at whichever is currently active
//! so callers can write `backend::Default::new()` without caring.

#[cfg(feature = "backend-wasmtime")]
pub mod wasmtime;

#[cfg(feature = "backend-wasmi")]
pub mod wasmi;

/// The active backend, named uniformly. Resolves to
/// `WasmtimeRuntime` on default builds and `WasmiRuntime` on
/// `--no-default-features --features backend-wasmi` builds.
#[cfg(feature = "backend-wasmtime")]
pub use self::wasmtime::WasmtimeRuntime as Default;

#[cfg(feature = "backend-wasmi")]
pub use self::wasmi::WasmiRuntime as Default;
