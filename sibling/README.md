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
  `0.1.3-leo4-ox7`. OX7 work tree: codegen fixes (BVar/Const
  ID tracking, return-type inference, UInt/Int/Float Rust
  type mapping, HAdd-family typeclass + Nat/UInt/Int instance
  registration). The fork also hosts `oxilean-parse-peg/`,
  the PEG-based Lean 4 parser donated upstream from the
  former `sibling/leo4-lean4-parse/` (subtree-imported with
  history preserved, then renamed + made leo4-independent
  via crate-local `ParseError`). Eventually upstreamed to
  `cool-japan/oxilean` (γ-1' contribution option 1).
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
  EXPERIMENTAL in v1.0 RC pending OX7 (see warning emitted
  by `leo4 run --impl rust-transpile`).
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
