# Agent Playbook for leo4

> Companion to `CLAUDE.md`. CLAUDE.md is the **working agreement** —
> how to behave. This file is the **cookbook** — what to actually
> type when you sit down at the keyboard. Read CLAUDE.md first;
> then jump here when you need patterns.

## 0. Document routing

Use this order when a new session starts and the task is unclear:

| Question | Read first |
|---|---|
| What is this project? What can I do? | `README.md` |
| How should I behave / not behave? | `CLAUDE.md` |
| Why is anything shaped the way it is? | `LEO4-DESIGN.md` |
| What are the next phases? Where are we? | `ROADMAP.md` |
| What's normative on the wire / IDL / mangling? | `SPEC/*.md` |
| What landed recently? What's the release state? | `CHANGELOG.md` |
| What's still wrong with the IDL stack? | `schema-idl-shortcomings.md` |
| How do OS-specific concerns get abstracted? | `OS-PORTABILITY.md` |
| Where do the four-language Typst books live? | `docs/` |
| Can I `cfg(target_os = …)` here? | `OS-PORTABILITY.md` §1 |

If you find yourself reading the source to answer a question that
one of these docs should answer, the doc is probably wrong; fix it
in a separate commit before fixing code.

## 1. Forward vs reverse direction — which mental model applies

leo4 has **two pipelines** that share an IDL and a mangling
scheme, but their build orchestrations are mirror images of each
other. Always know which one you're touching:

| | Forward (Lean → Rust calls Lean) | Reverse (Rust → Lean calls Rust) |
|---|---|---|
| User-tagged side | `@[leo4_export]` on Lean | `#[leo4::export]` on Rust |
| Caller side | Rust via `leo4::import!` | Lean (typed wrapper) |
| Wire shim | `<pkg>.leo4-shim.so` (Lean-built) | `libleo4_rust_bridge.a` (C TU) + `leo4-rust-worker` |
| `lean.h` allowed in | the `.leo4-shim.c` Lake plugin emits | **only** `shim/leo4_rust_bridge_lean.c` |
| Schema-hash suffix in mangled name | yes — `__h<hash>` | **no** — handshake JSON only |
| Build order (D8) | Lake first, Cargo second | Cargo first, emit, Lake second |
| Loader | `leo4-native` (Rust uses `libloading`) | `libleo4_rust_bridge.a` (dispatcher via worker process) |

**Hard rule**: do not import `<lean/lean.h>` from any Rust crate
or any C file other than the two designated shims. The whole
project exists to keep Rust ABI-decoupled from the Lean toolchain
version.

## 2. Commit cadence — what usually moves together

For a single phase landing, the commit message pattern in
`git log` is `Phase <N>-<step>: <one-line summary>`. The
typical files-changed footprint:

- **SPEC / design landing** (e.g. 9-0): `SPEC/<topic>.md` new,
  `LEO4-DESIGN.md` (D-row), `ROADMAP.md` (phase entry),
  `CHANGELOG.md` (Unreleased). Maybe `OS-PORTABILITY.md`,
  `CLAUDE.md` if policy-level.
- **Macro / proc-macro landing** (e.g. 9-1): one or more
  `crates/leo4-macros*/`, often `crates/leo4-abi/` (type
  defs), `crates/leo4/` (façade re-exports), `Cargo.toml`
  (new workspace deps), `Cargo.lock`, `CHANGELOG.md`.
- **CLI / harness binary** (e.g. 9-2, 9-3, 9-7): new
  workspace member under `crates/` or `examples/`, root
  `Cargo.toml` member list + workspace deps, `Cargo.lock`,
  `CHANGELOG.md`.
- **C shim** (e.g. 9-4a/b, 9-6): `shim/<name>.c`, often a
  `crates/<wrapper>/` with `build.rs` (via `cc` crate),
  `Cargo.toml` (`cc` build-dep), `Cargo.lock`,
  `CHANGELOG.md`.
- **End-to-end example** (e.g. 9-7): a directory under
  `examples/` containing Cargo + Lake side-by-side, root
  `Cargo.toml` member list, the example's own `README.md`
  with the manual workflow, `CHANGELOG.md`.

Each commit should pass `cargo check --workspace` and
`cargo test --workspace`. The workspace test count goes up
monotonically (or stays put for SPEC-only commits) — if it
goes down, you broke something silently.

