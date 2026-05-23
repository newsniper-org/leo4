# Rust-native Lean integration

> Pinned 2026-05-21. Defines an integration path for **Rust-
> native Lean implementations** — implementations that run as
> a Rust library in the same process as the leo4 caller. The
> path bypasses the `<lean/lean.h>` C ABI entirely and replaces
> the worker-IPC / shared-library dispatch with **direct Rust
> function calls**, while preserving the canonical-ABI wire
> format that cross-impl conformance depends on.
>
> Companion to `SPEC/lean-runtime-compat.md` (which defines the
> reference-Lean compat surface). Where that SPEC says "your
> Lean impl needs to satisfy §1.1–§1.4 to be a leo4 backend",
> this one says "or, if your Lean impl is Rust-native, satisfy
> the smaller §2 surface below and you get a much faster
> integration path."

## 0. Scope

leo4 currently has two transports for the IDL-defined
canonical-ABI wire format:

1. **leo4-native** (`SPEC/canonical-abi.md` §14): dynamic
   linker (`dlopen` of a `leanc`-built `.so`). Bytes flow
   through C ABI buffers exposed by the shim. Lean side speaks
   `<lean/lean.h>`.
2. **leo4-wasm** (`SPEC/wit/leo4-host.wit` C4.x.x): WASM
   Component Model. Bytes flow through `list<u8>` payloads on
   the WIT-defined `leo4:host/leo4-component@0.1.0` world.

This SPEC adds a third:

3. **leo4-rust-native**: in-process Rust function call. Bytes
   flow through `&[u8]` / `Vec<u8>` buffers — same process,
   same allocator, no marshalling beyond what the existing
   `LeanMarshal` impls already do. **No C ABI involved.**

The canonical wire format is unchanged across all three; only
the transport differs. schema_hash, mangling, and conformance
contracts (`tests/mangling/run.sh`,
`tests/conformance/run.sh`) all stay invariant.

## 1. Why a third path

