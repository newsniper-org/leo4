//! OX6 step 13 — translator from `oxilean_parse_peg`'s
//! Decl/Expr AST into `oxilean_parse`'s `Decl` /
//! `SurfaceExpr` AST.
//!
//! Goal: the rest of `leo4-oxilean-build` (elab, codegen)
//! keeps consuming `oxilean_parse::Decl` unchanged; this
//! module decouples the *front-end parser choice* from the
//! pipeline so the OX3/OX4 textual pre-rewrites and the
//! oxilean-parse-direct path can both be retired in step
//! 13d once the translator covers the corpus.
//!
//! ## Coverage plan (multi-commit rollout)
//!
//! - **13a (this commit — foundation)**: skeleton + trivial
//!   `Definition` translation (Ident + Lit + flat
//!   `App`-tree synthesis from `oxilean_parse_peg::Expr::App`).
//!   `BinOp`s, attributes, every other Decl variant return
//!   `Err(TranslateError::Unsupported)` — the production
//!   pipeline is NOT yet routed through this module.
//! - **13b**: `Structure`, `Inductive`, `Class`, `Instance`,
//!   `Section`, `Namespace`; `BinOp` → `App` lowering;
//!   attribute kind mapping (`@[simp]` etc.).
//! - **13c**: wire into `transpile_source_to_unit` /
//!   `transpile_source_to_units` behind a feature flag.
//! - **13d**: switch the feature flag default to ON;
//!   oxilean-parse becomes the fallback rather than the
//!   primary parser.
//!
//! ## Scope boundary
//!
//! Constructs that have NO oxilean-parse equivalent
//! (`DefinitionByArms`, `Dsl`, `HashCommand`, `MatchBind`,
//! `IfLet`, `DotFn`, `Omit`, `Include`) MAY be returned as
//! `Err(TranslateError::Unsupported)` permanently —
//! consumers needing those surfaces operate on the
//! oxilean-parse-peg AST directly, not via the translation
//! shim.

use oxilean_parse_peg::{
    Attribute as L4Attr, BinderGroup as L4BinderGroup, BinderKind as L4BinderKind,
    Ctor as L4Ctor, Decl as L4Decl, DeclKind as L4Kind, Expr as L4Expr,
    InstanceBody as L4InstanceBody, Literal as L4Lit, StructField as L4Field,
};
use oxilean_parse::{
    AttributeKind as OxAttr, Binder as OxBinder, BinderKind as OxBinderKind,
    Constructor as OxCtor, Decl as OxDecl, DoAction as OxDoAction,
    FieldDecl as OxField, Literal as OxLit, Located, MatchArm as OxMatchArm,
    Pattern as OxPattern, SortKind, SurfaceExpr as OxExpr,
};
use oxilean_parse::span_util::dummy_span;

/// Translation failure modes. Surfaces a precise diagnostic
/// per Decl/Expr shape so the production wiring (step 13c)
/// can fall back to the legacy oxilean-parse-direct path
/// cleanly when needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslateError {
    /// The Decl / Expr variant has no translation in the
    /// 13a–13d coverage scope; either deliberately
    /// out-of-scope or not yet implemented.
    Unsupported(&'static str),
}

impl std::fmt::Display for TranslateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(what) => {
                write!(f, "leo4_translate: unsupported in 13a scope: {what}")
            }
        }
    }
}

impl std::error::Error for TranslateError {}

/// Translate a `oxilean_parse_peg::Decl` into a
/// `Located<oxilean_parse::Decl>` ready for the existing
/// elab pipeline. Span info is `dummy_span()` —
/// oxilean-parse-peg doesn't carry source spans in its AST
/// today; recovering them is future work tracked under
/// the broader "diagnostic quality" non-RC item.
pub fn translate_decl(d: &L4Decl) -> Result<Located<OxDecl>, TranslateError> {
    let inner = translate_decl_kind(&d.kind, &d.univ_params)?;
    // Attach the leo4 attributes to the translated decl
    // *if* the decl variant has an `attrs` field. Variants
    // that don't (Inductive / ClassDecl / InstanceDecl /
    // Namespace / SectionDecl / Variable / Open / Mutual /
    // Import) silently drop the per-variant attrs at this
    // step.
    let inner = attach_attrs(inner, &d.attrs);
    // OX7 typeclass step (2026-05-27): downstream code
    // (`decl_has_leo4_export`, the export-registry walk
    // in `transpile_source_to_units`, etc.) expects the
    // *outer* `Decl::Attribute { attrs: Vec<String>,
    // decl: Box<…> }` wrapper the legacy walker
    // produces — it only reads `attrs` from that wrapper,
    // not from any per-variant `attrs` field. Wrap here
    // so the translate path is observation-equivalent to
    // the legacy walker for export discovery. This
    // closes the 4 `transpile_source_*` regressions
    // introduced when the translate path became
    // production for `Lam` bodies.
    let value = if d.attrs.is_empty() {
        inner
    } else {
        let attr_names: Vec<String> =
            d.attrs.iter().map(|a| a.name.clone()).collect();
        OxDecl::Attribute {
            attrs: attr_names,
            decl: Box::new(Located::new(inner, dummy_span())),
        }
    };
    Ok(Located::new(value, dummy_span()))
}

/// Attach leo4 `Attribute`s to the variants that carry an
/// `attrs: Vec<AttributeKind>` field. Variants without
/// that field pass through unchanged.
fn attach_attrs(d: OxDecl, attrs: &[L4Attr]) -> OxDecl {
    if attrs.is_empty() {
        return d;
    }
    let ox_attrs: Vec<OxAttr> = attrs.iter().map(translate_attr).collect();
    match d {
        OxDecl::Definition { name, univ_params, ty, val, where_clauses, .. } => {
            OxDecl::Definition { name, univ_params, ty, val, where_clauses, attrs: ox_attrs }
        }
        OxDecl::Theorem { name, univ_params, ty, proof, where_clauses, .. } => {
            OxDecl::Theorem { name, univ_params, ty, proof, where_clauses, attrs: ox_attrs }
        }
        OxDecl::Axiom { name, univ_params, ty, .. } => {
            OxDecl::Axiom { name, univ_params, ty, attrs: ox_attrs }
        }
        other => other,
    }
}

/// Translate a leo4 `Attribute` (name + raw_args) into
/// oxilean's typed `AttributeKind`. Known attribute names
/// (`simp` / `ext` / `instance` / `reducible` /
/// `irreducible` / `inline` / `noinline` / `specialize`)
/// map to dedicated variants; everything else lands as
/// `Custom(name)`. `raw_args` is dropped — oxilean's typed
/// kinds don't carry argument data; downstream attribute
/// handlers re-parse from the raw form if they need to.
fn translate_attr(a: &L4Attr) -> OxAttr {
    match a.name.as_str() {
        "simp" => OxAttr::Simp,
        "ext" => OxAttr::Ext,
        "instance" => OxAttr::Instance,
        "reducible" => OxAttr::Reducible,
        "irreducible" => OxAttr::Irreducible,
        "inline" => OxAttr::Inline,
        "noinline" => OxAttr::NoInline,
        "specialize" => OxAttr::SpecializeAttr,
        _ => OxAttr::Custom(a.name.clone()),
    }
}