If a commit message would naturally cover two unrelated
changes, split them. The recent history has plenty of
follow-up commits that intentionally stay small (e.g.
`Tier 2 gnullvm follow-up`, `leo4-rust-bridge build.rs:
opportunistic C23 upgrade`).

## 3. Adding a new boundary type — 8-step checklist

Refines the 7-step list in `CLAUDE.md`. Forward direction
unless stated otherwise.

1. **SPEC first**: update `SPEC/canonical-abi.md` with the
   wire format. If the type is a built-in, update
   `SPEC/idl-grammar.ebnf`. If the mangling has a new token,
   update `SPEC/mangling.md` §2.
2. **Rust side encode/decode**: implement `LeanMarshal` in
   `crates/leo4-abi/src/<topic>.rs`. Re-export from
   `crates/leo4-abi/src/lib.rs` and `crates/leo4/src/lib.rs`.
3. **Lean side encode/decode**: implement the matching
   `Leo4.LeanMarshal` instance in
   `lake/Leo4/Leo4/Builtins.lean` (primitives) or a
   dedicated module under `lake/Leo4/Leo4/`.
4. **Plugin discovery**: if the type is a new IDL `IDLType`
   variant, add it to `lake/Leo4Plugin/Leo4Plugin/AdmitSet.lean`
   (`IDLType` inductive) AND mirror it in
   `crates/schema-idl/src/idl.rs` (`IDLType` enum). They MUST
   stay in lockstep — `tests/mangling/` will catch
   divergence.
5. **Shim emitter**: if the type appears at function
   boundaries, add a `TyHandler` arm in
   `lake/Leo4Plugin/Leo4Plugin/Main.lean`.
6. **Conformance test**: a fixture in
   `tests/conformance/fixtures/` that round-trips the type
   through both encoders. Bytes must match exactly.
7. **Sample / example**: add a fixture in
   `tests/sample-lean/Sample.lean` and exercise it from one
   of `examples/`. The schema hash will rotate;
   `CHANGELOG.md` notes both old and new hash.
8. **Mathlib bridge (Phase 8 carrier types only)**: if the
   new type is a carrier mirroring an abstract math object,
   add a `lake/Leo4/Leo4/MathlibBridge/<Sub>.lean` module
   with `toMathlib` / `fromMathlib` functions. Keep
   `Leo4` core Mathlib-independent.

For **reverse direction** new types: in addition,
`rust_type_to_idl` in `crates/leo4-macros-backend/src/lib.rs`
and `lean_type_of_mangle` in `crates/leo4-rust-emit/src/main.rs`
both need a new arm. Otherwise the wrapper emits a `panic!`
stub.

## 4. Adding a new OS-specific layer

Policy: `OS-PORTABILITY.md` §1. The Phase 9-4 spawn / IPC
abstraction is the model. Steps:

1. Identify the concern. Confirm it's not already covered
   (`OS-PORTABILITY.md` §2 table).
2. Pick a name + define the interface in **one place**.
   Conventions: `LEO4_*` macro family or `leo4_*_ops_t` C
   table for shim code; trait or function set for Rust;
   module under `lake/Leo4/Leo4/Platform/` for Lean.
3. Ship a **stub backend first**. Every layer's first
   implementation must be a fallback that the build always
   compiles, even on unsupported platforms. Returning an
   error from every op is fine; building must not fail.
4. Real backends go behind `#ifdef` (C) / `cfg(target_os…)`
   (Rust). Each backend implements the same interface.
5. Update `OS-PORTABILITY.md` §2 with a new row. If a §3
   audit entry is now covered, mark it resolved.
6. Cite the layer in `SPEC/` when the concern is normative
   (e.g. visibility macros affect the ABI).

## 5. Phase entry-gate checklist

When opening a new phase (or new substep), spend the first
commit on **design only** before any code:

- `SPEC/<topic>.md` if normative.
- `LEO4-DESIGN.md` D-table row.
- `ROADMAP.md` phase entry with substeps.
- `CHANGELOG.md` Unreleased entry summarising the design.

Do not write code in the entry-gate commit. The follow-up
substep commits add code referencing back to the SPEC; that
ordering catches design errors before they ossify in code.

The reverse-direction entry (commit `95ad2f2` / `Phase 9-0`)
is the model.

## 6. Cargo / Lake / leanc cheatsheet

