//! OX5-oxi — elab env bootstrap for the rust-transpile
//! path (no lake/lean dependency).
//!
//! Replaces the historical `Environment::new()` (empty)
//! at every `transpile_source*` call site. The transpile
//! pipeline's elab step (`oxilean_elab::elaborate_decl`)
//! needs every name the source mentions to be resolvable;
//! against the empty env, even `def x : UInt64 := 0`
//! fails with `NameNotFound("UInt64")`.
//!
//! ## Two-layer strategy
//!
//! - **E1 (OxiLean's own builtin env)**:
//!   `oxilean_kernel::init_builtin_env` installs the
//!   OxiLean prelude — Bool, Unit, Empty, Nat, String,
//!   Eq, Prod, List, plus axioms (Quot, propext,
//!   Classical.choice, DecidableEq) and the corresponding
//!   `*.rec` / `*.ind` recursors / arithmetic ops
//!   (Nat.add, Nat.beq, …).
//! - **E2 (leo4-side augmentation)**:
//!   `add_leo4_primitives` adds boundary primitives leo4
//!   requires but OxiLean v0.1.2 doesn't ship by default
//!   — sized integers (UInt8/16/32/64/128, Int8/16/32/64
//!   /128), floats (Float32, Float64), Char. These come
//!   in as `Declaration::Axiom { ty: Sort 1, … }` — the
//!   elaborator just needs the names to resolve to *some*
//!   sort; the boundary marshalling layer encodes the
//!   actual wire format.
//!
//! ## Zero lake/lean overhead
//!
//! Both layers run in-process against `oxilean-kernel`
//! (cargo dep already pulled). No `lake` invocation, no
//! `.olean` file load, no Lean toolchain detection.
//! OxiLean-only users (`leo4.toml` with `kind =
//! "rust-transpile"` only) install nothing beyond
//! `leo4-oxilean-build`.

use oxilean_kernel::env::Environment;
use oxilean_kernel::{init_builtin_env, Declaration, Expr, Level, Name};

/// Build a fresh `Environment` populated with the
/// OxiLean prelude + leo4 boundary primitives, ready for
/// `transpile_source*` to use.
///
/// # Errors
///
/// Returns the underlying OxiLean error string if the
/// prelude or augmentation fails to install (e.g.
/// duplicate-name collision — should never happen with
/// the static name lists we use, but the error is
/// surfaced rather than panicked on).
pub fn bootstrap_env() -> Result<Environment, String> {
    let mut env = Environment::new();
    init_builtin_env(&mut env)?;
    add_leo4_primitives(&mut env)?;
    Ok(env)
}

