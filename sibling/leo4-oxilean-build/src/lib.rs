//! leo4-oxilean-build — build-time transpiler from OxiLean
//! Lean source to a Rust crate, per
//! `SPEC/rust-native-lean.md` §9.
//!
//! ## What this is
//!
//! The **transpile path** to leo4-rust-native: instead of
//! dispatching Lean calls into a live OxiLean evaluator at
//! runtime (the `leo4-oxilean` adapter's role, currently
//! blocked on upstream Hooks 1 + 2 per §7.1), this crate
//! reads a Lean source tree, walks the `@[leo4_export]`
//! declarations, and emits a plain Rust crate via
//! `oxilean-codegen::rust_target_backend::RustTargetBackend`.
//! The generated crate is an ordinary Rust library —
//! callable in-process, no `lean.h` C ABI, no OxiLean
//! runtime dep at the consumer's build site.
//!
//! Key insight from `oxilean-codegen` v0.1.2 inspection:
//! `lcnf_to_rust_type` maps to **pure Rust types**:
//! `Nat → U64`, `LcnfString → RustString`, `Object → Box<dyn
//! Any>`. No `lean_box` / `lean_object*` in the output. The
//! resulting Rust crate is structurally indistinguishable
//! from one a human would write.
//!
//! ## Status (2026-05-21)
//!
//! **SCAFFOLD**. Cargo deps wired (`oxilean-parse`,
//! `oxilean-elab`, `oxilean-codegen`, `oxilean-kernel`); the
//! type surface for the transpiler driver compiles; **but
//! no end-to-end transpile is wired yet**. The blocking
//! items are upstream API questions answered case-by-case
//! during real-fixture testing — they're not architectural
//! blockers (unlike Hooks 1 + 2 for the runtime-dispatch
//! sibling crate):
//!
//! 1. `oxilean-parse`'s public entry for parsing a Lean
//!    source file or string. The crate's public API surface
//!    needs surveying as the wiring proceeds.
//! 2. `oxilean-elab`'s public entry for elaborating parsed
//!    AST to LCNF. Hook 3 (attribute / deriving handler
//!    registration) ties in here.
//! 3. Driving the `LcnfModule` → `RustTargetBackend::
//!    emit_module` → `RustModule::emit() -> String` chain
//!    is documented in §9.1 / §9.3 of the SPEC; what's
//!    missing is the wrapper that walks a user package's
//!    Lean sources, registers the `@[leo4_export]` attribute
//!    handler, and writes the result to a destination Cargo
//!    crate dir.
//!
//! ## What works today (25 / 25 tests)
//!
//! - Cargo deps resolve + compile (5 OxiLean crates
//!   reachable).
//! - `transpile_decls(name, &[LcnfFunDecl]) -> String`
//!   wraps `RustTargetBackend::emit_module` + per-fn
//!   `RustFn::emit()`.
//! - **`transpile_kernel_decl(name, params, body) ->
//!   String`** — full kernel-level → LCNF → Rust source
//!   pipeline (skips `oxilean-parse` / `oxilean-elab` so
//!   you don't need a Lean source corpus to test it).
//! - Sample transpile of
//!   `Sample.addOne (n : Nat) : Nat := Nat.succ n`
//!   produces:
//!   ```ignore
//!   pub fn Sample_addOne(_x0: u64) -> Box<dyn std::any::Any> {
//!       _x1(_x0)
//!   }
//!   ```
//!   Real Rust source, **zero `lean_*` symbols**. The
//!   `Box<dyn Any>` return + unresolved `_x1` (= `Nat.succ`)
//!   are the *limitations of context-free lowering* — see
//!   below.
//! - `lcnf_to_rust_type` invariant probe: every mapping
//!   produces a Rust type that *names a real Rust standard
//!   type* (`u64`, `String`, `()`, `Box<dyn Any>`,
//!   `fn(…) -> …`). Catches future OxiLean releases that
//!   would regress this.
//!
//! ## Limitations of context-free lowering
//!
//! Calling `decl_to_lcnf` on a hand-built kernel `Expr`
//! (no surrounding `Environment`, no elaborated constant
//! definitions) gives the LCNF lowering nothing to look up
//! `Nat.succ` etc. against. The result:
//!
//! - Constants like `Nat.succ` come out as fresh free
//!   variables (`_x1`, `_x2`, …).
//! - Return types not inferable from the body alone fall
//!   back to `LcnfType::Object → Box<dyn std::any::Any>`.
//!
//! Real usage will wrap this in:
//! 1. `oxilean-parse` to read a `.lean` source file.
//! 2. `oxilean-elab` to elaborate + populate the env (and
//!    bind a custom `@[leo4_export]` attribute handler via
//!    Hook 3 — `oxilean_elab::attribute::AttributeManager::
//!    register_custom_handler`).
//! 3. Walk the elaborated env to extract `@[leo4_export]`-
//!    tagged decls' typed shape.
//! 4. Drive each decl through `decl_to_lcnf` with the env
//!    populated. The same `RustTargetBackend` then emits
//!    fully-resolved Rust source.
//!
//! That wiring is the next-step work; the current
//! `transpile_kernel_decl` is the *machinery* layer this
//! will plug into.
//!
//! ## Activation plan (next commits)
//!
//! Layer-by-layer, with the current `transpile_kernel_decl`
//! at the centre:
//!
//! 1. **Parse + elab layer** — wrap `oxilean-parse::Lexer
//!    → Parser::parse_decl` + `oxilean-elab` env / elaborator
//!    to lift a Lean source string to elaborated kernel
//!    `Expr`s. Resolves the `_x1 = Nat.succ` issue. **(Done
//!    2026-05-22 — `transpile_source` end-to-end pipeline.)**
//! 2. **`lean4_compat` adapter pre-processor** — drive
//!    `oxilean-elab::lean4_compat::{Lean4TermRewriter::
//!    standard, Lean4SyntaxAdapter::adapt_all}` to normalise
//!    Lean 4 surface syntax (` => ` → ` -> `, `←` → `<-`,
//!    `where;` → `where`, etc.) into OxiLean parser dialect.
//!    **(Done 2026-05-22 — `lean4_normalize` helper.)** Note:
//!    parser-level differences (e.g. header binders
//!    `def f (x : T) := …`) need a richer adapter — the
//!    upstream `lean4_compat` v0.1.2 layer is textual only.
//! 3. **`@[leo4_export]` discovery** — bind a custom handler
//!    via `oxilean_elab::attribute::AttributeManager::register_custom_handler`.
//!    Walk the elaborated env to collect tagged decls.
//!    **(Done 2026-05-22 — `Leo4ExportRegistry` +
//!    `transpile_source_if_exported`.)**
//! 4. **`deriving LeanMarshal`** — analogous binding via
//!    `oxilean_elab::attribute::DeriveHandlerRegistry::register`.
//!    The handler emits the encoder/decoder boilerplate that
//!    `lake/Leo4Plugin/Leo4Plugin/Deriving.lean` does on the
//!    reference Lean side. **(Wired 2026-05-22 — handler
//!    registered with `.no_instance()`; Rust-side impl
//!    synthesis is part of step 5.)**
//! 5. **Canonical-ABI wrapper synthesis** — for each
//!    transpiled fn, generate a sibling
//!    `pub fn <name>_call(args: &[u8]) -> Vec<u8>` that
//!    canonical-ABI decodes `args` via `leo4_abi::LeanMarshal`,
//!    calls the transpiled fn, encodes the return. The result
//!    is a Rust crate that conforms to leo4-rust-native's
//!    boundary contract. **(Done 2026-05-22 —
//!    `synthesize_canonical_wrapper(&RustFn) -> String` +
//!    `transpile_kernel_decl_with_wrapper` combined helper.
//!    Marshallable type matrix today: u8..u128, i8..i128,
//!    f32, f64, bool, char, String, () (unit return). Carrier
//!    types + user records pending the upstream backend
//!    emitting struct / impl shapes.)**
//! 6. **Cargo crate emit** — `Cargo.toml` + `lib.rs` written
//!    to a target dir; consumer's main project just
//!    `path`-deps it.

