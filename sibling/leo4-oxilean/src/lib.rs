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
    CallbackRegistry, CallingConvention, ExternCallError, ExternCallback, ExternDecl,
    ExternRegistry, FfiSafety, FfiSignature, FfiType,
};
use oxilean_kernel::{Expr, Name};
use oxilean_runtime::extern_resolver::{ExternResolver, SharedExternResolver};

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
    /// OX8.3c (2026-05-28): the fork-side
    /// `CallbackRegistry` storing per-symbol Rust
    /// closures. `OxiLeanInvoker::invoke` (+ via
    /// `ExternResolver`, the runtime's
    /// `dispatch_extern_const` helper) reads from here
    /// after the metadata-side `ExternRegistry` lookup
    /// confirms the symbol is registered.
    callbacks: Arc<Mutex<CallbackRegistry>>,
    /// P0b #75 step 3 (2026-05-28): outbound callback
    /// dispatch — when the OxiLean evaluator encounters a
    /// `callback_id` minted by a `leo4::import!`-emitted
    /// wrapper, [`Self::resolve`]'s extension path looks
    /// the id up here and invokes the registered Rust
    /// closure. Empty by default; the host attaches the
    /// `Lean::callback_registry()` Arc via
    /// [`Self::attach_outbound_registry`] before driving
    /// the evaluator.
    outbound: Arc<Mutex<Option<Arc<leo4_abi::RustCallbackRegistry>>>>,
}

