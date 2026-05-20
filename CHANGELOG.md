# Changelog

All notable changes to leo4 will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to Semantic Versioning once it reaches 0.1.0.

## [Unreleased]

### Added — Phase 8 #55: stable 128-bit integers (`u128`, `i128`) (2026-05-20)

Wide-integer carriers via the "Lean record pairs Rust primitive"
pattern. Stable Rust path — no nightly, no feature gate.

- `lake/Leo4/Leo4/Wide.lean` — new module, auto-imported from
  `Leo4`. Defines
  `structure LeanU128 where lo, hi : UInt64 deriving LeanMarshal` and
  matching `LeanI128`. Wire form is `lo (8B LE) + hi (8B LE)` —
  byte-identical to Rust's `u128::to_le_bytes()` /
  `i128::to_le_bytes()`.
- `crates/leo4-abi/src/scalars.rs` — `impl LeanMarshal for u128 /
  i128` directly (no newtype wrapper). 16 wire bytes LE.
- `crates/leo4-macros-backend/src/lib.rs` — `rust_type_to_idl` maps
  bare `u128 → Record { fqn: "Leo4.LeanU128" }` and
  `i128 → Record { fqn: "Leo4.LeanI128" }`. Users write bare
  `u128` in `leo4::import!` and the macro auto-routes.
- Sample: `def addU128 (a b : Leo4.LeanU128) : Leo4.LeanU128` with
  hand-rolled two-limb add + carry (Lean stdlib has no UInt128 ops;
  leo4 stays Mathlib-independent per ROADMAP §8).
- examples/01-hello rt: `sample::addU128(&lean, (1u128 << 64) |
  0xdeadbeefcafebabe, 1)` asserts high limb participates.

Schema hash rotates `2iomjhrbofmos` → `uj55sds6f7cpq`. Cross-impl
mangling: **65 mangled names byte-identical**.

Zero plugin changes — the existing record path handles
`Leo4.LeanU128` natively because its wire is just `record { lo: u64,
hi: u64 }`. Same applies for `LeanI128`. Future carriers (#56, #57)
follow this pattern.

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
  `schema_hash_bytes` → `LEO4_ERR_HANDSHAKE_MISMATCH` = 5) and the
  attribute-routed `Sample.stringify` pick (P5-b₃-iv).
- **`examples/02-roundtrip/`** — `Sample.echoes (xs : List UInt32) (n : Nat) : List UInt32`
  driving `list<u32>` on both argument and return + `bignat` as an
  argument. Also exercises `listSumU64` / `listConcat` and a
  multi-instantiation `listLen` pick over `list<u32>`. Phase 5
  exit demo for the list + bignat wires.
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

### Added — Phase 8 step 2b: external-marshal shim glue, `Rat` round-trips (2026-05-20)

Phase 8 second-step finisher. Closes the cross-boundary wire for
`Rat` and any future external-marshal nominal: the shim now
delegates encode/decode to Lean-emitted C-callable helpers
(`leo4_marshal_<fqnSeg>_dec/_enc`) instead of dissecting the
type's IDL fields.

Concrete plumbing:

- `lake/Leo4Plugin/Leo4Plugin/Main.lean`:
  - `renderLeanExports` now takes `userDecls` and emits a helper
    pair per `UserDecl.externalMarshal`:

      `@[export leo4_marshal_<seg>_dec] def … (buf) (off) : Except _ (T × Nat) := …`
      `@[export leo4_marshal_<seg>_enc] def … (val) (buf) : ByteArray := …`

    These wrap `Leo4.LeanMarshal.canonicalDecode/Encode` so the
    typeclass call gets compiled to a fixed C-callable symbol.
  - `externalMarshalHandler` (new `TyHandler`) generates the
    decode/encode glue for each call site: build a fresh Lean
    `ByteArray` (`lean_alloc_sarray` + `leo4_memcpy` over the
    remaining wire bytes), invoke the helper, unwrap
    `Except _ (T × Nat)` at the right ctor tag (Lean's `Except`
    is `error` = 0, `ok` = 1 — got bitten in dev), advance the
    shim's offset by the consumed-bytes count returned in the
    pair. Encode mirrors with `lean_sarray_cptr` /
    `lean_sarray_size`.
  - `handlerFor`'s `.record` arm now also matches
    `UserDecl.externalMarshal` and routes to the new handler.
  - `renderShimSource` forward-declares every external-marshal
    helper at the top of the shim TU
    (`extern lean_object * leo4_marshal_<seg>_dec/_enc(…);`).

