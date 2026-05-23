//! leo4-oxilean — adapter from leo4's rust-native trait
//! surface (`leo4_abi::{LeanProc, LeanProcInvoker}`) to the
//! [OxiLean](https://github.com/cool-japan/oxilean) Rust-native
//! Lean 4 implementation's FFI surface
//! (`oxilean_kernel::ffi::{ExternRegistry, ExternDecl, …}`).
//!
//! ## Status (2026-05-21)
//!
//! **Partial wiring** against OxiLean v0.1.2. What works:
//!
//! - `ExternRegistry` integration — `OxiLeanInvoker` wraps an
//!   `Arc<Mutex<ExternRegistry>>` and exposes
//!   `register_export(mangled, sig)` that pushes an
//!   `ExternDecl` per `#[leo4::export]` into OxiLean's
//!   registry under `lib_name = "leo4-rust-bridge"`.
//! - Adapter-side `LeanProc` + `LeanProcInvoker` trait
//!   implementations are real; both compile + are object-
//!   safe per `SPEC/rust-native-lean.md` §2 + §3.
//!
//! What's still stub'd, with the architectural reason
//! documented inline:
//!
//! - **`OxiLeanInvoker::invoke(mangled, args)`** returns
//!   `UNKNOWN_FUNCTION` for every input. OxiLean's
//!   `ExternRegistry` (deliberately by design) carries
//!   *metadata* only — the `lib_name` / `symbol_name` fields
//!   describe where the actual symbol lives; resolution is
//!   the responsibility of OxiLean's codegen / evaluator
//!   (typically a `dlsym(libname, symbol)` call). For
//!   `leo4-rust-native`'s in-process direct-Rust-call model
//!   to work, OxiLean would need to expose a *callback-
//!   registration* hook in its evaluator — passing a
//!   `Box<dyn Fn(&[u8]) -> Result<Vec<u8>, LeanError>>` per
//!   mangled name. **Such a hook does not exist in v0.1.2.**
//!   See §"OxiLean upstream prerequisite" in `README.md`.
//! - **`OxiLeanProc::call(mangled, args)`** likewise returns
//!   the not-yet-wired error. Wiring it requires OxiLean's
//!   evaluator to expose an "invoke `@[leo4_export]` body by
//!   name with byte payload" entry point, which is also
//!   absent.
//!
//! ## What lands in this commit
//!
//! 1. Real `ExternRegistry` wrapping (`OxiLeanInvoker`'s
//!    inner state is an actual OxiLean registry, not a stub
//!    `()`).
//! 2. `register_export(mangled, sig)` — does real
//!    `oxilean_kernel::ffi::ExternRegistry::register(decl)`
//!    calls and surfaces the resulting `FfiError` as a
//!    leo4 `LeanError`.
//! 3. Type mapping: leo4 canonical-ABI primitives →
//!    `FfiType` (helper `leo4_args_to_ffi_sig`).
//! 4. Tests that exercise the registration path against the
//!    actual OxiLean v0.1.2 API.
//!
//! ## OxiLean upstream — direct inspection of v0.1.2
//!
//! Three hooks needed for full integration; direct grep
//! into OxiLean v0.1.2 sources verified which exist:
//!
//! 1. Per-mangled-name **callback registration** in the
//!    OxiLean evaluator (the hook this adapter would tie
//!    `LeanProcInvoker::invoke` into). **NOT PRESENT** in
//!    v0.1.2 — `ExternRegistry` + `FunctionTable` both
//!    store metadata only; closure storage path doesn't
//!    exist.
//! 2. **By-name dispatch** of `@[leo4_export]` Lean
//!    definitions (the hook `LeanProc::call` would tie
//!    into). **NOT PRESENT** at high-level API surface —
//!    `Environment` is query-only; runtime side is
//!    `BytecodeChunk`-level (`execute_chunk`), not
//!    name-level.
//! 3. Source-side **`@[leo4_export]` attribute
//!    recognition** + `deriving LeanMarshal` handler
//!    registration (the equivalent of reference Lean's
//!    `registerBuiltinAttribute` /
//!    `registerDerivingHandler`).
//!    **PRESENT** in v0.1.2 —
//!    `oxilean_elab::attribute::AttributeManager::
//!    register_custom_handler` and
//!    `DeriveHandlerRegistry::register`.
//!
//! → Hooks 1 + 2 (dispatch layer) block this adapter's
//! `call` / `invoke` bodies until upstream PRs land.
//! Hook 3 (recognition layer) is unblocked — a separate
//! `leo4-oxilean-build` companion crate (out of scope for
//! this minimal adapter) can ship today by importing
//! `oxilean-elab` and binding both registries. See
//! `README.md` §"OxiLean upstream prerequisite".

