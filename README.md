# leo4

Lean 4 ↔ Rust interop that does not bind the Rust side to a Lean
toolchain version.

## Status

**Phases 0–8 released as v0.1.0 (2026-05-21). Phase 9
(reverse direction: Rust → Lean) fully end-to-end as of
2026-05-23.** The forward pipeline runs end-to-end on
Tier 1 (x86_64 Linux glibc): Lake plugin, Rust workspace,
shim synthesis, C ↔ Lean dispatch, mutual recursion, async
`io<T>` lift, and a Mathlib-compatible carrier-type subset
with opt-in bridges. `tests/mangling/` confirms cross-impl
byte-identical mangling across **70 mangled instantiations**
(29 logical entries) with schema_hash `qi5gb74dbjyxo` between
the Lean plugin and the Rust `schema-idl` crate.

**v1.0 RC progress (2026-05-29 update)** — most v1.0
RC blockers cleared. The remaining gauge is the
`oxilean_runtime::driver` IO walker; the walker now
covers `IO.pure`, `IO.bind` (arity-4 + arity-2), and
`@[extern]` Const dispatch — `EStateM` lowerings +
beta-application of `k` with concrete result from `m`
remain. Recent batch:

- **#76 P0c IO walker coverage** (2026-05-29). Fork
  `d357a01` adds `IO.bind` + `@[extern]` Const
  dispatch on top of the 2026-05-28 v0 walker. leo4
  main bumps the fork + adapts the runner caller
  (`c0f81c7`) and lands the leo4-oxilean outbound
  dispatch bridge (`44bb382`) — when the walker
  fires a Lean closure dereference, the
  `register_outbound_dispatch_callback` shim unpacks
  `(callback_id, rest)` and forwards to
  `OxiLeanInvoker::invoke_outbound`. The cool-japan
  driver API coordination is posted at
  [cool-japan/oxilean#2](https://github.com/cool-japan/oxilean/issues/2);
  body PR submission deferred until explicit
  maintainer feedback.

Recent batch (2026-05-28):

- **Phase 10-B1.x callback ABI runtime** (2026-05-28).
  Three leo4-side steps + fork-side v0 IO walker:
  - leo4-abi `RustCallbackRegistry` substrate
    (`a2c21d9`) — RAII-enforced per-call-scope contract
    (SPEC §13a).
  - `leo4::import!` macro emits register-encode-call-
    decode-deregister sequence for `fn(T₁,…,Tₙ) -> R`
    args + `Lean::callback_registry()` accessor
    (`32f26a7`).
  - `OxiLeanInvoker::{attach_outbound_registry,
    outbound_registry, invoke_outbound}` (`521979e`) —
    adapter-side dispatch surface.
  - Fork `8b2af9f` lands the v0 IO walker recognising
    `IO.pure` shape only; remaining shapes return
    `DriverError::NotYetImplemented` with the offending
    expression's debug repr.
  - `docs/cool-japan-driver-api-coordination-draft.md`
    (discussion-only) proposes the
    `oxilean_runtime::driver` API for cool-japan
    review; **submission deferred to post-v1.0 RC**
    alongside the OX7/OX8 upstream PR.
- **OX8 rust-transpile reverse direction** — all 5
  phases closed (audit → wrapper emit → evaluator →
  runner → scaffold). `sibling/leo4-oxilean-runner/`
  helper folds dlopen + EXPORTS walk + invoker
  registration + parse + elab into one `run_main(...)`
  call; final IO-effect drive blocked on the same
  IO walker body above.
- **OX7 codegen + parser donation** — fork branch
  `0.1.3-leo4-ox7` accumulates the codegen fixes
  (Const name / native scalars / typeclass projection
  fold / ite / Bool / HPow / String-literal coercion)
  + the OX6 PEG parser donation + the `extern_resolver`
  + `CallbackRegistry` extensions. cool-japan upstream
  PR draft complete; **submission deferred to
  post-v1.0 RC**.
- **Leaf crates dedup** — `sibling/leo4-oxilean-
  bootstrap/` + `sibling/leo4-oxilean-translate/`
  share-of-truth crates so the build and runner
  consumers stop vendoring duplicates.
- **`just linux-distro-audit <distro>`** — NO-hard-
  coding distro audit infra (`ci/linux-distro-audit/
  distros.toml` data + Python runner + payload shell
  script). 5 current-stable distros covered.
- **Windows support floor** — pinned to UCRT's own
  supported range (Vista SP2 + KB2999226 / Win 7 SP1
  + KB3118401 / newer NT). KB install is downstream's
  deployment concern, not leo4's.

**v1.0 RC progress (2026-05-24)** — the v1.0 RC OX
blockers cleared in a single push:

- **OX6** PEG-based Lean 4 parser (`sibling/leo4-lean4-parse`),
  steps 1–13d landed; `leo4-oxilean-build`'s default
  parser is now `leo4_lean4_parse::parse_decls` →
  `leo4_translate` → `oxilean_parse::Decl`; the legacy
  oxilean-parse-direct path remains as fallback. Strict
  superset of oxilean-parse v0.1.2's accepted surface.
- **OX5** elab env bootstrap — `leo4-oxilean-build`'s
  rust-transpile path uses `oxilean_kernel::init_builtin_env`
  + leo4 boundary primitives (UInt8..128, Int8..128,
  Float32/64, Char). Zero lake/lean overhead. `OX5-msl`
  closed no-op (mslean4 path uses lake plugin's Lean-native
  elab; no Rust-side analogue exists).
- **Post-OX6 CLI refactor** — `leo4 create` / `leo4 init`
  drop the `--impl <kind>` flag; runtime-impl selection
  moves into a per-(sub)crate `leo4.toml` config file.
  `leo4 create --subcrate` registers into the surrounding
  workspace's `members` array. `leo4 init` auto-migrates
  any legacy `.leo4-impl` marker. `leo4 run` reads
  `leo4.toml` with `--impl <kind>` as selector when
  multiple `[[impl]]` entries are present.
- **OS abstraction Leo4.Platform layer** — first leo4-Lean
  OS abstraction (`lake/Leo4/Leo4/Platform.lean`)
  encapsulates `.so` / `.dylib` / `.dll` choice and the
  POSIX-only `-Wl,-rpath` emission previously hardcoded
  in `Leo4.Build`.
- **Windows IPC** for reverse direction worker side
  (`leo4-rust-worker`'s `open_ipc_channel`) — the missing
  half of Phase 9-4c — now opens the dispatcher's named
  pipe via `CreateFileW` with retry on the spawn race.
  Cross-compile clean on `x86_64-pc-windows-gnullvm`.
- **musl Tier 1+ policy** (C5, v1.0 RC mandatory) for
  paths with no `leo4-mslean4` and no lake dependency
  (rust-transpile / scaffold-only / pure-Rust crates).
  14 workspace crates build musl-clean out of the box;
  `leo4-rust-bridge` / `leo4-wasm` need a host musl C
  toolchain (`musl-clang` or `musl-gcc`).
  `leo4-rust-bridge`'s build.rs auto-fixes Arch's
  `musl-clang` `stdatomic.h` packaging quirk.
  **Android `*-linux-android*` Tier 2** (C6) deferred
  to v1.x with the same path scope.
- **`*-pc-windows-gnullvm` Tier 2** runtime verification
  (C1) — `docs/windows-manual-test-plan.md` holds the
  pre-flight audit + test matrix; manual VirtualBox pass
  precedes CI infra.

The reverse pipeline (Phase 9) lets Rust expose
`#[leo4::export]`-tagged functions that Lean calls through a
long-running worker process. `examples/05-rust-export/` is the
end-to-end demo; **`cargo build && leo4-rust-emit --emit-lean
&& lake build`** is the entire user-visible workflow —
`just rust-export-05-build` chains it for examples/05. Lake
picks up `libleo4_rust_bridge.a` + the leanc-compiled glue
shim automatically via two `extern_lib`s in the `Leo4Rust`
Lake package (`lake/Leo4Rust/lakefile.lean`).

What works today:

- **Lean runtime** (`lake/Leo4/`) — `@[leo4_export]`,
  `@[leo4_specialize_when …]`, the `leo4_constraint` syntax category,
  `class LeanMarshal` over `ByteArray`, `class LeanResource` +
  `@[leo4_resource]`, primitive blanket instances, `deriving
  LeanMarshal` for `structure` / `inductive` (self-recursive variants
  + mutual clusters via `mutual ... end`), `LeanU128` / `LeanI128`,
  `LeanComplexF{32,64}x2`. Optional `Leo4.NightlyFloats` (`LeanF16`,
  `LeanBF16`, `LeanF128`, complex variants) and the opt-in
  `Leo4.MathlibBridge.*` modules (1-to-1 conversions to `Nat` / `Int`
  / `BitVec` / `ZMod (2^128)` / `ℝ` / `ℂ`).
- **Lake plugin** (`lake/Leo4Plugin/`, `lean_exe leo4plugin`) —
  re-loads the user package via `Lean.importModules (loadExts :=
  true)`, computes admit-sets (phantom / unbounded / class /
  value-erased / higher-kind reject — `LEO4-DESIGN.md §5`), handles
  `mutual` clusters with `Cyc<i>` cycle-breakers, lifts `IO α` to
  `future<α>`, recognises `UserDecl.externalMarshal` for
  proof-carrying types (`Rat`), emits the C shim
  (`<pkg>.leo4-shim.c`) and Lean wrapper, drives `leanc` to produce
  `<pkg>.leo4-shim.so`, atomically writes
  `<pkg>.leo4-schema` / `<pkg>.leo4-mangling` /
  `<pkg>.leo4-handshake`. `--with-lower` shells out to `leo4c` for
  optional `<pkg>.wit`.
- **Rust workspace** — `crates/schema-idl` (parser + mangle +
  canonical render), `crates/leo4-idl` (WIT lowering),
  `crates/leo4c` CLI (`parse` / `canonical` / `mangle` / `lower`),
  `crates/leo4-abi` (canonical-ABI marshal for scalars / composites /
  `BigNat` / `BigInt` / `LeanRat` / `LeanU128` / `LeanI128` /
  `LeanComplexF{32,64}x2`, optional `nightly-floats` feature),
  `crates/leo4-mslean4` (loader, `Arena<'a>`, `LeanRef<'a, T>`,
  dispatch cache), `crates/leo4-macros` (`leo4::import!`,
  `#[derive(LeanMarshal)]`), `crates/leo4-build` (build-script
  helper), `crates/leo4` (top-level façade).
- **Sibling projects** — `sibling/leo4-wasip3/` (stable Rust +
  `wasm32-wasip2` + `wasip3` v0.6, Phase 7 finisher),
  `sibling/mathlib-bridge-test/` (Lake package pulling Mathlib +
  `Leo4`; verifies every `MathlibBridge.*` module type-checks).
- **Cross-impl conformance** —
  - `tests/mangling/` (`just mangling-test`) — schema_hash +
    70 mangled instantiations byte-identical between Lake plugin
    and `leo4c`.
  - `tests/wit/` (`just wit-test`) — IDL cases lower to WIT that
    passes `wasm-tools component wit` and `wit-bindgen markdown`.
  - `tests/conformance/` (`just conformance-test`) — Lean encoder
    bytes reproduced byte-identical by the Rust encoder.
- **End-to-end examples** —
  - `examples/01-hello/` — scalars, four nominal-type wrappers,
    derive-only round-trips, `#[leo4(args = "…")]` attribute path,
    handshake-mismatch detection (`LEO4_ERR_HANDSHAKE_MISMATCH` =
    `0x05`), `addRat` (`external Rat`), `addU128`,
    `mulComplexF64x2`, async `asyncDouble` / `asyncNegate` /
    `asyncHalveF{32,64}`.
  - `examples/02-roundtrip/` — `list<u32>` + `bignat` round-trip,
    `listSumU64` / `listConcat`, multi-instantiation `listLen`.
  - `examples/04-mutual-ast/` — `Sample.Expr` / `Sample.Stmt`
    mutual cluster, hand-rolled `LeanMarshal` impls with `Box<T>`
    breaking the Rust-side cycle.
- **Multi-version CI matrix** (`ci/`, `just ci-matrix`) — hermetic
  Ubuntu container with `elan`, `rustup`, `wasm-tools`, and
  `wit-bindgen`. Matrix `v4.27.0 / v4.28.0 / v4.29.1 / v4.30.0-rc2`
  iterates inside a single container; hardlink-shared source mirror
  in a named volume gives union-FS-style dedup without privileged
  mounts. The matrix is the fallback path against GitHub-Actions
  outages.
- **Multilingual documentation** under `docs/` — Typst sources for
  a short learning overview and a long-form
  "implement-from-scratch" guide book, each in English / Korean /
  Japanese / German (`docs/learning/<lang>/main.typ`,
  `docs/implement-from-scratch/<lang>/main.typ`).
- **Reverse direction (Phase 9, 2026-05-23)** — Rust cdylibs
  expose `#[leo4::export]`-tagged functions that Lean calls
  through a long-running worker process. The pipeline:
  - `#[leo4::export]` proc-macro emits per-fn wrapper symbols
    + `linkme` metadata (9-1).
  - `leo4-rust-emit` walks the cdylib's `EXPORTS` slice and
    writes `<pkg>.leo4-rust-exports.idl` /
    `<pkg>.leo4-rust-handshake` /
    `<pkg>.leo4-rust-imports.lean` (9-2, 9-5).
  - `leo4-rust-worker` is the per-cdylib worker harness (9-3).
  - `libleo4_rust_bridge.a` is the dispatcher static archive
    with a POSIX backend (9-4a/4b) and a Windows backend
    (9-4c; gnullvm Tier 2). Spawn / IPC abstracted behind
    `leo4_worker_ops_t` so future backends (zygote, wasm)
    plug in without churn.
  - `shim/leo4_rust_bridge_lean.c` is the Lean-side glue
    shim, the only leo4 C TU that includes `<lean/lean.h>`
    (9-6).
  - `examples/05-rust-export/` is the end-to-end demo
    (mini-solver Rust functions called from Lean).
  - `#[leo4::export(isolated)]` opts a function into
    per-call fresh worker mode; `LEO4_RUST_WORKER_RECYCLE_CALLS`
    bounds the persistent worker's lifetime (9.X).
- **`leo4` CLI** (`crates/leo4-cli/`, 2026-05-23; refactor
  2026-05-24) — `leo4 create <direction> <dir>` scaffolds
  a new project; `leo4 init <direction>` integrates leo4
  into an existing Cargo crate (idempotent Cargo.toml
  append + lean/ scaffold). **Both write a `leo4.toml`**
  declaring runtime impls; `--impl <kind>` is no longer a
  CLI flag. `leo4 create --subcrate` registers the new
  crate into the surrounding workspace's `members` array.
  `leo4 init` auto-migrates the legacy `.leo4-impl`
  marker. `leo4 run` reads `leo4.toml` with `--impl`
  acting as a selector when multiple `[[impl]]` entries
  are present.
- **Sibling parser fork** (`sibling/leo4-lean4-parse/`,
  2026-05-22 → 2026-05-24) — PEG-based Lean 4 parser
  (`peg` crate), strict superset of `oxilean-parse`
  v0.1.2's accepted surface. Replaced the OX3/OX4
  textual pre-rewrite chain in
  `leo4-oxilean-build`. 289 tests (288 lib + 1
  cross-check against oxilean-parse on a shared
  corpus). leo4_translate (`sibling/leo4-oxilean-build`
  `leo4_translate` module) lowers
  `leo4_lean4_parse::Decl` → `oxilean_parse::Decl`
  so the elab / codegen pipeline stays unchanged.

Open items:

- Some `LeanError` codes (`0x02` / `0x03` / `0x04` / `0x06` / `0x08`)
  are reserved but not yet exercised by a test fixture.
- **C1** Windows runtime verification (Tier 2 CI matrix
  for `*-pc-windows-gnullvm`). Code compiles + worker-side
  Windows IPC landed 2026-05-24; manual VirtualBox pass
  (`docs/windows-manual-test-plan.md`) precedes CI.
- **C5** musl CI matrix row pending; code 0-changes
  needed (audit verified 2026-05-24).
- **G2** Publish to crates.io — API surface stabilised;
  metadata + dep-order publish remains.
- `LEO4_ERR_RUST_WORKER_RESTARTED` (0x00020002) is reserved
  but not surfaced — recycle is currently transparent.
- `LEO4_RUST_WORKER_RECYCLE_SECONDS` (time-based recycle)
  deferred; call-based ships.
- Callback / function-arrow ABI deferred (no concrete
  consumer yet).
- schema-idl items G (`ConstraintExpr<Atom>` typed AST) and the
  `wasm64` sibling stay deferred until a concrete consumer surfaces.

## Documents to read, in order

1. [`LEO4-DESIGN.md`](LEO4-DESIGN.md) — every design decision and its
   rationale (D1–D16, type-system layer, admit-set algorithm,
   forbidden constructs).
2. [`CLAUDE.md`](CLAUDE.md) — working agreement for Claude Code
   sessions in this repo.
3. [`AGENTS.md`](AGENTS.md) — cookbook for Claude Code sessions
   (entry routing table, commit cadence patterns, common
   pitfalls, recent decisions).
4. [`ROADMAP.md`](ROADMAP.md) — phased work plan, exit criteria per
   phase, the deferred IDL-output-grouping decision.
5. [`OS-PORTABILITY.md`](OS-PORTABILITY.md) — policy + audit
   ledger for OS-specific code. New `cfg(target_os=…)` /
   `#ifdef` branches go through this.
6. [`spike/SPIKE-0-FINDINGS.md`](spike/SPIKE-0-FINDINGS.md) — why the
   plugin re-imports `.olean` rather than hooking
   `Lake.Module.recBuildLean`.
7. `SPEC/*.md` — normative specifications:
   - [`SPEC/idl-grammar.ebnf`](SPEC/idl-grammar.ebnf) — IDL grammar
     (WIT-superset, `kind`, `Self`/`Self<…>`, `value_param`,
     `nominal_decl` short-form, `external_decl`).
   - [`SPEC/canonical-abi.md`](SPEC/canonical-abi.md) — wire format,
     error-code table (incl. forward + reverse passthrough
     ranges).
   - [`SPEC/mangling.md`](SPEC/mangling.md) — name mangling, schema
     hash (FNV-1a-64 → base32lc), kind discipline.
   - [`SPEC/handshake.md`](SPEC/handshake.md) — JSON file formats,
     atomic-emission contract, `.leo4-schema` canonical-form rules.
   - [`SPEC/phase-6-mutual.md`](SPEC/phase-6-mutual.md) — mutual
     recursion + `Cyc<i>`.
   - [`SPEC/reverse-direction.md`](SPEC/reverse-direction.md) —
     Phase 9: dispatcher API, worker lifecycle, IPC wire format,
     isolation matrix, build orchestration.

## Why leo4 and not leo3

`leo3` is a fine effort, but it compiles against `lean.h` directly.
That makes the Rust crate version-locked to a specific Lean toolchain,
and the lock breaks whenever Lean's internal layout shifts. leo4 puts
all Lean ABI knowledge in a build-time-generated C shim, and exposes
only a stable canonical ABI to the Rust crate. The Rust crate
therefore tracks the IDL, not the Lean toolchain.

See `LEO4-DESIGN.md` §0 for the longer version.

## Layout

```
.
├── LEO4-DESIGN.md          # single source of truth
├── CLAUDE.md               # Claude Code working agreement
├── ROADMAP.md              # phased plan
├── CHANGELOG.md            # release history
├── SPEC/                   # normative specs
│   ├── idl-grammar.ebnf
│   ├── canonical-abi.md
│   ├── mangling.md
│   ├── handshake.md
│   ├── phase-6-mutual.md
│   └── reverse-direction.md # Phase 9 dispatcher + worker + wire format
├── AGENTS.md               # Claude Code agent cookbook
├── OS-PORTABILITY.md       # cross-OS abstraction policy + audit
├── crates/                 # Cargo workspace
│   ├── schema-idl/         # parser + IDL types + mangling + canonical render
│   ├── leo4-idl/           # WIT lowering pass on top of schema-idl
│   ├── leo4c/              # CLI: parse / canonical / mangle / lower
│   ├── leo4-abi/           # LeanMarshal + LeanError + scalars / composites /
│   │                       # bignat / bigint / LeanRat / LeanU128/I128 /
│   │                       # LeanComplexF{32,64}x2 (+ optional nightly floats);
│   │                       # rust_exports module under `rust-exports` feature
│   ├── leo4-mslean4/        # native loader (libloading) + Arena + LeanRef
│   ├── leo4-macros/        # user-facing proc-macros (leo4::import!,
│   │                       # leo4::export, derive LeanMarshal)
│   ├── leo4-macros-backend # macro expander (syn + quote)
│   ├── leo4-build/         # build.rs helper (LEO4_SHIM_SO, wire_rust_exports)
│   ├── leo4/               # top-level user façade
│   ├── leo4-wasm/          # (scaffold) wasm loader — see sibling/leo4-wasip3
│   ├── leo4-rust-bridge/   # Phase 9 dispatcher static archive
│   │                       # (libleo4_rust_bridge.a; cc-built from
│   │                       # shim/leo4_rust_bridge.c)
│   ├── leo4-rust-worker/   # Phase 9 worker harness binary
│   ├── leo4-rust-emit/     # Phase 9 emit CLI (IDL + handshake + Lean wrapper)
│   └── leo4-cli/           # `leo4` scaffold CLI (create / init)
├── lake/                   # Lake workspace (Lean side)
│   ├── Leo4/               # runtime library
│   │   ├── Leo4/Platform.lean
│   │   │                   # OS abstraction layer (dynlib ext, rpath, …)
│   │   └── Leo4/MathlibBridge/
│   │                       # opt-in 1-to-1 conversions Lean carriers ↔ Mathlib
│   ├── Leo4Plugin/         # Lake plugin exe (leo4plugin)
│   └── Leo4Rust/           # Phase 9 declarative-link package — two
│                           # extern_libs auto-link libleo4_rust_bridge.a
│                           # + the leanc-compiled glue shim into any
│                           # `lean_exe` that `require Leo4Rust`s it
├── sibling/                # non-workspace Cargo / Lake projects
│   ├── leo4-wasip3/        # stable Rust + wasm32-wasip2 + wasip3 v0.6
│   ├── leo4-lean4-parse/   # OX6 PEG-based Lean 4 parser (strict
│   │                       # superset of oxilean-parse v0.1.2)
│   ├── leo4-oxilean-build/ # OxiLean transpile path (uses leo4-lean4-parse
│   │                       # + leo4_translate; OX5-oxi env bootstrap)
│   └── mathlib-bridge-test/# Lake package verifying Mathlib bridges
├── docs/                   # Typst documentation suite + plans
│   ├── template/leo4-book.typ
│   ├── learning/{en,ko,ja,de}/main.typ
│   ├── implement-from-scratch/{en,ko,ja,de}/main.typ
│   └── windows-manual-test-plan.md   # C1 + C5 prelim audit + test matrix
├── ci/                     # Multi-version Lean matrix infra
│   ├── Dockerfile.lean-test
│   ├── entrypoint.sh
│   └── matrix.sh
├── shim/                   # C shims
│   ├── leo4_rust_bridge.c       # dispatcher (Phase 9-4a/4b/4c)
│   └── leo4_rust_bridge_lean.c  # lean.h glue (Phase 9-6)
├── examples/               # end-to-end demos
│   ├── 01-hello/           # scalars + nominal + Rat + async + ...
│   ├── 02-roundtrip/       # list<T> + bignat + multi-instantiation
│   ├── 04-mutual-ast/      # Expr / Stmt mutual cluster
│   └── 05-rust-export/     # Phase 9 reverse-direction mini-solver
├── tests/                  # integration + conformance tests
│   ├── sample-lean/        # smoke fixture covering every emitted shape
│   ├── mangling/           # cross-impl mangling harness (Phase 2)
│   ├── wit/                # WIT lowering golden + wasm-tools validation
│   └── conformance/        # Lean ↔ Rust encoder byte-for-byte conformance
├── spike/                  # disposable experiments + findings
├── Cargo.toml
├── lakefile.lean
├── rust-toolchain.toml
├── lean-toolchain          # pinned: leanprover/lean4:v4.29.1
└── justfile
```

## Build and smoke-test

Lean toolchain pinned to **`leanprover/lean4:v4.29.1`**. The repo does
not require `elan` on the host; the system-installed Lean of that
version works. (The CI matrix container uses `elan` internally so it
can switch between matrix versions.)

### Cloning — submodules required

leo4 vendors its OxiLean fork (branch `0.1.3-leo4-ox7`) as a git
submodule at `sibling/oxilean/`. The rust-transpile path
(`--impl rust-transpile`), the `leo4-oxilean-build` CLI, and
the `leo4-oxilean` adapter all path-dep into that submodule's
crates, so a plain `git clone` without submodule init produces
an empty `sibling/oxilean/` directory and Cargo will refuse
the build with `couldn't read … Cargo.toml` errors.

Two safe initial-clone forms:

```bash
git clone --recursive https://github.com/<owner>/leo4
# or, post-clone, in an existing checkout:
git submodule update --init --recursive
```

Subsequent pulls that bump submodule references need
`git submodule update --init --recursive` again, or the fork
will track an outdated commit. The CI workflow uses
`actions/checkout@v4` with `submodules: recursive` so CI
matches a fresh `--recursive` clone.

Common recipes (run from repo root):

```bash
just                    # list recipes
just plugin-build       # build the Lake plugin (and Leo4)
just sample-build       # build tests/sample-lean
just smoke-plugin       # run leo4plugin against the sample, emit
                        # leo4_sample.{leo4-schema,leo4-mangling,leo4-handshake}
just smoke-plugin-with-wit  # also emit leo4_sample.wit via `leo4c lower`
just schema-hash        # print the sample's resolved schema hash
just clean              # nuke build outputs

# Cross-impl harnesses:
just mangling-test      # Lake plugin vs leo4c mangling (Phase 2)
just wit-test           # WIT golden + wasm-tools/wit-bindgen (Phase 3)
just conformance-test   # Lean encoder vs Rust encoder bytes (Phase 4)
just test               # full ladder = lake + cargo + mangling + wit + conformance

# Forward-direction end-to-end demos:
just smoke-plugin                          # produce / refresh shim .so
cargo run -p leo4-example-01-hello         # scalars, nominal types, Rat, async, handshake check
cargo run -p leo4-example-02-roundtrip     # list<T> + bignat round-trip
cargo run -p leo4-example-04-mutual-ast    # mutual Expr / Stmt cluster

# Reverse-direction (Phase 9) end-to-end demo:
just rust-export-05-build                  # cargo + emit + glue + lake build for examples/05
                                           # (manual leanc -o link line in
                                           # examples/05-rust-export/README.md)

# Reverse-direction helpers (parameterised):
just rust-bridge-build                                    # build all 3 cargo artefacts
just rust-emit CDYLIB OUT_DIR MODULE                      # emit IDL + handshake + Lean wrapper
just glue-shim-build OUT_OBJ                              # leanc -c the Lean glue shim

# Sibling tests (off the default ladder):
just mathlib-bridge-test                   # type-checks Mathlib bridges (1-2h cold)

# Project scaffolding (leo4 CLI; refactored 2026-05-24):
cargo install --path crates/leo4-cli       # install `leo4` binary on PATH
leo4 create forward my-app                 # new project (Lean exports + Rust caller)
leo4 create reverse my-solver              # new project (Rust cdylib + Lean caller)
leo4 create forward sub --subcrate         # scaffold as a subcrate of the current
                                           # Cargo workspace (auto-registers in `members`)
leo4 init forward                          # add leo4 to existing Cargo crate (cwd)
leo4 init reverse --dir path/to/crate      # same, with explicit dir
                                           # auto-migrates legacy `.leo4-impl` marker
                                           # to `leo4.toml` if found
leo4 run                                   # build + run end-to-end (impl resolved
                                           # from `leo4.toml`; --impl <kind> selects
                                           # when multiple [[impl]] entries listed)

# Multi-version Lean matrix (containerised):
just ci-image           # build the container image once
just ci-matrix          # run `just test` for v4.27/v4.28/v4.29.1/v4.30.0-rc2
just ci-version v4.29.1 # one version
just ci-clean-cache     # drop the matrix cache volume
```

After `just smoke-plugin` the emitted files describe `tests/sample-lean`'s
IDL in canonical form:

```
$ cat tests/sample-lean/.lake/build/leo4/leo4_sample.leo4-schema
package leo4-sample;
interface Sample {
  enum Sample.Color { red, green, blue };
  variant Sample.Either<T0, T1> { left(T0), right(T1) };
  record Sample.Pair<T0, T1> { fst: T0, snd: T1 };
  record Sample.Point { x: f64, y: f64 };
  variant Sample.Tree { leaf, node(Self, Self) };
  resource Sample.ParserHandle;
  func add(_0: u64, _1: u64) -> u64;
  func parserId(_0: Sample.ParserHandle) -> Sample.ParserHandle;
  func pointSum(_0: Sample.Point) -> f64;
  ...
}

$ just schema-hash
qi5gb74dbjyxo
```

Schema hashes rotate every time the canonical IDL form changes — by
design, so a stale Rust binary linking against a fresh shim fails at
link time. The exact value above is for the current sample fixture;
yours will rotate as soon as you edit `tests/sample-lean`.

### Running CI under outage / on a fresh host

`just ci-matrix` is hermetic: only `docker` (or `podman` with the
`docker` alias) is required on the host. The container image carries
`elan`, `rustup`, `just`, `wasm-tools`, and `wit-bindgen`; it stamps
the matrix `LEAN_VERSION` into every `lean-toolchain` file at run time
and lets `elan` install the matching toolchain on demand.

A single named volume `leo4-matrix-cache` survives between runs and
contains:

- `/cache/src/` — master source mirror (rsync'd from `/workspace`
  on each run).
- `/cache/work-<ver>/` — per-version work tree, populated as a
  hardlink mirror of `/cache/src/` so unchanged source files are
  shared on disk. `target/` and `.lake/` live here per-version and
  persist across matrix runs.

The matrix is a fallback path against GitHub-Actions outages — the
same `just test` ladder runs locally, and any divergence between
local-container output and GitHub Actions output is a real bug.

### Platform tiers (2026-05-20)

| Tier | Platforms | Guarantee |
|------|-----------|-----------|
| 1    | x86_64-unknown-linux-gnu                                                        | every commit, every matrix entry must pass |
| 1+   | `*-linux-musl*` for **no-mslean4-no-lake** paths (rust-transpile / scaffold / pure-Rust) | feature parity within the path scope; v1.0 RC mandatory (C5, locked 2026-05-24) |
| 2    | x86_64-pc-windows-**gnullvm**                                                   | feature parity, periodic CI (clang + lld + UCRT toolchain, see `LEO4-DESIGN.md §9.1`) |
| 2    | `*-linux-android*` for the same no-mslean4-no-lake scope                        | C6, deferred to v1.x |
| 3    | macOS (Apple Silicon / Intel)                                                   | best-effort; not gating, no CI |

The mslean4 runtime path is glibc-only because Lean ships
`libleanshared` linked against glibc; a musl process
cannot dlopen it across the ABI boundary. The
rust-transpile (OxiLean) path has no such constraint —
the entire pipeline is pure Rust + the `oxilean-kernel`
cargo dep.

macOS dropped from Tier 1 to Tier 3 on 2026-05-20 — see
`LEO4-DESIGN.md §9.1` for rationale. The code paths remain
platform-agnostic; only the test/exit-criteria scope shrunk.

See `OS-PORTABILITY.md` §0.1 for the per-distro musl
toolchain setup matrix.

## License

MIT OR Apache-2.0.
