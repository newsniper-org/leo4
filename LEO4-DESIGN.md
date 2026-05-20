# leo4 — Design Document

> **Status**: pre-implementation, all major design decisions resolved.
> Owner: 병익 (Honey-Be). Begin date: 2026-05.
>
> This document is the single source of truth for leo4's design. Implementation
> agents (Claude Code or otherwise) should read this first, then `CLAUDE.md`, then
> `ROADMAP.md`.

## 0. What leo4 is

leo4 is a Lean 4 ↔ Rust interop library that combines:

- **PyO3-style** lifetime-token ergonomics for managing Lean objects from Rust
- **Java FFM-style** layout-as-data, so the Rust side does not compile against
  `lean.h` and is therefore *not* bound to a specific Lean toolchain version
- **WIT / Component Model**-style canonical ABI as the transport contract,
  enabling native and WASM backends behind a single Rust API

The problem it solves over `leo3` is that `leo3` compiles against `lean.h` and
breaks every time Lean's internal layout shifts. leo4 pushes all Lean ABI
knowledge into a build-time-generated C shim and exposes only a stable
canonical ABI to Rust.

## 1. Resolved Decisions

These are all final unless explicitly reopened. Do not relitigate without
written justification.

| # | Decision | Resolution |
|---|---|---|
| D1 | Project name | `leo4` |
| D2 | Schema IDL | Strict superset of WIT; lowering to WIT when WASM target requires it |
| D3 | Generic strategy | **(α′)** Specialize-both with **constraint-driven** instantiation enumeration |
| D4 | Async model | Sync only until WASIp3 stabilizes; `io<T>` lowers to `result<T, error>` for now. **Surface form when lifted (decided 2026-05-19):** asynchrony is a *function-level effect flag* (`is_async` boolean, or equivalent enum if `stream<T>` also lifts), **not** an `IDLType` variant. `future<T>` / `stream<T>` in source IDL are syntactic sugar that the parser folds into the function's effect + payload type `T`. Rationale: keeps `IDLType` free of effect coloration, avoids effect-wrappers leaking into record/variant payloads, and matches the WIT-component-model "function ABI carries the future, not the value" mental model. |
| D5 | `leo4_specialize_when` syntax on Lean side | Lean metaprogram quotation via a dedicated `leo4_constraint` syntax category |
| D6 | Rust `extern` representation | `extern "C"` plus a macro layer (`#[leo4::import]`) on stable Rust; no new calling convention |
| D7 | Repository structure | Monorepo (Cargo workspace + Lake workspace, side by side) |
| D8 | Cargo/Lake build order | Lake first, Cargo second. Cargo's `build.rs` invokes Lake. |
| D9 | Handling Rust-discovered instantiations | **Lazy mode**: Lake pre-emits the entire admit-set; Rust uses a subset. Mangling is deterministic from normalized IDL form, so both sides independently compute identical symbol names. |
| D10 | Lean compiler version policy | `lean-toolchain` file is committed; Lake plugin depends on it. Rust side does NOT depend on the Lean version directly. |
| D11 | Universe polymorphism across boundary | Forbidden. `@[leo4_export]` on a `Sort u`-polymorphic function is rejected by the Lake plugin. |
| D12 | Dependent types across boundary | Forbidden at the boundary. Boundary types must be in `Type 0`. |
| D13 | `wasm64` policy | Target eventually but gated behind a feature flag until `wasmtime`'s `memory64` + Component Model intersection is stable. |
| D14 | Reference counting | Forward Lean's rc through `LeanRef<'a, T>`. `clone` = `lean_inc`, `Drop` = `lean_dec`. |
| D15 | `oneof` constraint expression power | First version: union only. Intersection expressed via `∧`. |

## 2. Architecture

```
                   User Rust code
                         │
                #[leo4_export] / #[leo4_import]
                         │
              ┌──────────▼──────────┐
              │   leo4 (ergonomics) │   Arena, LeanRef<'a, T>, traits
              └──────────┬──────────┘
                         │
              ┌──────────▼──────────┐
              │   leo4-abi          │   canonical ABI encode/decode
              │   (layout-as-data)  │
              └──────────┬──────────┘
                         │
                ┌────────┴────────┐
                ▼                 ▼
       ┌────────────────┐ ┌─────────────────────┐
       │ leo4-native    │ │ leo4-wasm           │
       │  (C shim →     │ │  (wasmtime          │
       │   Lean runtime)│ │   Component Model)  │
       └────────┬───────┘ └─────────┬───────────┘
                │                   │
                ▼                   ▼
        Lean runtime (in-proc)  Lean → wasm (compiled w/ leo4)
                ▲                   ▲
                └────────┬──────────┘
                         │
                  lake-leo4 plugin
                  (single WIT/IDL emit)
                         │
                 Lean code with @[leo4_export]