- `tests/sample-lean/Sample.lean`:
  `def addRat (a b : Rat) : Rat := a + b` re-enabled; now
  actually goes through the boundary instead of returning
  `LEO4_ERR_UNIMPLEMENTED`.

- `examples/01-hello/src/main.rs`:
  `sample::addRat(&lean, LeanRat::from_i64_u64(1, 3),
  LeanRat::from_i64_u64(1, 6))` returns
  `LeanRat { num: 1, den: 2 }` — Lean's `mkRat` normalises via
  gcd division.

Schema_hash rotates: `47swds7jpnqre` → `2iomjhrbofmos`. Cross-impl
mangling harness: **64 mangled names byte-identical**
(63 → 64; `addRat` joins). All four examples + workspace
`cargo test` pass.

Closes #54.

### Added — Phase 8 step 2a: UserDecl.ExternalMarshal AST + render (2026-05-20)

ROADMAP Phase 8 second landing (first half). Adds the IDL-level
recognition for types with custom `LeanMarshal` instances whose
fields the plugin can't lower (proof-carrying invariants like
`Rat`'s `den_nz` / `reduced`, opaque wrappers, …). Step 2b will
land the actual C-callable Lean helpers and shim glue so the
boundary round-trips end to end; this commit unblocks the IDL
form so `LeanMarshal Rat` being in scope no longer breaks
parsing / mangling.

- `lake/Leo4Plugin/Leo4Plugin/AdmitSet.lean`:
  `UserDecl.externalMarshal (fqn) (generics)` ctor. `walkUserDecl`
  falls back to `externalMarshal` when a record or variant body
  can't be lowered (any field's `exprToIDLSubst` returns `none`)
  AND the type carries a custom `LeanMarshal` instance.
- `lake/Leo4Plugin/Leo4Plugin/Mangling.lean`: `userDeclToIDL`
  renders `external <fqn>[<generics>]`. `declBand` puts the decl
  in the value band (0).
- Rust schema-idl mirror: `IDLType` reference layer unchanged
  (`func f(_0: Rat)` lowers to `IDLType::Record { fqn: "Rat" }`
  same as a regular record). The distinction lives at the
  `UserDecl` level; the shim emitter dispatches on it. New
  variant `UserDecl::ExternalMarshal { fqn, generics }` and
  matching `RawDecl::ExternalMarshal`.
- `SPEC/idl-grammar.ebnf` implicitly extended via the existing
  `nominal_decl` umbrella; parser recognises the `external`
  keyword in `parse_nominal_decl`, top-level docs, and
  interface bodies.
- `lake/Leo4/Leo4.lean` re-enables auto-import of
  `Leo4.MathlibSubset` (was opt-in in the prior commit because
  `Rat` exposure broke `walkUserDecl`; the externalMarshal
  fallback now closes that gap).
- WIT lowering routes external-marshal decls to opaque
  `resource <name>;` — wasm Component Model consumers see them
  as host-managed handles; the Lean-side custom marshal isn't
  visible across the wasm boundary.

Schema_hash rotates: `5wlbxlzagwuyy` → `47swds7jpnqre`. Cross-impl
mangling harness stays green at **63 mangled names byte-identical**
(`Sample.stringify` instantiation list now includes `Rat`).

Boundary round-trip for `Rat` still returns `LEO4_ERR_UNIMPLEMENTED`
— step 2b lands the actual wire-up.

### Added — Phase 7 step 1: future / stream effect desugar (2026-05-20)

ROADMAP Phase 7 first landing (parser-side, stable Rust, no
WASIp3 dep). `crates/schema-idl/src/parse.rs::parse_func_decl`
recognises `future<T>` / `stream<T>` at the immediate return
position of a `func_decl` and desugars them into
`FuncDecl { effect: Async / Stream, ret: T }`. The effect slot was
already in the AST (`FuncDecl.effect`, D-i 2026-05-19); this
landing wires the parser to populate it.

