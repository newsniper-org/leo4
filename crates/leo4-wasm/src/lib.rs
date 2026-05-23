//! leo4-wasm — wasm host runtime (Phase 10-C4 scaffolding, 2026-05-21).
//!
//! Mirrors the public API surface of `crates/leo4-native` so consumers
//! targeting wasm-or-native can `use leo4_wasm::{Lean, LeanError}` /
//! `use leo4_native::{Lean, LeanError}` interchangeably (cfg-gated at
//! the user's call site). The wire-format encode/decode layer is
//! shared via `leo4-abi`; only dispatch differs.
//!
//! ## Scope today
//!
//! - `Lean::open(handshake_path)` parses the handshake JSON, verifies
//!   `abi_version`, captures the schema_hash + target_module fields.
//! - `Lean::schema_hash()` exposes the parsed hash for downstream
//!   `check_schema_hash` calls.
//! - `Lean::call(mangled, args)` is currently a stub returning
//!   `LEO4_ERR_RUST_DLSYM_FAILED` (no wasmtime / component-model
//!   dispatch yet — see C4.x).
//!
//! ## What's deferred to C4.x
//!
//! - Adding `wasmtime` as a dep and instantiating a Component-Model
//!   loader.
//! - Designing and pinning `SPEC/wit/leo4-host.wit` — the interface
//!   describing the canonical-ABI bridge between a Lean-as-wasm
//!   component and the host.
//! - `wit-bindgen` invocation in `build.rs` to materialise the
//!   typed bindings.
//! - Replacing this module's `Lean::call` stub with the real
//!   per-callsite dispatch.
//!
//! The scaffolding shape lets downstream code that's structurally
//! wasm-vs-native cfg-gated compile cleanly today; the actual
//! component loader lands when the WIT design lands.

#![allow(clippy::missing_errors_doc)]

use std::path::Path;

pub use leo4_abi::{error::error_codes, LeanError, LeanMarshal};

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