```

## 3. The Lifetime / Arena Model

```rust
pub struct Lean { /* runtime handle, opaque */ }

pub struct Arena<'a> {
    _marker: PhantomData<&'a Lean>,
    // …
}

pub struct LeanRef<'a, T: LeanType> {
    handle: Handle,                  // u64 (works in wasm64 too)
    arena: &'a Arena<'a>,
    _t: PhantomData<T>,
}

impl Lean {
    pub fn init() -> Result<Self, LeanError> { /* … */ }

    /// scope returns control after every LeanRef inside is dropped
    pub fn scope<R>(
        &self,
        f: impl for<'a> FnOnce(&'a Arena<'a>) -> R,
    ) -> R { /* … */ }
}

impl<'a, T: LeanType> Drop for LeanRef<'a, T> {
    fn drop(&mut self) { self.arena.dec_rc(self.handle); }
}
```

- The `Lean` token alone does not grant Lean access; you must enter a `scope`
  to get an `Arena<'a>`. This is the FFM-style explicit arena pattern.
- `Arena<'a>` carries the backend dispatch (native vs wasm); `LeanRef` is
  backend-agnostic at the type level.
- `Handle` is always `u64`. On native, this is a pointer cast; on wasm, this
  is a Component Model resource handle. Both fit in 64 bits.

## 4. The IDL (WIT Strict Superset)

### 4.1 Additions over WIT

| Feature | Syntax | Rationale |
|---|---|---|
| Generic functions | `func map<A, B>(…)` | Native interop needs it; WIT lowers via monomorphization |
| Generic records | `record list<T> { … }` | Same |
| Type bounds | `<T: scalar + ord>` | Drives (α′) admit-set computation |
| Cyclic ADTs | `type expr = variant { … }` direct recursion | WIT lowers via `resource` wrapping |
| `bigint`, `bignat` | Builtin primitives | Lean has `Int`/`Nat` natively |
| `io<T>` | Effect wrapper | Sync-only for now: lowers to `result<T, error>` |

### 4.2 Constraint Sublanguage

```
constraint ::= "scalar"
             | "ord" | "eq" | "hash" | "pod" | "marshal" | "resource"
             | type ":" type            -- type class membership
             | constraint "∧" constraint
             | constraint "∨" constraint
             | "¬" constraint
             | "(" constraint ")"
             | type "=" type            -- decidable equality
```

- `scalar` = closed admit-set `{u8, u16, u32, u64, i8, i16, i32, i64, f32, f64}`
- Other named constraints (`ord`, `eq`, …) are typeclasses with open
  admit-sets that get closed by the dependency graph.

### 4.3 Forbidden Constructs at the Boundary

- Universe-polymorphic types (`Sort u` for any `u`)
- **Dependent codomain**: a function whose return type *syntactically*
  mentions one of its own value generics. Dependent *parameter* types
  (`(n : Nat) → Vec n α → α`) ARE allowed — the value `n` is erased and
  `Vec n α` lowers to `list<α>` at the boundary; see SPEC/mangling.md
  "Value-param erasure". Only when the return type itself depends on a
  value is the function unmarshallable.
- Non-`Type 0` types in *parameter / return positions*. Generic
  parameters themselves may have higher kinds (`F : Type → Type`) or
  value types (`n : Nat`) — both are erased / monomorphised by the
  plugin — but the *transported* types after substitution must land in
  `Type 0`.
- Recursive constraints (e.g., `T : Marshal` requiring `T → T : Marshal`)
- Open-ended negation (`¬(T : Marshal)`)
- **Mutual recursion between two nominal types** (each naming the other
  via `Self`-or-otherwise) — forbidden in v0; lifted in Phase 6
  (`ROADMAP.md`). v0 supports only *direct* self-recursion through the
  `Self` keyword (`SPEC/idl-grammar.ebnf`). Until Phase 6 lands, users
  break a mutual cycle by wrapping one side in a `LeanResource` handle.

