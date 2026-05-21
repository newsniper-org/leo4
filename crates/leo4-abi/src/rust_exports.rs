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
/// `static`, so all string fields are `&'static str`.
#[derive(Debug)]
pub struct ExportEntry {
    /// User-visible name. Today this is the Rust `fn` ident
    /// (e.g. `"solve_smt"`); Phase 9-2 may prefix it with a
    /// package / interface segment derived from
    /// `CARGO_PKG_NAME` when emitting `<pkg>.leo4-rust-exports.idl`.
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
