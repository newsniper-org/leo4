# leo4 — Roadmap

> Sequential phases. Each phase has a gate: do not advance until the
> exit criteria are satisfied. The roadmap is opinionated about *order*
> but not about *duration* — actual time per phase varies.

## Phase 0 — Lake Hook Spike — **DONE 2026-05-16**

See `spike/SPIKE-0-lake-hook.md` / `spike/SPIKE-0-FINDINGS.md`.

GREEN with architectural commitment: the plugin is a `lean_exe`
(`lake exe leo4plugin …`) that re-imports `.olean` via
`Lean.importModules`. We do **not** hook `Lake.Module.recBuildLean`
(LEO4-DESIGN.md §12 OQ#1 — resolved).

## Phase 1 — Lake plugin & Lean runtime library — **DONE 2026-05-16**

Lean side of the boundary, end to end:

- `lake/Leo4/` runtime library: `@[leo4_export]`, `leo4_constraint`
  syntax category, `@[leo4_specialize_when …]` parametric attribute,
  `class LeanMarshal`, `class LeanResource`, `@[leo4_resource]`,
  primitive blanket `LeanMarshal` instances, `deriving LeanMarshal`
  handler for structures and inductives.
- `lake/Leo4Plugin/` Lake plugin:
  - imports the user package via `Lean.importModules (loadExts := true)`
  - walks `@[leo4_export]` decls via `ext.getModuleEntries` (sub-ms)
  - extracts admit-sets per generic — phantom skip, value-param erasure,
    class-constraint enumeration, unbounded default; HKT and `Self<…>`
    syntax shapes covered in spec (Phase 4+ enforcement)
  - mangles names per `SPEC/mangling.md` (FNV-1a-64 → base32lc 13 chars,
    parameter-types-in-symbol, FQN dot→underscore, `self` token)
  - emits `<pkg>.leo4-schema` (canonical IDL text), `<pkg>.leo4-mangling`
    (JSON: generic_args + param_types with `uses_generics`), and
    `<pkg>.leo4-handshake` (JSON: schema hash + constraint universe)
- `tests/sample-lean/`: fixture covering primitives, scalar generics,
  class-constraint generics, phantom, unbounded, user record / enum /
  variant / resource, self-recursive variant.

**Exit criteria (met):** `just smoke-plugin` is green; emits all three
files with correct shape per `SPEC/handshake.md`.

## Phase 2 — Rust-side `leo4-idl` and cross-impl mangling conformance

Build the Rust counterpart of the Lean plugin's IDL + mangling layer so
both sides can independently produce the same mangled symbols from the
same IDL source.

**Deliverables:**

- `crates/leo4-idl/`:
  - IDL parser producing an AST matching `SPEC/idl-grammar.ebnf`
    (including `kind`, `Self<…>`, `value_param`, generic
    record/variant/resource).
  - `mangle_type` and `mangle` functions byte-for-byte identical to
    `Leo4Plugin.Mangling` — FNV-1a-64 (`0xcbf29ce484222325` /
    `0x100000001b3`), big-endian 8 bytes, RFC 4648 lowercase base32
    no padding.
  - Kind discipline checker per `SPEC/mangling.md §4` (rejects
    ill-kinded declarations).
  - Normalised-IDL serialiser (comments stripped, whitespace collapsed,
    sort rules per `SPEC/mangling.md §3`).
- `crates/leo4c/`: CLI binary —
  - `leo4c parse <file.leo4>` prints AST
  - `leo4c mangle <file.leo4>` prints the mangling table as JSON
    matching `<pkg>.leo4-mangling`
  - `leo4c canonical <file.leo4>` prints the canonical form used as
    hash input
- `tests/mangling/`: cross-impl conformance harness.
  - `cases/*.leo4` — hand-written IDL fragments covering every
    Worked Example in `SPEC/mangling.md` §4 plus regression cases for
    Self, HKT, FQN clashes.
  - `expected/*.json` — expected mangling tables (committed; treat as
    a golden file diff target).
  - `tests/mangling/run.sh`:
    1. Run the Lake plugin against the `cases/` IDL through a fixture
       Lean library that re-emits each case verbatim.
    2. Run `leo4c mangle` on the same `cases/`.
    3. `diff` outputs against `expected/`. Any divergence fails.

**Exit criteria:**

- Every `cases/*.leo4` produces byte-identical mangling JSON on both
  sides.
- `tests/mangling/run.sh` is wired into `just mangling-test` and
  passes.
- Mutating any field in `cases/` rotates the `schema_hash` on both
  sides identically.

**Dependencies:** none beyond Phase 1.

## Phase 3 — WIT lowering pass (`.wit` emit)

Produce a `<pkg>.wit` file alongside `.leo4-schema` so the IDL can be
consumed by wasmtime, jco, and other WIT tooling.

**Deliverables:**

- `crates/leo4-idl::wit` (lives inside the IDL crate, not a separate
  crate): pure-function lowering from a fully-resolved `Schema` to
  WIT text. Rules implemented for v0:
  - leo4 `i{8,16,32,64}` → WIT `s{8,16,32,64}`.
  - `bignat` → synthetic alias `bignat = list<u64>`.
  - `bigint` → synthetic alias `bigint = tuple<bool, list<u64>>`.
  - `io<T>` → `result<T, error>` plus a synthetic `error` record alias.
  - Cyclic ADTs (record/variant whose body mentions `Self` / `Self<…>`)
    → WIT opaque `resource <name>;` (LEO4-DESIGN.md §4.1). The wire
    contract on the Lean side is unchanged; the resource here is only
    the WASM-side surface.
  - Generic functions on the Lake-plugin side are already
    *monomorphised* before they reach WIT lowering; overloaded names
    are disambiguated with a kebab-cased mangle-type suffix.
  - Dotted FQN + camelCase identifiers → kebab-case for WIT.
- `crates/leo4c lower <file>` — CLI subcommand.
- `tests/wit/`: golden tests + validation via `wasm-tools component
  wit` (always) and `wit-bindgen markdown` (when on PATH).
- A default WIT `world` is emitted so wit-bindgen can consume the file
  end-to-end.

**Exit criteria (met):**

- ≥ 5 IDL cases lower to `.wit` that pass `wasm-tools component wit`
  and `wit-bindgen markdown`.
- Lowering is byte-deterministic (sorted decls + sorted side-aliases).
- Self-recursive types lower to `resource` rather than failing
  validation.

**Lake-side `<pkg>.wit` emit (opt-in):**

`lake exe leo4plugin <module> [<outDir>] [<pkg>] [<iface>] --with-lower`
shells out to `leo4c lower` after writing the canonical artefacts and
saves the result as `<pkg>.wit` in the same `outDir`. The flag is
*opt-in* precisely to preserve D8's Lake-then-Cargo order — without it,
`lake exe leo4plugin` has no Cargo dependency and works on a fresh
checkout. With it, the user is expected to have already built leo4c
(e.g. via `just cargo-build` or `just smoke-plugin-with-wit`). If
`leo4c` is missing from `PATH`, the plugin writes a stderr warning
and exits 0 — the schema/mangling/handshake artefacts remain
authoritative; only the optional `<pkg>.wit` is skipped.

Promote to *always-on* Lake-side emit later if/when the WASM backend
phase (Phase 7) needs it: at that point either port the lowering to
Lean (the natural step) or keep the shell-out and document the build
dependency explicitly.

**Open question deferred to Phase 3 design time:** how to surface
generic functions in WIT — direct monomorphised names (`bucketize_u8`),
or a sub-`interface` per type-arg combination. Picked the
*monomorphised-name* path; the plugin's lowering uses
`func-<param-mangle>` for overloaded names. Revisit if Phase 7
introduces a use case that prefers grouped interfaces.

**Dependencies:** Phase 2 (`leo4-idl` parser).

## Phase 4 — Runtime error paths

Wire the `LeanError` codes defined in `SPEC/canonical-abi.md` §13 and
`lake/Leo4/Leo4/Marshal.lean` into actually-exercised round-trip code
paths.

**Deliverables:**

- `crates/leo4-abi/`: Rust counterpart to Lean's `LeanMarshal`.
  - Scalar encode/decode (mirroring `lake/Leo4/Leo4/Builtins.lean`).
  - Error types matching `LeanError` codes.
- `crates/leo4-mslean4/` (skeleton): just enough to host one shim and
  call into it through a mock symbol.
- A minimal "handshake-only" example:
  - `examples/handshake/` — Lean export side that returns
    `LeanError.handshakeMismatch` (0x05) when the schema hash is
    deliberately tampered with by truncating the handshake JSON before
    load.
  - Rust caller side that decodes the error and surfaces it.
- Round-trip property test: for each primitive, every value in a
  representative sample encodes through the Lean encoder, gets read by
  the Rust decoder, and vice versa, with identical bytes.

**Exit criteria:**

- Each reserved error code in `LeanError.*` has at least one test that
  *triggers* it (decode-malformed, buffer-too-small, depth-exceeded,
  etc.).
- `tests/conformance/` exists, every primitive round-trips both
  directions, `just test` runs them.
- Hand-mutating the handshake JSON's `schema_hash` causes the Rust
  side to refuse the call with `0x0000_0005`.

**Dependencies:** Phase 2 (mangling agreement) — without it, Rust
can't address the right symbols. Phase 3 not strictly required (we
can use leo4 IDL natively for this phase), but if Phase 3 lands first
we share `.wit` for component-model packaging.