The Lake plugin rejects these with diagnostics.

## 5. The (α′) Algorithm

```
Input:
  R = { f<T1,…,Tn> : Rust call sites of leo4-exported functions }
  L = { f<T1,…,Tn> : Lean call sites of leo4-exported functions }
  C = { (f, [c1,…,cn]) : constraints on each parameter of each generic f }

Step 1 — admit-set per parameter:
  for each (f, [c1,…,cn]) in C:
    for i in 1..n:
      if the generic Ti is a VALUE_PARAM (its binder type is a `Type`-kinded
                                          expression like `Nat`, not a kind):
        admit(f, i) := ERASED       // value generics are not enumerated;
                                    // the plugin records the param's name
                                    // and erases it from the boundary.
                                    // See SPEC/mangling.md "Value-param erasure".
      else if Ti has a HIGHER kind (`Type -> Type` or above):
        if ci is absent:
          REJECT f with a diagnostic pointing at the binder.
          // Higher-kind admit-sets require an explicit constraint —
          // typically `@[leo4_specialize_when F : oneof {List, Option, …}]`.
          // SPEC/mangling.md "Mandatory checks" (5).
        else:
          admit(f, i) := evaluate(ci) over current environment
          // Normally `oneof { … }`; enumeration just reads that closed set.
      else if Ti is PHANTOM (does not appear in any parameter type or in
                            the return type of f's signature):
        admit(f, i) := PHANTOM     // a single dimensionless slot — see below
      else if ci is absent (the generic has no constraint at all):
        admit(f, i) := UNBOUNDED   // every primitive IDL type — see below
      else:
        admit(f, i) := evaluate(ci) over current environment
        // closed-form for `scalar` etc.
        // type class enumeration via Lean.Meta.SynthInstance.getInstances

  UNBOUNDED := { u8, u16, u32, u64, i8, i16, i32, i64, f32, f64,
                 bool, char, string, bigint, bignat }
  // Every leo4 primitive type the plugin can mechanically round-trip.
  // Composite types (record / variant / resource / ...) are NOT included
  // because their per-user-package admit-set is open-ended; if the caller
  // needs those, they must write an explicit constraint or `@[leo4_specialize_when …]`.

  PHANTOM is *not* a set of types; it is a marker that says "this generic
  has no observable effect on the function's ABI surface, so the plugin
  does not enumerate it." The generic occupies its slot in the function's
  `generics` array (the signature still declares it), but in the emitted
  mangling table its `generic_args` position is rendered as JSON `null` and
  the cartesian product skips this axis entirely — preventing the
  duplicate-mangled-name cascade that would otherwise occur, since the
  mangled name carries only parameter types and is therefore identical
  across every "instantiation" of a phantom T.

  An untouched class constraint on a phantom generic (e.g. `foo<T : Ord>`
  where `T` is unused in `foo`'s body) is ignored — phantom-ness takes
  precedence.

Step 2 — initial frontier:
  F := R ∪ L

Step 3 — expansion (lazy mode pre-emits full admit-set, not just frontier):
  for each generic f with admit-sets [A1,…,An]:
    F := F ∪ { f<T1,…,Tn> : Ti ∈ Ai }

Step 4 — validation:
  for each f<T1,…,Tn> in F:
    assert Ti ∈ admit(f, i)
    if depth(Ti) > max_depth: error

Step 5 — emit:
  for each f<T1,…,Tn> in F:
    name := mangle(f, [T1,…,Tn])
    Lake emits: Lean specialization, native shim entry, WIT function
    Rust independently computes the same mangled name from the IDL
```

`max_depth` defaults to 8. Configurable via `leo4.toml`.

## 6. Mangling Specification

