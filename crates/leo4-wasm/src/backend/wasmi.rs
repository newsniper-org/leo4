//! `wasmi`-backed `WasmRuntime` impl. Gated on the `backend-wasmi`
//! feature (opt-in).
//!
//! `wasmi` itself is a pure-Rust interpreter without native
//! Component Model support. The plan was to pair it with
//! `wasm_component_layer` + `wasmi_runtime_layer` to get CM on
//! top of wasmi's core engine; that path is **stalled
//! upstream** as of 2026-05-21:
//!
//! - `wasm_component_layer` (the layer that provides CM on
//!   arbitrary core-wasm engines) has had no commits since
//!   2025-03-03 — over a year.
//! - It pins `wasmtime-environ ^18` as a dep; wasmtime is on
//!   v45 today. Resolving the version conflict isn't a small
//!   patch — it's a serious internal refactor.
//! - The `waclay` fork (`crates.io/crates/waclay`) appears to
//!   be a more-recent attempt but is unproven for production.
//!
//! Until wasmi adds **native** Component Model support OR
//! `wasm_component_layer` resumes upstream maintenance OR
//! `waclay` proves itself, this backend stays a stub. The
//! feature flag stays in place so:
//!
//! 1. `backend::Default` resolution works under
//!    `--no-default-features --features backend-wasmi`.
//! 2. The mutex guard in `lib.rs` continues to enforce
//!    "exactly one backend".
//! 3. The day this gap closes, the wiring is one PR away
//!    (Cargo.toml dep + body of the three trait impls below).
//!
//! Phase 10-C4.x.x (2026-05-21): explicit deferral documented.

use crate::runtime::{WasmComponent, WasmInstance, WasmRuntime};
use crate::LeanError;

/// Wasmi-backed runtime. Zero-sized today; C4.x.x will hold a
/// `wasm_component_layer::Engine<wasmi_runtime_layer::Engine>`.
#[derive(Debug, Default, Clone, Copy)]
pub struct WasmiRuntime;

impl WasmiRuntime {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl WasmRuntime for WasmiRuntime {
    fn open_component(
        &self,
        _bytes: &[u8],
    ) -> Result<Box<dyn WasmComponent>, LeanError> {
        Err(LeanError::new(
            0x0002_0005,
            "leo4-wasm wasmi backend: open_component not yet implemented \
             (Phase 10-C4.x.x pending SPEC/wit/leo4-host.wit design + \
             wasm_component_layer wiring).",
        ))
    }
}

/// Stub component handle. C4.x.x impl wraps
/// `wasm_component_layer::Component`.
#[derive(Debug)]
pub struct WasmiComponent {
    _private: (),
}

impl WasmComponent for WasmiComponent {
    fn instantiate(&self) -> Result<Box<dyn WasmInstance>, LeanError> {
        Err(LeanError::new(
            0x0002_0005,
            "leo4-wasm wasmi backend: instantiate not yet implemented (C4.x.x).",
        ))
    }
}

/// Stub instance handle. C4.x.x impl wraps
/// `wasm_component_layer::Instance` + the typed bindings.
#[derive(Debug)]
pub struct WasmiInstance {
    _private: (),
}

impl WasmInstance for WasmiInstance {
    fn call(
        &mut self,
        mangled: &str,
        _args: &[u8],
    ) -> Result<Vec<u8>, LeanError> {
        Err(LeanError::new(
            0x0002_0005,
            format!(
                "leo4-wasm wasmi backend: call({mangled}) not yet implemented (C4.x.x)."
            ),
        ))
    }
}
