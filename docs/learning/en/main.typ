#import "../../template/leo4-book.typ": book
#show: book.with(
  title: "leo4 — Learning Material",
  subtitle: "English Edition",
  author: "윤병익 (leo4 project)",
  lang: "en",
)

= Introduction

`leo4` is a Lean 4 ↔ Rust interop library that intentionally
does *not* bind the Rust side to a specific Lean toolchain
version. Where the predecessor `leo3` compiled against
`lean.h` directly --- and broke whenever Lean's internal layout
shifted --- `leo4` puts all Lean ABI knowledge inside a
build-time-generated C shim, exposing only a stable canonical
ABI to the Rust crate.

The result: the Rust crate tracks the IDL (a small WIT-superset
schema language), not the Lean toolchain. Lean upgrades rotate
the shim but not the Rust binary.

This learning material walks through leo4 the way a senior
engineer would learn it: start from the surface (what does a
user write?), then peel back layers (how does that wire across
the boundary?), then look at the design decisions that drove
the architecture.

== Audience

You are comfortable with at least one of Lean 4 or Rust, and
willing to learn enough of the other to follow the boundary
crossing. We assume:

- Basic Rust: `Cargo.toml`, traits, lifetimes (`'a`), procedural
  macros at the user level (you don't need to write one, just
  understand what they generate).
- Basic Lean 4: `def`, `structure`, `inductive`, typeclasses
  (`class` / `instance`), and the idea that a Lean expression
  has both an abstract type and a compiled runtime representation.
- A vague sense of foreign function interfaces (FFI) at the C
  ABI level --- pointers, sizeof, calling conventions.

You don't need to know wasm Component Model or WASIp3 except
for the dedicated chapters on those backends.

= The thirty-second tour

The simplest leo4 use case looks like this. On the Lean side:

```lean
import Leo4

namespace Sample

@[leo4_export]
def add (a b : UInt64) : UInt64 := a + b
```

On the Rust side:

```rust
mod sample {
    leo4::import! {
        fn add(a: u64, b: u64) -> u64;
    }
}

fn main() -> Result<(), leo4::LeanError> {
    let lean = leo4::Lean::open(
        env!("LEO4_SHIM_SO"),
        env!("LEO4_HANDSHAKE_FILE"),
    )?;
    let r = sample::add(&lean, 2, 3)?;
    assert_eq!(r, 5);
    Ok(())
}
```

`@[leo4_export]` tells the Lake plugin "this declaration crosses
the boundary." `leo4::import!` on the Rust side reads the
mangling table the plugin produced and synthesises a Rust
wrapper that encodes the arguments per leo4's canonical ABI,
calls the matching C shim entry point, decodes the return
value, and wraps the result in a `Result`.

= Architecture overview

leo4 has six moving parts. Knowing what each owns is half the
mental model.

== The Lake plugin (`lake/Leo4Plugin/`)

A Lean executable that loads the user's package, walks every
`@[leo4_export]` definition, and emits four artefacts per build:

#table(
  columns: (auto, 1fr),
  table.header[*File*][*Purpose*],
  [`<pkg>.leo4-schema`],
  [Canonical IDL form: type declarations + function signatures
   in a stable text format. Input to the schema hash.],
  [`<pkg>.leo4-mangling`],
  [JSON table mapping logical function names + per-arg-type
   mangling to the unique C symbol the shim calls.],
  [`<pkg>.leo4-handshake`],
  [The schema hash + Lean toolchain identifier + a list of
   exported interfaces. The Rust loader reads this at
   `Lean::open` time.],
  [`<pkg>.leo4-shim.{c,so}`],
  [Generated C source compiled to a shared library; one
   `leo4_call_<mangled>` entry point per export. The only place
   in the system that `#include`s `lean/lean.h`.],
)

The plugin also writes a `<pkg>.leo4-exports.lean` file: a
Lean wrapper module the shim links against, providing
`@[export leo4_lean__<mangled>]` declarations that wrap the
user's exports in a known-name surface.

== `leo4-abi` (canonical-ABI marshalling)

A Rust crate that mirrors `lake/Leo4/Leo4/Marshal.lean` and
`Builtins.lean` byte-for-byte. Both sides implement the
`LeanMarshal` trait / typeclass; the test suite
(`tests/conformance/`) verifies that for every supported type
the Lean encoder and the Rust encoder produce byte-identical
output.

== `leo4-mslean4` (loader + dispatch)