```
mangle(f, [P1,…,Pm]) =
    "leo4__" ++ pkg ++ "__" ++ iface ++ "__" ++ fname ++
    "__" ++ join("_", map(mangle_type, [P1,…,Pm])) ++
    "__h" ++ base32lc( hash_be_bytes( fnv1a64(normalized_idl) ) )

-- [P1,…,Pm] = parameter types after generic substitution, in declaration
-- order. Each instantiation is also accompanied by the generic argument
-- vector [T1,…,Tn] in the emitted `<pkg>.leo4-mangling` table (see
-- SPEC/handshake.md), but the linker symbol carries only the parameter
-- types — the ABI surface — because that is what callers actually pass.

mangle_type(u8)              = "u8"
mangle_type(u16)             = "u16"
mangle_type(u32)             = "u32"
mangle_type(u64)             = "u64"
mangle_type(i8…i64)          = "i8" … "i64"
mangle_type(f32)             = "f32"
mangle_type(f64)             = "f64"
mangle_type(bool)            = "b"
mangle_type(string)          = "str"
mangle_type(bigint)          = "bI"
mangle_type(bignat)          = "bN"
mangle_type(list<T>)         = "L_"  ++ mangle_type(T) ++ "_l"
mangle_type(option<T>)       = "O_"  ++ mangle_type(T) ++ "_o"
mangle_type(result<T,E>)     = "Rz_" ++ mangle_type(T) ++ "_" ++ mangle_type(E) ++ "_z"
mangle_type(record R<T1,…>)  = "S_"  ++ R ++ "_" ++ … ++ "_s"
mangle_type(variant V<T1,…>) = "V_"  ++ V ++ "_" ++ … ++ "_v"
mangle_type(resource R)      = "X_"  ++ R ++ "_x"
mangle_type(tuple<T1,…,Tn>)  = "T_"  ++ … ++ "_t"
```

### Normalized IDL form (input to fnv1a64)

1. Strip comments and doc strings
2. Strip whitespace down to canonical single spaces
3. Sort `import`s lexicographically
4. Inline all `type` aliases
5. Order `interface` members canonically (alphabetic by name)
6. Emit as UTF-8 byte stream

### Hash construction

`fnv1a64` is the standard FNV-1a 64-bit hash (offset basis `0xcbf29ce484222325`,
prime `0x100000001b3`) over the UTF-8 bytes of the normalized form. The 8 hash
bytes are taken big-endian (MSB first) and encoded as 13 lowercase base32
characters using the RFC 4648 alphabet (`abcdefghijklmnopqrstuvwxyz234567`)
with no padding.

Why FNV-1a rather than a cryptographic hash: the role of this digest is to
*invalidate stale ABIs at link time*, not to resist adversarial collisions.
Cargo's `cargo:rerun-if-changed=` and the linker between them notice any
schema drift. Both the Lean and Rust sides must agree on the algorithm
byte-for-byte; that is the only normative requirement.

The 8-byte digest prefix in the mangled name **acts as the schema handshake**.
If the IDL changes, all mangled names change, so a stale Rust binary
linking against a fresh shim fails at link time, not silently.

## 7. Build Orchestration

### Phase 1 — Lake (always first)

```
lake build leo4plugin     # builds the plugin itself once per toolchain
lake build <user-pkg>     # emits:
                          #   - <pkg>.leo4-schema     (full IDL)
                          #   - <pkg>.wit             (lowered, if requested)
                          #   - <pkg>.leo4-shim.so    (native shim + Lean code)
                          #   - <pkg>.leo4-handshake  (schema hash + admit-set summary)
                          #   - <pkg>.leo4-mangling   (full mangled name table)
```

### Phase 2 — Cargo

```
cargo build               # build.rs in leo4-build:
                          #   - locates <pkg>.leo4-handshake
                          #   - sets rustc-link-search to Lake's output
                          #   - sets rustc-link-lib=dylib=<pkg>.leo4-shim
                          # leo4_macros::import expands using
                          #   the handshake + mangling table
```

### Cross-rebuild contract

- Lake rebuild is triggered by changes to `.lean` files or `lean-toolchain`.
- Cargo rebuild is triggered by changes to `.rs` files or the Lake handshake.
- Cargo never rebuilds Lake; Lake never reads Rust source.

## 8. Canonical ABI

leo4-abi follows the WIT Canonical ABI (component-model spec) with these
leo4-specific extensions:

- `bigint` and `bignat`: little-endian `u64` words, leading sign byte for
  `bigint`. Encoded as `list<u64>` + `bool` in the WIT lowering.
- Resource handle is `u64`, not `i32`. This is incompatible with default WIT
  for wasm32 but matches wasm64 and native.