Reference Lean is built in C++ + bootstrapped Lean and ships a
C runtime (`libleanshared`); the C ABI is the natural FFI
surface. **Rust-native Lean implementations** (today's
canonical example: [OxiLean](https://github.com/cool-japan/oxilean))
have a different shape:

- Runtime is a Rust library (`oxilean-runtime` in OxiLean's
  case), not a C shared library.
- Objects are Rust types managed by `Arc` / arena / pluggable
  GC, not `lean_object*`.
- "Calling a Lean function" is a Rust function call on the
  impl's `Env` / `Elab` / `Eval` API, not a `dlsym`'d C
  function.

For such impls, leo4-native's `<lean/lean.h>` requirement is
**a layer of indirection that buys nothing** — it would force
the impl to grow a `lean.h` compat shim purely to satisfy
leo4. leo4-rust-native sidesteps that by talking to the impl's
native Rust surface directly.

## 2. Required surface for a Rust-native Lean impl

The integration target ("rust-native impl") must expose a
single Rust trait + a small registration mechanism. Concrete
shape (this is the contract a leo4-rust-native backend's
adapter crate implements **against** the impl):

### 2.1 The `LeanProc` trait

```rust
/// One Rust-native Lean process / context. Owns the impl's
/// environment, elaborator state, and compiled bytecode.
/// Lifetime model: cheap to construct; cheap to clone
/// (typically `Arc`-cloned).
pub trait LeanProc: Send + Sync {
    /// Recompute the schema_hash from the loaded module's
    /// `@[leo4_export]` declarations. Must produce the same
    /// 13-char base32lc value `leo4-rust-emit` recorded into
    /// the corresponding `.leo4-handshake` JSON. Used during
    /// handshake verification (mirrors what the native shim
    /// and the wasm component's `verify-handshake` export do).
    fn schema_hash(&self) -> &str;

    /// `LEO4_ABI_VERSION` the impl was built against
    /// (currently always `1`).
    fn abi_version(&self) -> u32;

    /// Look up + invoke a `@[leo4_export]` body by its
    /// mangled name. `args` is canonical-ABI-encoded per
    /// `SPEC/canonical-abi.md`. Returns canonical-ABI-encoded
    /// result bytes on success; `LeanError` on dispatch
    /// failure or in-Lean exception.
    fn call(&self, mangled: &str, args: &[u8])
        -> Result<Vec<u8>, LeanError>;
}
```

The trait is **object-safe**. `Box<dyn LeanProc>` is the type
the leo4-rust-native dispatcher holds.

### 2.2 Registration: an adapter crate

For a given Rust-native impl X, the integration is a separate
**adapter crate** `leo4-X` (e.g. `leo4-oxilean`) that:

1. `impl LeanProc for X::SomeContext { … }` — implements the
   trait above by delegating to the impl's native API.
2. Optionally: provides a `LeanProc`-flavoured wrapper around
   the impl's source-loading entry point so the leo4 plugin's
   `lake exe leo4plugin` analogue can register
   `@[leo4_export]`s. This is the equivalent of
   `SPEC/lean-runtime-compat.md` §1.1's meta-programming API
   contract — adapters that don't expose this can't run the
   leo4 plugin and must rely on the user package shipping a
   pre-emitted handshake / mangling JSON pair.

The adapter crate lives **outside** the main leo4 workspace.
leo4 itself stays runtime-agnostic.

### 2.3 What's NOT in the surface

Crucially, the rust-native path does **not** require the impl
to expose:

- `<lean/lean.h>` C symbols (`lean_box`, `lean_alloc_ctor`,
  etc.). The trait above hands buffers, not lean_objects.
- `leanc` toolchain. The impl's own compile path (whatever it
  is — JIT, AOT, bytecode interp) handles compilation.
- Lake DSL. The impl can use whatever build system it wants.
- `@[extern]` lowering. Reverse direction goes through a
  separate `LeanProcInvoker` trait (next section), not Lean-
  side `@[extern]`.
- `dlopen` capability. No dynamic linking happens.

That covers §1.2 / §1.3 / §1.4 of `SPEC/lean-runtime-compat.md`
collectively. The impl only needs to satisfy the **§1.1
analogue** (its own meta-programming surface, expressed as the
trait above instead of a Lean meta-monad API).

## 3. Reverse direction — `LeanProcInvoker`

When leo4 reverse direction is used with a rust-native target,
the Lean side (running inside the impl) needs to invoke Rust
functions registered via `#[leo4::export]`. The native
pipeline does this through `@[extern]` + worker IPC; the wasm
pipeline does it through WIT `host-imports`. The rust-native
pipeline does it through a registered Rust callback:

```rust
/// Host-side hook the rust-native impl calls when its Lean
/// code reaches a `leo4_rust_call_lean(mangled, args)`
/// equivalent. Registered with the `LeanProc` instance at
/// construction time; one callback covers all
/// `#[leo4::export]`s of the linked cdylib.
pub trait LeanProcInvoker: Send + Sync {
    fn invoke(&self, mangled: &str, args: &[u8])
        -> Result<Vec<u8>, LeanError>;
}
```

Adapter crates wire this through the impl's own FFI / host-
function mechanism. For OxiLean: register the callback against
`oxilean-runtime`'s native function table.

Re-entrant calls (Lean → Rust → Lean, for Phase 10-B1
callback ABI) work transparently because everything is
**in-process Rust function calls** — no IPC frames to design,
no `LECQ` / `LECR` (`SPEC/reverse-direction.md` §10a) protocol
needed.

## 4. schema_hash invariant

leo4's cross-impl conformance contract
(`SPEC/mangling.md` §3) requires that the same IDL produces
byte-identical schema_hash on every implementation. The rust-
native path preserves this:

- The IDL is unchanged.
- The mangling rules are unchanged.
- The `LeanMarshal` typeclass on the Lean side produces the
  same bytes (the impl is responsible for satisfying that —
  it's part of the source-syntax compat surface).
- The `LeanMarshal` trait on the Rust side
  (`crates/leo4-abi`) is shared.
- `LeanProc::schema_hash()` returns the same string as the
  emit CLI recorded.

`tests/mangling/run.sh` runs against the reference Lean impl;
an analogous test for a rust-native impl would invoke its
`LeanProc::schema_hash()` and compare against the leo4c-
computed value. Adding such a test for OxiLean (once OxiLean
ships a `leo4-oxilean` adapter) is the canonical verification
step.

## 5. Why canonical ABI bytes, not typed Rust args

A more aggressive design would have `LeanProc::call` take and
return **typed Rust values** directly — no `LeanMarshal`
encode/decode roundtrip. Considered and rejected:

| Option | Bytes-on-the-wire | Cross-impl conformance | dev cost |
|---|---|---|---|
| **(A) — bytes-in, bytes-out** ← chosen | preserved | preserved (same `LeanMarshal` impls on both sides) | small (one trait, no codegen) |
| (B) — typed-in, typed-out | bypassed (zero-copy) | broken (the wire format is no longer a thing for this transport) | large (per-IDL Rust-trait codegen, new generator in `leo4-rust-emit`) |

(A) keeps the schema_hash + cross-impl conformance invariants
that the rest of the SPEC stack relies on. The marshalling
overhead is small in-process (~tens of ns for typical leo4
payloads — `Vec<u8>` copy + `LeanMarshal` codec) and is
dominated by dispatch / type-class resolution costs anyway.

(B) might be revisited when there's a measured-and-real
performance gap that (A) can't cross. Today it's premature.

## 6. Comparison table — three paths

| Aspect | leo4-native | leo4-wasm | leo4-rust-native |
|---|---|---|---|
| Transport | `dlopen` + shim | wasm Component Model | direct Rust call |
| C ABI dep | yes (`lean.h`) | no | **no** |
| Cross-process? | no (in same process) | no | no |
| Async? | sync via worker | sync (CM API) | sync |
| Marshalling | canonical-ABI bytes | canonical-ABI bytes | **canonical-ABI bytes** |
| Schema-hash check | runtime via handshake JSON | runtime via `verify-handshake` export | runtime via `LeanProc::schema_hash()` |
| Re-entrant callbacks | via worker IPC frames (Phase 10-B1.x) | via WIT `host-imports` (deferred) | **trivial — Rust function call** |
| Code in this repo | `crates/leo4-native`, `shim/leo4_rust_bridge*.c` | `crates/leo4-wasm` | **none** — adapter is out-of-tree |
| Adapter location | n/a (it IS leo4) | n/a (it IS leo4) | separate `leo4-<impl>` crate per impl |

## 7. First implementor candidate — OxiLean

OxiLean is the canonical target as of 2026-05-21:

- **`oxilean-runtime`** holds the impl's `Env` / `Elab` /
  `Eval` state. A `leo4-oxilean::OxiLeanProc` struct wraps it
  and `impl LeanProc for OxiLeanProc`.
- **`oxilean-elab/src/lean4_compat/`** + Mathlib4's 99.7%
  parse rate suggest `@[leo4_export]` + `deriving LeanMarshal`
  recognition is feasible at source-syntax level.
- **`oxilean-meta/src/synth_instance/`** provides the trait-
  based typeclass synthesis the leo4 admit-set algorithm
  needs.
- **`oxilean-elab/src/attribute/`** + `derive/` registrability
  let the `leo4-oxilean-build` analogue (whoever writes it)
  scan a user package's `@[leo4_export]`s and emit handshake +
  mangling JSON.

The adapter crate would live at e.g.
`github.com/Honey-Be/leo4-oxilean` (separate repo) and depend
on both `oxilean-runtime` and `leo4-abi` (path-deps or
crates.io once leo4 publishes). leo4 itself doesn't change.

## 8. Activation plan (not committed)

This SPEC defines the contract but doesn't commit leo4 to any
particular timeline. Activation requires:

1. OxiLean reaches a maturity bar where its API surface is
   stable enough for a downstream adapter to pin.
   (`//! Auto-generated module structure` headers will need to
   become hand-curated APIs first, OR a release with a stable
   public API contract.)
2. A concrete consumer wants the rust-native path. The
   reference-Lean + leo4-wasm paths cover today's needs.
   Activation pressure typically comes from "wants in-process
   Rust speed", "wants to avoid C toolchain complexity", or
   "wants to embed Lean in a long-running Rust server".
3. Someone (OxiLean maintainers, leo4 contributors, or a
   third party) writes the `leo4-oxilean` adapter crate.

Until all three happen, this SPEC is forward-looking
documentation. The three-paths comparison table in §6 stays
relevant the moment any rust-native impl maturity bar is met.

## 9. Cross-references

- `SPEC/lean-runtime-compat.md` — the surface for C-ABI / Lake
  / `leanc` based integrations (reference Lean today, future
  C-compat-shim'd rust-native impls).
- `SPEC/canonical-abi.md` — the wire format all three
  transports carry.
- `SPEC/mangling.md` — schema_hash + mangling that stays
  invariant across transports.
- `SPEC/wit/leo4-host.wit` — the leo4-wasm transport's WIT
  contract; the rust-native path is the analogue for in-
  process Rust impls.
- `LEO4-DESIGN.md` — D16 (reverse direction); the rust-native
  reverse direction is the same logical pipeline collapsed
  into a function call.
