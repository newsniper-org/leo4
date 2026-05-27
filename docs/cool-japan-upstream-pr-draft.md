# Upstream PR draft — leo4 fork (`0.1.3-leo4-ox7`) → cool-japan/oxilean

> Status: **DRAFT**, ready for human review before posting.
>
> Branch under contribution: `0.1.3-leo4-ox7`, rebased onto
> `cool-japan/oxilean` `46ad852` (the v0.1.3 base used by leo4).
> Total diff vs `origin/0.1.3`: **52 commits, +7892 / -3 lines** in 11
> files. Roughly 5740 lines are a new sibling crate (`oxilean-parse-peg`);
> the rest is additive to `oxilean-codegen`, `oxilean-kernel`, and
> `oxilean-runtime`.

## 1. Executive summary

This draft proposes upstreaming the OX7 / OX8 work that the
[leo4](https://github.com/<owner>/leo4) Lean ↔ Rust interop project
accumulated on top of OxiLean v0.1.3. The changes fall into four
self-contained, **non-breaking** groups:

| Group | Crate(s) | Lines | Nature |
|---|---|---|---|
| **G1. PEG-based Lean 4 parser** | new `oxilean-parse-peg` | ~5800 | New sibling crate (donated from leo4's `leo4-lean4-parse`) |
| **G2. OX7 Rust-target codegen folds** | `oxilean-codegen` | ~1000 | Additive — new arms in `try_builtin_app`, new `const_names` map |
| **G3. Extern-callback registry (data)** | `oxilean-kernel` | ~300 | Additive — `CallbackRegistry` + `ExternCallError` in `ffi/` |
| **G4. Extern resolver hook (runtime)** | `oxilean-runtime` | ~460 | Additive — `ExternResolver` trait + `dispatch_extern_const` |

No existing public API is modified or removed. v0.1.2/v0.1.3 callers
continue to compile and behave identically.

## 2. Motivation

leo4 is a from-scratch Lean 4 ↔ Rust interop library. Its
"rust-transpile" ladder path compiles Lean source straight to Rust
through OxiLean as kernel + elaborator + LCNF codegen — `leo4 run
--impl rust-transpile` works with zero Lake / zero lean-toolchain on
the user side.

While building this path, leo4 hit four upstream gaps that benefit
any OxiLean embedder, not just leo4:

1. **Parser coverage** — v0.1.2 accepts a small surface; leo4's PEG
   parser handles real Lean (`do` / `match` / typeclasses / structure
   literals / mutual blocks / `by` tactics / attribute lists).
2. **Codegen unfolding for `@[extern]` and typeclass projections** —
   the Rust target emitted unresolved identifiers (`HAdd.hAdd`,
   `ite`, `HPow.hPow`, `Bool.true/false`), link-failing any non-toy
   Lean source.
3. **`@[extern]` runtime dispatch** — v0.1.2 stores `@[extern]`
   metadata but the evaluator can't invoke a Rust callback on
   `Const` reduction. Any embedder (leo4, SMT scripts, theorem-prover
   hosts) needs this.
4. **Const-name preservation in codegen** — the Rust backend threw
   away kernel-side `Const` names, so cross-decl calls rendered as
   `_xN(_xM, _xK)` instead of `someFn(a, b)`.

## 3. Changes per crate

### 3.1 `oxilean-parse-peg` — new sibling crate (G1)

39 OX6 commits' worth of PEG-based Lean 4 parser (operator
precedence, lambda, match, structure / inductive / deriving,
attributes, `do`, string interpolation, classes / instances, mutual
blocks, layout-sensitive expression re-parsing) + subtree-import
(`5b89773`) + donation finalisation rename + `leo4-abi` drop
(`803bf8a`) + a PEG unit-literal / match-arm-arrow fix (`f2254c1`).

Public surface: `parse_decls(src: &str) -> Result<Vec<Decl>,
ParseError>` plus the `Decl` / `Expr` AST. Disjoint from existing
`oxilean-parse`; the two coexist (see §7.1). 288 lib + 1 integration
test pass; only third-party dep is `peg` (MIT).

### 3.2 `oxilean-codegen` — OX7 Rust-target codegen folds (G2)

Six fold commits + one spike, all in `rust_target_backend` +
`to_lcnf`, all gated by `try_builtin_app` so non-Rust backends are
unaffected:

| Commit | Effect |
|---|---|
| `3a99c0c` (OX7 1a) | `const_names: HashMap<LcnfVarId, String>` populated by `convert_const`; App heads print the real mangled name, not `_xN`. |
| `991191d` (OX7 #1+#2) | `UInt8..128` / `Int8..128` / `Float32/64` / `Char` lower to native Rust scalars; declared return type used instead of `()`. |
| `81a0fdc` (OX7 1b-β) | `Proj("add", _, Const("UInt64"))` → `Const("UInt64.add")` — eliminates ghost let-bindings. |
| `fba60b9` (OX7 typeclass) | 13 binary + 2 unary typeclass projections (`HAdd.hAdd`, `LT.lt`, `BEq.beq`, `Neg.neg`, …) fold to native Rust `BinOp` / `UnaryOp`. |
| `4e82655` (OX7 `ite`) | `@ite α c inst t e` → native `if cond { t } else { e }`. |
| `bd1a77f` (OX7 Bool lit) | `Bool.true` / `Bool.false` → native `true` / `false`. |
| `da49bec` (OX7 `HPow`) | `HPow.hPow lhs rhs` → `lhs.pow(rhs)` method call (Rust has no `**`). |

Test counts: 4708 → 4714 lib (+6, one spike per fold). Workspace
clippy clean.

**Note.** The `convert_fvar` re-substitution discussed in
`docs/ox7-1b-elab-bloat-findings.md` (issue 1b-α) is *not* in this
PR — it currently lives leo4-side in `unfold_decl`. We recommend a
follow-up upstream PR once the substitution algorithm has stabilised.

### 3.3 `oxilean-kernel` — CallbackRegistry (G3, OX8.3a)

One commit (`72add72`), one new file in `ffi/`:

- `pub type ExternCallback = Box<dyn Fn(&[u8]) -> Result<Vec<u8>, ExternCallError> + Send + Sync>`
- `pub enum ExternCallError { NotRegistered { lib, symbol }, CallbackFailed(String) }` + `Display` + `std::error::Error`.
- `pub struct CallbackRegistry` with `new` / `register` / `invoke` /
  `len` / `is_empty` / `Default`.

Keyed identically to the existing `ExternRegistry` so a metadata
entry and a callback entry correspond 1:1. +6 unit tests; workspace
green.

### 3.4 `oxilean-runtime` — ExternResolver + dispatch helper (G4, OX8.3b)

One commit (`bf17523`), one new file:

- `pub trait ExternResolver: Send + Sync { fn resolve(&self, &Name, &[u8]) -> Result<Vec<u8>, ExternCallError>; }`
- `pub type SharedExternResolver = Arc<dyn ExternResolver>`
- `pub fn dispatch_extern_const(env, registry, resolver, name, args) -> ExternDispatch`
- `pub fn dispatch_extern_decl(decl, registry, resolver, args) -> ExternDispatch`
- `pub enum ExternDispatch { Resolved(Vec<u8>), NotExtern, NoResolverInstalled, Failed(ExternCallError) }`

The dispatch helpers are **additive only** — they don't wire
themselves into `tco` / `bytecode_interp` / `lazy_eval` (see §6.3).
v0.1.2 behaviour (opaque `@[extern]` axioms) is preserved by the
`NoResolverInstalled` arm. +7 runtime tests; workspace green.

## 4. API additions — non-breaking summary

| Item | Crate | Visibility | Breaks anything? |
|---|---|---|---|
| `oxilean-parse-peg` crate | new | new | No (disjoint name) |
| `RustTargetBackend::try_builtin_app`, `const_names` | codegen | private | No |
| `CallbackRegistry`, `ExternCallback`, `ExternCallError` | kernel | `pub` (additive) | No |
| `ExternResolver`, `SharedExternResolver`, dispatch helpers, `ExternDispatch` | runtime | `pub` (additive) | No |

Users who don't install an `ExternResolver` see exactly the v0.1.2
evaluator behaviour. Users who don't use the Rust target backend see
the same codegen as v0.1.3 for already-supported fragments — the OX7
folds activate on identifiers (`HAdd.hAdd`, `ite`, …) that previously
emitted broken Rust, so there is no working baseline to regress.

## 5. PR sequencing recommendation

**Shape A — four small PRs (recommended).** Each is self-contained
and reviewable in one sitting:

1. **`oxilean-parse-peg`** as a new sibling crate (G1).
2. **`oxilean-codegen` OX7 folds** (G2). No dep on G3/G4.
3. **`oxilean-kernel` CallbackRegistry** (G3). Foundation for G4.
4. **`oxilean-runtime` ExternResolver** (G4). Depends on G3.

**Shape B — two PRs.** PR 1 = G1, PR 2 = G2+G3+G4. Coherent but
bigger diff.

**Shape C — one monolithic PR.** Discouraged — ~8 kloc across 4
crates with two loosely-related themes is hard to review and bisect.

We will rebase the fork's `0.1.3-leo4-ox7` branch into the chosen
shape once the maintainers respond.

## 6. Testing & backport

Aggregate counts on the fork's HEAD vs `origin/0.1.3`:

| Crate | Pre | Post | Δ |
|---|---|---|---|
| `oxilean-parse-peg` | n/a | 288 lib + 1 integration | new |
| `oxilean-codegen` | 4708 lib + 6 int | 4714 lib + 6 int | +6 |
| `oxilean-kernel` | 3307 | 3313 | +6 |
| `oxilean-runtime` | 1162 | 1169 | +7 |
| **Workspace** | 32 415 / 0 fail | 32 421+ / 0 fail | +25 (incl. parse-peg) |

`cargo clippy --workspace --tests -- -D warnings` is clean on HEAD.

leo4-side end-to-end: rust-transpile golden fixtures (`def add (a b
: UInt64) : UInt64 := a + b`, `def constU64 : UInt64 := if true then
1 else 0`, `def pow8 (n : UInt64) : UInt64 := n ^ 8`, …) now
transpile to **compilable** native Rust; pre-OX7 all failed at link
time.

**Backport target: v0.1.4.** All changes are MSRV-clean
(`rust-version = "1.70"`). Only G1 adds a third-party dep (`peg`,
MIT, tree-shake-friendly). If maintainers want to hold G1 for a
minor bump, G2/G3/G4 still fit a 0.1.4 patch.

## 7. Discussion points / open questions

### 7.1 `oxilean-parse-peg` as a separate crate or merged?

The PEG parser is a strict superset of `oxilean-parse` v0.1.2's
accepted surface (verified by the cross-check integration test).
Three options:

- **Side by side** (our default). Lowest disruption; opt-in coverage.
- **Replace `oxilean-parse`.** Breaking for custom-parser users but
  cleaner long-term.
- **Feature flag inside `oxilean-parse`.** Compromise.

No strong preference; we'll follow the maintainers' call.

### 7.2 `ExternResolver` API shape

The chosen split (`CallbackRegistry` in kernel + `ExternResolver`
trait in runtime) was picked over three alternatives — global
static, `Definition.val` synthesis at register time, and `Box<dyn
Fn>` on each `ExternDecl` — for test isolation, symmetry with
existing `ExternRegistry` / `ReductionStrategy` layering, and to
keep kernel free of `Box<dyn Fn>` fields. Full rationale (with the
4-option comparison table) lives in leo4's
`docs/ox8-3-callback-hook-design.md` §"Why this API shape vs.
alternatives".

### 7.3 Wiring `dispatch_extern_const` into existing reducers

Currently **not wired** into `tco` / `bytecode_interp` / `lazy_eval`
— embedders call the helper directly. This keeps the OX8.3b commit
minimal and leaves the "when does `Const` reduction call out"
decision to the maintainers. Happy to ship a follow-up integration
PR once the helper shape is settled.

### 7.4 Fork lifecycle

leo4's submodule currently pins fork branch `0.1.3-leo4-ox7`. Our
intent: keep the fork alive for **1–2 patch releases** of
cool-japan/oxilean (0.1.4 / 0.1.5) while these PRs merge, then drop
the fork and pin upstream. If shape A's four PRs can't all land in
the same patch cycle, the fork stays as a thin compatibility shim —
no leo4-specific patches will survive upstream merging.

---

*Companion docs in the leo4 repo (linkable from the GitHub PR if
useful):*

- `docs/ox7-1b-elab-bloat-findings.md` — diagnosis of the
  `_xN(_xM, _xK)` symptom motivating the Const-name preservation
  fix.
- `docs/ox8-1-leo4-oxilean-audit.md` — audit of how leo4 consumes
  OxiLean, identifying the four upstream gaps proposed here.
- `docs/ox8-3-callback-hook-design.md` — full design rationale for
  the `CallbackRegistry` + `ExternResolver` split.