A Rust crate providing `Lean::open`, `Arena<'a>`, and
`LeanRef<'a, T>`. The loader uses `libloading` to bring up the
shim's `.so`, initialises the Lean runtime once per process,
verifies the schema hash against the in-Rust constant,
runs the wrapper module's `initialize_*` symbol, and then
dispatches `leo4_call_<mangled>` calls via a per-name function
pointer cache.

== `leo4-macros` (`leo4::import!`, `#[derive(LeanMarshal)]`)

Procedural macros. `leo4::import!` parses an extern-style block
of `fn` signatures, looks them up in the mangling JSON the
build script surfaces via `OUT_DIR`, and emits Rust wrapper
functions. `#[derive(LeanMarshal)]` synthesises encode/decode
for user types matching the four canonical-ABI shapes (record,
all-unit enum, mixed-payload variant, single-`u64` resource).

== `leo4` façade

A thin re-export crate. Users add one line:
`leo4 = { workspace = true }`. Everything else --- `Lean`,
`LeanRef`, `LeanError`, `import!`, `LeanMarshal` --- lives at
`leo4::*`.

== `leo4-build`

A `build.rs` helper. One line in the consumer crate's
`build.rs`:

```rust
fn main() {
    leo4_build::wire("path/to/<pkg>/.lake/build/leo4").unwrap();
}
```

emits the right `cargo:rustc-link-search`,
`cargo:rerun-if-changed=`, and `env!("LEO4_SHIM_SO")` /
`env!("LEO4_HANDSHAKE_FILE")` constants the macro and the
loader expect.

= The IDL --- a WIT superset

leo4's IDL is the canonical type-level interface between the
two sides. It started from the WebAssembly Component Model's
WIT and added the small set of constructs Lean's dependent
types need to fit at the boundary.

The grammar lives in `SPEC/idl-grammar.ebnf`. The headline
extensions over WIT are:

#table(
  columns: (auto, 1fr),
  table.header[*Construct*][*Why*],
  [`generic_params` on nominal decls],
  [Lean's user-defined types are generic. `record Pair<α, β>`
   parses as a record with two type parameters; each
   instantiation gets its own mangled name.],
  [`Self` / `Self<…>` self-references],
  [Variants like `Tree { leaf, node(Self, Self) }` recurse
   through the enclosing decl. The mangling rule
   (`SPEC/mangling.md` §"Self and Self<…>") emits a short
   token rather than the full FQN.],
  [`mutual { … }` clusters + `Cyc<i>`],
  [Phase 6: mutual recursion between two nominal types.
   `Cyc<i>` references the `i`-th member of the cluster.],
  [`constraint <name> = <body>` declarations],
  [Constraints like `oneof { … }` pin the admit-set of a
   generic. Type-level only; never reach the wire.],
  [`bigint` / `bignat`],
  [Arbitrary-precision integers. Wire form is sign+limbs
   (SPEC/canonical-abi.md §6).],
  [`external <fqn>`],
  [Phase 8: a nominal type whose wire format lives in a custom
   `LeanMarshal` instance rather than per-field codegen. Used
   for `Rat` and any other Mathlib-shaped type with
   proof-carrying fields.],
)

WIT-side, leo4 lowers each IDL fragment to a WIT file via
`leo4c lower`. The WIT output is consumable by `wasm-tools` and
`wit-bindgen` for Component Model deployment.

= The canonical ABI --- bytes on the wire

`SPEC/canonical-abi.md` is normative. The Rust and Lean encoders
must produce identical bytes for the same logical value; the
conformance harness (`tests/conformance/run.sh`) pins this
across 29 fixtures.

Highlights, in case you don't want to read the whole spec:

