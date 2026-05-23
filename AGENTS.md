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
| Loader | `leo4-mslean4` (Rust uses `libloading`) | `libleo4_rust_bridge.a` (dispatcher via worker process) |

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
- **Lean `@[extern]` declared as `BaseIO α` reads back
  differently from `IO α` when called from an `IO` block.**
  Type-level lift works; the C-side `lean_object*` returned
  by `lean_io_result_mk_ok` matches the layout either way,
  but the *caller's* expected layout differs. Use `IO α`
  for Lean externs invoked from `IO` blocks.
- **`UInt32 × ByteArray` is not `lean_alloc_ctor(0, 2, 0)`
  with both fields boxed.** Lean 4 inlines `UInt32` as a
  scalar field in the ctor. When you need both a status and
  a ByteArray across an `@[extern]`, the safe move is a
  single ByteArray whose first 4 bytes hold the status as
  LE u32 — no Prod codegen involved.
- **Worker handshake frame (25 bytes) must be consumed by
  the dispatcher right after spawn.** Skipping it causes
  garbage status values on the first response read. See
  `leo4_consume_handshake` in `shim/leo4_rust_bridge.c`.

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
- **2026-05-23 — Lake `extern_lib` integration landed**
  (overrides the earlier "deferred" note). `lake/Leo4Rust/`
  package's two `extern_lib`s auto-link the dispatcher
  archive + glue shim into any `lean_exe` that
  `require Leo4Rust`s it. Spike findings:
  `spike/SPIKE-1-lake-extern-lib.md`. Patterns 4a (path
  resolution) + 4b (leanc + ar with optional `freshcheck`
  gate) are what ships. Pattern 4c (`buildFileUnlessUpToDate'`)
  was unnecessary in the end — logicutils' content-hash
  guard handles the inner cache layer adequately.
- **2026-05-23 — `leo4` CLI semantics**: `leo4 create` is
  for NEW directories (cargo-new ergonomics); `leo4 init`
  is for IN-PLACE integration into an existing Cargo crate
  (idempotent Cargo.toml append + lean/ scaffold; never
  touches existing `src/`). Don't mix the two — `create`
  refuses a non-empty target, `init` requires a Cargo.toml
  to be present.
- **2026-05-23 — glue shim extern uses `IO ByteArray` +
  status prefix, not `BaseIO (UInt32 × ByteArray)`**: Lean
  4's `BaseIO`/`IO` lift is purely type-level; the actual
  lowered C ABI when called from an `IO` block via `← x`
  reads back ByteArray data differently when the extern was
  declared as `BaseIO α`. Use `IO α` directly. Also the
  `UInt32 × ByteArray` Prod codegen inlines the `UInt32` as
  a scalar field — `lean_alloc_ctor(0, 2, 0)` +
  `lean_box_uint32` does NOT produce the layout Lean expects.
  Both bugs were caught when examples/05 first ran
  end-to-end. Avoid both by returning a single ByteArray
  whose first 4 bytes carry the status as LE u32.
- **2026-05-23 — Dispatcher MUST consume the worker's 25-byte
  handshake before any request**. Worker (Phase 9-3) sends
  it on init; skipping the consume causes the bytes to pile
  up in the IPC buffer and the dispatcher's subsequent
  response read decodes them as a response header.
  Implemented as `leo4_consume_handshake(w)` called inside
  both `leo4_get_or_spawn_persistent` and
  `leo4_dispatch_isolated` right after `spawn` succeeds.
- **2026-05-21 — Phase 10 plan locked**. Ordered substep
  sequence: D1 (`leo4 run`) → F1 (reserved error fixtures)
  → B1 (callback ABI) → D2 (lake-side emit auto-call) →
  B5 (variant payload widening) → A4+A5 (time-based
  recycle + WORKER_RESTARTED side-channel) → C4 (leo4-wasm
  proper) → P10-Docs (E1+E2+E3 in one commit). Each line =
  one commit unless noted. P10.4 minus C4 deferred to ≥
  v1.x. C1 (Windows runtime CI) + G2 (crates.io publish)
  deferred to v1.0 RC pre-release window. Larger context
  in `ROADMAP.md` Phase 10 section.
