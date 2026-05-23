//! leo4-wasm — wasm host runtime (Phase 10-C4 + C4.x landings, 2026-05-21).
//!
//! Mirrors the public API surface of `crates/leo4-native` so consumers
//! targeting wasm-or-native can `use leo4_wasm::{Lean, LeanError}` /
//! `use leo4_native::{Lean, LeanError}` interchangeably (cfg-gated at
//! the user's call site). The wire-format encode/decode layer is
//! shared via `leo4-abi`; only dispatch differs.
//!
//! ## Scope today (C4 + C4.x)
//!
//! - `Lean::open(handshake_path)` parses the handshake JSON, verifies
//!   `abi_version`, captures the schema_hash + target_module fields.
//! - `Lean::schema_hash()` / `target_module()` / `abi_version()`
//!   getters.
//! - `Lean::call(mangled, args)` returns `LEO4_ERR_RUST_DLSYM_FAILED`
//!   stub.
//! - **`runtime::WasmRuntime` + `WasmComponent` + `WasmInstance`
//!   traits** (C4.x): backend-neutral abstraction over the wasm
//!   Component Model. Two feature-gated backends:
//!   `backend-wasmtime` (default) and `backend-wasmi` (opt-in).
//! - **`SPEC/wit/leo4-host.wit`** (C4.x): the Component Model
//!   interface that both backends wrap. Pinned at version
//!   `leo4:host@0.1.0`.
//!
//! ## Backend feature mutex (safety guard)
//!
//! Exactly one backend feature MUST be enabled. The two
//! `compile_error!`s below reject `--no-default-features` without
//! an explicit alternative AND reject simultaneously enabling
//! both backends (which would make the `backend::Default` alias
//! ambiguous and bloat binaries with two CM runtimes). Rationale:
//! `crates/leo4-wasm` is "one wasm runtime per build" by design
//! — testing multiple backends in one process is an advanced
//! workflow that belongs in downstream code, not the leo4-wasm
//! crate itself.
//!
//! ## What's deferred to C4.x.x
//!
//! - `wit-bindgen` invocation in `build.rs` for typed bindings.
//! - Replacing each backend's stub impls with real loader +
//!   dispatch via the pinned WIT.
//! - Optional: real cdylib build of a Lean module (out of the
//!   existing `leanc` invocation chain) into a wasm component.

#![allow(clippy::missing_errors_doc)]

// ─── Backend feature mutex (build-time safety guard) ──────────────
//
// Exactly one of `backend-wasmtime` / `backend-wasmi` must be
// active. See module docs above for rationale.

#[cfg(not(any(feature = "backend-wasmtime", feature = "backend-wasmi")))]
compile_error!(
    "leo4-wasm requires exactly one wasm backend feature, but none is enabled.\n\
     If you disabled default features, re-add `default-features = true`,\n\
     or opt in explicitly: `features = [\"backend-wasmtime\"]` or `[\"backend-wasmi\"]`."
);

#[cfg(all(feature = "backend-wasmtime", feature = "backend-wasmi"))]
compile_error!(
    "leo4-wasm requires exactly ONE wasm backend feature, but both\n\
     `backend-wasmtime` and `backend-wasmi` are active.\n\
     Set `default-features = false` and pick one explicitly."
);

use std::path::Path;

pub use leo4_abi::{error::error_codes, LeanError, LeanMarshal};

pub mod runtime;
pub mod backend;

/// Wasm-host counterpart of `leo4_native::Lean`. Owns the handshake
/// metadata + (eventually) a wasmtime `Engine` + `Store`.
#[derive(Debug, Clone)]
pub struct Lean {
    schema_hash: String,
    target_module: String,
    abi_version: u32,
}

