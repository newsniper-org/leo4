# OX7 (1b) — elab-side bloat: diagnosis

Opened: 2026-05-27. **Status (2026-05-31): RESOLVED — OX7
closed**. Fix 1b-α landed leo4-side in `unfold_decl`
(re-substitution path, kept leo4-side per the
`docs/cool-japan-upstream-pr-draft.md` §3.2 note for a
follow-up upstream PR once the substitution algorithm
stabilises). Fix 1b-β landed on the fork as commit
`81a0fdc` (`Proj("add", _, Const("UInt64"))` →
`Const("UInt64.add")`). Diagnosis below is preserved as
the historical record.

## Symptom

`def add (a b : UInt64) : UInt64 := UInt64.add a b` transpiled via
`leo4 run --impl rust-transpile` (post-OX7 (1a + #1 + #2)) emits:

```rust
pub fn addU64(_x0: u64, _x1: u64) -> u64 {
    _x4(_x5, _x6)   // expected: Nat_add(_x0, _x1) or UInt64_add(_x0, _x1)
}
```

The signature is right (#1 + #2 working) but the body is corrupt —
`_x4`, `_x5`, `_x6` are placeholder identifiers for `LcnfVarId`s
that don't exist in the function. Raw-kernel `decl_to_lcnf` of the
same shape produces `Nat_add(_x0, _x1)` (the OX7 (1a) spike), so
the corruption is introduced by elab between parser output and the
input `decl_to_lcnf` actually sees.

## Diagnosis — what elab does

`examples/ox7_1b_dump.rs` runs the fixture through
`oxilean_elab::elab_decl::elaborate_decl` (using
`leo4_env_bootstrap::bootstrap_env`) and prints the resulting
`PendingDecl::Definition.{ty, val}`. The val for the fixture is:

```text
Lam("a", FVar(1000000),
  Lam("b", FVar(1000001),
    App(
      App(
        Proj("add", 0, Const("UInt64")),       ← (1) namespace lowering
        FVar(0)),                              ← (2) BVar → FVar
      FVar(1))))
```

(The Lam **binder type** field carries `FVar(1000000)` /
`FVar(1000001)` — large IDs that look like distinct metavars
introduced by elab. The **body** references the parameters via
`FVar(0)` / `FVar(1)` — small IDs starting at 0.)

Two transformations are at play that the raw-kernel `decl_to_lcnf`
never sees:

### Issue 1b-α: BVar → FVar substitution

Elab opens binders by substituting de Bruijn references with fresh
free-variable IDs. After unfolding the outer Lams in
`unfold_decl`, the body still references the parameters as
`FVar(0)` / `FVar(1)` rather than `BVar(1)` / `BVar(0)`.

The current `to_lcnf::convert_to_atomic` matches `Expr::FVar(fid)`
by calling `convert_fvar`, which **allocates a fresh
`LcnfVarId`** and registers a synthetic `fv_<N>` name in the
`name_map`. That fresh ID is unrelated to the param's
`LcnfVarId` allocated earlier in `decl_to_lcnf_inner`. So:

| Source                | LcnfVarId allocated   |
|-----------------------|------------------------|
| param `a`             | `0` (fresh_named_var) |
| param `b`             | `1` (fresh_named_var) |
| `Proj("add", …)` head | `3` or `4` (fallback through `convert_expr`) |
| `FVar(0)` ref to `a`  | `5` (convert_fvar)    |
| `FVar(1)` ref to `b`  | `6` (convert_fvar)    |

Hence the `_x4(_x5, _x6)` symptom: the body's arg references
don't reuse the param IDs.

### Issue 1b-β: `UInt64.add` lowered as `Proj("add", 0, …)`

`oxilean-elab` resolves `UInt64.add a b` as a namespace projection
on the `UInt64` constant: `Proj(field_name = "add", index = 0,
base = Const("UInt64"))` applied to `a`, `b`. This is consistent
with Lean stdlib's namespacing semantics (a dotted name on a
`Const`-typed base resolves to a field/method), but `to_lcnf` has
no specialised handling for `Proj` whose base is a `Const(name)` —
it falls through `convert_to_atomic`'s `_` arm into `convert_expr`,
which emits a let-binding for the projection. That let-binding's
ID is yet another fresh `LcnfVarId`, and the result var becomes
the head of the App.

The Rust backend then prints the head ID as `_xN` (no `const_names`
entry, since the Proj's underlying name `UInt64.add` was never
inserted via `convert_const`).

## Why issue 1b-α is the more general problem

Even fixing the `Proj` head — say by teaching `convert_to_atomic`
to special-case `Proj(name, _, Const(base))` and emit a kernel
name `base + "." + name` straight into `name_map` — won't fix the
arg references. **Every body that uses a parameter** will go
through the same `FVar → convert_fvar → fresh_var` path. 1b-α is
the structural fix; 1b-β is a smaller specific fix on top.

## Fix shape (under review)

### Fix 1b-α — option A (leo4-side): re-substitute FVar → BVar in `unfold_decl`

In `leo4-oxilean-build`'s `unfold_decl`, walk each Lam-binder layer
and substitute the body's `FVar(<binder's fvar id>)` references
back to `BVar(<de Bruijn index>)`. The fvar IDs are visible — the
Lam at depth `i` introduces fvar `i` (per the dump). After
substitution, the body becomes raw-kernel-shaped and existing
`decl_to_lcnf` handles it correctly (the OX7 (1a) spike already
proves this for the raw form).

**Trade-off**: leo4-side workaround, doesn't fix the upstream
issue. But: zero fork-side change, no API churn, ships
immediately.

### Fix 1b-α — option B (fork-side): teach `convert_fvar` to look up param-mapped fvars

In `to_lcnf::ToLcnfState`, add a `fvar_to_param: HashMap<u64,
LcnfVarId>` map populated by `decl_to_lcnf_inner` when pushing
params. `convert_fvar` checks this map *before* allocating a fresh
LcnfVarId; on hit, returns the param's existing var_id.

The map is populated only with the fvar IDs that elab assigns to
the binders. Open question: how does `decl_to_lcnf` know those
IDs? Two sub-options:

- B.i — the caller supplies them (new param on
  `decl_to_lcnf_full`).
- B.ii — `convert_type` of the Lam binder records the binder's
  fvar ID as a side-effect of conversion. Fragile — depends on
  elab consistently using the same fvar across `ty` and `val`.

**Trade-off**: fork-side, but addresses the structural issue.
upstream-friendlier than leo4 workaround.

### Recommendation

Start with option A (leo4-side) for a quick T7 win, then port the
logic into fork-side option B once the substitution algorithm is
proven. Both can ship in OX7 (1b); A is one leo4 commit, B becomes
a fork commit plus a leo4-side wire-up.

## Fix 1b-β — `Proj("name", _, Const(base))` as `Const(base.name)`

Small targeted fix in `to_lcnf::convert_to_atomic`: when the expr
is `Proj(name, _idx, base)` and the base is `Const(c_name, _)`,
construct the composite kernel name `c_name + "." + name` and
treat it as a normal `Const` reference (same path as the existing
`Expr::Const` arm — `fresh_named_var(mangled)` +
`name_map.insert(mangled, var_id)`).

This is correct as long as `to_lcnf` doesn't independently see a
matching `Const(c_name.name)` declaration in env (which would
collide on the var_id). The dispatch is name-based at codegen
time, so there's no kernel-level meaning leak.

## Spike artifacts

- `sibling/leo4-oxilean-build/examples/ox7_1b_dump.rs` — runs the
  fixture through real elab, prints the `(ty, val)` shape.
- `parse_decls_for_transpile` is now `pub` so the example can call
  it without leaking the entire `transpile_source_*` pipeline.

## Acceptance for the eventual (1b) fix

The same `def add (a b : UInt64) : UInt64 := UInt64.add a b`
fixture, post-fix, emits:

```rust
pub fn addU64(_x0: u64, _x1: u64) -> u64 {
    UInt64_add(_x0, _x1)
}
```

(No `_x4(_x5, _x6)`. The head is the mangled kernel name; args
reuse the param `LcnfVarId`s.)
