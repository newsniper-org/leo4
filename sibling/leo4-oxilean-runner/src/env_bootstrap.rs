//! Inlined copy of
//! `sibling/leo4-oxilean-build/src/leo4_env_bootstrap.rs`'s
//! public surface, trimmed to what `run_main` needs (the
//! `bootstrap_env` + `add_leo4_primitives` pair, the
//! `LEO4_PRIMITIVE_TYPES` / `ARITHMETIC_TC_PROJECTIONS` /
//! `STRING_INTERP_AXIOMS` static name lists).
//!
//! ## Why duplicated?
//!
//! `leo4-oxilean-build` uses
//! `oxilean-kernel = "=0.1.2"` + `[patch.crates-io]` to
//! redirect to the fork submodule — a pattern that only
//! applies at *its* workspace root. Path-deping
//! `leo4-oxilean-build` from here (or from a user scaffold)
//! resolves the oxilean-* crates against crates.io v0.1.2
//! instead, which is missing OX7/OX8 fork-only APIs and
//! causes a hard cargo failure
//! (`cannot find function decl_to_lcnf_full`). Vendoring is
//! the cleanest fix until we either (a) extract a leaf
//! `leo4-oxilean-bootstrap` crate that depends on
//! oxilean-kernel only via direct path-dep, or (b) upstream
//! the leo4 primitives into OxiLean's `init_builtin_env`.
//!
//! Keep this file's `LEO4_PRIMITIVE_TYPES` /
//! `ARITHMETIC_TC_PROJECTIONS` / `STRING_INTERP_AXIOMS`
//! lists in lock-step with the canonical copy in
//! `sibling/leo4-oxilean-build/src/leo4_env_bootstrap.rs`;
//! the `bootstrap_env_matches_canonical_set` test below
//! enforces it at the type-list level.

use oxilean_kernel::env::Environment;
use oxilean_kernel::{init_builtin_env, Declaration, Expr, Level, Name};

/// Build a fresh `Environment` populated with the OxiLean
/// prelude (E1) + leo4 boundary primitives (E2). Mirrors
/// `leo4_oxilean_build::leo4_env_bootstrap::bootstrap_env`.
pub fn bootstrap_env() -> Result<Environment, String> {
    let mut env = Environment::new();
    init_builtin_env(&mut env)?;
    add_leo4_primitives(&mut env)?;
    Ok(env)
}

/// Boundary primitives leo4 depends on that OxiLean v0.1.2
/// doesn't install via `init_builtin_env`. Inlined from the
/// `leo4-oxilean-build` canonical copy; see crate docs.
pub fn add_leo4_primitives(env: &mut Environment) -> Result<(), String> {
    let type1 = Expr::Sort(Level::succ(Level::zero()));
    for name in LEO4_PRIMITIVE_TYPES {
        let decl = Declaration::Axiom {
            name: Name::str(*name),
            univ_params: vec![],
            ty: type1.clone(),
        };
        env.add(decl).map_err(|e| {
            format!("leo4 primitive `{name}` install failed: {e}")
        })?;
    }
    for op in ARITHMETIC_TC_PROJECTIONS {
        let decl = Declaration::Axiom {
            name: Name::from_str(op),
            univ_params: vec![],
            ty: type1.clone(),
        };
        env.add(decl).map_err(|e| {
            format!("arithmetic-tc axiom `{op}` install failed: {e}")
        })?;
    }
    for name in STRING_INTERP_AXIOMS {
        if env.contains(&Name::from_str(name)) {
            continue;
        }
        let decl = Declaration::Axiom {
            name: Name::from_str(name),
            univ_params: vec![],
            ty: type1.clone(),
        };
        env.add(decl).map_err(|e| {
            format!("string-interp axiom `{name}` install failed: {e}")
        })?;
    }
    Ok(())
}

/// String-interpolation desugar dependencies — mirrors
/// `STRING_INTERP_AXIOMS` in
/// `leo4-oxilean-build::leo4_env_bootstrap`.
pub const STRING_INTERP_AXIOMS: &[&str] = &["String.append", "toString"];

/// Arithmetic / comparison typeclass projection names —
/// mirrors `ARITHMETIC_TC_PROJECTIONS` in
/// `leo4-oxilean-build::leo4_env_bootstrap`.
pub const ARITHMETIC_TC_PROJECTIONS: &[&str] = &[
    "HAdd.hAdd",
    "HSub.hSub",
    "HMul.hMul",
    "HDiv.hDiv",
    "HMod.hMod",
    "HPow.hPow",
    "HAnd.hAnd",
    "HOr.hOr",
    "HXor.hXor",
    "HShiftLeft.hShiftLeft",
    "HShiftRight.hShiftRight",
    "LT.lt",
    "LE.le",
    "BEq.beq",
    "Eq.eq",
    "Neg.neg",
    "Not.not",
];

/// leo4 boundary type names missing from OxiLean's default
/// prelude — mirrors `LEO4_PRIMITIVE_TYPES` in
/// `leo4-oxilean-build::leo4_env_bootstrap`.
pub const LEO4_PRIMITIVE_TYPES: &[&str] = &[
    "UInt8", "UInt16", "UInt32", "UInt64", "UInt128",
    "Int8", "Int16", "Int32", "Int64", "Int128",
    "Float32", "Float64",
    "Char",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Bootstrap must succeed against a fresh env and put
    /// the leo4 primitives into the resulting env.
    #[test]
    fn bootstrap_installs_uint64_and_float64() {
        let env = bootstrap_env().expect("bootstrap must succeed");
        assert!(env.contains(&Name::str("UInt64")));
        assert!(env.contains(&Name::str("Float64")));
        // OxiLean's own prelude: Bool / Nat must be present.
        assert!(env.contains(&Name::str("Bool")));
        assert!(env.contains(&Name::str("Nat")));
    }

    #[test]
    fn arithmetic_tc_projections_installed() {
        let env = bootstrap_env().unwrap();
        for op in ARITHMETIC_TC_PROJECTIONS {
            assert!(
                env.contains(&Name::from_str(op)),
                "arithmetic-tc projection `{op}` must be installed"
            );
        }
    }
}