/// Boundary primitives leo4 depends on that OxiLean v0.1.2
/// doesn't install via `init_builtin_env`. Each lands as
/// `Declaration::Axiom { ty: Sort 1, … }` — the
/// elaborator only needs the *name* to resolve; the
/// marshalling layer encodes the actual wire format
/// (`crates/leo4-abi/src/marshal/primitive_types.rs`).
///
/// Kept as a separate fn so unit tests can verify the
/// exact set + downstream consumers can call it on a
/// custom env if they need partial bootstrapping.
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
    // OX7 typeclass step (2026-05-27): register
    // arithmetic-typeclass *projection names* as axioms
    // so oxilean-elab's `+`/`-`/`*`/`/`/`%` → `HAdd.hAdd`
    // (etc.) parser desugar resolves without
    // NameNotFound. The actual instance dispatch is
    // handled in the OxiLean fork's `to_lcnf` Const
    // matchers — when codegen sees
    // `Const("HAdd.hAdd") a b`, it emits the matching
    // native BinOp directly. So these axioms exist
    // only to give the parser-desugared identifier
    // somewhere to resolve.
    //
    // Type signature is the most permissive form that
    // still satisfies elab's identifier lookup:
    // `Sort 1` (i.e. `Type`). Treating each as a leaf
    // axiom avoids structure / instance-resolution
    // machinery — we never call them at runtime, we
    // pattern-match on them at codegen.
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
    // OX7 InterpStr step (2026-05-28): register identifiers
    // the `s!"…{x}…"` lowering emits but OxiLean's
    // `init_builtin_env` doesn't ship. `String.append` is
    // already present from the kernel's builtin install (see
    // oxilean-kernel/src/builtin/functions.rs), so we only
    // need `toString` here. Treating it as a leaf axiom
    // (Sort 1) avoids re-deriving the `ToString` typeclass
    // — the same model as ARITHMETIC_TC_PROJECTIONS: the
    // name exists for elab's identifier-resolution pass,
    // codegen pattern-matches on it at LCNF time if it ever
    // needs to emit a real conversion.
    for name in STRING_INTERP_AXIOMS {
        // Defensively skip names that are already present —
        // `String.append` slots through OxiLean's builtin
        // install and would otherwise trip
        // DuplicateDeclaration here.
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

/// OX7 InterpStr step (2026-05-28): identifiers the
/// `L4Expr::InterpStr` → `String.append`/`toString`
/// desugar in `leo4_translate` emits. Installed as leaf
/// axioms so the parser-desugared identifiers resolve at
/// elab time without `NameNotFound`. Mirrors the
/// ARITHMETIC_TC_PROJECTIONS pattern.
///
/// Names already shipped by OxiLean's `init_builtin_env`
/// (currently `String.append`) are listed here for
/// completeness + future-proofing; the install loop
/// skips any name that's already present to avoid
/// DuplicateDeclaration.
pub const STRING_INTERP_AXIOMS: &[&str] = &[
    "String.append",
    "toString",
];

/// OX7 typeclass step (2026-05-27): the projection
/// names oxilean-parse desugars infix arithmetic /
/// comparison operators to. Each lands as a leaf
/// axiom so elab's `NameNotFound("+")` (and friends)
/// goes away; the fork's `to_lcnf` Const matchers
/// pattern-match on them at codegen time. Mirrors
/// the notation table in oxilean-parse-0.1.2's
/// `ast/functions.rs`.
pub const ARITHMETIC_TC_PROJECTIONS: &[&str] = &[
    // Arithmetic.
    "HAdd.hAdd",
    "HSub.hSub",
    "HMul.hMul",
    "HDiv.hDiv",
    "HMod.hMod",
    "HPow.hPow",
    // Bitwise.
    "HAnd.hAnd",
    "HOr.hOr",
    "HXor.hXor",
    "HShiftLeft.hShiftLeft",
    "HShiftRight.hShiftRight",
    // Comparison (lt / le land via these projections
    // too in oxilean-parse's notation table).
    "LT.lt",
    "LE.le",
    "BEq.beq",
    // OX7 (2026-05-27): propositional equality `a = b`.
    "Eq.eq",
    // Unary.
    "Neg.neg",
    "Not.not",
];

/// The static list of leo4 boundary-required type names
/// missing from OxiLean's default prelude. Mirrors
/// `leo4-abi`'s canonical primitive set
/// (`crates/leo4-abi/src/marshal/primitive_types.rs`).
///
/// **Not included** (covered by OxiLean's
/// `init_builtin_env`): `Bool`, `Unit`, `Nat`, `String`.
///
/// **Nightly-floats variants** (`Float16`, `BFloat16`,
/// `Float128`, `Complex*16x2`, `Complex*128x2`) are NOT
/// in this list — OxiLean has no use for them and
/// `synthesize_canonical_wrapper`'s nightly-feature path
/// is handled separately at the Rust-codegen layer.
pub const LEO4_PRIMITIVE_TYPES: &[&str] = &[
    // Unsigned integers.
    "UInt8",
    "UInt16",
    "UInt32",
    "UInt64",
    "UInt128",
    // Signed integers.
    "Int8",
    "Int16",
    "Int32",
    "Int64",
    "Int128",
    // Floats (stable subset; nightly variants are not
    // exposed to the elaborator at all).
    "Float32",
    "Float64",
    // Char.
    "Char",
];


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_env_installs_oxilean_prelude() {
        let env = bootstrap_env().expect("bootstrap must succeed");
        assert!(env.contains(&Name::str("Bool")), "Bool from prelude");
        assert!(env.contains(&Name::str("Nat")), "Nat from prelude");
        assert!(env.contains(&Name::str("Nat.add")), "Nat.add from prelude");
        assert!(env.contains(&Name::str("String")), "String from prelude");
        assert!(env.contains(&Name::str("List")), "List from prelude");
    }

    #[test]
    fn bootstrap_env_installs_leo4_primitives() {
        let env = bootstrap_env().expect("bootstrap must succeed");
        for prim in LEO4_PRIMITIVE_TYPES {
            assert!(
                env.contains(&Name::str(*prim)),
                "leo4 primitive `{prim}` must be installed"
            );
        }
    }

    #[test]
    fn bootstrap_env_includes_arithmetic_tc_projections() {
        let env = bootstrap_env().expect("bootstrap_env");
        for proj in ARITHMETIC_TC_PROJECTIONS {
            assert!(
                env.contains(&Name::from_str(proj)),
                "arithmetic-tc projection axiom `{proj}` must be installed"
            );
        }
    }

    #[test]
    fn bootstrap_env_includes_string_interp_axioms() {
        let env = bootstrap_env().expect("bootstrap_env");
        for ax in STRING_INTERP_AXIOMS {
            assert!(
                env.contains(&Name::from_str(ax)),
                "string-interp axiom `{ax}` must be installed"
            );
        }
    }

    #[test]
    fn add_leo4_primitives_alone_works_on_fresh_env() {
        // Caller may want to bootstrap a partial env (e.g.
        // OxiLean prelude already installed by an
        // embedder); `add_leo4_primitives` MUST be safe to
        // call on a fresh env too.
        let mut env = Environment::new();
        add_leo4_primitives(&mut env).expect("add_leo4_primitives on fresh env");
        assert!(env.contains(&Name::str("UInt64")));
        assert!(env.contains(&Name::str("Float64")));
        assert!(env.contains(&Name::from_str("HAdd.hAdd")));
    }

    #[test]
    fn leo4_primitive_list_covers_all_sized_ints() {
        for ty in [
            "UInt8", "UInt16", "UInt32", "UInt64", "UInt128",
            "Int8", "Int16", "Int32", "Int64", "Int128",
            "Float32", "Float64", "Char",
        ] {
            assert!(
                LEO4_PRIMITIVE_TYPES.contains(&ty),
                "LEO4_PRIMITIVE_TYPES must list `{ty}`"
            );
        }
    }
}
