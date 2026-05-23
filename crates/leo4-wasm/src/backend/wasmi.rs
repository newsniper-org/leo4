//! `wasmi`-backed `WasmRuntime` impl. Gated on the `backend-wasmi`
//! feature (opt-in).
//!
//! `wasmi` itself is a pure-Rust interpreter without native
//! Component Model support; the C4.x.x impl will pair it with
//! `wasm_component_layer` (which exposes `wasmi` as a CM backend
//! through `wasmi_runtime_layer`). Choosing this backend trades
//! wasmtime's JIT/AOT performance for a much smaller binary +
//! easier portability (no Cranelift / LLVM dep, no
//! mmap-of-executable-pages requirement on the host).
//!
//! Phase 10-C4.x scaffolding (2026-05-21): trait impls return
//! `LEO4_ERR_RUST_DLSYM_FAILED` until the real impls land in
//! C4.x.x alongside `SPEC/wit/leo4-host.wit`.

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
