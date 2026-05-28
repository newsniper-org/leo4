//! Function-arrow / callback ABI — typed opaque holder for a Lean
//! closure that crossed the leo4 boundary into Rust as a function
//! parameter (`fn(T₁,…,Tₙ) -> R` on the wire).
//!
//! Wire shape: a single `u64 callback_id`, little-endian, per
//! `SPEC/canonical-abi.md` §13a.
//!
//! - `LeanCallback::from_id(id)` constructs a raw token from the
//!   decoded id. The token alone cannot dispatch — the invocation
//!   path is impl-specific (oxilean: in-process evaluator
//!   re-entry; mslean4: LECQ/LECR IPC frames).
//! - The adapter (`sibling/leo4-oxilean/` for the oxilean transport,
//!   `crates/leo4-rust-bridge/` for mslean4) calls
//!   [`LeanCallback::bind`] after decoding to install a typed
//!   [`CallbackInvoker`]. Until `bind` is called, [`LeanCallback::invoke`]
//!   returns an `ENCODE_ERROR` rather than panicking — surface the bug
//!   loudly without aborting the worker.
//! - The Phase 10-B1.x runtime status (which impls have a wired
//!   [`CallbackInvoker`]) is tracked in `ROADMAP.md` and
//!   `SPEC/reverse-direction.md` §10a. v0 ships the type + canonical
//!   encode/decode; runtime wiring lands per-impl.

use core::marker::PhantomData;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::{
    error::{error_codes, LeanError},
    marshal::LeanMarshal,
};

/// Adapter-supplied dispatch hook. The adapter or dispatcher
/// installs one of these on every [`LeanCallback`] it decodes
/// before handing the typed value to the user export.
///
/// Implementations live impl-side:
/// - **oxilean** (rust-native / rust-transpile): the in-process
///   evaluator re-entry path documented in
///   `docs/ox8-3-callback-hook-design.md`. `invoke_bytes` resolves
///   `callback_id` through the `OxiLean` `CallbackRegistry`
///   (OX8.3a) and re-enters the evaluator on the registered
///   `FfiValue::Fn`.
/// - **mslean4**: emits a `LECQ` frame (`SPEC/reverse-direction.md`
///   §10a) and blocks until the matching `LECR` returns. Runtime
///   deferred to Phase 10-B1.x.
pub trait CallbackInvoker: Send + Sync {
    /// Invoke the underlying Lean closure identified by
    /// `callback_id` with `args_buf`'s canonical-ABI bytes,
    /// returning the canonically-encoded result.
    ///
    /// # Errors
    /// `LeanError` for any decode/encode/transport failure.
    fn invoke_bytes(
        &self,
        callback_id: u64,
        args_buf: &[u8],
    ) -> Result<Vec<u8>, LeanError>;
}

/// Typed opaque token for a Lean closure passed across the leo4
/// boundary. The phantom carries the arrow type so user code calls
/// `cb.invoke(args)` with type-checked arguments and return.
pub struct LeanCallback<R, Args> {
    callback_id: u64,
    invoker: Option<Arc<dyn CallbackInvoker>>,
    _phantom: PhantomData<fn(Args) -> R>,
}

impl<R, Args> LeanCallback<R, Args> {
    /// Build an unbound token from a raw wire id. The adapter then
    /// installs an invoker via [`bind`]. Public so adapters /
    /// macro-emitted wrappers can construct one explicitly when
    /// decoding outside the standard `LeanMarshal` path.
    #[must_use]
    pub fn from_id(callback_id: u64) -> Self {
        Self {
            callback_id,
            invoker: None,
            _phantom: PhantomData,
        }
    }

    /// Install the adapter's dispatch hook. Idempotent — re-binding
    /// replaces the previous invoker. Callable after `canonical_decode`.
    pub fn bind(&mut self, invoker: Arc<dyn CallbackInvoker>) {
        self.invoker = Some(invoker);
    }

    /// Raw wire `callback_id`. Useful for adapter-side bookkeeping
    /// (e.g. correlating with the per-call closure registry).
    #[must_use]
    pub fn id(&self) -> u64 {
        self.callback_id
    }

    /// `true` iff [`bind`] has been called.
    #[must_use]
    pub fn is_bound(&self) -> bool {
        self.invoker.is_some()
    }
}

