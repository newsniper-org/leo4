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
    let value = translate_decl_kind(&d.kind, &d.univ_params)?;
    Ok(Located::new(value, dummy_span()))
}

fn translate_decl_kind(
    k: &L4Kind,
    univ_params: &[String],
) -> Result<OxDecl, TranslateError> {
    match k {
        L4Kind::Definition { name, binders, ty, value } => {
            if !binders.is_empty() {
                return Err(TranslateError::Unsupported(
                    "Definition binders (lands in 13b-2)",
                ));
            }
            let ty = match ty {
                Some(t) => Some(translate_expr_located(t)?),
                None => None,
            };
            let val = translate_expr_located(value)?;
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
            if !binders.is_empty() {
                return Err(TranslateError::Unsupported(
                    "Theorem binders (lands in 13b-2)",
                ));
            }
            Ok(OxDecl::Theorem {
                name: name.clone(),
                univ_params: univ_params.to_vec(),
                ty: translate_expr_located(ty)?,
                proof: translate_expr_located(proof)?,
                where_clauses: Vec::new(),
                attrs: Vec::new(),
            })
        }
        L4Kind::Axiom { name, binders, ty } => {
            if !binders.is_empty() {
                return Err(TranslateError::Unsupported(
                    "Axiom binders (lands in 13b-2)",
                ));
            }
            Ok(OxDecl::Axiom {
                name: name.clone(),
                univ_params: univ_params.to_vec(),
                ty: translate_expr_located(ty)?,
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
        L4Kind::Example { .. } => Err(TranslateError::Unsupported("Example (lands in 13b-2)")),
        L4Kind::Structure { .. } => Err(TranslateError::Unsupported("Structure (lands in 13b-4)")),
        L4Kind::Class { .. } => Err(TranslateError::Unsupported("Class (lands in 13b-4)")),
        L4Kind::Inductive { .. } => Err(TranslateError::Unsupported("Inductive (lands in 13b-4)")),
        L4Kind::Instance { .. } => Err(TranslateError::Unsupported("Instance (lands in 13b-4)")),
        L4Kind::Section { .. } => Err(TranslateError::Unsupported("Section (lands in 13b-5)")),
        L4Kind::Mutual { .. } => Err(TranslateError::Unsupported("Mutual (deferred)")),
        L4Kind::Open { .. } => Err(TranslateError::Unsupported("Open (lands in 13b-5)")),
        L4Kind::Variable { .. } => Err(TranslateError::Unsupported("Variable (lands in 13b-5)")),
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

    #[test]
    fn theorem_with_binders_unsupported_in_13b1() {
        let decls = parse_decls("theorem t (h : T) : U := h").expect("must parse");
        let err = translate_decl(&decls[0]).expect_err("binders defer to 13b-2");
        assert!(matches!(err, TranslateError::Unsupported(s) if s.contains("binders")));
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
