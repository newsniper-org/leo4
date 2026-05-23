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

## OxiLean upstream prerequisite

`leo4-oxilean` becomes fully functional once OxiLean
upstream exposes the following hooks. Listed in dependency
order:

- [ ] **Callback-registration entry point in the OxiLean
      evaluator** for ExternRegistry symbols.
      `oxilean_kernel::ffi::ExternRegistry::register(decl)`
      currently stores metadata only — `decl.lib_name` /
      `decl.symbol_name` describe where the actual symbol
      lives, and OxiLean's codegen / evaluator resolves it
      via `dlsym(lib, symbol)` at runtime.
      `leo4-rust-native`'s in-process direct-call model
      needs OxiLean to accept a
      `Box<dyn Fn(&[u8]) -> Result<Vec<u8>, _>>` closure
      *per mangled name*, dispatching into it instead of
      doing `dlsym`. Suggested API shape:
      `ExternRegistry::register_callback(decl, callback)`.
- [ ] **By-name `@[leo4_export]` dispatch in the OxiLean
      evaluator**. `OxiLeanProc::call(mangled, args)` needs
      OxiLean to expose
      `Env::call_by_mangled_name(name, ffi_args) ->
      FfiValue`. Equivalent to reference Lean's
      `dlsym(leo4_call_<mangled>)` for the forward
      direction.
- [ ] **`@[leo4_export]` attribute recognition** in the
      `oxilean-elab` attribute pipeline (the
      `registerBuiltinAttribute` analogue). Until this lands,
      `lake/Leo4/Leo4/Export.lean` doesn't elaborate as-is on
      OxiLean and a `leo4-oxilean-build` analogue can't scan
      a user package's exports to emit handshake JSON.

These three items are tracked as
`SPEC/rust-native-lean.md` §8's activation conditions. If
you (or anyone) upstreams them to OxiLean, ping the leo4
maintainers and this adapter's `LeanProc` / `LeanProcInvoker`
bodies fill in transparently.

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
