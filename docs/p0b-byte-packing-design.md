# P0b — OxiLean adapter byte-packing of Fn args + closure registry encoding side

> Status: **DESIGN SKETCH** (2026-05-28). Tracked as task #75.
> Implementation lands in a follow-up commit on top of the
> #74 / #76 / #77 / #79 batch (commit landing in flight).

## The gap

The leo4 callback ABI (Phase 10-B1) has two halves:

  - **Inbound (Rust receives a Lean closure)** — landed in
    `83cbbcc`. `LeanCallback<R, Args>` + `CallbackInvoker`
    in `crates/leo4-abi/src/callback.rs`. Macro recognises
    the type via `rust_type_to_idl` → `IDLType::Fn`. Wire
    decoder allocates the typed token from the `u64
    callback_id`.

  - **Outbound (Rust passes a Rust closure to Lean)** —
    NOT YET LANDED. The user-facing surface would look
    like:

    ```rust
    leo4::import! {
        fn v(u: fn(u64) -> u64) -> u64;
    }
    fn u(t: u64) -> u64 { t * t }
    let r = v(&lean, u)?;
    ```

    The macro needs to (1) allocate a `callback_id` for
    `u`, (2) register `u` against that id in a main-side
    registry, (3) encode the id into the args buffer, (4)
    call shim, (5) deallocate the id on return. The
    adapter (oxilean) or dispatcher (mslean4) needs to
    answer when the receiving side dereferences the id
    during `v`'s execution.

## Why this is hard for oxilean

`OxiLeanInvoker::invoke(mangled, args: &[u8])` already
receives a *packed* canonical-ABI ByteArray. So the
question "how does an `FfiValue::Fn` get into that
ByteArray?" has two possible answers:

### Option A — Lean-side packing (preferred)

`leo4-oxilean-build --mode reverse` emits an `@[extern]`
wrapper for `v` in `lean/<Iface>/Rust.lean`. That wrapper
runs *inside* the OxiLean evaluator and could call a
leo4-side primitive `Leo4.registerClosure : (α → β) → IO UInt64`
(or similar) before packing the ByteArray. The primitive
inserts the closure into the same `CallbackRegistry`
(OX8.3a) that inbound callbacks already use, and the
packed bytes carry the resulting `u64`.

Symmetry advantage: the inbound and outbound paths share
one registry. The closure stays in the registry only for
the duration of the enclosing call (the wrapper deregisters
on return).

Open sub-question: who emits the primitive? Most likely:

  - `leo4-oxilean-build --mode reverse` (the existing
    wrapper emitter) gains a `--include-callback-prelude`
    flag that prepends a tiny Lean module defining
    `Leo4.registerClosure` as `@[extern "leo4_register_closure_…"]`
    + the matching Rust callback wired into
    `OxiLeanInvoker` at startup.

### Option B — Adapter-side hook on `dispatch_extern_const`

leo4-oxilean intercepts the evaluator's `dispatch_extern_const`
callback (OX8.3b) and reads the raw `FfiValue` args. When
it spots an `FfiValue::Fn`, it registers + substitutes a
`callback_id` before re-packing the ByteArray. Requires
the OxiLean evaluator to expose pre-pack hooks, which it
doesn't today.

Cost: requires upstream API additions in
`oxilean-runtime::dispatch_extern_const`. More invasive
than option A. Option A is the leading candidate.

## Why mslean4 is conceptually identical, mechanically different

Same problem ("Rust closure → callback_id + register +
unregister") but the registry lives on the main process
side while the receiving Lean closure thunk lives on the
worker side. The wire frame is SPEC `LECQ`/`LECR`
(`SPEC/reverse-direction.md` §10a) — but the existing text
only spells out the *worker → main* direction (reverse
direction's "Lean closure called from Rust"). Forward
direction's "Rust callback called from Lean" reuses the
same frame, but the SPEC text needs to generalise.

This is a v1.0 RC pre-release window concern, not v1.0 RC
mandatory.

## Implementation plan (next commit)

  - **Step 1 (leo4-abi)**: add `RustCallbackRegistry` —
    main-side outbound registry. New helper
    `register_rust_callback<R, Args>(closure) -> u64`
    that allocates an id + stores the closure +
    encode-decode functions. Pair with a `Drop`
    guard ensuring per-call-scope cleanup.
  - **Step 2 (leo4-macros-backend)**: the `leo4::import!`
    macro recognises `fn(T₁,…,Tₙ) -> R` / `impl Fn(…)
    -> R` parameter types. For each, emit the
    register-encode-call-decode-deregister sequence
    around the existing shim call.
  - **Step 3 (leo4-oxilean)**: option-A scaffolding —
    leo4-oxilean-build's `--mode reverse` learns to
    prepend a Lean module with the
    `Leo4.registerClosure` primitive, and
    `OxiLeanInvoker` registers the matching Rust
    callback at adapter init.
  - **Step 4 (smoke)**: a fixture
    `sum_squares_100(f: LeanCallback<u64, (u64,)>) -> Result<u64, LeanError>`
    + Main.lean `sum_squares_100 (fun x => x * x)` →
    return 328350.

The fork-side `driver::run_main` (#76) needs an IO
walker before step 4 can fire end-to-end; pre-walker the
steps 1-3 surface lands but step 4 hits the same
`NotYetImplemented` sentinel the rest of the runner
returns today.