Rejection: `future<T>` / `stream<T>` anywhere *except* a func's
return position (inside `list<…>`, record fields, variant
payloads, etc.) raises a `ParseError::Expected` with a clear
message. `parse_type` checks the keyword and bails before
attempting to treat it as a nominal.

Renderer (`render::render_canonical`) re-wraps `effect = Async`
ret types in `future<…>` and `effect = Stream` in `stream<…>` so
`parse → resolve → render` round-trips. Schema-hash is unaffected
for existing (Sync-effect) funcs.

Four unit tests pin the desugar happy path (`future<u64>`,
`stream<u8>`), the in-payload rejection (`list<future<u32>>`),
and the in-record-field rejection (`record { y: stream<u32> }`).
schema-idl test count: 53 → 57.

The Lean-side mirror (plugin recognises `future<T>` / `stream<T>`
patterns in user exports — e.g., wrapping `IO T` exports as Async
effect) and the matching shim emit / macro expansion path are
Phase 7 step 2 (next).

### Added — WASIp3 sibling project skeleton (2026-05-20)

`sibling/leo4-wasip3/` — Cargo project **outside** the main leo4
workspace, pinning `nightly` Rust via its own
`rust-toolchain.toml` and targeting `wasm32-wasip3`. The skeleton:

- `Cargo.toml` declares the nightly toolchain + path-dep on
  `crates/leo4-abi` (the canonical-ABI marshalling layer is
  shared between native and wasm; only dispatch / loader differs).
- `src/lib.rs` carries a placeholder `pub struct Lean` mirroring
  the planned `leo4_native::Lean` surface so downstream code can
  `use leo4_wasip3::Lean` interchangeably under wasm.
- `sibling/README.md` documents the convention for non-workspace
  projects + the planned `leo4-wasm64/` sibling deferred until
  upstream stabilisation.

The WASIp3 API design crystallised in the same session: **sync API
on both targets** — the wasm side uses `futures::executor::block_on`
(or `wasmtime_wasi::block_on`) inside its sync wasm export to drive
async sub-tasks. WASIp3 explicitly avoids function coloring, so a
sync wasm export can call async imports. This means the
`leo4::import!` macro emits the same shape on both targets, no
per-target `cfg!` at the call site, and the `async-runtime`
feature stays opt-in for users who want native concurrency.

Phase 7 lands the concrete host import bindings, `block_on` choice,
and `Lean::open` equivalent for wasm; this commit is the
infrastructure shell so the wire-up has a home.

### Added — Phase 8 step 1: `Rat` LeanMarshal infrastructure (2026-05-20)

ROADMAP Phase 8 (Mathlib-compatible subset) first landing. Mirror
implementations on both sides for Lean-core `Rat`
(`Init.Data.Rat.Basic`):

- `lake/Leo4/Leo4/MathlibSubset.lean` — new module under `Leo4`
  carrying `instance : LeanMarshal Rat`. Wire format: `bigint num`
  followed by `bignat den` (SPEC/canonical-abi.md §§5-6). Decode
  uses Lean core's smart constructor `mkRat num den`, which
  normalises (divides by gcd) and degenerates to `0/1` when
  `den == 0`, so malformed wire payloads degrade rather than
  panicking.
- `crates/leo4-abi/src/rat.rs` — new module with `LeanRat { num:
  BigInt, den: BigNat }`. `LeanMarshal` impl delegates to the
  existing `BigInt` / `BigNat` impls so wire format stays
  byte-identical to the Lean side.
- `leo4` façade re-exports `LeanRat` + the `rat` module.
- Two unit tests pin the Rust-side round-trip for the basic cases
  (positive, negative, zero, max-magnitude) and a default-zero
  edge.

**Not yet wired**: cross-boundary calls. The plugin's `walkUserDecl`
rejects `Rat` because of its two `Prop`-typed proof fields
(`den_nz`, `reduced`), which prevents the shim from emitting a
real boundary entry for an `(a b : Rat) → Rat` export. Step 2 of
Phase 8 adds an "external marshal" path to the plugin so types
with custom `LeanMarshal` instances are treated as opaque blobs at
the shim and the boundary just routes bytes through.

