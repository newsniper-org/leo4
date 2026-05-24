//! OX6 step 13 — translator from `leo4_lean4_parse`'s
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
//!   `App`-tree synthesis from `leo4_lean4_parse::Expr::App`).
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
//! leo4-lean4-parse AST directly, not via the translation
//! shim.

use leo4_lean4_parse::{
    Decl as L4Decl, DeclKind as L4Kind, Expr as L4Expr,
    Literal as L4Lit,
};
use oxilean_parse::{
    Decl as OxDecl, Literal as OxLit, Located, SurfaceExpr as OxExpr,
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

/// Translate a `leo4_lean4_parse::Decl` into a
/// `Located<oxilean_parse::Decl>` ready for the existing
/// elab pipeline. Span info is `dummy_span()` —
/// leo4-lean4-parse doesn't carry source spans in its AST
/// today; recovering them is future work tracked under
/// the broader "diagnostic quality" non-RC item.
pub fn translate_decl(d: &L4Decl) -> Result<Located<OxDecl>, TranslateError> {
    let value = translate_decl_kind(&d.kind)?;
    Ok(Located::new(value, dummy_span()))
}

fn translate_decl_kind(k: &L4Kind) -> Result<OxDecl, TranslateError> {
    match k {
        L4Kind::Definition { name, binders, ty, value } => {
            if !binders.is_empty() {
                return Err(TranslateError::Unsupported(
                    "Definition binders (lands in 13b)",
                ));
            }
            let ty = match ty {
                Some(t) => Some(Box::new(translate_expr_located(t)?)),
                None => None,
            };
            let val = Box::new(translate_expr_located(value)?);
            Ok(OxDecl::Definition {
                name: name.clone(),
                univ_params: Vec::new(),
                ty: ty.map(|b| *b),
                val: *val,
                where_clauses: Vec::new(),
                attrs: Vec::new(),
            })
        }
        L4Kind::DefinitionByArms { .. } => {
            Err(TranslateError::Unsupported("DefinitionByArms (no oxilean equivalent)"))
        }
        L4Kind::Theorem { .. } => Err(TranslateError::Unsupported("Theorem (lands in 13b)")),
        L4Kind::Axiom { .. } => Err(TranslateError::Unsupported("Axiom (lands in 13b)")),
        L4Kind::Example { .. } => Err(TranslateError::Unsupported("Example (lands in 13b)")),
        L4Kind::Structure { .. } => Err(TranslateError::Unsupported("Structure (lands in 13b)")),
        L4Kind::Class { .. } => Err(TranslateError::Unsupported("Class (lands in 13b)")),
        L4Kind::Inductive { .. } => Err(TranslateError::Unsupported("Inductive (lands in 13b)")),
        L4Kind::Instance { .. } => Err(TranslateError::Unsupported("Instance (lands in 13b)")),
        L4Kind::Namespace { .. } => Err(TranslateError::Unsupported("Namespace (lands in 13b)")),
        L4Kind::Section { .. } => Err(TranslateError::Unsupported("Section (lands in 13b)")),
        L4Kind::Mutual { .. } => Err(TranslateError::Unsupported("Mutual (lands in 13b)")),
        L4Kind::Open { .. } => Err(TranslateError::Unsupported("Open (lands in 13b)")),
        L4Kind::Import { .. } => Err(TranslateError::Unsupported("Import (lands in 13b)")),
        L4Kind::Variable { .. } => Err(TranslateError::Unsupported("Variable (lands in 13b)")),
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

fn translate_expr(e: &L4Expr) -> Result<OxExpr, TranslateError> {
    match e {
        L4Expr::Ident(s) => Ok(OxExpr::Var(s.clone())),
        L4Expr::Lit(L4Lit::Nat(n)) => Ok(OxExpr::Lit(OxLit::Nat(*n))),
        L4Expr::Lit(L4Lit::Str(s)) => Ok(OxExpr::Lit(OxLit::String(s.clone()))),
        L4Expr::Lit(L4Lit::Float(s)) => {
            // leo4-lean4-parse holds floats as the raw source
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
        L4Expr::BinOp(..) => {
            Err(TranslateError::Unsupported("BinOp (lowering to App lands in 13b)"))
        }
        _ => Err(TranslateError::Unsupported("Expr variant (lands in 13b)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use leo4_lean4_parse::parse_decls;

    /// Helper: parse one decl and translate it. Returns
    /// the inner `OxDecl` (panics on parse/translate err so
    /// failure shows the diagnostic).
    fn parse_and_translate(src: &str) -> OxDecl {
        let decls = parse_decls(src).expect("leo4-lean4-parse must accept");
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

    #[test]
    fn definition_with_binders_is_unsupported_in_13a() {
        let decls = parse_decls("def add (a : Nat) (b : Nat) : Nat := a")
            .expect("must parse");
        let err = translate_decl(&decls[0]).expect_err("13a does not handle binders");
        assert!(matches!(
            err,
            TranslateError::Unsupported(s) if s.contains("binder")
        ));
    }

    #[test]
    fn theorem_is_unsupported_in_13a() {
        let decls = parse_decls("theorem refl : a = a := rfl").expect("must parse");
        let err = translate_decl(&decls[0]).expect_err("13a does not handle Theorem");
        assert!(matches!(err, TranslateError::Unsupported(s) if s.contains("Theorem")));
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
