# leo4-oxilean

> Adapter: [leo4](https://github.com/Honey-Be/leo4) ↔
> [OxiLean](https://github.com/cool-japan/oxilean) via
> `SPEC/rust-native-lean.md`'s `LeanProc` /
> `LeanProcInvoker` trait surface.
>
> **Status (2026-05-21)**: SCAFFOLD. Compiles + tests pass,
> but every dispatch returns a stub error. See §Activation
> below before relying on it.

## What this is

leo4 defines three transports for its
canonical-ABI interop (`SPEC/canonical-abi.md`):

| Transport | Crate | C ABI? | Implementor location |
|---|---|---|---|
| `leo4-mslean4` | `crates/leo4-mslean4` | yes (`<lean/lean.h>`) | in-tree |
| `leo4-wasm` | `crates/leo4-wasm` | no (WASM Component Model) | in-tree |
| **`leo4-rust-native`** | adapter crates like this one | **no — direct Rust call** | **out-of-tree** |

This crate is the first concrete `leo4-rust-native` adapter,
targeting OxiLean — a pure-Rust Calculus-of-Inductive-
Constructions implementation that targets Lean 4 source
compatibility. Per `SPEC/rust-native-lean.md` §2.2, every
rust-native adapter lives **outside** the main leo4
workspace, which is why this crate sits at
`sibling/leo4-oxilean/` with its own `[workspace]` marker
rather than as a workspace member.

## What this is NOT

This is **not** a leo4 backend you can drop in today. The
trait surface compiles; the **invoker side wraps a real
OxiLean `ExternRegistry`** (export-metadata registration is
functional + tested); but the actual **dispatch
direction** — calling either way across the boundary —
needs hooks OxiLean v0.1.2 doesn't yet expose. Activation
work below.

## What works today (8 / 8 tests passing)

- `OxiLeanInvoker::new()` constructs an
  `Arc<Mutex<oxilean_kernel::ffi::ExternRegistry>>` wrapper.
- `register_export(mangled)` pushes one `ExternDecl` per
  `#[leo4::export]` into the OxiLean registry under
  `lib_name = "leo4-rust-bridge"`, `symbol_name = mangled`,
  signature `(ByteArray) -> ByteArray` (the canonical-ABI
  shape every leo4 export collapses to).
- Duplicate-symbol detection (registry rejects, adapter
  surfaces `ENCODE_ERROR` 0x02).
- `LeanProcInvoker::invoke` distinguishes
  `UNKNOWN_FUNCTION` (export not registered) from
  `RUST_DLSYM_FAILED` 0x0002_0005 (registered but no
  upstream callback hook).
- All trait surfaces are object-safe per
  `SPEC/rust-native-lean.md`.

## OxiLean upstream prerequisite — direct inspection results

Three hooks needed for full integration; **direct grep into
OxiLean v0.1.2 sources verified which exist** (2026-05-21):

- [ ] **Hook 1 — Callback-registration entry point in the
      OxiLean evaluator** for `ExternRegistry` symbols.
      **Status: NOT PRESENT in v0.1.2.**
      `oxilean_kernel::ffi::ExternRegistry::register(decl)`
      stores metadata only — `decl.lib_name` /
      `decl.symbol_name` describe *where* the actual symbol
      lives; the codegen / evaluator resolves it via
      `dlsym(lib, symbol)` at runtime.
      `oxilean_runtime::closure::FunctionTable` (the
      parallel "function decl" registry) is the same
      shape — `FunctionEntry { name, arity, convention,
      is_builtin, … }`, no closure storage.
      `leo4-rust-native`'s in-process direct-call model
      needs OxiLean to accept a
      `Box<dyn Fn(&[u8]) -> Result<Vec<u8>, _>>` closure
      *per mangled name*, dispatching into it instead of
      doing `dlsym`. Suggested API:
      `ExternRegistry::register_callback(decl, callback)`.
- [ ] **Hook 2 — By-name `@[leo4_export]` dispatch in the
      OxiLean evaluator**.
      **Status: NOT PRESENT (at high-level API surface)
      in v0.1.2.**
      `Environment`'s public API (30+ functions inspected)
      is all metadata / query (`merge_environments`,
      `filter_environment`, `constants_with_prefix`,
      `contains_any`, …); no `Env::call_by_name` / `run` /
      `invoke` entry point.
      What's there at runtime side:
      `oxilean_runtime::bytecode_interp::execute_chunk(
      &BytecodeChunk)` and a wasm-side
      `execute_function(...)` — both chunk-level, not
      name-level. An adapter would either have to assemble
      a `BytecodeChunk` for each call (deep + brittle) or
      wait for an upstream high-level wrapper. Suggested
      API: `Env::call_by_mangled_name(name, ffi_args) ->
      FfiValue`.
- [x] **Hook 3 — Attribute / deriving registration**
      (the `registerBuiltinAttribute` analogue).
      **Status: PRESENT in v0.1.2.**
      `oxilean_elab::attribute::AttributeManager::
      register_custom_handler(handler: AttrHandler)` lets
      adapters register a custom attribute name with a
      string-based `AttrAction`.
      `oxilean_elab::attribute::DeriveHandlerRegistry::
      register(handler: DeriveHandler)` accepts a custom
      `deriving` handler keyed by class name — direct
      analogue of reference Lean's
      `registerDerivingHandler`.
      *This means `@[leo4_export]` recognition and
      `deriving LeanMarshal` are achievable today inside a
      `leo4-oxilean-elab-plugin` companion crate. They're
      the only piece of the three that doesn't block on
      OxiLean upstream PRs.*

Adapter implications:

* The **forward-direction recognition layer** (a
  `leo4-oxilean-build` analogue: scan a user package for
  `@[leo4_export]`-tagged decls + emit handshake JSON) is
  *unblocked* — it can ship today by binding
  `AttributeManager::register_custom_handler("leo4_export",
  ...)` and `DeriveHandlerRegistry::register(...)` for
  `LeanMarshal`.
* The **actual dispatch layer** (`OxiLeanProc::call` +
  `OxiLeanInvoker::invoke`) stays blocked on Hooks 1 & 2.
  Until they land upstream, this adapter's traits return
  the `RUST_DLSYM_FAILED` (0x0002_0005) stubs that the
  current 8 tests pin down.

Tracking: `SPEC/rust-native-lean.md` §7.1 + §8 reflects
this 1-of-3 status. If you (or anyone) upstreams Hooks 1 +
2 to OxiLean, ping the leo4 maintainers and this adapter's
`LeanProc` / `LeanProcInvoker` bodies fill in
transparently.

## Activation checklist (orthogonal questions)

Beyond the three upstream hooks above, three OxiLean-side
**maturity questions** still gate full integration (per
`SPEC/rust-native-lean.md` §7.1):

- [ ] **`oxilean-runtime` link-exposes `lean_box`-family C
      symbols** (vs. delegating to `libleanshared`)? If yes,
      `crates/leo4-mslean4` can also run unmodified against
      OxiLean and this adapter becomes one of two paths. If
      no, this adapter is the only working path against
      OxiLean.
- [ ] **`oxilean-cli` / `oxilean-build` accept Lake-shaped
      project layouts**? If yes, the existing
      `lake/Leo4Plugin` plugin drives OxiLean unchanged. If
      no, a thin bridge is needed.
- [ ] **`oxilean-elab/src/lean4_compat/` elaborates
      `lake/Leo4/Leo4/Export.lean` as-is**? Single best
      litmus test for adapter activation. If pass:
      `@[leo4_export]` recognition + `deriving LeanMarshal`
      transfer transparently; otherwise the adapter has to
      stub OxiLean-specific equivalents.

Once those answers + the three upstream hooks are in, the
work the adapter itself does (approximate, may evolve):

- [ ] Uncomment the `oxilean-*` deps in `Cargo.toml` and pin
      to a specific OxiLean release (their `//! Auto-
      generated module structure` headers signal trait
      surfaces can move between versions).
- [ ] Replace `OxiLeanProc::new_stub` with `OxiLeanProc::new(
      env: Arc<oxilean_runtime::Env>, handshake_path: &Path)`.
- [ ] Implement `LeanProc::call` by resolving the mangled
      export in `oxilean-runtime`'s env and invoking via
      `FfiValue::Bytes(args.to_vec())` →
      `oxilean_kernel::ffi::FfiSignature{params: [ByteArray],
      ret: ByteArray}`.
- [ ] Implement `OxiLeanInvoker::register_export(mangled,
      sig, callback)` writing to
      `oxilean_kernel::ffi::ExternRegistry` so OxiLean's Lean
      code reaching `@[extern "<mangled>"]` dispatches into
      the Rust callback. One-time bulk-registration at
      adapter init; the `LeanProcInvoker::invoke` impl just
      looks up the per-call entry.
- [ ] End-to-end test: a minimal Lean module with one
      `@[leo4_export]` runs under OxiLean's elaborator,
      `OxiLeanProc::call` returns the encoded result, and
      the bytes match what `crates/leo4-mslean4` would
      produce for the same fixture (cross-impl conformance
      preserved).

## Phase 10-B1 callback ABI — free with OxiLean

leo4's native pipeline defers Phase 10-B1's callback ABI
runtime (re-entrant Lean ↔ Rust closures) to B1.x because
the LECQ/LECR re-entry IPC protocol is hard to design.
**With OxiLean as the impl, that whole problem dissolves**:

- OxiLean's `oxilean_kernel::ffi::FfiType::Fn(params, ret)`
  is a first-class FFI type.
- Re-entrant Lean → Rust → Lean is just a stack of in-
  process Rust function calls; no IPC frames to layer on.
- The adsmt SMT-solver use case (the canonical reason B1
  exists) is therefore easiest to ship with `leo4-oxilean`
  as the runtime, even before `leo4-mslean4`'s B1.x lands.

## Pinned deps (when activated)

| Crate | Version | Purpose |
|---|---|---|
| `leo4-abi` | path-dep | `LeanProc` + `LeanProcInvoker` + `LeanMarshal` + `LeanError` |
| `oxilean-kernel` | `^0.1.2` (when uncommented) | `FfiType` / `FfiValue` / `ExternRegistry` / `FfiSignature` |
| `oxilean-runtime` | `^0.1.2` (when uncommented) | `Env` / `Eval` / refcounted closures |
| `oxilean-elab` (opt) | `^0.1.2` (when uncommented) | source loading + `lean4_compat/` syntax adapter |
| `oxilean-build` (opt) | `^0.1.2` (when uncommented) | Rust-driven `oxilean build` for the plugin equivalent |

Bump these as a unit on each OxiLean release; mixing minor
versions across the OxiLean workspace is undefined.

## Cross-references

- `SPEC/rust-native-lean.md` — the trait + transport contract.
- `SPEC/rust-native-lean.md` §7 + §7.1 — OxiLean-specific
  integration analysis.
- `SPEC/lean-runtime-compat.md` — surface that ANY Lean impl
  needs to satisfy; rust-native impls bypass §1.2 / §1.3 /
  §1.4 entirely.
- `sibling/leo4-wasip3/` — companion sibling crate for the
  WASIp3 guest-side story.

## License

MIT OR Apache-2.0, matching leo4 + OxiLean.
