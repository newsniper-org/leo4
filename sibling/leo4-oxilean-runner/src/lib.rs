//! leo4-oxilean-runner — production-form runner helper for
//! `leo4 run --impl rust-transpile` (reverse direction).
//!
//! The OX8.5 scaffold's `src/main.rs` used to be a hand-
//! editable placeholder: instantiate an `OxiLeanInvoker`,
//! `eprintln!` a TODO list, exit. This crate folds the four
//! placeholder bullet points into a single
//! [`run_main`] entry point users invoke from their
//! scaffolded runner binary:
//!
//! 1. `libloading::Library::new(cdylib_path)` opens the user
//!    crate's cdylib.
//! 2. The cdylib's `leo4_rust_describe_exports` symbol (FFI
//!    entry declared in `crates/leo4-abi/src/rust_exports.rs`)
//!    yields a `*const ExportEntry` + length. We walk that
//!    slice without re-implementing `linkme`'s discovery —
//!    `repr(C)` makes this stable across cdylib builds.
//! 3. Each entry is paired-registered with the invoker:
//!    [`OxiLeanInvoker::register_export`] for metadata (so
//!    the OxiLean kernel's `ExternRegistry` knows the
//!    symbol) and [`OxiLeanInvoker::register_export_callback`]
//!    for the runtime closure (a thin `dlsym` wrapper that
//!    calls into the cdylib's `extern "C"` wrapper symbol
//!    with canonical-ABI byte buffers).
//! 4. The user's `lean/Main.lean` is parsed and elaborated
//!    against an `Environment` bootstrapped via
//!    [`leo4_oxilean_build::leo4_env_bootstrap::bootstrap_env`]
//!    (OxiLean prelude + leo4 boundary primitives) plus the
//!    `@[extern]` axioms the OX8.2 wrapper emit produced.
//!
//! ## `main : IO α` execution — walker coverage
//!
//! OxiLean v0.1.3-leo4-ox7 (the fork branch this crate
//! tracks) ships `oxilean_runtime::driver::run_main` —
//! an IO walker that drives an elaborated `main : IO α`
//! to its IO effects under an installed `ExternResolver`
//! + `ExternRegistry`. As of fork commit `469ffea`
//! (2026-05-31) the walker recognises:
//!
//! - The full monad transformer family at the `IO`
//!   leaf — `IO.pure` / `Pure.pure` / `EIO.pure` /
//!   `EStateM.pure` / `ExceptT.pure` / `StateT.pure` /
//!   `ReaderT.pure` (and the underscore-mangled
//!   spellings) as terminals; matching `*.bind` siblings
//!   as monadic sequence.
//! - `IO.bind m k` beta-application — when `m` walks to
//!   a concrete result and `k` is a `Lam`, the walker
//!   substitutes and walks the body.
//! - `@[extern]`-attributed `Const` reductions via
//!   `dispatch_extern_const`, with full canonical-ABI
//!   arg encoding (literals + Bool + Unit + sized-integer
//!   typeclass projections via `OfNat.ofNat` / `Neg.neg` /
//!   `Char.ofNat` + composite ctors `Prod.mk` /
//!   `Subtype.mk` / `Option.some` / `Sum.inl` + **user-
//!   defined record / inductive ctors via env-lookup of
//!   `ConstantInfo::Constructor`**).
//! - Stdlib `IO.println` / `IO.eprintln` / `IO.print` /
//!   `IO.eprint` direct dispatch against `print!` /
//!   `println!` / `eprint!` / `eprintln!`, ahead of the
//!   `@[extern]` resolver arm.
//! - Stdlib `IO.FS.*` family (read / write / append /
//!   remove / create / rename) direct dispatch against
//!   `std::fs`. `readFile` surfaces contents as
//!   `Ok(Some(Lit(Str)))` so `IO.bind m k` beta-applies
//!   `k` against the bytes; IO errors wrap into
//!   `DriverError::ExternFailed`.
//!
//! Shapes outside the recognised set are *out-of-scope by
//! design*, not implementation gaps: non-IO monad-class
//! run projections (`StateT.run` / `ReaderT.run` /
//! `ExceptT.run` belong at the LCNF / bytecode
//! interpreter layer below the walker), `IO.FS.Handle.*`
//! (needs host-side `File` lifetime modelling the walker
//! doesn't carry), compile-time hooks (`dbg_trace` /
//! `panic!` / `unreachable!` — elaborator-handled), and
//! float-literal lowering (constant-folded by reducer).
//! Each surfaces a clean [`LeanError`] with code
//! `0x0002_0005` (`RUST_DLSYM_FAILED`) carrying the
//! offending sub-expression's debug repr.
//!
//! The intermediate pipeline steps — cdylib load, EXPORTS
//! walk, invoker registration, parse, elab,
//! `ExternResolver` installation — are individually
//! observable via [`run_main_diagnostics`] for testing.
//! Outbound callback dispatch (Lean closure dereferences
//! firing registered Rust closures) goes through
//! [`OxiLeanInvoker::register_outbound_dispatch_callback`]
//! + the matching wrapper-Lean `@[extern]` symbol; pair
//! that call with `attach_outbound_registry(...)` at
//! adapter init time.
//!
//! The driver's API shape is under discussion upstream
//! at <https://github.com/cool-japan/oxilean/issues/2>;
//! upstream-PR submission (the API surface + the
//! recognised-shape set in the fork's
//! `oxilean_runtime/driver/mod.rs`) lands once that
//! discussion settles. As of 2026-05-31 the issue has
//! been silent 3+ days.