- Integers are little-endian, unsigned and signed share the
  same bit pattern (signed is two's complement).
- Strings are `u32 len + utf-8 bytes`.
- Lists are `u32 len + N element encodings`.
- Options are `u8 disc (0=none, 1=some) + payload`.
- Results are `u8 disc (0=ok, 1=err) + payload`.
- Variants use `u32 LE disc + payload` (SPEC §9; we pinned u32
  in 2026-05-20 commit b2aa323 even though SPEC allowed u8 for
  ≤256 cases --- both encoders now emit 4 bytes).
- Records concatenate field encodings in declaration order.
- Resources are an opaque `u64` handle.
- `bigint` / `bignat` are length-prefixed limb arrays plus sign.

The shim emitter and the Rust derive macro generate code that
follows these formats. The plugin's `walkUserDecl` discovers
user types and synthesises the matching encode/decode without
hand-writing per type.

= Mangling --- naming conventions

`SPEC/mangling.md` defines the C symbol names. The full form is

```
leo4__<pkg_seg>__<iface>__<fname>__<arg_mangles>__h<schema_hash>
```

Each piece is ASCII-safe; dots in FQNs become underscores;
generic args expand into per-instantiation segments. The
schema hash is `FNV-1a-64` over the normalised IDL text,
rendered as 13-char base32lc. A change to any export's
signature rotates the hash and therefore every mangled name in
the package --- so a stale Rust binary linking against a fresh
shim fails at link time.

The hash construction is documented in `SPEC/mangling.md` §3.
Both implementations (Rust `crates/schema-idl` and Lean
`lake/Leo4Plugin/Leo4Plugin/Mangling.lean`) must compute it
identically; `tests/mangling/` pins 67+ names byte-identical
between the two.

= Type system on the Rust side

The boundary uses two main traits:

- `LeanMarshal` --- canonical-ABI encode/decode. Implemented for
  all primitive types, composites (`Vec<T>`, `Option<T>`,
  `Result<T,E>`, tuples), and via `#[derive]` for user records,
  enums, variants, and resources.
- `LeanType` --- type-system marker that connects to the schema
  layer. Most users don't touch this directly; `#[derive]` and
  the macros handle it.

There's also `LeanResource` for opaque handles. A type can't be
both `LeanMarshal` and `LeanResource` --- the plugin enforces
this.

The Lean side mirrors the trait/typeclass with `class
Leo4.LeanMarshal` and the matching `deriving LeanMarshal`
handler. The two byte streams have to agree; the conformance
harness is the cross-impl check.

= Phase ladder --- where each capability landed

leo4 development follows a phase ladder. Knowing which phase
each feature comes from helps when reading commit messages.

#table(
  columns: (auto, 1fr),
  table.header[*Phase*][*What landed*],
  [0], [Lake hook spike --- found the right plugin integration
        point (`lean_exe` invoked after `lake build`, not a
        `recBuildLean` hook).],
  [1], [Lean runtime library + Lake plugin; admit-set algorithm.],
  [2], [Rust `leo4-idl` + cross-impl mangling conformance.],
  [3], [WIT lowering pass + `wasm-tools` validation.],
  [4], [Canonical-ABI conformance harness, `bignat` / `bigint`.],
  [5], [C shim synthesis + `leo4-mslean4` + `leo4-macros` +
        `examples/01-hello`, `examples/02-roundtrip`.
        End-to-end pipeline.],
  [6], [Mutual recursion between nominal types
        (`mutual { … }` IDL block, `Cyc<i>`,
        `examples/04-mutual-ast`).],
  [7], [Async `io<T>` lowering. Parser desugars `future<T>` /
        `stream<T>`; shim wraps `IO α` Lean wrappers in
        `lean_io_result_*`. WASIp3 sibling project for the
        wasm-async surface.],
  [8], [Mathlib-compatible subset: `LeanRat`, `LeanU128` /
        `LeanI128`, `LeanComplexF{32,64}x2`, `LeanF16` /
        `LeanBF16` / `LeanF128` (nightly), Mathlib bridges
        with IEEE-754 RTNE rounding.],
  [9], [Reverse direction (Rust → Lean). `#[leo4::export]` on
        Rust ↔ generated Lean wrapper that calls into a Rust
        cdylib through a worker-process dispatcher
        (`libleo4_rust_bridge.a` + `leo4-rust-worker`).
        Isolation-backend-neutral; default mode is one
        long-running worker per cdylib, opt-in fresh-worker
        per call via `#[leo4::export(isolated)]`. Lake
        `extern_lib` integration auto-links the bridge +
        glue archives. See `SPEC/reverse-direction.md`.],
  [10], [DX + ABI surface widening: `leo4 run` CLI orchestrates
         build + emit + execute; function-arrow IDL type
         (`fn(T1,…,Tn) -> R`) with `A_…_a` mangling for
         the callback ABI; reserved `LeanError` codes
         0x02–0x08 now have real triggers; `lake run
         Leo4Rust/regenerate` script abstracts the
         reverse-direction emit step;
         `LEO4_RUST_WORKER_RECYCLE_SECONDS` time-based
         recycle + `leo4_rust_bridge_take_restart_flag`
         side-channel; variant payload widening
         (all-scalars multi-field); `leo4-wasm` scaffold.],
)

