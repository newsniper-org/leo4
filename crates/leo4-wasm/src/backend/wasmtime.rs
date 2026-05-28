//! `wasmtime`-backed `WasmRuntime`. Default feature
//! (`backend-wasmtime`). Phase 10-C4.x.x landing (2026-05-21):
//! real Component-Model dispatch via wasmtime's untyped
//! `Val`-based API.
//!
//! Why untyped over `wasmtime::component::bindgen!`? leo4 only
//! ever calls one exported function per component
//! (`exports.call(mangled, args)`), and its signature is fixed
//! at the WIT level. Generating typed bindings for that one
//! call buys nothing vs. one manual `Val::List(…)` build +
//! result decode, and lets us avoid invalidating the WIT
//! version every time a leo4 schema rotates.

use wasmtime::component::{Component, Linker, Val};
use wasmtime::{Config, Engine, Store};

use crate::runtime::{WasmComponent, WasmInstance, WasmRuntime};
use crate::LeanError;
use leo4_abi::error::error_codes;

// Reserved Rust-worker passthrough codes from SPEC/canonical-abi.md §13.
// Not in leo4-abi's `error_codes` constants today (host-side bridge
// crates use them directly).
const LEO4_ERR_RUST_PANIC: u32 = 0x0002_0001;
const LEO4_ERR_RUST_SPAWN_FAILED: u32 = 0x0002_0003;
const LEO4_ERR_RUST_DLSYM_FAILED: u32 = 0x0002_0005;

/// Wasmtime-backed runtime. Owns the shared `Engine`; one per
/// process is conventional.
#[derive(Debug, Clone)]
pub struct WasmtimeRuntime {
    engine: Engine,
}

impl WasmtimeRuntime {
    /// Construct a runtime with Component Model + Cranelift JIT
    /// enabled. Cheap to clone (the underlying `Engine` is
    /// internally `Arc`'d).
    ///
    /// # Errors
    /// `LeanError` with `LEO4_ERR_RUST_SPAWN_FAILED` (0x00020003)
    /// if the engine cannot be configured.
    pub fn new() -> Result<Self, LeanError> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        let engine = Engine::new(&config).map_err(|e| {
            LeanError::new(
                LEO4_ERR_RUST_SPAWN_FAILED,
                format!("leo4-wasm wasmtime: engine init failed: {e}"),
            )
        })?;
        Ok(Self { engine })
    }
}

impl Default for WasmtimeRuntime {
    fn default() -> Self {
        Self::new().expect("wasmtime engine must initialise with default Config")
    }
}

impl WasmRuntime for WasmtimeRuntime {
    fn open_component(
        &self,
        bytes: &[u8],
    ) -> Result<Box<dyn WasmComponent>, LeanError> {
        let component = Component::from_binary(&self.engine, bytes).map_err(|e| {
            LeanError::new(
                error_codes::DECODE_ERROR,
                format!("leo4-wasm wasmtime: parse component bytes: {e}"),
            )
        })?;
        Ok(Box::new(WasmtimeComponent {
            engine: self.engine.clone(),
            component,
        }))
    }
}

pub struct WasmtimeComponent {
    engine: Engine,
    component: Component,
}

impl std::fmt::Debug for WasmtimeComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmtimeComponent").finish_non_exhaustive()
    }
}

impl WasmComponent for WasmtimeComponent {
    fn instantiate(&self) -> Result<Box<dyn WasmInstance>, LeanError> {
        let mut store = Store::new(&self.engine, ());

        // Host imports — `leo4:host/host-imports@0.1.0`. Today's
        // WIT exposes just `log`; provide a stderr-printing impl.
        let mut linker: Linker<()> = Linker::new(&self.engine);
        if let Ok(mut iface) =
            linker.instance("leo4:host/host-imports@0.1.0")
        {
            // `log: func(level: u32, msg: string)` — best-effort.
            // We swallow the error of `func_wrap` failure (it'd
            // only fire if the WIT changed away from our shape)
            // and let `instantiate` surface the resulting linker
            // mismatch with a clearer error.
            let _ = iface.func_wrap(
                "log",
                |_store: wasmtime::StoreContextMut<'_, ()>,
                 (level, msg): (u32, String)|
                 -> wasmtime::Result<()> {
                    eprintln!("[leo4-wasm/component lvl={level}] {msg}");
                    Ok(())
                },
            );
        }

        let instance = linker
            .instantiate(&mut store, &self.component)
            .map_err(|e| {
                LeanError::new(
                    LEO4_ERR_RUST_DLSYM_FAILED,
                    format!("leo4-wasm wasmtime: instantiate failed: {e}"),
                )
            })?;
        Ok(Box::new(WasmtimeInstance { store, instance }))
    }
}

