# leo4-oxilean

> Adapter: [leo4](https://github.com/Honey-Be/leo4) ↔
> [OxiLean](https://github.com/cool-japan/oxilean) via
> `SPEC/rust-native-lean.md`'s `LeanProc` /
> `LeanProcInvoker` trait surface.
>
> **Status (2026-05-31)**: PRODUCTION-WIRED on the leo4
> fork branch `0.1.3-leo4-ox7`, cleared for v1.0 RC 1
> tagging. Forward + reverse dispatch (inbound `@[extern]`
> callbacks via `ExternResolver` + outbound
> `RustCallbackRegistry` bridge) is functional. The
> underlying IO walker is no longer "shape-by-shape" —
> #76 P0c closed 2026-05-31 with the full monad-transformer
> family covered, `IO.bind` beta-application, full
> canonical-ABI arg encoding (including user-defined
> record + inductive ctors via env-lookup of
> `ConstantInfo::Constructor`), and direct dispatch for
> stdlib `IO.println` + `IO.FS.*` families. Out-of-scope
> tail (`StateT.run`, `IO.FS.Handle`, `dbg_trace`, float
> literals) is explicitly classified, not open. Fork tests
> 1219 passing; translate tests 56 passing (#72 closed
> 2026-05-31). The earlier "every dispatch returns a stub
> error" / "scaffold-only" framing is historical — see
> `docs/ox8-1-leo4-oxilean-audit.md`'s top-of-file note.

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

Not yet a leo4 backend on **cool-japan's upstream
OxiLean v0.1.2** — the dispatch hooks the adapter
needs (`CallbackRegistry`, `ExternResolver`,
`dispatch_extern_const`, the `driver` module) live on
the leo4 fork branch `0.1.3-leo4-ox7` until the
relevant cool-japan PR series merges. Today the adapter
works **against the fork**, which is what
`leo4-oxilean-build`, `leo4-oxilean-runner`, and the
`leo4 create reverse --impl rust-transpile` scaffold
all depend on. cool-japan upstream PRs for OX7 / OX8 /
the driver API are tracked in `docs/cool-japan-*.md`;
once they land upstream, this adapter compiles
against an unmodified OxiLean release with no leo4-side
churn.

## What works today (19 / 19 adapter tests passing; fork side 1219)

- `OxiLeanInvoker::new()` constructs an
  `Arc<Mutex<oxilean_kernel::ffi::ExternRegistry>>` wrapper
  + an `Arc<Mutex<CallbackRegistry>>` for runtime closures
  + an `Arc<Mutex<Option<Arc<RustCallbackRegistry>>>>` for
  outbound dispatch (#75 step 3, 2026-05-28).
- `register_export(mangled)` records `ExternDecl` metadata
  under `lib_name = "leo4-rust-bridge"`, `symbol_name =
  mangled`, signature `(ByteArray) -> ByteArray`.
- `register_export_callback(mangled, closure)` (OX8.3c,
  2026-05-28) supplies the actual runtime closure the
  evaluator calls via `dispatch_extern_const` /
  `ExternResolver`.
- `LeanProcInvoker::invoke` dispatches:
  - `UNKNOWN_FUNCTION` (0x0002_0004) — export not
    registered as metadata,
  - `RUST_DLSYM_FAILED` (0x0002_0005) — metadata exists
    but no runtime callback installed,
  - `ENCODE_ERROR` (0x02) on callback-side failures with
    the original message threaded through.
- `attach_outbound_registry(...)` + `outbound_registry()`
  + `invoke_outbound(callback_id, args)` (#75 step 3) +
  `register_outbound_dispatch_callback(mangled)` (#76,
  2026-05-29) close the Phase 10-B1.x outbound path:
  Lean closure dereferences fire the bridge callback,
  which unpacks `(callback_id LE prefix, &args[8..])`
  and forwards to the host's
  [`RustCallbackRegistry`](https://docs.rs/leo4-abi/).
- `ExternResolver` impl routes evaluator-side
  `dispatch_extern_const` calls through the same
  callback registry.

## OxiLean upstream prerequisite — direct inspection results

Three hooks needed for full integration; all three now
exist on the leo4 fork branch `0.1.3-leo4-ox7`. Cool-japan
upstream PR status is tracked in
`docs/cool-japan-upstream-pr-draft.md` (codegen + parser
donation + Hooks 1 / 2) and
`docs/cool-japan-driver-api-coordination-draft.md`
(driver API — posted at
<https://github.com/cool-japan/oxilean/issues/2>, awaiting
maintainer feedback as of 2026-05-31).

- [x] **Hook 1 — Callback-registration entry point in the
      evaluator** for `ExternRegistry` symbols.
      Landed on the fork as `CallbackRegistry` +
      `ExternCallback` + `ExternCallError` in
      `oxilean-kernel/src/ffi/callbackregistry_traits.rs`
      (OX8.3a, fork commit `72add72`).
- [x] **Hook 2 — Evaluator-side dispatch** of an
      `@[extern]`-attributed `Const` reduction through a
      pluggable resolver.
      Landed as `ExternResolver` trait +
      `dispatch_extern_const(env, registry, resolver,
      name, args)` + `dispatch_extern_decl` in
      `oxilean-runtime/src/extern_resolver.rs` (OX8.3b,
      fork commit `bf17523`). The
      `oxilean_runtime::driver` module drives
      `main : IO α` through that hook. #76 P0c (closed
      2026-05-31) extended coverage to the full monad
      transformer family, `IO.bind` beta-application,
      full canonical-ABI argument encoding (including
      user-defined record + inductive ctors via
      env-lookup of `ConstantInfo::Constructor`), and
      direct dispatch for stdlib `IO.println` +
      `IO.FS.*` families. Out-of-scope tail
      (`StateT.run`, `IO.FS.Handle`, `dbg_trace`, float
      literals) is explicitly classified rather than
      pending.
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
  `OxiLeanInvoker::invoke`) is **live on the fork** —
  Hooks 1 & 2 landed (OX8.3a/b, 2026-05-28) and the
  driver IO walker closed under #76 P0c on 2026-05-31.
  Registered exports + callbacks dispatch through
  `ExternResolver` + `dispatch_extern_const` end-to-end;
  the `RUST_DLSYM_FAILED` (0x0002_0005) code now only
  fires when a caller registered export metadata but
  didn't install the runtime callback closure, which is
  a programmer-error path covered by tests.
* On **cool-japan upstream OxiLean v0.1.2** the adapter
  remains blocked until the PR series merges; tracking
  in `docs/cool-japan-*.md`.

Tracking: `SPEC/rust-native-lean.md` §7.1 + §8 reflects
this 3-of-3 status (all hooks now exist on the fork and
the driver is wired through).

## Phase 10-B1 callback ABI — wired end-to-end

leo4's reference `leo4-mslean4` pipeline defers Phase 10-B1's
callback ABI runtime (re-entrant Lean ↔ Rust closures) to
B1.x because the LECQ/LECR re-entry IPC protocol is hard
to design. **With OxiLean as the impl, that whole problem
dissolves**:

- OxiLean's `oxilean_kernel::ffi::FfiType::Fn(params, ret)`
  is a first-class FFI type.
- Re-entrant Lean → Rust → Lean is just a stack of in-
  process Rust function calls; no IPC frames to layer on.
- leo4's outbound bridge — `register_outbound_dispatch_callback(mangled)`
  + `attach_outbound_registry(...)` + `invoke_outbound(id, args)`
  — closes the loop today on the fork: the
  `leo4::import!` macro registers Rust closures into
  `Lean::callback_registry()`, encodes a `callback_id`
  into the canonical args buffer, and the bridge
  unpacks + forwards back into the registered closure
  when the Lean side dereferences it.
- The adsmt SMT-solver use case (the canonical reason B1
  exists) is therefore easiest to ship with
  `leo4-oxilean` as the runtime, even before
  `leo4-mslean4`'s B1.x lands. mslean4 LECQ/LECR landing
  is gated on a dedicated `feat/mslean4-lecq-lecr-ipcs`
  branch that forks from leo4's main once C1 / C5 / G2
  manual verification closes.

## Pinned deps

| Crate | Source | Purpose |
|---|---|---|
| `leo4-abi` | path-dep | `LeanProc` + `LeanProcInvoker` + `LeanMarshal` + `LeanError` + `RustCallbackRegistry` |
| `oxilean-kernel` | path-dep into `sibling/oxilean/` fork submodule | `FfiType` / `FfiValue` / `ExternRegistry` / `FfiSignature` / `CallbackRegistry` (OX8.3a) |
| `oxilean-runtime` | same | `ExternResolver` / `dispatch_extern_const` (OX8.3b) + `driver::run_main` (#76) |

Direct `path =` deps rather than `version + [patch.crates-io]`
— consumers path-dep into this crate from their own
workspaces, so the patch block at *this* crate's root
wouldn't apply (`f90ee50` policy lock). Mixing minor
versions across the fork crates is undefined.

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
