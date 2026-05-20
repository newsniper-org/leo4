# leo4

Lean 4 ↔ Rust interop that does not bind the Rust side to a Lean
toolchain version.

## Status

**Phases 1–4 complete. Phase 5 substantially landed (2026-05-20):
the C shim emitter, Rust loader, `leo4::import!` macro,
`#[derive(LeanMarshal)]`, `leo4-build` helper, and
`examples/01-hello/` are all green on Tier 1 (x86_64 Linux), including
the handshake-mismatch exit criterion. `examples/02-roundtrip/`
(list + Nat + value-param) is the remaining Phase 5 deliverable.**
See [`ROADMAP.md`](ROADMAP.md) for the full phase ladder.

What works today:

- **Lean side runtime** (`lake/Leo4/`) — `@[leo4_export]`,
  `@[leo4_specialize_when …]`, the `leo4_constraint` syntax category,
  `class LeanMarshal` (`canonicalEncode`/`canonicalDecode` over a
  `ByteArray`), `class LeanResource` + `@[leo4_resource]`, primitive
  blanket `LeanMarshal` instances, and a `deriving LeanMarshal`
  handler for `structure` / `inductive` (including self-recursive
  variants).
- **Lake plugin** (`lake/Leo4Plugin/`, `lean_exe leo4plugin`) — re-loads
  the user package via `Lean.importModules (loadExts := true)`,
  computes admit-sets (phantom / unbounded / class / value-erased /
  higher-kind reject — see `LEO4-DESIGN.md §5`), mangles names per
  [`SPEC/mangling.md`](SPEC/mangling.md), atomically writes
  `<pkg>.leo4-schema`, `<pkg>.leo4-mangling`, `<pkg>.leo4-handshake`.
  Optional `--with-lower` shells out to `leo4c` to also write
  `<pkg>.wit`.
- **Rust-side `leo4-idl` crate** — full IDL parser (`SPEC/idl-grammar.ebnf`:
  `package`/`use`/`interface`/`world`/`type`/`constraint_decl`/
  `nominal_decl`, kind annotations, `value_param`, `Self<…>`),
  byte-identical mangling + FNV-1a-64 schema hash, canonical-form
  renderer, and WIT lowering (records/variants/enums/resources,
  cyclic ADTs → opaque `resource`, `bigint`/`bignat`/`io<T>` → side
  aliases).
- **`leo4c` CLI** — `parse`, `canonical`, `mangle`, `lower`.
- **`leo4-abi` crate** — Rust mirror of `Leo4.LeanMarshal`. Scalars
  (`u8`..`i64`, `f32`/`f64`, `bool`, `char`), composites (`String`,
  `Vec<T>`, `Option<T>`, `Result<T,E>`, tuples), arbitrary-precision
  `BigNat`/`BigInt`. `LeanError` + the 8 reserved codes from
  `SPEC/canonical-abi.md` §13. `handshake::check_schema_hash` returns
  `0x05` on mismatch.
- **Cross-impl conformance** — three harnesses:
  - `tests/mangling/` (`just mangling-test`) — schema_hash + 50
    mangled names byte-identical between Lake plugin and `leo4c`.
  - `tests/wit/` (`just wit-test`) — 5 IDL cases lower to WIT that
    passes `wasm-tools component wit` and `wit-bindgen markdown`.
  - `tests/conformance/` (`just conformance-test`) — 29 fixtures
    where the Lean encoder bytes are reproduced byte-identical by the
    Rust encoder.
- **`tests/sample-lean/`** smoke fixture covering every shape the
  plugin emits.
- **Multi-version CI matrix** (`ci/`, `just ci-matrix`) — hermetic
  Ubuntu container with `elan`, `rustup`, `wasm-tools`, and
  `wit-bindgen`. Matrix `v4.27.0 / v4.28.0 / v4.29.1 / v4.30.0-rc2`
  iterates inside a single container; hardlink-shared source mirror
  in a named volume gives union-FS-style dedup without privileged
  mounts. **All 4 versions verified green** on the full `just test`
  ladder (mangling / wit / conformance / cargo / lake); each version
  produces the same `schema_hash` over the sample fixture
  (`7vi56qcxzb3xw`), confirming leo4's *Lean-version-independent
  output* invariant at the matrix level. First-run cold ~50 minutes
  (toolchain pull + cold builds); subsequent runs ~7 minutes with the
  persisted cache volume. GitHub Actions is the planned primary CI;
  Tart is the fallback path once Apple Silicon hardware is available.

Phase 5 — landed end-to-end on Tier 1:

- **C shim synthesis** — `lake/Leo4Plugin/Leo4Plugin/Main.lean`
  emits `<pkg>.leo4-shim.c`, drives `leanc` to produce
  `<pkg>.leo4-shim.so`, and links `libleanshared` + the user
  package's `.so` (resolved via `lake-manifest.json`'s
  `packages[].dir`) at the right RPATH so the loader picks them up.
- **`crates/leo4-native/`** — `Lean::open` (handshake + runtime init
  + wrapper-module init), `Arena<'a>`, `LeanRef<'a, T>`,
  per-callsite `Mutex<HashMap>` dispatch cache, inline
  `lean_io_result_is_ok` + `lean_dec_ref` (with `lean_dec_ref_cold`
  delegated via `dlsym` for the cold path).
- **`crates/leo4-macros/`** — `leo4::import! { fn add(a: u64, b: u64) -> u64; }`,
  plus `#[derive(LeanMarshal)]` for the four nominal shapes
  (record / all-unit enum / mixed-payload variant / `#[leo4(resource)]`
  single-`u64` struct). The macro reads the Lake-emitted mangling
  JSON and resolves each call in three tiers: (1) explicit
  `#[leo4(args = "…")]` attribute hint for multi-instantiation
  exports (P5-b₃-iv); (2) match by mangled arg list when every arg
  type lowers via `rust_type_to_idl`; (3) fname-only
  single-instantiation lookup when a parameter is a user-defined
  nominal type the macro can't lower syntactically.
- **`crates/leo4-build/`** — `leo4_build::wire(lake_build_dir)`
  emits `LEO4_SHIM_SO` / `LEO4_HANDSHAKE_FILE` env vars + the
  `cargo:rerun-if-changed=` lines.
- **`examples/01-hello/`** — `add(u64, u64) -> u64`, `hello() -> String`,
  four nominal user-type wrappers (`pointSum` / `colorName` /
  `isLeaf` / `parserId`), a derive-only Rust-side round-trip across
  record / enum / variant / resource / generic-record, and the
  handshake-mismatch exit check (`code == 5` on `schema_hash_bytes`
  tamper).

What is **not** built yet:

- `examples/02-roundtrip/` — `def echoes (xs : List u32) (n : Nat) : List u32`
  exercising list / Nat / value-param erasure. The remaining
  Phase 5 exit criterion.
- Phase 6 (mutual recursion), Phase 7 (async `io<T>` lowering), and
  the larger schema-idl items (G: `ConstraintExpr<Atom>` typed AST;
  H: mutual-recursion lift) — tracked in
  `schema-idl-shortcomings.md` and the γ-plan.
- Actual triggers for `LeanError` codes `0x02` / `0x03` / `0x04` /
  `0x06` / `0x08` beyond what the native shim path now exercises.

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
├── SPEC/                   # normative specs
├── crates/                 # Cargo workspace
│   ├── leo4-idl/           # IDL parser + mangle + canonical render + WIT lower
│   ├── leo4c/              # CLI: parse / canonical / mangle / lower
│   ├── leo4-abi/           # LeanMarshal + LeanError + scalars/composites/bignat/bigint
│   ├── leo4-native/        # (scaffold) Phase 5 — libloading + Arena/LeanRef
│   ├── leo4-macros/        # (scaffold) Phase 5 — #[leo4::import]
│   ├── leo4-macros-backend # (scaffold)
│   ├── leo4-build/         # (scaffold) Phase 5 — build.rs helper
│   ├── leo4/               # (scaffold) Phase 5 — top-level user API
│   └── leo4-wasm/          # (scaffold) Phase 7+
├── lake/                   # Lake workspace (Lean side)
│   ├── Leo4/               # runtime library
│   └── Leo4Plugin/         # Lake plugin exe (leo4plugin)
├── ci/                     # Multi-version Lean matrix infra
│   ├── Dockerfile.lean-test
│   ├── entrypoint.sh       # hardlink-shared source mirror + per-version work dir
│   └── matrix.sh           # single docker run wrapper
├── shim/                   # C shim for the native backend (Phase 5)
├── examples/               # end-to-end demos (Phase 5)
├── tests/                  # integration + conformance tests
│   ├── sample-lean/        # smoke fixture (record/enum/variant/resource/self-rec)
│   ├── mangling/           # cross-impl mangling harness (Phase 2)
│   ├── wit/                # WIT lowering golden + wasm-tools/wit-bindgen validation (Phase 3)
│   └── conformance/        # encoder byte-for-byte conformance (Phase 4)
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

# Phase 5 end-to-end demo:
just smoke-plugin                          # produce / refresh shim .so
cargo run -p leo4-example-01-hello         # call Lean from Rust

# Multi-version Lean matrix (containerised, Phase 5 prep):
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
4apuhe7gzvtzs
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
