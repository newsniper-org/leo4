# Changelog

All notable changes to leo4 will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to Semantic Versioning once it reaches 0.1.0.

## [Unreleased]

### Added — Phase 5 (2026-05-16 → 2026-05-20)

- **`crates/leo4-native/`** — `Lean::open` (handshake + Lean runtime
  init + wrapper-module init), `Arena<'a>`, `LeanRef<'a, T>`, a
  per-callsite `Mutex<HashMap>` dispatch cache, inline
  `lean_io_result_is_ok` and `lean_dec_ref` (with `lean_dec_ref_cold`
  resolved via `dlsym`).
- **`crates/leo4-macros/`** — `leo4::import! { fn add(a: u64, b: u64) -> u64; }`
  expands to the dispatch wrapper. Scalars (P5-b₁), `String` and
  composite payloads (P5-b₂), nominal user types (P5-b₃-iii), and
  per-fn `#[leo4(args = "…")]` attribute hints for
  multi-instantiation disambiguation (P5-b₃-iv) all flow through
  `LeanMarshal` encode / decode.
- **`#[derive(LeanMarshal)]`** for the four nominal shapes —
  record (struct), enum (all-unit), variant (mixed payload),
  resource (`#[leo4(resource)]` on a single-`u64` struct). Generic
  records (`Pair<A, B>`) get a `T: LeanMarshal` bound synthesised on
  the generated impl. Mirrors `lake/Leo4/Leo4/Deriving.lean`
  byte-identically.
- **`crates/leo4-build/`** — `leo4_build::wire(lake_build_dir)`
  surfaces `LEO4_SHIM_SO` and `LEO4_HANDSHAKE_FILE` as `env!()`
  values for downstream `build.rs` callers and emits the
  matching `cargo:rerun-if-changed=` lines.
- **`crates/leo4/`** — top-level façade re-exporting the loader, the
  derive macro, `import!`, and `encode` / `decode` helpers.
- **Lake plugin shim emitter** — `<pkg>.leo4-shim.c` with one
  `leo4_call_<mangled>` entry point per `@[leo4_export]` ×
  instantiation. `leanc`-driven build of `<pkg>.leo4-shim.so` with
  RPATH-linked `libleanshared` + dep `.so` discovery via
  `lake-manifest.json`'s `packages[].dir` (F4-α).
- **Lake plugin Lean-side ABI conformance** — the emitter now
  agrees with Lean's actual FFI signatures for unboxed types:
  all-nullary inductives get `uint8_t` / `uint16_t` / `uint32_t`
  according to `Lean/Compiler/LCNF/ToImpureType.lean`'s
  `impureTypeForEnum`; `≥ 2³²` ctors short-circuit through the
  `LEO4_ERR_UNIMPLEMENTED` stub. `@[leo4_resource]` single-`UInt64`
  structures pass / return a raw `uint64_t` (not `lean_object *`).
- **Deterministic wrapper-module init symbol** — Lean invoked with
  `--root=<wrapper-dir>` so the export module's `initialize_*`
  symbol is path-independent.
- **`examples/01-hello/`** — `add` / `hello` / `pointSum` /
  `colorName` / `isLeaf` / `parserId` round-trip end-to-end on
  Tier 1 Linux. Includes the handshake-mismatch exit check (mutated
  `schema_hash_bytes` → `LEO4_ERR_HANDSHAKE_MISMATCH` = 5).
- **Sample fixture** — `Sample.Pair<α, β>`, `Sample.Either<α, β>`,
  and the four nominal user types (`Point` / `Color` / `Tree` /
  `ParserHandle`).
- **macOS platform tier** — demoted to Tier 3 (best-effort, no CI)
  on 2026-05-20. Code paths remain platform-agnostic; only the
  exit-criteria and CI matrix scope shrunk.

### Added — earlier phases

- Design document (`LEO4-DESIGN.md`) capturing all resolved decisions.
- Working agreement for Claude Code (`CLAUDE.md`).
- Phased roadmap (`ROADMAP.md`).
- Phase 0 spike findings (`spike/SPIKE-0-FINDINGS.md`).
- Normative specifications under `SPEC/`:
  - `idl-grammar.ebnf`
  - `canonical-abi.md`
  - `mangling.md`
  - `handshake.md`
- Monorepo scaffold (Cargo workspace + Lake workspace).
- Lake plugin (`leo4plugin` exe), runtime library (`lake/Leo4/`),
  cross-impl mangling + WIT lowering + canonical-ABI conformance
  harnesses, and the multi-version Lean CI matrix.

### Pending

- `examples/02-roundtrip/` (P5-e) — list + Nat + value-param erasure.
  Last Phase 5 exit criterion.
<!-- P5-b₃-iv landed 2026-05-20 -->

- Phase 6 (mutual recursion), Phase 7 (async), Phase 8 (Mathlib
  subset).
- schema-idl items G (`ConstraintExpr<Atom>` typed AST) and H
  (mutual-recursion lift) per `schema-idl-shortcomings.md`.
