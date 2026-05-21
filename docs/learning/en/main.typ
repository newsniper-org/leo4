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

== `leo4-native` (loader + dispatch)

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
  [5], [C shim synthesis + `leo4-native` + `leo4-macros` +
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
)

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
