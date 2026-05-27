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
        // OxiLean prelude: spot-check a few names from
        // `init_builtin_env`'s install list.
        assert!(env.contains(&Name::str("Bool")), "Bool from prelude");
        assert!(env.contains(&Name::str("Nat")), "Nat from prelude");
        assert!(env.contains(&Name::str("Nat.add")), "Nat.add from prelude");
        assert!(env.contains(&Name::str("String")), "String from prelude");
        assert!(env.contains(&Name::str("List")), "List from prelude");
    }

    #[test]
    fn bootstrap_env_installs_leo4_primitives() {
        let env = bootstrap_env().expect("bootstrap must succeed");
        for name in LEO4_PRIMITIVE_TYPES {
            assert!(
                env.contains(&Name::str(*name)),
                "leo4 primitive `{name}` must be installed"
            );
        }
    }

    #[test]
    fn bootstrap_env_uint64_and_int64_distinct_axioms() {
        let env = bootstrap_env().unwrap();
        // Each landed as a distinct axiom — not aliased.
        let uint = env.find(&Name::str("UInt64")).expect("UInt64 present");
        let int = env.find(&Name::str("Int64")).expect("Int64 present");
        assert_ne!(uint.name(), int.name());
    }

    #[test]
    fn bootstrap_env_idempotent_within_single_call() {
        // Two consecutive bootstraps must each succeed (no
        // shared mutable global state).
        bootstrap_env().expect("first call");
        bootstrap_env().expect("second call");
    }

    #[test]
    fn add_leo4_primitives_alone_works_on_fresh_env() {
        // Caller may want to bring their own OxiLean env
        // (e.g. for testing) and just add leo4's slice.
        let mut env = Environment::new();
        add_leo4_primitives(&mut env).expect("must install");
        assert!(env.contains(&Name::str("UInt64")));
        // Without init_builtin_env, Bool is NOT present.
        assert!(!env.contains(&Name::str("Bool")));
    }

    #[test]
    fn leo4_primitive_list_covers_all_sized_ints() {
        let names: std::collections::HashSet<&str> =
            LEO4_PRIMITIVE_TYPES.iter().copied().collect();
        for width in [8, 16, 32, 64, 128] {
            assert!(
                names.contains(format!("UInt{width}").as_str()),
                "UInt{width} must be in the primitive list"
            );
            assert!(
                names.contains(format!("Int{width}").as_str()),
                "Int{width} must be in the primitive list"
            );
        }
    }

    // ─── OX5-oxi step 2: end-to-end via transpile_source_to_unit ──

    #[test]
    fn bootstrap_env_resolves_uint64_in_def_signature() {
        // The minimum fixture proving the augmentation
        // works end-to-end: a `def` whose signature
        // mentions `UInt64`. Body just returns the bound
        // param so no `OfNat` / arithmetic-typeclass
        // resolution is needed — purely the `UInt64`
        // identifier resolution that the empty env
        // historically failed on.
        let env = bootstrap_env().expect("env bootstrap");
        let mut registry = crate::Leo4ExportRegistry::new();
        let src = "@[leo4_export]\ndef ident_u64 (x : UInt64) : UInt64 := x";

        let result = crate::transpile_source_to_unit(
            &env,
            &mut registry,
            src,
            "_test_ident_u64_mangled",
        );

        // The transpile may still fail downstream (LCNF
        // / codegen) on `UInt64` if the marshalling
        // layer disagrees, but the elab step MUST NOT
        // surface `NameNotFound("UInt64")` anymore — that
        // is OX5-oxi's contract. Any other diagnostic is
        // a separate concern.
        if let Err(e) = &result {
            assert!(
                !e.message.contains("NameNotFound") || !e.message.contains("UInt64"),
                "OX5-oxi regression: UInt64 must resolve under the bootstrapped env. \
                 Diagnostic was: {e:?}"
            );
        }
    }

    #[test]
    fn bootstrap_env_resolves_nat_in_def_signature() {
        // Same shape, but with Nat (which OxiLean's own
        // `init_builtin_env` already installs). This
        // exercises the E1 layer in isolation.
        let env = bootstrap_env().expect("env bootstrap");
        let mut registry = crate::Leo4ExportRegistry::new();
        let src = "@[leo4_export]\ndef ident_nat (x : Nat) : Nat := x";

        let result = crate::transpile_source_to_unit(
            &env,
            &mut registry,
            src,
            "_test_ident_nat_mangled",
        );
        if let Err(e) = &result {
            assert!(
                !e.message.contains("NameNotFound") || !e.message.contains("Nat"),
                "Nat must resolve under OxiLean's own init_builtin_env. \
                 Diagnostic was: {e:?}"
            );
        }
    }

    #[test]
    fn empty_env_would_have_failed_on_uint64() {
        // Sanity / regression check: with a completely
        // empty env (no bootstrap), elab *should* report
        // NameNotFound on UInt64. If this test stops
        // failing without bootstrap, OxiLean upstream
        // started shipping UInt64 itself — at which point
        // the leo4 augmentation list can be trimmed.
        let env = oxilean_kernel::env::Environment::new();
        let mut registry = crate::Leo4ExportRegistry::new();
        let src = "@[leo4_export]\ndef regress_u64 (x : UInt64) : UInt64 := x";
        let result = crate::transpile_source_to_unit(
            &env,
            &mut registry,
            src,
            "_test_regress_u64",
        );
        // Don't assert on the *exact* error shape — the
        // pipeline has several layers any of which may
        // catch a missing name first. Just confirm the
        // empty-env path doesn't silently succeed.
        assert!(
            result.is_err(),
            "regression: empty env should NOT successfully \
             transpile a UInt64-typed def. If OxiLean now \
             ships UInt64, trim LEO4_PRIMITIVE_TYPES + this \
             test."
        );
    }

    #[test]
    fn bootstrap_env_installs_string_interp_axioms() {
        // OX7 InterpStr step (2026-05-28): `toString` must
        // land in the bootstrapped env so the
        // `s!"…{x}…"` desugar in `leo4_translate`
        // (`InterpStr` arm) resolves at elab time.
        // `String.append` was already shipped by
        // OxiLean's `init_builtin_env`; the install loop
        // skips it but the post-condition still holds.
        let env = bootstrap_env().expect("bootstrap must succeed");
        for name in STRING_INTERP_AXIOMS {
            assert!(
                env.contains(&Name::str(*name)),
                "string-interp axiom `{name}` must be installed"
            );
        }
    }

    #[test]
    fn no_overlap_between_leo4_primitives_and_oxilean_builtins() {
        // Sanity: leo4's augmentation list MUST NOT
        // contain any name OxiLean already installs, else
        // bootstrap_env would fail with DuplicateDeclaration.
        let oxi_names: std::collections::HashSet<&str> =
            oxilean_kernel::builtin::all_builtin_names()
                .into_iter()
                .collect();
        for leo4_name in LEO4_PRIMITIVE_TYPES {
            assert!(
                !oxi_names.contains(leo4_name),
                "`{leo4_name}` is in both OxiLean builtins and leo4 augmentation"
            );
        }
    }
}
