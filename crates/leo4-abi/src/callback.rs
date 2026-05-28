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
use std::sync::Arc;

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
///   `callback_id` through the OxiLean `CallbackRegistry`
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
}