### Changed — Variant discriminator 4-byte canonical (2026-05-20)

- Shim emitter (`lake/Leo4Plugin/Leo4Plugin/Main.lean`'s
  `renderVariantHelpers`) and Rust `#[derive(LeanMarshal)]`'s
  `expand_derive_variant` both flip variant disc encode/decode from
  `u8` to `u32 LE`. Matches `SPEC/canonical-abi.md` §9 ("encoders
  MUST emit 4 bytes"); the previous u8 fast path was a SPEC
  violation that both sides shared so the wire still round-tripped.
- Coordinated change — wire format is byte-incompatible with
  pre-2026-05-20 callers. Cross-impl mangling harness re-runs
  unchanged (62 names byte-identical, schema_hash unchanged because
  it covers the IDL canonical form, not the ABI wire).
- `examples/04-mutual-ast` round-trip bytes 12 → 24, matching the
  expected 4-byte-per-disc widening across 3 levels of nesting.

### Added — Plugin value-param erasure (2026-05-20)

- `analyzeExport` recognises implicit value-typed binders
  (`{N : Nat}` and friends) and erases them from the boundary
  signature per SPEC/mangling.md §"Value-param erasure". The
  wrapper renderer fills each erased binder with `default` at the
  Lean call site so elaboration succeeds (binder type must be
  `Inhabited`). `Sample.doubleVal {_N : Nat} (x : UInt32) : UInt32`
  fixture round-trips through `examples/01-hello` as a plain
  `(u32) -> u32` export.

### Added — Phase 6 (2026-05-20)

- **`SPEC/phase-6-mutual.md`** locks the four mutual-recursion
  design decisions: explicit `mutual { … }` block, `Cyc<i>`
  cycle-breaker token (0-based, scoped to the enclosing group),
  group-shared decode-depth counter, and a single `mutual ... end`
  block per cluster in the deriving handler.
- **`SPEC/idl-grammar.ebnf`** grows `mutual_decl` and `cyc_type`.
- **`crates/schema-idl`** — `IDLType::Cyc(u32)` + `UserDecl::Mutual {
  members }` + parser / renderer / mangler / resolver coverage; new
  diagnostics for out-of-scope / out-of-range Cyc and singleton
  groups.
- **`lake/Leo4Plugin`** mirrors the Rust schema-idl side: `IDLType.cyc`,
  `UserDecl.mutual`, `walkMutualGroup` that pre-detects `iv.all`
  clusters and rewrites peer references to `Cyc<i>`; shim emitter
  resolves `Cyc<i>` payloads to peer helper cross-calls
  (`leo4_enc_<peerSuffix>` / `leo4_dec_<peerSuffix>`) with forward
  declarations at the top of each helper block.
- **`lake/Leo4/Leo4/Deriving.lean`** detects a genuine mutual cluster
  (`iv.all` matches across members) and emits one `mutual ... end`
  block carrying every member's `partial def _leo4_encode /
  _leo4_decode` pair, then one `instance` per member. Cross-decl
  payload references route through direct function references
  rather than typeclass-instance forward references.
- **`tests/sample-lean/Sample.lean`** gains a `mutual inductive Expr
  / Stmt end` cluster with two exports (`exprIsLit`, `stmtIsNop`).
- **`examples/04-mutual-ast/`** — Rust mirror of `Sample.Expr` /
  `Sample.Stmt` with hand-rolled `LeanMarshal` impls (`Box<T>` breaks
  the Rust-side cycle). Exit demo for Phase 6.
- **`tests/mangling/`** cross-impl harness picks up the new cluster
  fixture; 61 mangled names + schema_hash `gj6daa3oelheu` are
  byte-identical between the Lake plugin and `leo4c`.

### Pending
- Phase 6 (mutual recursion), Phase 7 (async), Phase 8 (Mathlib
  subset).
- schema-idl items G (`ConstraintExpr<Atom>` typed AST) and H
  (mutual-recursion lift) per `schema-idl-shortcomings.md`.