#![allow(clippy::missing_errors_doc)]

use std::sync::{Arc, Mutex};

use leo4_abi::{LeanError, LeanProc, LeanProcInvoker};

use oxilean_kernel::ffi::{
    CallingConvention, ExternDecl, ExternRegistry, FfiSafety, FfiSignature, FfiType,
};
use oxilean_kernel::{Expr, Name};

/// Convention: every `#[leo4::export]` becomes an
/// `ExternDecl` under this `lib_name`. Mirrors the
/// `libleo4_rust_bridge.a` namespace from the
/// `leo4-mslean4` reverse-direction pipeline.
pub const LEO4_RUST_BRIDGE_LIB_NAME: &str = "leo4-rust-bridge";

/// Canonical ABI bytes-in / bytes-out signature. leo4's IDL
/// types all serialise to a single contiguous `ByteArray`
/// payload via `LeanMarshal`, so every `#[leo4::export]`'s
/// boundary looks identical at the OxiLean FFI level:
/// `(ByteArray) -> ByteArray`. The leo4 IDL types are
/// preserved inside the bytes; OxiLean doesn't need to
/// type-check them.
#[must_use]
pub fn leo4_canonical_signature() -> FfiSignature {
    FfiSignature::new(vec![FfiType::ByteArray], Box::new(FfiType::ByteArray))
}

/// Map a leo4-mangled body name (the
/// `leo4__pkg__iface__fn__args__hHASH` form) to an OxiLean
/// `Name`. Uses the dotted-segments convention; OxiLean's
/// `Name::str` builds a single-segment name verbatim, which
/// is sufficient because the mangled body already encodes
/// the package + interface + function in one C-identifier-
/// safe string.
fn mangled_to_oxi_name(mangled: &str) -> Name {
    Name::str(mangled)
}

/// `LeanProcInvoker` implementation for OxiLean. Wraps an
/// `oxilean_kernel::ffi::ExternRegistry` and provides bulk
/// registration of `#[leo4::export]` callbacks.
///
/// Thread-safety: the inner `ExternRegistry` is wrapped in
/// `Arc<Mutex<…>>` so multiple host threads can register
/// concurrently. OxiLean's single-Lean-thread invariant
/// (matches reference Lean's, per `LEO4-DESIGN.md` §16)
/// means contention on `invoke` is irrelevant; the mutex is
/// purely for the registration phase.
#[derive(Clone)]
pub struct OxiLeanInvoker {
    registry: Arc<Mutex<ExternRegistry>>,
}

impl std::fmt::Debug for OxiLeanInvoker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.registry.lock().map(|r| r.count()).unwrap_or(0);
        f.debug_struct("OxiLeanInvoker")
            .field("registered_externs", &count)
            .finish()
    }
}

impl Default for OxiLeanInvoker {
    fn default() -> Self {
        Self::new()
    }
}

impl OxiLeanInvoker {
    /// Construct an empty invoker. Pair `register_export`
    /// with each `#[leo4::export]` at adapter init time.
    #[must_use]
    pub fn new() -> Self {
        Self {
            registry: Arc::new(Mutex::new(ExternRegistry::new())),
        }
    }

    /// Construct an invoker that shares an existing OxiLean
    /// registry. Useful when the host process is already
    /// running OxiLean and just needs leo4 exports added.
    #[must_use]
    pub fn with_registry(registry: Arc<Mutex<ExternRegistry>>) -> Self {
        Self { registry }
    }

