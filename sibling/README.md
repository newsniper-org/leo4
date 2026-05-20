# `sibling/` — non-workspace projects depending on `leo4`

This directory holds Cargo projects that **are not members of the
main leo4 workspace** (`/Cargo.toml`). They exist so leo4 can grow
features that require a different rustc channel / target / lock
file without polluting the main workspace's MSRV and CI policy.

Current siblings:

- **`leo4-wasip3/`** — WASIp3 backend. Pins nightly Rust (WASIp3
  + `wasip3` crate are nightly-only as of Rust 1.95). Depends on
  `crates/leo4-abi` via path so the canonical-ABI marshalling
  layer stays single-source. Phase 7 lights this up with concrete
  host import bindings and `block_on`-driven async dispatch
  behind a sync user-facing API.

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