#![allow(clippy::missing_errors_doc)]
// FFI / type-name-in-docs noise — backticking every
// `LeanError` / `Library` / `Arc<…>` mention in this
// crate's docs would obscure the prose; leo4-oxilean
// itself takes the same stance.
#![allow(clippy::doc_markdown)]
// The `register_one_export` closure block legitimately
// declares `RET_BUF_CAP` next to where it's used.
#![allow(clippy::items_after_statements)]
// `Declaration::Axiom` is the right collapsing for both
// `PendingDecl::Axiom` and `PendingDecl::Inductive` until
// the runner grows a real inductive-installation path
// (matches oxilean-cli's own wrapper).
#![allow(clippy::match_same_arms)]

use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;

// `error_codes::DECODE_ERROR` / `ENCODE_ERROR` are used in
// the parse/elab error mapping; the runner-specific
// `RUST_DLSYM_FAILED` (0x0002_0005) is defined inline below
// because `leo4-abi`'s `error_codes` module doesn't yet
// expose it as a named constant (cf. `crates/leo4-abi/src/
// error.rs` — only DECODE_ERROR..DECODE_DEPTH_EXCEEDED).
use leo4_abi::error::error_codes;
use leo4_abi::rust_exports::{ExportEntry, UserTypeEntry};
use leo4_abi::LeanError;

/// Domain-specific error code for "runner could not complete
/// the dispatch path" — matches the value `leo4-oxilean` uses
/// in its own stub diagnostics (0x0002_0005). This mirrors
/// the convention there is no canonical "rust dlsym failed"
/// in `error_codes` yet; cf. `SPEC/canonical-abi.md` §13.
const RUST_DLSYM_FAILED: u32 = 0x0002_0005;

use leo4_oxilean::OxiLeanInvoker;

mod env_bootstrap;
pub use env_bootstrap::bootstrap_env;

// OX8 follow-up #79 (2026-05-28). Vendored copy of
// `sibling/leo4-oxilean-build/src/leo4_translate.rs` so the
// runner's parse step can take the PEG path (oxilean-parse-peg
// → translate → oxilean-parse Decl) the same way the build
// crate does. Lifts the legacy parser's limit on `s!"…"`
// string interpolation that the OX8.5 scaffold's stock
// Main.lean template uses.
//
// Duplicated from leo4-oxilean-build because that crate is
// binary-only with `[patch.crates-io]` (see commit `f90ee50`
// patch-policy lock) — we can't safely import it as a library
// here. Task #78 tracks extracting both this vendor + the
// `env_bootstrap.rs` vendor into a shared
// `leo4-oxilean-bootstrap` leaf crate.
#[allow(clippy::all, clippy::pedantic, dead_code)]
mod leo4_translate;

use oxilean_elab::{
    elaborate_decl as elab_decl_impl, ElabContext, PendingDecl,
};
use oxilean_kernel::{check_declaration, Declaration, Environment, ReducibilityHint};
use oxilean_parse::{Lexer, Parser};

/// Signature of the cdylib's `leo4_rust_describe_exports`
/// FFI entry — mirrors the declaration in
/// `crates/leo4-abi/src/rust_exports.rs`.
type DescribeExportsFn =
    unsafe extern "C" fn(*mut *const ExportEntry, *mut usize) -> i32;

/// RC.5 (2026-05-31) — signature of the cdylib's
/// `leo4_rust_describe_user_types` FFI entry. Mirrors the
/// declaration in `crates/leo4-abi/src/rust_exports.rs`.
type DescribeUserTypesFn =
    unsafe extern "C" fn(*mut *const UserTypeEntry, *mut usize) -> i32;

/// Signature of each individual `#[leo4::export]`-wrapped
/// symbol that the leo4 proc macro emits inside the cdylib.
/// Documented at `crates/leo4-macros-backend/src/lib.rs`
/// (search `pub unsafe extern "C" fn #wrapper_ident`).
///
/// `args_ptr` + `args_len` are the canonical-ABI encoded
/// argument bytes. `ret_ptr` + `ret_cap` is a caller-
/// allocated buffer the wrapper writes the encoded return
/// into; `ret_len` reports how many bytes were actually
/// produced (>= `ret_cap` if `BUFFER_TOO_SMALL`). The
/// `i32` return code is `0` on success, a leo4 error
/// code otherwise.
type WrapperFn = unsafe extern "C" fn(
    args_ptr: *const u8,
    args_len: usize,
    ret_ptr: *mut u8,
    ret_cap: usize,
    ret_len: *mut usize,
) -> i32;

/// Diagnostic envelope returned by [`run_main_diagnostics`].
/// Each field is non-fatally settable so partial-success
/// cases (e.g. cdylib loaded + EXPORTS walked but Main.lean
/// missing) still surface what *did* work.
#[derive(Debug, Default)]
pub struct RunDiagnostics {
    /// Number of `ExportEntry` rows walked from the
    /// cdylib's `leo4_rust_describe_exports`.
    pub exports_seen: usize,
    /// Number of exports successfully paired-registered
    /// (`register_export` + `register_export_callback`)
    /// with the invoker.
    pub exports_registered: usize,
    /// Number of top-level declarations parsed from
    /// `lean/Main.lean`.
    pub decls_parsed: usize,
    /// Number of those declarations that successfully
    /// elaborated *and* type-checked against the env.
    pub decls_elaborated: usize,
    /// `true` iff the env was successfully bootstrapped via
    /// `leo4_env_bootstrap::bootstrap_env`.
    pub env_bootstrapped: bool,
    /// `true` iff `OxiLeanInvoker::as_shared_resolver()`
    /// produced a resolver and the cdylib + EXPORTS walk
    /// completed without error. Pre-condition for an
    /// upstream `run_main` driver to actually fire.
    pub resolver_ready: bool,
    /// RC.5 (2026-05-31) — number of `UserTypeEntry` rows
    /// walked from the cdylib's
    /// `leo4_rust_describe_user_types`. Each entry surfaces
    /// as a stub `Axiom <fqn> : Type` declaration in the env
    /// so Main.lean references like `def foo : AdsmtVerdict
    /// → … := …` elaborate name-resolution-wise. (Full
    /// inductive declarations with constructors + pattern-
    /// match support require parsing the auto-generated
    /// wrapper Lean file alongside Main.lean — outside the
    /// runner's scope. The wrapper contains the
    /// `inductive`/`structure` with `deriving Leo4.LeanMarshal`;
    /// users running `leo4 run` get the full path. This stub
    /// covers the direct runner-as-library usage.)
    pub user_types_seen: usize,
}