    /// Register a `#[leo4::export]` mangled body with
    /// OxiLean's `ExternRegistry`. Records the canonical
    /// `(ByteArray) -> ByteArray` signature under
    /// `lib_name = "leo4-rust-bridge"` and the mangled name
    /// as the symbol name.
    ///
    /// Today this **only** populates the metadata
    /// `ExternRegistry`. Once OxiLean upstream grows a
    /// callback-hook entry point (see crate docs), this
    /// method will additionally accept + store the actual
    /// `Box<dyn Fn(&[u8]) -> Result<Vec<u8>, LeanError>>`
    /// closure.
    ///
    /// # Errors
    /// `LeanError` wrapping the underlying `FfiError` if
    /// OxiLean rejects the registration (duplicate symbol,
    /// invalid signature).
    pub fn register_export(&self, mangled: &str) -> Result<(), LeanError> {
        let decl = ExternDecl::new(
            mangled_to_oxi_name(mangled),
            // Lean-side type expression: we don't have the
            // typed Lean signature available at the adapter
            // level (leo4 carries it through schema_hash
            // instead). Use an opaque `String`-typed
            // placeholder; OxiLean's FFI safety check doesn't
            // gate on this field.
            Expr::Const(Name::str("ByteArray"), vec![]),
            LEO4_RUST_BRIDGE_LIB_NAME.to_string(),
            mangled.to_string(),
            FfiSafety::Safe,
            CallingConvention::Rust,
            leo4_canonical_signature(),
        );
        self.registry
            .lock()
            .map_err(|e| {
                LeanError::new(
                    leo4_abi::error::error_codes::OOM,
                    format!("leo4-oxilean: registry mutex poisoned: {e}"),
                )
            })?
            .register(decl)
            .map_err(|e| {
                LeanError::new(
                    leo4_abi::error::error_codes::ENCODE_ERROR,
                    format!(
                        "leo4-oxilean: ExternRegistry::register({mangled}) failed: {e:?}"
                    ),
                )
            })
    }

    /// Number of currently-registered `#[leo4::export]`s.
    /// Diagnostic only.
    #[must_use]
    pub fn registered_count(&self) -> usize {
        self.registry
            .lock()
            .map(|r| r.count())
            .unwrap_or(0)
    }

    /// Access the underlying OxiLean registry for caller-
    /// side wiring (e.g. inserting into an OxiLean
    /// `Environment` once that entry point exists).
    #[must_use]
    pub fn registry_handle(&self) -> Arc<Mutex<ExternRegistry>> {
        self.registry.clone()
    }
}

impl LeanProcInvoker for OxiLeanInvoker {
    fn invoke(&self, mangled: &str, _args: &[u8]) -> Result<Vec<u8>, LeanError> {
        // ExternRegistry::lookup gives us back the ExternDecl
        // (metadata), confirming the export is registered.
        // But OxiLean v0.1.2 doesn't expose a per-call
        // callback hook in the evaluator — there's nowhere to
        // put the actual Rust closure that would handle the
        // call. So even on a successful lookup the dispatch
        // is currently not possible.
        //
        // We surface a distinct error code depending on which
        // half failed: UNKNOWN_FUNCTION if the export wasn't
        // registered, RUST_DLSYM_FAILED (0x0002_0005) if it
        // was registered but the OxiLean-side hook is
        // missing.
        let registry = self.registry.lock().map_err(|e| {
            LeanError::new(
                leo4_abi::error::error_codes::OOM,
                format!("leo4-oxilean: registry mutex poisoned: {e}"),
            )
        })?;
        match registry.lookup(&mangled_to_oxi_name(mangled)) {
            Ok(_decl) => Err(LeanError::new(
                0x0002_0005,
                format!(
                    "leo4-oxilean: `{mangled}` is registered in OxiLean's \
                     ExternRegistry, but OxiLean v0.1.2 has no callback-hook \
                     entry point in its evaluator to dispatch into a host Rust \
                     closure. See `README.md` §\"OxiLean upstream prerequisite\"."
                ),
            )),
            Err(_) => Err(LeanError::unknown_function(mangled)),
        }
    }
}

/// `LeanProc` implementation for OxiLean. Scaffold —
/// `call` is stub'd because OxiLean v0.1.2 has no by-name
/// `@[leo4_export]` dispatch entry point in its evaluator.
/// See crate docs.
#[derive(Debug, Clone)]
pub struct OxiLeanProc {
    schema_hash: String,
    abi_version: u32,
}