- Error path: every leo4 call returns a status `i32`; non-zero means the
  return buffer contains a serialized `LeanError`.

`SPEC/canonical-abi.md` enumerates each type's wire format.

## 9. Backends

### 9.1 leo4-native

- Loads a `.so` / `.dll` shim produced by Lake (Tier 1: Linux,
  Windows). macOS `.dylib` is **Tier 3 (best-effort)** as of
  2026-05-20 — the loader compiles and may work, but CI does not
  verify it and regressions on macOS are out-of-scope for v0.
- Shim is a thin C layer over `lean.h`; the *shim* is version-locked to Lean,
  the Rust crate is not.
- Uses `libloading` for dynamic load (lets users swap shims at runtime).

#### Lean's unboxed FFI rules (shim emitter must mirror)

The shim's `extern` declarations must match the C signature Lean's
compiler actually emits for each `@[leo4_export]` helper, otherwise
arguments and return values silently corrupt across the boundary.
Lean's rule (see `Lean/Compiler/LCNF/ToImpureType.lean`'s
`impureTypeForEnum` and the `hasTrivialImpureStructure?` codepath):

| Lean type                                                   | C ABI seen by the shim |
| ----------------------------------------------------------- | ---------------------- |
| all-nullary inductive, `numCtors < 2⁸`                      | `uint8_t`              |
| all-nullary inductive, `2⁸ ≤ numCtors < 2¹⁶`                | `uint16_t`             |
| all-nullary inductive, `2¹⁶ ≤ numCtors < 2³²`               | `uint32_t`             |
| all-nullary inductive, `numCtors ≥ 2³²`                     | `lean_object *` (boxed)|
| single-`UInt64`-field structure (incl. `@[leo4_resource]`)  | `uint64_t`             |
| every other inductive (mixed payload, > 1 relevant field)   | `lean_object *`        |

The wire format on the leo4 side stays unchanged — `u32 LE` for IDL
`enum`, `u64 LE` for `resource` — but the shim must narrow / widen
across the boundary in step with Lean's unboxing. Discovered
2026-05-20 while wiring nominal-type wrappers in
`examples/01-hello/`; encoded in
`lake/Leo4Plugin/Leo4Plugin/Main.lean`'s `enumScalar` and
`resourceHandler`.

### Platform tier policy

| Tier | Platforms                       | Guarantee |
|------|---------------------------------|-----------|
| 1    | x86_64-unknown-linux-gnu        | every commit verified by CI; regressions block merge |
| 2    | x86_64-pc-windows-msvc          | feature parity expected, periodic CI |
| 3    | aarch64-apple-darwin / x86_64-apple-darwin | best-effort; community fixes welcome but not gating |

The macOS demotion (2026-05-20) was a scope-cut decision: the
canonical-ABI shim, the `LeanMarshal` derivation handler, and the
plugin emit path are all platform-agnostic in code, so the demotion
only changes which platforms appear in the exit criteria and CI
matrix, not the source.

### 9.2 leo4-wasm

- Hosts a `*.wasm` component via `wasmtime`.
- Resources flow through the Component Model resource type.
- `wasm32` first, `wasm64` behind `feature = "memory64"`.
- WASIp2 today; WASIp3 once stable (D4).

## 10. Type System on the Rust Side

```rust
pub trait LeanType: Sized + 'static {
    const SCHEMA: SchemaItem;
    fn nominal_name() -> &'static str;
}

pub trait LeanScalar: LeanType + Copy {
    const SCALAR_TAG: ScalarTag;
}

pub trait LeanResource: LeanType {}

pub trait LeanOrd: LeanType { /* … */ }
pub trait LeanEq: LeanType { /* … */ }
pub trait LeanHash: LeanType { /* … */ }
pub trait LeanMarshal: LeanType {
    fn canonical_encode(&self, buf: &mut EncodeBuf) -> Result<()>;
    fn canonical_decode(arena: &Arena, buf: &[u8]) -> Result<Self>;
}
```

Blanket impls for all scalars; `derive(LeanType)` for user records and variants.

## 10.1. Mirroring Type System on the Lean Side

The Lean runtime library (`lake/Leo4`) exposes typeclasses that the plugin
discovers via `Lean.Meta.instanceExtension`:

```lean
class LeanMarshal (T : Type) where
  canonicalEncode : T → ByteArray → ByteArray
  -- Append `T`'s little-endian wire encoding to `buf`, return updated buffer.
  canonicalDecode : ByteArray → Nat → Except LeanError (T × Nat)
  -- Decode one `T` starting at offset `off` in `buf`; return value and the
  -- offset one past the value's last byte.

class LeanResource (T : Type)                          -- marker, no methods

-- Mutual exclusion: instance LeanMarshal T → no LeanResource T, and vice versa.
-- The plugin enforces this; users get a diagnostic at instance registration.
```

`LeanMarshal` and `LeanResource` cover **disjoint** populations: a type is
either marshalled inline (record / variant / enum / scalar — its bytes
cross the boundary) or held as a resource handle (its bytes never cross;
a `u64` handle does). The plugin's admit-set computation treats
`LeanMarshal` and `LeanResource` as separate `marshal` and `resource`
constraints (`LEO4-DESIGN.md §4.2`).

### `deriving LeanMarshal`

```lean
structure Point where
  x : Float
  y : Float
  deriving LeanMarshal           -- field encode/decode in declaration order

inductive Color where
  | red | green | blue
  deriving LeanMarshal           -- all-nullary → encoded as IDL `enum` (u32 case index)

inductive Tree where
  | leaf
  | node : Tree → Tree → Tree
  deriving LeanMarshal           -- self-recursive → encoded as IDL `variant`,
                                 -- recursive fields lower to `Self` in the IDL.
```

The deriving handler synthesises one `instance : LeanMarshal X` plus, on
the plugin side, an IDL declaration emitted into `<pkg>.leo4-schema`.
Field order, nominal-name disambiguation, recursion handling, and
generic-record mangling all follow `SPEC/mangling.md` and
`SPEC/canonical-abi.md`. Mutual recursion (two declarations that name each
other) is **not supported in v0** — the plugin rejects it; users break
the cycle with a `LeanResource` handle.

### `@[leo4_resource]` shorthand

```lean
@[leo4_resource]
opaque ParserHandle : Type
```

equivalent to a hand-written `instance : LeanResource ParserHandle := ⟨⟩`.

## 11. Out-of-Scope (v0)

- mathlib usage from Rust — out of v0; named subset deferred to
  Phase 8 (`ROADMAP.md`) on an opt-in, type-by-type basis. No
  general Mathlib reflection at the boundary, ever.
- Lean macros executing inside Rust process (definitely never)
- Lean tactic mode from Rust (would require LSP-style backend, separate project)
- Effect handlers, algebraic effects beyond `IO`
- Custom calling conventions (`extern "leo4"`)
- Async surfaces in the public API — out of v0; covered by Phase 7
  once WASIp3 stabilises (`ROADMAP.md`, D4).
- Mutual recursion between nominal types — out of v0 (§4.3); see
  Phase 6.

## 12. Open Questions Deferred to Implementation

- ~~`Lake.Module.recBuildLean` hook stability for the plugin (→ spike 0)~~
  **RESOLVED 2026-05-16** — we do not hook `recBuildLean`. The plugin is a
  `lean_exe` (`lake/Leo4Plugin/lakefile.lean` → `lean_exe leo4plugin`) that
  is invoked as `lake exe leo4plugin <user-module>` after Lake has built
  the user package's `.olean` files. The exe calls `Lean.initSearchPath`
  then `Lean.importModules (loadExts := true)` and walks the resulting
  `Environment` using only public Lean/Lake API. `recBuildLean` remained
  `private` across v4.27.0 → v4.29.1 and is the wrong integration point.
  See `spike/SPIKE-0-FINDINGS.md` for the full investigation and timing
  budget.
- Exact placement of Lake outputs in `target/` vs `build/` (→ Week 1)
- `cargo:rerun-if-changed=` granularity for Lake outputs (→ Week 3)
- Whether `bigint` should also be ABI-compatible with Rust's `num-bigint` layout
  (→ Week 5 or punt)
- **IDL output grouping in `<pkg>.leo4-schema`** — one `func` line per
  monomorphisation (current) vs one per generic signature with per-mono
  detail in `.leo4-mangling` only. Trade-off matrix and rationale in
  `ROADMAP.md` "Open question — deferred decision". 병익 is reviewing;
  do not change the emit shape until reopened.