impl Lean {
    /// Open a leo4 wasm component identified by its handshake JSON.
    ///
    /// Today: parses + validates the handshake, returns a `Lean`
    /// holding the metadata. Calls via `call` return a
    /// "not yet implemented" error.
    ///
    /// C4.x: a `component_path` arg will be added (or the handshake
    /// will be extended with a `wasm_component_path` field), and
    /// `Lean::open` will lazily instantiate the wasm module via
    /// wasmtime's Component Model.
    ///
    /// # Errors
    /// `LeanError` with code `DECODE_ERROR` if the handshake JSON
    /// can't be parsed, or `HANDSHAKE_MISMATCH` if its
    /// `abi_version` doesn't match the runtime's expectation.
    pub fn open(handshake_path: impl AsRef<Path>) -> Result<Self, LeanError> {
        let path = handshake_path.as_ref();
        let raw = std::fs::read_to_string(path).map_err(|e| {
            LeanError::new(
                error_codes::DECODE_ERROR,
                format!("leo4-wasm: read {}: {e}", path.display()),
            )
        })?;
        let json: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
            LeanError::new(
                error_codes::DECODE_ERROR,
                format!("leo4-wasm: parse {}: {e}", path.display()),
            )
        })?;
        let obj = json.as_object().ok_or_else(|| {
            LeanError::new(
                error_codes::DECODE_ERROR,
                format!("leo4-wasm: {}: top-level not an object", path.display()),
            )
        })?;
        let schema_hash = obj
            .get("schema_hash")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                LeanError::new(
                    error_codes::DECODE_ERROR,
                    "leo4-wasm: handshake missing `schema_hash`",
                )
            })?
            .to_string();
        let target_module = obj
            .get("target_module")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let abi_version_u64 = obj
            .get("abi_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                LeanError::new(
                    error_codes::DECODE_ERROR,
                    "leo4-wasm: handshake missing or malformed `abi_version`",
                )
            })?;
        let abi_version = u32::try_from(abi_version_u64).map_err(|_| {
            LeanError::new(
                error_codes::HANDSHAKE_MISMATCH,
                format!("leo4-wasm: abi_version {abi_version_u64} exceeds u32"),
            )
        })?;
        if abi_version != 1 {
            return Err(LeanError::new(
                error_codes::HANDSHAKE_MISMATCH,
                format!(
                    "leo4-wasm: abi_version {abi_version} unsupported (expected 1)"
                ),
            ));
        }
        Ok(Self {
            schema_hash,
            target_module,
            abi_version,
        })
    }

    /// The handshake's `schema_hash` (13-char base32lc).
    #[must_use]
    pub fn schema_hash(&self) -> &str {
        &self.schema_hash
    }

    /// The handshake's `target_module` (e.g. `"Sample"`).
    #[must_use]
    pub fn target_module(&self) -> &str {
        &self.target_module
    }

    /// `abi_version` from the handshake. v0 expects `1`.
    #[must_use]
    pub fn abi_version(&self) -> u32 {
        self.abi_version
    }

    /// Dispatch a call to a mangled symbol.
    ///
    /// **Stub today.** Returns `LEO4_ERR_RUST_DLSYM_FAILED` regardless
    /// of input — the real wasmtime / component-model dispatch lands
    /// in C4.x after the host-import WIT is pinned.
    ///
    /// # Errors
    /// Always returns `LEO4_ERR_RUST_DLSYM_FAILED` with a "not yet
    /// implemented" message.
    #[allow(clippy::unused_self)]
    pub fn call(
        &self,
        mangled: &str,
        _args: &[u8],
    ) -> Result<Vec<u8>, LeanError> {
        Err(LeanError::new(
            // Reuse the reverse-direction's DLSYM_FAILED code as the
            // closest semantic match for "the dispatch layer couldn't
            // resolve this symbol". A dedicated wasm-side code may
            // get reserved in C4.x.
            0x0002_0005,
            format!(
                "leo4-wasm: dispatch for {mangled} not yet implemented \
                 (Phase 10-C4 scaffolding; wasmtime + Component-Model \
                 loader lands in C4.x after `SPEC/wit/leo4-host.wit` is pinned)"
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp(json: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "leo4-wasm-test-{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&p, json).unwrap();
        p
    }

    #[test]
    fn open_parses_minimal_handshake() {
        let p = write_temp(
            r#"{"schema_hash":"abc","target_module":"Sample","abi_version":1}"#,
        );
        let lean = Lean::open(&p).expect("open");
        assert_eq!(lean.schema_hash(), "abc");
        assert_eq!(lean.target_module(), "Sample");
        assert_eq!(lean.abi_version(), 1);
    }

    #[test]
    fn open_rejects_wrong_abi_version() {
        let p = write_temp(
            r#"{"schema_hash":"abc","target_module":"S","abi_version":2}"#,
        );
        let err = Lean::open(&p).unwrap_err();
        assert_eq!(err.code, error_codes::HANDSHAKE_MISMATCH);
    }

    #[test]
    fn open_errors_on_missing_file() {
        let err = Lean::open("/no/such/file/at/all.json").unwrap_err();
        assert_eq!(err.code, error_codes::DECODE_ERROR);
    }

    /// Minimal valid Component Model binary: just the 8-byte
    /// magic + component-header version words. Produced by
    /// `echo '(component)' | wasm-tools parse`. Used as a
    /// "smoke test" fixture — opens cleanly, instantiates with
    /// no imports needed, but has no `exports` interface so
    /// `call(…)` returns DLSYM_FAILED.
    const EMPTY_COMPONENT_BYTES: &[u8] = &[
        0x00, 0x61, 0x73, 0x6d, // \0asm magic
        0x0d, 0x00, 0x01, 0x00, // component encoding header (version 0x000d 0001)
    ];

    #[cfg(feature = "backend-wasmtime")]
    #[test]
    fn wasmtime_loads_empty_component_call_fails_dlsym() {
        use crate::backend::wasmtime::WasmtimeRuntime;
        use crate::runtime::WasmRuntime as _;
        let rt = WasmtimeRuntime::new().expect("engine init");
        let component = rt
            .open_component(EMPTY_COMPONENT_BYTES)
            .expect("empty component parses");
        let mut instance = match component.instantiate() {
            Ok(i) => i,
            Err(e) => panic!("empty component must instantiate: {e:?}"),
        };
        let err = match instance.call("foo", &[]) {
            Ok(_) => panic!("empty component has no exports; call must fail"),
            Err(e) => e,
        };
        // No `leo4:host/exports@0.1.0` interface in an empty
        // component → DLSYM_FAILED.
        assert_eq!(err.code, 0x0002_0005);
    }

    #[test]
    fn backend_default_open_rejects_empty_bytes() {
        // Empty input is not a valid wasm component; the wasmtime
        // backend's `Component::from_binary` rejects it with a
        // parse error (mapped to DECODE_ERROR). The wasmi backend
        // (when wired) is expected to do the same. Either way,
        // open_component must NOT return Ok on `&[]`.
        use crate::runtime::WasmRuntime as _;
        let rt = crate::backend::Default::default();
        let err = match rt.open_component(&[]) {
            Ok(_) => panic!("backend must reject empty bytes"),
            Err(e) => e,
        };
        // wasmtime: DECODE_ERROR; wasmi stub: 0x0002_0005.
        assert!(
            err.code == error_codes::DECODE_ERROR || err.code == 0x0002_0005,
            "unexpected error code: {:#010x}",
            err.code
        );
    }

    #[test]
    fn call_returns_dlsym_failed_stub() {
        let p = write_temp(
            r#"{"schema_hash":"abc","target_module":"S","abi_version":1}"#,
        );
        let lean = Lean::open(&p).unwrap();
        let err = lean.call("leo4__pkg__iface__fn__u32__habcdefghijklm", &[]).unwrap_err();
        assert_eq!(err.code, 0x0002_0005);
    }
}