## Phase 5 prep — CI matrix infra — **DONE 2026-05-16**

Multi-version Lean matrix in a hermetic Ubuntu container, mounted as a
fallback against GitHub Actions outages. Versions `v4.27.0 / v4.28.0 /
v4.29.1 / v4.30.0-rc2` iterate in a single container; one named volume
(`leo4-matrix-cache`) holds a master source mirror plus per-version
work trees populated via `rsync --link-dest=` (hardlink dedup, no
privileged mounts).

**Results:** all four versions green on `just test`
(mangling + wit + conformance + cargo + lake); identical
`schema_hash` (`7vi56qcxzb3xw`) across versions; cold ~50 min, warm
~7 min via cache persistence.

**Cross-version drift caught during W7-0a:** v4.30.0-rc2 now requires
`Lean.enableInitializersExecution` before `Lean.importModules
(loadExts := true)`. The function is present back to v4.27.0 (no-op
when called too early), so the plugin calls it unconditionally just
before `importModules`. This is exactly the kind of API tightening
the matrix exists to catch. `spike/SPIKE-0-FINDINGS.md` Q4 has been
extended with the v4.30 data point.

## Phase 5 — ABI shim synthesis + `cc`/`leanc` driving — **DONE 2026-05-20**

The C shim that bridges the canonical-ABI wire format to Lean's
native ABI (lean.h symbols), built from the Lake plugin and linked
into a shared library that `crates/leo4-mslean4/` loads via
`libloading`.

**Status (2026-05-20):** every deliverable below landed on Tier 1
Linux. Shim emitter, loader, `leo4::import!`,
`#[derive(LeanMarshal)]`, `leo4-build`, and both end-to-end
examples — `examples/01-hello/` (scalars + nominal user types +
handshake-mismatch ⇒ code 5) and `examples/02-roundtrip/`
(`list<u64>→u64`, `list<str>→str`, `list<u32> + bignat → list<u32>`,
generic `listLen`) — pass.

**Deliverables:**

- ✅ **Lake plugin emits `<pkg>.leo4-shim.c`** per CLAUDE.md "How to
  Work With the Lake Plugin", step 5–6.
  - Per `@[leo4_export]` instantiation, one `leo4_call_<mangled>`
    shim function with the exact mangled name from `SPEC/mangling.md`.
  - Each shim function: decode args via the canonical-ABI wire
    format, call the matching `leo4_lean__<mangled>` helper, encode
    return value back into the caller's buffer.
  - Shim depends on `lean.h` (the *only* place we touch it — Rust
    side stays free of `lean.h`).
  - The emitter matches Lean's actual FFI ABI for unboxed types
    (`uint8_t` / `uint16_t` / `uint32_t` for all-nullary inductives
    per `impureTypeForEnum`; raw `uint64_t` for single-`UInt64`
    resource structures), discovered 2026-05-20 while wiring the
    nominal-type wrappers.
- ✅ **Lake plugin drives `leanc`** to produce `<pkg>.leo4-shim.so`
  on Linux. `.dll` and `.dylib` not yet exercised (Tier 2 / 3).
  Links `libleanshared` + the user package's `.so` (resolved via
  `lake-manifest.json`'s `packages[].dir`) with the matching RPATH.
- ✅ **`crates/leo4-mslean4/`** — `Lean::open` (= `Lean::init` +
  handshake + wrapper-module init), `Arena<'a>`, `LeanRef<'a, T>`,
  per-callsite `Mutex<HashMap>` dispatch cache, inline
  `lean_io_result_is_ok` / `lean_dec_ref`, `lean_dec_ref_cold` via
  `dlsym`.
- ✅ **`crates/leo4-macros/`** — `leo4::import!` expands to the
  dispatch code; `#[derive(LeanMarshal)]` synthesises the
  canonical-ABI encode / decode for the four nominal shapes (record,
  all-unit enum, mixed-payload variant, single-`u64` resource).
  Generic records get a `T: LeanMarshal` bound on the generated
  impl. Multi-instantiation exports resolve in three tiers:
  explicit `#[leo4(args = "…")]` attribute hint (P5-b₃-iv), then
  by computed mangled arg list when every arg's `rust_type_to_idl`
  succeeds, then by fname-only single-instantiation lookup.