```bash
# Default flow — every commit
cargo check --workspace                      # must be clean
cargo test --workspace                       # must be 0 fails
cargo clippy --workspace -- -D warnings      # pedantic; not gating

# Lake side
cd lake && lake build leo4plugin             # plugin
cd lake && lake build Leo4                   # runtime library
just smoke-plugin                            # plugin against sample
just schema-hash                             # current sample's hash

# Cross-impl conformance
just mangling-test                           # Lake vs leo4c
just wit-test                                # WIT lower validation
just conformance-test                        # encoder byte parity
just test                                    # full ladder

# Reverse direction (Phase 9)
cargo build --release -p leo4-rust-bridge    # libleo4_rust_bridge.a
cargo build --release -p leo4-rust-worker    # worker binary
cargo run -p leo4-rust-emit -- --cdylib <so> --out-dir <dir>
cargo run -p leo4-rust-emit -- ... --emit-lean   # +Lean wrapper
leanc -c -std=c2x shim/leo4_rust_bridge_lean.c   # glue shim

# Multi-version Lean matrix (containerised)
just ci-image
just ci-matrix
just ci-version v4.29.1
```

Lean toolchain pinned in `lean-toolchain`. Rust MSRV in
`rust-toolchain.toml`. Tier 2 Windows = `*-pc-windows-gnullvm`
(see `LEO4-DESIGN.md §9.1`).

## 7. Common pitfalls

- **`@[leo4_export]` proc-macros do not know the cdylib's
  schema_hash at expand time.** Reverse direction's mangled
  body deliberately omits the `__h<hash>` suffix; schema_hash
  lives in the handshake JSON + cdylib const only. Forward
  direction is the one that bakes the hash into mangled
  names — different rules.
- **POSIX C source under `-std=c17`/`-std=c2x` needs
  `_GNU_SOURCE` / `_DARWIN_C_SOURCE`** to see `kill`,
  `posix_spawn`, `waitpid`. Define them **before** any
  `#include`. See `shim/leo4_rust_bridge.c`.
- **`--gc-sections` strips exported symbols with no caller**
  during a standalone link. The production link path keeps
  them because the Lean `@[extern]` declaration references
  them. Don't worry about standalone-link `nm` output
  showing exported symbols missing — check `.o`-level `nm`.
- **`cc::Build::std()` takes only one string**. For C23
  opportunistic upgrade with C17 fallback, use last-wins
  semantics via `flag` + `flag_if_supported`:
  ```rust
  build.flag("-std=c17");
  build.flag_if_supported("-std=c2x");
  build.flag_if_supported("-std=c23");
  ```
- **Lean's `IO α` boundary export lowers to `future<α>`** in
  the canonical IDL, not `result<α, error>`. The wire format
  encodes bare `α`; the shim wraps `lean_io_result_*`. See
  SPEC `canonical-abi.md` §13 + `mangling.md` (the `io<T>`
  wire mangle is `I_…_i`, unchanged for cross-impl
  conformance).
