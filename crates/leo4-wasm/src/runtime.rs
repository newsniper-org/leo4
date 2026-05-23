//! `WasmRuntime` trait — backend-neutral abstraction over wasm
//! Component Model runtimes (Phase 10-C4.x, 2026-05-21).
//!
//! Selecting a concrete backend at compile time via Cargo features:
//!
//! - `backend-wasmtime` (default) — uses `wasmtime::component::*`
//!   when implemented in C4.x.x.
//! - `backend-wasmi` (opt-in) — uses `wasmi` + `wasm_component_layer`
//!   for Component Model support (`wasmi` itself doesn't have native
//!   CM; the layer crate provides it on top).
//!
//! Both features may be enabled simultaneously — additive. Set
//! `--no-default-features --features backend-wasmi` to swap.
//!
//! Why a wrapper trait and not `wasm_component_layer` directly?
//! Decision 2026-05-21: leo4 doesn't tie its public API to any
//! single third-party crate's lifecycle. A thin local trait lets
//! us swap backends (or even bypass `wasm_component_layer`) later
//! without churning leo4-wasm's surface.
//!
//! The trait is intentionally **object-safe** — backend-specific
//! types (`wasmtime::component::Component` vs whatever the wasmi
//! path uses) stay behind `Box<dyn …>` returns. Public callers
//! never see them.
//!
//! ## What lands in this commit vs C4.x.x
//!
//! - **This commit (C4.x)**: trait definitions, backend modules
//!   wired to features, stub `unimplemented` impls. The
//!   abstraction surface is real; the backends compile clean.
//! - **C4.x.x**: real impl bodies. Requires `SPEC/wit/leo4-host.wit`
//!   to be designed + pinned first, since that's what
//!   `wit-bindgen` consumes to produce the typed bindings each
//!   backend wraps.

use crate::LeanError;

/// A wasm Component Model runtime. Stateless — holds engine
/// configuration but not per-component state. Implementations
/// MUST be cheap to construct (one per process is typical).
pub trait WasmRuntime: Send + Sync {
    /// Parse + validate wasm component bytes. Returns a loaded
    /// component ready for instantiation.
    ///
    /// # Errors
    /// `LeanError` with `DECODE_ERROR` (0x01) if the bytes don't
    /// parse as a valid component; backend-specific errors get
    /// mapped to host-flavoured `LeanError`s with codes from the
    /// `LEO4_ERR_RUST_*` reserved range (0x00020000–0x0002FFFF).
    fn open_component(
        &self,
        bytes: &[u8],
    ) -> Result<Box<dyn WasmComponent>, LeanError>;
}

/// A parsed + validated component. Can be instantiated 0..N
/// times; each instantiation gets its own linear-memory state.
pub trait WasmComponent: Send + Sync {
    /// Instantiate this component. Imports must be satisfied by
    /// the leo4-host.wit interface (C4.x.x deliverable).
    ///
    /// # Errors
    /// `LeanError` if instantiation fails (import resolution,
    /// linear-memory init, …). C4.x.x will map specific failure
    /// modes onto the reserved leo4-host error codes.
    fn instantiate(&self) -> Result<Box<dyn WasmInstance>, LeanError>;
}

/// A live instance ready to receive leo4-mangled calls.
pub trait WasmInstance: Send + Sync {
    /// Invoke a `leo4_rust__<mangled>` export with
    /// canonical-ABI-encoded args. Returns canonical-ABI-encoded
    /// result bytes; `LeanError` on dispatch or marshalling
    /// failure.
    ///
    /// # Errors
    /// `LEO4_ERR_RUST_DLSYM_FAILED` (0x00020005) if `mangled`
    /// isn't a known export; `LEO4_ERR_RUST_PANIC` (0x00020001)
    /// if the component's export traps mid-call; canonical-ABI
    /// errors propagate from the marshalling layer.
    fn call(
        &mut self,
        mangled: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, LeanError>;
}