- ✅ **`crates/leo4-build/`** — `leo4_build::wire(lake_build_dir)`
  reads `<pkg>.leo4-handshake`, surfaces `LEO4_SHIM_SO` /
  `LEO4_HANDSHAKE_FILE` as `env!()` values, and emits the
  `cargo:rerun-if-changed=` lines.
- ✅ **`examples/01-hello/`** — `add` / `hello` / `pointSum` /
  `colorName` / `isLeaf` / `parserId` round-trip end-to-end +
  in-process derive round-trip across all four nominal shapes +
  handshake-mismatch detection (mutated `schema_hash_bytes` ⇒
  code 5).
- ✅ **`examples/02-roundtrip/`** —
  `def echoes (xs : List UInt32) (n : Nat) : List UInt32`, plus
  `listSumU64` / `listConcat` / multi-inst `listLen` calls. Covers
  `list<T>` on both arg and return, and `bignat` as an argument.
  The SPEC's value-param erasure (implicit `{N : Nat}` binders) is
  *not* yet wired through the plugin — left as a follow-up; the
  ROADMAP-level "value-param" phrasing here is satisfied by the
  explicit `n : Nat` runtime arg.

**Exit criteria:**

- ✅ `examples/01-hello/` runs end to end on Linux Tier 1.
- ✅ `examples/02-roundtrip/` runs end to end on Linux Tier 1.
  macOS was demoted to Tier 3 on 2026-05-20: builds may still
  produce a `.dylib`, but neither this exit criterion nor the CI
  matrix requires it.
- ✅ Handshake-mismatch detection lands in `examples/01-hello/`
  (the in-process test mutates `schema_hash_bytes` and expects
  `LeanError { code: 5 }`).
- ✅ `just test` runs all sides green on Linux.

**Dependencies:** Phase 2 (mangling), Phase 4 (error paths). Phase 3
(WIT) optional.

## Phase 6 — Mutual recursion between nominal types — **DONE 2026-05-20**

Lift the v0 ban (LEO4-DESIGN.md §4.3) on two nominal types referencing
each other. Today `Self`/`Self<…>` covers *direct* self-recursion
inside one declaration; cross-declaration cycles must be broken with a
`LeanResource` handle. Phase 6 makes them first-class.

**Design questions to settle at entry:**

- IDL syntax for mutual groups: a `mutual { record Foo { … }; variant Bar { … } }`
  bracket (Lean-style), or a flat list where the plugin discovers cycles
  via dependency analysis?
- Mangling: do mutually-recursive type names appear unchanged in each
  other's mangled forms (relying on the schema hash to prevent
  collisions), or do we introduce a dedicated cycle-breaker token like
  `Self<…>` already uses?
- Canonical ABI: does encode/decode keep the per-type `max_decode_depth`
  cap (`SPEC/canonical-abi.md` §8.1), or does it switch to a shared
  walked-set when the cycle is between distinct nominal types?
- `deriving LeanMarshal`: the v0 handler refuses mutual groups
  (`lake/Leo4/Leo4/Deriving.lean`); it must accept the new IDL form
  and emit one `partial def` per type with cross-call recursion.

**Deliverables:**

- `SPEC/idl-grammar.ebnf`: a `mutual_group` production wrapping a
  contiguous set of `type_decl`s that the kind-discipline checker
  treats as a single recursion frame.
- `SPEC/mangling.md`, `SPEC/canonical-abi.md`: rules for mutual
  groups; specifically the depth-cap interaction and the schema-hash
  treatment.
- `LEO4-DESIGN.md §4.3`: remove the "mutual recursion" forbidden
  entry and replace it with a reference to Phase 6's design.
- Lake plugin: `walkUserDecl` handles a `mutual` cluster atomically
  (walk all members, then emit). `deriving LeanMarshal` synthesises
  the cluster as one `mutual` block of `partial def`s.
- Rust side (`crates/leo4-idl`, `crates/leo4-abi`): mirror the mutual
  encode/decode contract.

**Exit criteria:**

- `examples/04-mutual-ast/` — a representative case (e.g. an `Expr` /
  `Stmt` pair) round-trips both directions.
- `tests/mangling/cases/mutual.leo4` proves cross-impl agreement on
  the new mangling rule.
- Self-recursive variants still work (Phase 1 regression).

**Dependencies:** Phase 5 (end-to-end pipeline). Touches Phase 2's
mangling contract — every mutually-recursive declaration will rotate
the schema hash on adoption.

## Phase 7 — Async IDL (`io<T>` lowering to `future<T>`) — **DONE 2026-05-21**

Lifted D4. The IDL now carries `future<T>` / `stream<T>` as
function-level effect modifiers (not `IDLType` variants), and `IO α`
Lean exports surface as `func foo(…) -> future<α>` in the canonical
IDL. The user-facing API stays **sync on both native and wasm**: the
WASIp3 sibling drives async wasip3 imports via
`futures::executor::block_on` inside a sync wasm export, so the
`leo4::import!` macro emits the same shape on both targets.

**Original entry gate** (resolved during landing): WASIp3 reaching
stable wasmtime was *not* required — the `wasip3` v0.6 crate
publishes API bindings as compatibility shims on wasip2's Component
Model, so a stable Rust toolchain + `wasm32-wasip2` target carries
the whole sibling.

**Landed in three steps:**

- **Step 1 (2026-05-20)** — schema-idl parser desugars
  `future<T>` / `stream<T>` at func-return position into
  `FuncDecl { effect: Async / Stream, ret: T }`. Rejection inside
  composite types. Renderer round-trips `Sync` / `Async` / `Stream`
  back to the surface form.
- **Step 2a–2b (2026-05-20 → 2026-05-21)** — Lean plugin recognises
  `IO α` in boundary positions
  (`exprToIDLSubst` lifts to `IDLType.io α`), renders as
  `future<α>`. Shim emits an IO-unwrap block: invoke the
  `leo4_lean__…` helper, check `lean_io_result_is_ok`, return
  `LEO4_ERR_IO_FAILED = 0x00010001` on failure, otherwise unbox
  and encode the inner value. Sample fixture `asyncDouble(21) =
  42` round-trips.
- **Step 2c (2026-05-21)** — `scalarUnbox` helper covers
  `lean_unbox` / `lean_unbox_uint32` / `lean_unbox_uint64` /
  `lean_unbox_float` / `lean_unbox_float32`; signed types share
  the matching unsigned width with a cast at the call site.
  Fixtures: `asyncNegate (Int32)`, `asyncHalveF64`,
  `asyncHalveF32` round-trip.

**Sibling project (`sibling/leo4-wasip3/`)** — stable Rust pinned
via `rust-toolchain.toml`, targets `wasm32-wasip2`, depends on
`wasip3` v0.6 + `futures` `executor` feature. Skeleton compiles;
`Lean::open` is a placeholder pending the concrete host-import WIT.

**Dependencies (met):** Phase 5 (sync runtime), Phase 3 (WIT
lowering).

## Phase 8 — Mathlib-compatible subset — **DONE 2026-05-21**