pub struct WasmtimeInstance {
    store: Store<()>,
    instance: wasmtime::component::Instance,
}

impl std::fmt::Debug for WasmtimeInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmtimeInstance").finish_non_exhaustive()
    }
}

impl WasmInstance for WasmtimeInstance {
    fn call(
        &mut self,
        mangled: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, LeanError> {
        // Resolve the `exports.call` function. The wasmtime 45 API
        // walks the export hierarchy via `get_export_index`:
        //   1. Look up the `leo4:host/exports@0.1.0` interface index
        //      (top-level — pass `None` as the parent).
        //   2. Look up `call` inside that interface.
        //   3. Materialise the `Func` via `get_func` with the
        //      resolved leaf index.
        let iface_idx = self
            .instance
            .get_export_index(
                &mut self.store,
                None,
                "leo4:host/exports@0.1.0",
            )
            .ok_or_else(|| {
                LeanError::new(
                    LEO4_ERR_RUST_DLSYM_FAILED,
                    "leo4-wasm wasmtime: component is missing \
                     `leo4:host/exports@0.1.0` interface",
                )
            })?;
        let call_idx = self
            .instance
            .get_export_index(&mut self.store, Some(&iface_idx), "call")
            .ok_or_else(|| {
                LeanError::new(
                    LEO4_ERR_RUST_DLSYM_FAILED,
                    "leo4-wasm wasmtime: component's `exports` \
                     interface is missing the `call` function",
                )
            })?;
        let call_fn = self
            .instance
            .get_func(&mut self.store, call_idx)
            .ok_or_else(|| {
                LeanError::new(
                    LEO4_ERR_RUST_DLSYM_FAILED,
                    "leo4-wasm wasmtime: `call` export-index didn't \
                     resolve to a Func",
                )
            })?;

        // Build args: (string, list<u8>).
        let val_args = [
            Val::String(mangled.to_string()),
            Val::List(args.iter().map(|b| Val::U8(*b)).collect()),
        ];
        let mut val_results = [Val::Bool(false)];

        call_fn
            .call(&mut self.store, &val_args, &mut val_results)
            .map_err(|e| {
                LeanError::new(
                    LEO4_ERR_RUST_PANIC,
                    format!("leo4-wasm wasmtime: component trapped during call({mangled}): {e}"),
                )
            })?;
        // wasmtime 45+: post_return is no longer required for the
        // dynamic Val API — the runtime cleans up after each call.

        // Decode `result<list<u8>, lean-error>`.
        match val_results.into_iter().next() {
            Some(Val::Result(Ok(Some(boxed)))) => match *boxed {
                Val::List(list) => {
                    let mut out = Vec::with_capacity(list.len());
                    for v in list {
                        if let Val::U8(b) = v {
                            out.push(b);
                        } else {
                            return Err(LeanError::new(
                                error_codes::DECODE_ERROR,
                                "leo4-wasm wasmtime: ok payload element \
                                 is not u8",
                            ));
                        }
                    }
                    Ok(out)
                }
                _ => Err(LeanError::new(
                    error_codes::DECODE_ERROR,
                    "leo4-wasm wasmtime: ok payload is not list<u8>",
                )),
            },
            Some(Val::Result(Ok(None))) => Ok(Vec::new()),
            Some(Val::Result(Err(Some(boxed)))) => {
                // `lean-error` is a record { code: u32, message: string }
                let (code, message) = decode_lean_error_record(&boxed);
                Err(LeanError::new(code, message))
            }
            Some(Val::Result(Err(None))) => Err(LeanError::new(
                LEO4_ERR_RUST_PANIC,
                "leo4-wasm wasmtime: component returned bare Err()",
            )),
            other => Err(LeanError::new(
                error_codes::DECODE_ERROR,
                format!(
                    "leo4-wasm wasmtime: unexpected call result shape: {other:?}"
                ),
            )),
        }
    }
}

/// Pull `(code, message)` out of a `lean-error` record `Val`.
/// Returns a (`LEO4_ERR_RUST_PANIC`, "<malformed>") fallback if
/// the shape doesn't match.
fn decode_lean_error_record(v: &Val) -> (u32, String) {
    let fallback = (LEO4_ERR_RUST_PANIC, "<malformed lean-error record>".to_string());
    let Val::Record(fields) = v else {
        return fallback;
    };
    let mut code = None;
    let mut message = None;
    for (name, val) in fields {
        match name.as_str() {
            "code" => {
                if let Val::U32(c) = val {
                    code = Some(*c);
                }
            }
            "message" => {
                if let Val::String(s) = val {
                    message = Some(s.clone());
                }
            }
            _ => {}
        }
    }
    match (code, message) {
        (Some(c), Some(m)) => (c, m),
        _ => fallback,
    }
}
