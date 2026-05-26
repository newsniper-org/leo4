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
    Constructor as OxCtor, Decl as OxDecl, FieldDecl as OxField, Literal as OxLit,
    Located, SortKind, SurfaceExpr as OxExpr,
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
    let value = translate_decl_kind(&d.kind, &d.univ_params)?;
    // Attach the leo4 attributes to the translated decl
    // *if* the decl variant has an `attrs` field. Variants
    // that don't (Inductive / ClassDecl / InstanceDecl /
    // Namespace / SectionDecl / Variable / Open / Mutual /
    // Import) silently drop attrs — oxilean's elab path
    // re-discovers them through a separate
    // `Decl::Attribute` wrapping shape that we don't emit
    // here. Wrapping in an outer `Decl::Attribute` is a
    // possible follow-up if downstream needs it.
    let value = attach_attrs(value, &d.attrs);
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
            // Lower `BinOp("+", lhs, rhs)` to the
            // application tree `App(App(Var("+"), lhs),
            // rhs)`. The operator surfaces as a `Var` —
            // oxilean's elaborator resolves `+` against
            // its `HAdd.hAdd` typeclass entry (the same
            // path it takes for explicit-form sources).
            // For Unicode operators (≤, ≥, ≠, ×, ÷, ∈,
            // ∉, ∪, ∩, ⊆) the op symbol is passed
            // through verbatim; oxilean handles dispatch
            // identically to ASCII.
            let f = Located::new(OxExpr::Var(op.clone()), dummy_span());
            let lhs = translate_expr_located(lhs)?;
            let rhs = translate_expr_located(rhs)?;
            let f_lhs = Located::new(
                OxExpr::App(Box::new(f), Box::new(lhs)),
                dummy_span(),
            );
            Ok(OxExpr::App(Box::new(f_lhs), Box::new(rhs)))
        }
        L4Expr::UnaryOp(op, x) => {
            let f = Located::new(OxExpr::Var(op.clone()), dummy_span());
            let x = translate_expr_located(x)?;
            Ok(OxExpr::App(Box::new(f), Box::new(x)))
        }
        _ => Err(TranslateError::Unsupported("Expr variant (lands in 13b)")),
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
        // `a + b` should become `App(App(Var("+"), a), b)`.
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
        assert!(matches!(f.value, OxExpr::Var(ref s) if s == "+"));
        assert!(matches!(lhs.value, OxExpr::Var(ref s) if s == "a"));
    }

    #[test]
    fn binop_unicode_op_preserved() {
        let decls = parse_decls("def x : T := a ≤ b").expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        let OxDecl::Definition { val, .. } = d else { panic!("expected Definition") };
        let OxExpr::App(f_lhs, _) = val.value else { panic!("expected App") };
        let OxExpr::App(f, _) = f_lhs.value else { panic!("expected App") };
        assert!(matches!(f.value, OxExpr::Var(ref s) if s == "≤"));
    }

    #[test]
    fn unary_op_lowers_to_app() {
        let decls = parse_decls("def x : T := -y").expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        let OxDecl::Definition { val, .. } = d else { panic!("expected Definition") };
        let OxExpr::App(f, x) = val.value else { panic!("expected App") };
        assert!(matches!(f.value, OxExpr::Var(ref s) if s == "-"));
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
        let src = "@[simp]\ndef x : T := y";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        match d {
            OxDecl::Definition { attrs, .. } => {
                assert_eq!(attrs.len(), 1);
                assert_eq!(attrs[0], OxAttr::Simp);
            }
            other => panic!("expected Definition, got {other:?}"),
        }
    }

    #[test]
    fn attribute_unknown_falls_to_custom() {
        let src = "@[my_attr]\ndef x : T := y";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        let OxDecl::Definition { attrs, .. } = d else { panic!("expected Definition") };
        assert_eq!(attrs[0], OxAttr::Custom("my_attr".to_string()));
    }

    #[test]
    fn attribute_dropped_on_variants_without_attrs_field() {
        // Inductive has no `attrs` field on oxilean. The
        // translator drops them silently (no panic, no
        // error) — `Decl::Attribute` wrapping is a
        // possible follow-up if downstream needs.
        let src = "@[inline]\ninductive Color where\n  | red";
        let decls = parse_decls(src).expect("must parse");
        let d = translate_decl(&decls[0]).expect("must translate").value;
        match d {
            OxDecl::Inductive { name, .. } => assert_eq!(name, "Color"),
            other => panic!("expected Inductive, got {other:?}"),
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
