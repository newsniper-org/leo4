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

1. **leo4-mslean4** (`SPEC/canonical-abi.md` §14): dynamic
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

For such impls, leo4-mslean4's `<lean/lean.h>` requirement is
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

| Aspect | leo4-mslean4 | leo4-wasm | leo4-rust-native |
|---|---|---|---|
| Transport | `dlopen` + shim | wasm Component Model | direct Rust call |
| C ABI dep | yes (`lean.h`) | no | **no** |
| Cross-process? | no (in same process) | no | no |
| Async? | sync via worker | sync (CM API) | sync |
| Marshalling | canonical-ABI bytes | canonical-ABI bytes | **canonical-ABI bytes** |
| Schema-hash check | runtime via handshake JSON | runtime via `verify-handshake` export | runtime via `LeanProc::schema_hash()` |
| Re-entrant callbacks | via worker IPC frames (Phase 10-B1.x) | via WIT `host-imports` (deferred) | **trivial — Rust function call** |
| Code in this repo | `crates/leo4-mslean4`, `shim/leo4_rust_bridge*.c` | `crates/leo4-wasm` | **none** — adapter is out-of-tree |
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

### 7.1 OxiLean FFI deep-dive (2026-05-21)

OxiLean ships a complete FFI infrastructure split across two
layers, both already in v0.1.2:

**Kernel layer** (`crates/oxilean-kernel/src/ffi/`) — the
semantic model:

```rust
// FFI-compatible types. Matches leo4's IDL primitives 1-to-1.
pub enum FfiType {
    Bool,
    UInt8, UInt16, UInt32, UInt64,
    Int8,  Int16,  Int32,  Int64,
    Float32, Float64,
    String,         // null-terminated in C
    ByteArray,      // ← canonical-ABI bytes go here
    Unit,
    Ptr(Box<FfiType>),
    Fn(Vec<FfiType>, Box<FfiType>),   // ← first-class function pointer!
    OxiLean(String),                  // opaque (≈ leo4 LeanResource)
}

// Runtime values that cross the boundary.
pub enum FfiValue {
    Bool(bool), UInt(u64), Int(i64), Float(f64),
    Str(String), Bytes(Vec<u8>),     // ← canonical-ABI bytes payload
    Unit,
}

// ExternDecl + ExternRegistry: where host functions live.
pub struct ExternDecl {
    name: Name,                       // Lean-side fn name
    lean_type: Expr,                  // Lean-level type
    lib_name: String,                 // e.g. "libc" or "leo4-rust-bridge"
    symbol_name: String,              // C symbol (the mangled name)
    safety: FfiSafety,                // Safe / Unsafe / System
    calling_convention: CallingConvention, // Rust / C / System
    signature: FfiSignature,          // params + return
}

pub struct ExternRegistry { /* HashMap<Name, ExternDecl> + lookup */ }

// FfiValue ↔ Expr conversion (both directions exposed).
impl FfiValue {
    pub fn try_from_expr(expr: &Expr, ty: &FfiType) -> Result<Self, FfiError>;
    pub fn to_expr(&self) -> Expr;
}
```

**Codegen layer** (`crates/oxilean-codegen/src/ffi_bridge/`)
— the mechanical wiring. Notably, `marshal_type(lcnf_ty)`
emits C calls against **the reference Lean C ABI**:

```rust
pub fn marshal_type(lcnf_ty: &LcnfType) -> FfiMarshalInfo {
    match lcnf_ty {
        LcnfType::Nat => FfiMarshalInfo::with_conversion(
            FfiNativeType::U64,
            "lean_unbox(${arg})",
            "lean_box(${result})",
        ),
        LcnfType::LcnfString => FfiMarshalInfo {
            native_type: FfiNativeType::CStr,
            to_native: "lean_string_cstr(${arg})".to_string(),
            from_native: "lean_mk_string(${result})".to_string(),
            …
        },
        LcnfType::Object => FfiMarshalInfo::trivial(FfiNativeType::LeanObject),
        …
    }
}
```