/// Production-form runner entry point.
///
/// Steps end-to-end:
///
/// 1. `libloading::Library::new(cdylib_path)`.
/// 2. Resolve `leo4_rust_describe_exports`, walk the slice
///    (cdylib retains ownership of the entries — pointers
///    into its `.rodata`).
/// 3. For each entry, `OxiLeanInvoker::register_export`
///    (metadata) + `register_export_callback` (a closure
///    holding an `Arc<Library>` so the cdylib stays loaded
///    for the closure's lifetime; the closure `dlsym`s the
///    mangled wrapper symbol *once* on first call and
///    caches the function pointer in an `OnceLock`).
/// 4. Parse `main_lean_path` via a `Lexer` + `Parser`
///    loop (mirroring `oxilean-cli`'s `check_source`).
/// 5. Bootstrap an env (`bootstrap_env`) and `elaborate_decl`
///    + `check_declaration` each parsed decl into it.
/// 6. Install the invoker as the env's
///    `SharedExternResolver` (via
///    `Arc::clone(&invoker).as_shared_resolver()`).
/// 7. Drive `main : IO α` via
///    [`oxilean_runtime::driver::run_main`] under the
///    invoker's `ExternResolver` + `ExternRegistry` handle.
///    The walker currently recognises `IO.pure`, `IO.bind`
///    (arity-4 + arity-2), and `@[extern]` Const dispatch;
///    unrecognised shapes surface as
///    `LeanError(0x0002_0005)` carrying the offending
///    sub-expression's debug repr. Outbound callback
///    dispatch (Lean closure dereference firing a
///    registered Rust closure) goes through
///    [`OxiLeanInvoker::register_outbound_dispatch_callback`]
///    + the matching wrapper-Lean `@[extern]` symbol.
///
/// The cdylib + invoker remain alive for the duration of
/// the call; the `Arc<Library>` inside each registered
/// callback holds the cdylib loaded across calls without
/// requiring `Library::leak`.
pub fn run_main(
    cdylib_path: &Path,
    main_lean_path: &Path,
) -> Result<(), LeanError> {
    let (invoker, env, _diag) = run_main_inner(cdylib_path, main_lean_path)?;
    // OX8 follow-up #76 (2026-05-28..29): fork-side
    // `driver::run_main` recognises `IO.pure`, `IO.bind`
    // (arity-4 + arity-2), and `@[extern]` Const
    // dispatch via `dispatch_extern_const`. Unrecognised
    // shapes (e.g. `EStateM Error IO.RealWorld α`
    // lowerings, beta-application of `k` in `IO.bind m k`
    // with a concrete result feed) still surface as
    // `LeanError(0x0002_0005)` carrying the offending
    // sub-expression's debug repr, so the gap is
    // visible per-shape.
    let main_name = oxilean_kernel::Name::str("main");
    // Fork commit `d357a01` (2026-05-29) added the
    // `extern_registry: &ExternRegistry` param so the
    // walker can run `dispatch_extern_const` against the
    // canonical "is this @[extern]-backed?" lookup. Grab
    // the invoker's metadata registry handle (kept in
    // lock-step with `register_export(...)` calls).
    let registry_handle = invoker.registry_handle();
    let extern_registry_guard = registry_handle
        .lock()
        .map_err(|e| LeanError::new(
            RUST_DLSYM_FAILED,
            format!(
                "leo4-oxilean-runner: ExternRegistry mutex poisoned at \
                 driver::run_main call site: {e}"
            ),
        ))?;
    let resolver = Arc::clone(&invoker).as_shared_resolver();
    match oxilean_runtime::driver::run_main(&env, &extern_registry_guard, resolver, &main_name) {
        Ok(()) => Ok(()),
        Err(oxilean_runtime::driver::DriverError::NotYetImplemented { reason }) => {
            Err(LeanError::new(
                RUST_DLSYM_FAILED,
                format!(
                    "leo4-oxilean-runner: cdylib + EXPORTS + invoker + parse \
                     + elab all wired, and `oxilean_runtime::driver::run_main` \
                     reached — walker hit an unrecognised shape \
                     ({reason}). Main.lean = `{}`",
                    main_lean_path.display(),
                ),
            ))
        }
        Err(e) => Err(LeanError::new(
            RUST_DLSYM_FAILED,
            format!(
                "leo4-oxilean-runner: driver::run_main returned {e}. \
                 Main.lean = `{}`",
                main_lean_path.display(),
            ),
        )),
    }
}

/// Like [`run_main`] but returns a [`RunDiagnostics`]
/// describing how far the pipeline got even on the
/// upstream-blocked final step. Intended for the e2e smoke
/// test below and for users who want to introspect the
/// wiring before the upstream driver lands.
pub fn run_main_diagnostics(
    cdylib_path: &Path,
    main_lean_path: &Path,
) -> Result<RunDiagnostics, LeanError> {
    let (_invoker, _env, diag) =
        run_main_inner(cdylib_path, main_lean_path)?;
    Ok(diag)
}

