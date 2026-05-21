//! Phase 9 reverse-direction — distributed-slice registry for
//! `#[leo4::export]`-tagged Rust functions.
//!
//! Gated behind the `rust-exports` cargo feature so stable
//! workspace builds stay free of the `linkme` dependency.
//!
//! See `SPEC/reverse-direction.md` §1, §2, §8.
//!
//! ## Usage
//!
//! Users never touch this module directly. The
//! `#[leo4::export]` proc-macro emits the metadata entries and
//! the wrapper shims; the build script
//! (`leo4-build::wire_rust_exports`, Phase 9-2) reads
//! [`EXPORTS`] when emitting `<pkg>.leo4-rust-exports.idl` and
//! `<pkg>.leo4-rust-handshake`.
//!
//! ## Layout
//!
//! Each tagged function contributes one [`ExportEntry`] to the
//! [`EXPORTS`] distributed slice via `linkme`. The macro emits
//! the entry verbatim — no allocation, no init order; the
//! slice is assembled by the linker.

use linkme::distributed_slice;

/// One row in the cdylib's reverse-direction export table.
///
/// Static lifetime everywhere: the macro emits the entry as a
/// `static`, so all string fields are `&'static str`. The
/// `#[repr(C)]` is so external tools (the `leo4-rust-emit` CLI,
/// and later the worker harness) can read entries via FFI by
/// `dlopen`ing the cdylib — Rust's default repr does not
/// guarantee field order or layout, but `repr(C)` does.
#[repr(C)]
#[derive(Debug)]
pub struct ExportEntry {
    /// User-visible name. Today this is the Rust `fn` ident
    /// (e.g. `"solve_smt"`); Phase 9-2's emit CLI may prefix it
    /// with a package / interface segment derived from
    /// `CARGO_PKG_NAME` when writing `<pkg>.leo4-rust-exports.idl`.
    pub logical_name: &'static str,

    /// The C-linkage symbol the dispatcher reaches via
    /// `dlsym` / `GetProcAddress`. Format:
    /// `leo4_rust__<fname>__<param_mangles>` (no `__h<hash>`
    /// suffix — schema_hash lives in the handshake JSON only;
    /// see `SPEC/reverse-direction.md` §2).
    pub mangled: &'static str,

    /// Parameter type mangles in declaration order, each
    /// produced by `mangle_type` (`SPEC/mangling.md` §2).
    /// Empty for zero-arg functions.
    pub param_types: &'static [&'static str],

    /// Return type mangle. `""` for functions returning unit.
    pub ret_type: &'static str,

    /// Set if `#[leo4::export(isolated)]` — dispatcher runs
    /// this function in a fresh worker per call rather than the
    /// persistent one (`SPEC/reverse-direction.md` §4.2).
    pub isolated: bool,

    /// ABI version this entry was emitted against. Matches
    /// `<pkg>.leo4-rust-handshake`'s `abi_version`. Currently
    /// always `1`.
    pub abi_version: u32,
}

/// All `#[leo4::export]` functions in this cdylib.
///
/// Populated at link time by `linkme`; safe to iterate from any
/// `extern "C"` symbol or build script that loads the cdylib.
/// Empty on cdylibs that use leo4 only in the forward direction.
#[distributed_slice]
pub static EXPORTS: [ExportEntry] = [..];

// ─── FFI entry for external introspection (Phase 9-2) ────────────────
//
// The `leo4-rust-emit` CLI and (later) the worker harness load the
// cdylib via `dlopen` and need a way to walk its `EXPORTS` slice
// without resolving `linkme`'s private internals. The function
// below is the stable, repr-C-safe gateway: it writes the slice's
// in-process pointer and length into caller-provided out params.
//
// Safety contract: the returned pointer is valid for the lifetime
// of the cdylib (i.e. until `dlclose`). The `&'static str` fields
// inside each `ExportEntry` likewise point into the cdylib's
// .rodata. Callers must not retain pointers across `dlclose`.

/// `extern "C"` entry exposing the `EXPORTS` slice to in-process
/// `dlopen`-style consumers. Always emitted when the
/// `rust-exports` feature is enabled.
///
/// # Safety
///
/// `out_ptr` and `out_len` must be valid writable pointers. The
/// returned pointer is valid for the lifetime of the cdylib and
/// the entries' `&'static str` fields point into the cdylib's
/// constant pool — do not retain the data across `dlclose`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn leo4_rust_describe_exports(
    out_ptr: *mut *const ExportEntry,
    out_len: *mut usize,
) -> i32 {
    if out_ptr.is_null() || out_len.is_null() {
        return 1; // null-arg error
    }
    let slice: &[ExportEntry] = &EXPORTS;
    // SAFETY: callers must pass valid out-pointers (documented above).
    unsafe {
        *out_ptr = slice.as_ptr();
        *out_len = slice.len();
    }
    0
}