impl std::fmt::Debug for OxiLeanInvoker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.registry.lock().map(|r| r.count()).unwrap_or(0);
        let cb_count = self.callbacks.lock().map(|c| c.len()).unwrap_or(0);
        f.debug_struct("OxiLeanInvoker")
            .field("registered_externs", &count)
            .field("registered_callbacks", &cb_count)
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
            callbacks: Arc::new(Mutex::new(CallbackRegistry::new())),
            outbound: Arc::new(Mutex::new(None)),
        }
    }

    /// Construct an invoker that shares an existing OxiLean
    /// registry. Useful when the host process is already
    /// running OxiLean and just needs leo4 exports added.
    #[must_use]
    pub fn with_registry(registry: Arc<Mutex<ExternRegistry>>) -> Self {
        Self {
            registry,
            callbacks: Arc::new(Mutex::new(CallbackRegistry::new())),
            outbound: Arc::new(Mutex::new(None)),
        }
    }

    /// P0b #75 step 3 (2026-05-28) — attach the `Lean`
    /// instance's outbound `RustCallbackRegistry`. The
    /// macro-emitted `leo4::import!` wrapper mints
    /// `callback_id`s into that registry on each boundary
    /// call (step 2); this method tells the invoker where
    /// to find the registry when the OxiLean evaluator
    /// fires the receiving end.
    ///
    /// Idempotent — re-attaching replaces the previous
    /// reference. Embedders typically call this once at
    /// adapter init time with `lean.callback_registry().clone()`.
    pub fn attach_outbound_registry(
        &self,
        registry: Arc<leo4_abi::RustCallbackRegistry>,
    ) {
        let mut slot = self.outbound.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(registry);
    }

    /// P0b #75 step 3 — get the attached outbound registry
    /// (if any). Used by the evaluator-side callback
    /// dispatch path when it observes a `callback_id`
    /// reduction during `IO.bind` walking. Returns `None`
    /// when no host attached one; in that case the
    /// evaluator falls back to the existing inbound
    /// `CallbackRegistry` (OX8.3a) which has no entry for
    /// the id and the boundary call surfaces an explicit
    /// "callback dispatch not wired" error.
    #[must_use]
    pub fn outbound_registry(
        &self,
    ) -> Option<Arc<leo4_abi::RustCallbackRegistry>> {
        self.outbound
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// P0b #75 step 3 — dispatch a `callback_id` minted
    /// outbound by the macro layer (step 2). Looks up the
    /// id in the attached outbound registry; if no
    /// registry is attached, or no callback is registered
    /// for the id, returns an explicit `LeanError` instead
    /// of falling through silently.
    ///
    /// Intended caller: the evaluator-side IO walker
    /// (`#76`), via the `dispatch_extern_const` extension
    /// pathway, when the elaborated `main : IO Unit` body
    /// reaches a Lean closure dereference whose
    /// `callback_id` was minted by a forward-direction
    /// import wrapper.
    ///
    /// # Errors
    /// - `0x0002_0005` if no outbound registry is attached.
    /// - `INVALID_HANDLE` if the registry has no entry for
    ///   the id.
    /// - Propagates whatever `LeanError` the registered
    ///   callback itself returns.
    pub fn invoke_outbound(
        &self,
        callback_id: u64,
        args: &[u8],
    ) -> Result<Vec<u8>, LeanError> {
        let registry_ref = self.outbound_registry().ok_or_else(|| {
            LeanError::new(
                0x0002_0005,
                "leo4-oxilean: no outbound RustCallbackRegistry attached. \
                 Call `OxiLeanInvoker::attach_outbound_registry(...)` at \
                 adapter init time with `lean.callback_registry().clone()`."
                    .to_string(),
            )
        })?;
        registry_ref.invoke(callback_id, args)
    }

    /// P0b #75 step 3 + #76 IO walker integration
    /// (2026-05-29) — register a *bridge* callback for
    /// `<mangled>` that the OxiLean evaluator hits when
    /// the user's Lean code dereferences a Rust closure
    /// passed in via a `leo4::import!`-emitted wrapper.
    ///
    /// The bridge's body unpacks the args as
    /// `(callback_id: u64, rest: &[u8])` — first 8 bytes
    /// little-endian — and forwards to
    /// [`Self::invoke_outbound`]. This is the missing
    /// piece between "Lean closure invoked inside the IO
    /// action" and "Rust closure registered in the
    /// outbound `RustCallbackRegistry`".
    ///
    /// Pair with [`Self::register_export`] for the same
    /// `mangled` (so the evaluator's metadata side knows
    /// about the symbol) before driving the walker. The
    /// `mangled` value is the symbol the wrapper Lean
    /// source's `@[extern "<mangled>"]` clause names.
    ///
    /// # Errors
    /// Same as [`Self::register_export_callback`]: mutex
    /// poisoning surfaces as `OOM` with a clear message;
    /// successful registration cannot fail today.
    pub fn register_outbound_dispatch_callback(
        &self,
        mangled: &str,
    ) -> Result<(), LeanError> {
        let outbound_slot = self.outbound.clone();
        let mangled_owned = mangled.to_string();
        self.register_export_callback(mangled, move |args| {
            // Unpack callback_id + rest.
            if args.len() < 8 {
                return Err(oxilean_kernel::ffi::ExternCallError::CallbackFailed(
                    format!(
                        "leo4-oxilean outbound bridge `{mangled_owned}`: \
                         args shorter than 8 bytes (need callback_id u64 LE \
                         prefix); have {} bytes",
                        args.len()
                    ),
                ));
            }
            let mut id_bytes = [0u8; 8];
            id_bytes.copy_from_slice(&args[..8]);
            let callback_id = u64::from_le_bytes(id_bytes);
            let rest = &args[8..];

            let slot = outbound_slot.lock().unwrap_or_else(|e| e.into_inner());
            let registry = slot.clone().ok_or_else(|| {
                oxilean_kernel::ffi::ExternCallError::CallbackFailed(
                    format!(
                        "leo4-oxilean outbound bridge `{mangled_owned}`: no \
                         outbound RustCallbackRegistry attached; call \
                         `attach_outbound_registry` at adapter init."
                    ),
                )
            })?;
            drop(slot);

            registry
                .invoke(callback_id, rest)
                .map_err(|e| {
                    oxilean_kernel::ffi::ExternCallError::CallbackFailed(
                        format!(
                            "leo4-oxilean outbound bridge `{mangled_owned}` \
                             (callback_id={callback_id}): {e}"
                        ),
                    )
                })
        })
    }

    /// OX8.3c (2026-05-28) — register a Rust closure as
    /// the runtime callback for a previously-registered
    /// `#[leo4::export]` mangled body. Pair with
    /// [`Self::register_export`]: the former records
    /// metadata for the OxiLean evaluator to find the
    /// symbol; this one supplies the actual Rust
    /// function the evaluator calls (via
    /// `dispatch_extern_const`).
    ///
    /// The closure receives canonical-ABI byte buffers
    /// (encoded args in, encoded return out) — same
    /// shape as the forward path's `leo4-rust-bridge`
    /// dispatcher uses.
    ///
    /// # Errors
    /// `LeanError` if the callback registry mutex is
    /// poisoned. (Successful registration cannot fail
    /// today; future "duplicate registration" rejection
    /// can.)
    pub fn register_export_callback<F>(
        &self,
        mangled: &str,
        cb: F,
    ) -> Result<(), LeanError>
    where
        F: Fn(&[u8]) -> Result<Vec<u8>, ExternCallError> + Send + Sync + 'static,
    {
        let boxed: ExternCallback = Box::new(cb);
        self.callbacks
            .lock()
            .map_err(|e| {
                LeanError::new(
                    leo4_abi::error::error_codes::OOM,
                    format!("leo4-oxilean: callback registry mutex poisoned: {e}"),
                )
            })?
            .register(LEO4_RUST_BRIDGE_LIB_NAME, mangled, boxed);
        Ok(())
    }

    /// Diagnostic: number of registered callbacks. Pairs
    /// with [`Self::registered_count`] which counts
    /// metadata entries.
    #[must_use]
    pub fn registered_callbacks(&self) -> usize {
        self.callbacks.lock().map(|c| c.len()).unwrap_or(0)
    }

    /// Wrap this invoker as a shared `ExternResolver` for
    /// installation into an OxiLean evaluator's resolver
    /// slot. Use this when a host process drives an
    /// OxiLean evaluator that needs to dispatch
    /// `@[extern]` decls into leo4-registered callbacks.
    #[must_use]
    pub fn as_shared_resolver(self: Arc<Self>) -> SharedExternResolver {
        self
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
    fn invoke(&self, mangled: &str, args: &[u8]) -> Result<Vec<u8>, LeanError> {
        // OX8.3c (2026-05-28): the prior stub returned
        // `RUST_DLSYM_FAILED` because the fork's evaluator
        // had no callback-dispatch entry. With OX8.3a's
        // `CallbackRegistry` + OX8.3b's
        // `dispatch_extern_const` landed, we now:
        //   1. Confirm the metadata `ExternRegistry` carries
        //      this mangled symbol (registration was paired).
        //   2. Look up the runtime callback in
        //      `CallbackRegistry` and invoke it with the
        //      canonical-ABI byte buffer.
        //
        // `UNKNOWN_FUNCTION` still fires when metadata
        // registration is missing — the caller forgot
        // `register_export(mangled)` upstream of
        // `register_export_callback`. `RUST_DLSYM_FAILED`
        // now means metadata exists but no callback was
        // registered for it (rare; usually indicates a
        // build-time bug).
        let registry = self.registry.lock().map_err(|e| {
            LeanError::new(
                leo4_abi::error::error_codes::OOM,
                format!("leo4-oxilean: registry mutex poisoned: {e}"),
            )
        })?;
        if registry.lookup(&mangled_to_oxi_name(mangled)).is_err() {
            return Err(LeanError::unknown_function(mangled));
        }
        drop(registry);
        let callbacks = self.callbacks.lock().map_err(|e| {
            LeanError::new(
                leo4_abi::error::error_codes::OOM,
                format!("leo4-oxilean: callback registry mutex poisoned: {e}"),
            )
        })?;
        match callbacks.invoke(LEO4_RUST_BRIDGE_LIB_NAME, mangled, args) {
            Ok(bytes) => Ok(bytes),
            Err(ExternCallError::NotRegistered { lib, symbol }) => {
                Err(LeanError::new(
                    0x0002_0005,
                    format!(
                        "leo4-oxilean: `{symbol}` is registered as metadata in \
                         OxiLean's ExternRegistry (lib `{lib}`), but no Rust \
                         callback has been installed via \
                         `register_export_callback`. Pair the two register \
                         calls at adapter init time."
                    ),
                ))
            }
            Err(ExternCallError::CallbackFailed(msg)) => {
                Err(LeanError::new(
                    leo4_abi::error::error_codes::ENCODE_ERROR,
                    format!("leo4-oxilean: callback `{mangled}` failed: {msg}"),
                ))
            }
        }
    }
}

impl ExternResolver for OxiLeanInvoker {
    /// OX8.3c (2026-05-28) — the fork-side
    /// `dispatch_extern_const` helper calls this when the
    /// evaluator reduces a `@[extern]`-attributed `Const`.
    /// We bridge from OxiLean's `Name` to leo4's mangled-
    /// string convention (`mangled_to_oxi_name` is the
    /// inverse) and dispatch through the callback
    /// registry.
    fn resolve(
        &self,
        decl_name: &Name,
        args: &[u8],
    ) -> Result<Vec<u8>, ExternCallError> {
        // Reconstruct the leo4-side mangled symbol from
        // the OxiLean `Name`. `mangled_to_oxi_name`
        // converts mangled → OxiLean name; the reverse is
        // the kernel-name's flattened string form.
        let symbol = oxi_name_to_mangled(decl_name);
        let callbacks =
            self.callbacks.lock().map_err(|_| ExternCallError::CallbackFailed(
                "leo4-oxilean: callback registry mutex poisoned".to_string(),
            ))?;
        callbacks.invoke(LEO4_RUST_BRIDGE_LIB_NAME, &symbol, args)
    }
}

/// Reverse of `mangled_to_oxi_name`. The latter builds an
/// OxiLean `Name` whose flattened dot-joined form equals
/// the leo4 mangled string; this fn pulls that string back
/// out.
fn oxi_name_to_mangled(n: &Name) -> String {
    // OxiLean's `Name` has a `to_string()` impl that
    // dot-joins all segments. Our `mangled_to_oxi_name`
    // single-segments the input via `Name::str`, so the
    // round-trip is verbatim.
    n.to_string()
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
    fn invoker_invoke_returns_dlsym_failed_when_metadata_registered_but_callback_missing() {
        // OX8.3c (2026-05-28): the prior stub returned
        // `RUST_DLSYM_FAILED` because OxiLean v0.1.2 had no
        // callback-hook entry. Now the callback registry
        // exists; this test verifies the "metadata
        // registered but caller forgot to install the Rust
        // closure" path still reports `0x0002_0005` (with a
        // refined error message that points at
        // `register_export_callback`).
        let inv = OxiLeanInvoker::new();
        inv.register_export("leo4__pkg__iface__ping__u32__hping0000ping00")
            .unwrap();
        let err = inv
            .invoke("leo4__pkg__iface__ping__u32__hping0000ping00", &[])
            .unwrap_err();
        assert_eq!(err.code, 0x0002_0005);
        assert!(
            err.message.contains("register_export_callback"),
            "error must point at the missing-callback step: {}",
            err.message
        );
    }

    #[test]
    fn invoker_register_export_callback_and_dispatch_round_trip() {
        // OX8.3c (2026-05-28) — pairing
        // `register_export` (metadata) +
        // `register_export_callback` (runtime closure)
        // lets `invoke` actually call the closure and
        // return the encoded bytes.
        let inv = OxiLeanInvoker::new();
        let mangled = "leo4__pkg__iface__echo__L_u8_l__hecho0000echo00";
        inv.register_export(mangled).unwrap();
        inv.register_export_callback(mangled, |args| {
            // Trivial echo: return the input byte buffer.
            Ok(args.to_vec())
        })
        .unwrap();
        let out = inv.invoke(mangled, &[1, 2, 3, 4]).unwrap();
        assert_eq!(out, vec![1, 2, 3, 4]);
    }

    #[test]
    fn invoker_callback_error_propagates_as_encode_error() {
        // The closure's `Err(CallbackFailed)` surfaces as
        // a leo4 `LeanError(ENCODE_ERROR)` with the
        // closure's message embedded.
        let inv = OxiLeanInvoker::new();
        let mangled = "leo4__pkg__iface__fail__u8__hfail0000fail00";
        inv.register_export(mangled).unwrap();
        inv.register_export_callback(mangled, |_args| {
            Err(ExternCallError::CallbackFailed("custom bug".to_string()))
        })
        .unwrap();
        let err = inv.invoke(mangled, &[]).unwrap_err();
        assert_eq!(err.code, leo4_abi::error::error_codes::ENCODE_ERROR);
        assert!(err.message.contains("custom bug"), "{}", err.message);
    }

    #[test]
    fn invoker_extern_resolver_impl_routes_through_callback_registry() {
        // OX8.3c verifying the `impl ExternResolver`
        // path — when the OxiLean evaluator (via
        // `dispatch_extern_const`) calls `resolve` with a
        // `Name` matching a registered mangled symbol,
        // the right callback fires.
        let inv = OxiLeanInvoker::new();
        let mangled = "leo4__pkg__iface__id__u64__hid000000id0000";
        inv.register_export(mangled).unwrap();
        inv.register_export_callback(mangled, |args| Ok(args.to_vec()))
            .unwrap();
        // The OxiLean evaluator passes Name; here we
        // synthesise the same `Name::str(mangled)` form
        // that `mangled_to_oxi_name` produces.
        let name = mangled_to_oxi_name(mangled);
        let out = <OxiLeanInvoker as ExternResolver>::resolve(
            &inv,
            &name,
            &[42],
        )
        .unwrap();
        assert_eq!(out, vec![42]);
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

    // ─── #75 step 3: outbound callback dispatch ──────────

    #[test]
    fn outbound_registry_default_none() {
        let inv = OxiLeanInvoker::new();
        assert!(inv.outbound_registry().is_none());
    }

    #[test]
    fn outbound_registry_attach_and_get_round_trips() {
        let inv = OxiLeanInvoker::new();
        let reg: Arc<leo4_abi::RustCallbackRegistry> =
            Arc::new(leo4_abi::RustCallbackRegistry::new());
        inv.attach_outbound_registry(reg.clone());
        let got = inv.outbound_registry().expect("attached");
        // Same Arc — pointer equality.
        assert!(Arc::ptr_eq(&got, &reg));
    }

    #[test]
    fn outbound_invoke_without_attached_registry_errors() {
        let inv = OxiLeanInvoker::new();
        let err = inv.invoke_outbound(42, &[]).unwrap_err();
        assert_eq!(err.code, 0x0002_0005);
    }

    #[test]
    fn outbound_invoke_round_trips_through_registry() {
        let inv = OxiLeanInvoker::new();
        let reg: Arc<leo4_abi::RustCallbackRegistry> =
            Arc::new(leo4_abi::RustCallbackRegistry::new());
        let (id, _g) = reg.register(|args: &[u8]| {
            // Echo args + a sentinel byte.
            let mut out = args.to_vec();
            out.push(0xAA);
            Ok(out)
        });
        inv.attach_outbound_registry(reg);
        let out = inv.invoke_outbound(id, &[1, 2, 3]).unwrap();
        assert_eq!(out, vec![1, 2, 3, 0xAA]);
    }

    #[test]
    fn outbound_invoke_unknown_id_yields_invalid_handle() {
        let inv = OxiLeanInvoker::new();
        let reg: Arc<leo4_abi::RustCallbackRegistry> =
            Arc::new(leo4_abi::RustCallbackRegistry::new());
        inv.attach_outbound_registry(reg);
        let err = inv.invoke_outbound(0xDEAD_BEEF, &[]).unwrap_err();
        assert_eq!(err.code, leo4_abi::error::error_codes::INVALID_HANDLE);
    }

    // ─── #76 P0c outbound dispatch bridge ──────────────

    #[test]
    fn outbound_bridge_routes_callback_id_to_registry() {
        let inv = OxiLeanInvoker::new();
        let mangled = "leo4__bridge__test__cb__h0a_a";

        // Bridge needs the metadata entry first (paired
        // register_export call).
        inv.register_export(mangled).unwrap();

        // Outbound registry pre-populated with one
        // callback that doubles a u8.
        let reg: Arc<leo4_abi::RustCallbackRegistry> =
            Arc::new(leo4_abi::RustCallbackRegistry::new());
        let (id, _guard) = reg.register(|args: &[u8]| {
            assert_eq!(args.len(), 1, "callback expects 1 byte");
            Ok(vec![args[0].wrapping_mul(2)])
        });
        inv.attach_outbound_registry(reg);

        // Bridge: any inbound call to `mangled` unpacks
        // (callback_id, rest) and forwards to invoke_outbound.
        inv.register_outbound_dispatch_callback(mangled).unwrap();

        // Simulate the evaluator firing the bridge with
        // the args [callback_id_le ‖ 0x15].
        let mut args = id.to_le_bytes().to_vec();
        args.push(0x15);
        let out = <OxiLeanInvoker as LeanProcInvoker>::invoke(&inv, mangled, &args)
            .expect("bridge dispatch should succeed");
        assert_eq!(out, vec![0x2A]);
    }

    #[test]
    fn outbound_bridge_rejects_short_args() {
        let inv = OxiLeanInvoker::new();
        let mangled = "leo4__bridge__test__cb_short__h0b_a";
        inv.register_export(mangled).unwrap();
        let reg: Arc<leo4_abi::RustCallbackRegistry> =
            Arc::new(leo4_abi::RustCallbackRegistry::new());
        inv.attach_outbound_registry(reg);
        inv.register_outbound_dispatch_callback(mangled).unwrap();

        // Only 4 bytes — bridge needs at least 8 for the
        // callback_id LE prefix.
        let err = <OxiLeanInvoker as LeanProcInvoker>::invoke(&inv, mangled, &[1, 2, 3, 4])
            .unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("8 bytes") || msg.contains("shorter"),
            "error should mention the short-args case: {msg}"
        );
    }

    #[test]
    fn outbound_bridge_without_attached_registry_surfaces_clear_error() {
        let inv = OxiLeanInvoker::new();
        let mangled = "leo4__bridge__test__no_reg__h0c_a";
        inv.register_export(mangled).unwrap();
        inv.register_outbound_dispatch_callback(mangled).unwrap();
        // Don't attach an outbound registry.

        let mut args = 42u64.to_le_bytes().to_vec();
        args.push(0x99);
        let err = <OxiLeanInvoker as LeanProcInvoker>::invoke(&inv, mangled, &args)
            .unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("no outbound") || msg.contains("attach_outbound_registry"),
            "error should mention the unattached-registry case: {msg}"
        );
    }
}