#[allow(clippy::too_many_lines)]
fn translate_decl_kind(
    k: &L4Kind,
    univ_params: &[String],
) -> Result<OxDecl, TranslateError> {
    match k {
        L4Kind::Definition { name, binders, ty, value } => {
            let oxbinders = translate_binders(binders)?;
            let ty = match ty {
                Some(t) => {
                    let inner_ty = translate_expr_located(t)?;
                    Some(wrap_pi(&oxbinders, inner_ty))
                }
                None => None,
            };
            let val = wrap_lam(&oxbinders, translate_expr_located(value)?);
            Ok(OxDecl::Definition {
                name: name.clone(),
                univ_params: univ_params.to_vec(),
                ty,
                val,
                where_clauses: Vec::new(),
                attrs: Vec::new(),
            })
        }
        L4Kind::Theorem { name, binders, ty, proof } => {
            let oxbinders = translate_binders(binders)?;
            let inner_ty = translate_expr_located(ty)?;
            let ty = wrap_pi(&oxbinders, inner_ty);
            let proof = wrap_lam(&oxbinders, translate_expr_located(proof)?);
            Ok(OxDecl::Theorem {
                name: name.clone(),
                univ_params: univ_params.to_vec(),
                ty,
                proof,
                where_clauses: Vec::new(),
                attrs: Vec::new(),
            })
        }
        L4Kind::Axiom { name, binders, ty } => {
            let oxbinders = translate_binders(binders)?;
            let inner_ty = translate_expr_located(ty)?;
            let ty = wrap_pi(&oxbinders, inner_ty);
            Ok(OxDecl::Axiom {
                name: name.clone(),
                univ_params: univ_params.to_vec(),
                ty,
                attrs: Vec::new(),
            })
        }
        L4Kind::Import { path } => Ok(OxDecl::Import {
            path: path.split('.').map(str::to_string).collect(),
        }),
        L4Kind::Namespace { name, decls } => {
            let mut out = Vec::with_capacity(decls.len());
            for inner in decls {
                out.push(translate_decl(inner)?);
            }
            Ok(OxDecl::Namespace { name: name.clone(), decls: out })
        }
        L4Kind::DefinitionByArms { .. } => {
            Err(TranslateError::Unsupported("DefinitionByArms (no oxilean equivalent)"))
        }
        L4Kind::Example { .. } => Err(TranslateError::Unsupported("Example (lands in 13b-5)")),
        L4Kind::Structure { name, extends, fields, deriving: _ } => {
            // `deriving` info has no place on
            // oxilean's `Structure` variant — it surfaces
            // via separate `Derive { … }` decls. The
            // strict-superset translator drops it
            // silently; the elab pipeline can re-emit
            // `Derive` decls if downstream cares.
            Ok(OxDecl::Structure {
                name: name.clone(),
                univ_params: univ_params.to_vec(),
                extends: extends.clone(),
                fields: translate_fields(fields)?,
            })
        }
        L4Kind::Class { name, binders, extends, fields, deriving: _ } => {
            if !binders.is_empty() {
                return Err(TranslateError::Unsupported(
                    "Class binders (lands in 13b-5)",
                ));
            }
            Ok(OxDecl::ClassDecl {
                name: name.clone(),
                univ_params: univ_params.to_vec(),
                extends: extends.clone(),
                fields: translate_fields(fields)?,
            })
        }
        L4Kind::Inductive { name, ty, ctors, deriving: _ } => {
            // `ty` defaults to `Sort(Type)` when the
            // source omits the annotation (`inductive
            // Color where | red | green` style).
            let ind_ty = match ty {
                Some(t) => translate_expr_located(t)?,
                None => Located::new(OxExpr::Sort(SortKind::Type), dummy_span()),
            };
            let ox_ctors = ctors
                .iter()
                .map(|c| translate_ctor(c, name))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(OxDecl::Inductive {
                name: name.clone(),
                univ_params: univ_params.to_vec(),
                params: Vec::new(),
                indices: Vec::new(),
                ty: ind_ty,
                ctors: ox_ctors,
            })
        }
        L4Kind::Instance { name, binders, ty, body } => {
            if !binders.is_empty() {
                return Err(TranslateError::Unsupported(
                    "Instance binders (lands in 13b-5)",
                ));
            }
            // oxilean wants `class_name` as a separate
            // field; in leo4 it lives at the head of the
            // instance type's App chain (`Monad List` →
            // head `Monad`).
            let class_name = head_ident(ty).ok_or(TranslateError::Unsupported(
                "Instance type with non-ident head (e.g. `(C ∘ D) X`)",
            ))?;
            let ox_ty = translate_expr_located(ty)?;
            let defs = match body {
                L4InstanceBody::Where(fields) => {
                    fields
                        .iter()
                        .map(|f| {
                            translate_expr_located(&f.ty).map(|e| (f.name.clone(), e))
                        })
                        .collect::<Result<Vec<_>, _>>()?
                }
                L4InstanceBody::Term(_) => {
                    return Err(TranslateError::Unsupported(
                        "Instance with single-term body (lands in 13b-5)",
                    ));
                }
            };
            Ok(OxDecl::InstanceDecl {
                name: name.clone(),
                class_name,
                ty: ox_ty,
                defs,
            })
        }
        L4Kind::Section { name, decls } => {
            // oxilean's `SectionDecl` requires a name —
            // anonymous leo4 sections (`section` with no
            // name) get the empty string. Matches Lean 4's
            // own behaviour for `section ... end` blocks
            // without a name marker.
            let mut out = Vec::with_capacity(decls.len());
            for inner in decls {
                out.push(translate_decl(inner)?);
            }
            Ok(OxDecl::SectionDecl {
                name: name.clone().unwrap_or_default(),
                decls: out,
            })
        }
        L4Kind::Mutual { .. } => Err(TranslateError::Unsupported("Mutual (deferred)")),
        L4Kind::Open { items, raw_tail } => {
            // leo4 captures a *list* of opened modules
            // (`open Foo Bar Baz` → items=[Foo, Bar, Baz]).
            // oxilean's `Open` decl carries ONE module per
            // decl. For 13b-5, only single-item opens
            // translate; multi-item opens would need to
            // split into multiple decls (caller's job —
            // not expressible via translate_decl's
            // 1→1 return). Selective form
            // (`open Foo (x y)`) and renaming
            // (`open Foo renaming x → y`) live in
            // raw_tail and are deferred — translation
            // returns Unsupported when raw_tail is
            // non-empty.
            if items.len() != 1 {
                return Err(TranslateError::Unsupported(
                    "Open with 0 or >1 items (caller must split)",
                ));
            }
            if !raw_tail.trim().is_empty() {
                return Err(TranslateError::Unsupported(
                    "Open with selective / renaming tail",
                ));
            }
            Ok(OxDecl::Open {
                path: items[0].split('.').map(str::to_string).collect(),
                names: Vec::new(),
            })
        }
        L4Kind::Variable { binders } => {
            let ox_binders = translate_binders(binders)?;
            Ok(OxDecl::Variable { binders: ox_binders })
        }
        L4Kind::Dsl { .. }
        | L4Kind::HashCommand { .. }
        | L4Kind::Omit { .. }
        | L4Kind::Include { .. } => {
            Err(TranslateError::Unsupported("DSL / # / omit / include (no oxilean equivalent)"))
        }
    }
}

fn translate_expr_located(e: &L4Expr) -> Result<Located<OxExpr>, TranslateError> {
    Ok(Located::new(translate_expr(e)?, dummy_span()))
}

/// Translate a list of `BinderGroup`s into oxilean's
/// per-name `Vec<Binder>`. Each group `(a b : Nat)` →
/// `[Binder { name: "a", ty: Some(Nat), info: Default },
///   Binder { name: "b", ty: Some(Nat), info: Default }]`.
fn translate_binders(groups: &[L4BinderGroup]) -> Result<Vec<OxBinder>, TranslateError> {
    let mut out = Vec::new();
    for g in groups {
        let ty = translate_expr_located(&g.ty)?;
        let info = translate_binder_kind(&g.kind);
        for name in &g.names {
            out.push(OxBinder {
                name: name.clone(),
                ty: Some(Box::new(ty.clone())),
                info: info.clone(),
            });
        }
    }
    Ok(out)
}