/// Shared core for [`run_main`] + [`run_main_diagnostics`].
/// Returns the live `OxiLeanInvoker`, the populated
/// `Environment`, and the diagnostic counters.
fn run_main_inner(
    cdylib_path: &Path,
    main_lean_path: &Path,
) -> Result<(Arc<OxiLeanInvoker>, Environment, RunDiagnostics), LeanError> {
    let mut diag = RunDiagnostics::default();

    // ── Step 1: dlopen the cdylib. The `Arc` keeps the
    //    Library alive for as long as the runner's closures
    //    (registered into the invoker) need it.
    // SAFETY: the cdylib path comes from `LEO4_OXILEAN_CDYLIB`
    // / the user's --cdylib arg; cargo built it from
    // `crate-type = ["cdylib"]` in the scaffold's
    // Cargo.toml, so its module-init code is benign.
    let lib = unsafe { libloading::Library::new(cdylib_path) }.map_err(|e| {
        LeanError::new(
            RUST_DLSYM_FAILED,
            format!(
                "leo4-oxilean-runner: dlopen({}) failed: {e}",
                cdylib_path.display()
            ),
        )
    })?;
    let lib = Arc::new(lib);

    // ── Step 2: walk EXPORTS via `leo4_rust_describe_exports`.
    let entries = unsafe { describe_exports(&lib) }?;
    diag.exports_seen = entries.len();

    // ── Step 3: pair-register every export.
    let invoker = Arc::new(OxiLeanInvoker::new());
    for entry in entries {
        register_one_export(&invoker, &lib, entry)?;
        diag.exports_registered += 1;
    }
    diag.resolver_ready = true;

    // ── Step 4 + 5: parse + elab Main.lean.
    let env_result = bootstrap_env().map_err(|e| {
        LeanError::new(
            error_codes::ENCODE_ERROR,
            format!("leo4-oxilean-runner: bootstrap_env failed: {e}"),
        )
    });
    let mut env = match env_result {
        Ok(env) => {
            diag.env_bootstrapped = true;
            env
        }
        Err(e) => return Err(e),
    };

    // RC.5 (2026-05-31) — walk USER_TYPES alongside EXPORTS.
    // RC.6 F4 (2026-05-31) lifts the stub axiom synth to
    // `ConstantInfo::Inductive` + per-ctor `ConstantInfo::Constructor`
    // so Main.lean references to user-defined types AND their
    // constructors resolve by name. Ctor types are the
    // trivial `Const(parent)` shape (no Pi-chain over field
    // args) — field-arity-aware type-checking still requires
    // wrapper-Lean-file loading; the `leo4 run` happy path
    // loads the wrapper file alongside Main.lean. Cdylibs
    // pre-dating RC.2's schema channel (no
    // `leo4_rust_describe_user_types` symbol) report a
    // missing symbol — treated as no-op, not fatal.
    let user_types = unsafe { describe_user_types(&lib) }.unwrap_or(&[]);
    diag.user_types_seen = user_types.len();
    synthesise_user_type_decls(&mut env, user_types);

    let main_src = std::fs::read_to_string(main_lean_path).map_err(|e| {
        LeanError::new(
            RUST_DLSYM_FAILED,
            format!(
                "leo4-oxilean-runner: cannot read main_lean_path `{}`: {e}",
                main_lean_path.display()
            ),
        )
    })?;

    elaborate_main_lean(&main_src, &mut env, &mut diag)?;

    // ── Step 6 left implicit: the OxiLean kernel's
    //    Environment does not (in v0.1.3-leo4-ox7) carry a
    //    `SharedExternResolver` slot. The resolver is
    //    plumbed through to `dispatch_extern_const` by the
    //    evaluator driver — which is exactly the missing
    //    upstream piece. We hand the live `Arc<OxiLeanInvoker>`
    //    back to the caller so once the driver lands the
    //    wire-up is a one-line addition.

    Ok((invoker, env, diag))
}

