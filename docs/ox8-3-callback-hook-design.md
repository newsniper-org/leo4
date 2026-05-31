# OX8.3 — OxiLean evaluator callback-registration hook design

Opened: 2026-05-27. **Status (2026-05-31): IMPLEMENTED on the leo4
fork branch `0.1.3-leo4-ox7`.** The design below was the original
proposal; the actual landed shape closely follows it. Fork commits:

- `72add72` (2026-05-28) — `CallbackRegistry` + `ExternCallback` +
  `ExternCallError` in `oxilean-kernel/src/ffi/callbackregistry_traits.rs`
  (OX8.3a).
- `bf17523` (2026-05-28) — `ExternResolver` trait + `SharedExternResolver`
  + `dispatch_extern_const(env, registry, resolver, name, args)` +
  `dispatch_extern_decl` in `oxilean-runtime/src/extern_resolver.rs`
  (OX8.3b).
- `91430ae` (2026-05-28) — leo4-oxilean adapter wires
  `register_export_callback(mangled, closure)` against the fork hooks
  (OX8.3c).
- `f9bfd45` / `8b2af9f` / `d357a01` (2026-05-28..29) — `driver` module
  drives `main : IO α` through the resolver chain (#76 P0c).
- `521979e` / `44bb382` (2026-05-28..29) — outbound dispatch +
  `register_outbound_dispatch_callback` (P0b #75 step 3 + P0c #76).

cool-japan upstream PR drafts (post-RC submission):
`docs/cool-japan-upstream-pr-draft.md` covers the OX7 + OX8.3a/b
contribution stream; `docs/cool-japan-driver-api-coordination-draft.md`
covers the driver API (posted at
[cool-japan/oxilean#2](https://github.com/cool-japan/oxilean/issues/2)
2026-05-28; no maintainer feedback yet as of 2026-05-31).

This doc is preserved for the design rationale + the API-comparison
table (4-option study) below.

## Why this hook?

OX8.1 (`docs/ox8-1-leo4-oxilean-audit.md`) established that the
`leo4-oxilean` adapter's `OxiLeanInvoker::invoke` /
`OxiLeanProc::call` currently return `RUST_DLSYM_FAILED` (0x0002_0005)
because the OxiLean evaluator has no way to *execute* a Rust callback
registered for a given `@[extern "<mangled>"]` declaration. Only the
metadata side (`ExternRegistry` at `oxilean-kernel/src/ffi/types.rs`
1469–1539) is implemented in v0.1.2 — at runtime, the evaluator's
`Const` reduction stops short of invoking any external code.

OX8.3 closes this gap by adding a runtime-side `CallbackRegistry` +
an `ExternResolver`-style hook the evaluator calls when it encounters
an `@[extern]` declaration during reduction.

## Current state — what's already there

- `oxilean_kernel::ffi::ExternRegistry` — stores `(decl_name,
  lib_name, symbol_name)` metadata for each `@[extern]` decl. Today
  this is populated lazily by `oxilean-elab` during `@[extern]`
  attribute processing and read by no one at runtime.
- `oxilean-runtime` evaluator layers — `tco`, `bytecode_interp`,
  `lazy_eval`. None have an `@[extern]` dispatch entry point;
  `Const` reduction either looks up the constant's `Definition.val`
  (unfolding) or treats `Axiom`/`Constant` as opaque (irreducible).
  An `@[extern]` decl currently lowers to an `Axiom` (no `val`), so
  reduction stops there silently — no error, no dispatch.
- `leo4-oxilean::OxiLeanInvoker::register_export` — pushes
  `ExternDecl { mangled, params: ByteArray, ret: ByteArray, lib:
  "leo4-rust-bridge" }` into `Arc<Mutex<ExternRegistry>>`. 8/8 unit
  tests pass. The metadata side is solid.

## Proposed API — `CallbackRegistry`

A new sibling structure to `ExternRegistry`, sitting in
`oxilean-kernel/src/ffi/` (same module so they evolve together):

```rust
pub type ExternCallback =
    Box<dyn Fn(&[u8]) -> Result<Vec<u8>, ExternCallError> + Send + Sync>;

pub struct CallbackRegistry {
    /// Map from `(lib_name, symbol_name)` to the registered
    /// callback. The key matches `ExternRegistry`'s lookup key
    /// 1:1 so a metadata entry + a callback entry always
    /// correspond.
    callbacks: HashMap<(String, String), ExternCallback>,
}

impl CallbackRegistry {
    pub fn new() -> Self { /* … */ }

    pub fn register(
        &mut self,
        lib: impl Into<String>,
        symbol: impl Into<String>,
        cb: ExternCallback,
    ) { /* … */ }

    pub fn invoke(
        &self,
        lib: &str,
        symbol: &str,
        args: &[u8],
    ) -> Result<Vec<u8>, ExternCallError> { /* … */ }
}

#[derive(Debug, Clone)]
pub enum ExternCallError {
    NotRegistered { lib: String, symbol: String },
    CallbackFailed(String),
}
```

The `Send + Sync` bounds on `ExternCallback` are so the
evaluator (which the runtime may parallelise in future) can dispatch
without `&mut`. The owned `Vec<u8>` return aligns with leo4's
canonical-ABI byte buffer.

## Evaluator integration

`oxilean-runtime` gains an `ExternResolver` trait:

```rust
pub trait ExternResolver: Send + Sync {
    fn resolve(
        &self,
        decl_name: &Name,
        args: &[u8],
    ) -> Result<Vec<u8>, ExternCallError>;
}
```

The evaluator's constructor accepts an `Option<Arc<dyn
ExternResolver>>`. When `Const` reduction hits a name backed by an
`@[extern]` declaration:

```rust
match env.find_decl(name) {
    Some(decl) if decl.is_extern() => {
        // Look up extern metadata (lib, symbol) from ExternRegistry.
        let ExternMetadata { lib, symbol, .. } =
            ffi_registry.lookup(name).ok_or(/* internal error */)?;
        // Dispatch to the user-supplied resolver.
        match &self.extern_resolver {
            Some(resolver) => {
                let arg_bytes = encode_args_canonical_abi(&args);
                let result_bytes = resolver.resolve(name, &arg_bytes)?;
                decode_canonical_abi(decl.ret_ty(), &result_bytes)
            }
            None => Err(/* no resolver installed */),
        }
    }
    // … existing arms (Definition unfold, Axiom opaque, etc.)
}
```

This is the *only* point in the evaluator that needs to change.
Existing `Definition`/`Axiom`/`Constructor`/`Recursor` reduction
paths stay untouched.

## leo4-oxilean adapter integration

```rust
pub struct OxiLeanInvoker {
    extern_registry: Arc<Mutex<ExternRegistry>>,
    callback_registry: Arc<Mutex<CallbackRegistry>>,  // NEW
}

impl OxiLeanInvoker {
    pub fn register_export_callback<F>(&self, mangled: &str, cb: F)
    where
        F: Fn(&[u8]) -> Result<Vec<u8>, ExternCallError> + Send + Sync + 'static,
    {
        self.callback_registry.lock().unwrap().register(
            "leo4-rust-bridge",
            mangled,
            Box::new(cb),
        );
    }
}

impl ExternResolver for OxiLeanInvoker {
    fn resolve(
        &self,
        decl_name: &Name,
        args: &[u8],
    ) -> Result<Vec<u8>, ExternCallError> {
        // Look up `(lib, symbol)` for this decl via
        // ExternRegistry, then dispatch via CallbackRegistry.
        let meta = self.extern_registry.lock().unwrap()
            .lookup_by_name(decl_name)?;
        self.callback_registry.lock().unwrap()
            .invoke(&meta.lib, &meta.symbol, args)
    }
}
```

The adapter's caller (the eventual `leo4 run --impl rust-transpile`
reverse runner, OX8.4) instantiates `OxiLeanInvoker`, calls
`register_export_callback` for every entry in the cdylib's `EXPORTS`
slice (the callback closes over a `libloading::Library` + a `dlsym`
resolution of the mangled symbol), and passes the invoker as the
evaluator's `extern_resolver`.

## Patch scope

| Crate | Lines | What |
|---|---|---|
| `oxilean-kernel` | ~80 | `CallbackRegistry` + `ExternCallback` type alias + `ExternCallError` enum |
| `oxilean-runtime` | ~120 | `ExternResolver` trait + evaluator dispatch point in `Const` reduction |
| `leo4-oxilean` | ~50 | Adapter wiring (`register_export_callback` method, `ExternResolver` impl) |

Total ≈ 250 SLOC across three crates. No breaking change — existing
evaluator callers without an `extern_resolver` get the current
behaviour (extern decls reduce to themselves, no dispatch).

## Why this API shape vs. alternatives

| Alternative | Pros | Cons |
|---|---|---|
| (chosen) `CallbackRegistry` + `ExternResolver` trait | Decouples metadata from runtime; resolver is per-evaluator-instance | One extra registry struct |
| Global static `CallbackRegistry` | Simpler API | Hostile to test isolation; bad for multi-tenant runtime |
| `Definition.val` synthesised at register time | Fits existing reduction path | Forces canonical-ABI roundtrip even when caller has direct access; harder to swap callback later |
| `Box<dyn Fn>` on each `ExternDecl` | No separate registry | Mutates kernel-side data structures; couples kernel to runtime |

The chosen split (kernel keeps metadata only; runtime owns
callbacks; adapter bridges them) follows the same boundary OxiLean
already uses for `Definition.val` (kernel-side AST) vs.
`ReductionStrategy` (runtime-side dispatch).

## Upstream PR viability

Estimated cool-japan/oxilean upstream acceptance probability:
**70–80%**. Reasoning:

- ✓ Minimal scope (runtime only, kernel almost-trivial, no elab
  changes).
- ✓ No breaking change — old evaluator callers unaffected.
- ✓ Symmetric with existing FFI-side design (`ExternRegistry` is
  metadata-only; this adds the runtime sibling).
- ✓ The `ExternResolver` trait is reusable beyond leo4 — anyone
  embedding OxiLean for scripting needs this same hook.
- ⚠ Documentation burden: the `@[extern]` attribute + extern call
  contract needs SPEC-level documentation on the cool-japan side.
  Likely 1–2 follow-up PRs after the runtime patch lands.

## Implementation plan for fork's `0.1.3-leo4-ox7` branch

Three sub-commits, in order:

1. **OX8.3a** — `CallbackRegistry` + `ExternCallback` /
   `ExternCallError` types. Pure data structure; no evaluator
   changes. Unit-tested with a registered closure + invoke
   round-trip.
2. **OX8.3b** — `ExternResolver` trait + evaluator `Const`
   reduction dispatch point in `oxilean-runtime`. Unit-tested with
   a mock resolver that returns canned bytes.
3. **OX8.3c** — leo4-oxilean adapter wiring (in our `sibling/
   leo4-oxilean/` crate, not the fork). Calls register +
   ExternResolver impl. End-to-end test: register a closure,
   evaluate a Lean source that calls into it, assert the closure
   ran with expected args.

OX8.3a + OX8.3b land in the fork. OX8.3c lands in leo4.

## Acceptance criteria

- An `@[extern "<mangled>"] opaque foo (a : UInt64) : UInt64`
  declaration, when evaluated under an `OxiLeanInvoker` with a
  registered closure for `<mangled>`, runs the closure with the
  encoded arg + decodes the return.
- The closure's signature uses leo4's canonical-ABI byte buffer
  (zero new ABI design — reuses what `leo4-rust-bridge` already
  encodes for the forward path).
- Without a resolver installed, `@[extern]` reduction returns
  `ExternCallError::NotRegistered` cleanly — no panic.
- 8/8 existing `leo4-oxilean` tests stay passing; the dispatch
  stub's `RUST_DLSYM_FAILED` path becomes unreachable for the
  registered case.

## Out of scope for OX8.3

- Multi-threaded callback execution (the `Send + Sync` bounds are
  there for future use; v0 dispatches sequentially).
- Async callbacks (`async fn` resolvers). The WASIp3 sibling path
  needs this eventually but rust-transpile's reverse path doesn't.
- Hot-swappable callbacks (re-registering a symbol after evaluator
  start). Possible but not needed for OX8 acceptance — defer.

## Next step

Implement OX8.3a (the data structure) as a single fork commit on
`0.1.3-leo4-ox7`. That unblocks OX8.3b. OX8.4 / OX8.5 can begin in
parallel with OX8.3b — they don't depend on the dispatch hook
working, only on the `EXPORTS`-reading CLI surface (OX8.2b).