i.e. OxiLean's *generated code* speaks `lean_box`,
`lean_unbox`, `lean_string_cstr`, `lean_mk_string`,
`lean_object*` — the same surface
`SPEC/lean-runtime-compat.md` §1.2 lists. This means
OxiLean's compile output is on a path to be link-compatible
with leo4's existing forward-direction shim if
`oxilean-runtime` exposes those symbols (currently unclear
from public docs; verifying this is a key open question for
the adapter author).

**Implications for the leo4-rust-native trait surface**:

| `LeanProc` / `LeanProcInvoker` need | OxiLean provides | Adapter work |
|---|---|---|
| `LeanProc::call(mangled, args: &[u8])` returning `Vec<u8>` | `ExternRegistry::lookup(mangled)` + invocation via `FfiValue::Bytes(args.to_vec())` | thin wrapper |
| `LeanProc::schema_hash()` | Plugin scans `@[leo4_export]`s via OxiLean's attribute API; reuses leo4's `schema-idl` to compute the hash | the existing leo4 plugin's algorithm transferable |
| `LeanProcInvoker::invoke(mangled, args)` | Register an `ExternDecl { name: …, lib_name: "leo4-rust-bridge", symbol_name: mangled, calling_convention: C, signature: FfiSignature{params: [Bytes], ret: Bytes} }` in OxiLean's `ExternRegistry`; the impl pointer is a closure that calls the leo4 dispatcher | one-time bulk registration of all `#[leo4::export]`s at adapter startup |
| Phase 10-B1 callback ABI (function-arrow ABI) | OxiLean already has `FfiType::Fn(Vec<FfiType>, Box<FfiType>)` — first-class function pointers | leo4-rust-native's callback path is trivial: thread a Rust closure through `FfiValue::Fn`-equivalent |

**Surprising finding — `Fn(params, ret)` is first-class in
OxiLean's FFI from day one**. leo4-rust-native's callback
ABI is essentially free with OxiLean as the impl, in
contrast to native (B1.x re-entrant IPC frames) or wasm (WIT
host-imports). This makes OxiLean potentially the
*lowest-friction* integration point for the adsmt SMT-solver
use case once a `leo4-oxilean` adapter exists.

**Direct-inspection results (2026-05-21)** — three hooks
needed for full leo4-rust-native integration; grep into
OxiLean v0.1.2 sources verified which exist:

| Hook | Status in v0.1.2 | Location of evidence |
|---|---|---|
| **(1) callback registration** — closure storage in evaluator | **NOT PRESENT** | `oxilean_kernel::ffi::ExternRegistry` + `oxilean_runtime::closure::FunctionTable` both metadata-only |
| **(2) by-name dispatch** — `Env::call_by_mangled_name` analogue | **NOT PRESENT (high-level)** | `Environment` public API is query/merge only; runtime is `BytecodeChunk`-level (`execute_chunk`), not name-level |
| **(3) attribute / deriving registration** — `registerBuiltinAttribute` analogue | **PRESENT** | `oxilean_elab::attribute::AttributeManager::register_custom_handler(AttrHandler)` + `DeriveHandlerRegistry::register(DeriveHandler)` |

So 1-of-3 hooks ships today. Implications:

* The **recognition layer** (scanning a user package for
  `@[leo4_export]` + `deriving LeanMarshal` to emit
  handshake JSON) is **unblocked** — a separate
  `leo4-oxilean-build` companion crate can bind Hook 3
  and ship today.
* The **dispatch layer** (`LeanProc::call` +
  `LeanProcInvoker::invoke` actually running) **stays
  blocked on Hooks 1 + 2** until upstream PRs land. The
  scaffold adapter at `sibling/leo4-oxilean/` currently
  surfaces these as `LEO4_ERR_RUST_DLSYM_FAILED`
  (0x0002_0005) stubs.