- **2026-05-21 — Flagship reverse-direction demo lives in
  `Honey-Be/adsmt`, not in leo4 repo**. leo4's
  `examples/05-rust-export/` mini-solver suffices as an
  in-repo smoke. The real SMT solver integration is a
  separate project at https://github.com/Honey-Be/adsmt
  that consumes leo4 as a dependency. Do not bundle
  SMT-specific types (`Term`, `Sort`, …) into leo4.
- **2026-05-21 — `leo4-cli` must remain version-independent
  from `leo4` lib**. No `use leo4_*::…` imports inside
  `crates/leo4-cli/src/`. Scaffold output uses
  `path = "{leo4_root}/..."` (never `version = "x.y.z"`).
  `crates/leo4-cli/Cargo.toml` carries its own `version`,
  detached from `version.workspace`. Helper-binary
  invocations (`leo4-rust-emit`, `-worker`) go through
  user-overridable `--leo4-root` / env so leo4-cli stays
  agnostic to which leo4 lib version is in use. Future
  changes that would require knowing a specific leo4
  internal are a signal the work belongs in a different
  crate (e.g. `leo4-build` or a leo4-version-aware
  subcrate), not in `leo4-cli`.
- **2026-05-21 — leo4-wasm: wasmtime default + wasmi opt-in,
  wasmer rejected**. `crates/leo4-wasm`'s `WasmRuntime` trait
  has two feature-gated backends: `backend-wasmtime` (default)
  and `backend-wasmi` (opt-in). Wasmer was investigated and
  rejected as a backend candidate: its only "Component Model"
  surface is `wai-bindgen-wasmer`, which targets the older
  WAI fork (not WIT), and its README explicitly marks the crate
  as transitional / rewrite-pending. `wasm_component_layer`
  (the multi-runtime CM abstraction) also doesn't list wasmer
  among its supported backends — only wasmtime / wasmi /
  JS-host. If wasmer ever ships real WIT-based Component Model
  in the main `wasmer` crate, adding a third backend is a
  trivial trait impl; until then, including it would mean
  shipping a non-functional feature flag.
- **2026-05-21 — leo4-wasm enforces "exactly-one backend" at
  compile time**. Two `compile_error!` guards in
  `crates/leo4-wasm/src/lib.rs` reject builds that activate
  zero or two backends. Verified against the three scenarios
  (no-default-features → reject; default-features + explicit
  `backend-wasmi` → reject; `--no-default-features --features
  backend-wasmi` → success). Rationale: `crates/leo4-wasm` is
  "one wasm runtime per build" by design — `backend::Default`
  alias resolution would be ambiguous with both backends
  active, and shipping two CM runtimes side-by-side in one
  binary is a bloat the user almost certainly doesn't want.
  If a downstream consumer ever needs multiple backends in
  one process (multi-tenancy, etc.), they can use both
  backend modules' types directly bypassing `Default`, in a
  downstream crate.
