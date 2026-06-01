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
    /// suffix — `schema_hash` lives in the handshake JSON only;
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

// ─── RC.2 patch 2 (2026-05-31) — User-defined type schema ───────────
//
// `#[derive(LeanMarshal)]` on a struct or enum emits a static
// `UserTypeEntry` into the `USER_TYPES` distributed slice carrying
// the type's surface schema (fields for records, ctors+fields for
// variants/enums). leo4-rust-emit reads this slice via the FFI
// entry below and emits matching Lean `inductive` / `structure`
// mirror declarations into the wrapper file, so reverse-direction
// projects don't need to hand-write Lean mirror types.
//
// Wire model: each field carries its Rust type as source-text
// (whitespace-normalised, e.g. `"Vec<(String,String)>"`) plus the
// IDL-side mangle string. The wrapper emitter prefers the mangle
// (consistent with `mangle_type`) and falls back to the Rust-type
// string for diagnostics.

/// `UserTypeKind` discriminator for `UserTypeEntry`. `#[repr(u8)]`
/// keeps the C-ABI layout fixed across cdylib boundaries.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserTypeKind {
    /// Single-ctor struct with named fields (`pub struct Point { x: u32, y: u32 }`).
    Record = 0,
    /// Single-ctor struct with positional fields
    /// (`pub struct Wrapper(pub u64)` — Rust newtype/tuple-struct).
    /// Mangle pattern: same `S_<fqn>_s` as Record.
    TupleRecord = 1,
    /// Multi-ctor enum, possibly with payloads (the flagship
    /// typed-enum case — `pub enum AdsmtVerdict { Sat { … }, … }`).
    /// Mangle pattern: `V_<fqn>_v`.
    Variant = 2,
    /// Multi-ctor enum where every ctor is unit (C-style enum —
    /// `pub enum Color { Red, Green, Blue }`).
    /// Mangle pattern: `E_<fqn>_e`.
    UnitEnum = 3,
    /// Single unit variant — degenerate one-ctor `struct Foo;` or
    /// `enum Empty {}`. Rare, present for completeness.
    UnitStruct = 4,
}

/// One field inside a record / tuple-record, or one positional
/// payload inside a variant ctor.
#[repr(C)]
#[derive(Debug)]
pub struct FieldEntry {
    /// Lean field/binder name. Empty string `""` for positional
    /// (tuple-struct or tuple-variant) fields — wrapper emit
    /// falls back to `_arg0` / `_arg1` etc.
    pub name: &'static str,
    /// IDL-side mangle of the field type
    /// (`SPEC/mangling.md` §2 — `u32`, `L_str_l`, `O_u64_o`,
    /// `V_AdsmtVerdict_v`, etc.). Empty when the macro couldn't
    /// compute a mangle (proc-macro path doesn't have IDL access);
    /// the wrapper emitter falls back to `rust_type` in that case.
    pub type_mangle: &'static str,
    /// Source-text of the field's Rust type, whitespace-normalised
    /// (e.g. `"Vec<(String,String)>"`). Diagnostic for users when
    /// the mangle path can't synthesise a Lean type name.
    pub rust_type: &'static str,
}

/// One constructor inside a variant / unit-enum entry.
#[repr(C)]
#[derive(Debug)]
pub struct CtorEntry {
    /// Rust ctor identifier in source form (e.g. `"Sat"`,
    /// `"Unsat"`, `"Abductive"`, `"Unknown"`). Wrapper emit
    /// lowercases the first char to derive the Lean ctor name
    /// (Lean convention).
    pub name: &'static str,
    /// Field list for this ctor. Empty for unit-variant entries
    /// (`UnitEnum.ctors[i].fields == []`).
    pub fields: &'static [FieldEntry],
}

/// One row in the cdylib's user-defined-type schema table.
///
/// `#[derive(LeanMarshal)]` on a struct/enum emits exactly one of
/// these per nominal type into the `USER_TYPES` slice. leo4-rust-emit
/// walks the slice when rendering the wrapper Lean file and emits
/// a matching `inductive` / `structure` declaration for each entry,
/// so reverse-direction projects need zero hand-written mirror types.
#[repr(C)]
#[derive(Debug)]
pub struct UserTypeEntry {
    /// User-visible name (the Rust type's ident — e.g.
    /// `"AdsmtVerdict"`). Dots in the FQN are NOT preserved here
    /// because Rust types don't carry namespace dots; the wrapper
    /// emitter uses this verbatim as the Lean type name (no
    /// namespace prefix). User projects with namespace-qualified
    /// types layer it via Lean's `namespace` keyword in their
    /// mirror module — but as of RC.2 the derive macro emits
    /// flat names matching the Rust ident.
    pub fqn: &'static str,
    /// Schema kind discriminant. See [`UserTypeKind`].
    pub kind: UserTypeKind,
    /// Field list — populated for `Record` and `TupleRecord`;
    /// empty for the other kinds.
    pub fields: &'static [FieldEntry],
    /// Constructor list — populated for `Variant` and `UnitEnum`;
    /// empty for the other kinds.
    pub ctors: &'static [CtorEntry],
}

/// All user-defined nominal types in this cdylib that carry a
/// `#[derive(LeanMarshal)]`. Empty on cdylibs that don't use the
/// reverse direction. Populated at link time by `linkme`.
#[distributed_slice]
pub static USER_TYPES: [UserTypeEntry] = [..];

/// `extern "C"` entry exposing the `USER_TYPES` slice. Same shape +
/// safety contract as `leo4_rust_describe_exports`.
///
/// # Safety
///
/// `out_ptr` and `out_len` must be valid writable pointers. The
/// returned pointer is valid for the lifetime of the cdylib and
/// the entries' `&'static str` fields point into the cdylib's
/// constant pool — do not retain the data across `dlclose`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn leo4_rust_describe_user_types(
    out_ptr: *mut *const UserTypeEntry,
    out_len: *mut usize,
) -> i32 {
    if out_ptr.is_null() || out_len.is_null() {
        return 1;
    }
    let slice: &[UserTypeEntry] = &USER_TYPES;
    // SAFETY: callers must pass valid out-pointers (documented above).
    unsafe {
        *out_ptr = slice.as_ptr();
        *out_len = slice.len();
    }
    0
}
