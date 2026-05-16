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
| D4 | Async model | Sync only until WASIp3 stabilizes; `io<T>` lowers to `result<T, error>` for now |
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
- Dependent function types where the codomain depends on a value of the domain
- Non-`Type 0` types
- Recursive constraints (e.g., `T : Marshal` requiring `T → T : Marshal`)
- Open-ended negation (`¬(T : Marshal)`)

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
      admit(f, i) := evaluate(ci) over current environment
      // closed-form for `scalar` etc.
      // type class enumeration via Lean.Meta.SynthInstance.getInstances

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
mangle(f<T1,…,Tn>) =
    "leo4__" ++ pkg ++ "__" ++ iface ++ "__" ++ fname ++
    "__" ++ join("_", map(mangle_type, [T1,…,Tn])) ++
    "__h" ++ base32lc( first_8_bytes( blake3(normalized_idl) ) )

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

### Normalized IDL form (input to BLAKE3)

1. Strip comments and doc strings
2. Strip whitespace down to canonical single spaces
3. Sort `import`s lexicographically
4. Inline all `type` aliases
5. Order `interface` members canonically (alphabetic by name)
6. Emit as UTF-8 byte stream

The 8-byte BLAKE3 prefix in the mangled name **acts as the schema handshake**.
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

- Loads a `.so`/`.dylib`/`.dll` shim produced by Lake.
- Shim is a thin C layer over `lean.h`; the *shim* is version-locked to Lean,
  the Rust crate is not.
- Uses `libloading` for dynamic load (lets users swap shims at runtime).

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

## 11. Out-of-Scope (v0)

- mathlib usage from Rust (likely never; subset only)
- Lean macros executing inside Rust process (definitely never)
- Lean tactic mode from Rust (would require LSP-style backend, separate project)
- Effect handlers, algebraic effects beyond `IO`
- Custom calling conventions (`extern "leo4"`)

## 12. Open Questions Deferred to Implementation

- `Lake.Module.recBuildLean` hook stability for the plugin (→ spike 0)
- Exact placement of Lake outputs in `target/` vs `build/` (→ Week 1)
- `cargo:rerun-if-changed=` granularity for Lake outputs (→ Week 3)
- Whether `bigint` should also be ABI-compatible with Rust's `num-bigint` layout
  (→ Week 5 or punt)