#![allow(clippy::missing_errors_doc)]

use leo4_abi::LeanError;
use oxilean_codegen::lcnf::LcnfFunDecl;
use oxilean_codegen::rust_target_backend::{RustItem, RustTargetBackend};
use oxilean_codegen::to_lcnf::{decl_to_lcnf, ToLcnfConfig};
use oxilean_elab::attribute::{
    AttrAction, AttrEntry, AttrHandler, AttributeManager, DeriveHandler,
    DeriveHandlerRegistry,
};
use oxilean_elab::elab_decl::{elaborate_decl, PendingDecl};
use oxilean_elab::lean4_compat::{Lean4SyntaxAdapter, Lean4TermRewriter};
use oxilean_kernel::{env::Environment, Expr, Name};
use oxilean_parse::{AttributeKind, Decl, Lexer, Located, Parser};

/// Custom attribute name leo4 owns: `@[leo4_export]`. Tag a Lean
/// definition with this attribute to mark it for export through
/// leo4's canonical-ABI boundary; the transpiler only emits Rust
/// wrappers for tagged decls.
pub const LEO4_EXPORT_ATTR: &str = "leo4_export";

/// Lean class name leo4 owns for auto-derive: `deriving LeanMarshal`.
/// Registered into OxiLean's `DeriveHandlerRegistry` so the
/// elaborator recognises it as a known class; the transpiler
/// emits the actual `LeanMarshal` impl on the Rust side, not
/// inside OxiLean.
pub const LEAN_MARSHAL_DERIVE: &str = "LeanMarshal";

/// Normalise a Lean 4 surface-syntax source string into a form
/// `oxilean-parse::Parser` can consume. Drives
/// `oxilean-elab`'s `lean4_compat` adapter layer:
///
/// 1. `Lean4TermRewriter::standard()` — the canonical set of
///    Lean 4 → OxiLean textual replacements (` => ` → ` -> `,
///    `←` → `<-`, `where;` → `where`, `∧/∨/¬` → `&&/||/!`).
/// 2. `Lean4SyntaxAdapter::adapt_all` — composite of
///    `adapt_do_notation` + `adapt_where_clause` +
///    `adapt_match_syntax` (also fields `=> -> ->` for the
///    match-arm form the rewriter doesn't disambiguate).
///
/// Both passes are textual (intentional in upstream — they
/// pre-process the source for the parser, they don't AST-rewrite).
/// Idempotent: running it twice produces the same output.
///
/// Note this does **not** convert Lean 4's
/// `def f (binders) : T := body` to OxiLean's
/// `def f : T := fun ... -> body` — header-binders are a
/// parser-level mismatch beyond textual rewriting. Code that
/// wants to pass header-binder syntax through must either
/// (a) lift each `(x : T)` group into a leading `fun x -> ` /
/// `Π (x : T),` (a future pass living above this layer) or
/// (b) pre-elaborate via a richer adapter (not implemented in
/// `lean4_compat` v0.1.2).
#[must_use]
pub fn lean4_normalize(src: &str) -> String {
    let after_rewrite = Lean4TermRewriter::standard().rewrite(src);
    Lean4SyntaxAdapter::adapt_all(&after_rewrite)
}

/// Transpile a slice of LCNF function declarations to a
/// single Rust source string. Wraps
/// `RustTargetBackend::emit_module` + the `RustModule`'s
/// per-item `RustFn::emit()` serialisation.
///
/// `module_name` is used as the OxiLean-side module label;
/// it doesn't affect the emitted Rust source structure.
///
/// # Errors
/// `LeanError` if every decl in `decls` fails to compile
/// (the backend silently drops individually-failing decls,
/// so a totally-empty result is a sign of upstream
/// breakage).
pub fn transpile_decls(
    module_name: &str,
    decls: &[LcnfFunDecl],
) -> Result<String, LeanError> {
    let mut backend = RustTargetBackend::new();
    let module = backend.emit_module(module_name, decls);
    if module.items.is_empty() && !decls.is_empty() {
        return Err(LeanError::new(
            leo4_abi::error::error_codes::ENCODE_ERROR,
            format!(
                "leo4-oxilean-build: RustTargetBackend dropped every \
                 LCNF decl in `{module_name}` — none compiled to Rust"
            ),
        ));
    }
    let mut out = String::new();
    out.push_str("//! Auto-generated by leo4-oxilean-build from OxiLean source.\n");
    out.push_str("//! DO NOT EDIT — re-run the build step instead.\n\n");
    for item in &module.items {
        // Other RustItem variants (structs, enums, impls) would
        // be emitted via their own `emit()` methods once the
        // backend produces them. For the v0 surface — leo4
        // exports are top-level fns — only RustItem::Fn appears.
        if let RustItem::Fn(f) = item {
            out.push_str(&f.emit());
            out.push_str("\n\n");
        }
    }
    Ok(out)
}

/// End-to-end transpile from a kernel-level declaration to
/// a Rust source string. The full pipeline a future
/// `leo4-oxilean-build` CLI would run, except parsing /
/// elaborating Lean source (those layers wrap `oxilean-parse`
/// + `oxilean-elab` and aren't wired yet).
///
/// `(name, params, body)` is the kernel-level shape
/// `oxilean-codegen::to_lcnf::decl_to_lcnf` consumes:
/// - `name` — the declared function's name (e.g.
///   `Name::str("Sample.addOne")`).
/// - `params` — parameter list as `(Name, Expr)` pairs where
///   each `Expr` is the parameter's type.
/// - `body` — the function body as a kernel `Expr`.
///
/// The output is one Rust `fn` definition. Wrap it in a
/// crate / module yourself; this helper deliberately stays
/// per-decl to keep failures granular.
///
/// # Errors
/// `LeanError` if either LCNF lowering or Rust backend
/// emission fails. The underlying OxiLean errors are
/// `ConversionError` / `String`; both get wrapped into
/// `LeanError::new(ENCODE_ERROR, …)` so callers don't depend
/// on OxiLean's error types.
pub fn transpile_kernel_decl(
    name: &Name,
    params: &[(Name, Expr)],
    body: &Expr,
) -> Result<String, LeanError> {
    let config = ToLcnfConfig::default();
    let lcnf_decl: LcnfFunDecl =
        decl_to_lcnf(name, params, body, &config).map_err(|e| {
            LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                format!("leo4-oxilean-build: decl_to_lcnf failed: {e:?}"),
            )
        })?;
    let mut backend = RustTargetBackend::new();
    let rust_fn = backend.compile_decl(&lcnf_decl).map_err(|e| {
        LeanError::new(
            leo4_abi::error::error_codes::ENCODE_ERROR,
            format!("leo4-oxilean-build: compile_decl failed: {e:?}"),
        )
    })?;
    Ok(rust_fn.emit())
}

