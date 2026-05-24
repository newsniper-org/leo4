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
- **`leo4-lean4-parse/`** (2026-05-22 → 2026-05-24) — OX6 PEG-based
  Lean 4 parser built from scratch with the `peg` crate. Strict
  superset of `oxilean-parse` v0.1.2's accepted surface. AST
  shapes mirror upstream for downstream interop. 289 tests
  (288 lib + 1 integration cross-check against `oxilean-parse`
  on a shared corpus). All ~25 sub-steps landed; replaces the
  OX3/OX4 textual pre-rewrite chain in `leo4-oxilean-build`.
- **`leo4-oxilean-build/`** — OxiLean transpile path (the
  `rust-transpile` impl kind in `leo4.toml`). Pipeline:
  parse via leo4-lean4-parse → translate to oxilean's `Decl`
  via the `leo4_translate` module → elab against
  `leo4_env_bootstrap::bootstrap_env()` (OxiLean
  `init_builtin_env` + leo4 boundary primitives, **zero
  lake/lean overhead**) → lower via `oxilean_codegen::to_lcnf`
  → emit a Rust crate. `leo4-parser` cargo feature (default
  ON since 2026-05-24) selects the new path;
  `--no-default-features` falls back to oxilean-parse-direct.
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