fn translate_binder_kind(k: &L4BinderKind) -> OxBinderKind {
    match k {
        L4BinderKind::Explicit => OxBinderKind::Default,
        L4BinderKind::Implicit => OxBinderKind::Implicit,
        L4BinderKind::Instance => OxBinderKind::Instance,
    }
}

/// OX7 (α, 2026-05-27) — expand one `LamBinder` (the
/// PEG's lambda-binder shape) into one or more
/// `OxBinder`s (the surface-AST shape). `Typed`'s
/// `names` list spreads the same `ty` across each
/// name; `Untyped` becomes a single binder with no
/// annotation (oxilean-elab will infer the type from
/// context).
fn lam_binder_to_ox_binders(
    lb: &oxilean_parse_peg::LamBinder,
) -> Result<Vec<OxBinder>, TranslateError> {
    use oxilean_parse_peg::LamBinder;
    match lb {
        LamBinder::Untyped(name) => Ok(vec![OxBinder {
            name: name.clone(),
            ty: None,
            info: OxBinderKind::Default,
        }]),
        LamBinder::Typed { kind, names, ty } => {
            let ty_loc = translate_expr_located(ty)?;
            let info = translate_binder_kind(kind);
            let mut out = Vec::with_capacity(names.len());
            for name in names {
                out.push(OxBinder {
                    name: name.clone(),
                    ty: Some(Box::new(ty_loc.clone())),
                    info: info.clone(),
                });
            }
            Ok(out)
        }
    }
}

/// OX7 (2026-05-27) — translate one PEG `DoStmt` into
/// the surface AST's `OxDoAction`. The PEG enum is
/// wider — it carries `For`, `While`, `Until` (and
/// maybe more layout-sensitive variants); these surface
/// `TranslateError::Unsupported` so the legacy walker
/// picks the source up. The common quadruple
/// (`Let` / `Bind` / `Return` / pure expr) translates
/// cleanly.
fn translate_do_stmt(
    s: &oxilean_parse_peg::DoStmt,
) -> Result<OxDoAction, TranslateError> {
    use oxilean_parse_peg::DoStmt;
    match s {
        DoStmt::Let { name, value } => {
            let v = translate_expr_located(value)?;
            Ok(OxDoAction::Let(name.clone(), v))
        }
        DoStmt::Bind { name, value } => {
            let v = translate_expr_located(value)?;
            Ok(OxDoAction::Bind(name.clone(), v))
        }
        DoStmt::Return(value) => {
            let v = translate_expr_located(value)?;
            Ok(OxDoAction::Return(v))
        }
        DoStmt::Expr(value) => {
            // Bare expression statement (its effect is
            // sequenced via the monad). Surface AST's
            // `OxDoAction::Expr` takes the same shape.
            let v = translate_expr_located(value)?;
            Ok(OxDoAction::Expr(v))
        }
        // Catch-all: surfaces the variant name in the
        // diagnostic so the production-coverage step
        // knows what's still missing.
        other => Err(TranslateError::Unsupported(
            do_stmt_variant_name(other),
        )),
    }
}

/// OX7 (2026-05-27) — `oxilean_parse_peg::Pattern` →
/// `oxilean_parse::Pattern` translation for `Match` arm
/// bodies. Wildcard / Var / Lit / Ctor map 1-to-1.
/// `DotCtor(name, args)` becomes `Ctor(name, args)` —
/// oxilean-parse has no dedicated `.ctor` variant and
/// the elab path treats both forms the same. `Paren`
/// unwraps. `Tuple` is `Unsupported` until oxilean-parse
/// grows a Tuple-pattern variant or we lower to a
/// chained anonymous-ctor.
fn translate_pattern(
    p: &oxilean_parse_peg::Pattern,
) -> Result<OxPattern, TranslateError> {
    use oxilean_parse_peg::Pattern as L4Pat;
    match p {
        L4Pat::Wildcard => Ok(OxPattern::Wild),
        L4Pat::Var(name) => Ok(OxPattern::Var(name.clone())),
        L4Pat::Lit(lit) => Ok(OxPattern::Lit(translate_literal(lit))),
        L4Pat::Ctor(name, args) => {
            let mut out = Vec::with_capacity(args.len());
            for a in args {
                out.push(Located::new(translate_pattern(a)?, dummy_span()));
            }
            Ok(OxPattern::Ctor(name.clone(), out))
        }
        L4Pat::DotCtor(name, args) => {
            let mut out = Vec::with_capacity(args.len());
            for a in args {
                out.push(Located::new(translate_pattern(a)?, dummy_span()));
            }
            // Lean 4's `.ctor` is shorthand for the
            // namespace-qualified `T.ctor` where `T` is
            // the scrutinee's type. The elaborator
            // resolves the namespace; for translation we
            // just hand the bare name to the same `Ctor`
            // variant.
            Ok(OxPattern::Ctor(name.clone(), out))
        }
        L4Pat::Paren(inner) => translate_pattern(inner),
        L4Pat::Tuple(_) => Err(TranslateError::Unsupported("Pattern::Tuple")),
    }
}

/// Map `oxilean_parse_peg::Literal` to `oxilean_parse::
/// Literal`. Trivial 1-to-1 for the four currently
/// supported variants (Nat, Float as string, Str, Char).
fn translate_literal(l: &oxilean_parse_peg::Literal) -> OxLit {
    use oxilean_parse_peg::Literal as L4Lit2;
    match l {
        L4Lit2::Nat(n) => OxLit::Nat(*n),
        L4Lit2::Str(s) => OxLit::String(s.clone()),
        L4Lit2::Float(s) => {
            // OxLit::Float takes f64; we round-trip via
            // parse (acceptable: any literal that parsed
            // as Float-shaped text under PEG also parses
            // as a Rust f64).
            s.parse::<f64>()
                .map(OxLit::Float)
                .unwrap_or_else(|_| OxLit::String(s.clone()))
        }
    }
}

fn do_stmt_variant_name(s: &oxilean_parse_peg::DoStmt) -> &'static str {
    use oxilean_parse_peg::DoStmt;
    match s {
        DoStmt::Let { .. }     => "DoStmt::Let",
        DoStmt::Bind { .. }    => "DoStmt::Bind",
        DoStmt::Return(_)      => "DoStmt::Return",
        DoStmt::For { .. }     => "DoStmt::For",
        DoStmt::While { .. }   => "DoStmt::While",
        DoStmt::Until { .. }   => "DoStmt::Until",
        _ => "DoStmt::<other>",
    }
}

/// Wrap `inner` in a Pi-type over the given binders.
/// Empty binders pass through unchanged.
fn wrap_pi(binders: &[OxBinder], inner: Located<OxExpr>) -> Located<OxExpr> {
    if binders.is_empty() {
        inner
    } else {
        Located::new(OxExpr::Pi(binders.to_vec(), Box::new(inner)), dummy_span())
    }
}

/// Wrap `inner` in a Lambda over the given binders.
/// Empty binders pass through unchanged.
fn wrap_lam(binders: &[OxBinder], inner: Located<OxExpr>) -> Located<OxExpr> {
    if binders.is_empty() {
        inner
    } else {
        Located::new(OxExpr::Lam(binders.to_vec(), Box::new(inner)), dummy_span())
    }
}

/// Translate a list of leo4 `StructField`s into oxilean's
/// `FieldDecl`s. leo4 doesn't track per-field default
/// values today — `default: None` always.
fn translate_fields(fields: &[L4Field]) -> Result<Vec<OxField>, TranslateError> {
    fields
        .iter()
        .map(|f| {
            Ok(OxField {
                name: f.name.clone(),
                ty: translate_expr_located(&f.ty)?,
                default: None,
            })
        })
        .collect()
}