/// RC.6 F4 (2026-05-31) — synthesise kernel `ConstantInfo`
/// declarations for each `UserTypeEntry` so Main.lean
/// references to user-defined types AND their constructors
/// resolve by name. Per entry:
///
/// - One `ConstantInfo::Inductive(InductiveVal)` for the type
///   itself, with `ctors` listing the ctor names.
/// - One `ConstantInfo::Constructor(ConstructorVal)` per ctor,
///   with `cidx`, `num_fields`, and a trivial `Const(parent)`
///   common.ty (no Pi-chain over field types).
///
/// The trivial ctor type means `AdsmtVerdict.sat` resolves
/// as a constant of type `AdsmtVerdict`, but
/// `AdsmtVerdict.sat <field_args>` won't type-check against
/// the field arity. Users hitting that case load the wrapper
/// Lean file alongside Main.lean (the `leo4 run` path emits
/// the wrapper with full `inductive` decls including ctor
/// arg signatures + `deriving Leo4.LeanMarshal` — that path
/// is the fully-typed one). Direct-library callers using
/// `run_main` against `USER_TYPES` alone get name resolution
/// + ctor enumeration; field-arity-aware type-checking stays
/// a follow-up.
///
/// `Record` / `TupleRecord` / `UnitStruct` synthesise a
/// single ctor named `<fqn>.mk` mirroring Lean 4 stdlib
/// convention.
fn synthesise_user_type_decls(
    env: &mut Environment,
    user_types: &[UserTypeEntry],
) {
    use oxilean_kernel::{
        ConstantInfo, ConstantVal, ConstructorVal, Expr, InductiveVal, Level, Name,
    };
    use leo4_abi::rust_exports::UserTypeKind;

    let type_sort = Expr::Sort(Level::succ(Level::zero()));

    for ut in user_types {
        let parent_name = Name::str(ut.fqn);
        // Collect ctor names + per-ctor field counts.
        let ctor_specs: Vec<(Name, u32)> = match ut.kind {
            UserTypeKind::Record
            | UserTypeKind::TupleRecord
            | UserTypeKind::UnitStruct => {
                // Single anonymous ctor `<fqn>.mk` with
                // num_fields = entry.fields.len().
                let ctor = Name::str(&format!("{}.mk", ut.fqn));
                vec![(ctor, ut.fields.len() as u32)]
            }
            UserTypeKind::Variant | UserTypeKind::UnitEnum => {
                ut.ctors
                    .iter()
                    .map(|c| {
                        // Lean stdlib convention: lowercase
                        // first char on ctor name (matches
                        // the wrapper-file emit's
                        // `lowercase_first`).
                        let lc = lowercase_first(c.name);
                        let ctor =
                            Name::str(&format!("{}.{lc}", ut.fqn));
                        (ctor, c.fields.len() as u32)
                    })
                    .collect()
            }
        };

        let inductive = ConstantInfo::Inductive(InductiveVal {
            common: ConstantVal {
                name: parent_name.clone(),
                level_params: Vec::new(),
                ty: type_sort.clone(),
            },
            num_params: 0,
            num_indices: 0,
            all: vec![parent_name.clone()],
            ctors: ctor_specs.iter().map(|(n, _)| n.clone()).collect(),
            num_nested: 0,
            is_rec: false,
            is_unsafe: false,
            is_reflexive: false,
            is_prop: false,
        });
        // DuplicateDeclaration ignored — user Lean source
        // might separately declare the same inductive
        // (common when the user has both a hand-written and
        // a USER_TYPES-derived path).
        let _ = env.add_constant(inductive);

        for (cidx, (ctor_name, num_fields)) in ctor_specs.iter().enumerate() {
            let ctor = ConstantInfo::Constructor(ConstructorVal {
                common: ConstantVal {
                    name: ctor_name.clone(),
                    level_params: Vec::new(),
                    // Trivial ctor type: just the parent
                    // inductive's `Const(name)` — no Pi-
                    // chain over field types. The wrapper-
                    // file emit covers the fully-typed
                    // ctor signature; this stub gets us
                    // name resolution.
                    ty: Expr::Const(parent_name.clone(), Vec::new()),
                },
                induct: parent_name.clone(),
                cidx: cidx as u32,
                num_params: 0,
                num_fields: *num_fields,
                is_unsafe: false,
            });
            let _ = env.add_constant(ctor);
        }
    }
}

/// Lowercase the first ASCII alphabetic char in `s`. Mirrors
/// the wrapper emit's `lowercase_first` so ctor names match
/// between the runtime synth and the wrapper file.
fn lowercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_ascii_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// RC.5 (2026-05-31) — resolve and call
/// `leo4_rust_describe_user_types`; return a borrowed slice
/// view of the USER_TYPES entries. Cdylibs pre-dating the RC.2
/// schema channel surface as `LeanError(RUST_DLSYM_FAILED)` —
/// caller treats that as "no user types" rather than fatal.
///
/// # Safety
/// Caller must keep `lib` alive for the lifetime of the
/// returned slice — same lifetime contract as
/// [`describe_exports`].
unsafe fn describe_user_types(
    lib: &libloading::Library,
) -> Result<&[UserTypeEntry], LeanError> {
    let symbol: libloading::Symbol<'_, DescribeUserTypesFn> = unsafe {
        lib.get(b"leo4_rust_describe_user_types\0")
    }
    .map_err(|e| {
        LeanError::new(
            RUST_DLSYM_FAILED,
            format!(
                "leo4-oxilean-runner: dlsym(leo4_rust_describe_user_types) \
                 failed: {e}. Pre-RC.2 cdylibs lack this symbol; the runner \
                 falls back to no user-type axiom synthesis."
            ),
        )
    })?;

    let mut ptr: *const UserTypeEntry = std::ptr::null();
    let mut len: usize = 0;
    // SAFETY: stack-local outparams; signature matches.
    let rc = unsafe { symbol(&raw mut ptr, &raw mut len) };
    if rc != 0 {
        return Err(LeanError::new(
            RUST_DLSYM_FAILED,
            format!(
                "leo4-oxilean-runner: leo4_rust_describe_user_types returned \
                 non-zero status {rc}"
            ),
        ));
    }
    if len == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() {
        return Err(LeanError::new(
            RUST_DLSYM_FAILED,
            "leo4-oxilean-runner: leo4_rust_describe_user_types reported \
             non-zero len but null pointer"
                .to_string(),
        ));
    }
    // SAFETY: the cdylib's USER_TYPES lives in .rodata for the
    // lifetime of `lib`; caller bound to that.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

/// Resolve and call `leo4_rust_describe_exports`; return a
/// borrowed slice view of the EXPORTS entries.
///
/// # Safety
/// Caller must keep `lib` alive for the lifetime of the
/// returned slice — the entries' `&'static str` fields are
/// pointers into the cdylib's `.rodata` and become dangling
/// after `dlclose`.
unsafe fn describe_exports(
    lib: &libloading::Library,
) -> Result<&[ExportEntry], LeanError> {
    // SAFETY: `leo4_rust_describe_exports` is declared
    // `#[unsafe(no_mangle)] pub unsafe extern "C" fn` in
    // `leo4-abi`; the signature exactly matches
    // `DescribeExportsFn`. The cdylib carries it iff the
    // `rust-exports` feature was enabled (scaffold sets
    // `features = ["rust-exports"]` on the `leo4` dep).
    let symbol: libloading::Symbol<'_, DescribeExportsFn> = unsafe {
        lib.get(b"leo4_rust_describe_exports\0")
    }
    .map_err(|e| {
        LeanError::new(
            RUST_DLSYM_FAILED,
            format!(
                "leo4-oxilean-runner: dlsym(leo4_rust_describe_exports) \
                 failed: {e}. The cdylib was likely built without the \
                 `rust-exports` cargo feature — check your Cargo.toml's \
                 `leo4 = {{ …, features = [\"rust-exports\"] }}`."
            ),
        )
    })?;

    let mut ptr: *const ExportEntry = std::ptr::null();
    let mut len: usize = 0;
    // SAFETY: pointers to local stack slots; the FFI fn
    // contract is `out_ptr` + `out_len` writable.
    let rc = unsafe { symbol(&raw mut ptr, &raw mut len) };
    if rc != 0 {
        return Err(LeanError::new(
            RUST_DLSYM_FAILED,
            format!(
                "leo4-oxilean-runner: leo4_rust_describe_exports returned \
                 non-zero status {rc}"
            ),
        ));
    }
    if len == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() {
        return Err(LeanError::new(
            RUST_DLSYM_FAILED,
            "leo4-oxilean-runner: leo4_rust_describe_exports reported \
             non-zero len but null pointer"
                .to_string(),
        ));
    }
    // SAFETY: the cdylib's EXPORTS lives in .rodata for the
    // lifetime of `lib`; caller bound to that.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