= Phase 9 in detail --- the reverse direction

leo4's first pipeline (forward direction) is Rust calling
Lean: `@[leo4_export]` on Lean, `leo4::import!` on Rust,
shim ↔ `lean.h` glue produced by the Lake plugin. Phase 9
adds the *second* pipeline going the other way.

== Why a second pipeline

The forward direction's mental model is "Rust embeds Lean":
the Lean side ships the entry points, the Rust side links
against them. That fits use cases where Lean is the
correctness-providing core and Rust is the runtime embedder.

The Phase 9 driver is the *inverse* use case: a Rust-side
solver (z3, cvc5, a research SMT prototype) that Lean's
proof tooling wants to drive interactively with
`push`/`pop`-style state preserved across calls. The
architecture is genuinely different --- schema_hash travels
in a JSON file (not baked into mangled symbols), the
dispatch goes through a worker process (not `libloading`),
and the C shim that touches `lean.h` sits on the Lean
caller's side (not the Rust callee's).

== Architecture summary

```
Lean process
  │
  ├── libleo4_rust_bridge.a    (statically linked dispatcher,
  │     │                       single C TU, C17 / C2x)
  │     ▼  posix_spawn / CreateProcess on first call
  │   leo4-rust-worker          (one per cdylib; long-running;
  │     │                       loads the user cdylib via dlopen)
  │     ▼  dlsym(leo4_rust__<mangled>) cached
  │   user cdylib               (#[leo4::export] functions)
```

The dispatcher is *isolation-backend-neutral*: a single C
entry point `leo4_rust_call(mangled, args, ret)` lets the
backend swap (long-running ↔ zygote-fork ↔ wasm sandbox)
without the Lean wrapper or the Rust macro noticing.

== Mangling delta

Forward direction mangles `…__h<hash>` so the linker
catches schema_hash mismatches at load time. Reverse
direction *cannot* do that --- the Rust proc-macro runs at
crate-compile time and has no access to the schema_hash
(which is only computable after the *complete* cdylib's
exports are known). Phase 9 puts the schema_hash in the
handshake JSON file emitted by `leo4-rust-emit` and checks
it at runtime via the worker's first frame.

== Wire-level handshake

The worker emits a 25-byte handshake immediately on spawn:
`u32 magic` + `u32 hash_len (13)` + `u32 abi_version` +
`13-byte schema_hash`. The dispatcher MUST consume this
before any request goes out --- skipping it causes the
handshake bytes to pile up in the IPC buffer and the
dispatcher decodes them as a response header. (Watch for
"garbage status values" if you see this.)

== `#[leo4::export(isolated)]`

The default mode is one long-running worker --- fast, but
not memory-isolated against accumulating state. For
security-sensitive workloads, add `(isolated)` to the
attribute. The macro prepends an `iso:` prefix to the
mangled name; the dispatcher detects it and routes through
a per-call fresh-worker path (spawn → call → `_exit`). No
wire-format or API change.

== Phase 10 follow-ups already landed (2026-05-21)

The Phase 9 surface has been smoothed out:

- `leo4 run` (D1) collapses the cargo + emit + lake-build
  + run ladder into one command.
- `lake run Leo4Rust/regenerate` (D2) puts emit behind a
  Lake script so the toolchain is abstracted.
- Function-arrow IDL (`fn(T1,…,Tn) -> R`) with mangling
  rule (`A_<tuple>_<ret>_a`) and re-entrant callback frame
  protocol designed (B1; runtime in B1.x).
- `LEO4_RUST_WORKER_RECYCLE_SECONDS` time-based recycle
  alongside the existing `_CALLS` knob;
  `leo4_rust_bridge_take_restart_flag` side-channel for
  observing `LEO4_ERR_RUST_WORKER_RESTARTED` (A4 / A5).

= Closing notes

This learning material is a starting point. The companion
`implement-from-scratch` guide book takes the next step:
walking through how to *build* each layer of leo4 yourself, in
the order the original phases landed.

For day-to-day reference:

- `SPEC/*.md` are normative; if something is unclear, check
  the spec.
- `CHANGELOG.md` lists every commit's effect with rationale.
- `ROADMAP.md` describes the phase ladder.
- `LEO4-DESIGN.md` captures every architectural decision and
  the rationale behind it.

