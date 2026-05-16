# leo4 — Roadmap

> Sequential phases. Each phase has a gate: do not advance until the gate's
> exit criteria are satisfied. The roadmap is opinionated about *order* but
> not about *duration* — actual time per phase varies.

## Phase 0 — Lake Hook Spike

See `spike/SPIKE-0-lake-hook.md` in full.

**Exit criteria:**
- `spike/SPIKE-0-FINDINGS.md` exists, signed, with a GREEN or YELLOW recommendation.
- If RED: an alternative path is documented and agreed on with 병익 before Phase 1.

## Phase 1 — IDL parser and skeleton

**Lean side:**
- `lake/Leo4/Syntax.lean`: define `leo4_constraint` syntax category with the
  productions from `LEO4-DESIGN.md` §4.2.
- `lake/Leo4/Export.lean`: register `@[leo4_export]` attribute (no-op for now,
  but discoverable).

**Rust side:**
- `crates/leo4-idl/`: IDL parser producing AST. Cover WIT subset first, then
  add generics, constraints, cyclic ADTs.
- `crates/leo4c/`: CLI binary that can `leo4c parse foo.leo4` (prints AST)
  and `leo4c lower foo.leo4 --to wit` (emits WIT).

**Exit criteria:**
- 5 hand-written IDL files in `examples/idl/` round-trip through the parser.
- `cargo test -p leo4-idl` passes.
- `leo4c lower` produces valid WIT for at least 3 examples.

## Phase 2 — Admit-set extraction (scalar only)

**Lean side:**
- `lake/Leo4Plugin/AdmitSet.lean`: implement scalar admit-set as a constant
  table. Implement parsing of `@[leo4_specialize_when]` quotations.
- `lake/Leo4Plugin/Main.lean`: minimal driver that walks env, finds
  `@[leo4_export]`, prints admit-sets to stdout.

**Rust side:**
- `crates/leo4-idl/`: extend with `scalar` constraint resolution.
- `SPEC/mangling.md`: write spec.
- `crates/leo4-idl/`: implement mangling per the spec.

**Exit criteria:**
- Toy Lean package with one scalar-generic export reports correct admit-set.
- Mangling tests pass on both sides (cross-impl harness in `tests/mangling/`).

## Phase 3 — End-to-end scalar pipeline

**Lake plugin:**
- Emit `.leo4-handshake` and `.leo4-mangling` files.
- Emit C shim source.
- Drive `leanc` to compile shim + Lean code into `.so`.

**Rust runtime:**
- `crates/leo4-abi/`: scalar canonical-encode/decode.
- `crates/leo4-native/`: load `.so` via `libloading`, dispatch calls.
- `crates/leo4/`: `Lean`, `Arena<'a>`, `LeanRef<'a, T>` for scalars.

**Macro:**
- `crates/leo4-macros/`: `#[leo4::import]` for non-generic functions only.

**Example:**
- `examples/01-hello/` — Lean function `fn add(a: u64, b: u64) -> u64`
  callable from Rust end-to-end.

**Exit criteria:**
- `examples/01-hello/` runs and produces correct output.
- `cargo test` covers handshake-mismatch detection (mutate the IDL, expect
  link-time failure).

## Phase 4 — Constraint quotation and macro generics

**Lake plugin:**
- Hook into `Lean.Meta.SynthInstance.getInstances` for typeclass-based
  admit-sets.
- Validate `∧`, `∨`, `¬` over closed sets.
- Reject forbidden constructs (universe polymorphism, etc.).

**Rust macro:**
- `#[leo4::import]` expands to `match T::SCALAR_TAG { … }` dispatch tables
  for scalar generics.
- Generated `extern "C"` blocks use the mangling table.

**Example:**
- `examples/02-bucketize/` — `fn bucketize<T: scalar + ord>(xs, bs)` callable
  for any T in the admit-set from Rust.

**Exit criteria:**
- `examples/02-bucketize/` runs for u32, u64, f64.
- Switching the IDL changes mangled names → link error caught before runtime.

## Phase 5 — Trait constraints and complex types

**Lake plugin:**
- Closed-world enumeration of `T : Marshal`-style constraints.
- Cyclic ADT lowering to resources (WIT path).

**Rust runtime:**
- `LeanResource` trait and resource handle management.
- Non-scalar generic dispatch (dyn-dispatch fallback if needed).

**Example:**
- `examples/03-cyclic-adt/` — `expr` ADT (lit, add, mul) round-trips between
  Lean and Rust.

**Exit criteria:**
- `examples/03-cyclic-adt/` runs.
- A user-defined record with `#[derive(LeanType)]` works.

## Phase 6 — Decision point: deepen native or pivot to wasm

At the end of Phase 5, decide:

- **Path A**: stay native, polish ergonomics (better errors, more derives,
  perf work).
- **Path B**: start `crates/leo4-wasm` for wasm32 targets, accepting that
  Lean→wasm itself is fragile.

This decision depends on the actual use case 병익 picks for leo4. Defer
until then; do not pre-empt with code.

## Cross-cutting deliverables (every phase)

- `SPEC/*.md` updated where applicable.
- At least one example or test exercising the new feature.
- `CHANGELOG.md` entry.
- `just test` passes.
