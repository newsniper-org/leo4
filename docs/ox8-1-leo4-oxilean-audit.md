# OX8.1 — leo4-oxilean adapter audit

Opened: 2026-05-27. **Status (2026-05-31): historical
audit doc — all OX8 phases closed.** The "blocked on
upstream OxiLean callback-registration hook" framing
below was accurate on 2026-05-27; the fork branch
`0.1.3-leo4-ox7` has since shipped that hook + the
ExternResolver + the driver module, and leo4 is using
all three via the leo4-oxilean adapter. This doc is
preserved to record the original analysis; current
status of each layer lives in
`sibling/leo4-oxilean/README.md`.

## Findings (2026-05-27 audit, preserved verbatim)

### leo4-oxilean adapter classification

| Layer | Status (as of 2026-05-27) | Status (2026-05-31 update) |
|---|---|---|
| Registration (`OxiLeanInvoker::register_export`) | **DONE** — `Arc<Mutex<ExternRegistry>>` push of `ExternDecl { mangled, params: ByteArray, ret: ByteArray, lib: "leo4-rust-bridge" }`. 8/8 unit tests pass. | Still DONE; 19/19 tests now (callback runtime layered on top). |
| Dispatch (`OxiLeanInvoker::invoke` / `OxiLeanProc::call`) | **SCAFFOLD** — returns `RUST_DLSYM_FAILED` (0x0002_0005) stub. Blocked on upstream OxiLean callback-registration hook. | **WIRED** via OX8.3a/b/c (fork commits `72add72` / `bf17523` / `91430ae`) + P0b/c (`a2c21d9` / `32f26a7` / `521979e` / `44bb382`). |

Crate at `/home/ybi/leo4/sibling/leo4-oxilean/`. Standalone
`[workspace]`, pinned to OxiLean v0.1.2 (will bump to fork's
`0.1.3-leo4-ox7` once we wire the adapter into reverse direction).

### OX8.2 (reverse wrapper emit) — inputs/outputs settled

**Inputs** (from cdylib):

- `dlopen(cdylib_path)` + call exported `leo4_rust_describe_exports(out_ptr, out_len)` — declared in
  `crates/leo4-abi/src/rust_exports.rs`.
- Copies `ExportEntry[]` slice — `{ logical_name, mangled, param_types: &[&str], ret_type: &str, isolated: bool, abi_version: u32 }`. `#[repr(C)]` stable.
- Same data the existing `crates/leo4-rust-emit/` consumer reads (Phase 9-2, mslean4 reverse).

**Output** (`lean/<Iface>/Rust.lean`): one
`@[extern "<mangled>"] def <logical_name>(…) : …` per entry. Signature
reconstructed from `param_types` / `ret_type`. Same shape mslean4
reverse's wrapper emits today — diff-testable.

**Implementation outline** for the OX8.2 commit:

- New `--mode reverse` (or `--reverse-from-cdylib <path>`) flag on the
  `leo4-oxilean-build` binary.
- Copy `crates/leo4-rust-emit/src/render.rs::render_lean_wrapper`'s
  logic into `sibling/leo4-oxilean-build/src/reverse_emit.rs`.
- Conformance fixture under `tests/conformance/` — given two
  `#[leo4::export]` fns, the emitted wrapper round-trips through
  `oxilean-parse-peg` cleanly (OX6 already covers `@[extern]` syntax).

### OX8.3 (`@[extern]` dispatcher) — depends on upstream

Two architectural options:

- **Option A (required)** — upstream OxiLean exposes a
  callback-registration hook in its evaluator:
  ```rust
  oxilean_runtime::register_extern_callback(
      lib_name: "leo4-rust-bridge",
      symbol_name: "leo4_rust__add__u64_u64",
      callback: Box<dyn Fn(&[u8]) -> Result<Vec<u8>, Error>>,
  )
  ```
  Adapter's `OxiLeanInvoker` would register all exports this way at
  init time; `invoke()` then dispatches via the registry. Unblocks
  the `RUST_DLSYM_FAILED` stub.
- **Option B (fallback)** — leo4-oxilean-build emits a Rust wrapper
  module (not `.lean`) that bypasses the OxiLean evaluator and
  dispatches via `libloading` directly. Viable but defeats the
  "Lean-side `@[extern]` native dispatch" model OX8 is built around.

Plan: pursue option A — parallel-path upstream contribution into the
fork's `0.1.3-leo4-ox7` branch (OX7's contribution channel). If
upstream stalls past v1.0 RC, fall back to option B.

### OX7 codegen sufficiency for OX8

**SUFFICIENT** — all six landed OX7 codegen fixes (1a / #1 / #2 /
1b-α / 1b-β / typeclass step) are oriented at the *forward* path's
LCNF + Rust emit. The OX8 wrapper module is **pure `@[extern]`
decls** — no Lean → Rust transpile, no LCNF cost. The EXPORTS slice
already carries `param_types[]` / `ret_type` / `logical_name` in the
exact shape OX8.2 needs (consumed by mslean4 reverse since Phase
9-2).

User's own `lean/Main.lean` body still goes through elab via
leo4-oxilean — OX7 translate coverage matters there. But that's not
a blocker for OX8.2..OX8.5; the wrapper path is decoupled.

### Critical gaps + blockers

| Gap | Severity | Blocks | Owner |
|---|---|---|---|
| OxiLean callback-hook + by-name dispatch | Critical | OX8.3 | upstream (cool-japan/oxilean, contributed via fork's `0.1.3-leo4-ox7`) |
| leo4-oxilean-build `--mode reverse` CLI | Medium | OX8.2 | leo4 |
| Reverse runner in leo4-cli | Low | OX8.4 | leo4 |
| Scaffold for `leo4 create reverse --impl rust-transpile` | Low | OX8.5 | leo4 |

No leo4-side code gaps discovered. OX8.2 / OX8.4 / OX8.5 are
deterministic mechanical commits.

## Recommendations

1. **Start OX8.2 immediately** — `--mode reverse` CLI on
   leo4-oxilean-build. Copy-paste leo4-rust-emit's `render_lean_wrapper`
   logic; no design decisions left.
2. **Engage upstream OxiLean in parallel** — file an issue / PR on the
   fork's `0.1.3-leo4-ox7` branch for the callback-registration hook.
   OX8.3 dispatch needs this; the rest of OX8 doesn't.
3. **OX7 follow-ups can run alongside** — the two tracks touch disjoint
   code (translate / codegen vs. reverse-emit / evaluator-dispatch).
   Multi-decl modules, `HPow.hPow`, the cool-japan upstream PR are all
   independent.

## Closure for OX8.1

This audit doc plus its memory cross-reference closes OX8.1 (the
gate phase). The next OX8 commit can be OX8.2 directly — no further
investigation needed.
