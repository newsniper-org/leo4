//! leo4-native — load and dispatch into a `<pkg>.leo4-shim.so`.
//!
//! Minimum API surface landed in **P5-a₂** (2026-05-20):
//!
//!   * [`Lean::open`] reads `<pkg>.leo4-handshake` (the JSON sidecar
//!     the Lake plugin emits), opens the `.so` via `libloading`, and
//!     calls the shim's `leo4_handshake` entry point to verify
//!     schema-hash + ABI-version equality. Mismatch → [`LeanError`].
//!   * [`Lean::scope`] runs a closure inside a fresh [`Arena<'a>`].
//!     The lifetime parameter pins all [`LeanRef<'a, T>`] handles
//!     created during the scope so they cannot escape; the closure
//!     return value is the only thing that crosses the boundary.
//!
//! Deferred to **P5-a₃**:
//!   * Lean runtime init + per-module init (`lean_initialize_runtime_module`,
//!     `lean_initialize`, `initialize_<target_module>`,
//!     `lean_io_mark_end_initialization`).
//!   * Per-instantiation `call_shim` byte-buffer dispatch.
//!   * `LeanRef<'a, T>::Drop` honoring `lean_dec`.
//!
//! See `LEO4-DESIGN.md` §9.1, `SPEC/canonical-abi.md` §§14–15.

use std::ffi::c_char;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use libloading::Library;

/// Result of any leo4-native fallible operation.
pub type LeanResult<T> = Result<T, LeanError>;

/// Errors surfaced by the loader / handshake / dispatch paths.
///
/// `code` mirrors `SPEC/canonical-abi.md §13` status codes (e.g.
/// `0x0000_0005` for handshake mismatch); host-side errors that have
/// no canonical-ABI counterpart use `code = -1`. `detail` is a
/// human-readable explanation suitable for `tracing` / panics.
#[derive(Debug, Clone)]
pub struct LeanError {
    pub code: i32,
    pub detail: String,
}

impl LeanError {
    fn host(detail: impl Into<String>) -> Self {
        Self {
            code: -1,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for LeanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "leo4-native: code={:#010x} {}", self.code as u32, self.detail)
    }
}

impl std::error::Error for LeanError {}

/// Parsed contents of `<pkg>.leo4-handshake`. Held privately on
/// [`Lean`] so the loader can re-use them across multiple scopes.
#[derive(Debug, Clone)]
struct HandshakeMeta {
    schema_hash_be: [u8; 8],
    /// User-supplied Lean module the `@[leo4_export]`s live in
    /// (e.g. `"Sample"`). P5-a₃ will dlsym `initialize_<this>` from
    /// the shim and call it after Lean runtime init.
    #[allow(dead_code)]
    target_module: String,
    /// ABI version baked into the shim (currently always 1).
    abi_version: u32,
}

fn parse_handshake(path: &Path) -> LeanResult<HandshakeMeta> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| LeanError::host(format!("read {path:?}: {e}")))?;
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| LeanError::host(format!("parse {path:?}: {e}")))?;
    let hex = v
        .get("schema_hash_bytes")
        .and_then(|x| x.as_str())
        .ok_or_else(|| LeanError::host("handshake.schema_hash_bytes missing"))?;
    if hex.len() != 16 {
        return Err(LeanError::host(format!(
            "handshake.schema_hash_bytes is {} chars, expected 16",
            hex.len()
        )));
    }
    let mut bytes = [0u8; 8];
    for i in 0..8 {
        bytes[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|e| LeanError::host(format!("hex byte {i}: {e}")))?;
    }
    let abi_version = u32::try_from(
        v.get("abi_version")
            .and_then(|x| x.as_u64())
            .ok_or_else(|| LeanError::host("handshake.abi_version missing"))?,
    )
    .map_err(|e| LeanError::host(format!("abi_version out of u32 range: {e}")))?;
    let target_module = v
        .get("target_module")
        .and_then(|x| x.as_str())
        .ok_or_else(|| LeanError::host("handshake.target_module missing"))?
        .to_string();
    Ok(HandshakeMeta {
        schema_hash_be: bytes,
        target_module,
        abi_version,
    })
}

/// `SPEC/canonical-abi.md §15` handshake signature.
type LeoHandshakeFn =
    unsafe extern "C" fn(*const u8, u32, *mut c_char, usize) -> i32;

/// A loaded `<pkg>.leo4-shim.so` whose handshake has succeeded.
///
/// One `Lean` instance per process (Lean runtime is single-threaded
/// per `SPEC/canonical-abi.md §16`; P5-a₃ enforces this with an
/// `std::sync::Once` over `lean_initialize_runtime_module`).
pub struct Lean {
    #[allow(dead_code)]
    lib: Library,
    meta: HandshakeMeta,
    so_path: PathBuf,
}

impl std::fmt::Debug for Lean {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lean")
            .field("so_path", &self.so_path)
            .field("target_module", &self.meta.target_module)
            .field("abi_version", &self.meta.abi_version)
            .finish()
    }
}

