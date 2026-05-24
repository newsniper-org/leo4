# Lean runtime compatibility surface

> Pinned 2026-05-21. Defines the API / ABI / build-system
> surface that leo4 depends on when integrating with a Lean 4
> implementation. Any **alternative implementation** (OxiLean,
> a future Rust-native Lean fork, an academic reimplementation,
> etc.) that wants to be a supported leo4 runtime must satisfy
> every section marked **Required**. **Optional** sections
> describe surface leo4 *consumes if present* but degrades
> gracefully on absence.

## 0. Scope

leo4 was developed against the **reference Lean 4 implementation**
([leanprover/lean4](https://github.com/leanprover/lean4)). That
implementation is the de-facto contract today; this document
*extracts* that contract from the codebase so other
implementations have a target.

Two pipelines (`SPEC/reverse-direction.md` §0 distinguishes
them) impose different surface requirements:

| Pipeline | What's needed from Lean | Owning crate |
|---|---|---|
| **Forward** (Rust → Lean) | Meta-programming API, Lake plugin hooks, `lean.h` C ABI in the user package, `leanc` C-compile path | `lake/Leo4/`, `lake/Leo4Plugin/`, `crates/leo4-mslean4` |
| **Reverse** (Lean → Rust) | `@[extern]` declarations, `dlopen` of native cdylibs (or wasm import in C4.x.x), `IO ByteArray` lowering | `lake/Leo4Rust/`, `crates/leo4-rust-bridge`, `shim/leo4_rust_bridge_lean.c` |

## 1. Surface table

### 1.1 Required — forward direction

| Item | What it is | Used by |
|---|---|---|
| `Lean.importModules (loadExts := true)` | Re-load a user package's `.olean` files into the current Lean process, with `MetaM` extensions populated. | `lake/Leo4Plugin/Leo4Plugin/Main.lean` |
| `Lean.enableInitializersExecution` | Side-effect: arms the runtime's initializer-execution path before `importModules`. Called unconditionally (no-op on impls that don't need it; required on Lean ≥ v4.30 per `spike/SPIKE-0-FINDINGS.md` Q4). | Same |
| `ext.getModuleEntries` | Walks per-module declaration entries for an attribute extension (the `leo4_export` attribute's). | `lake/Leo4Plugin/Leo4Plugin/Main.lean` |
| `Lean.Meta.SynthInstance.getInstances` | Closed-world enumeration of typeclass instances for the admit-set algorithm. | `lake/Leo4Plugin/Leo4Plugin/AdmitSet.lean` |
| Attribute system: `@[leo4_export]` registration | Parametric attribute, declared via `registerBuiltinAttribute` (or equivalent). | `lake/Leo4/Leo4/Export.lean` |
| Syntax category: `leo4_constraint` | New Lean syntax category, declared via `declare_syntax_cat`. | `lake/Leo4/Leo4/Syntax.lean` |
| Custom `deriving` handler | `deriving LeanMarshal` lowers structures + inductives. Hooked via `registerDerivingHandler`. | `lake/Leo4/Leo4/Deriving.lean` |
| Lake DSL surface | `package`, `lean_lib`, `lean_exe`, `extern_lib`, `target`, `script`, `require <pkg> from "..."`, `srcDir`, `globs := #[…]`, `extraDepTargets`. | All `lakefile.lean` files |
| `Lake.Module.recBuildLean` is **not** required | Spike 0 (2026-05-16) confirmed the plugin doesn't need to hook this private API; it runs as a `lean_exe` invoked after `lake build`. | — |
| `Lean.MonadEnv` access (env's `extensions`) | The plugin queries an attribute extension's state via `getEnv`. | `lake/Leo4Plugin/Leo4Plugin/AdmitSet.lean` |

### 1.2 Required — `lean.h` C ABI surface (forward shim)

The shim emitted by the Lake plugin (`<pkg>.leo4-shim.c`)
`#include`s `<lean/lean.h>` and uses these symbols. An
alternative Lean must expose them under the same names + same
ABI (or a header-shim must alias them):

| Symbol family | Specific names used |
|---|---|
| Object header / tagging | `lean_obj_arg` (=`b_lean_obj_arg`), `lean_object*`, `lean_obj_tag` |
| Boxing / unboxing scalars | `lean_box`, `lean_box_uint32`, `lean_box_uint64`, `lean_box_float32`, `lean_box_float`, `lean_unbox`, `lean_unbox_uint32`, `lean_unbox_uint64`, `lean_unbox_float32`, `lean_unbox_float` |
| Constructor allocation / access | `lean_alloc_ctor`, `lean_ctor_get`, `lean_ctor_set`, `lean_ctor_get_uint8` / `_uint16` / `_uint32` / `_uint64` / `_float` / `_float32`, `lean_ctor_set_uint8` / `_uint16` / `_uint32` / `_uint64` / `_float` / `_float32` |
| Sized arrays (ByteArray) | `lean_alloc_sarray`, `lean_sarray_cptr`, `lean_sarray_size`, `lean_sarray_set_size` |
| Strings | `lean_string_cstr`, `lean_string_size` |
| IO result wrapping | `lean_io_result_mk_ok`, `lean_io_result_is_ok`, `lean_io_result_get_value`, `lean_io_result_show_error`, `lean_io_mark_end_initialization` |
| Reference counting | `lean_dec`, `lean_dec_ref`, `lean_inc`, `lean_inc_ref` (and the `_cold` variants for the slow path) |
| Initialization | `lean_initialize_runtime_module`, `initialize_<ModuleName>` (one per user module that the loader brings up) |
| Logging / errors | `lean_io_result_show_error` (printed by `leo4-mslean4` on `LEO4_ERR_IO_FAILED`) |

ABI commitments:
- `lean_object*` is a pointer-sized opaque handle. Reference-
  counted via the symbols above; layout is opaque (alternative
  impls may use a different in-memory shape as long as the
  symbols' contracts hold).
- `lean_alloc_ctor(tag, num_objs, scalar_sz)` produces an object
  whose **object slots** are indexed `0..num_objs-1` and whose
  **scalar slots** are byte-packed at running offsets within the
  trailing `scalar_sz` bytes — `SPEC/canonical-abi.md` §8 + §9
  rely on this layout for both records and `Phase 10-B5`
  multi-scalar variant payloads.
- `lean_io_result_*` produces a 2-tag union: `Ok payload` vs
  `Err err`. `lean_io_result_is_ok(x)` returns `1` iff `Ok`.
- `lean_unbox_uint32`-family round-trips the corresponding
  `lean_box_*`. Signed integer types share their unsigned-width
  unbox + a cast at the call site (Phase 7 step 2c).

### 1.3 Required — compilation toolchain (forward)

| Tool | What for | Used by |
|---|---|---|
| `leanc` | Compile the user package + shim C TUs to a `.so` / `.dylib` / `.dll`. Linker driver that knows how to link `libleanshared`. Must accept `-c -std=c2x` and the standard C compile flags. | `lake/Leo4Plugin/Leo4Plugin/Main.lean` (driving), `lake/Leo4Rust/lakefile.lean` (compiling the glue shim) |
| `libleanshared` | Shared library containing the Lean runtime. Linked into the user package's compiled `.so`. | (transitively via `leanc`) |
| `lean-toolchain` file convention | One-line file (`leanprover/lean4:vX.Y.Z`) at package root; `elan` consumes. Optional but conventional. | All Lake packages |

### 1.4 Required — reverse direction

| Item | What it is | Used by |
|---|---|---|
| `@[extern "C_NAME"]` declaration | Lean syntax for declaring an FFI binding to a C symbol. The compiler emits the call site that invokes the named C function with the Lean ABI. | `shim/leo4_rust_bridge_lean.c` callers in `<pkg>.leo4-rust-imports.lean` |
| `IO ByteArray` extern return type | The actual lowered ABI of `IO ByteArray` extern returns must match what `lean_io_result_mk_ok(lean_alloc_sarray(1, cap, cap))` produces. (Phase 9 runtime fix 2026-05-23 nailed this against `BaseIO ByteArray` divergence — the latter has different lowering.) | Same |
| `lean_exe` produces a process that can `dlopen` a sibling cdylib at runtime | The worker dispatcher (`leo4-rust-worker`) is `posix_spawn`'d by the Lean process and `dlopen`s the user cdylib. The Lean process itself doesn't need wasm; only native-target Lean impls participate today. | `crates/leo4-rust-bridge`, `crates/leo4-rust-worker` |

### 1.5 Optional — Phase 7 async + Phase 8 Mathlib subset

- **`IO α` lifting to `future<α>`** (Phase 7): the shim wraps
  `lean_io_result_*` and surfaces `LEO4_ERR_IO_FAILED` on `Err`.
  An impl without `IO` is irrelevant — leo4's `IO`-exporting
  fixtures would simply not compile on it.
- **Mathlib bridges** (Phase 8): live in opt-in
  `lake/Leo4/Leo4/MathlibBridge/*` modules. An impl without
  Mathlib doesn't break leo4 core; it just can't run the
  bridge type-check (`just mathlib-bridge-test`).
- **External marshal carriers** (Phase 8 step 2, `LeanRat`,
  `LeanU128`, …): rely on the standard `LeanMarshal` typeclass.
  No runtime-specific dep beyond §1.1–1.3.

## 2. The OxiLean question (2026-05-21)

[OxiLean](https://github.com/cool-japan/oxilean) is a pure-Rust
Lean-4-inspired ITP. v0.1.2 (2026-05-03). 1.35M+ Rust lines,
zero C/Fortran deps, has its own `oxilean-build`, its own
`oxilean-runtime` (Rust-native: actor model, arena, bytecode
interp), and its own `oxilean-wasm` (a wasm bytecode
interpreter).

OxiLean explicitly targets **CiC semantics compatible with
Lean 4**, NOT byte-level ABI / Lake / `lean.h` compatibility.
From the surface table above:

- ✅ §1.5 (Mathlib bridges) is a non-issue — opt-in modules
  type-check or don't; no runtime support needed.
- ❌ §1.1 (meta-programming API) — OxiLean's introspection API
  is different; the Lake plugin would need an OxiLean-specific
  port.
- ❌ §1.2 (`lean.h` C ABI) — OxiLean's runtime is Rust-native.
  Either OxiLean grows a `lean.h` compat shim, or the leo4
  forward-direction shim emitter needs an OxiLean-flavoured
  parallel.
- ❌ §1.3 (`leanc` toolchain) — OxiLean has `oxilean-cli`, not
  `leanc`. The Lake plugin's compile driver would need to
  detect + use it.
- ❌ §1.4 (reverse direction) — `@[extern]` semantics + IO
  ByteArray lowering would need OxiLean-side equivalents.

**Strong maturity signals** (per OxiLean's TODO.md, last
updated 2026-05-03):

| Signal | Implication for leo4 §1.1 |
|---|---|
| **Mathlib4 compat: 99.7% parse rate** (181,326 / 181,890 decls) | OxiLean parses essentially all Lean 4 source. `lake/Leo4/Leo4/*.lean` would almost certainly parse cleanly. |
| 32,345 tests passing, 0 warnings | Production-ish stability claim. |
| 320 curated theorem proofs: 100% pass rate | Elaboration + typechecking + tactics work end-to-end on real proofs. |
| `oxilean-elab/src/attribute/` — "10+ attribute kinds" with custom-attribute registration paths | `@[leo4_export]` registration plausible. |
| `oxilean-elab/src/derive/` (1,672 LOC) + `derive_adv/` (2,543 LOC) — "10+ derive handlers" | `deriving LeanMarshal` handler registrable. |
| `oxilean-elab/src/macro_expand/` (1,361 LOC) — "5 macro kinds" + `notation.rs` (1,351 LOC) | `leo4_constraint` syntax category plausible (would map onto OxiLean's macro / notation infra). |
| `oxilean-meta/src/synth_instance/` — trait-based (`InstanceSynthesizer`, `SynthInstanceConfig`) | Direct adapter point for `Lean.Meta.SynthInstance.getInstances`. |
| `oxilean-elab/src/lean4_compat/` — active source-syntax compat layer (`Lean4CompatMatrix`, `Lean4NamespaceTracker`, `Lean4OptionConfig`, `Lean4SectionManager`, `Lean4SyntaxAdapter`, `Lean4SyntaxVersion`, `Lean4TermRewriter`) | Bridges the parser layer of §1.1 to OxiLean's internal AST. |
| **Codegen backends: Rust / WASM / LLVM / JS / C** | C and WASM both exist as backends — these are §1.2 / §1.3 / C4.x.x adjacent. |
| `oxilean-runtime`: "Pluggable GC strategies and WASM runtime integration"; "Reference-counted closures, lazy thunks, tail-call optimization" | Same refcount + closure model as reference Lean; same conceptual ABI shape (though symbol names differ). |
| OxiZ SMT integration (`SmtContext::check_sat` via `oxiz-solver 0.2.1`) | Already exercises an SMT-backed tactic use-case — the same pattern leo4's adsmt flagship integration targets. |

**Caveats**:

- Most OxiLean modules carry a `//! Auto-generated module
  structure` doc-comment header. That signals heavy use of
  codegen rather than hand-curated APIs; the trait surfaces
  may evolve fast as the generator refines. Production
  integration should pin to a specific OxiLean release and
  retest on bump.
- "Mathlib4 99.7% parse rate" is a **parse** metric, not an
  **elaborate** metric. Mathlib4 type-checking under OxiLean
  is not claimed at that percentage. leo4's needs are
  closer to "elaborate + run attribute handlers" than to
  "parse"; expect §1.1 score to be lower than the 99.7%
  parse rate suggests.
- Codegen "C backend" almost certainly produces C against
  OxiLean's *own* runtime ABI, NOT against `<lean/lean.h>`.
  So §1.2 stays unsupported until OxiLean explicitly ships
  a `lean.h` compat layer — possible (their `lean4_compat/`
  trend suggests interest) but not currently advertised.
- OxiLean's WASM backend produces wasm against OxiLean's
  runtime, NOT a leo4-host.wit world. C4.x.x WIT-world
  conformance would still be a deliberate OxiLean-side
  feature.

**Conclusion**: OxiLean support is achievable in principle but
requires *substantial* work on **the OxiLean side** (or on a
leo4-OxiLean compat-layer crate), not on leo4 itself. leo4's
position is **runtime-spec-agnostic**: any implementation that
satisfies §1.1–1.4 is supported transparently; impls that don't
satisfy them need a glue layer.

The leo4-wasm pipeline (C4 / C4.x / C4.x.x) is one plausible
OxiLean integration point — OxiLean's own `oxilean-wasm`
could theoretically be extended to expose the
`leo4:host/leo4-component@0.1.0` world from
`SPEC/wit/leo4-host.wit`. That bypasses §1.2's C-ABI
requirement entirely and uses CM-based interop instead.

**An even better fit for OxiLean (and any other rust-native
Lean impl)**: `SPEC/rust-native-lean.md` defines an
in-process Rust-to-Rust integration path that bypasses §1.2,
§1.3, AND §1.4 — the only contract is a single `LeanProc`
trait an adapter crate (e.g. `leo4-oxilean`) implements
against the impl's native Rust API. Same canonical-ABI bytes
on the wire (cross-impl conformance preserved), but transport
is a direct Rust function call. See `SPEC/rust-native-lean.md`
§2 + §6 for the trait surface and the three-paths comparison
table.

**Status update 2026-05-24** — the OxiLean **transpile**
path (the §9 variant of `rust-native-lean.md`, not the
in-process `LeanProc` variant) is now end-to-end:
`sibling/leo4-oxilean-build` parses with the OX6 PEG-based
Lean 4 parser (`sibling/leo4-lean4-parse`, strict
superset of `oxilean-parse` v0.1.2), translates to
`oxilean_parse::Decl` via `leo4_translate`, elaborates
against an env bootstrapped by `leo4_env_bootstrap`
(OxiLean `init_builtin_env` + leo4 boundary primitives,
**zero lake/lean overhead**), and lowers via
`oxilean_codegen::to_lcnf`. OxiLean-only users install
nothing beyond `leo4-oxilean-build`. The in-process
`LeanProc` variant of `rust-native-lean.md` remains
future work (§8 in that doc).

### Two-axis classification of leo4 surface satisfiability

For an alt-impl (OxiLean or future) to be a leo4 backend, it
needs to satisfy each surface section. The matrix:

| Section | OxiLean status (2026-05-21) | Effort to satisfy |
|---|---|---|
| §1.1 meta-programming API | **likely satisfiable** via `lean4_compat/` (source-syntax adapter) + `synth_instance/` (trait-based) + `attribute/` (custom-reg) + `derive*/` (handler-reg) + `macro_expand/` (notation/syntax categories). Mathlib4 99.7% parse rate suggests the surface coverage is broad. | medium — OxiLean side, may already be 80%+ there. |
| §1.2 `lean.h` C ABI | unsupported (Rust-native runtime; "C codegen" exists but targets `oxilean-runtime`, not `<lean/lean.h>`) | large — OxiLean grows a `lean.h` compat shim, OR leo4-wasm's CM path bypasses §1.2 entirely. |
| §1.3 `leanc` toolchain | unsupported (`oxilean-cli`, `oxilean-build`); but OxiLean's "C / WASM / Rust codegen backends" exist as analogues | medium — Lake plugin's compile driver gains an OxiLean-tool detection branch; or OxiLean's `oxilean-build` grows a Lake-compatible mode. |
| §1.4 reverse direction | unsupported (no `@[extern]` lowering equivalent surfaced); but `oxilean-runtime` is refcount + closure model (same conceptual ABI shape) | large — needs FFI-call semantics + worker handshake equivalent. |
| §1.5 Phase 7 / 8 optional | trivial (opt-in modules type-check or don't) | none — no runtime support needed |

The recommended integration route, prioritised by ROI:

1. **OxiLean-side**: extend `oxilean-wasm` to export the
   `leo4:host/leo4-component@0.1.0` WIT world. This bypasses
   §1.2 / §1.3 entirely — the leo4-wasm pipeline becomes the
   transport. Smallest scope; testable in isolation.
2. **OxiLean-side**: complete `lean4_compat/` enough to
   compile / elaborate the `lake/Leo4/Leo4/Export.lean`
   attribute declaration as a litmus test. If that works,
   `@[leo4_export]` recognition transfers transparently.
3. **leo4-OxiLean compat crate** (separate repo): if §1.2 /
   §1.3 / §1.4 truly need a leo4-side adapter, that adapter
   lives outside the main leo4 workspace — keeps leo4 itself
   runtime-spec-agnostic per the policy above.

This document does not commit leo4 to *any* of those routes.
It defines the contract they'd satisfy.

## 3. Versioning

This SPEC is **descriptive** of the current leo4 codebase, not
prescriptive of a stable Lean ABI. Bumps to leo4's required
surface (e.g. Phase 10-B1's callback ABI introducing a re-
entrant dispatch frame) get added here as bullets dated, NOT
as new SemVer revisions of this document.

A future "minimum required Lean surface" SemVer (analogous to
how `leo4:host@0.1.0` versions the WIT) becomes worth defining
*once* there's a non-reference impl actively trying to satisfy
this surface. Currently: reference Lean is the only impl.

## 4. Cross-references

- Pipeline architecture: `LEO4-DESIGN.md` §3 (forward) + §16 +
  D16 (reverse).
- Wire format that everything above transports: `SPEC/canonical-abi.md`.
- Where the `lean.h` symbols above get emitted into the shim:
  `lake/Leo4Plugin/Leo4Plugin/Main.lean`'s `renderShimSource`.
- The Lake plugin's hook decision: `spike/SPIKE-0-FINDINGS.md`.
- C4.x.x WIT contract that bypasses the C ABI for the wasm
  target: `SPEC/wit/leo4-host.wit` + `SPEC/wit/README.md`.