The repository is the single source of truth. Everything else
is commentary.

= Update — 2026-05-24

A condensed roll-up of the changes that landed after the
v0.1.0 cut.

- *OX6 — PEG-based Lean 4 parser (complete)*.
  `sibling/leo4-lean4-parse` replaces the textual
  pre-rewrite chain (OX3/OX4) inside
  `leo4-oxilean-build`. AST shapes mirror
  `oxilean-parse` v0.1.2; a strict superset of its
  accepted surface. Lean 4 forms previously rejected
  by the rust-transpile pipeline (block / doc
  comments, Unicode operators `≤ ≥ ≠ × ÷ ∈`,
  `if let` / `match h : e with` / pattern guards,
  anonymous fn `(· + 1)`, DSL decls like `notation` /
  `macro_rules` / `syntax` / `elab`, equational
  `def | pat =>`, …) now elaborate.

- *OX5 — elab env bootstrap (complete)*. The
  rust-transpile path's elaborator no longer fails on
  `NameNotFound("UInt64")`. The OxiLean-only user
  installs *nothing* beyond `leo4-oxilean-build` —
  `oxilean_kernel::init_builtin_env` plus a small
  leo4-side augmentation (sized integers, floats,
  Char) populates the env in-process.

- *Post-OX6 CLI refactor — `leo4.toml`*. `leo4 create`
  and `leo4 init` dropped the `--impl <kind>` flag.
  Every scaffolded project now carries `leo4.toml`
  declaring runtime impls; multiple `[[impl]]`
  entries are allowed, with disjoint output paths
  enforced at parse time. `leo4 create --subcrate`
  registers the new crate into the surrounding
  workspace's `members` array. `leo4 init`
  auto-migrates legacy `.leo4-impl` markers.

- *C5 — musl Tier 1+ (v1.0 RC mandatory)*. Linux
  `*-linux-musl*` targets are supported for paths
  with no `leo4-mslean4` runtime and no `lake`
  invocation. The leo4 source needs no per-target
  branches; one host quirk (Arch's `musl-clang`
  wrapper missing freestanding headers) is auto-fixed
  by `leo4-rust-bridge`'s `build.rs`. `*-linux-android*`
  (C6) deferred to v1.x with the same path scope.

- *Leo4.Platform layer*. `lake/Leo4/Leo4/Platform.lean`
  is the first leo4-Lean OS abstraction layer.
  Centralises `.so` / `.dylib` / `.dll` choice and
  the POSIX-only `-Wl,-rpath` emission previously
  hardcoded in `Leo4.Build`.

- *Windows IPC worker side*. Phase 9-4c's missing
  half landed; `leo4-rust-worker`'s
  `open_windows_pipe` opens the dispatcher's named
  pipe via `CreateFileW` with retry on the spawn
  race. Cross-compile clean on
  `x86_64-pc-windows-gnullvm`.

If you read the learning material end-to-end before
2026-05-24, no architectural decision changed — this
update is about *which* RC blockers landed and which
user-visible surface expanded.

= Update — 2026-05-29

After the 2026-05-24 RC progress batch, three further
work streams landed before the v1.0 RC tag. None of
them change the architecture, but they change which
surfaces a user can rely on today.

== Function-arrow callback ABI (Phase 10-B1.x)

The Phase 10-B1 work mangled the function-arrow type at
the IDL + wire layer on 2026-05-21. The runtime side
landed across 2026-05-28..29 in two halves:

*Outbound direction* (Rust passes a Rust closure to Lean
through `leo4::import!`):

- `leo4-abi` now ships `RustCallbackRegistry` — a
  main-side per-`Lean` registry that mints `u64`
  `callback_id`s, stores the closure under that id, and
  enforces the per-call lifetime contract through an
  RAII `RegistrationGuard`. Drop deregisters.
- `Lean::callback_registry()` exposes the per-instance
  Arc<RustCallbackRegistry> so the macro layer can grab
  it without needing thread-local state.
- The `leo4::import!` macro recognises `fn(T₁,…,Tₙ) -> R`
  parameter types automatically: the emitted wrapper
  registers the closure on entry, encodes the
  `callback_id` (u64 LE) into the canonical args buffer,
  calls the shim, then drops the guard on return.
  Generic `impl Fn(...) -> R` intentionally not
  supported — users pass `fn` pointers and explicit
  state through other channels.