- **`linkme::distributed_slice` `static`s look unused to
  rustc.** The macro emits them so the linker collects them
  into the cdylib's `EXPORTS` slice; suppress `#[allow(
  non_upper_case_globals)]` etc. as the macro already does.
- **`schema_hash` recomputation in the worker MUST match the
  emit CLI byte-for-byte.** The two paths (`leo4-rust-emit`
  and `leo4-rust-worker`) share the same algorithm
  (`fnv1a64_base32lc`) and need the same `pkg` / `iface`
  inputs — the dispatcher passes them via
  `LEO4_RUST_HANDSHAKE_PKG` / `_IFACE` env. If you see
  `LEO4_ERR_HANDSHAKE_MISMATCH` in a freshly-built demo,
  check those envs first.
- **`cargo:rerun-if-changed=`** is sticky: once set on a
  build script run, the listed paths must exist on later
  runs or Cargo will not invalidate the cache. Use absolute
  paths in `leo4-build` helpers; do not lean on
  manifest-relative ones.
- **Variant discriminator is `u32 LE`, not `u8`.** Both
  encoders emit 4 bytes — SPEC `canonical-abi.md` §9. Phase
  6 commit `48` was a coordinated change; mixing
  pre- and post-fix bytes makes the wire silently corrupt.

## 8. Subagent guidance

This repo's code is medium-sized but the docs surface is
large. Use subagents as follows:

- **Explore** (read-only) — for "where does X live?" or
  "which files reference Y?" questions across the whole
  tree. Don't bother for a single-file lookup; use Read.
- **Plan** — for designing a new feature where the right
  approach is non-obvious. Make sure the plan ends with a
  *first commit* concrete enough to act on, not a phased
  multi-month roadmap that defers the next move.
- **general-purpose** — for tasks that mix research and a
  write step (e.g. "find every place that calls X and add a
  feature flag to all of them"). Brief the agent on the
  context they're missing.

When in doubt, do the work yourself; subagents are best for
*parallelisable* work or for *protecting your context window*
from huge tool outputs.

## 9. Recent decisions worth remembering

A short list of things that took conversation to land and
would be expensive to relitigate:

- **2026-05-21 — D16 reverse direction**: adopted; Phase 9
  active. SPEC `reverse-direction.md` is the source of truth.
- **2026-05-21 — Tier 2 = `*-pc-windows-gnullvm`**: chosen
  over `*-pc-windows-msvc` to keep the LLVM C toolchain
  stack uniform across tiers. clang `__attribute__((visibility))`
  works everywhere; no MSVC `__declspec` fork.
- **2026-05-21 — Reverse isolation model = long-running
  worker + opt-in `#[leo4::export(isolated)]`**: not the
  zygote-fork variant. SMT-solver state preservation is the
  default; per-call fresh worker is opt-in.
- **2026-05-21 — Spawn / IPC abstraction layer first** (Phase
  9-4a): `leo4_worker_ops_t` ops table in the same single C
  TU; OS-specific code never named outside the backend
  block. The model for any future OS-portability concern.
- **2026-05-20 — D4 async lift complete**: `IO α` → `future<α>`
  in canonical IDL. User-facing Rust/wasm API stays sync;
  wasm side uses `futures::executor::block_on` inside a sync
  wasm export. No `async fn` in public API.
- **2026-05-21 — v0.1.0 cut, schema_hash `qi5gb74dbjyxo`**:
  Phases 0–8 done.
- **2026-05-23 — Phase 9 code landed across 9-0..9-7 +
  9-4c + 9.X**: dispatcher (`libleo4_rust_bridge.a`),
  worker harness (`leo4-rust-worker`), emit CLI
  (`leo4-rust-emit --emit-lean`), Lean-side glue shim
  (`shim/leo4_rust_bridge_lean.c`), POSIX + Windows
  backends, `#[leo4::export(isolated)]` via `iso:` prefix
  trick, `LEO4_RUST_WORKER_RECYCLE_CALLS=N` recycle policy,
  `examples/05-rust-export/` e2e demo.
- **2026-05-23 — `iso:` prefix as the isolated-mode wire
  signal**: the Lean wrapper prepends `"iso:"` to the
  mangled name passed to `leo4_rust_call`; dispatcher
  strips it and routes through a per-call fresh worker. No
  wire format change, no new dispatcher API entry,
  backwards-compatible with default callers. The `iso:`
  string is reserved in the mangled-name space.
- **2026-05-23 — Lake `extern_lib` integration deferred**:
  Lake 5.x's `extern_lib` DSL is Job-based and our trial
  attempt was too brittle for a single safe commit. The
  current state of the art is `Leo4.Build.RustBridge`
  helpers + `just rust-export-05-build` + a manual `leanc
  -o` final link line. A focused Lake-API spike is the
  prerequisite for the declarative integration.
- **2026-05-23 — `leo4` CLI semantics**: `leo4 create` is
  for NEW directories (cargo-new ergonomics); `leo4 init`
  is for IN-PLACE integration into an existing Cargo crate
  (idempotent Cargo.toml append + lean/ scaffold; never
  touches existing `src/`). Don't mix the two — `create`
  refuses a non-empty target, `init` requires a Cargo.toml
  to be present.

Anything in this list that needs to change → discuss with
병익 before touching code. See CLAUDE.md "If a request from
병익 contradicts LEO4-DESIGN.md, raise the conflict
explicitly".

## 10. When you don't know what to do next

Read `ROADMAP.md` for in-flight phases and substeps. The
"Open follow-ups" section at the bottom of `CHANGELOG.md`'s
[Unreleased] block lists known deferrals. If those don't
yield a concrete next step, ask 병익 directly with a
specific question rather than guessing.