- **2026-05-21 — leo4 is Lean-impl-spec-agnostic; OxiLean is
  the canonical alt-impl case study**. Reference Lean 4 is the
  only impl leo4 currently runs against, but the surface leo4
  depends on is now extracted as
  `SPEC/lean-runtime-compat.md` (§1.1 meta-programming API,
  §1.2 `lean.h` C ABI surface, §1.3 `leanc` toolchain, §1.4
  reverse-direction extern/FFI). Any impl that satisfies that
  surface is supported transparently; impls that don't need a
  glue layer.

  [OxiLean](https://github.com/cool-japan/oxilean) (pure-Rust
  CiC ITP, v0.1.2 2026-05-03) is the canonical "alt impl"
  reference case in that SPEC. It satisfies §1.5 (Mathlib
  bridges) trivially (opt-in modules) but **NOT** §1.1–1.4 —
  Rust-native `oxilean-runtime` doesn't expose `lean.h`
  symbols, `oxilean-build` is not Lake, and the meta-
  programming API is OxiLean-shaped. Integration is
  achievable but requires substantial work **on the OxiLean
  side** (or in a leo4-OxiLean compat-layer crate), not on
  leo4 itself.

  Most plausible OxiLean integration points (Phase 11+
  candidates, not Phase 10):

  1. **`SPEC/rust-native-lean.md` direct path** (preferred for
     rust-native impls). Pinned 2026-05-21. Defines a small
     `LeanProc` + `LeanProcInvoker` trait surface that an
     out-of-tree adapter crate (`leo4-oxilean`) implements
     against `oxilean-runtime`'s native Rust API. Same
     canonical-ABI bytes (cross-impl conformance preserved),
     direct Rust function call as transport — no C ABI / no
     IPC / no wasm sandbox. Re-entrant callbacks (Phase 10-B1
     pattern) become trivial because everything is in-process
     Rust. Bypasses §1.2 / §1.3 / §1.4 of
     `SPEC/lean-runtime-compat.md` entirely.
  2. **C4.x.x wasm pipeline**. OxiLean's `oxilean-wasm` could
     expose the `leo4:host/leo4-component@0.1.0` world from
     `SPEC/wit/leo4-host.wit`. Bypasses §1.2 only; §1.3 still
     needs some compile-path equivalent.
  3. **`lean.h` compat shim in OxiLean**. Heaviest option;
     OxiLean would expose a layer of C symbols matching
     reference Lean's ABI. Out of scope for OxiLean per
     today's design.

  **Deeper-dive findings (2026-05-21)**:
  `oxilean-elab/src/lean4_compat/` and
  `oxilean-meta/src/synth_instance/` are encouraging — the
  former is a source-level Lean 4 syntax compat layer in
  active development (submodule names: `Lean4CompatMatrix`,
  `Lean4NamespaceTracker`, `Lean4OptionConfig`,
  `Lean4SectionManager`, `Lean4SyntaxAdapter`,
  `Lean4SyntaxVersion`, `Lean4TermRewriter`), and the latter
  exposes typeclass synthesis via a trait surface
  (`InstanceSynthesizer`, `SynthInstanceConfig`) that's a
  plausible adapter point for the admit-set algorithm. So
  §1.1 of `SPEC/lean-runtime-compat.md` may be more
  satisfiable than the surface inspection suggested.
  Counterbalance: many OxiLean modules carry an
  `//! Auto-generated module structure` doc-comment — a
  project-maturity tell. Production use should wait until a
  leo4-relevant integration test exists upstream that
  exercises the compat layer end-to-end against real Lean 4
  source files. The two-axis surface satisfiability matrix
  lives in `SPEC/lean-runtime-compat.md` §2.

  **FFI-deep-dive findings (2026-05-21)** — most
  significant of the OxiLean investigations:
  `crates/oxilean-kernel/src/ffi/` exposes a complete
  `FfiType` / `FfiValue` / `ExternDecl` / `ExternRegistry`
  model that matches leo4's `LeanProc` / `LeanProcInvoker`
  trait surface (`SPEC/rust-native-lean.md`) almost
  1-to-1:
  * `FfiType` includes every primitive leo4 IDL has
    (u8..u64, i8..i64, f32, f64, bool, String, ByteArray,
    Unit, Ptr, **`Fn(params, ret)` first-class**, OxiLean
    opaque ≈ `LeanResource`).
  * `FfiValue::Bytes(Vec<u8>)` is the natural carrier for
    leo4 canonical-ABI payloads.
  * `ExternRegistry` is the mechanism a `leo4-oxilean`
    adapter uses to register the `LeanProcInvoker::invoke`
    callback for the reverse direction.
  * `crates/oxilean-codegen/src/ffi_bridge/`'s `marshal_type`
    emits `lean_box`/`lean_unbox`/`lean_string_cstr`/
    `lean_mk_string`/`lean_object*` — *the same C ABI
    symbols `SPEC/lean-runtime-compat.md` §1.2 requires.*
    Whether `oxilean-runtime` actually link-exposes these
    symbols (vs. delegating to `libleanshared`) is the
    biggest open question for an adapter author —
    answering it determines whether leo4-mslean4 can run
    against OxiLean OR only leo4-rust-native can.
  * `FfiType::Fn(…)` being first-class means the Phase
    10-B1 callback ABI is **essentially free with OxiLean
    as the impl** (no LECQ/LECR-equivalent re-entry
    protocol needed — just a Rust closure threaded through
    OxiLean's FFI). That's a big architectural win for the
    adsmt flagship use case.
  Full deep-dive table in `SPEC/rust-native-lean.md` §7.1.

  If 병익 (or any other contributor) ever proposes
  "support implementation X", redirect them to
  `SPEC/lean-runtime-compat.md` — that's the checklist that
  determines whether the work is leo4-side or X-side.
- **2026-05-21 — `SPEC/wit/leo4-host.wit` pinned at v0.1.0**.
  The Component Model interface that wasm-targeting leo4
  backends wrap. Key design choices: (a) **opaque `list<u8>`
  canonical-ABI payloads** rather than typed WIT records —
  cross-impl wire identity (native + wasm produce byte-
  identical bytes for the same leo4 type) is the invariant
  worth preserving, and re-encoding through CM's own ABI
  would break it; (b) **one generic `call(mangled, args)`
  export** rather than one typed WIT export per
  `@[leo4_export]` — keeps the WIT stable across schema_hash
  rotations and matches the native pipeline's
  `dlsym(leo4_call_<mangled>)` dispatch model; (c) **schema-
  hash verification on the component side** (via the
  `verify-handshake` export), with the host providing the
  expected value — convention is "side that owns the data
  exports the verifier"; (d) **schema_hash and WIT version
  are independent** — user IDL changes rotate schema_hash but
  not WIT version; leo4 runtime ABI changes rotate WIT
  version but not schema_hash. The `handshake-frame.abi-
  version` field is the WIT-version negotiation channel,
  schema_hash is the IDL-shape one. Full rationale in
  `SPEC/wit/README.md`.
- **2026-05-21 — leo4 has no compile-time plug-in system,
  and that is intentional**. `schema-idl::IDLType` and
  `Leo4Plugin.AdmitSet.IDLType` are closed enums/inductives;
  adding a new IDL primitive or mangling rule requires
  forking + CLAUDE.md's 8-step boundary-type checklist on
  both sides. Reason: `SPEC/mangling.md` §3's schema_hash
  (FNV-1a-64 of normalised IDL bytes) only carries meaning
  if both implementations produce **byte-identical** IDL
  for the same input — a plug-in that adds IDLTypes,
  mangling tokens, or wire-format rules would make
  schema_hash depend on the active plug-in set, breaking:
  (a) cross-impl mangling conformance
  (`tests/mangling/run.sh`), (b) handshake mismatch
  semantics (LEO4_ERR_HANDSHAKE_MISMATCH 0x05 stops
  distinguishing "real type drift" from "plug-in set
  differs"), (c) the `tests/mangling/cases/` golden-file
  contract. The only naturally safe plug-in slot is
  **IDL-consumer-side lowering** (e.g. `leo4c lower`
  emitting WIT today; future emitters to other targets
  could be plug-ins because they don't redefine the IDL,
  only re-express it). Anything that touches the
  IDL/mangling/schema-hash contract is not a plug-in
  candidate — it's a phase ladder addition that requires
  schema-hash rotation. If 병익 ever requests a "plug-in
  system", clarify: consumer-side codegen plug-ins are
  acceptable; IDL-vocabulary plug-ins are not, per this
  decision.

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