- `OxiLeanInvoker::attach_outbound_registry(...)` /
  `invoke_outbound(...)` / `register_outbound_dispatch_callback(...)`
  wire the dispatch back: when the OxiLean evaluator
  reaches a Lean closure dereference (the Lean side of
  the boundary calls the Rust `fn`), the bridge
  callback unpacks `(callback_id, rest) = (u64 LE
  prefix, &args[8..])` and forwards to the registered
  closure.

*Inbound direction* (Rust receives a Lean closure into a
`#[leo4::export]` body) was wired in `83cbbcc`
(2026-05-28) via `LeanCallback<R, Args>` + the
`CallbackInvoker` trait. The two halves use the same
`callback_id: u64` wire shape (SPEC §13a), differing
only in which side mints and which side dereferences.

== `oxilean_runtime::driver` IO walker (#76 P0c)

The fork branch `0.1.3-leo4-ox7` grew a new
`oxilean_runtime::driver` module that drives an
elaborated `def main : IO α := …` to its IO effects under
an installed `ExternResolver`. The walker as of
2026-05-29 recognises:

- `IO.pure` / `Pure.pure` — nullary terminal (action
  complete, result discarded).
- `IO.bind α β m k` (arity-4) and `Bind.bind m k`
  (arity-2 after implicit erasure) — walk `m` then `k`.
  Beta-application of `k` with a concrete result feed
  from `m` is the next sub-step.
- `@[extern]`-attributed `Const` reductions —
  `dispatch_extern_const` against the supplied
  `ExternRegistry`; resolver returns drive the action
  forward.

Everything else returns
`DriverError::NotYetImplemented` with the offending
expression's debug repr in the reason field —
downstream knows exactly which shape needs wiring.

The upstream API is being discussed at
`cool-japan/oxilean#2`; submission of a body PR is
deferred until the API shape gets explicit maintainer
feedback.

== Distro audit infra + Windows support floor

A new `just linux-distro-audit <distro>` recipe lands at
`ci/linux-distro-audit/`. Distros are data-driven via
`distros.toml`; the runner picks up new entries
automatically with no hard-coded distro names in the
Python driver. Initial set (current stable as of
2026-05-29): archlinux, debian-13, ubuntu-26.04,
fedora-44, alpine-3.22.

The Windows support floor is now pinned to UCRT's own
officially-supported range: Windows Vista SP2 +
KB2999226 (NT 6.0) or Windows 7 SP1 + KB3118401
(NT 6.1), through Windows 11 / Server 2025+. The KB
install is the *downstream application developer's*
deployment concern — leo4 doesn't redistribute or
document an end-user-facing UCRT install flow.

== Leaf-crate dedup

Two sibling leaf crates land to dedupe code previously
vendored twice (in `sibling/leo4-oxilean-build/` and
`sibling/leo4-oxilean-runner/`):

- `sibling/leo4-oxilean-bootstrap/` — OX5-oxi env
  bootstrap + leo4 boundary primitive axioms.
- `sibling/leo4-oxilean-translate/` — OX6 step 13 PEG →
  legacy-Decl translator.

Both consumers' previous ~1880-LOC vendored copies
collapse to one-line `pub use <leaf>::*;` re-export
shims.

If you read the learning material end-to-end before
2026-05-29, the architectural picture is unchanged —
this update is about *which* runtime surfaces are now
callable and which were dedup'd into leaf crates.

= Update --- 2026-05-31

== OxiLean driver walker --- coverage closure

The 2026-05-29 walker entry described "v0 covers
`IO.pure`, `IO.bind`, and `@[extern]` Const dispatch;
everything else returns `NotYetImplemented`". That
gap list is now closed. The 2026-05-31 walker
recognises:

- *`IO.pure` family* across the full monad
  transformer chain --- `IO.pure` / `Pure.pure` /
  `EIO.pure` / `EStateM.pure` / `ExceptT.pure` /
  `StateT.pure` / `ReaderT.pure` (plus the underscore-
  mangled spellings the elaborator's unfold may pick).
- *`IO.bind` family* with same transformer-chain
  coverage. Beta-application of the continuation `k`
  against `m`'s concrete result (when statically
  known) lands in this batch too --- `IO.bind
  (IO.pure x) k` now reduces to `k x` instead of
  walking `k` as opaque.
- *`@[extern]` Const dispatch with canonical-ABI arg
  encoding*. The walker statically lowers `Nat` /
  `String` literals, `Bool.true` / `Bool.false`,
  `Unit.unit`, sized-integer typeclass projections
  (`OfNat.ofNat <type> n` over UInt8..128 / USize /
  Int8..128 / ISize), signed-integer negation
  (`Neg.neg`), `Char.ofNat` Unicode code points,
  named record / variant ctors (`Prod.mk` /
  `Subtype.mk` / `Option.some` / `Sum.inl` etc.),
  *and* user-defined record / inductive ctors via
  env-lookup (`ConstantInfo::Constructor` →
  `Inductive.ctors.len()` discriminant prefix when
  multi-ctor, recursively encoded fields).
- *Stdlib `IO.println` family* --- `IO.println` /
  `IO.eprintln` / `IO.print` / `IO.eprint` fire
  directly against `print!`/`println!`/`eprint!`/
  `eprintln!` ahead of the resolver, so embedders
  don't have to layer a resolver for the common
  stdout / stderr write path.
- *Stdlib `IO.FS.*` family* --- `readFile` /
  `writeFile` / `appendFile` / `removeFile` /
  `createDir` / `createDirAll` / `removeDir` /
  `removeDirAll` / `rename` fire directly against
  `std::fs`. `readFile` surfaces contents as
  `Ok(Some(Lit(Str)))` so an enclosing `IO.bind m k`
  beta-applies `k` against the bytes. `std::io::Error`
  wraps into `DriverError::ExternFailed` via
  `ExternCallError::CallbackFailed`.

The remaining `NotYetImplemented` arm fires only for
shapes that are *out-of-scope by design*: non-IO
monad-class run projections (`StateT.run` /
`ReaderT.run` / `ExceptT.run` --- belong at the LCNF
/ bytecode interpreter layer, *below* the walker),
`IO.FS.Handle.*` (host-side `File` lifetime modelling
the walker doesn't carry), compile-time hooks
(`dbg_trace` / `panic!` / `unreachable!` ---
elaborator-handled), and float-literal lowering
(constant-folded by reducer). Embedders intercept via
the resolver when needed.

cool-japan/oxilean#2 (driver-API coordination issue)
remains the upstream contribution path; submission of
a body PR is deferred until the API shape gets
explicit maintainer feedback. As of 2026-05-31 the
discussion has been silent for 3+ days.

== Rust-transpile translate coverage tail (#72 OX7)

The 2026-05-27 OX7 typeclass-step covered arithmetic
`+` / `-` / `*` / `/` / `%` / `^` + comparison `<` /
`<=` / `==` / `=`. The 2026-05-31 translate coverage
tail closes the rest:

- *BinOp coverage expand* across `>` / `≥` / `≠` /
  `&&` / `||` / `↔` / `∈` / `∉` / `⊆` / `→` (Unicode
  arrow). `arith_op_to_tc_projection` now returns a
  three-arm `BinOpMapping` enum: `Direct(tc)` for
  ordinary projections, `Swapped(tc)` for `>` /
  `≥` (Lean stdlib expresses these as flipped `<` /
  `≤`), `Negated(tc)` for `≠` / `∉` (`¬ (Eq.eq a
  b)` / `¬ Membership.mem a b`). Unicode arrow `→`
  joins ASCII `->` at the special-case Pi-lowering
  head.
- *Explicit `By` / `DotFn` / `Raw` Expr arms* ---
  each variant now carries an actionable diagnostic
  (term-mode/axiom rewrite hint for `By`,
  `fun x => …` rewrite hint for `DotFn`, parser-
  shape-coverage hint for `Raw`). The Expr match is
  now exhaustive over `L4Expr` --- future parser
  variants force a build break, intentional safety.
- *Decl coverage* --- `DefinitionByArms`
  (equational `def NAME : T | pat₀ => body₀ | …`)
  desugars into a `Definition` whose body is
  `fun <binders> => match <last_binder> with
  <arms>`. `Mutual { decls }` wraps the inner
  translations into `OxDecl::Mutual { decls:
  [<translated>] }`. Inner attribute wrappers
  preserved.

translate test count: 36 → 56 across the three sub-
commits. The remaining `TranslateError::Unsupported`
arms (Class binders / Instance Term body / Open
multi-item / Dsl / HashCommand / Omit / Include)
carry explanatory messages but stay deferred ---
they need either a 1→N translate-API change or
oxilean-side variant support to close cleanly.

= Update --- 2026-05-31: RC.2~RC.4 typed-enum closure

The flagship scenario this batch closes: a user
writes a Rust enum with named-field variants
(struct-variant sum type) and uses it directly in
`#[leo4::export]`. Concretely, something like

```rust
#[derive(Clone, Debug, LeanMarshal)]
pub enum AdsmtVerdict {
    Sat { model: Vec<(String, String)> },
    Unsat { core: Vec<String>, cert: String },
    Abductive { candidates: Vec<AbductiveCandidate> },
    Unknown { reason: String },
}

#[leo4::export]
pub fn solve(v: AdsmtVerdict) -> u64 { … }
```

Pre-RC.2 this hit four separate walls in series:
`leo4-rust-emit::lean_type_of_mangle` didn't decode
user-defined-nominal mangle prefixes (`S_<fqn>_s`,
`V_<fqn>_v`, etc.) and fell through to a
`panic!`-bodied stub wrapper; `leo4-rust-emit` had
no path to emit a mirror Lean `inductive` for
`AdsmtVerdict` so the user had to hand-write the
Lean side; `leo4::import!`'s `rust_type_to_idl`
returned `None` for user-defined idents which broke
multi-instantiation imports without `#[leo4(args =
"…")]` hints; and `#[leo4::export]` itself rejected
user-defined types in param/return positions.

== What's now in place

- *Patch 1* (`b260ed8`) --- `lean_type_of_mangle`
  decodes all 5 user-defined-nominal mangle
  prefixes, so the wrapper signature gets the right
  Lean type instead of falling through to a
  `panic!` stub.
- *Patch 2* (`b260ed8`) --- a new
  `linkme::distributed_slice` channel
  `leo4_abi::rust_exports::USER_TYPES` carries one
  `UserTypeEntry` per `#[derive(LeanMarshal)]` site
  with `(fqn, kind, fields, ctors)`. The derive
  macro auto-emits the entry; `leo4-rust-emit`
  reads the slice via the new FFI symbol
  `leo4_rust_describe_user_types` and emits real
  Lean `structure` / `inductive` mirror decls with
  `deriving Leo4.LeanMarshal`. A new
  `rust_type_to_lean_type` translator (syn-based
  AST walk) handles nested types: `Vec<(String,
  String)>` → `Array ((String × String))`,
  `Vec<u8>` → `ByteArray`, `Result<T, E>` →
  `Except E T`, etc.
- *Patch 2 follow-up* (`cfda354`) --- removed
  `#[cfg(feature = "rust-exports")]` from the
  derive emit so downstream user crates don't see
  `unexpected_cfg` lints. `linkme` becomes an
  unconditional dependency; the `rust-exports`
  feature stays as a no-op alias for backward
  compatibility.
- *Patch 3* (`29a941f`) ---
  `leo4::import!`'s `rust_type_to_idl_candidates`
  returns all 5 kind candidates for user-defined
  idents. The macro takes the Cartesian product
  over args, then the first mangling-JSON hit wins.
  `leo4::import! { fn solve(v: AdsmtVerdict) ->
  AdsmtVerdict; }` now resolves Variant via
  candidate iteration even when the export has
  multiple instantiations.
- *Patch 4* (`5d786f0`) --- a strict
  `rust_type_to_idl` lowers user-defined idents to
  `Record { fqn, args }` so `#[leo4::export]`
  accepts them in fn param/return positions.
  Lifetime-arg paths (Cow, etc.) still reject.
  Scope locked: attaching `#[leo4::export]` to
  enum/struct items themselves still parse-errors
  on purpose --- type wire format is
  `#[derive(LeanMarshal)]`'s job.

== End-result

The flagship `AdsmtVerdict` scenario now ships with
*zero hand-written Lean* on the reverse direction.
The auto-generated wrapper Lean file contains both
the mirror inductive and the typed `IO`-returning
wrapper:

```lean
inductive AdsmtVerdict where
  | sat (model : Array ((String × String))) : AdsmtVerdict
  | unsat (core : Array String) (cert : String) : AdsmtVerdict
  | abductive (candidates : Array AbductiveCandidate) : AdsmtVerdict
  | unknown (reason : String) : AdsmtVerdict
  deriving Leo4.LeanMarshal

def solve (a0 : AdsmtVerdict) : IO (UInt64) := do …
```

Workspace test count moved 254 → 260 (RC.3) → 262
(RC.4); the most relevant crates are
`leo4-macros-backend` 16 → 22 → 24 and
`leo4-rust-emit` 20 → 29.