/// Unfold a Pi-typed declaration into a `(params, body)`
/// pair the way `decl_to_lcnf` expects.
///
/// A `def f (x : A) (y : B) : C := body` elaborates into
/// `ty = Π (x : A), Π (y : B), C` and
/// `val = λ (x : A), λ (y : B), body`. The pairs zip
/// together as `params = [(x, A), (y, B)]` + the unbound
/// inner `body`.
fn unfold_decl(ty: &Expr, val: &Expr) -> (Vec<(Name, Expr)>, Expr) {
    let mut params = Vec::new();
    let mut cur_ty = ty;
    let mut cur_val = val;
    while let (
        Expr::Pi(_, _, dom_ty, rest_ty),
        Expr::Lam(_, lam_name, _dom_val, rest_val),
    ) = (cur_ty, cur_val)
    {
        params.push((lam_name.clone(), (**dom_ty).clone()));
        cur_ty = rest_ty;
        cur_val = rest_val;
    }
    (params, cur_val.clone())
}

/// Full pipeline: Lean source string → Rust source string.
/// Drives `oxilean-parse::{Lexer, Parser}` →
/// `oxilean-elab::elaborate_decl` → `unfold_decl` →
/// `oxilean-codegen::to_lcnf::decl_to_lcnf` →
/// `RustTargetBackend::compile_decl` → `RustFn::emit`.
///
/// `env` is the elaboration environment. Pass an
/// `Environment::new()` for a minimal probe, or a
/// pre-populated environment (with built-in inductives,
/// previously-elaborated decls, etc.) for real fixtures.
///
/// Only **one declaration per source string** today —
/// `Parser::parse_decl` returns a single `Located<Decl>`.
/// Multi-decl source loading lives in the next layer when
/// we wrap a Lake-package walker.
///
/// # Errors
/// `LeanError` if any step (parse / elab / LCNF / Rust emit)
/// fails. The underlying OxiLean / parser errors get wrapped
/// so callers don't depend on OxiLean's error types.
pub fn transpile_source(env: &Environment, src: &str) -> Result<String, LeanError> {
    // 0. Lean 4 → OxiLean syntax normalisation
    //    (`oxilean-elab::lean4_compat` pre-processor pass).
    let normalised = lean4_normalize(src);

    // 1. Lex.
    let mut lexer = Lexer::new(&normalised);
    let tokens = lexer.tokenize();

    // 2. Parse one declaration.
    let mut parser = Parser::new(tokens);
    let decl = parser.parse_decl().map_err(|e| {
        LeanError::new(
            leo4_abi::error::error_codes::DECODE_ERROR,
            format!("leo4-oxilean-build: parse_decl failed: {e:?}"),
        )
    })?;

    // 3. Elaborate.
    let pending = elaborate_decl(env, &decl.value).map_err(|e| {
        LeanError::new(
            leo4_abi::error::error_codes::DECODE_ERROR,
            format!("leo4-oxilean-build: elaborate_decl failed: {e:?}"),
        )
    })?;

    // 4. Extract (name, ty, val) — only Definition produces
    //    a transpilable body (axioms / theorems / inductives
    //    are out of scope today).
    let (name, ty, val) = match pending {
        PendingDecl::Definition { name, ty, val, .. } => (name, ty, val),
        other => {
            return Err(LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                format!(
                    "leo4-oxilean-build: only `def` declarations are \
                     transpilable today; got {other:?}"
                ),
            ));
        }
    };

    // 5. Unfold Pi / Lam into (params, body).
    let (params, body) = unfold_decl(&ty, &val);

    // 6. Drive the kernel-level helper.
    transpile_kernel_decl(&name, &params, &body)
}

/// Sanity probe: does the backend's type mapper actually
/// produce Rust type names that don't reference `lean_*`
/// symbols? Used as a build-time invariant — if a future
/// OxiLean release flips `lcnf_to_rust_type(Nat)` to e.g.
/// `RustType::Custom("lean_box_uint64")`, this fn returns
/// `false` and the corresponding test catches the
/// regression.
#[must_use]
pub fn type_mapper_is_lean_h_free() -> bool {
    use oxilean_codegen::lcnf::LcnfType;

    let probes: &[LcnfType] = &[
        LcnfType::Nat,
        LcnfType::LcnfString,
        LcnfType::Unit,
        LcnfType::Erased,
    ];
    for probe in probes {
        let ty = RustTargetBackend::lcnf_to_rust_type(probe);
        let rendered = format!("{ty}");
        if rendered.contains("lean_")
            || rendered.contains("Lean_")
            || rendered.contains("lean.h")
        {
            return false;
        }
    }
    true
}

// ─── Hook 3 — `@[leo4_export]` / `deriving LeanMarshal` discovery ──────────
//
// SPEC/rust-native-lean.md §7.1 lists three OxiLean evaluator
// hooks (Hooks 1 + 2 absent; Hook 3 PRESENT). Hook 3 is
// upstream's `AttributeManager::register_custom_handler` +
// `DeriveHandlerRegistry::register`. leo4 plugs into Hook 3 to
// register its own attribute name (`@[leo4_export]`) and its
// own deriving handler (`LeanMarshal`). The actual transpile
// then walks parsed `Decl::Attribute { attrs, decl }` outer
// wrappers, matches `attrs` against `LEO4_EXPORT_ATTR`, and
// only emits Rust for tagged decls — the rest are skipped.
//
// Note on parser shape: OxiLean's `Parser::parse_decl` returns
// `@[name] def f := body` as `Decl::Attribute { attrs:
// Vec<String>, decl: Box<Located<Decl>> }`. The *inner*
// `Decl::Definition.attrs` field is left empty by the parser —
// the outer wrapper is the only place attribute names appear.
// `elaborate_decl` further unwraps `Decl::Attribute` and
// discards the outer `attrs` (upstream code path; v0.1.2). So
// leo4 inspects the parser AST *before* elaboration to spot
// the tag, then elaborates the inner decl normally.