Three remaining **maturity** questions an adapter author
still needs to settle (in addition to the dispatch hooks):

1. **Does `oxilean-runtime` link-expose `lean_box` /
   `lean_unbox` / `lean_string_cstr` / `lean_mk_string` /
   `lean_object*` / `lean_alloc_ctor` as its public C
   surface?** `marshal_type` emits string templates against
   these names, but whether `oxilean-runtime` provides
   their actual implementations or expects to call out to
   `libleanshared` for them is unclear. If the former,
   `SPEC/lean-runtime-compat.md` §1.2 is *also* satisfied
   and the leo4-mslean4 path can run unchanged against
   OxiLean. If the latter, leo4-rust-native is the only
   working path.
2. **Does `oxilean-cli` / `oxilean-build` accept Lake-
   shaped project layouts?** If yes, leo4's existing Lake
   plugin can also drive OxiLean. If no, the adapter
   writes a bridge.
3. **Is the `lean4_compat/` layer mature enough to accept
   the `lake/Leo4/` runtime library as-is?** Single best
   litmus test for an adapter is "can OxiLean elaborate
   `lake/Leo4/Leo4/Export.lean` without modification?"

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

## 9. Transpile path — `oxilean-codegen::rust_target_backend` (2026-05-21)

A **second route** to the same in-process-direct-Rust-call
endpoint, discovered while inspecting OxiLean v0.1.2's codegen
suite. Instead of waiting on the three "evaluator hooks" from
§7.1 (callback storage / by-name dispatch / etc.), this route
bypasses OxiLean's evaluator entirely by **transpiling** the
Lean source into a plain Rust crate at build time, then
calling its `pub fn`s as ordinary in-process Rust functions.

### 9.1 The codegen pipeline

`oxilean-codegen` ships ~50 backends including
`rust_target_backend`. The relevant public API:

```rust
use oxilean_codegen::rust_target_backend::{RustTargetBackend, RustModule, RustFn, RustType};
use oxilean_codegen::lcnf::{LcnfFunDecl, LcnfModule, LcnfType};

let mut backend = RustTargetBackend::new();
// `decls` comes from oxilean-elab's LCNF lowering of the
// user package's Lean source.
let rust_mod: RustModule = backend.emit_module("user_pkg", &decls);

// Per-fn lowering also exposed:
let rust_fn: RustFn = backend.compile_decl(&decl)?;
println!("{}", rust_fn);   // → real Rust source via Display
```

The critical mapping (`RustTargetBackend::lcnf_to_rust_type`):

```rust
LcnfType::Nat        → RustType::U64           // not lean_box_uint64!
LcnfType::LcnfString → RustType::RustString    // not lean_string_cstr!
LcnfType::Unit       → RustType::Unit
LcnfType::Object     → RustType::Custom("Box<dyn std::any::Any>")
LcnfType::Fun(p, r)  → RustType::Fn(p, r)      // first-class function type
LcnfType::Ctor(n, a) → RustType::Custom(n) | Generic(n, a)
```

**No `lean_*` C symbol appears anywhere in the output.** The
generated Rust crate has zero dependency on `<lean/lean.h>` or
on OxiLean's own runtime — it's just a normal Rust crate that
happens to have been emitted from Lean source.

`RustFn::emit() -> String` serialises one function to actual
Rust source code; `impl fmt::Display for RustFn` makes it
`format!`-friendly. `RustModule` collects multiple `RustItem`s
and serialises end-to-end.

### 9.2 What this path bypasses

Compared to §7.1's three OxiLean evaluator hooks (1 present,
2 absent), the transpile path **bypasses all three**:

| Evaluator hook (§7.1) | Why irrelevant for transpile |
|---|---|
| (1) Callback storage in ExternRegistry | Lean fn is already a Rust fn after transpile — call directly. |
| (2) `Env::call_by_mangled_name` high-level entry | Rust fn is `pub`-imported, no name dispatch needed. |
| (3) Attribute / deriving registration (PRESENT) | Still useful — same plugin discovers `@[leo4_export]`s and tells the transpiler which fns to emit + how. |

So **transpile becomes the more practical leo4-rust-native
activation path today** — Hooks 1 + 2 stay deferred upstream
forever, this path doesn't need them.

### 9.3 Architecture under transpile

```
[Lean source: lake/Leo4/Leo4/* + user package's @[leo4_export]s]
        │ oxilean-elab::lean4_compat (textual pre-processor:
        │                              `Lean4TermRewriter::standard`
        │                              + `Lean4SyntaxAdapter::adapt_all`
        │                              normalise ` => → -> `, `← → <-`,
        │                              `where; → where`, `∧∨¬ → &&||!`)
        ▼
[Lean source in OxiLean parser dialect]
        │ oxilean-parse::Parser::parse_decl
        ▼
[Located<Decl> (parser AST)]
        │ leo4_oxilean_build::decl_has_leo4_export
        │   → Decl::Attribute { attrs, .. } inspection
        │   → tagged decls recorded into Leo4ExportRegistry
        │     (Hook 3: AttributeManager + DeriveHandlerRegistry
        │      pre-populated with @[leo4_export] custom handler
        │      + LeanMarshal derive handler)
        │   → untagged decls returned as Ok(None) — skipped
        ▼
[Tagged Located<Decl>]
        │ oxilean-elab::elaborate_decl (unwraps Decl::Attribute,
        │                               elaborates inner decl)
        ▼
[OxiLean Env + PendingDecl]
        │ leo4_oxilean_build::unfold_decl + decl_to_lcnf
        │ oxilean-codegen::lcnf normalisation
        ▼
[LcnfFunDecl[]]
        │ RustTargetBackend::compile_decl → RustFn (Rust AST)
        ▼
[RustFn]
        │ RustFn::emit() → "pub fn <name>(args...) -> R { ... }"
        │ + leo4_oxilean_build::synthesize_canonical_wrapper
        │     → "pub fn <name>_call(args: &[u8])
        │         -> Result<Vec<u8>, LeanError> { ... }"
        │     decode-each-arg → call → encode-return
        ▼
[Pair of Rust source strings per export]
        │ leo4-oxilean-build helper crate orchestrates:
        │   * Cargo.toml setup with leo4-abi dep
        │   * LeanProc impl emitting a mangled-name → _call
        │     dispatch table (§6 — next)
        ▼
[A user-facing crate that exposes the original Lean exports
 as ordinary Rust pub fns + their canonical-ABI shims, ready
 to import + call directly OR plug into a `LeanProc` host.]
```

The result is *not* a `LeanProc`-style dispatcher — it's a
normal Rust library. The `LeanProc` / `LeanProcInvoker` traits
defined in §2 + §3 remain the *runtime-dispatch* model for
when an evaluator-based impl materialises; the **transpile
model is its zero-runtime sibling**, achieving the same
in-process-direct-Rust-call goal through a different
mechanism.

### 9.4 Adapter layout — two complementary crates

Per the model split:

| Crate | Path | Responsibility | OxiLean deps |
|---|---|---|---|
| `leo4-oxilean` | `sibling/leo4-oxilean/` | Runtime-dispatch adapter implementing `LeanProc` / `LeanProcInvoker`. Blocked on Hooks 1 + 2 today. | `oxilean-kernel`, `oxilean-runtime` |
| **`leo4-oxilean-build`** | **`sibling/leo4-oxilean-build/`** | **Build-time transpiler.** Reads a Lean source tree, walks `@[leo4_export]`s, emits a Cargo crate via `RustTargetBackend`. Unblocked today. | `oxilean-parse`, `oxilean-elab`, `oxilean-codegen` |

