//! leo4-native — load and dispatch into a `<pkg>.leo4-shim.so`.
//!
//! API surface as of **P5-a₃** (2026-05-20):
//!
//!   * [`Lean::open`] reads `<pkg>.leo4-handshake`, opens the shim via
//!     `libloading`, runs the schema-hash + ABI handshake, drives the
//!     Lean runtime init sequence (`lean_initialize_runtime_module`
//!     + wrapper-module `initialize_*` + `lean_io_mark_end_initialization`),
//!     and returns a ready-to-use [`Lean`]. One per process.
//!   * [`Lean::scope`] runs a closure inside a fresh [`Arena<'a>`].
//!     The lifetime parameter pins all [`LeanRef<'a, T>`] handles
//!     created during the scope so they cannot escape; the closure
//!     return value is the only thing that crosses the boundary.
//!   * [`Lean::call_shim`] is the byte-buffer dispatch entry point
//!     `#[leo4::import]` (P5-b) builds on. Given a mangled body, it
//!     dlsym's `leo4_call_<body>` once (cached) and invokes it with
//!     caller-supplied argument / return buffers.
//!   * [`LeanRef<'a, T>::Drop`] calls `lean_dec` on the boxed handle.
//!
//! See `LEO4-DESIGN.md` §9.1, `SPEC/canonical-abi.md` §§14–15.

use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::Once;

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

/// Lift a `leo4_abi::LeanError` (raised by `canonical_decode` /
/// `canonical_encode` failures) into the loader-flavoured `LeanError`
/// so `leo4::import!`-generated wrappers can use a single `?` to
/// propagate both decode errors and dispatch errors.
impl From<leo4_abi::LeanError> for LeanError {
    fn from(e: leo4_abi::LeanError) -> Self {
        Self {
            #[allow(clippy::cast_possible_wrap)]
            code: e.code as i32,
            detail: e.message,
        }
    }
}

/// Parsed contents of `<pkg>.leo4-handshake`. Held privately on
/// [`Lean`] so the loader can re-use them across multiple scopes.
#[derive(Debug, Clone)]
struct HandshakeMeta {
    schema_hash_be: [u8; 8],
    /// User-supplied Lean module the `@[leo4_export]`s live in
    /// (e.g. `"Sample"`). Informational — the loader actually
    /// initialises through `wrapper_init_symbol`, which transitively
    /// brings up `initialize_<target_module>`.
    #[allow(dead_code)]
    target_module: String,
    /// Linker-visible `initialize_*` symbol the loader dlsym's and
    /// invokes after `lean_initialize_runtime_module`.
    wrapper_init_symbol: String,
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
    let wrapper_init_symbol = v
        .get("wrapper_init_symbol")
        .and_then(|x| x.as_str())
        .ok_or_else(|| LeanError::host("handshake.wrapper_init_symbol missing"))?
        .to_string();
    Ok(HandshakeMeta {
        schema_hash_be: bytes,
        target_module,
        wrapper_init_symbol,
        abi_version,
    })
}

/// `SPEC/canonical-abi.md §15` handshake signature.
type LeoHandshakeFn =
    unsafe extern "C" fn(*const u8, u32, *mut c_char, usize) -> i32;

/// `SPEC/canonical-abi.md §14` per-instantiation entry-point signature.
///
/// `(arena, args_ptr, args_len, ret_ptr, ret_cap, ret_len)` → status.
/// `arena` is reserved (the W7-2a..d scalar/composite path ignores it);
/// `*ret_len` is read for buffer-too-small reporting and written on
/// success.
type LeoCallShimFn = unsafe extern "C" fn(
    *mut c_void,
    *const u8,
    usize,
    *mut u8,
    usize,
    *mut usize,
) -> i32;

/// Lean runtime entry points the loader needs. All come from
/// `libleanshared.so`, which `<pkg>.leo4-shim.so` links against — so
/// `dlsym` against the *shim* picks them up transitively. `lean.h`
/// defines several of these as `static inline`, which the linker
/// does NOT export; those (currently just `lean_io_result_is_ok`)
/// we re-implement inline in Rust using the documented
/// `lean_object` layout below.
type VoidFn = unsafe extern "C" fn();
/// Module init: `lean_object * initialize_<Mod>(uint8_t builtin)`.
type ModInitFn = unsafe extern "C" fn(u8) -> *mut c_void;
/// `lean_dec_ref` is `static inline` in `lean.h`; only its cold
/// fall-through `lean_dec_ref_cold` is `LEAN_EXPORT`'d. We dlsym
/// the cold path and inline the fast-path m_rc decrement ourselves.
type LeanDecRefColdFn = unsafe extern "C" fn(*mut c_void);
/// `lean_dec_ref` fast path (re-implemented in Rust below) compatible
/// with `LeanDecRefColdFn`; stored on `Lean` so `LeanRef::Drop` can
/// invoke without a back-reference.
pub type LeanDecRefFn = LeanDecRefColdFn;
type LeanIoResultShowErrorFn = unsafe extern "C" fn(*mut c_void);