impl Lean {
    /// Load `so_path` and run the schema handshake against
    /// `handshake_path`. Both files come from the Lake plugin's
    /// `<pkg>.leo4-shim.so` / `<pkg>.leo4-handshake` pair.
    ///
    /// # Errors
    ///
    /// * `LeanError { code = -1, ... }` for any host-side failure
    ///   (file I/O, JSON parse, libloading dlopen, symbol lookup).
    /// * `LeanError { code = 5, ... }` for an ABI / schema mismatch
    ///   (`LEO4_ERR_HANDSHAKE_MISMATCH`).
    ///
    /// # Safety
    ///
    /// `so_path` must point to a leo4-shim produced by a compatible
    /// Lake plugin version. Loading an arbitrary `.so` is rejected
    /// by the handshake but exposes the host to whatever the `.so`
    /// does in its global constructors.
    pub fn open(so_path: impl AsRef<Path>, handshake_path: impl AsRef<Path>) -> LeanResult<Self> {
        let meta = parse_handshake(handshake_path.as_ref())?;
        // SAFETY: callers of `Lean::open` accept the risk that
        // `so_path` runs arbitrary `__attribute__((constructor))`
        // code; the handshake below catches schema-mismatch only.
        let lib = unsafe { Library::new(so_path.as_ref()) }
            .map_err(|e| LeanError::host(format!("dlopen {:?}: {e}", so_path.as_ref())))?;
        // Probe leo4_handshake before anything else: the shim's
        // global ctors have already run by this point, but if the
        // schema doesn't match we want to bail before any real
        // function is dispatched.
        let mut detail = [0u8; 256];
        let rc = unsafe {
            let symbol: libloading::Symbol<LeoHandshakeFn> = lib
                .get(b"leo4_handshake\0")
                .map_err(|e| LeanError::host(format!("dlsym leo4_handshake: {e}")))?;
            symbol(
                meta.schema_hash_be.as_ptr(),
                meta.abi_version,
                detail.as_mut_ptr().cast::<c_char>(),
                detail.len(),
            )
        };
        if rc != 0 {
            return Err(LeanError {
                code: rc,
                detail: format!(
                    "handshake rejected by shim at {:?} (abi_version={}, schema_hash_be={})",
                    so_path.as_ref(),
                    meta.abi_version,
                    hex8(meta.schema_hash_be)
                ),
            });
        }
        Ok(Self {
            lib,
            meta,
            so_path: so_path.as_ref().to_path_buf(),
        })
    }

    /// Target Lean module the user's `@[leo4_export]`s live in.
    /// Surfaced for downstream tooling (P5-a₃ uses it to drive module
    /// init).
    #[must_use]
    pub fn target_module(&self) -> &str {
        &self.meta.target_module
    }

    /// Run `f` inside a fresh [`Arena<'a>`]. The closure's return
    /// value is the only thing that crosses the scope boundary —
    /// [`LeanRef<'a, T>`] handles bound to the arena cannot outlive
    /// the call.
    pub fn scope<R>(&self, f: impl for<'a> FnOnce(&'a Arena<'a>) -> R) -> R {
        let arena = Arena {
            _marker: PhantomData,
            _no_send: PhantomData,
        };
        f(&arena)
    }
}

fn hex8(bytes: [u8; 8]) -> String {
    let mut s = String::with_capacity(16);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// A short-lived region tied to a [`Lean::scope`] call. Future
/// `LeanRef<'a, T>` handles borrow from `'a` so they cannot outlive
/// the closure.
///
/// `Arena` is `!Send` + `!Sync` per `LEO4-DESIGN.md §16`: the Lean
/// runtime is single-threaded today, and a cross-thread handle would
/// trip both the runtime and the closed-world admit-set assumption
/// in the leo4 plugin.
pub struct Arena<'a> {
    _marker: PhantomData<&'a ()>,
    /// Marker to opt out of `Send` / `Sync` without adding any
    /// runtime state. The `*const ()` raw pointer is a standard
    /// trick: pointers are `!Send + !Sync` by default and carry no
    /// drop glue.
    _no_send: PhantomData<*const ()>,
}

/// Lean object handle, lifetime-bound to the originating
/// [`Arena<'a>`]. `T` is a phantom type parameter; P5-a₃ wires it
/// to the per-export wrapper types produced by `#[leo4::import]`.
///
/// `LeanRef` is `!Send + !Sync` for the same reasons as `Arena<'_>`.
pub struct LeanRef<'a, T: ?Sized> {
    /// `lean_object *`. Opaque from this crate's perspective —
    /// `leo4-native` only passes it back into shim entry points.
    #[allow(dead_code)]
    ptr: *mut std::ffi::c_void,
    _marker: PhantomData<(&'a (), fn() -> T)>,
}

impl<T: ?Sized> std::fmt::Debug for LeanRef<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LeanRef")
            .field("ptr", &self.ptr)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_handshake_round_trip() {
        let dir = std::env::temp_dir().join("leo4-native-handshake");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.handshake");
        std::fs::write(
            &path,
            r#"{"abi_version": 1, "schema_hash_bytes": "0102030405060708",
                 "target_module": "Sample", "schema_hash": "x"}"#,
        )
        .unwrap();
        let meta = parse_handshake(&path).unwrap();
        assert_eq!(meta.schema_hash_be, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(meta.abi_version, 1);
        assert_eq!(meta.target_module, "Sample");
    }

    #[test]
    fn handshake_bad_hex_len_rejected() {
        let dir = std::env::temp_dir().join("leo4-native-handshake-bad");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.handshake");
        std::fs::write(
            &path,
            r#"{"abi_version": 1, "schema_hash_bytes": "01020304",
                 "target_module": "Sample"}"#,
        )
        .unwrap();
        let err = parse_handshake(&path).unwrap_err();
        assert!(err.detail.contains("16"), "unexpected detail: {}", err.detail);
    }
}