The hardest, longest-tailed phase. LEO4-DESIGN.md §11 marks Mathlib
usage as "likely never (subset only)"; Phase 8 narrows that to
"chosen subset, on demand, with a clear cost story". Nothing in the
phase is generic Mathlib support — Mathlib is too large and too
internally-recursive for that to be a sensible goal.

**Landed in 5 substeps + a follow-on bridge layer:**

- **Step 1 (2026-05-20)** — `Leo4.MathlibSubset` carrying
  `instance : LeanMarshal Rat` (wire: `bigint num + bignat den`),
  matching Rust `LeanRat { num: BigInt, den: BigNat }`.
- **Step 2a (2026-05-20)** — `UserDecl.ExternalMarshal` AST so the
  plugin recognises types whose fields it cannot lower
  (proof-carrying invariants like `Rat`'s `den_nz`, `reduced`).
- **Step 2b (2026-05-20)** — shim glue: Lean-emitted C-callable
  helpers `leo4_marshal_<seg>_dec/_enc` wrap the typeclass
  `canonicalDecode` / `canonicalEncode` so the shim never sees the
  type's internal layout. `Rat` round-trips end to end.
- **#55 (2026-05-20)** — `LeanU128` / `LeanI128` carriers
  (`{ lo, hi : UInt64 }`); Rust macro auto-routes bare `u128` /
  `i128` to the carrier record.
- **#56 (2026-05-20)** — `LeanComplexF{32,64}x2` machine-complex
  carriers. `xN` suffix extends to quaternion (`xN=4`) / octonion
  (`xN=8`) when needed.
- **#57 (2026-05-20)** — nightly-only `LeanF16` / `LeanBF16` /
  `LeanF128` + three complex variants behind the `nightly-floats`
  cargo feature. Stable builds unchanged.
- **Mathlib bridge layer (2026-05-21)** — each `Lean*` carrier
  ships an opt-in `Leo4.MathlibBridge.<Sub>` module providing
  1-to-1 conversions to / from Mathlib types (`Nat` / `Int` /
  `BitVec` / `ZMod (2^128)` / `ℝ` / `ℂ`). leo4 core stays
  Mathlib-independent — bridges live in their own modules,
  type-checked under `sibling/mathlib-bridge-test/`. Reverse
  paths (e.g. `ℝ → LeanF*`) pin **IEEE-754 round-to-nearest-even
  (RTNE)**; computable path goes via `Rat`, abstract `ℝ` path is
  `noncomputable`.

**Entry gate:**

- 병익 names the specific Mathlib types that need to cross the
  boundary (e.g. `Real`, `Complex`, `Polynomial R`, `Matrix R m n`).
  Anything outside that named subset stays out.
- For each named type, a clear answer to: "what does the Rust side
  *do* with this value once it has it?" If the answer is "compute
  back into Mathlib via a returned closure", the type is a
  `LeanResource`, not a `LeanMarshal`, and the phase is much
  smaller.

**Design questions to settle at entry:**

- Marshal vs. resource per Mathlib type. Most will be resources;
  `Real`/`Complex` plausibly resources (proof-carrying); `Rat` /
  bounded `Polynomial` plausibly marshalable as `bigint` pairs / lists.
- Whether Phase 8 ships Lean-side `LeanMarshal` instances for the
  chosen types, or whether each user writes their own (the plugin's
  admit-set is fine either way — it watches for instance presence,
  not who wrote it).
- License compatibility: Mathlib is Apache-2.0; leo4 is MIT OR
  Apache-2.0. Anything we *redistribute* (e.g. example code) needs
  the joint header.

**Deliverables (per named type):**

- A worked `LeanResource` or `LeanMarshal` instance, with handle
  lifetime documented and a `LeanRef<'a, T>`-friendly Rust shape.
- One `examples/` end-to-end demonstrating the chosen pattern.
- A regression test that pins both encode/decode behaviour and any
  proof-carrying invariants the type relies on.

**Non-deliverables:**

- General Mathlib reflection at the boundary.
- Bringing `mathlib` into `lean-toolchain` of the leo4 repo itself —
  user packages depend on Mathlib through their own `lakefile.lean`;
  leo4 stays Mathlib-independent.

**Exit criteria:**

- The named subset (typically 3–5 types) crosses the boundary in
  both directions, with documented performance characteristics.
- `crates/leo4/` doesn't gain a Mathlib dependency — the Rust side
  only sees the chosen wire shapes.

**Dependencies:** Phase 5 (end-to-end pipeline), Phase 6 (mutual
recursion — `Polynomial R` / similar may need it depending on the
chosen representation).

## Phase 9 — Reverse-direction boundary (Rust → Lean) — **CODE LANDED 2026-05-23**

Adopted 2026-05-21 (D16). leo4 grows a second pipeline so that
Rust functions tagged `#[leo4::export]` become callable from
Lean as ordinary `IO α` actions. Use case driver: combining a
Rust-implemented SMT solver (z3 / cvc5 / a research prototype)
with Lean's proof tooling, where the Lean side wants
incremental `push/pop`-style state preserved across calls.

**Normative SPEC**: `SPEC/reverse-direction.md` (drafted in this
phase entry-gate commit, 9-0).

**Architecture**:

```
Lean process
  │
  ├── libleo4_rust_bridge.a   (statically linked; single C TU,
  │                            C17 baseline, optional C23)
  │      │
  │      ▼  posix_spawn / CreateProcess on first call
  │   worker process          (one per cdylib; long-running by default;
  │      │                     loads the user cdylib via dlopen /
  │      │                     LoadLibrary; runs each request serially)
```

The dispatcher is **isolation-backend-neutral**: a single C
entry point `leo4_rust_call(mangled, args, ret)` lets us swap
the underlying worker model (long-running / zygote-fork / wasm
sandbox) without touching either the Lean wrapper or the Rust
macro.

**Isolation model — long-running worker with opt-in fresh
worker per call**:

| Threat | Default mode | `#[leo4::export(isolated)]` |
|---|---|---|
| T1 memory corruption | Worker is a separate OS process; cannot reach Lean memory. Cross-call accumulation inside the worker is the user's responsibility. | Worker `_exit`s after each call; fresh address space next time. |
| T2 Rust panic | `catch_unwind` + worker abort; dispatcher reports `LEO4_ERR_RUST_PANIC` and respawns lazily. Lean stack untouched. | Same. |
| T3 Thread leak | Threads stay in the worker; recycle policy (env-driven) bounds the lifetime. | Worker `_exit` reaps all threads. |

Build orchestration: Cargo emits `<pkg>.leo4-rust-exports.idl`
+ `<pkg>.leo4-rust-handshake` (via `leo4-build::wire_rust_exports`).
Lake reads them and generates `<pkg>.leo4-rust-imports.lean`. The
Rust cdylib is **not** in Lake's build graph — it loads at
runtime via the path-resolution chain
(`LEO4_RUST_CDYLIB` env → handshake `cdylib_path` → sibling
search).

**Substeps** (all DONE 2026-05-23 unless noted):

- **9-0 (design, 2026-05-21)** ✅ — ROADMAP entry, D16 in
  `LEO4-DESIGN.md`, `SPEC/reverse-direction.md`,
  `SPEC/canonical-abi.md` §13 extended with the
  `0x0002_xxxx` Rust-worker passthrough range.
- **9-1** ✅ — `#[leo4::export]` proc-macro in
  `crates/leo4-macros/`. Emits per-fn wrapper shim
  `leo4_rust__<body>` (canonical-ABI decode → call →
  encode) and registers the function via `linkme` distributed
  slice. Schema-hash suffix deliberately omitted from the
  wrapper symbol (lives in the handshake JSON only).
- **9-2** ✅ — `crates/leo4-rust-emit/` CLI walks the cdylib's
  `EXPORTS` slice via `leo4_rust_describe_exports` and writes
  `<pkg>.leo4-rust-exports.idl` + `<pkg>.leo4-rust-handshake`.
  `leo4-build::wire_rust_exports()` exposes the path env vars
  to consuming `build.rs`.
- **9-3** ✅ — `crates/leo4-rust-worker/` harness binary.
  Loads cdylib, recomputes schema_hash, sends handshake, runs
  the request loop. POSIX IPC via inherited `--ipc-fd`;
  Windows named-pipe path is the 9-4c follow-up here.
- **9-4a** ✅ — `shim/leo4_rust_bridge.c` skeleton:
  `leo4_worker_ops_t` table, dispatcher request loop,
  `_Atomic` worker slot, stub backend. Links on every
  platform from day 1.
- **9-4b** ✅ — POSIX backend in the same TU: `posix_spawn` +
  `socketpair(AF_UNIX, SOCK_STREAM)` + `waitpid`. Tier 1.
- **9-4c** ✅ — Windows backend in the same TU:
  `CreateProcessA` + `CreateNamedPipeA` +
  `WaitForSingleObject`. Compiles under the `*-pc-windows-gnullvm`
  clang target. Tier 2 runtime verification follows.
- **9-5** ✅ — `leo4-rust-emit --emit-lean` generates
  `<pkg>.leo4-rust-imports.lean` with one typed `IO α`
  wrapper per export + a single `@[extern
  "leo4_rust_call_lean"]` raw entry + a compile-time
  `schemaHash` pin.
- **9-6** ✅ — `shim/leo4_rust_bridge_lean.c` is the
  Lean-side glue shim (lean.h ↔ byte buffer). Sole leo4 C TU
  that includes `<lean/lean.h>`. Declarative Lake `extern_lib`
  integration landed via the new `lake/Leo4Rust/` package
  (commits 1/3, 2/3, 3/3 + runtime fix, 2026-05-23): two
  `extern_lib`s (`leo4RustBridge` resolves the cargo-built
  `.a`, `leo4RustBridgeLean` compiles + ar-wraps the glue
  shim, with `freshcheck` as an optional incremental gate)
  let `lean_exe`'s link line pick up both archives
  automatically. Glue shim's extern signature is
  `(@& String) (@& ByteArray) → IO ByteArray` with the first
  4 bytes carrying a LE u32 status (avoids the Lean
  `UInt32 × ByteArray` Prod inline-scalar ABI mismatch the
  initial design hit).
- **9-7** ✅ — `examples/05-rust-export/` mini-solver demo:
  4 `#[leo4::export]`s (`is_prime`, `next_prime`,
  `count_primes_below`, `factor_smallest`) called from Lean
  with correct values for every export (true e2e
  verified 2026-05-23).

**9.X follow-ups landed alongside (2026-05-23)**:

- ✅ **`#[leo4::export(isolated)]`** dispatcher path — the
  Lean wrapper prepends an `iso:` prefix to the mangled
  name; dispatcher detects it and routes through a per-call
  fresh-worker path (`posix_spawn` per call, `_exit` after).
  No wire-format / API change.
- ✅ **`LEO4_RUST_WORKER_RECYCLE_CALLS=N`** — after N
  completed calls the persistent worker is reaped + lazy
  respawned. Time-based recycle deferred.
- ✅ **`leo4` CLI** (`crates/leo4-cli/`) — `leo4 create
  <direction> <dir>` for new projects;
  `leo4 init <direction>` for in-place integration into an
  existing Cargo crate (idempotent Cargo.toml append +
  lean/ scaffold). `forward` / `reverse` directions.

**9.X candidates — promoted to Phase 10 or deferred (2026-05-21):**

- Callback / function-arrow ABI → **Phase 10-B1**.
- `LEO4_ERR_RUST_WORKER_RESTARTED` surfacing → **Phase 10-A5**.
- Time-based recycle (`LEO4_RUST_WORKER_RECYCLE_SECONDS`) →
  **Phase 10-A4**.
- Stronger isolation backends (zygote-fork, wasm sandbox) →
  deferred ≥ v1.x. Dispatcher's single-entry API preserves
  the swap option.
- Windows runtime verification (Tier 2 CI matrix) →
  deferred to v1.0 RC pre-release window. gnullvm code path
  already compiles clean.

**Dependencies**: Phase 4 (canonical ABI for marshal),
Phase 5 (forward pipeline as the reference). Does **not**
depend on Phase 7 (sync API on both sides; reverse direction
introduces no new async surface).

## Phase 10 — DX consolidation + callback ABI — **PLAN LOCKED 2026-05-21**

Phases 0–9 are done (Phase 9 code landed 2026-05-23, with
declarative Lake-DSL integration as the one residual
follow-up). Phase 10 is a **locked, ordered sequence** of
small commits; each substep ships in one commit unless noted.

The intent is to round out leo4 to a v0.2.0-cuttable state:
DX gaps, SPEC compliance for reserved error codes, the one
new ABI surface (callbacks) that the **adsmt** flagship
demo needs, and a docs sweep. Larger isolation /
backend-swap work, Windows runtime CI, and crates.io publish
all wait for the v1.0 RC window or later.

**Locked substep order:**

- **P10-D1** — `leo4 run` CLI: forward + reverse build +
  env wiring + execute as one command. Eliminates the
  manual `cargo build && lake build && leanc -o && ./bin`
  ladder from each scaffold's README.
- **P10-F1** — Reserved `LeanError` code fixtures:
  trigger 0x02 / 0x03 / 0x04 / 0x06 / 0x08 in
  `tests/conformance/`. Closes Phase 4 exit criterion that
  shipped partial-coverage.
- **P10-B1** — Callback / function-arrow ABI. Adds
  function-pointer mangling (`SPEC/mangling.md` §3 TBD slot)
  + re-entrant dispatcher path. Unblocks the **adsmt**
  flagship integration's `push/pop` + sub-formula inquiry
  pattern. Schema hash will rotate.
- **P10-D2** — Lake-side `leo4-rust-emit` auto-call:
  reverse-direction build collapses from 2 commands to 1.
  `cargo build && lake build` is enough; lake invokes
  `leo4-rust-emit` transparently when the cdylib changes.
- **P10-B5** — Variant payload widening
  (schema-idl-shortcomings #12 W7-2d-iii): multi-field /
  composite-payload variants in the plugin emitter. Schema
  hash will rotate for any variant in user code that picks
  the new shape.
- **P10-A4 + A5** — `LEO4_RUST_WORKER_RECYCLE_SECONDS`
  time-based recycle + `LEO4_ERR_RUST_WORKER_RESTARTED`
  side-channel surfacing on recycle. Both currently
  reserved in SPEC §13 but unwired.
- **P10-C4** — `leo4-wasm` proper implementation
  (out of the scaffold-only state it has shipped in since
  Phase 5). Native-equivalent surface via wasmtime;
  WASIp3-sibling stays where it is.
- **P10-Docs** (single commit, E1+E2+E3) — Typst books'
  Phase 9 chapter in all four implementation languages,
  reverse-direction byte-parity harness under
  `tests/conformance/reverse/`, and a SPEC quickstart page
  alongside `SPEC/reverse-direction.md`.

**Deferred to the v1.0 RC pre-release window**:

- **C1** Windows runtime CI matrix. Code compiles clean
  against `*-pc-windows-gnullvm`; runtime verification
  waits for CI infra.
- **G2** Publish to crates.io. API surface stabilises
  through Phase 10 first.
- **OX1** `leo4-oxilean-build` invocation wiring (locked
  2026-05-22; step a landed 2026-05-22).

  **Step a (DONE 2026-05-22)** — `leo4-oxilean-build` CLI
  binary exists at `sibling/leo4-oxilean-build/src/bin/`.
  Reads a line-oriented manifest from `--manifest <path>`
  or stdin, walks every `source=<lean> <mangled>` line,
  drives `transpile_source_to_unit` (new lib fn — superset
  of `transpile_source_if_exported` that assembles a
  `TranspileUnit` including the wrapper source), then
  `emit_crate` + `write_to_dir`. Exit codes 0/1/2 follow
  the standard success / transpile-failure / usage-error
  pattern. 6 integration tests in `tests/cli_smoke.rs`
  exercise help / missing-arg / skip-path / stdin-input /
  bogus-field / transpile-error.

  **Step b (DONE 2026-05-22)** — lake plugin
  (`lake/Leo4Plugin/Leo4Plugin/Main.lean`) gained four new
  flags (`--transpile <lean-file>`, `--transpile-out-dir <p>`,
  `--transpile-crate-name <n>`, `--transpile-abi-dep <s>`)
  and a `transpileSource`-driven branch that, after writing
  the canonical artefacts, builds a multi-decl
  `leo4-oxilean-build` manifest (one `source=<file>` line +
  per-export `bind=<decl_name>=<mangled>` lines) and shells
  out to `leo4-oxilean-build --manifest <path>`. The CLI
  produces the emitted Cargo crate at the configured
  out-dir.

  Wiring infrastructure is complete + tested end-to-end on
  the `tests/sample-lean/Sample.lean` fixture (manifest is
  produced with all 60+ instantiation mangled names).

  **OX3 (DONE 2026-05-22)** — Lean 4 header-binder syntax
  + attribute-arg strip. Two new textual pre-rewrites in
  `lean4_normalize`:

  - `rewrite_header_binders(src)`: lifts
    `def NAME (a b : T1) (c : T2) : R := body` into
    `def NAME : T1 → T1 → T2 → R := fun a b c → body`.
    Character-level scanner with bracket balancing.
    Implicit `{T : Type}` and instance `[Ord T]` binders are
    stripped from the head (they're auto-bound). Pass-through
    for `theorem` / `structure` / `inductive` / def's
    already without binders / def's inside strings or
    comments. Idempotent.
  - `strip_attribute_args(src)`: reduces
    `@[leo4_specialize_when scalar ∧ ord]` to
    `@[leo4_specialize_when]`. OxiLean's parser only
    accepts bare idents in `@[…]` lists, not arg-bearing
    attributes. UTF-8 safe (multi-byte chars preserved).

  E2E parse-pass verified — what was previously a parser-
  reject error on `tests/sample-lean/Sample.lean` now
  surfaces as elab-level errors instead (`NameNotFound`),
  meaning the surface-syntax layer is correctly cleared.

  **OX4 (PARTIAL 2026-05-22, multiple sub-rewrites landed)** —
  Lean 4 surface coverage tail. Three new textual pre-
  rewrites landed in `lean4_normalize`:

  - `strip_ctor_dot_shorthand`: `.lt` → `lt` after boundary
    chars (`|`, `(`, `,`, `=`, `[`, etc.); preserves
    `foo.field` projections + `..` ranges + UTF-8 + strings
    / comments.
  - `rewrite_inductive_where`: lifts Lean 4 `inductive
    NAME where | a | b` into OxiLean's `inductive NAME :
    Type | a : NAME | b : NAME`; preserves existing
    `: payload` annotations + block-exits on next top-level
    decl keyword.
  - `strip_deriving_clause`: removes `deriving Foo, Bar`
    lines (leo4-oxilean-build synthesises the LeanMarshal
    impls itself via the OX2 user-records path).

  Also fixed: `rewrite_header_binders` was eating the
  trailing newline before the next decl. Emits `\n` now.

  Tests 103 → 120 (+17 OX4 tests across the three new
  module-private test blocks).

  **Still pending in OX4 for v1.0 RC** (discovered while
  iterating against `tests/sample-lean/Sample.lean`):

  - Binary operator notation (`==`, `+`, `<` …) — OxiLean
    parser's `parse_expr` v0.1.2 doesn't recognise these
    as native syntax; needs notation registration or a
    textual lower (e.g. `a == b` → `Eq.beq a b`).
  - String interpolation (`s!"…{x}…"`) — OxiLean has no
    parse rule.
  - `Except`-typed exports (`Except String UInt64`).
  - Other corners surfacing as the parse position advances.

  Realistically the rest of OX4 is iterating a known
  procedure: hit the next parse error, identify the
  surface form, add a textual rewrite or note it as
  unfixable-via-textual (forcing OxiLean parser fork).
  Tail items not split into separate ROADMAP entries —
  they live under OX4 until the gap closes.

  **OX6 (v1.0 RC blocker, locked 2026-05-22; plan
  expanded 2026-05-22 to cover the full surface needed
  for the Lean 4 corpus)** — `leo4-lean4-parse` PEG-based
  Lean 4 parser. The OX4 textual approach reached its
  limits (operator precedence, string interpolation, ctor
  name resolution all need real grammar work). Decided to
  fork: new sibling crate `sibling/leo4-lean4-parse/`
  builds a PEG-based parser from scratch using the `peg`
  crate. **Strict superset** of `oxilean-parse` v0.1.2's
  accepted surface where overlapping; AST shapes designed
  to mirror upstream for downstream interop.

  **All sub-steps below are mandatory for v1.0 RC** —
  v1.0 RC ships only after the OX6 parser handles the
  full Lean 4 corpus, oxilean-parse fallback is removed
  for the transpile path, and leo4-oxilean-build runs on
  OX6 by default.

  **Progress sub-steps** (each = one commit, ordered):

  ─── Grammar build-out (steps 1–10) ───
  1. ✅ Scaffold + `def NAME [binders]+ [: TYPE] := VALUE`
  2. ✅ Expression grammar with operator precedence
  3. ✅ `if-then-else` + `match` arms with patterns
  4. ✅ Lambda + `fun` / `λ`
  5. ✅ `structure` + `inductive` + `deriving`
  6. ✅ Attribute lists with args
  7. ✅ Multi-line field types + layout-sensitive
        parsing + full expression re-parse
  8. ✅ `do` notation (single-line statements)
  9. ✅ String interpolation `s!"…{x}…"`
  9.5 ✅ Quantifiers `forall` / `∀` / `exists` / `∃`
        (inserted at 병익's request)
  10a. ✅ `theorem` / `lemma` / `axiom` decls + `=`
        propositional equality
  10b. ✅ `instance` + `class` decls
  10c. ✅ `namespace` + `section` + `mutual` blocks
  10d. ✅ `open` + `import` + `variable` decls

  ─── Surface coverage fill-in (steps 11a–11l;
       v1.0 RC mandatory) ───
  11a. ✅ Block comments `/- … -/` (nested) + doc
       comments `/-- … -/` (semantic binding in 11u)
  11b. ✅ Anonymous ctor `⟨a, b⟩` (Unicode angle brackets)
  11c. ✅ Modifier prefixes (`partial def`,
       `noncomputable def`, `private def`,
       `protected def`, `abbrev`)
  11d. ✅ let-in expression `let x := e; body`
  11e. ✅ `by …` tactic block (term-level entry into
       tactic mode)
  11f. ✅ Multi-line `do` statements (`if` / `match` /
       `let` spanning lines inside `do`)
  11g. ✅ Anonymous structure literal `{ x := 1, y := 2 }`
  11h. ✅ List literal `[1, 2, 3]`
  11i. ✅ Universe annotation `def foo.{u, v} : Sort u`
  11j. ✅ `@` explicit args (`@id Nat 0`)
  11k. ✅ `example : T := proof` (anonymous theorem)
  11l. ✅ Numeric literal extensions (`0x1F` hex,
       `0b101` binary, `3.14` float, `1_000` separator),
       extended string escapes (`\xHH`, `\u{…}`),
       multiline strings `"""…"""`

  ─── Surface coverage tail (steps 11m+; ALSO v1.0 RC
       mandatory per 병익) ───
  11m. ✅ `if let pat := e then … else …` (let-else
        deferred — not a Lean 4 term-mode form)
  11n. ✅ `match h : e with …` (scrutinee binding)
  11o. ✅ Pattern guards (`| pat if cond => …`)
  11p. ✅ `(· + 1)` anonymous fn shorthand (`·` placeholder)
  11q. ✅ Unicode operators (`≤`, `≥`, `≠`, `×`, `÷`,
       `∈`, `∉`, `∪`, `∩`, `⊆`)
  11r. ✅ `do for in`, `do while`, `do until` loops
  11s. ✅ DSL declarations: `notation`, `macro_rules`,
       `syntax`, `elab`, `infix` / `infixl` / `infixr` /
       `prefix` / `postfix` (split into 11s-a fixity +
       11s-b multi-line DSL commits)
  11t. ✅ Debug commands: `#check`, `#eval`, `#print`,
       `#guard`, `#guard_msgs`
  11u. ✅ Doc strings `/-- … -/` semantic binding
       (attaching to the next decl)
  11v. ✅ `omit` / `include` section-variable management
  11w. ✅ `def f | 0 => … | n+1 => …` pattern-matching
       def

  ─── Integration (steps 12–13) ───
  12. ⏳ Cross-check against `oxilean-parse` on a shared
        corpus (overlapping inputs must produce
        equivalent ASTs).
  13. ⏳ leo4-oxilean-build switches its default parser
        from `oxilean_parse::Parser` to
        `leo4_lean4_parse::parse_decls`. OX3/OX4 textual
        pre-rewrites in `lean4_normalize` become legacy
        (still kept for the optional `oxilean-parse`
        fallback path, but OX6 handles the surface
        natively).

  Once step 13 lands the OX3 / OX4 work transitions from
  "active dialect bridge" to "legacy compatibility layer".

  **Post-OX6 CLI refactor (planned 2026-05-24)** —
  applied **only after OX6 is fully done**:

  - `leo4 create` and `leo4 init` both drop the
    `--impl <runtime-impl-identifier>` option. Per-
    (sub)crate runtime-impl selection moves entirely
    into a `leo4.toml` config file. Multiple impls may
    be specified per (sub)crate, but if more than one
    is listed each impl's output path **must** be
    disjoint from every other's — overlap is rejected
    at config-parse time.
  - `leo4 create` additionally gains an optional
    `--subcrate` flag: when set, `create` performs the
    scaffold as a subcrate of the *current workspace*
    (located relative to CWD) rather than as a
    standalone crate.
  - `leo4 init` follows the same `--impl` →
    `leo4.toml` move, but does **not** gain
    `--subcrate` (init's contract is "this directory").

  Sequencing rationale: keep the CLI surface stable
  while OX6 churn lands; flip both together once the
  parser is the single source of truth.

  **OX5 (NEW v1.0 RC blocker, locked 2026-05-22)** — elab
  env bootstrap. CLI's transpile path runs `elaborate_decl`
  in an empty `Environment::new()`, so even successfully-
  parsed code fails on `NameNotFound("UInt64")` /
  `NameNotFound("+")`. The CLI needs to populate the env
  with the Lean stdlib + leo4 runtime decls before elab.
  Option (a): bake an env snapshot built ahead-of-time;
  Option (b): point the CLI at a pre-elaborated `.olean`
  cache the lake plugin produced.
- **OX2** Marshallable matrix expansion (locked 2026-05-22,
  carrier-types layer landed 2026-05-22).
  Built-ins now covered by `synthesize_canonical_wrapper`:
  - Primitives: u8..u128, i8..i128, f32, f64, bool, char,
    String, unit.
  - Carrier types (via `carrier_path_for`): `BigNat`,
    `BigInt`, `LeanRat`, `LeanComplexF32x2`,
    `LeanComplexF64x2`. Nightly variants (`LeanF16`,
    `LeanBF16`, `LeanF128`, `LeanComplexF16x2`,
    `LeanComplexBF16x2`, `LeanComplexF128x2`) behind a
    `nightly-floats` cargo feature forwarded to leo4-abi.
  - Generic containers (recursive): `Vec<T>`, `Option<T>`,
    `Result<T, E>`, `Box<T>`, tuples arity 2..=5.
  - Both `RustType::Vec/Option/Result` dedicated variants
    AND `RustType::Generic("Vec"|"Option"|"Result"|"Box", _)`
    forms are recognised — upstream may lower to either.

  User-defined records / inductives **landed 2026-05-22**
  (option (b) per the OX2-user-records decision):
  leo4-oxilean-build synthesises Rust struct / enum shapes
  from the parser-AST `Decl::Structure` / `Decl::Inductive`
  directly. `synthesize_struct_type` /
  `synthesize_enum_type` + `transpile_source_to_unit`
  recognise `@[leo4_export]`-tagged structures + inductives
  in the source stream, register the names in
  `Leo4ExportRegistry::user_types`, and emit type decls
  alongside the fn / wrapper sources. Rust-keyword field /
  variant names are raw-ident-escaped via
  `escape_rust_ident`. Wire form byte-compatible with
  hand-written `#[derive(LeanMarshal)]`.

**Deferred to ≥ v1.x** (all of P10.4 minus C4 above):

- A1 zygote-fork backend / A2 wasm-sandbox backend.
- B2 ConstraintExpr<Atom> typed AST.
- B3 async reverse exports.
- C2 macOS Tier 1 promotion / C3 wasm64 sibling project.
- D3 VS Code extension.
- D4 logicutils removal in favor of native
  `buildFileUnlessUpToDate'`.

**Flagship Phase 10 demo (SMT solver integration)** —
lives outside this repo at
[`Honey-Be/adsmt`](https://github.com/Honey-Be/adsmt). leo4
ships the building blocks (B1 is the critical one); adsmt
proves them out by integrating with z3 / cvc5 / its own
solver backend. Do NOT bundle SMT-specific types
(`Term`, `Sort`, …) into leo4.

## Future / not yet on the phase ladder

- **Alternative Lean 4 implementation support** (Phase 11+
  candidate, 2026-05-21): leo4 was developed against the
  reference Lean 4 implementation
  ([leanprover/lean4](https://github.com/leanprover/lean4)).
  The surface leo4 depends on is now extracted as
  `SPEC/lean-runtime-compat.md` — any implementation
  satisfying §1.1–1.4 is supported transparently; impls that
  don't need a glue layer.

  The canonical case study: **OxiLean**
  ([cool-japan/oxilean](https://github.com/cool-japan/oxilean),
  pure-Rust CiC ITP, v0.1.2 2026-05-03). OxiLean explicitly
  targets CiC semantics compatible with Lean 4 but NOT byte-
  level ABI / Lake / `lean.h` compatibility. Integration
  requires substantial work on OxiLean's side (or in a leo4-
  OxiLean compat-layer crate); leo4 itself doesn't change.

  Most plausible OxiLean integration point: the C4.x.x wasm
  pipeline. OxiLean's `oxilean-wasm` could expose the
  `leo4:host/leo4-component@0.1.0` world from
  `SPEC/wit/leo4-host.wit` — that bypasses §1.2's C-ABI
  requirement entirely and uses CM-based interop instead.
  No leo4 change needed; the bottleneck is OxiLean-side
  plugin development.

  Tracking position: Phase 11+ opportunity (not Phase 10
  deliverable). Activates if/when an OxiLean contributor (or
  the maintainers of another alt-impl) proposes a compat
  layer.
- **WASM backend** (`crates/leo4-wasm`): Lean→wasm itself is fragile;
  needs WASIp3 stable (Phase 7's gate) and a concrete use case. May
  fold into Phase 7 once both arrive.
- **General HKT closed-world enumeration**: deliberately *not* on the
  phase ladder. The reasoning (decided after the Phase 1 reviews):
  the boundary cross is monomorphic, so Rust never sees an HK
  parameter directly; Lean-internal HK is the user's business. The
  one rule the plugin enforces is that an `@[leo4_export]` whose
  signature includes an unconstrained higher-kind generic
  (`{F : Type → Type}` with no `@[leo4_specialize_when F : oneof {…}]`)
  is rejected at admit-set time with a diagnostic
  (`SPEC/mangling.md` Mandatory check 5). Constrained HK
  (`oneof { List, Option, … }`) works through the same enumeration
  path as any other constraint — the plugin just reads the closed
  set. If a use case ever needs *general* HK enumeration (every
  1-arity inductive in the user package), we will design it then;
  not before.
- **Mutable resource state from Rust**: today `LeanResource` is an
  opaque `u64` handle; users may want to mutate the pointed-to state
  via callbacks. Direct mutation through `LeanRef<'a, T>` is the
  wrong shape — it races with Lean's referential transparency and
  forces awkward borrow-checker dances on the Rust side. When this
  is tackled, the API should wrap mutating operations in a
  *monad* (e.g. `LeanRefM<'a, T, A>`) so that:
  - the order of state updates is explicit at the type level;
  - Lean-side handlers receive each update as a discrete `IO`/`StateM`
    action;
  - `Arena<'a>`'s lifetime stays the upper bound — handles cannot
    escape the monad's scope.
  Sketch: `state.update(|h| …) : LeanRefM<'a, T, A>` whose bind
  sequences shim calls; the matching Lean body runs as `StateM σ α`
  (or `IO α` for genuine effects). Concrete signature decided after
  Phase 5 (end-to-end shim) and Phase 6 (mutual recursion may share
  the same monad design) so we shape it against real ergonomics, not
  a hypothesis.

## Open question — deferred decision

**IDL output grouping in `.leo4-schema`.** Today the plugin emits one
`func` line per *monomorphisation* (e.g. `listLen` is 15 lines, one per
admit-set element). An alternative is to emit one declaration per
*generic* signature (`func listLen<T : marshal>(xs: list<T>) -> bignat;`)
and keep the per-instantiation table only inside `.leo4-mangling`.

Trade-off:

| Aspect | Per-mono (current) | Per-generic (proposed) |
|---|---|---|
| `.leo4-schema` size | linear in admit-set | constant per export |
| Hash sensitivity | every admit-set change rotates the hash | only signature changes rotate |
| WIT lowering | direct: monos already present | need to monomorphise during lower |
| Conformance test | rows-per-mono in golden file | smaller, but resolution depends on admit-set evaluation order |
| Reader ergonomics | concrete and grep-friendly | matches IDL grammar more naturally |

병익 deferred this; do not change the emit shape before it is
re-opened. If we do switch, the schema hash *will* rotate and every
mangled name will change.

## Cross-cutting deliverables (every phase)

- `SPEC/*.md` updated where applicable, in a commit that precedes the
  code change (CLAUDE.md "spec first").
- At least one example or test exercising the new feature.
- `CHANGELOG.md` entry summarising what shipped and any
  schema-hash-rotating change.
- `just test` passes.