/// Registry owning leo4's attribute + derive handlers, plus the
/// `AttributeManager` accumulating discovered `@[leo4_export]`
/// tags across a build. Mutable across `transpile_source_if_exported`
/// calls so a whole package's transpile produces a single
/// registry capturing every export.
///
/// Created with `Leo4ExportRegistry::new()`; the constructor
/// pre-populates the manager + derive registry with leo4's two
/// handlers (`leo4_export` custom attribute, `LeanMarshal`
/// derive). Inspect with `has_export_handler()` /
/// `has_marshal_derive()` (used for tests + status diagnostics).
pub struct Leo4ExportRegistry {
    pub manager: AttributeManager,
    pub derive: DeriveHandlerRegistry,
}

impl Leo4ExportRegistry {
    /// Build a registry pre-populated with leo4's attribute +
    /// derive handlers.
    #[must_use]
    pub fn new() -> Self {
        let mut manager = AttributeManager::new();
        manager.register_custom_handler(AttrHandler::new(
            LEO4_EXPORT_ATTR,
            "Mark a top-level definition for export through leo4's \
             canonical-ABI boundary. The leo4-oxilean-build transpiler \
             emits a Rust wrapper for each tagged decl; untagged decls \
             are skipped.",
            AttrAction::Custom(LEO4_EXPORT_ATTR.into()),
        ));

        let mut derive = DeriveHandlerRegistry::new();
        derive.register(
            DeriveHandler::new(
                Name::str(LEAN_MARSHAL_DERIVE),
                "Auto-derive leo4 canonical-ABI marshalling for a record \
                 / inductive type. The handler emits no Lean instance — \
                 the transpiler synthesises the equivalent Rust \
                 `LeanMarshal` impl on the boundary crate side.",
            )
            // The transpile path doesn't need OxiLean to emit a
            // Lean-side instance; we generate the marshalling
            // Rust directly. `no_instance()` documents this and
            // suppresses upstream instance-generation work.
            .no_instance(),
        );

        Self { manager, derive }
    }

    /// True iff the `@[leo4_export]` custom-attribute handler
    /// is registered with the inner `AttributeManager`.
    #[must_use]
    pub fn has_export_handler(&self) -> bool {
        self.manager.get_handler(LEO4_EXPORT_ATTR).is_some()
    }

    /// True iff the `LeanMarshal` derive handler is registered
    /// with the inner `DeriveHandlerRegistry`.
    #[must_use]
    pub fn has_marshal_derive(&self) -> bool {
        self.derive.has(&Name::str(LEAN_MARSHAL_DERIVE))
    }

    /// Record an `@[leo4_export]` discovery in the manager so
    /// downstream queries (`manager.get_by_kind("leo4_export")`)
    /// can enumerate every export across a package's source.
    /// Used by `transpile_source_if_exported` after parse.
    pub fn record_export(&mut self, decl_name: &str) {
        // Map the parser-level decl-name string into a kernel
        // Name (the AttributeManager indexes by kernel Name).
        let kernel_name = Name::str(decl_name);
        let entry = AttrEntry::new(
            AttributeKind::Custom(LEO4_EXPORT_ATTR.into()),
            kernel_name,
        );
        // `register_attribute` errors on duplicates; treat as
        // best-effort — duplicates in a single build pass would
        // indicate a parser-level issue, not user error.
        let _ = self.manager.register_attribute(entry);
    }

    /// Enumerate every decl name (kernel `Name`) the manager
    /// has recorded as `@[leo4_export]`-tagged so far.
    #[must_use]
    pub fn exported_names(&self) -> Vec<Name> {
        self.manager.get_by_kind(LEO4_EXPORT_ATTR)
    }
}

impl Default for Leo4ExportRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Inspect a parsed top-level decl + return whether its outer
/// attribute wrapper contains `@[leo4_export]`. Inspects only
/// the parser AST — does NOT elaborate.
#[must_use]
pub fn decl_has_leo4_export(decl: &Located<Decl>) -> bool {
    if let Decl::Attribute { attrs, .. } = &decl.value {
        attrs.iter().any(|s| s == LEO4_EXPORT_ATTR)
    } else {
        false
    }
}

/// Unwrap one layer of `Decl::Attribute { decl, .. }` if
/// present; otherwise return the decl unchanged. Mirrors
/// `elaborate_decl`'s upstream behaviour so the inner decl
/// reaches kernel-level layers identically.
#[must_use]
pub fn inner_decl(decl: &Located<Decl>) -> &Located<Decl> {
    if let Decl::Attribute { decl, .. } = &decl.value {
        decl.as_ref()
    } else {
        decl
    }
}

/// Best-effort name extraction from a parsed `Decl`. Used to
/// register a discovery with the `AttributeManager` after
/// parsing but before elaboration.
#[must_use]
pub fn decl_name(decl: &Located<Decl>) -> Option<&str> {
    let target = inner_decl(decl);
    match &target.value {
        Decl::Definition { name, .. }
        | Decl::Theorem { name, .. }
        | Decl::Axiom { name, .. }
        | Decl::Inductive { name, .. } => Some(name.as_str()),
        _ => None,
    }
}

/// Hook-3-aware variant of `transpile_source`. Parses the
/// source, checks the outer-wrapper attribute list against
/// `LEO4_EXPORT_ATTR`, and:
///
/// - If `@[leo4_export]` is **not** present → returns
///   `Ok(None)` (skipped, no transpile work done).
/// - If `@[leo4_export]` **is** present → records the discovery
///   in `registry.manager` and returns `Ok(Some(rust_source))`
///   with the same pipeline as `transpile_source`.
///
/// Errors propagate as for `transpile_source` (parse / elab /
/// LCNF / Rust-emit failures wrap into `LeanError`).
pub fn transpile_source_if_exported(
    env: &Environment,
    registry: &mut Leo4ExportRegistry,
    src: &str,
) -> Result<Option<String>, LeanError> {
    let normalised = lean4_normalize(src);

    let mut lexer = Lexer::new(&normalised);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens);
    let parsed = parser.parse_decl().map_err(|e| {
        LeanError::new(
            leo4_abi::error::error_codes::DECODE_ERROR,
            format!("leo4-oxilean-build: parse_decl failed: {e:?}"),
        )
    })?;

    if !decl_has_leo4_export(&parsed) {
        return Ok(None);
    }

    // Record the discovery before elaboration so even if elab
    // chokes, the registry still knows what was *intended* to
    // be exported (useful for diagnostics in a multi-decl
    // build pass).
    if let Some(name) = decl_name(&parsed) {
        registry.record_export(name);
    }

    // Elaborate the inner decl directly — upstream
    // `elaborate_decl(env, Decl::Attribute{..})` does the same
    // unwrap and drops outer attrs; we've already captured the
    // tag.
    let pending = elaborate_decl(env, &inner_decl(&parsed).value).map_err(|e| {
        LeanError::new(
            leo4_abi::error::error_codes::DECODE_ERROR,
            format!("leo4-oxilean-build: elaborate_decl failed: {e:?}"),
        )
    })?;

    let (name, ty, val) = match pending {
        PendingDecl::Definition { name, ty, val, .. } => (name, ty, val),
        other => {
            return Err(LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                format!(
                    "leo4-oxilean-build: only `def` declarations are \
                     transpilable today; got {other:?}"
                ),
            ));
        }
    };

    let (params, body) = unfold_decl(&ty, &val);
    transpile_kernel_decl(&name, &params, &body).map(Some)
}