/// `lean.h` `lean_object` layout (taken verbatim from
/// `/opt/lean4/include/lean/lean.h`):
///
/// ```c
/// typedef struct {
///     int      m_rc;       // bytes 0..3
///     unsigned m_cs_sz:16; // bytes 4..5
///     unsigned m_other:8;  // byte  6
///     unsigned m_tag:8;    // byte  7
/// } lean_object;
/// ```
///
/// `lean_io_result_is_ok(r)` is `lean_ptr_tag(r) == 0`, i.e. byte 7
/// equals 0. This is stable across all Tier-1 Linux x86_64 builds
/// of Lean (ABI is the Itanium C++ ABI / sysv x86_64 ABI here,
/// neither of which reorders bit-fields).
const LEAN_OBJECT_TAG_OFFSET: usize = 7;

#[inline]
unsafe fn lean_io_result_is_ok(r: *mut c_void) -> bool {
    let tag_ptr = unsafe { (r as *const u8).add(LEAN_OBJECT_TAG_OFFSET) };
    unsafe { *tag_ptr == 0 }
}

/// Inline `lean_dec_ref` — fast path is an `m_rc` decrement; the cold
/// path (free / atomic) is delegated to `lean_dec_ref_cold` which the
/// caller passes in. `m_rc` lives at offset 0 of `lean_object`
/// (signed int per the layout struct).
#[inline]
unsafe fn lean_dec_ref_inline(o: *mut c_void, cold: LeanDecRefColdFn) {
    if o.is_null() {
        return;
    }
    let rc_ptr = o as *mut i32;
    let rc = unsafe { *rc_ptr };
    if rc > 1 {
        unsafe { *rc_ptr = rc - 1 };
    } else if rc != 0 {
        // rc == 1: real decrement to free; rc < 0: multi-threaded.
        // Both go through the cold path which handles atomics +
        // deallocation.
        unsafe { cold(o) };
    }
    // rc == 0: persistent / compact, no-op.
}

/// Process-wide guard: `lean_initialize_runtime_module` must run
/// exactly once.
static LEAN_RUNTIME_INIT: Once = Once::new();

/// A loaded `<pkg>.leo4-shim.so` whose handshake has succeeded and
/// whose Lean runtime + module init has finished.
///
/// One `Lean` instance per process (Lean runtime is single-threaded
/// per `SPEC/canonical-abi.md §16`; `LEAN_RUNTIME_INIT` enforces
/// this with `std::sync::Once`). The `lean_dec` / `lean_io_*`
/// callbacks are cached as raw function pointers so dispatch doesn't
/// re-`dlsym` on every call.
pub struct Lean {
    #[allow(dead_code)]
    lib: Library,
    meta: HandshakeMeta,
    so_path: PathBuf,
    /// Cached function pointers for the runtime helpers `LeanRef`
    /// drop / IO error formatting need. Filled at `Lean::open` time.
    lean_dec_ref: LeanDecRefFn,
    /// `dlsym` cache for `leo4_call_<mangled>` entry points. Keyed by
    /// the mangled *body* (no `leo4_call_` prefix). Wrapped in
    /// `UnsafeCell` would suffice for `&mut self` accessors, but
    /// callers go through `&self`, so we use `std::sync::Mutex` to
    /// keep the cache concurrency-safe even though the Lean runtime
    /// itself is `!Sync`. The mutex is contended only on first call
    /// per mangled name.
    call_cache: std::sync::Mutex<HashMap<String, LeoCallShimFn>>,
}