/// Translate a leo4 inductive `Ctor` into oxilean's
/// `Constructor`. A bare ctor (`| red`) has `ty: None` in
/// leo4 — the ctor's type is the inductive itself, so we
/// synthesize `Var(inductive_name)` to satisfy oxilean's
/// required `ty: Located<SurfaceExpr>` field.
fn translate_ctor(c: &L4Ctor, inductive_name: &str) -> Result<OxCtor, TranslateError> {
    let ty = match &c.ty {
        Some(t) => translate_expr_located(t)?,
        None => Located::new(OxExpr::Var(inductive_name.to_string()), dummy_span()),
    };
    Ok(OxCtor { name: c.name.clone(), ty })
}

/// Walk the head of an `Expr::App` chain to find the
/// leftmost `Ident` — used to extract the class name from
/// an instance type (`Monad List` → `"Monad"`).
fn head_ident(e: &L4Expr) -> Option<String> {
    match e {
        L4Expr::Ident(s) => Some(s.clone()),
        L4Expr::App(f, _) => head_ident(f),
        L4Expr::Paren(inner) => head_ident(inner),
        _ => None,
    }
}

/// OX7 typeclass step (2026-05-27): map a surface
/// arithmetic / comparison operator symbol to the
/// Lean stdlib typeclass-projection identifier
/// oxilean-elab + leo4_env_bootstrap expect. Unknown
/// operators fall through verbatim — they'll surface
/// as NameNotFound during elab if the env doesn't
/// carry them. Mirror of
/// `leo4_env_bootstrap::ARITHMETIC_TC_PROJECTIONS`.
fn arith_op_to_tc_projection(op: &str) -> &str {
    match op {
        // Arithmetic.
        "+" => "HAdd.hAdd",
        "-" => "HSub.hSub",
        "*" => "HMul.hMul",
        "/" => "HDiv.hDiv",
        "%" => "HMod.hMod",
        "^" => "HPow.hPow",
        // Bitwise.
        "&&&" => "HAnd.hAnd",
        "|||" => "HOr.hOr",
        "^^^" => "HXor.hXor",
        "<<<" => "HShiftLeft.hShiftLeft",
        ">>>" => "HShiftRight.hShiftRight",
        // Comparison.
        "<" => "LT.lt",
        "<=" | "≤" => "LE.le",
        // `a > b` and `a ≥ b` swap to `LT.lt b a` /
        // `LE.le b a` in Lean stdlib. We can't do the
        // arg swap here without restructuring the App
        // tree, so for now they pass through and
        // codegen handles them as TODO — same as the
        // legacy lowering's behaviour.
        "==" => "BEq.beq",
        // OX7 (2026-05-27): propositional equality `a = b`
        // lowers to `Eq.eq a b`. Distinct from `==` (BEq).
        "=" => "Eq.eq",
        // Unmapped — keep `Var(op)` for the legacy path.
        // Includes ">", ">=", "&&", "||", "!=", "≠",
        // "→", "↔", "∈", "∉", "⊆", etc. Some will
        // surface as NameNotFound until env coverage
        // expands; this is intentional (we don't want
        // silent fallback for operators with non-trivial
        // semantics).
        _ => op,
    }
}

