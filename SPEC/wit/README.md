# SPEC/wit/ — Component Model interface

> Phase 10-C4.x landing (2026-05-21). Pins the WIT contract that
> the wasm-targeting leo4 backends (`crates/leo4-wasm`) wrap and
> that `wit-bindgen` will consume in C4.x.x.

## Files

| File | Status | Contents |
|---|---|---|
| `leo4-host.wit` | **Pinned 2026-05-21** | The canonical Component Model interface. `package leo4:host@0.1.0`, exposes `world leo4-component` with `import host-imports` + `export exports`. |
| `README.md` | This file | Design rationale + cross-references. |

## Design decisions worth recording

### 1. Why opaque byte buffers (`canonical-bytes = list<u8>`)

The leo4 canonical ABI (`SPEC/canonical-abi.md`) and the
Component Model's own ABI are **distinct**:

- leo4's wire format is byte-identical across native + wasm
  backends (it's the cross-impl conformance invariant pinned by
  `tests/mangling/run.sh` + `tests/conformance/run.sh`).
- WIT's `record` / `variant` / `list` types compile through the
  Component Model's canonical ABI, which has its own layout
  rules (padding, fanin / fanout, etc.).

If `leo4-host.wit` declared every leo4 type as a WIT type, the
Component Model's ABI would re-encode the bytes a second time —
double indirection, and worse, **a leo4 type's wire bytes
inside wasm would no longer match the same type's wire bytes in
native**. Cross-impl conformance would break.

Solution: leo4's canonical bytes pass through CM as opaque
`list<u8>`. The receiving side decodes via the same
`canonical_decode` / `canonical_encode` it would use on
native. **One ABI, one wire format, two transport mediums** (CM
vs. native shim).

Cost: one extra `list<u8>` boxing per call (the CM ABI allocates
guest memory and copies). Mitigation: the canonical ABI is
already designed to amortise (one big buffer per call, not many
small reads); the `list<u8>` boxing is a single contiguous
memcpy per direction.

### 2. Why generic `call(mangled, args)` not per-fn typed exports

Two valid designs for "how does a Lean component expose its
`@[leo4_export]`s through WIT":

**Option A — one WIT export per `@[leo4_export]` function**
- WIT regenerates whenever the user adds / removes an export.
- `wit-bindgen` produces typed Rust bindings — each call is a
  typed function on the host side.
- *Schema_hash rotation forces a WIT regeneration* — any IDL
  change at all rotates the hash, which rotates the WIT, which
  forces `wit-bindgen` to rerun. Big rebuild blast radius.

**Option B (chosen) — one generic `call(mangled, args)` export**
- WIT stays stable across schema_hash rotations.
- Host dispatches by mangled name string at runtime (same model
  as `dlsym(leo4_call_<mangled>)` on the native pipeline).
- The schema_hash check happens once at instantiation via
  `verify-handshake`, after which the host trusts the
  component's dispatch table.
- Trade-off: per-call type safety lives at the **canonical-ABI
  layer** (Rust's `LeanMarshal` impls + Lean's `LeanMarshal`
  instances), not at the WIT layer. That's where it already
  lives in the native pipeline — consistency over redundancy.

Picked **B** because:
- leo4's native pipeline already uses generic-dispatch-by-
  mangled-name. WIT layer should match, not introduce a second
  model.
- WIT changes are expensive (consumers regenerate bindings); we
  want them to happen only for genuine ABI version bumps, not
  for every user IDL edit.
- Component Model `list<u8>` is well-supported on every CM
  runtime (`wasmtime`, `wasm_component_layer` + `wasmi`, etc.).
  Typed records would push us into the long tail of CM type
  support — risk for cross-runtime portability.

### 3. Why `host-imports` is so small

Today: just `log` (optional, fire-and-forget) and the
`handshake-frame` type. No alloc / dealloc / panic imports.

Reason: leo4 wasm components target `wasm32-wasip2` (or its
WASIp3 successor), which already provides allocator + abort +
the I/O surface via WASI. We don't re-define those.

What might land in `host-imports` later (post-C4.x.x):
- **`leo4-callback`** — re-entrant callback invocation channel
  for Phase 10-B1's function-arrow ABI when it runtime-lands.
  Format: `host-callback: func(id: u64, args: canonical-bytes)
  -> result<canonical-bytes, lean-error>`. The component
  invokes this when a `LeanCallback` typed Rust thunk fires.

### 4. Why `verify-handshake` is an **export**, not an import

The host *initiates* the handshake (it knows the expected
hash from the `.leo4-handshake` JSON), but the **component
decides whether to accept it** (it knows its own compiled-in
hash). Convention: the side that owns the data is the side
that exports the verification function.

Equivalent: native pipeline calls `leo4_handshake` (a Lean-
side symbol) immediately after `dlopen` and before any
`leo4_call_<mangled>`. WIT mirrors that flow.

## Cross-references

- **Normative**: `SPEC/canonical-abi.md` §13 (`LeanError` codes
  table), `SPEC/handshake.md` (schema_hash format), `SPEC/
  reverse-direction.md` §5 (IPC wire format the native worker
  speaks; the CM equivalent lives here in `exports.call`).
- **Implementation**: `crates/leo4-wasm/src/runtime.rs`
  (host-side `WasmRuntime` trait wrapping this WIT),
  `crates/leo4-wasm/src/backend/{wasmtime,wasmi}.rs`
  (backend impls — C4.x.x will wire `wit-bindgen` output).
- **Plan**: `ROADMAP.md` Phase 10-C4 / C4.x / C4.x.x;
  `AGENTS.md` "Recent decisions" 2026-05-21 entry on closed-
  world IDL (the consumer-side WIT lowering is consistent with
  that decision — WIT is a *consumer* of the leo4 IDL, not a
  redefinition of it).

## Versioning

The package version (`leo4:host@0.1.0`) follows semver:

- **Patch bump** (0.1.0 → 0.1.1): documentation-only changes,
  field reordering that preserves wire compatibility. Does
  NOT rotate schema_hash; existing components stay valid.
- **Minor bump** (0.1.x → 0.2.0): new optional imports/exports.
  Old components remain valid (new interfaces are opt-in for
  newer components); host MUST tolerate components that don't
  use the new surface.
- **Major bump** (0.x.y → 1.0.0): breaking change. Rotates
  the *abi-version* number on the wire (`exports.abi-version`)
  and forces every component to be rebuilt. The host's
  `handshake-frame.abi-version` field is the negotiation point.

The schema_hash and the WIT version are **independent**: a
schema_hash rotation is a *user IDL change* (some
`@[leo4_export]` signature moved); a WIT version bump is a
*leo4 runtime ABI change* (this WIT file moved). The
`verify-handshake` flow checks both — schema_hash via the
expected/actual pair, abi-version via the frame field.