The two are parallel — a user picks one depending on whether
they need dynamic dispatch (evolving Lean code at runtime; not
common) or compile-time-frozen exports (the SMT-solver-style
adsmt use case). leo4 IDL + canonical-ABI marshalling +
schema_hash check apply equally to both.

### 9.5 Other backends inspected, NOT viable for this path

| Backend | Why not |
|---|---|
| `native_backend` | Emits machine-level IR (`NativeInst`, `Register`, `BasicBlock`) — register-allocated post-instruction-selection layer. Not Rust source. Would need a `NativeBackend → cargo`-equivalent that doesn't exist. |
| `c_backend` | Emits C source against `oxilean-runtime`'s ABI shape (which has overlap with `lean.h` but isn't identical) — see also `marshal_type` in `oxilean-codegen::ffi_bridge/functions.rs` that emits `lean_box`/`lean_unbox`/etc. against a "lean.h-shaped" ABI. Closer to leo4-mslean4 than to leo4-rust-native — different transport, not in-process direct call. |
| `llvm_backend` / `cranelift_backend` | Third-party IR with their own toolchain deps. Out of scope for a minimal adapter. |
| `lean4_backend` | Round-trip back to Lean 4 source — useless for this purpose. |
| `wasm_backend` (if any) | Routes through `leo4-wasm` transport, not this one. |

Confirmed via direct grep into `oxilean-codegen` v0.1.2: ~50
backends total, **only `rust_target_backend` produces plain
Rust source**.

### 9.6 Open questions for the transpile path

- [x] **Does `oxilean-elab` parse arbitrary Lean 4 source
      reliably?** Partial — `oxilean-elab::lean4_compat` v0.1.2
      provides a *textual* pre-processor (`Lean4TermRewriter::
      standard` + `Lean4SyntaxAdapter::adapt_all`) that handles
      arrow / bind / where / logic-op surface differences, but
      *not* parser-level shape differences. Specifically,
      OxiLean's `parse_definition` accepts
      `def name {univs} : type := value` only — Lean 4
      header binders `def f (x : T) : R := body` need an
      *AST-level* lift above the parser (not yet implemented).
      The leo4 runtime library (`lake/Leo4/Leo4/*.lean`) is
      written in OxiLean-native body-lambda shape; user
      packages that use header-binder syntax will need a
      pre-pass. Wired in `sibling/leo4-oxilean-build/src/lib.rs::lean4_normalize`
      (2026-05-22).
- [x] **Does the LCNF lowering preserve attribute metadata
      (specifically `@[leo4_export]` tags)?** No — upstream
      `elaborate_decl` v0.1.2 unwraps `Decl::Attribute { attrs,
      decl }` and discards the outer `attrs`. The inner
      `Decl::Definition.attrs` field is left empty by the
      parser, so attribute info doesn't reach `PendingDecl` /
      LCNF. **leo4's wiring works around this** by inspecting
      the parser AST *before* elaboration
      (`decl_has_leo4_export` + `inner_decl` in
      `sibling/leo4-oxilean-build/src/lib.rs`), recording the
      tag in a separate `Leo4ExportRegistry::manager`, then
      elaborating the inner decl normally. The tag survives
      in the registry, not in the elab output. Wired
      2026-05-22.
- [ ] **How does the transpiled Rust handle Lean
      `Nat`-vs-`UInt64` distinction?** `lcnf_to_rust_type`
      maps both to `u64`; that's fine for leo4's IDL surface
      (we never expose unbounded Nat at the boundary —
      `bignat` goes through `LeanMarshal`'s byte serialisation
      anyway).
- [ ] **Cross-impl conformance**: does the transpiled output
      produce byte-identical canonical-ABI encoding to
      reference Lean's `leo4-mslean4` path on the same IDL?
      The answer should be yes — both sides use
      `leo4-abi`'s `LeanMarshal` impls — but it's worth a
      conformance fixture once the pipeline is end-to-end.

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