/// Pair-register one export: metadata into the invoker's
/// `ExternRegistry`, runtime closure into its
/// `CallbackRegistry`. The closure captures an
/// `Arc<libloading::Library>` so the cdylib stays loaded;
/// the wrapper function pointer is resolved on the first
/// call and cached.
fn register_one_export(
    invoker: &Arc<OxiLeanInvoker>,
    lib: &Arc<libloading::Library>,
    entry: &ExportEntry,
) -> Result<(), LeanError> {
    invoker.register_export(entry.mangled)?;

    // Capture-by-clone for the closure. We cannot capture
    // `&ExportEntry` because the closure must be `'static`
    // for `register_export_callback`'s `Send + Sync +
    // 'static` bound. The mangled name is `&'static str`
    // pointing into the cdylib's .rodata, but the cdylib
    // is held by the closure's `lib_cell` Arc so the
    // `'static` claim is safe at *runtime* even though
    // Rust's static-lifetime tracking can't see through
    // the indirection. We copy the bytes into an owned
    // `String` to sidestep that entirely.
    let mangled_owned: String = entry.mangled.to_string();
    let lib_cell: Arc<libloading::Library> = Arc::clone(lib);
    // Cap the response buffer at 4 MiB. Canonical-ABI
    // responses for leo4 boundary types fit comfortably
    // below this; `BUFFER_TOO_SMALL` (the wrapper's
    // overflow signal) surfaces cleanly if it doesn't.
    const RET_BUF_CAP: usize = 4 * 1024 * 1024;

    invoker.register_export_callback(entry.mangled, move |args: &[u8]| {
        // Resolve the wrapper symbol from the cdylib. We
        // resolve on every call rather than caching: the
        // `libloading::Symbol` borrow is bound to `&lib`
        // and we'd need an `unsafe` lifetime widening to
        // store it. The dlsym hit cost is dominated by
        // the actual call's marshalling, so re-resolving
        // is cheap.
        //
        // SAFETY: the mangled symbol was emitted by the
        // `#[leo4::export]` macro with exactly the
        // `WrapperFn` signature; the cdylib outlives this
        // closure (it holds an `Arc<Library>`).
        let sym_name = format!("{mangled_owned}\0");
        let wrapper: libloading::Symbol<'_, WrapperFn> =
            unsafe { lib_cell.get(sym_name.as_bytes()) }.map_err(|e| {
                oxilean_kernel::ffi::ExternCallError::CallbackFailed(format!(
                    "leo4-oxilean-runner: dlsym({mangled_owned}) failed: {e}"
                ))
            })?;

        let mut buf: Vec<u8> = vec![0u8; RET_BUF_CAP];
        let mut ret_len: usize = 0;
        let args_ptr = if args.is_empty() {
            std::ptr::null()
        } else {
            args.as_ptr()
        };
        // SAFETY: the wrapper signature is fixed; we pass
        // valid pointers / lengths per its contract.
        let rc = unsafe {
            wrapper(
                args_ptr,
                args.len(),
                buf.as_mut_ptr(),
                buf.len(),
                &raw mut ret_len,
            )
        };
        if rc != 0 {
            return Err(
                oxilean_kernel::ffi::ExternCallError::CallbackFailed(format!(
                    "leo4-oxilean-runner: wrapper `{mangled_owned}` returned \
                     non-zero code 0x{rc:08x}"
                )),
            );
        }
        if ret_len > buf.len() {
            return Err(
                oxilean_kernel::ffi::ExternCallError::CallbackFailed(format!(
                    "leo4-oxilean-runner: wrapper `{mangled_owned}` wanted \
                     {ret_len} bytes; ret-buf is {} (raise RET_BUF_CAP)",
                    buf.len()
                )),
            );
        }
        buf.truncate(ret_len);
        Ok(buf)
    })?;
    Ok(())
}