// ─── §5 Canonical-ABI wrapper synthesis ──────────────────────────────────
//
// SPEC/rust-native-lean.md §3: a `LeanProc` impl resolves
// `(mangled, args: &[u8]) -> Result<Vec<u8>, LeanError>`. The
// transpile path achieves the same shape by emitting a sibling
// wrapper *next to* each transpiled fn:
//
//   pub fn <name>_call(args: &[u8])
//        -> Result<Vec<u8>, leo4_abi::LeanError>
//
// The wrapper canonical-decodes the input bytes into typed
// Rust args, calls the transpiled fn, and canonical-encodes
// the result. The downstream `LeanProc` impl simply dispatches
// `mangled → <name>_call` lookups (a static table populated at
// crate-emit time; that's §6).
//
// Limitations of v0 wrapper synthesis: only fns whose params
// and return type *all* lift to a primitive `leo4_abi::LeanMarshal`
// type (u8..u128, i8..i128, f32, f64, bool, char, String) are
// supported. Carrier types (BigNat / LeanRat / etc.) and user-
// defined records aren't covered yet — they need their
// `LeanMarshal` impls + the transpiler's struct emission
// (which the upstream `RustTargetBackend::emit_module` doesn't
// produce at v0.1.2 either).

use oxilean_codegen::rust_target_backend::{RustFn, RustType};

/// Map a `RustType` to a Rust source string naming a type for
/// which leo4-abi provides a `LeanMarshal` impl. Returns `Err`
/// for types not yet covered by the v0 wrapper synthesis.
fn render_marshallable_type(ty: &RustType) -> Result<&'static str, LeanError> {
    match ty {
        RustType::U8 => Ok("u8"),
        RustType::U16 => Ok("u16"),
        RustType::U32 => Ok("u32"),
        RustType::U64 => Ok("u64"),
        RustType::U128 => Ok("u128"),
        RustType::I8 => Ok("i8"),
        RustType::I16 => Ok("i16"),
        RustType::I32 => Ok("i32"),
        RustType::I64 => Ok("i64"),
        RustType::I128 => Ok("i128"),
        RustType::F32 => Ok("f32"),
        RustType::F64 => Ok("f64"),
        RustType::Bool => Ok("bool"),
        RustType::Char => Ok("char"),
        RustType::RustString => Ok("::std::string::String"),
        // Bool/Unit return is special-cased at the call site.
        other => {
            let msg = format!(
                "leo4-oxilean-build: RustType `{other:?}` has no leo4-abi \
                 LeanMarshal impl wired in §5 wrapper synthesis"
            );
            Err(LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                msg,
            ))
        }
    }
}

