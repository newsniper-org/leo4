# `sibling/` — non-workspace projects depending on `leo4`

This directory holds Cargo projects that **are not members of the
main leo4 workspace** (`/Cargo.toml`). They exist so leo4 can grow
features that require a different rustc channel / target / lock
file without polluting the main workspace's MSRV and CI policy.

Current siblings:

- **`leo4-wasip3/`** — WASIp3 backend. Pins **stable** Rust + the
  `wasm32-wasip2` target (verified 2026-05-21: the `wasip3` crate
  v0.6 ships WASIp3 API bindings as compatibility shims on
  wasip2's Component Model, so a stable toolchain compiles it).
  Depends on `crates/leo4-abi` via path so the canonical-ABI
  marshalling layer stays single-source. Phase 7 lights this up
  with concrete host import bindings and
  `futures::executor::block_on`-driven async dispatch behind a
  sync user-facing API.
- **`oxilean/`** (git submodule, since 2026-05-26) — OxiLean
  fork at `https://github.com/newsniper-org/oxilean`, branch
  `0.1.3-leo4-ox7`. As of 2026-05-31 the work tree carries:
  the OX7 codegen series (BVar/Const ID tracking, return-type
  inference, UInt/Int/Float Rust type mapping, HAdd-family
  typeclass + Nat/UInt/Int instance registration); the OX8.3a
  `CallbackRegistry` + OX8.3b `ExternResolver` /
  `dispatch_extern_const` hooks; and the `oxilean_runtime::
  driver` module (#76 P0c, closed 2026-05-31) — full monad-
  transformer-family IO walker with `IO.bind` beta-application,
  canonical-ABI arg encoding (including user-defined record /
  inductive ctors via env-lookup of `ConstantInfo::Constructor`),
  and direct dispatch for `IO.println` + `IO.FS.*` stdlib
  families. Out-of-scope tail (`StateT.run`, `IO.FS.Handle`,
  `dbg_trace`, float literals) is explicitly classified rather
  than open. Fork tests: 1219 passing. The fork also hosts
  `oxilean-parse-peg/`, the PEG-based Lean 4 parser donated
  upstream from the former `sibling/leo4-lean4-parse/`
  (subtree-imported with history preserved, then renamed +
  made leo4-independent via crate-local `ParseError`).
  Eventually upstreamed to `cool-japan/oxilean` (γ-1'
  contribution option 1).
- **`leo4-oxilean-build/`** — OxiLean transpile path (the
  `rust-transpile` impl kind in `leo4.toml`). Pipeline:
  parse via `oxilean-parse-peg` (from the `oxilean/` fork
  submodule) → translate to oxilean's `Decl` via the
  `leo4_translate` module → elab against
  `leo4_env_bootstrap::bootstrap_env()` (OxiLean
  `init_builtin_env` + leo4 boundary primitives, **zero
  lake/lean overhead**) → lower via `oxilean_codegen::to_lcnf`
  → emit a Rust crate. `leo4-parser` cargo feature (default
  ON since 2026-05-24) selects the new path;
  `--no-default-features` falls back to oxilean-parse-direct.
  Cleared for v1.0 RC 1 (2026-05-31): #72 (codegen, translate
  tests 36 → 56) + #76 P0c (driver IO walker) both closed;
  primitive-arithmetic, `if`/`match`/`let-in`, multi-decl,
  `Mutual`, `BinOp` Unicode + negated forms, and user-record /
  inductive ctor encoding all transpile end-to-end. Out-of-
  scope tail remains classified, not blocking.
- **`leo4-oxilean-runner/`** — OX8.5 B1/B2 runner helper
  (2026-05-28; #76 P0c-completed 2026-05-31). Folds the
  cdylib `dlopen` + `EXPORTS`-slice walk + `OxiLeanInvoker`
  callback registration + `lean/Main.lean` parse + elab
  into a single `run_main` entry point. The `leo4 create
  reverse --impl rust-transpile` scaffold's `src/main.rs`
  collapses to one call. The "drive `main : IO Unit` to its
  IO effects" step now executes via the fork's
  `oxilean_runtime::driver` (full monad-transformer-family
  walker on the fork side); cool-japan upstream tracking of
  the same API stays in `docs/cool-japan-driver-api-
  coordination-draft.md`.
- **`leo4-oxilean-bootstrap/`** — leaf crate for the
  OX5-oxi env bootstrap (`bootstrap_env`,
  `add_leo4_primitives`, `LEO4_PRIMITIVE_TYPES`,
  `ARITHMETIC_TC_PROJECTIONS`, `STRING_INTERP_AXIOMS`).
  Single source of truth shared by `leo4-oxilean-build`
  (CLI) and `leo4-oxilean-runner` (scaffold helper) — both
  consume via `path =` and re-export from the shim file
  they used to vendor (commit `41542da`, task #78).
- **`leo4-oxilean-translate/`** — leaf crate for the OX6
  step 13 translator (`translate_decl`,
  `oxilean_parse_peg::Decl` → `oxilean_parse::Decl`).
  Single source of truth shared by the same two consumers
  as `leo4-oxilean-bootstrap` (2026-05-28, task #78
  follow-up). Replaces the ~1730-line vendor that used to
  live in each consumer's `src/leo4_translate.rs`.
- **`leo4-lean4-parse/`** — historical home of the
  PEG-based Lean 4 parser. As of 2026-05-26 the crate was
  donated upstream into the `oxilean/` fork submodule as
  `oxilean-parse-peg` (subtree-imported with history). The
  directory remains only as a `target/` shell from earlier
  builds and is no longer a source-of-truth crate; do not
  edit it. Consumers (`leo4-oxilean-build`,
  `leo4-oxilean-runner`) depend on `oxilean-parse-peg`
  directly via the submodule.
- **`mathlib-bridge-test/`** — Lake package pulling Mathlib +
  `Leo4`. Type-checks every `Leo4.MathlibBridge.*` module
  end-to-end. Mathlib's cold build is 1-2 hours, so this isn't on
  the default `just test` ladder — `just mathlib-bridge-test`
  drives it explicitly. First run downloads + compiles all of
  Mathlib's transitive deps via Lake's reservoir; subsequent runs
  hit the local `.lake` cache.

Planned (deferred):

- **`leo4-wasm64/`** — Memory64 backend. Blocked on Rust stable
  promoting `wasm64-*` past tier 3; revisit when `rustc --print
  target-list` shows it as tier 2 or 1.

## Build / test conventions

Each sibling has its own `rust-toolchain.toml`, `Cargo.toml`, and
`target/` directory. `cd sibling/<name> && cargo build` picks up
the local toolchain without affecting the workspace `target/`. CI
(when added) runs each sibling under its pinned channel
independently.

The main workspace's `justfile` may grow `just sibling-build
<name>` helpers later; today we keep them invocation-isolated to
make it obvious which toolchain is in play.