/// Parse + elab + type-check every decl in `main_src`
/// against `env`. Updates the `decls_parsed` /
/// `decls_elaborated` diagnostic counters in lockstep.
///
/// Mirrors `oxilean-cli`'s `commands::check_source` rather
/// than calling the public `oxilean_parse::parser::parse_decls`
/// helper, because the public helper relies on
/// `ParseError::is_eof()` to terminate the loop and that
/// predicate only matches `ParseErrorKind::UnexpectedEof` —
/// but `parse_decl` returns a *non-EOF* `UnexpectedToken`
/// error on natural end-of-file, never `UnexpectedEof`. The
/// CLI side compensates by stringly-matching `"end of file"`
/// in the error message; we do the same here to keep
/// behaviour aligned with what `oxilean check <file>`
/// accepts.
fn elaborate_main_lean(
    main_src: &str,
    env: &mut Environment,
    diag: &mut RunDiagnostics,
) -> Result<(), LeanError> {
    let trimmed = main_src.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    // OX8 follow-up #79 (2026-05-28): try the PEG parser
    // first. `oxilean_parse_peg::parse_decls` handles
    // surfaces oxilean-parse v0.1.2's lexer/parser rejects
    // — most importantly `s!"…{x}…"` string interpolation
    // (used by the OX8.5 scaffold's stock Main.lean
    // template). On any failure (parse error OR any decl's
    // translate returning Unsupported) we fall back to the
    // legacy oxilean-parse walker so the runner doesn't
    // regress on inputs that worked pre-#79.
    if let Some(translated) = try_parse_via_peg(main_src) {
        for located in &translated {
            diag.decls_parsed += 1;
            let pending = elab_decl_impl(env, &located.value).map_err(|e| {
                LeanError::new(
                    error_codes::DECODE_ERROR,
                    format!(
                        "leo4-oxilean-runner: elaborate_decl in Main.lean \
                         failed: {e:?}"
                    ),
                )
            })?;
            let kernel_decl = pending_to_kernel_decl(pending);
            check_declaration(env, kernel_decl).map_err(|e| {
                LeanError::new(
                    error_codes::DECODE_ERROR,
                    format!(
                        "leo4-oxilean-runner: check_declaration in \
                         Main.lean failed: {e}"
                    ),
                )
            })?;
            diag.decls_elaborated += 1;
        }
        let _ = ElabContext::new(env);
        return Ok(());
    }

    // Legacy path — preserves pre-#79 behaviour for inputs
    // the PEG parser can't yet translate.
    let mut lexer = Lexer::new(main_src);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens);

    loop {
        match parser.parse_decl() {
            Ok(located) => {
                diag.decls_parsed += 1;
                let pending = elab_decl_impl(env, &located.value).map_err(|e| {
                    LeanError::new(
                        error_codes::DECODE_ERROR,
                        format!(
                            "leo4-oxilean-runner: elaborate_decl in Main.lean \
                             failed: {e:?}"
                        ),
                    )
                })?;
                let kernel_decl = pending_to_kernel_decl(pending);
                check_declaration(env, kernel_decl).map_err(|e| {
                    LeanError::new(
                        error_codes::DECODE_ERROR,
                        format!(
                            "leo4-oxilean-runner: check_declaration in \
                             Main.lean failed: {e}"
                        ),
                    )
                })?;
                diag.decls_elaborated += 1;
            }
            Err(e) => {
                // Mirror oxilean-cli's check_source EOF
                // heuristic — `parse_decl` reports the EOL
                // token as `UnexpectedToken` rather than
                // `UnexpectedEof`, so the structured
                // `is_eof()` predicate never fires. Falling
                // back to the displayed message is the same
                // workaround upstream uses.
                let msg = e.to_string();
                if msg.contains("end of file") || msg.contains("<eof>") {
                    break;
                }
                return Err(LeanError::new(
                    error_codes::DECODE_ERROR,
                    format!(
                        "leo4-oxilean-runner: parse_decl(Main.lean) failed: {e}"
                    ),
                ));
            }
        }
    }
    let _ = ElabContext::new(env); // keep symbol used; reserved for future
    Ok(())
}

/// OX8 follow-up #79 — best-effort PEG parse + translate.
/// Returns `Some(decls)` only when every parsed PEG decl
/// translates cleanly; on any failure (parse error or any
/// `TranslateError`) returns `None` so the caller falls back
/// to the legacy walker. The legacy diagnostic remains
/// authoritative for users — PEG is an opportunistic upgrade.
fn try_parse_via_peg(main_src: &str) -> Option<Vec<oxilean_parse::Located<oxilean_parse::Decl>>> {
    let peg_decls = oxilean_parse_peg::parse_decls(main_src).ok()?;
    let mut out = Vec::with_capacity(peg_decls.len());
    for pd in &peg_decls {
        match leo4_translate::translate_decl(pd) {
            Ok(d) => out.push(d),
            Err(_) => return None,
        }
    }
    Some(out)
}

/// Convert an elaborated `PendingDecl` to the kernel-side
/// `Declaration` shape `check_declaration` expects. Mirrors
/// the `elaborate_decl` private wrapper in
/// `oxilean-cli/src/commands/functions.rs`.
fn pending_to_kernel_decl(pending: PendingDecl) -> Declaration {
    match pending {
        PendingDecl::Definition { name, ty, val, .. } => Declaration::Definition {
            name,
            univ_params: vec![],
            ty,
            val,
            hint: ReducibilityHint::Regular(0),
        },
        PendingDecl::Theorem { name, ty, proof, .. } => Declaration::Theorem {
            name,
            univ_params: vec![],
            ty,
            val: proof,
        },
        PendingDecl::Axiom { name, ty, .. } => Declaration::Axiom {
            name,
            univ_params: vec![],
            ty,
        },
        // Inductive lands as a placeholder Axiom — mirrors
        // the oxilean-cli wrapper. Full inductive
        // installation needs the `compute_recursors` path
        // which the runner doesn't drive.
        PendingDecl::Inductive { name, ty, .. } => Declaration::Axiom {
            name,
            univ_params: vec![],
            ty,
        },
        PendingDecl::Opaque { name, ty, val } => Declaration::Opaque {
            name,
            univ_params: vec![],
            ty,
            val,
        },
    }
}