impl<R, Args> LeanCallback<R, Args>
where
    R: LeanMarshal,
    Args: LeanMarshal,
{
    /// Invoke the Lean closure with `args`. Encodes via
    /// [`LeanMarshal`], dispatches through the bound invoker,
    /// decodes the canonical return.
    ///
    /// # Errors
    /// - `ENCODE_ERROR` if [`bind`] hasn't been called by the adapter
    ///   (a wiring bug, not user error).
    /// - Propagates any `LeanError` from the invoker.
    /// - `DECODE_ERROR` if the return buffer has trailing bytes.
    pub fn invoke(&self, args: Args) -> Result<R, LeanError> {
        let invoker = self.invoker.as_ref().ok_or_else(|| {
            LeanError::new(
                error_codes::ENCODE_ERROR,
                "LeanCallback::invoke called before the adapter bound the invoker — \
                 wiring bug in the impl's decoder"
                    .to_string(),
            )
        })?;
        let mut args_buf = Vec::new();
        args.canonical_encode(&mut args_buf);
        let ret_buf = invoker.invoke_bytes(self.callback_id, &args_buf)?;
        let (val, off) = R::canonical_decode(&ret_buf, 0)?;
        if off != ret_buf.len() {
            return Err(LeanError::new(
                error_codes::DECODE_ERROR,
                format!(
                    "LeanCallback::invoke: trailing bytes in return buffer \
                     ({off} of {len})",
                    len = ret_buf.len()
                ),
            ));
        }
        Ok(val)
    }
}

impl<R, Args> core::fmt::Debug for LeanCallback<R, Args> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LeanCallback")
            .field("callback_id", &self.callback_id)
            .field("bound", &self.is_bound())
            .finish()
    }
}

impl<R, Args> LeanMarshal for LeanCallback<R, Args> {
    /// Encode a `LeanCallback` as its wire id (little-endian u64).
    /// Encoding is rarely exercised in v0 (the Lean → Rust callback
    /// pass is the dominant direction); we ship the symmetric path
    /// so the trait is total over `LeanCallback`.
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.callback_id.to_le_bytes());
    }

    /// Decode a `LeanCallback` from a `u64 callback_id` reading.
    /// `callback_id == 0` is the null sentinel
    /// (`SPEC/canonical-abi.md` §13a) — rejected with
    /// `INVALID_HANDLE`. The decoded value has `invoker == None`;
    /// the adapter MUST call [`bind`] before user code touches it.
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        if off.saturating_add(8) > buf.len() {
            return Err(LeanError::new(
                error_codes::DECODE_ERROR,
                format!(
                    "LeanCallback: short buffer (need 8 bytes at offset {off}, \
                     have {} bytes total)",
                    buf.len()
                ),
            ));
        }
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&buf[off..off + 8]);
        let id = u64::from_le_bytes(bytes);
        if id == 0 {
            return Err(LeanError::new(
                error_codes::INVALID_HANDLE,
                "LeanCallback: null sentinel callback_id == 0 (SPEC/canonical-abi.md §13a)"
                    .to_string(),
            ));
        }
        Ok((Self::from_id(id), off + 8))
    }
}

// ─── Outbound direction (P0b step 1, 2026-05-28) ────────────

/// Type-erased Rust callback stored in [`RustCallbackRegistry`].
/// Receives args as canonical-ABI bytes, returns canonical-ABI
/// bytes for the result (or a `LeanError` to propagate back).
pub type ErasedRustCallback =
    Arc<dyn Fn(&[u8]) -> Result<Vec<u8>, LeanError> + Send + Sync>;

/// Main-side registry for Rust closures that are passed across
/// the leo4 boundary as function-arrow arguments (`fn(...) -> R`
/// in `leo4::import!` declarations). Pairs with the inbound
/// [`LeanCallback<R, Args>`]: that one holds an id minted on the
/// *other* side; this one mints ids on *this* side.
///
/// Per-call lifetime: the macro-emitted wrapper allocates fresh
/// ids on entry, encodes them into the args buffer, calls the
/// shim, then drops the registration on return. Concrete
/// allocation/deallocation API is exposed via [`Self::register`]
/// + [`RegistrationGuard`] (RAII).
///
/// Thread-safety: behind an internal `Mutex<HashMap>`. The
/// callback closures themselves are `Send + Sync`.
///
/// **NOT exposed in v0.** This commit ships the registry as the
/// substrate for the macro substitution (P0b step 2) that lands
/// in the follow-up. v0 users with hand-written boundary callers
/// can construct one explicitly; the `leo4::import!` macro
/// substitution + impl-side wiring (adapter for oxilean, IPC
/// frame for mslean4) is what turns this into a first-class
/// surface.
pub struct RustCallbackRegistry {
    next_id: AtomicU64,
    callbacks: Mutex<HashMap<u64, ErasedRustCallback>>,
}