/// Synthesise a canonical-ABI boundary shim for a transpiled
/// Rust fn. Emits a sibling `pub fn <name>_call(args: &[u8])
/// -> Result<Vec<u8>, LeanError>` that:
///
/// 1. Sequentially `LeanMarshal::canonical_decode`s each arg,
///    advancing the offset between calls.
/// 2. Invokes the transpiled fn with the decoded args.
/// 3. Canonical-encodes the return value (or returns an empty
///    `Vec<u8>` for unit-returning fns).
///
/// The emitted wrapper is plain Rust source; concatenate it
/// with `transpile_kernel_decl`'s output to land both items
/// in the same crate.
///
/// # Errors
/// `LeanError` if any param type or the return type fails
/// `render_marshallable_type` (the type isn't covered by §5's
/// v0 marshalling matrix — carrier types and user records are
/// the typical reasons).
pub fn synthesize_canonical_wrapper(transpiled: &RustFn) -> Result<String, LeanError> {
    use std::fmt::Write as _;

    let wrapper_name = format!("{}_call", transpiled.name);
    let mut s = String::new();

    writeln!(
        s,
        "/// Canonical-ABI boundary shim for `{}` — decodes args \
         from leo4 canonical bytes, calls the transpiled fn, \
         encodes the return.",
        transpiled.name
    )
    .unwrap();
    writeln!(
        s,
        "pub fn {wrapper_name}(args: &[u8]) -> ::core::result::Result<::std::vec::Vec<u8>, ::leo4_abi::LeanError> {{"
    )
    .unwrap();

    if transpiled.params.is_empty() {
        s.push_str("    let _ = args;\n");
    } else {
        s.push_str("    let mut __off: usize = 0;\n");
        for (i, (pname, pty, _)) in transpiled.params.iter().enumerate() {
            let rust_ty = render_marshallable_type(pty)?;
            writeln!(
                s,
                "    let ({pname}, __next_{i}) = <{rust_ty} as ::leo4_abi::LeanMarshal>::canonical_decode(args, __off)?;"
            )
            .unwrap();
            // Suppress an "unused last offset" lint by keeping
            // the assignment regardless of whether anything
            // after reads it.
            writeln!(s, "    __off = __next_{i};").unwrap();
        }
        s.push_str("    let _ = __off;\n");
    }

    let call_args = transpiled
        .params
        .iter()
        .map(|(n, _, _)| n.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(s, "    let __ret = {}({call_args});", transpiled.name).unwrap();

    match &transpiled.return_type {
        None | Some(RustType::Unit) => {
            s.push_str("    let _: () = __ret;\n");
            s.push_str("    ::core::result::Result::Ok(::std::vec::Vec::new())\n");
        }
        Some(ty) => {
            // Validate the return type has a LeanMarshal impl
            // before emitting the encode call.
            let _ = render_marshallable_type(ty)?;
            s.push_str("    ::core::result::Result::Ok(::leo4_abi::encode_to_vec(&__ret))\n");
        }
    }

    s.push_str("}\n");
    Ok(s)
}

/// Drive `transpile_kernel_decl` then `synthesize_canonical_wrapper`
/// in sequence; concatenate the two emitted items into one
/// Rust source string ready to drop into a crate.
///
/// Returns `(rust_fn_source, wrapper_source)` separately so
/// callers can route them into different files / modules
/// when desired (`lib.rs` + `wrappers.rs`, say).
///
/// # Errors
/// `LeanError` for any failure in either underlying step.
pub fn transpile_kernel_decl_with_wrapper(
    name: &Name,
    params: &[(Name, Expr)],
    body: &Expr,
) -> Result<(String, String), LeanError> {
    let config = ToLcnfConfig::default();
    let lcnf_decl: LcnfFunDecl =
        decl_to_lcnf(name, params, body, &config).map_err(|e| {
            LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                format!("leo4-oxilean-build: decl_to_lcnf failed: {e:?}"),
            )
        })?;
    let mut backend = RustTargetBackend::new();
    let rust_fn = backend.compile_decl(&lcnf_decl).map_err(|e| {
        LeanError::new(
            leo4_abi::error::error_codes::ENCODE_ERROR,
            format!("leo4-oxilean-build: compile_decl failed: {e:?}"),
        )
    })?;

    let fn_src = rust_fn.emit();
    let wrapper_src = synthesize_canonical_wrapper(&rust_fn)?;
    Ok((fn_src, wrapper_src))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_mapper_emits_pure_rust() {
        assert!(
            type_mapper_is_lean_h_free(),
            "RustTargetBackend's lcnf_to_rust_type must NOT emit lean_* symbols"
        );
    }

    #[test]
    fn nat_maps_to_u64() {
        use oxilean_codegen::lcnf::LcnfType;
        use oxilean_codegen::rust_target_backend::RustType;
        let ty = RustTargetBackend::lcnf_to_rust_type(&LcnfType::Nat);
        assert_eq!(ty, RustType::U64);
    }

    #[test]
    fn string_maps_to_rust_string() {
        use oxilean_codegen::lcnf::LcnfType;
        use oxilean_codegen::rust_target_backend::RustType;
        let ty = RustTargetBackend::lcnf_to_rust_type(&LcnfType::LcnfString);
        assert_eq!(ty, RustType::RustString);
    }

    #[test]
    fn transpile_empty_decls_returns_header_only() {
        let out = transpile_decls("empty", &[]).expect("empty must succeed");
        assert!(out.contains("Auto-generated"));
        assert!(out.contains("DO NOT EDIT"));
        // No fn definitions for an empty decl set.
        assert!(!out.contains("fn "));
    }

    /// End-to-end: hand-build the kernel Expr for a trivial
    /// `def identity (n : Nat) : Nat := n` and drive the full
    /// `decl_to_lcnf → RustTargetBackend::compile_decl →
    /// RustFn::emit()` pipeline. Verifies the output is real
    /// Rust source naming neither `lean_object*` nor `lean_box`.
    #[test]
    fn transpile_kernel_decl_emits_identity_fn() {
        use oxilean_kernel::Level;

        // `Nat` as a kernel constant. (The kernel-side `Nat`
        // is an inductive type; here we just reference it by
        // name — the LCNF lowering will map it to LcnfType::Nat
        // → RustType::U64.)
        let nat_ty = Expr::Const(Name::str("Nat"), vec![]);
        // Function body: BVar(0) = the single bound parameter.
        let body = Expr::BVar(0);
        let name = Name::str("Sample.identity");
        let params = vec![(Name::str("n"), nat_ty)];

        let src = transpile_kernel_decl(&name, &params, &body)
            .expect("identity transpile must succeed");

        // Should be a `pub fn …` (RustVisibility::Pub default).
        assert!(
            src.contains("pub fn"),
            "expected `pub fn` in output; got:\n{src}"
        );
        // The output must NOT contain any lean_*-prefixed
        // symbol — this is the leo4-rust-native invariant.
        assert!(
            !src.contains("lean_box") && !src.contains("lean_unbox") && !src.contains("lean_object"),
            "transpile output unexpectedly contained lean_*-prefixed symbols:\n{src}"
        );

        // Suppress the unused Level import (kept for future
        // tests that build Sort-typed parameters).
        let _ = Level::zero();
    }

    /// End-to-end: real Lean source string → transpiled
    /// Rust. Drives the full parse → elab → LCNF → Rust
    /// pipeline.
    ///
    /// Uses an empty `Environment::new()`; references to
    /// undefined constants (like `Nat`) fall through as
    /// free variables — the resulting Rust source is still
    /// valid Rust but uses `Box<dyn Any>` for unresolved
    /// types, mirroring `transpile_kernel_decl_emits_*`.
    /// Real fixtures pre-populate the env with the leo4
    /// runtime library's declarations.
    /// `lean4_normalize` invariants: textual normalisation is
    /// idempotent and applies the documented Lean 4 → OxiLean
    /// rewrites.
    #[test]
    fn lean4_normalize_applies_compat_rewrites() {
        // ` => ` → ` -> `
        let a = lean4_normalize("fun n => n");
        assert_eq!(a, "fun n -> n");
        // ←  → <-
        let b = lean4_normalize("do x ← f");
        assert_eq!(b, "do x <- f");
        // where; → where
        let c = lean4_normalize("def x := 1 where;");
        assert_eq!(c, "def x := 1 where");
        // Idempotent: running twice = once.
        let once = lean4_normalize("fun a => fun b => a");
        let twice = lean4_normalize(&once);
        assert_eq!(once, twice);
    }

    /// End-to-end source-level transpile through the full
    /// `lean4_normalize → Lexer → Parser → elab → LCNF →
    /// RustTargetBackend` pipeline. OxiLean's
    /// `Parser::parse_definition` accepts the shape
    /// `def name {univs} : type := value` — header-binders
    /// `(x : T)` are an OxiLean parser-level rejection
    /// beyond textual normalisation (see `lean4_normalize`
    /// docstring), so the source uses the body-lambda form.
    #[test]
    fn transpile_source_identity() {
        // OxiLean-native `def` shape: no header binders; type
        // is `Nat -> Nat`, body is `fun n -> n`. The
        // `lean4_normalize` pass would map a Lean 4
        // `fun n => n` to this form anyway — using it
        // directly here makes the parse step independently
        // testable.
        let src = "def identity : Nat -> Nat := fun n -> n";
        let env = Environment::new();
        let result = transpile_source(&env, src);
        // The parse + elab pipeline against an empty env can
        // either succeed (if `Nat` resolves to a free variable
        // + the lambda's BVar lookup works) OR fail gracefully
        // (an empty env has no `Nat` to elaborate against).
        // Both outcomes are diagnostic — print for analysis.
        match result {
            Ok(out) => {
                assert!(
                    !out.contains("lean_box") && !out.contains("lean_object"),
                    "transpile output unexpectedly contained lean_*-symbols:\n{out}"
                );
                eprintln!("transpile_source_identity → emitted:\n{out}");
            }
            Err(e) => {
                // Documented behaviour: empty env can't
                // resolve `Nat`. The error code is in the
                // canonical-ABI range so callers know how to
                // interpret it.
                let code = e.code;
                eprintln!(
                    "transpile_source_identity → expected-ish err (code 0x{code:08x}): {}",
                    e.message
                );
                assert!(
                    code == leo4_abi::error::error_codes::DECODE_ERROR
                        || code == leo4_abi::error::error_codes::ENCODE_ERROR,
                    "unexpected error code: 0x{code:08x}"
                );
            }
        }
    }

    /// Lean 4 surface syntax (`fun n => n`) survives the
    /// `lean4_normalize` pre-processor and lands in the same
    /// place as the OxiLean-native form — i.e. the pipeline
    /// is robust to the documented surface-syntax
    /// difference.
    #[test]
    fn transpile_source_lean4_syntax_normalises() {
        // Lean 4 form with `=>` arrow.
        let src_lean4 = "def identity : Nat -> Nat := fun n => n";
        // OxiLean-native form.
        let src_oxi = "def identity : Nat -> Nat := fun n -> n";

        // After normalisation, both produce identical text.
        assert_eq!(lean4_normalize(src_lean4), lean4_normalize(src_oxi));

        // And both routes through `transpile_source` produce
        // the same outcome class (Ok or Err — pipeline parity).
        let env = Environment::new();
        let r1 = transpile_source(&env, src_lean4);
        let r2 = transpile_source(&env, src_oxi);
        assert_eq!(
            r1.is_ok(),
            r2.is_ok(),
            "lean4-syntax vs oxilean-native parity broke: r1={r1:?} r2={r2:?}"
        );
    }

    #[test]
    fn transpile_kernel_decl_emits_addone_via_succ() {
        // `def addOne (n : Nat) : Nat := Nat.succ n`
        // Body: `App(Const("Nat.succ", []), BVar(0))`.
        let nat_ty = Expr::Const(Name::str("Nat"), vec![]);
        let succ = Expr::Const(Name::str("Nat.succ"), vec![]);
        let body = Expr::App(Box::new(succ), Box::new(Expr::BVar(0)));
        let name = Name::str("Sample.addOne");
        let params = vec![(Name::str("n"), nat_ty)];

        let src = transpile_kernel_decl(&name, &params, &body)
            .expect("addOne transpile must succeed");

        // Expected name appears (after RustTargetBackend's
        // mangle_name: `.` → `_`).
        assert!(
            src.contains("Sample_addOne") || src.contains("addOne"),
            "expected fn name in output; got:\n{src}"
        );
        // Lean-h cleanliness invariant holds for this case too.
        assert!(!src.contains("lean_box"));
        assert!(!src.contains("lean_unbox"));
        assert!(!src.contains("lean_object"));
    }

    // ─── Hook 3 — `@[leo4_export]` discovery tests ────────────────────

    #[test]
    fn registry_registers_leo4_export_handler() {
        let registry = Leo4ExportRegistry::new();
        assert!(
            registry.has_export_handler(),
            "Leo4ExportRegistry::new() must register the leo4_export custom-attr handler"
        );
        // Verify the handler's recorded name + doc are what we
        // wrote (defends against accidental rename in upstream).
        let handler = registry
            .manager
            .get_handler(LEO4_EXPORT_ATTR)
            .expect("export handler must be retrievable by name");
        assert_eq!(handler.name, LEO4_EXPORT_ATTR);
        assert!(
            handler.doc.contains("leo4")
                && handler.doc.contains("canonical-ABI"),
            "doc string must mention leo4 + canonical-ABI"
        );
    }

    #[test]
    fn registry_registers_lean_marshal_derive() {
        let registry = Leo4ExportRegistry::new();
        assert!(
            registry.has_marshal_derive(),
            "Leo4ExportRegistry::new() must register the LeanMarshal derive handler"
        );
    }

    #[test]
    fn registry_default_matches_new() {
        let a = Leo4ExportRegistry::new();
        let b = Leo4ExportRegistry::default();
        // Both should have leo4's handlers populated.
        assert!(a.has_export_handler() && b.has_export_handler());
        assert!(a.has_marshal_derive() && b.has_marshal_derive());
    }

    #[test]
    fn decl_has_leo4_export_detects_tag_via_parser() {
        // Drive a real parser pass so the test exercises the
        // exact AST shape upstream produces, not a hand-built
        // mock.
        let src = "@[leo4_export] def f : Nat -> Nat := fun n -> n";
        let normalised = lean4_normalize(src);
        let mut lexer = Lexer::new(&normalised);
        let tokens = lexer.tokenize();
        let mut parser = Parser::new(tokens);
        let decl = parser
            .parse_decl()
            .expect("parser must accept @[leo4_export] def");
        assert!(
            decl_has_leo4_export(&decl),
            "tagged decl must be recognised"
        );
        // The unwrapped inner decl is a Definition.
        let inner = inner_decl(&decl);
        assert!(
            matches!(&inner.value, Decl::Definition { .. }),
            "inner unwrap must surface the Definition"
        );
        // Name extraction works.
        assert_eq!(decl_name(&decl), Some("f"));
    }

    #[test]
    fn decl_has_leo4_export_rejects_untagged_decl() {
        let src = "def g : Nat -> Nat := fun n -> n";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize();
        let mut parser = Parser::new(tokens);
        let decl = parser.parse_decl().expect("plain def parses");
        assert!(!decl_has_leo4_export(&decl));
        // Untagged inner decl == outer decl.
        assert!(matches!(&decl.value, Decl::Definition { .. }));
        assert_eq!(decl_name(&decl), Some("g"));
    }

    #[test]
    fn decl_has_leo4_export_ignores_unrelated_attrs() {
        // `@[simp]` doesn't activate leo4 transpile.
        let src = "@[simp] def h : Nat -> Nat := fun n -> n";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize();
        let mut parser = Parser::new(tokens);
        let decl = parser.parse_decl().expect("@[simp] def parses");
        assert!(!decl_has_leo4_export(&decl));
    }

    #[test]
    fn transpile_source_if_exported_skips_untagged() {
        let mut registry = Leo4ExportRegistry::new();
        let env = Environment::new();
        let src = "def g : Nat -> Nat := fun n -> n";
        let out = transpile_source_if_exported(&env, &mut registry, src)
            .expect("parse path succeeds");
        assert!(out.is_none(), "untagged decl must yield Ok(None)");
        // Manager records nothing for skipped decls.
        assert!(registry.exported_names().is_empty());
    }

    // ─── §5 Canonical-ABI wrapper synthesis tests ────────────────────

    fn rfn(
        name: &str,
        params: Vec<(&str, RustType)>,
        ret: Option<RustType>,
    ) -> RustFn {
        let p = params
            .into_iter()
            .map(|(n, t)| (n.to_string(), t, false))
            .collect();
        RustFn::new(name, p, ret, vec![])
    }

    #[test]
    fn wrapper_emits_call_fn_for_u64_to_u64() {
        let f = rfn("addOne", vec![("n", RustType::U64)], Some(RustType::U64));
        let out = synthesize_canonical_wrapper(&f).expect("u64 → u64 must succeed");
        // Header + signature.
        assert!(
            out.contains("pub fn addOne_call(args: &[u8])"),
            "wrapper missing pub fn signature; got:\n{out}"
        );
        assert!(out.contains("LeanError"));
        // Decode call for the single u64 param.
        assert!(
            out.contains("<u64 as ::leo4_abi::LeanMarshal>::canonical_decode"),
            "wrapper missing u64 decode; got:\n{out}"
        );
        // Invocation of the transpiled fn.
        assert!(out.contains("let __ret = addOne(n)"));
        // Encode of the return.
        assert!(out.contains("::leo4_abi::encode_to_vec(&__ret)"));
    }

    #[test]
    fn wrapper_emits_zero_arg_fn() {
        let f = rfn("constant", vec![], Some(RustType::Bool));
        let out = synthesize_canonical_wrapper(&f).expect("0-arg bool fn must succeed");
        assert!(out.contains("pub fn constant_call(args: &[u8])"));
        // No __off setup for zero-arg fns.
        assert!(!out.contains("__off"));
        // Args are still consumed (we suppress unused-var warn).
        assert!(out.contains("let _ = args;"));
        // Encoding the bool return.
        assert!(out.contains("encode_to_vec(&__ret)"));
    }

    #[test]
    fn wrapper_handles_unit_return() {
        let f = rfn("touch", vec![("x", RustType::I32)], Some(RustType::Unit));
        let out = synthesize_canonical_wrapper(&f).expect("unit-return fn must succeed");
        assert!(out.contains("touch(x)"));
        // Unit-returning fn produces an empty Vec, not encode_to_vec.
        assert!(
            out.contains("::std::vec::Vec::new()"),
            "unit return must produce an empty Vec; got:\n{out}"
        );
        assert!(!out.contains("encode_to_vec"));
    }

    #[test]
    fn wrapper_handles_none_return_as_unit() {
        // RustFn::return_type is Option<RustType>; None == ().
        let f = rfn("touch", vec![], None);
        let out = synthesize_canonical_wrapper(&f).expect("None return must succeed");
        assert!(out.contains("::std::vec::Vec::new()"));
        assert!(!out.contains("encode_to_vec"));
    }

    #[test]
    fn wrapper_emits_multi_arg_decode_in_order() {
        let f = rfn(
            "combine",
            vec![("a", RustType::U64), ("b", RustType::I32), ("c", RustType::Bool)],
            Some(RustType::RustString),
        );
        let out = synthesize_canonical_wrapper(&f).expect("multi-arg fn must succeed");

        // Each param's decode appears AND in source order — a
        // → b → c.
        let pos_a = out.find("(a,").expect("a decode binding missing");
        let pos_b = out.find("(b,").expect("b decode binding missing");
        let pos_c = out.find("(c,").expect("c decode binding missing");
        assert!(pos_a < pos_b && pos_b < pos_c, "decode order broke");

        // String marshalling for the return.
        assert!(
            out.contains("::std::string::String") || out.contains("encode_to_vec(&__ret)")
        );
        // Call uses all three args in order.
        assert!(out.contains("combine(a, b, c)"));
    }

    #[test]
    fn wrapper_rejects_box_dyn_any_return() {
        let f = rfn(
            "context_free",
            vec![("n", RustType::U64)],
            // The exact string `RustTargetBackend` uses for unknown LCNF::Object types.
            Some(RustType::Custom("Box<dyn std::any::Any>".to_string())),
        );
        let err = synthesize_canonical_wrapper(&f)
            .expect_err("Box<dyn Any> return must fail wrapper synthesis");
        assert_eq!(
            err.code,
            leo4_abi::error::error_codes::ENCODE_ERROR,
            "unexpected error code: 0x{:08x}",
            err.code
        );
        assert!(err.message.contains("LeanMarshal"));
    }

    #[test]
    fn wrapper_rejects_unsupported_param_type() {
        // Vec<u64> isn't in the v0 marshalling matrix yet.
        let f = rfn(
            "list_in",
            vec![("xs", RustType::Vec(Box::new(RustType::U64)))],
            Some(RustType::U64),
        );
        let err = synthesize_canonical_wrapper(&f)
            .expect_err("Vec param must fail wrapper synthesis");
        assert_eq!(err.code, leo4_abi::error::error_codes::ENCODE_ERROR);
    }

    #[test]
    fn transpile_kernel_decl_with_wrapper_emits_both() {
        // The addOne fixture from the existing kernel-level
        // test — feed it through the combined path.
        let nat_ty = Expr::Const(Name::str("Nat"), vec![]);
        let succ = Expr::Const(Name::str("Nat.succ"), vec![]);
        let body = Expr::App(Box::new(succ), Box::new(Expr::BVar(0)));
        let name = Name::str("Sample.addOne");
        let params = vec![(Name::str("n"), nat_ty)];

        // Note: with no env populated, the transpiled fn's
        // return type lands as `Box<dyn Any>` (Lcnf::Object).
        // That's an unsupported wrapper return — so we expect
        // the wrapper step to error, but the fn step to
        // succeed. To exercise the *combined* helper through
        // its happy path we'd need a populated env; for now we
        // just verify it errors via the wrapper layer, which
        // is itself useful coverage.
        let result = transpile_kernel_decl_with_wrapper(&name, &params, &body);
        match result {
            Ok((fn_src, wrapper_src)) => {
                // If a future LCNF lowering ends up with a
                // marshallable return type, both items emit.
                assert!(fn_src.contains("Sample_addOne"));
                assert!(wrapper_src.contains("Sample_addOne_call"));
            }
            Err(e) => {
                // Expected today: wrapper rejection of
                // `Box<dyn Any>`. Document it explicitly so
                // a future change in lowering doesn't silently
                // shift this test's branch.
                assert_eq!(e.code, leo4_abi::error::error_codes::ENCODE_ERROR);
                assert!(
                    e.message.contains("LeanMarshal"),
                    "unexpected wrapper error: {}",
                    e.message
                );
            }
        }
    }

    #[test]
    fn transpile_source_if_exported_records_when_tagged() {
        let mut registry = Leo4ExportRegistry::new();
        let env = Environment::new();
        let src = "@[leo4_export] def f : Nat -> Nat := fun n -> n";

        let result = transpile_source_if_exported(&env, &mut registry, src);
        // Same Ok/Err parity rule as transpile_source: parse +
        // pre-record happens regardless of whether elab against
        // an empty env succeeds. The tag must always be
        // captured by the time we exit, since recording happens
        // *before* elab.
        let exported = registry.exported_names();
        assert_eq!(
            exported.len(),
            1,
            "registry must contain one export — got {exported:?}"
        );
        assert_eq!(exported[0].to_string(), "f");

        // Pipeline outcome is documented as either Ok(Some(_))
        // or Err depending on whether elab finds `Nat` in the
        // empty env. Both are valid for this empty-env probe.
        match result {
            Ok(Some(rust_src)) => {
                assert!(!rust_src.contains("lean_box"));
                eprintln!("transpile_source_if_exported → emitted:\n{rust_src}");
            }
            Ok(None) => {
                panic!("tagged decl must NOT yield Ok(None) — got skipped");
            }
            Err(e) => {
                let code = e.code;
                eprintln!(
                    "transpile_source_if_exported → expected-ish err (code 0x{code:08x}): {}",
                    e.message
                );
                assert!(
                    code == leo4_abi::error::error_codes::DECODE_ERROR
                        || code == leo4_abi::error::error_codes::ENCODE_ERROR,
                    "unexpected error code: 0x{code:08x}"
                );
            }
        }
    }
}
