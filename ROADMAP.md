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
- `crates/leo4-native/` (skeleton): just enough to host one shim and
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

## Phase 5 — ABI shim synthesis + `cc`/`leanc` driving — **MOSTLY DONE 2026-05-20**

The biggest remaining gap: the C shim that bridges the canonical-ABI
wire format to Lean's native ABI (lean.h symbols), built from the
Lake plugin and linked into a shared library that
`crates/leo4-native/` loads via `libloading`.

**Status (2026-05-20):** every deliverable below is landed on Tier 1
Linux except `examples/02-roundtrip/`. The shim emitter, loader,
`leo4::import!`, `#[derive(LeanMarshal)]`, `leo4-build`, and the
`examples/01-hello/` exit check (handshake-mismatch ⇒ code 5) all
pass. `examples/02-roundtrip/` is the last exit criterion still open.

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
- ✅ **`crates/leo4-native/`** — `Lean::open` (= `Lean::init` +
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
- ⏳ **`examples/02-roundtrip/`** —
  `def echoes (xs : List u32) (n : Nat) : List u32` using
  user-defined types and a value-param. **Open**; last Phase 5
  exit criterion.

**Exit criteria:**

- ✅ `examples/01-hello/` runs end to end on Linux Tier 1.
- ⏳ `examples/02-roundtrip/` runs end to end on Linux Tier 1.
  macOS was demoted to Tier 3 on 2026-05-20: builds may still
  produce a `.dylib`, but neither this exit criterion nor the CI
  matrix requires it.
- ✅ Handshake-mismatch detection lands in `examples/01-hello/`
  (the in-process test mutates `schema_hash_bytes` and expects
  `LeanError { code: 5 }`).
- ✅ `just test` runs all sides green on Linux.

**Dependencies:** Phase 2 (mangling), Phase 4 (error paths). Phase 3
(WIT) optional.

## Phase 6 — Mutual recursion between nominal types

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

## Phase 7 — Async IDL (`io<T>` lowering to `future<T>`)

Lift D4. While WASIp3 was in flux the plugin lowered `io<T>` to
`result<T, error>` and kept the Rust public API entirely sync. Phase
7 introduces the async surface and is gated on WASIp3 reaching
stable.

**Entry gate (sequential, not negotiable):**

1. WASIp3 stabilises in `wasmtime` (stable, not preview).
2. Component-model `future<T>` / `stream<T>` host bindings land in
   `wit-bindgen` ≥ the chosen pinned version.
3. 병익 picks a concrete async use case for leo4 — generic async is
   harder than just turning a knob, and we will not generalise
   speculatively.

**Deliverables:**

- IDL grammar: `future<T>` / `stream<T>` enter the IDL **as
  function-level effect modifiers**, not as `IDLType` variants
  (LEO4-DESIGN.md D4 decision, 2026-05-19). Concretely: extend
  `func_decl` with an optional `async` / `stream` qualifier (or a
  single `effect` enum field on `FuncDecl`) so that the source form
  `func foo(x: T) -> future<U>;` parses into
  `FuncDecl { effect: Async, params: [(x, T)], ret: U }`. The
  parser desugars `future<T>` / `stream<T>` at the function-boundary
  position only; their appearance inside record / variant payloads
  remains a parse error (effects don't compose into values).
- Lean side: `LeanMarshal Task α` / `LeanMarshal IO α` adapters for
  callers who want their `IO` actions visible as `future<T>` across
  the boundary. Decide: keep the Lean API blocking and only widen the
  Rust API to `async fn`, or expose `Task α` directly?
- Rust side: `#[leo4::import]` learns an `async` mode. The macro emits
  `async fn` wrappers that hold an `Arena<'a>` across `.await`, which
  requires `Arena` to participate safely in the chosen runtime
  (interaction with `Send`/`Sync` constraints on `LeanRef<'a, T>` —
  LEO4-DESIGN.md §16 currently says `!Send` and `!Sync`; Phase 7
  may revisit per-type).
- Canonical ABI: lower `io<T>` to WIT `future<T>` (Phase 3 lowering
  pass extension), not `result<T, error>`. Sync-only callers keep
  working via a compatibility layer.

**Exit criteria:**

- One async example end-to-end on `wasmtime` HEAD with WASIp3
  enabled.
- The sync path from Phase 5 still works without code changes (the
  compatibility layer hides the new lowering).

**Dependencies:** Phase 5 (sync runtime fully working), Phase 3
(WIT lowering — extends here).

## Phase 8 — Mathlib-compatible subset

The hardest, longest-tailed phase. LEO4-DESIGN.md §11 marks Mathlib
usage as "likely never (subset only)"; Phase 8 narrows that to
"chosen subset, on demand, with a clear cost story". Nothing in the
phase is generic Mathlib support — Mathlib is too large and too
internally-recursive for that to be a sensible goal.

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

## Future / not yet on the phase ladder

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