impl Default for RustCallbackRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RustCallbackRegistry {
    /// Construct an empty registry. The first id minted is `1`
    /// (id `0` is the wire-level null sentinel per SPEC §13a).
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            callbacks: Mutex::new(HashMap::new()),
        }
    }

    /// Register a callback and return its freshly-minted
    /// `callback_id` along with an RAII guard. Dropping the
    /// guard deregisters the callback. Callers MUST hold the
    /// guard for the entire duration of any boundary call that
    /// passes the id, and MUST drop it before the call returns
    /// — id lifetimes are per-call-scope per SPEC §13a.
    ///
    /// The closure receives canonical-ABI args bytes (encoded
    /// upstream by the boundary's invoker) and returns
    /// canonical-ABI return bytes (decoded downstream by the
    /// boundary's invoker). For typed Rust `Fn(...) -> R`
    /// closures, the macro substitution layer (P0b step 2)
    /// will generate the encode/decode shim automatically;
    /// for hand-written callers, build the byte-shaped
    /// closure yourself.
    pub fn register<F>(self: &Arc<Self>, callback: F) -> (u64, RegistrationGuard)
    where
        F: Fn(&[u8]) -> Result<Vec<u8>, LeanError> + Send + Sync + 'static,
    {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        debug_assert!(id != 0, "RustCallbackRegistry: id 0 is reserved (SPEC §13a)");
        let arc: ErasedRustCallback = Arc::new(callback);
        self.callbacks
            .lock()
            .expect("RustCallbackRegistry mutex poisoned at register")
            .insert(id, arc);
        let guard = RegistrationGuard {
            registry: Arc::clone(self),
            id,
        };
        (id, guard)
    }

    /// Look up a callback by id and invoke it with `args`.
    ///
    /// Used by the boundary adapter (oxilean) or dispatcher
    /// (mslean4) when the receiving side dereferences a
    /// callback id during a boundary call's body.
    ///
    /// # Errors
    /// `INVALID_HANDLE` if no callback is registered for `id`
    /// (either it was never registered or the guard was
    /// dropped early). Propagates whatever `LeanError` the
    /// callback itself returns.
    pub fn invoke(&self, id: u64, args: &[u8]) -> Result<Vec<u8>, LeanError> {
        let cb = self
            .callbacks
            .lock()
            .expect("RustCallbackRegistry mutex poisoned at invoke")
            .get(&id)
            .cloned();
        let cb = cb.ok_or_else(|| {
            LeanError::new(
                error_codes::INVALID_HANDLE,
                format!(
                    "RustCallbackRegistry::invoke: callback_id {id} not \
                     registered (either never minted or guard dropped early)"
                ),
            )
        })?;
        cb(args)
    }

    /// Diagnostic: number of currently-registered callbacks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.callbacks
            .lock()
            .map(|c| c.len())
            .unwrap_or(0)
    }

    /// `true` iff no callbacks are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Forcibly deregister an id. Normally called by
    /// [`RegistrationGuard::drop`]; exposed for symmetry +
    /// for tests that bypass the guard.
    pub fn deregister(&self, id: u64) {
        self.callbacks
            .lock()
            .expect("RustCallbackRegistry mutex poisoned at deregister")
            .remove(&id);
    }
}

/// RAII handle returned by [`RustCallbackRegistry::register`].
/// Dropping the guard removes the callback from the registry —
/// SPEC §13a's per-call lifetime contract enforced by the type
/// system rather than convention.
pub struct RegistrationGuard {
    registry: Arc<RustCallbackRegistry>,
    id: u64,
}

impl RegistrationGuard {
    /// The minted `callback_id`. Identical to the first
    /// element of the `register(...)` tuple — kept here so
    /// the guard alone is sufficient for callers that want to
    /// forward only the id.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }
}