/// Convenience: load a cdylib from any `AsRef<OsStr>`-able
/// path argument shape. Mirrors `libloading::Library::new`'s
/// API for users that already have a typed path-y value
/// (e.g. `PathBuf`, `&str` from env var).
pub fn run_main_os(
    cdylib_path: impl AsRef<OsStr>,
    main_lean_path: impl AsRef<Path>,
) -> Result<(), LeanError> {
    run_main(
        Path::new(cdylib_path.as_ref()),
        main_lean_path.as_ref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use leo4_abi::rust_exports::{CtorEntry, FieldEntry, UserTypeKind};

    fn mk_user_type(
        fqn: &'static str,
        kind: UserTypeKind,
        fields: &'static [FieldEntry],
        ctors: &'static [CtorEntry],
    ) -> UserTypeEntry {
        UserTypeEntry {
            fqn,
            kind,
            fields,
            ctors,
        }
    }

    /// RC.6 F4 — synth registers `ConstantInfo::Inductive` +
    /// `Constructor` decls per UserTypeEntry. Verify against a
    /// flagship typed-enum (`AdsmtVerdict`).
    #[test]
    fn synthesise_user_type_decls_emits_inductive_plus_ctors_for_variant() {
        use oxilean_kernel::Name;

        let user_types = vec![mk_user_type(
            "AdsmtVerdict",
            UserTypeKind::Variant,
            &[],
            &[
                CtorEntry { name: "Sat", fields: &[] },
                CtorEntry { name: "Unsat", fields: &[] },
                CtorEntry { name: "Unknown", fields: &[] },
            ],
        )];
        let mut env = bootstrap_env().expect("bootstrap");
        synthesise_user_type_decls(&mut env, &user_types);

        // Inductive registered.
        let av = env.find(&Name::str("AdsmtVerdict"))
            .expect("AdsmtVerdict registered");
        assert!(av.is_inductive(), "AdsmtVerdict must be Inductive");
        let iv = av.to_inductive_val().unwrap();
        assert_eq!(iv.ctors.len(), 3, "3 ctors registered on parent");

        // Each ctor registered with lowercased first char.
        for (i, expected_name) in
            ["AdsmtVerdict.sat", "AdsmtVerdict.unsat", "AdsmtVerdict.unknown"]
                .iter()
                .enumerate()
        {
            let ctor = env.find(&Name::str(*expected_name))
                .unwrap_or_else(|| panic!("missing ctor {expected_name}"));
            assert!(ctor.is_constructor());
            let cv = ctor.to_constructor_val().unwrap();
            assert_eq!(cv.cidx, i as u32);
            assert_eq!(cv.induct.to_string(), "AdsmtVerdict");
        }
    }

    #[test]
    fn synthesise_user_type_decls_record_emits_mk_ctor() {
        use oxilean_kernel::Name;

        let user_types = vec![mk_user_type(
            "Point",
            UserTypeKind::Record,
            &[
                FieldEntry {
                    name: "x",
                    type_mangle: "",
                    rust_type: "u32",
                },
                FieldEntry {
                    name: "y",
                    type_mangle: "",
                    rust_type: "u32",
                },
            ],
            &[],
        )];
        let mut env = bootstrap_env().expect("bootstrap");
        synthesise_user_type_decls(&mut env, &user_types);

        let p = env.find(&Name::str("Point")).expect("Point registered");
        assert!(p.is_inductive());
        let mk = env.find(&Name::str("Point.mk")).expect("Point.mk registered");
        let cv = mk.to_constructor_val().unwrap();
        assert_eq!(cv.num_fields, 2, "record ctor carries num_fields");
    }

    #[test]
    fn synthesise_user_type_decls_unit_enum_emits_lowercased_ctors() {
        use oxilean_kernel::Name;

        let user_types = vec![mk_user_type(
            "Color",
            UserTypeKind::UnitEnum,
            &[],
            &[
                CtorEntry { name: "Red", fields: &[] },
                CtorEntry { name: "Green", fields: &[] },
                CtorEntry { name: "Blue", fields: &[] },
            ],
        )];
        let mut env = bootstrap_env().expect("bootstrap");
        synthesise_user_type_decls(&mut env, &user_types);

        assert!(env.find(&Name::str("Color.red")).is_some());
        assert!(env.find(&Name::str("Color.green")).is_some());
        assert!(env.find(&Name::str("Color.blue")).is_some());
    }

    /// `RunDiagnostics::default` is all-zero / all-false.
    /// Lightweight check — the smoke test that exercises
    /// the actual cdylib + Main.lean pipeline lives at
    /// the workspace level (the parent leo4-cli's cli_smoke
    /// or the OX8.5 e2e fixture).
    #[test]
    fn default_diagnostics_are_empty() {
        let d = RunDiagnostics::default();
        assert_eq!(d.exports_seen, 0);
        assert_eq!(d.exports_registered, 0);
        assert_eq!(d.decls_parsed, 0);
        assert_eq!(d.decls_elaborated, 0);
        assert!(!d.env_bootstrapped);
        assert!(!d.resolver_ready);
    }

    /// `run_main` on a non-existent cdylib path surfaces
    /// `RUST_DLSYM_FAILED` (0x0002_0005) and points at the
    /// dlopen failure. We don't need a real cdylib to
    /// exercise the error path.
    #[test]
    fn run_main_missing_cdylib_is_dlsym_failed() {
        let nowhere = Path::new("/nonexistent/leo4-oxilean-runner/nope.so");
        let main = Path::new("/nonexistent/leo4-oxilean-runner/Main.lean");
        let err = run_main(nowhere, main).unwrap_err();
        assert_eq!(err.code, RUST_DLSYM_FAILED);
        assert!(
            err.message.contains("dlopen"),
            "error must point at dlopen step: {}",
            err.message
        );
    }

    /// Same shape via the `OsStr`-accepting convenience.
    #[test]
    fn run_main_os_threads_through_the_same_error() {
        let err = run_main_os(
            "/nonexistent/leo4-oxilean-runner/nope2.so",
            "/nonexistent/leo4-oxilean-runner/Main2.lean",
        )
        .unwrap_err();
        assert_eq!(err.code, RUST_DLSYM_FAILED);
    }
}