fn translate_expr(e: &L4Expr) -> Result<OxExpr, TranslateError> {
    match e {
        L4Expr::Ident(s) => Ok(OxExpr::Var(s.clone())),
        L4Expr::Lit(L4Lit::Nat(n)) => Ok(OxExpr::Lit(OxLit::Nat(*n))),
        L4Expr::Lit(L4Lit::Str(s)) => Ok(OxExpr::Lit(OxLit::String(s.clone()))),
        L4Expr::Lit(L4Lit::Float(s)) => {
            // oxilean-parse-peg holds floats as the raw source
            // text (NaN-comparable preserve); oxilean takes
            // an f64. A round-trip failure here is a
            // diagnostic-quality issue, not a translation
            // bug — fall through with `Unsupported` so the
            // production path can decide whether to retry
            // via the legacy parser.
            s.parse::<f64>().map(|f| OxExpr::Lit(OxLit::Float(f)))
                .map_err(|_| TranslateError::Unsupported("Float literal not parseable as f64"))
        }
        L4Expr::App(f, x) => {
            let f = translate_expr_located(f)?;
            let x = translate_expr_located(x)?;
            Ok(OxExpr::App(Box::new(f), Box::new(x)))
        }
        L4Expr::Paren(inner) => translate_expr(inner),
        L4Expr::BinOp(op, lhs, rhs) => {
            // OX7 (α, 2026-05-27): the `->` function-type
            // arrow is a BinOp at the PEG level but
            // lowers to a non-dependent `Pi` in surface
            // AST, not an application of a `Var("->"")`.
            // Handle it specially before the arithmetic
            // mapping table.
            if op == "->" {
                let dom = translate_expr_located(lhs)?;
                let codom = translate_expr_located(rhs)?;
                let binder = OxBinder {
                    name: "_".to_string(),
                    ty: Some(Box::new(dom)),
                    info: OxBinderKind::Default,
                };
                return Ok(OxExpr::Pi(vec![binder], Box::new(codom)));
            }
            // OX7 typeclass step (2026-05-27): map the
            // surface operator to its Lean stdlib
            // typeclass-projection identifier
            // (`HAdd.hAdd`, `LT.lt`, …). oxilean-parse-peg
            // preserves the operator symbol verbatim
            // (`BinOp("+", lhs, rhs)`), unlike the legacy
            // oxilean-parse which desugared at parse
            // time. We do the desugar here so the
            // identifier looks the same to oxilean-elab
            // regardless of which parser produced the
            // tree.
            //
            // The mapped identifier needs to be present
            // in the env so elab's identifier-lookup
            // succeeds; `leo4_env_bootstrap` registers
            // each one as a leaf axiom
            // (`ARITHMETIC_TC_PROJECTIONS`). Codegen
            // pattern-matches on the `Const(name)` head
            // at LCNF time to emit native Rust BinOps.
            //
            // Unknown operators (currently anything not
            // in the table) pass through as `Var(op)`
            // for compatibility with the legacy lowering
            // path; they'll surface as NameNotFound if
            // the env doesn't carry an entry.
            let mapped_op = arith_op_to_tc_projection(op);
            let f = Located::new(
                OxExpr::Var(mapped_op.to_string()),
                dummy_span(),
            );
            let lhs = translate_expr_located(lhs)?;
            let rhs = translate_expr_located(rhs)?;
            let f_lhs = Located::new(
                OxExpr::App(Box::new(f), Box::new(lhs)),
                dummy_span(),
            );
            Ok(OxExpr::App(Box::new(f_lhs), Box::new(rhs)))
        }
        L4Expr::UnaryOp(op, x) => {
            // OX7 (2026-05-27): same typeclass-projection
            // desugar as BinOp — `-` (prefix) → `Neg.neg`,
            // `!` → `Not.not`. Unknown ops pass through.
            let mapped_op: &str = match op.as_str() {
                "-" => "Neg.neg",
                "!" => "Not.not",
                other => other,
            };
            let f = Located::new(OxExpr::Var(mapped_op.to_string()), dummy_span());
            let x = translate_expr_located(x)?;
            Ok(OxExpr::App(Box::new(f), Box::new(x)))
        }
        // OX7 (2026-05-27) — coverage expansion. Each
        // arm here landed once a coverage spike showed
        // a real fixture falling back to the legacy
        // walker. Pre-2026-05-27 these all hit the
        // catch-all `Unsupported` branch.
        L4Expr::If(cond, then_branch, else_branch) => {
            let cond = translate_expr_located(cond)?;
            let then_b = translate_expr_located(then_branch)?;
            let else_b = translate_expr_located(else_branch)?;
            Ok(OxExpr::If(Box::new(cond), Box::new(then_b), Box::new(else_b)))
        }
        L4Expr::Let { name, ty, value, body } => {
            // PEG carries an optional type annotation; the
            // surface AST's `Let` slot also takes `Option`.
            let ty_loc = match ty {
                Some(t) => Some(Box::new(translate_expr_located(t)?)),
                None => None,
            };
            let value = translate_expr_located(value)?;
            let body = translate_expr_located(body)?;
            Ok(OxExpr::Let(
                name.clone(),
                ty_loc,
                Box::new(value),
                Box::new(body),
            ))
        }
        L4Expr::List(items) => {
            // Lean 4's `[a, b, c]` literal — surface AST
            // has a matching `ListLit` variant; downstream
            // elab desugars to the `List.cons … List.nil`
            // tree.
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                out.push(translate_expr_located(it)?);
            }
            Ok(OxExpr::ListLit(out))
        }
        L4Expr::At(inner) => {
            // `@f x y` — explicit-args marker. The surface
            // AST has no dedicated variant for this; elab
            // distinguishes implicit vs explicit at the
            // applied-arg level, not at the head. So we
            // pass the inner expression through
            // transparently — semantics-preserving in the
            // rust-transpile path which doesn't synthesise
            // implicit args anyway (codegen sees only the
            // explicit App-tree).
            translate_expr(inner)
        }
        // OX7 (2026-05-27) coverage expansion, 2nd batch:
        // dependent / monadic / anonymous-ctor surfaces.
        L4Expr::Forall(binders, body) => {
            // `∀ (x : Nat), P x` / `forall (x : Nat), P x`
            // — universal quantification at type level.
            // Lowers to a dependent `OxExpr::Pi` over the
            // bound parameters. Same binder-expansion as
            // `Lam` (see `lam_binder_to_ox_binders`).
            let mut ox_binders: Vec<OxBinder> = Vec::new();
            for lb in binders {
                ox_binders.extend(lam_binder_to_ox_binders(lb)?);
            }
            let body_loc = translate_expr_located(body)?;
            Ok(OxExpr::Pi(ox_binders, Box::new(body_loc)))
        }
        L4Expr::Do(stmts) => {
            // `do { ... }` — monadic block. PEG's
            // `DoStmt` enum is wider than oxilean-parse's
            // `DoAction` (we also have `For`, `While`,
            // `Until`); the unsupported variants surface
            // a `TranslateError::Unsupported("DoStmt::<x>")`
            // so the legacy walker can still pick the
            // source up.
            let mut actions: Vec<OxDoAction> = Vec::with_capacity(stmts.len());
            for s in stmts {
                actions.push(translate_do_stmt(s)?);
            }
            Ok(OxExpr::Do(actions))
        }
        L4Expr::AnonCtor(items) => {
            // `⟨a, b, c⟩` — anonymous constructor for
            // structures / inductives with a single
            // explicit ctor. Surface AST has a direct
            // `AnonymousCtor` variant.
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                out.push(Located::new(translate_expr(it)?, dummy_span()));
            }
            Ok(OxExpr::AnonymousCtor(out))
        }
        L4Expr::Exists(binders, body) => {
            // `∃ (x : T), P x` desugars to `Exists (fun
            // (x : T) => P x)` in Lean 4. Lower to an
            // explicit `App(Var("Exists"), Lam(...))`. The
            // elaborator + codegen handle this as any
            // other higher-order app; the env should
            // carry `Exists` as an axiom (or
            // `init_builtin_env` from OxiLean ships it).
            let mut ox_binders: Vec<OxBinder> = Vec::new();
            for lb in binders {
                ox_binders.extend(lam_binder_to_ox_binders(lb)?);
            }
            let body_loc = translate_expr_located(body)?;
            let lam = OxExpr::Lam(ox_binders, Box::new(body_loc));
            let lam_loc = Located::new(lam, dummy_span());
            let head = Located::new(
                OxExpr::Var("Exists".to_string()),
                dummy_span(),
            );
            Ok(OxExpr::App(Box::new(head), Box::new(lam_loc)))
        }
        L4Expr::IfLet { pattern, scrutinee, then_branch, else_branch } => {
            // `if let pat := scrut then t else e` ≡
            // `match scrut with | pat => t | _ => e`.
            // Direct desugar; the `_` fallback ensures
            // exhaustiveness.
            let scrut_loc = translate_expr_located(scrutinee)?;
            let pat = translate_pattern(pattern)?;
            let pat_loc = Located::new(pat, dummy_span());
            let then_loc = translate_expr_located(then_branch)?;
            let else_loc = translate_expr_located(else_branch)?;
            let arm_match = OxMatchArm {
                pattern: pat_loc,
                guard: None,
                rhs: then_loc,
            };
            let arm_wild = OxMatchArm {
                pattern: Located::new(OxPattern::Wild, dummy_span()),
                guard: None,
                rhs: else_loc,
            };
            Ok(OxExpr::Match(
                Box::new(scrut_loc),
                vec![arm_match, arm_wild],
            ))
        }
        L4Expr::MatchBind { binding: _, scrutinee, arms } => {
            // `match h : SCRUT with | …` — scrutinee
            // binding form. The `h : SCRUT = pat` evidence
            // is propositional; the rust-transpile path
            // doesn't model propositions, so we drop the
            // binding and translate as a plain `Match`.
            // Lossy on the proof side but semantically
            // equivalent at the value level.
            let scrut_loc = translate_expr_located(scrutinee)?;
            let mut ox_arms: Vec<OxMatchArm> = Vec::with_capacity(arms.len());
            for arm in arms {
                let pat = translate_pattern(&arm.pattern)?;
                let pat_loc = Located::new(pat, dummy_span());
                let guard_loc = match &arm.guard {
                    Some(g) => Some(translate_expr_located(g)?),
                    None => None,
                };
                let rhs_loc = translate_expr_located(&arm.body)?;
                ox_arms.push(OxMatchArm {
                    pattern: pat_loc,
                    guard: guard_loc,
                    rhs: rhs_loc,
                });
            }
            Ok(OxExpr::Match(Box::new(scrut_loc), ox_arms))
        }
        L4Expr::AnonStruct(fields) => {
            // `{ x := 1, y := 2 }` — named-field
            // anonymous struct. Surface AST has no direct
            // variant; closest match is `AnonymousCtor`
            // with field values in declaration order. The
            // elaborator infers field names from the
            // target type. Loss: source field names. Most
            // structs ctor-from-positional just fine in
            // Lean 4, so this is acceptable for the
            // rust-transpile primitive subset.
            let mut out = Vec::with_capacity(fields.len());
            for (_field_name, value) in fields {
                out.push(Located::new(translate_expr(value)?, dummy_span()));
            }
            Ok(OxExpr::AnonymousCtor(out))
        }
        L4Expr::Match(scrut, arms) => {
            // `match SCRUT with | pat1 => body1 | pat2 => body2 …`
            // Direct Pattern + arm shape mapping. Patterns
            // that don't have an oxilean-parse equivalent
            // (e.g. `Pattern::Tuple`) surface as
            // `TranslateError::Unsupported("Pattern::<name>")`.
            let scrut_loc = translate_expr_located(scrut)?;
            let mut ox_arms: Vec<OxMatchArm> = Vec::with_capacity(arms.len());
            for arm in arms {
                let pat = translate_pattern(&arm.pattern)?;
                let pat_loc = Located::new(pat, dummy_span());
                let guard_loc = match &arm.guard {
                    Some(g) => Some(translate_expr_located(g)?),
                    None => None,
                };
                let rhs_loc = translate_expr_located(&arm.body)?;
                ox_arms.push(OxMatchArm {
                    pattern: pat_loc,
                    guard: guard_loc,
                    rhs: rhs_loc,
                });
            }
            Ok(OxExpr::Match(Box::new(scrut_loc), ox_arms))
        }
        // OX7 (α, 2026-05-27) — `fun BINDERS => body` and
        // its synonyms. PEG `LamBinder::Typed { names,
        // ty, kind }` expands to one OxBinder per name
        // sharing the same type; `LamBinder::Untyped`
        // becomes a no-annotation OxBinder. The body
        // arrow's two surface forms (`=>` / `->`) are
        // already normalised by `lean4_normalize` at
        // OX3 — both reach us as the same `Lam` shape.
        //
        // Top-level `def`s emit a Lam wrapping the
        // body whose binders mirror the declaration's
        // params. This is the path that lit up
        // NameNotFound on `+` — `def add (a b :
        // UInt64) ...` PEG's the body as
        // `Lam(binders, BinOp("+", Var("a"), Var("b")))`.
        L4Expr::Lam(binders, body) => {
            let mut ox_binders: Vec<OxBinder> = Vec::new();
            for lb in binders {
                let group = lam_binder_to_ox_binders(lb)?;
                ox_binders.extend(group);
            }
            let body_loc = translate_expr_located(body)?;
            Ok(OxExpr::Lam(ox_binders, Box::new(body_loc)))
        }
        // OX7 (γ, 2026-05-27): name the variant in the
        // diagnostic so production logs say *which*
        // shape forced the legacy-walker fallback. The
        // catch-all also points the user at the
        // production-code path that needs the new arm.
        other => Err(TranslateError::Unsupported(
            expr_variant_name(other),
        )),
    }
}