impl Drop for RegistrationGuard {
    fn drop(&mut self) {
        self.registry.deregister(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CountingInvoker {
        last_id: std::sync::Mutex<u64>,
        last_args: std::sync::Mutex<Vec<u8>>,
        canned_ret: Vec<u8>,
    }

    impl CallbackInvoker for CountingInvoker {
        fn invoke_bytes(
            &self,
            callback_id: u64,
            args_buf: &[u8],
        ) -> Result<Vec<u8>, LeanError> {
            *self.last_id.lock().unwrap() = callback_id;
            *self.last_args.lock().unwrap() = args_buf.to_vec();
            Ok(self.canned_ret.clone())
        }
    }

    #[test]
    fn encode_decode_roundtrip() {
        let cb: LeanCallback<u64, (u64,)> = LeanCallback::from_id(0x1234_5678_DEAD_BEEF);
        let mut buf = Vec::new();
        cb.canonical_encode(&mut buf);
        assert_eq!(buf.len(), 8);
        let (decoded, off): (LeanCallback<u64, (u64,)>, _) =
            LeanCallback::canonical_decode(&buf, 0).unwrap();
        assert_eq!(off, 8);
        assert_eq!(decoded.id(), 0x1234_5678_DEAD_BEEF);
        assert!(!decoded.is_bound());
    }

    #[test]
    fn decode_rejects_null_sentinel() {
        let buf = [0u8; 8];
        let r = LeanCallback::<u64, (u64,)>::canonical_decode(&buf, 0);
        let err = r.unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_HANDLE);
    }

    #[test]
    fn decode_rejects_short_buffer() {
        let buf = [1u8, 2, 3, 4];
        let r = LeanCallback::<u64, (u64,)>::canonical_decode(&buf, 0);
        let err = r.unwrap_err();
        assert_eq!(err.code, error_codes::DECODE_ERROR);
    }

    #[test]
    fn invoke_unbound_yields_encode_error() {
        let cb: LeanCallback<u64, (u64,)> = LeanCallback::from_id(7);
        let err = cb.invoke((42,)).unwrap_err();
        assert_eq!(err.code, error_codes::ENCODE_ERROR);
    }

    #[test]
    fn invoke_round_trips_through_invoker() {
        // canned return: u64 = 84
        let canned = 84u64.to_le_bytes().to_vec();
        let invoker = Arc::new(CountingInvoker {
            last_id: std::sync::Mutex::new(0),
            last_args: std::sync::Mutex::new(Vec::new()),
            canned_ret: canned,
        });
        let mut cb: LeanCallback<u64, (u64,)> = LeanCallback::from_id(13);
        cb.bind(invoker.clone());
        let r = cb.invoke((42,)).unwrap();
        assert_eq!(r, 84);
        assert_eq!(*invoker.last_id.lock().unwrap(), 13);
        // (u64,) encodes as 8 LE bytes.
        assert_eq!(invoker.last_args.lock().unwrap().len(), 8);
    }

    #[test]
    fn debug_omits_invoker_internals() {
        let cb: LeanCallback<u64, (u64,)> = LeanCallback::from_id(99);
        let s = format!("{cb:?}");
        assert!(s.contains("99"));
        assert!(s.contains("bound: false"));
    }

    // ─── RustCallbackRegistry (P0b step 1) ─────────────────

    #[test]
    fn rust_callback_registry_mints_nonzero_ids() {
        let reg = Arc::new(RustCallbackRegistry::new());
        let (id1, _g1) = reg.register(|_args: &[u8]| Ok(Vec::new()));
        let (id2, _g2) = reg.register(|_args: &[u8]| Ok(Vec::new()));
        assert_ne!(id1, 0, "id 0 reserved as null sentinel");
        assert_ne!(id2, 0);
        assert_ne!(id1, id2, "each register call must mint a fresh id");
    }

    #[test]
    fn rust_callback_registry_invoke_routes_to_callback() {
        let reg = Arc::new(RustCallbackRegistry::new());
        let (id, _guard) = reg.register(|args: &[u8]| {
            // Echo args+1 byte
            let mut out = args.to_vec();
            out.push(1);
            Ok(out)
        });
        let out = reg.invoke(id, &[10, 20]).unwrap();
        assert_eq!(out, vec![10, 20, 1]);
    }

    #[test]
    fn rust_callback_registry_guard_drop_deregisters() {
        let reg = Arc::new(RustCallbackRegistry::new());
        let (id, guard) = reg.register(|_args: &[u8]| Ok(Vec::new()));
        assert_eq!(reg.len(), 1);
        drop(guard);
        assert_eq!(reg.len(), 0, "guard drop must purge the callback");
        let err = reg.invoke(id, &[]).unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_HANDLE);
    }

    #[test]
    fn rust_callback_registry_invoke_unknown_id_yields_invalid_handle() {
        let reg = Arc::new(RustCallbackRegistry::new());
        let err = reg.invoke(0xDEAD_BEEF, &[]).unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_HANDLE);
    }

    #[test]
    fn rust_callback_registry_supports_concurrent_register_invoke() {
        let reg = Arc::new(RustCallbackRegistry::new());
        let n = 32;
        let guards: Vec<_> = (0..n)
            .map(|i| {
                let want = i as u8;
                reg.register(move |_args: &[u8]| Ok(vec![want]))
            })
            .collect();
        for (i, (id, _g)) in guards.iter().enumerate() {
            let r = reg.invoke(*id, &[]).unwrap();
            assert_eq!(r, vec![i as u8]);
        }
    }
}
