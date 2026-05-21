# leo4

Lean 4 ↔ Rust interop that does not bind the Rust side to a Lean
toolchain version.

## Status

**Phases 0–8 complete (2026-05-21). v0.1.0.** The full pipeline runs
end-to-end on Tier 1 (x86_64 Linux): Lake plugin, Rust workspace,
shim synthesis, C ↔ Lean dispatch, mutual recursion, async `io<T>`
lift, and a Mathlib-compatible carrier-type subset with opt-in
bridges. `tests/mangling/` confirms cross-impl byte-identical
mangling across **70 mangled instantiations** (29 logical entries)
with schema_hash `qi5gb74dbjyxo` between the Lean plugin and the
Rust `schema-idl` crate.

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
  `crates/leo4-native` (loader, `Arena<'a>`, `LeanRef<'a, T>`,
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

Open items:

- Some `LeanError` codes (`0x02` / `0x03` / `0x04` / `0x06` / `0x08`)
  are reserved but not yet exercised by a test fixture.
- schema-idl items G (`ConstraintExpr<Atom>` typed AST) and the
  `wasm64` sibling stay deferred until a concrete consumer surfaces.

## Documents to read, in order

1. [`LEO4-DESIGN.md`](LEO4-DESIGN.md) — every design decision and its
   rationale (D1–D15, type-system layer, admit-set algorithm,
   forbidden constructs).
2. [`CLAUDE.md`](CLAUDE.md) — working agreement for Claude Code
   sessions in this repo.
3. [`ROADMAP.md`](ROADMAP.md) — phased work plan, exit criteria per
   phase, the deferred IDL-output-grouping decision.
4. [`spike/SPIKE-0-FINDINGS.md`](spike/SPIKE-0-FINDINGS.md) — why the
   plugin re-imports `.olean` rather than hooking
   `Lake.Module.recBuildLean`.
5. `SPEC/*.md` — normative specifications:
   - [`SPEC/idl-grammar.ebnf`](SPEC/idl-grammar.ebnf) — IDL grammar
     (WIT-superset, `kind`, `Self`/`Self<…>`, `value_param`,
     `nominal_decl` short-form).
   - [`SPEC/canonical-abi.md`](SPEC/canonical-abi.md) — wire format.
   - [`SPEC/mangling.md`](SPEC/mangling.md) — name mangling, schema
     hash (FNV-1a-64 → base32lc), kind discipline.
   - [`SPEC/handshake.md`](SPEC/handshake.md) — JSON file formats,
     atomic-emission contract, `.leo4-schema` canonical-form rules.

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
│   └── phase-6-mutual.md
├── crates/                 # Cargo workspace
│   ├── schema-idl/         # parser + IDL types + mangling + canonical render
│   ├── leo4-idl/           # WIT lowering pass on top of schema-idl
│   ├── leo4c/              # CLI: parse / canonical / mangle / lower
│   ├── leo4-abi/           # LeanMarshal + LeanError + scalars / composites /
│   │                       # bignat / bigint / LeanRat / LeanU128/I128 /
│   │                       # LeanComplexF{32,64}x2 (+ optional nightly floats)
│   ├── leo4-native/        # native loader (libloading) + Arena + LeanRef
│   ├── leo4-macros/        # user-facing proc-macros (leo4::import!, derive)
│   ├── leo4-macros-backend # macro expander (syn + quote)
│   ├── leo4-build/         # build.rs helper (LEO4_SHIM_SO, ...)
│   ├── leo4/               # top-level user façade
│   └── leo4-wasm/          # (scaffold) wasm loader — see sibling/leo4-wasip3
├── lake/                   # Lake workspace (Lean side)
│   ├── Leo4/               # runtime library
│   │   └── Leo4/MathlibBridge/
│   │                       # opt-in 1-to-1 conversions Lean carriers ↔ Mathlib
│   └── Leo4Plugin/         # Lake plugin exe (leo4plugin)
├── sibling/                # non-workspace Cargo / Lake projects
│   ├── leo4-wasip3/        # stable Rust + wasm32-wasip2 + wasip3 v0.6
│   └── mathlib-bridge-test/# Lake package verifying Mathlib bridges
├── docs/                   # Typst documentation suite
│   ├── template/leo4-book.typ
│   ├── learning/{en,ko,ja,de}/main.typ
│   └── implement-from-scratch/{en,ko,ja,de}/main.typ
├── ci/                     # Multi-version Lean matrix infra
│   ├── Dockerfile.lean-test
│   ├── entrypoint.sh
│   └── matrix.sh
├── shim/                   # static C shim header shared with generated TUs
├── examples/               # end-to-end demos
│   ├── 01-hello/           # scalars + nominal + Rat + async + ...
│   ├── 02-roundtrip/       # list<T> + bignat + multi-instantiation
│   └── 04-mutual-ast/      # Expr / Stmt mutual cluster
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

# End-to-end demos:
just smoke-plugin                          # produce / refresh shim .so
cargo run -p leo4-example-01-hello         # scalars, nominal types, Rat, async, handshake check
cargo run -p leo4-example-02-roundtrip     # list<T> + bignat round-trip
cargo run -p leo4-example-04-mutual-ast    # mutual Expr / Stmt cluster

# Sibling tests (off the default ladder):
just mathlib-bridge-test                   # type-checks Mathlib bridges (1-2h cold)

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
| 1    | x86_64 Linux (glibc)              | every commit, every matrix entry must pass |
| 2    | x86_64 Windows                    | feature parity, periodic CI |
| 3    | macOS (Apple Silicon / Intel)     | best-effort; not gating, no CI |

macOS dropped from Tier 1 to Tier 3 on 2026-05-20 — see
`LEO4-DESIGN.md §9.1` for rationale. The code paths remain
platform-agnostic; only the test/exit-criteria scope shrunk.

## License

MIT OR Apache-2.0.