// SAFETY: the cached function pointers are valid for the lifetime of
// `self.lib`. The `call_cache` Mutex makes concurrent inserts safe;
// running a leo4 dispatch from two threads at the same time still
// violates the Lean-runtime invariant (LEO4-DESIGN.md §16) and is
// the caller's responsibility. `LeanRef<'_, _>` remains `!Send +
// `!Sync`, so a fully type-checked program cannot end up there.
unsafe impl Send for Lean {}
unsafe impl Sync for Lean {}

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

        // Run the Lean init sequence once per process: runtime module
        // + the wrapper's `initialize_<wrapper_init_symbol>`, then
        // `lean_io_mark_end_initialization`. The wrapper init
        // transitively initialises `initialize_Init` and the user's
        // `initialize_<target_module>`, so we only need this single
        // call.
        //
        // Each `dlsym` is scoped in its own block so the
        // `libloading::Symbol` borrow ends before we move `lib` into
        // `Self`. Function pointers themselves are `Copy` and remain
        // valid as long as `lib` lives — which is for the lifetime
        // of the returned `Lean`.
        let init_runtime_fn: VoidFn = unsafe {
            let s: libloading::Symbol<VoidFn> = lib
                .get(b"lean_initialize_runtime_module\0")
                .map_err(|e| LeanError::host(format!("dlsym lean_initialize_runtime_module: {e}")))?;
            *s
        };
        let wrapper_init_name = format!("{}\0", meta.wrapper_init_symbol);
        let mod_init_fn: ModInitFn = unsafe {
            let s: libloading::Symbol<ModInitFn> = lib
                .get(wrapper_init_name.as_bytes())
                .map_err(|e| LeanError::host(format!("dlsym {}: {e}", meta.wrapper_init_symbol)))?;
            *s
        };
        let io_show_err_fn: LeanIoResultShowErrorFn = unsafe {
            let s: libloading::Symbol<LeanIoResultShowErrorFn> = lib
                .get(b"lean_io_result_show_error\0")
                .map_err(|e| LeanError::host(format!("dlsym lean_io_result_show_error: {e}")))?;
            *s
        };
        let dec_ref_cold_fn: LeanDecRefColdFn = unsafe {
            let s: libloading::Symbol<LeanDecRefColdFn> = lib
                .get(b"lean_dec_ref_cold\0")
                .map_err(|e| LeanError::host(format!("dlsym lean_dec_ref_cold: {e}")))?;
            *s
        };
        let end_init_fn: VoidFn = unsafe {
            let s: libloading::Symbol<VoidFn> = lib
                .get(b"lean_io_mark_end_initialization\0")
                .map_err(|e| LeanError::host(format!("dlsym lean_io_mark_end_initialization: {e}")))?;
            *s
        };

        // SAFETY: each fn pointer points into the shim's mapped code
        // segment, which stays mapped for as long as `lib` is held.
        // `LEAN_RUNTIME_INIT.call_once` ensures `lean_initialize_runtime_module`
        // runs exactly once per process.
        unsafe {
            LEAN_RUNTIME_INIT.call_once(|| init_runtime_fn());
            // builtin = 1: matches how lean -c-emitted main calls it.
            let res = mod_init_fn(1);
            if !lean_io_result_is_ok(res) {
                io_show_err_fn(res);
                lean_dec_ref_inline(res, dec_ref_cold_fn);
                end_init_fn();
                return Err(LeanError::host(format!(
                    "wrapper module init `{}` returned IO error (details printed to stderr)",
                    meta.wrapper_init_symbol
                )));
            }
            lean_dec_ref_inline(res, dec_ref_cold_fn);
            end_init_fn();
        }

        Ok(Self {
            lib,
            meta,
            so_path: so_path.as_ref().to_path_buf(),
            lean_dec_ref: dec_ref_cold_fn,
            call_cache: std::sync::Mutex::new(HashMap::new()),
        })
    }

    /// Dispatch one shim entry point by mangled body.
    ///
    /// `mangled_body` is the form stored in `<pkg>.leo4-mangling`'s
    /// `instantiations[*].mangled` field — the `leo4_call_` prefix is
    /// added internally before `dlsym`. `args` carries the
    /// canonical-ABI argument tuple; `ret` is the caller's
    /// return buffer. Returns the number of bytes written into `ret`
    /// (`*ret_len` from the C signature) or a [`LeanError`] for any
    /// non-zero status.
    ///
    /// On buffer-too-small (`LEO4_ERR_RETURN_BUF_TOO_SMALL = 7`) the
    /// shim writes the required size into `*ret_len`; callers can
    /// retry with a larger buffer. We surface the requested size in
    /// `LeanError.detail`.
    ///
    /// # Safety
    ///
    /// The caller is responsible for matching `mangled_body` to a
    /// signature whose argument tuple bytes are correctly encoded
    /// per `SPEC/canonical-abi.md`. `#[leo4::import]` (P5-b)
    /// handles this; raw users own the type discipline.
    pub fn call_shim(
        &self,
        mangled_body: &str,
        args: &[u8],
        ret: &mut [u8],
    ) -> LeanResult<usize> {
        let fn_ptr = self.dispatch_lookup(mangled_body)?;
        let mut ret_len: usize = 0;
        // SAFETY: the function was dlsym'd against the shim and its
        // signature matches `LeoCallShimFn` by construction (the
        // plugin generates exactly this shape per
        // `SPEC/canonical-abi.md §14`).
        let rc = unsafe {
            fn_ptr(
                std::ptr::null_mut(), // arena reserved
                args.as_ptr(),
                args.len(),
                ret.as_mut_ptr(),
                ret.len(),
                &mut ret_len,
            )
        };
        match rc {
            0 => Ok(ret_len),
            7 => Err(LeanError {
                code: 7,
                detail: format!(
                    "return buffer too small for `{mangled_body}`: need {ret_len} bytes, got {}",
                    ret.len()
                ),
            }),
            other => Err(LeanError {
                code: other,
                detail: format!("`leo4_call_{mangled_body}` returned status {other:#010x}"),
            }),
        }
    }

    fn dispatch_lookup(&self, mangled_body: &str) -> LeanResult<LeoCallShimFn> {
        {
            let cache = self
                .call_cache
                .lock()
                .map_err(|_| LeanError::host("call_cache mutex poisoned"))?;
            if let Some(p) = cache.get(mangled_body) {
                return Ok(*p);
            }
        }
        let sym_name = format!("leo4_call_{mangled_body}\0");
        // SAFETY: dlsym against a shim built by a compatible Lake
        // plugin version. The handshake at `Lean::open` time
        // confirmed the schema; missing symbols here imply
        // out-of-band tampering with the .so.
        let fn_ptr: LeoCallShimFn = unsafe {
            let s: libloading::Symbol<LeoCallShimFn> = self
                .lib
                .get(sym_name.as_bytes())
                .map_err(|e| LeanError::host(format!("dlsym leo4_call_{mangled_body}: {e}")))?;
            *s
        };
        let mut cache = self
            .call_cache
            .lock()
            .map_err(|_| LeanError::host("call_cache mutex poisoned"))?;
        cache.insert(mangled_body.to_string(), fn_ptr);
        Ok(fn_ptr)
    }

    /// Lower-level handle for callers that need to drop a Lean
    /// object themselves. Exposed for `#[leo4::import]`'s generated
    /// code; ordinary users should hold `LeanRef<'_, _>` and let its
    /// `Drop` handle it.
    #[doc(hidden)]
    #[must_use]
    pub fn lean_dec_ref_fn(&self) -> LeanDecRefFn {
        self.lean_dec_ref
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
/// [`Arena<'a>`]. `T` is a phantom type parameter; `#[leo4::import]`
/// (P5-b) wires it to the per-export wrapper types.
///
/// `LeanRef` is `!Send + !Sync` for the same reasons as `Arena<'_>`.
pub struct LeanRef<'a, T: ?Sized> {
    /// `lean_object *`. Opaque from this crate's perspective —
    /// `leo4-native` only passes it back into shim entry points.
    ptr: *mut c_void,
    /// Cached `lean_dec_ref` pointer from the `Lean` instance the
    /// handle was minted by. Stashing it on the handle means `Drop`
    /// doesn't have to carry a `&Lean` reference (which would
    /// re-introduce the `Sync` constraint we explicitly opt out of).
    dec_ref: LeanDecRefFn,
    _marker: PhantomData<(&'a (), fn() -> T)>,
}

impl<'a, T: ?Sized> LeanRef<'a, T> {
    /// Wrap a raw `lean_object*` for the given arena. Caller must
    /// ensure the pointer was minted by the same `Lean` instance
    /// `dec_ref` came from and that we now own one strong reference
    /// (every shim return path that produces a new lean object hands
    /// off ownership to the caller).
    ///
    /// # Safety
    ///
    /// `ptr` must be a valid `lean_object*` owned at refcount-≥1 by
    /// the caller; `dec_ref` must be the matching cached pointer.
    #[doc(hidden)]
    #[must_use]
    pub unsafe fn from_raw(_arena: &'a Arena<'a>, ptr: *mut c_void, dec_ref: LeanDecRefFn) -> Self {
        Self {
            ptr,
            dec_ref,
            _marker: PhantomData,
        }
    }

    /// Borrow the raw `lean_object*`. Stays valid as long as `self`
    /// is in scope; do not store the pointer beyond that point.
    #[doc(hidden)]
    #[must_use]
    pub fn as_ptr(&self) -> *mut c_void {
        self.ptr
    }
}

impl<T: ?Sized> std::fmt::Debug for LeanRef<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LeanRef")
            .field("ptr", &self.ptr)
            .finish()
    }
}

impl<T: ?Sized> Drop for LeanRef<'_, T> {
    fn drop(&mut self) {
        // SAFETY: `from_raw`'s preconditions guarantee the pointer is
        // valid + owned. `lean_dec_ref_inline` does the fast-path m_rc
        // decrement; the cached `self.dec_ref` is the cold-path
        // delegate (`lean_dec_ref_cold` from the same library load).
        unsafe { lean_dec_ref_inline(self.ptr, self.dec_ref) }
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
                 "target_module": "Sample", "schema_hash": "x",
                 "wrapper_init_symbol": "initialize_x"}"#,
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
                 "target_module": "Sample",
                 "wrapper_init_symbol": "initialize_x"}"#,
        )
        .unwrap();
        let err = parse_handshake(&path).unwrap_err();
        assert!(err.detail.contains("16"), "unexpected detail: {}", err.detail);
    }
}