/// OX7 (γ, 2026-05-27) — return a stable, short
/// identifier for an `L4Expr` variant, used in
/// `TranslateError::Unsupported` so production logs
/// pinpoint the exact shape that fell back to the
/// legacy walker.
fn expr_variant_name(e: &L4Expr) -> &'static str {
    match e {
        L4Expr::Ident(_) => "Ident",
        L4Expr::Lit(_) => "Lit",
        L4Expr::App(_, _) => "App",
        L4Expr::BinOp(_, _, _) => "BinOp",
        L4Expr::UnaryOp(_, _) => "UnaryOp",
        L4Expr::Paren(_) => "Paren",
        L4Expr::If(_, _, _) => "If",
        L4Expr::IfLet { .. } => "IfLet",
        L4Expr::Match(_, _) => "Match",
        L4Expr::MatchBind { .. } => "MatchBind",
        L4Expr::Lam(_, _) => "Lam",
        L4Expr::Let { .. } => "Let",
        L4Expr::Forall(_, _) => "Forall",
        L4Expr::Exists(_, _) => "Exists",
        L4Expr::Do(_) => "Do",
        L4Expr::InterpStr(_) => "InterpStr",
        L4Expr::By(_) => "By",
        L4Expr::List(_) => "List",
        L4Expr::AnonStruct(_) => "AnonStruct",
        L4Expr::AnonCtor(_) => "AnonCtor",
        L4Expr::At(_) => "At",
        L4Expr::DotFn(_) => "DotFn",
        L4Expr::Raw(_) => "Raw",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxilean_parse_peg::parse_decls;

    /// Helper: parse one decl and translate it. Returns
    /// the inner `OxDecl` (panics on parse/translate err so
    /// failure shows the diagnostic).
    fn parse_and_translate(src: &str) -> OxDecl {
        let decls = parse_decls(src).expect("oxilean-parse-peg must accept");
        assert_eq!(decls.len(), 1, "expected exactly one decl in `{src}`");
        translate_decl(&decls[0])
            .unwrap_or_else(|e| panic!("translate must succeed for `{src}`: {e}"))
            .value
    }

    #[test]
    fn definition_with_ident_body() {
        let d = parse_and_translate("def x : Nat := y");
        match d {
            OxDecl::Definition { name, val, ty, .. } => {
                assert_eq!(name, "x");
                assert!(matches!(val.value, OxExpr::Var(ref s) if s == "y"));
                let ty_val = ty.expect("ty must be Some");
                assert!(matches!(ty_val.value, OxExpr::Var(ref s) if s == "Nat"));
            }
            other => panic!("expected Definition, got {other:?}"),
        }
    }

    #[test]
    fn definition_with_nat_lit_body() {
        let d = parse_and_translate("def n : Nat := 42");
        match d {
            OxDecl::Definition { val, .. } => {
                assert!(matches!(val.value, OxExpr::Lit(OxLit::Nat(42))));
            }
            other => panic!("expected Definition, got {other:?}"),
        }
    }

    #[test]
    fn definition_with_app_body() {
        let d = parse_and_translate("def y : Nat := f x");
        match d {
            OxDecl::Definition { val, .. } => {
                let OxExpr::App(f, x) = val.value else {
                    panic!("expected App");
                };
                assert!(matches!(f.value, OxExpr::Var(ref s) if s == "f"));
                assert!(matches!(x.value, OxExpr::Var(ref s) if s == "x"));
            }
            other => panic!("expected Definition, got {other:?}"),
        }
    }

    #[test]
    fn definition_with_no_type_ann_is_unsupported_today() {
        // No type annotation → `ty: None`. In 13a we still
        // support this (the field is Option<…>).
        let d = parse_and_translate("def z := 1");
        match d {
            OxDecl::Definition { name, ty, .. } => {
                assert_eq!(name, "z");
                assert!(ty.is_none());
            }
            other => panic!("expected Definition, got {other:?}"),
        }
    }

    // ─── 13b-2: binders lift into Pi (type) / Lam (body) ──

    #[test]
    fn definition_with_one_explicit_binder_lifts_to_pi_and_lam() {
        // `def f (a : Nat) : T := a` should translate to:
        //   ty:  Pi([a:Nat], T)
        //   val: Lam([a], a)
        let decls = parse_decls("def f (a : Nat) : T := a").expect("must parse");
        let d = translate_decl(&decls[0]).expect("13b-2 supports binders").value;
        match d {
            OxDecl::Definition { ty, val, .. } => {
                let ty_val = ty.expect("ty must be Some");
                match ty_val.value {
                    OxExpr::Pi(binders, _body) => {
                        assert_eq!(binders.len(), 1);
                        assert_eq!(binders[0].name, "a");
                        assert_eq!(binders[0].info, OxBinderKind::Default);
                    }
                    other => panic!("expected Pi, got {other:?}"),
                }
                match val.value {
                    OxExpr::Lam(binders, _body) => {
                        assert_eq!(binders.len(), 1);
                        assert_eq!(binders[0].name, "a");
                    }
                    other => panic!("expected Lam, got {other:?}"),
                }
            }
            other => panic!("expected Definition, got {other:?}"),
        }
    }

    #[test]
    fn definition_multi_binder_group_expands_per_name() {
        // `def f (a b : Nat) : T := a` — single BinderGroup
        // with 2 names → 2 oxilean Binders.
        let decls = parse_decls("def f (a b : Nat) : T := a").expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        match d {
            OxDecl::Definition { ty, .. } => {
                let ty_val = ty.expect("ty must be Some");
                let OxExpr::Pi(binders, _) = ty_val.value else {
                    panic!("expected Pi");
                };
                assert_eq!(binders.len(), 2);
                assert_eq!(binders[0].name, "a");
                assert_eq!(binders[1].name, "b");
            }
            other => panic!("expected Definition, got {other:?}"),
        }
    }

    #[test]
    fn binder_kinds_map_correctly() {
        // Explicit `()` → Default, Implicit `{}` → Implicit,
        // Instance `[]` → Instance.
        let decls = parse_decls("def f (a : T) {b : T} [c : T] : T := a")
            .expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        let OxDecl::Definition { ty, .. } = d else { panic!("expected Definition") };
        let ty_val = ty.expect("ty must be Some");
        let OxExpr::Pi(binders, _) = ty_val.value else { panic!("expected Pi") };
        assert_eq!(binders.len(), 3);
        assert_eq!(binders[0].info, OxBinderKind::Default);
        assert_eq!(binders[1].info, OxBinderKind::Implicit);
        assert_eq!(binders[2].info, OxBinderKind::Instance);
    }

    #[test]
    fn theorem_with_binders_lifts() {
        let decls = parse_decls("theorem t (h : T) : U := h").expect("must parse");
        let d = translate_decl(&decls[0]).expect("13b-2 supports theorem binders").value;
        match d {
            OxDecl::Theorem { ty, proof, .. } => {
                assert!(matches!(ty.value, OxExpr::Pi(ref bs, _) if bs.len() == 1));
                assert!(matches!(proof.value, OxExpr::Lam(ref bs, _) if bs.len() == 1));
            }
            other => panic!("expected Theorem, got {other:?}"),
        }
    }

    #[test]
    fn no_binders_skips_pi_lam_wrap() {
        // `def x : T := y` — empty binders, ty/val pass
        // through without Pi/Lam.
        let decls = parse_decls("def x : T := y").expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        let OxDecl::Definition { ty, val, .. } = d else { panic!("expected Definition") };
        let ty_val = ty.expect("ty must be Some");
        assert!(matches!(ty_val.value, OxExpr::Var(_)));
        assert!(matches!(val.value, OxExpr::Var(_)));
    }

    // ─── 13b-1 coverage: Theorem / Axiom / Import / Namespace ──

    #[test]
    fn theorem_with_ident_proof() {
        // `=` is a binary operator → still Unsupported at
        // the expression level (BinOp → App lowering lands
        // in a later 13b sub-step), so use an Ident type.
        let decls = parse_decls("theorem t : T := proof_term").expect("must parse");
        let oxdecl = translate_decl(&decls[0]).expect("13b-1 supports Theorem").value;
        match oxdecl {
            OxDecl::Theorem { name, ty, proof, where_clauses, .. } => {
                assert_eq!(name, "t");
                assert!(matches!(ty.value, OxExpr::Var(ref s) if s == "T"));
                assert!(matches!(proof.value, OxExpr::Var(ref s) if s == "proof_term"));
                assert!(where_clauses.is_empty());
            }
            other => panic!("expected Theorem, got {other:?}"),
        }
    }

    #[test]
    fn axiom_with_ident_type() {
        let decls = parse_decls("axiom em : T").expect("must parse");
        let oxdecl = translate_decl(&decls[0]).expect("13b-1 supports Axiom").value;
        match oxdecl {
            OxDecl::Axiom { name, ty, .. } => {
                assert_eq!(name, "em");
                assert!(matches!(ty.value, OxExpr::Var(ref s) if s == "T"));
            }
            other => panic!("expected Axiom, got {other:?}"),
        }
    }

    #[test]
    fn import_dotted_path_splits_on_dot() {
        let decls = parse_decls("import Foo.Bar.Baz").expect("must parse");
        let oxdecl = translate_decl(&decls[0]).expect("13b-1 supports Import").value;
        match oxdecl {
            OxDecl::Import { path } => {
                assert_eq!(path, vec!["Foo", "Bar", "Baz"]);
            }
            other => panic!("expected Import, got {other:?}"),
        }
    }

    #[test]
    fn namespace_with_one_inner_def() {
        let src = "namespace Foo\ndef x : T := y\nend Foo";
        let decls = parse_decls(src).expect("must parse");
        let oxdecl = translate_decl(&decls[0]).expect("13b-1 supports Namespace").value;
        match oxdecl {
            OxDecl::Namespace { name, decls: inner } => {
                assert_eq!(name, "Foo");
                assert_eq!(inner.len(), 1);
                assert!(matches!(inner[0].value, OxDecl::Definition { ref name, .. } if name == "x"));
            }
            other => panic!("expected Namespace, got {other:?}"),
        }
    }

    #[test]
    fn univ_params_propagate_into_definition() {
        let decls = parse_decls("def f.{u} : T := y").expect("must parse");
        let oxdecl = translate_decl(&decls[0]).expect("must translate").value;
        match oxdecl {
            OxDecl::Definition { univ_params, .. } => {
                assert_eq!(univ_params, vec!["u".to_string()]);
            }
            other => panic!("expected Definition, got {other:?}"),
        }
    }

    // ─── 13b-3: BinOp / UnaryOp → App lowering ─────────

    #[test]
    fn binop_plus_lowers_to_nested_app() {
        // OX7 typeclass step (2026-05-27): `a + b` now
        // lowers to `App(App(Var("HAdd.hAdd"), a), b)`
        // — `+` desugars to its Lean stdlib
        // typeclass-projection identifier here at
        // translate time, so oxilean-elab can resolve
        // the name against `leo4_env_bootstrap`'s
        // axiom set.
        let decls = parse_decls("def x : T := a + b").expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        let OxDecl::Definition { val, .. } = d else { panic!("expected Definition") };
        let OxExpr::App(f_lhs, rhs) = val.value else {
            panic!("expected outer App");
        };
        assert!(matches!(rhs.value, OxExpr::Var(ref s) if s == "b"));
        let OxExpr::App(f, lhs) = f_lhs.value else {
            panic!("expected inner App");
        };
        assert!(matches!(f.value, OxExpr::Var(ref s) if s == "HAdd.hAdd"));
        assert!(matches!(lhs.value, OxExpr::Var(ref s) if s == "a"));
    }

    #[test]
    fn binop_unicode_op_preserved() {
        // OX7 (2026-05-27): `≤` maps to `LE.le` (alias
        // of `<=`) in the typeclass-projection table.
        // Other Unicode ops (`×`, `÷`, `∪`, `∩`, `→`,
        // …) fall through verbatim until they're added
        // to `arith_op_to_tc_projection`.
        let decls = parse_decls("def x : T := a ≤ b").expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        let OxDecl::Definition { val, .. } = d else { panic!("expected Definition") };
        let OxExpr::App(f_lhs, _) = val.value else { panic!("expected App") };
        let OxExpr::App(f, _) = f_lhs.value else { panic!("expected App") };
        assert!(matches!(f.value, OxExpr::Var(ref s) if s == "LE.le"));
    }

    #[test]
    fn unary_op_lowers_to_app() {
        // OX7 typeclass step (2026-05-27): UnaryOp `-`
        // now desugars to `Neg.neg` (and `!` to
        // `Not.not`) before becoming an App. Matches
        // the BinOp arm's `arith_op_to_tc_projection`
        // treatment so the codegen typeclass-fold path
        // can later recognise the projection identifier.
        let decls = parse_decls("def x : T := -y").expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        let OxDecl::Definition { val, .. } = d else { panic!("expected Definition") };
        let OxExpr::App(f, x) = val.value else { panic!("expected App") };
        assert!(matches!(f.value, OxExpr::Var(ref s) if s == "Neg.neg"));
        assert!(matches!(x.value, OxExpr::Var(ref s) if s == "y"));
    }

    #[test]
    fn left_assoc_chain_nests_correctly() {
        // `a + b + c` parses as `(a + b) + c` →
        // App(App(+, App(App(+, a), b)), c).
        let decls = parse_decls("def x : T := a + b + c").expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        let OxDecl::Definition { val, .. } = d else { panic!("expected Definition") };
        // Outermost is App(App(+, …), c).
        let OxExpr::App(_, rhs_c) = &val.value else {
            panic!("expected outer App");
        };
        assert!(matches!(rhs_c.value, OxExpr::Var(ref s) if s == "c"));
    }

    // ─── 13b-4: Structure / Inductive / Class / Instance ───

    #[test]
    fn structure_basic_fields() {
        let src = "structure Point where\n  x : Nat\n  y : Nat";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("13b-4 supports Structure").value;
        match d {
            OxDecl::Structure { name, fields, .. } => {
                assert_eq!(name, "Point");
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].name, "x");
                assert_eq!(fields[1].name, "y");
                assert!(matches!(fields[0].ty.value, OxExpr::Var(ref s) if s == "Nat"));
            }
            other => panic!("expected Structure, got {other:?}"),
        }
    }

    #[test]
    fn structure_with_extends() {
        let src = "structure Point3D extends Point where\n  z : Nat";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        let OxDecl::Structure { extends, .. } = d else { panic!("expected Structure") };
        assert_eq!(extends, vec!["Point".to_string()]);
    }

    #[test]
    fn inductive_with_bare_ctors_synthesizes_self_typed_ctors() {
        let src = "inductive Color where\n  | red\n  | green\n  | blue";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("13b-4 supports Inductive").value;
        match d {
            OxDecl::Inductive { name, ctors, ty, .. } => {
                assert_eq!(name, "Color");
                assert_eq!(ctors.len(), 3);
                assert_eq!(ctors[0].name, "red");
                // Bare ctors synthesize ty as Var(inductive_name).
                assert!(matches!(ctors[0].ty.value, OxExpr::Var(ref s) if s == "Color"));
                // No explicit Sort annotation → defaults to Sort(Type).
                assert!(matches!(ty.value, OxExpr::Sort(SortKind::Type)));
            }
            other => panic!("expected Inductive, got {other:?}"),
        }
    }

    #[test]
    fn class_basic() {
        let src = "class Foo where\n  bar : Nat";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("13b-4 supports Class").value;
        match d {
            OxDecl::ClassDecl { name, fields, .. } => {
                assert_eq!(name, "Foo");
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].name, "bar");
            }
            other => panic!("expected ClassDecl, got {other:?}"),
        }
    }

    #[test]
    fn instance_where_form_extracts_class_name() {
        // `instance : Monad List where` — head of `Monad List`
        // is the class name `Monad`.
        let src = "instance : Monad List where\n  pure : a";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("13b-4 supports Instance").value;
        match d {
            OxDecl::InstanceDecl { name, class_name, defs, .. } => {
                assert!(name.is_none());
                assert_eq!(class_name, "Monad");
                assert_eq!(defs.len(), 1);
                assert_eq!(defs[0].0, "pure");
            }
            other => panic!("expected InstanceDecl, got {other:?}"),
        }
    }

    #[test]
    fn instance_named_form() {
        let src = "instance natOrd : Ord Nat where\n  compare : a";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        match d {
            OxDecl::InstanceDecl { name, class_name, .. } => {
                assert_eq!(name.as_deref(), Some("natOrd"));
                assert_eq!(class_name, "Ord");
            }
            other => panic!("expected InstanceDecl, got {other:?}"),
        }
    }

    // ─── 13b-5: Section / Variable / Open / attributes ─────

    #[test]
    fn section_with_inner_decl() {
        let src = "section Foo\ndef x : T := y\nend Foo";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("13b-5 supports Section").value;
        match d {
            OxDecl::SectionDecl { name, decls: inner } => {
                assert_eq!(name, "Foo");
                assert_eq!(inner.len(), 1);
                assert!(matches!(inner[0].value, OxDecl::Definition { ref name, .. } if name == "x"));
            }
            other => panic!("expected SectionDecl, got {other:?}"),
        }
    }

    #[test]
    fn anonymous_section_gets_empty_name() {
        let src = "section\ndef x : T := y\nend";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        match d {
            OxDecl::SectionDecl { name, .. } => assert_eq!(name, ""),
            other => panic!("expected SectionDecl, got {other:?}"),
        }
    }

    #[test]
    fn variable_translates_binders() {
        let src = "variable (a : Nat)";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("13b-5 supports Variable").value;
        match d {
            OxDecl::Variable { binders } => {
                assert_eq!(binders.len(), 1);
                assert_eq!(binders[0].name, "a");
            }
            other => panic!("expected Variable, got {other:?}"),
        }
    }

    #[test]
    fn open_single_module() {
        let src = "open Foo.Bar";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("13b-5 supports single-item Open").value;
        match d {
            OxDecl::Open { path, names } => {
                assert_eq!(path, vec!["Foo", "Bar"]);
                assert!(names.is_empty());
            }
            other => panic!("expected Open, got {other:?}"),
        }
    }

    #[test]
    fn open_multi_item_unsupported_in_13b5() {
        let src = "open Foo Bar Baz";
        let decls = parse_decls(src).expect("must parse");
        let err = translate_decl(&decls[0]).expect_err("multi-item Open needs caller to split");
        assert!(matches!(err, TranslateError::Unsupported(s) if s.contains("Open")));
    }

    #[test]
    fn attribute_simp_maps_to_typed_kind() {
        // OX7 typeclass step (2026-05-27): the translator
        // now also wraps attributed decls in an outer
        // `OxDecl::Attribute { attrs: Vec<String>, decl
        // }` (mirroring the legacy walker) so the
        // production attr-discovery path
        // (`decl_has_leo4_export`) finds the leo4_export
        // marker. The inner Definition still carries the
        // typed `OxAttr` list — both representations live
        // side-by-side.
        let src = "@[simp]\ndef x : T := y";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        let inner = match &d {
            OxDecl::Attribute { attrs, decl } => {
                assert_eq!(attrs, &vec!["simp".to_string()]);
                &decl.value
            }
            _ => panic!("expected outer Attribute wrapper, got {d:?}"),
        };
        match inner {
            OxDecl::Definition { attrs, .. } => {
                assert_eq!(attrs.len(), 1);
                assert_eq!(attrs[0], OxAttr::Simp);
            }
            other => panic!("expected inner Definition, got {other:?}"),
        }
    }

    #[test]
    fn attribute_unknown_falls_to_custom() {
        let src = "@[my_attr]\ndef x : T := y";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        let OxDecl::Attribute { attrs, decl } = d else {
            panic!("expected outer Attribute wrapper")
        };
        assert_eq!(attrs, vec!["my_attr".to_string()]);
        let OxDecl::Definition { attrs: typed, .. } = decl.value else {
            panic!("expected inner Definition")
        };
        assert_eq!(typed[0], OxAttr::Custom("my_attr".to_string()));
    }

    #[test]
    fn attribute_dropped_on_variants_without_attrs_field() {
        // OX7 typeclass step (2026-05-27): even for
        // variants whose inner shape doesn't carry an
        // `attrs` field (Inductive / Class / Instance /
        // Namespace / Section / Variable / Open / Mutual
        // / Import), the outer `OxDecl::Attribute`
        // wrapper still surfaces the attribute list so
        // downstream attr discovery works uniformly. The
        // *inner* variant is still attribute-free.
        let src = "@[inline]\ninductive Color where\n  | red";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        let OxDecl::Attribute { attrs, decl } = d else {
            panic!("expected outer Attribute wrapper")
        };
        assert_eq!(attrs, vec!["inline".to_string()]);
        match decl.value {
            OxDecl::Inductive { name, .. } => assert_eq!(name, "Color"),
            other => panic!("expected inner Inductive, got {other:?}"),
        }
    }

    #[test]
    fn dsl_decl_is_unsupported_by_design() {
        // DSL decls (notation, macro_rules, …) have no
        // oxilean-parse equivalent; the translator returns
        // Unsupported permanently for these.
        let decls = parse_decls(r#"infix:65 " + " => HAdd.hAdd"#).expect("must parse");
        let err = translate_decl(&decls[0]).expect_err("DSL must be Unsupported");
        assert!(matches!(err, TranslateError::Unsupported(s) if s.contains("DSL")));
    }
}