impl OxiLeanProc {
    /// Construct an `OxiLeanProc` from a handshake JSON's
    /// pinned fields. Once `OxiLeanProc::call` is wired,
    /// this will additionally take an
    /// `Arc<oxilean_runtime::Env>` (or whatever the public
    /// runtime handle type ends up being).
    #[must_use]
    pub fn new(schema_hash: impl Into<String>, abi_version: u32) -> Self {
        Self {
            schema_hash: schema_hash.into(),
            abi_version,
        }
    }
}

impl LeanProc for OxiLeanProc {
    fn schema_hash(&self) -> &str {
        &self.schema_hash
    }

    fn abi_version(&self) -> u32 {
        self.abi_version
    }

    fn call(&self, mangled: &str, _args: &[u8]) -> Result<Vec<u8>, LeanError> {
        Err(LeanError::new(
            0x0002_0005,
            format!(
                "leo4-oxilean: OxiLeanProc::call({mangled}) is not yet wired. \
                 OxiLean v0.1.2 doesn't expose a by-name `@[leo4_export]` \
                 invocation entry point in its public runtime API. See \
                 `README.md` §\"OxiLean upstream prerequisite\"."
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_signature_is_byte_array_round_trip() {
        let sig = leo4_canonical_signature();
        assert_eq!(sig.params.len(), 1);
        assert_eq!(sig.params[0], FfiType::ByteArray);
        assert_eq!(*sig.ret_type, FfiType::ByteArray);
    }

    #[test]
    fn invoker_registers_one_export_and_counts_it() {
        let inv = OxiLeanInvoker::new();
        assert_eq!(inv.registered_count(), 0);
        inv.register_export("leo4__pkg__iface__fn__u32__habcdefghijklm")
            .expect("first registration");
        assert_eq!(inv.registered_count(), 1);
    }

    #[test]
    fn invoker_rejects_duplicate_export() {
        let inv = OxiLeanInvoker::new();
        inv.register_export("leo4__pkg__iface__double__u32__hsomehashstring")
            .unwrap();
        let err = inv
            .register_export("leo4__pkg__iface__double__u32__hsomehashstring")
            .unwrap_err();
        // OxiLean's ExternRegistry returns FfiError::DuplicateSymbol,
        // which we map to ENCODE_ERROR (0x02) on the leo4 side.
        assert_eq!(err.code, leo4_abi::error::error_codes::ENCODE_ERROR);
        assert!(err.message.contains("double"), "{}", err.message);
    }

    #[test]
    fn invoker_invoke_returns_unknown_for_unregistered() {
        let inv = OxiLeanInvoker::new();
        let err = inv.invoke("nonexistent", &[]).unwrap_err();
        assert_eq!(err.code, leo4_abi::error::error_codes::UNKNOWN_FUNCTION);
    }

    #[test]
    fn invoker_invoke_returns_dlsym_failed_for_registered_but_no_callback_hook() {
        let inv = OxiLeanInvoker::new();
        inv.register_export("leo4__pkg__iface__ping__u32__hping0000ping00")
            .unwrap();
        let err = inv
            .invoke("leo4__pkg__iface__ping__u32__hping0000ping00", &[])
            .unwrap_err();
        assert_eq!(err.code, 0x0002_0005);
        assert!(
            err.message.contains("OxiLean v0.1.2"),
            "{}",
            err.message
        );
    }

    #[test]
    fn proc_constructs_and_reports_handshake_fields() {
        let p = OxiLeanProc::new("qi5gb74dbjyxo", 1);
        assert_eq!(p.schema_hash(), "qi5gb74dbjyxo");
        assert_eq!(p.abi_version(), 1);
    }

    #[test]
    fn proc_and_invoker_are_object_safe() {
        let _: Box<dyn LeanProc> = Box::new(OxiLeanProc::new("x", 1));
        let _: Box<dyn LeanProcInvoker> = Box::new(OxiLeanInvoker::new());
    }

    #[test]
    fn registry_handle_shared_state_across_clones() {
        let inv = OxiLeanInvoker::new();
        let inv2 = inv.clone();
        inv.register_export("leo4__shared__test__one__u32__hsharedone000a")
            .unwrap();
        assert_eq!(inv2.registered_count(), 1);
    }
}
