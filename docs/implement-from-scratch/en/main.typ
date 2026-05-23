#import "../../template/leo4-book.typ": book
#show: book.with(
  title: "leo4 — Implement-From-Scratch Guide",
  subtitle: "English Edition",
  author: "윤병익 (leo4 project)",
  lang: "en",
)

= Preface

This book walks you through building leo4 from nothing,
landing each capability in the same order the original project
did. By the end you will have:

- A Lean 4 library that exposes the `@[leo4_export]` attribute
  and the `LeanMarshal` typeclass.
- A Lake plugin that discovers exports, computes a stable
  schema hash, mangles symbols, and emits a C shim plus a
  handshake file.
- A Rust workspace that loads the shim via `libloading`,
  encodes calls into canonical-ABI bytes, dispatches through
  the mangled table, and decodes the result --- all behind a
  procedural macro that surfaces a clean `fn add(a: u64,
  b: u64) -> u64;`-style declaration.
- Optional extensions: WIT lowering, generic-record support,
  mutual recursion, async `io<T>`, Mathlib-flavoured carrier
  types.

This book does not duplicate `SPEC/*.md`. The specs are
normative; this book builds toward them. When the spec
disagrees with this book, follow the spec.

== What you need

- A Lean 4 toolchain matching the version pinned in
  `lean-toolchain` (the published leo4 uses `v4.29.1`).
- A Rust toolchain ≥ 1.85 (Edition 2024).
- A C compiler (`clang` or `gcc`) the Lake plugin can drive
  via `leanc`.
- `cargo`, `lake`, `just`, `jq`, `wasm-tools` (for the
  optional WIT chapter).
- Several hours of focused time. The pipeline is deep; you
  cannot build it in a lunch break.

== Layout

This book follows the phase ladder
(`ROADMAP.md` in the published project). Each part lands a
single capability end-to-end. After each part you can run a
demo and see something move.

#table(
  columns: (auto, 1fr),
  table.header[*Part*][*Lands*],
  [I],   [Lean runtime library and the `@[leo4_export]`
          attribute.],
  [II],  [Lake plugin scaffold; admit-set algorithm for
          generics.],
  [III], [The IDL: types, mangling, schema hash. Stable
          contract between Lean and Rust.],
  [IV],  [Canonical-ABI marshalling: Lean side
          (`LeanMarshal` typeclass) and Rust side (the
          `LeanMarshal` trait).],
  [V],   [C shim emission: per-export `leo4_call_<mangled>`
          translation units.],
  [VI],  [Rust loader (`leo4-native`) and the
          `leo4::import!` proc-macro.],
  [VII], [WIT lowering pass and `wasm-tools` validation
          (optional but cleanly separable).],
  [VIII],[Mutual recursion + `Cyc<i>`.],
  [IX],  [Async `io<T>`, WASIp3 sibling project.],
  [X],   [Mathlib-flavoured carrier types and bridges.],
)

You can stop at the end of any part and have a working,
useful system. Stopping after part V already gives you a
working Lean-Rust round-trip for scalars and strings. Stopping
after part VIII gives you the full Phase-6 surface that
production code might need.

= Part I --- The Lean runtime library

Start by carving out the Lean-side surface. The plugin and the
Rust loader build on top of attributes, typeclasses, and the
canonical-ABI encoder/decoder; the Lean library is where they
live.

== Project layout

Create a Lake package:

```
lake/
  Leo4/
    Leo4.lean          -- top-level re-export
    Leo4/
      Syntax.lean      -- leo4_constraint syntax category
      Export.lean      -- @[leo4_export] attribute
      Marshal.lean     -- LeanMarshal typeclass + LeanError
      Resource.lean    -- LeanResource marker
      Builtins.lean    -- LeanMarshal instances for primitives
      Deriving.lean    -- deriving LeanMarshal handler
      Build.lean       -- user-facing Build helpers
    lakefile.lean
```

The `Leo4` library will be `require`d by every downstream
package. Keep the top-level imports minimal.

== Defining `LeanError`

Every Lean-side fallible operation needs a way to return an
error code + message. We use a flat structure:

```lean
namespace Leo4

structure LeanError where
  code : UInt32
  detail : String
  deriving Repr

namespace LeanError
def mk' (code : UInt32) (detail : String) : LeanError := { code, detail }
end LeanError

end Leo4
```

The error codes follow `SPEC/canonical-abi.md` §13. Reserve
`0x00000001` for `decodeError`, `0x00000005` for handshake
mismatch, `0x00000007` for return-buffer-too-small, `0x00000064`
for unimplemented. Define them as `def` constants in the same
file.

== The `@[leo4_export]` attribute

`@[leo4_export]` is an empty marker attribute. The plugin
discovers tagged declarations by querying the attribute
extension. Lean 4 provides `registerBuiltinAttribute` for this.

```lean
import Lean
namespace Leo4

initialize leo4ExportAttr : Lean.TagAttribute ←
  Lean.registerTagAttribute `leo4_export
    "marks a declaration as a leo4 boundary export"

end Leo4
```

Sanity check: define a trivial export in another module and
verify `leo4ExportAttr.hasTag (← getEnv) ``YourModule.add`
returns `true`.

== The `LeanMarshal` typeclass

`LeanMarshal` is the typeclass every cross-boundary value
implements. The encode side appends bytes to a `ByteArray`;
the decode side reads from one and returns `(value, newOffset)`
plus an error path.

```lean
namespace Leo4

class LeanMarshal (T : Type) where
  canonicalEncode : T → ByteArray → ByteArray
  canonicalDecode : ByteArray → Nat → Except LeanError (T × Nat)

end Leo4
```

The Lean `ByteArray` is a packed `Array UInt8`. `Nat`
offset gives unbounded indexing for safety; the wire format's
length prefixes contain the bound.

== Built-in instances

Start with the scalar types: `UInt8`, `UInt16`, `UInt32`,
`UInt64`, `Int8` through `Int64`, `Float`, `Float32`, `Bool`,
`Char`. For each, encode as little-endian bytes; decode reads
the same.

A representative implementation, `UInt32`:

```lean
namespace Leo4

instance : LeanMarshal UInt32 where
  canonicalEncode n buf :=
    let b0 := (n.toUInt8)
    let b1 := ((n >>> 8).toUInt8)
    let b2 := ((n >>> 16).toUInt8)
    let b3 := ((n >>> 24).toUInt8)
    buf.push b0 |>.push b1 |>.push b2 |>.push b3
  canonicalDecode buf off := do
    if off + 4 > buf.size then
      throw (LeanError.mk' 1 "u32: out of bounds")
    let v : UInt32 :=
      buf[off]!.toUInt32 |||
      (buf[off+1]!.toUInt32 <<< 8) |||
      (buf[off+2]!.toUInt32 <<< 16) |||
      (buf[off+3]!.toUInt32 <<< 24)
    return (v, off + 4)

end Leo4
```

Repeat for every primitive. Then build composite instances:

- `String` --- `u32 len + utf-8 bytes`.
- `List T` --- `u32 len + N elements`.
- `Option T` --- `u8 disc + payload`.
- `Except E T` --- `u8 disc + payload`.
- `α × β` --- two elements concatenated.
- `Nat` (`bignat`) --- `u32 limb count + LE u64 limbs`.
- `Int` (`bigint`) --- `u8 sign + bignat magnitude`.

Each instance has both directions. Write decode-side bounds
checks aggressively; the wire input is untrusted by definition.

== The deriving handler

`#[derive(LeanMarshal)]` on the Rust side and `deriving
LeanMarshal` on the Lean side both synthesise field-wise
encode/decode for user-defined types. On the Lean side this
uses `registerDerivingHandler`:

```lean
namespace Leo4.Deriving

open Lean Elab Command Meta

private def mkLeanMarshalHandler (declNames : Array Name)
    : CommandElabM Bool := do
  let env ← getEnv
  for declName in declNames do
    let some (.inductInfo indVal) := env.find? declName
      | return false
    -- Dispatch on shape: single ctor → record, all-nullary
    -- multi-ctor → enum, mixed → variant.
    -- (Detail in Deriving.lean of the published project.)
    pure ()
  return true

initialize
  registerDerivingHandler ``Leo4.LeanMarshal mkLeanMarshalHandler

end Leo4.Deriving
```

The handler's body is the hard part: it walks the inductive's
constructors, builds the encode arms (one match-arm per ctor
that pushes the discriminator then encodes each field), then
the decode arms (one per discriminator that pulls the inner
fields). For Phase 6 mutual support, all members of a
`mutual ... end` cluster get one `mutual ... end` block of
`partial def` encoders / decoders + one instance each.

Land a single-shape (e.g., record only) implementation first.
You can add enum / variant / mutual support in later phases.

== Sanity check

Write a Lean file outside the Leo4 library:

```lean
import Leo4

structure Point where
  x : Float
  y : Float
  deriving Leo4.LeanMarshal

#eval do
  let p : Point := ⟨1.5, 2.5⟩
  let buf : ByteArray := Leo4.LeanMarshal.canonicalEncode p ByteArray.empty
  IO.println s!"encoded {buf.size} bytes: {buf.toList}"
  let (p', off) := match Leo4.LeanMarshal.canonicalDecode (T := Point) buf 0 with
    | .ok r => r | .error e => panic! s!"decode: {e.detail}"
  IO.println s!"decoded x={p'.x} y={p'.y}, ate {off} bytes"
```

You should see 16 bytes encoded (two `f64`s) and round-trip
back. If not, fix the encoder/decoder before moving on.

= Part II --- The Lake plugin scaffold

The Lake plugin is a `lean_exe` that runs after `lake build`.
It walks every `@[leo4_export]` definition in the user's
package, computes its admit-set (for generics), and emits the
artefacts listed in chapter 3 of the learning material.

== Project layout

```
lake/
  Leo4Plugin/
    Leo4Plugin.lean      -- top-level
    Leo4Plugin/
      AdmitSet.lean      -- IDLType + UserDecl ADT, admit-set algo
      Mangling.lean      -- mangleType, schema hash
      Emit.lean          -- file writers, JSON shapes
      Main.lean          -- the runPlugin driver
    Main.lean            -- exe entry point
    lakefile.lean
```

`lakefile.lean` declares the package, requires `Leo4` (the
runtime library from part I), and exposes `lean_exe
leo4plugin` whose root module is `Main.lean`.

== Discovering exports

The plugin runs as a standalone executable. It receives the
user's module name as a command-line argument:

```
$ lake exe leo4plugin Sample
```

The entry point loads the user's compiled modules via
`Lean.importModules (loadExts := true)`, then walks the
environment looking for `@[leo4_export]`-tagged decls:

```lean
def gatherExports (env : Environment) : Array Name := Id.run do
  let mut out : Array Name := #[]
  for (n, _) in env.constants do
    if Leo4.leo4ExportAttr.hasTag env n then
      out := out.push n
  return out
```

This produces a sorted list of `Name` values to analyse. For
each, the analyser computes the function's type, splits it
into generic parameters / value parameters / return type, and
lowers each to the IDL.

== The IDLType ADT

The plugin's IDL representation is a Lean inductive that
mirrors the Rust-side `schema-idl::IDLType`. Define it in
`AdmitSet.lean`:

```lean
inductive IDLType where
  | u8 | u16 | u32 | u64
  | i8 | i16 | i32 | i64
  | f32 | f64
  | bool | char | string
  | bigint | bignat
  | list (t : IDLType)
  | option (t : IDLType)
  | result (t : IDLType) (e : Option IDLType)
  | tuple (ts : Array IDLType)
  | record (fqn : String) (args : Array IDLType)
  | variant (fqn : String) (args : Array IDLType)
  | enumT (fqn : String)
  | flagsT (fqn : String)
  | resource (fqn : String) (args : Array IDLType)
  | io (t : IDLType)
  | self
  | selfApp (args : Array IDLType)
  | cyc (i : UInt32)
  deriving Repr, Inhabited, BEq
```

Variant by variant, this is the canonical IDL. The Rust side
mirrors this exactly (modulo Rust naming conventions); the
mangling rule (chapter 5) maps each to a stable ASCII string.

The `UserDecl` ADT collects nominal-type declarations:

```lean
inductive UserDecl where
  | record   (fqn) (generics : Array Name) (fields : Array (Name × IDLType))
  | enumT    (fqn) (cases : Array Name)
  | variant  (fqn) (generics : Array Name) (cases : Array (Name × Array IDLType))
  | resource (fqn) (generics : Array Name)
  | mutual   (members : Array UserDecl)
  | externalMarshal (fqn) (generics : Array Name)
```

The two extra constructors (`mutual` and `externalMarshal`)
land in phases 6 and 8; they're listed here so the ADT is
complete from the start.

== Walking an export

For each tagged `Name`, fetch its `ConstantInfo`, telescope its
type, and lower each binder:

```lean
def analyzeExport (n : Name) : MetaM (Option ExportAnalysis) := do
  let env ← getEnv
  let some info := env.find? n | return none
  Meta.forallTelescope info.type fun args body => do
    -- Classify binders into:
    --   - implicit kind-typed → generic type parameter
    --   - implicit value-typed → erased value parameter
    --   - inst-implicit → typeclass constraint
    --   - explicit → value parameter at the boundary
    -- Then lower each value-parameter type via exprToIDLSubst.
    sorry
```

`exprToIDLSubst` is the recursive type lowerer: given a Lean
`Expr` and a substitution map (from generic binders to
concrete `IDLType` values), it returns the corresponding
`IDLType` or `none` if it can't be lowered. Special cases for
`List`, `Option`, `Except`, `Prod`, `IO`, and the Self
short-circuit. User-defined inductives lower to
`record`/`variant`/`enumT`/`resource` based on the inductive's
shape.

The crucial detail: use `Meta.forallTelescope` (no reducing)
so the original `IO α` shape survives. The reducing variant
unfolds `IO α = IO.RealWorld → EStateM …` which exposes
spurious `IO.RealWorld` params.

== The admit-set algorithm

For an export with generic parameters, the plugin enumerates
all instantiations that satisfy the binder's constraints. For
each combination, it produces a separate IDL signature and
mangled name. The algorithm:

1. For each generic `T_i`, determine its admit-set: the set
   of `IDLType` values it may take. Default: all primitives
   (`unboundedAdmitSet`). Class-constrained: intersect with
   `classAdmitSet` for each class.
2. Compute the cartesian product. Each tuple is one
   instantiation.
3. For each instantiation, substitute into the parameter
   types and produce a `paramInfo` array.

Phantom generics (binders not referenced anywhere) skip the
combinatorial blowup --- emit one instantiation with phantom
slots set to `none`.

This algorithm is in `Main.lean`'s `analyzeExport` in the
published codebase. Read it once before re-implementing; the
edge cases (higher-kind generics, value generics, generic args
inside Self-recursive types) take a while to get right.

= Part III --- Mangling and schema hash

Once you have `IDLType` values + a list of exports + the
discovered user types, you can produce the stable text form
the schema hash consumes.

== `mangleType`

`mangleType : IDLType → String` is byte-identical between
Lean and Rust. Each constructor maps to a fixed token:

```
u8 → "u8"           list T  → "L_" ++ mangle T ++ "_l"
u16 → "u16"         option T → "O_" ++ mangle T ++ "_o"
...                 result T none → "Rz_" ++ mangle T ++ "__z"
i8 → "i8"           tuple [A,B] → "T_" ++ mangle A ++ "_" ++ mangle B ++ "_t"
...                 record fqn args → "S_" ++ fqnSeg fqn ++ ... ++ "_s"
                    variant fqn args → "V_" ++ ...
                    enum fqn → "E_" ++ fqnSeg fqn ++ "_e"
                    resource fqn args → "X_" ++ ...
                    Self → "self"
                    Cyc<i> → "c" ++ toString i ++ "c"
```

The full rule is in `SPEC/mangling.md` §2.

== The full mangled name

```
leo4__<pkg_seg>__<iface>__<fname>__<arg_mangles>__h<schema_hash>
```

Where `arg_mangles` joins the mangled forms of each parameter
type with underscores. The schema hash is the FNV-1a-64 of
the *normalised IDL form* (text), rendered as 13-character
base32lc (lowercase, no padding).

FNV-1a-64 is straightforward: offset basis 0xCBF29CE484222325,
prime 0x00000100000001B3, XOR each byte then multiply.
Base32lc uses the alphabet `abcdefghijklmnopqrstuvwxyz234567`
(RFC 4648 lowercase, no padding).

== Render canonical IDL

`renderCanonical : Config → Array UserDecl → Array Member → Bool → String`
produces the text:

```
package leo4-sample;
interface Sample {
  record Sample.Point { x: f64, y: f64 };
  variant Sample.Tree { leaf, node(Self, Self) };
  func add(_0: u64, _1: u64) -> u64;
  func midpoint(_0: Sample.Point, _1: Sample.Point) -> Sample.Point;
}
```

Two modes: `pretty := true` (newlines, indent) for the
on-disk `.leo4-schema` file; `pretty := false` (collapsed,
single space between tokens) for the schema-hash input.

Sort user decls by FQN within their band (records and enums
in band 0, resources in band 1, mutual clusters in band 0
preserving source order). Sort functions by name. Determinism
is non-negotiable --- the hash depends on byte-identical
output.

The hash input is the *collapsed* form. Run FNV-1a over its
UTF-8 bytes to get a `UInt64`; convert big-endian to a base32
string for the suffix.

== Sanity check

Write a small fixture program (Lean side):

```lean
@[leo4_export]
def add (a b : UInt64) : UInt64 := a + b
```

Run `lake exe leo4plugin Sample` and verify:

- `tests/sample-lean/.lake/build/leo4/leo4_sample.leo4-schema`
  is a sensible text file.
- `tests/sample-lean/.lake/build/leo4/leo4_sample.leo4-handshake`
  contains a 16-char base32lc schema hash (`schema_hash` JSON
  field).
- The hash is stable across re-runs.

Then implement the Rust mirror (`crates/schema-idl/`) in
parallel and add a cross-impl harness (`tests/mangling/`)
that compares the Lean output against `leo4c mangle <schema>`'s
output. They must match byte-for-byte.

= Part IV --- Canonical-ABI marshalling

The Lean library has its `LeanMarshal` typeclass. The Rust
side needs a matching trait. Both must produce identical
bytes for every value type they share.

== Rust trait

```rust
pub trait LeanMarshal: Sized + 'static {
    fn canonical_encode(&self, buf: &mut Vec<u8>);
    fn canonical_decode(buf: &[u8], off: usize)
        -> Result<(Self, usize), LeanError>;
}
```

`Vec<u8>` for encode (grows on demand), `&[u8] + off` for
decode (same shape as the Lean side). `LeanError` carries a
`u32` code + `String` detail, matching the Lean
`Leo4.LeanError`.

== Primitive impls

For each Rust primitive, write a direct impl:

```rust
impl LeanMarshal for u32 {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(&self.to_le_bytes());
    }
    fn canonical_decode(buf: &[u8], off: usize)
        -> Result<(Self, usize), LeanError>
    {
        if buf.len() < off + 4 {
            return Err(LeanError::new(
                error_codes::DECODE_ERROR,
                "u32: out of bounds",
            ));
        }
        let v = u32::from_le_bytes(
            buf[off..off + 4].try_into().unwrap(),
        );
        Ok((v, off + 4))
    }
}
```

Repeat for every primitive. The exact LE behaviour matters ---
write the bytes Lean's `(n.toUInt8, ..., (n >>> 24).toUInt8)`
chain produces.

== The conformance harness

The two sides must agree byte-for-byte. Build a fixture:

```
tests/conformance/
  fixtures/
    u32.lean       -- emits `u32 42` as bytes via Leo4.LeanMarshal
    u32.rs         -- emits `42u32` as bytes via leo4-abi
    point.lean     -- record example
    point.rs       -- same
    ...
  run.sh
```

`run.sh` runs both fixtures with the same logical value,
compares their byte outputs, and fails if any pair diverges.
This is the test that catches subtle byte-order mistakes
before they ship.

Land at least one fixture per type: every primitive, every
composite shape (list, option, result, tuple), and at least
two user-defined types (record, variant).

= Part V --- C shim emission

The C shim is where Lean's native ABI (`lean_object*`,
`lean_alloc_ctor`, `lean_io_result_*`, …) meets the canonical
ABI's byte stream. The plugin generates one `.c` file per
package, with one `LEO4_EXPORT int32_t
leo4_call_<mangled>(...)` entry point per export ×
instantiation.

== Shim source structure

The shim translation unit has:

```c
#include <lean/lean.h>
#include <stdint.h>
#include <stddef.h>

#define leo4_memcpy __builtin_memcpy
#define LEO4_EXPORT __attribute__((visibility("default")))

#define LEO4_OK                          0
#define LEO4_ERR_DECODE                  0x00000001
#define LEO4_ERR_HANDSHAKE_MISMATCH      0x00000005
#define LEO4_ERR_RETURN_BUF_TOO_SMALL    0x00000007
#define LEO4_ERR_IO_FAILED               0x00010001
#define LEO4_ERR_UNIMPLEMENTED           0x00000064

typedef void leo4_arena_t;

/* per-helper extern decls follow */
extern uint64_t leo4_lean__leo4__sample__Sample__add__u64_u64__h<hash>(uint64_t, uint64_t);

LEO4_EXPORT int32_t leo4_call_leo4__sample__Sample__add__u64_u64__h<hash>(
    leo4_arena_t* arena,
    const uint8_t* args_ptr, size_t args_len,
    uint8_t* ret_ptr, size_t ret_cap, size_t* ret_len)
{
    /* decode args */
    /* invoke */
    /* encode return */
}
```

The signature is fixed (`SPEC/canonical-abi.md` §14). The
loader binds against it via dlsym; the macro generates Rust
code that calls into it.

== Per-type handlers

The shim emitter's core data structure is `TyHandler`:

```lean
private structure TyHandler where
  cType        : String   -- e.g. "uint64_t"
  externCType  : String   -- C type in extern decl
  ownsRef      : Bool     -- needs lean_dec at end?
  scalarKind   : Option String  -- "uint8" etc. for ctor accessors
  ctorScalarSz : Nat
  decodeBlock  : String → String → String  -- (var, cleanup) → C
  encodeBlock  : String → String → String
  boxExpr      : String → String  -- value → lean_object*
  unboxExpr    : String → String  -- lean_object* → value
```

For each IDL type, the emitter resolves a `TyHandler`. Scalar
types use a generic `scalarHandler`. Strings use
`stringHandler` (delegates to a runtime helper).
Lists / options / results / tuples are higher-order ---
`listHandler ih` takes the inner type's handler and wraps it.

User-defined records produce a `recordHandler` over the
field handlers. Variants get their own emitter that produces
two helper functions per (fqn, args) instantiation
(`leo4_dec_Sample_Tree` and `leo4_enc_Sample_Tree` for
example), each handling the disc + payload.

Self-references inside variants recursively call the same
helper. Mutual clusters use `Cyc<i>` references that resolve
to the peer's helper at emit time (chapter VIII).

== The main render loop

```lean
def renderOneShim (cfg userDecls a schemaHash params ret) : String :=
  let mangled := mangle cfg.pkg cfg.iface a.fname (params.map ...) schemaHash
  let entry  := s!"leo4_call_{mangled}"
  let helper := s!"leo4_lean__{mangled}"
  -- Build paramHs : Array TyHandler from params via handlerFor.
  -- Build retH    : TyHandler from ret.
  -- If any handler is `none`, emit a LEO4_ERR_UNIMPLEMENTED stub.
  -- Otherwise emit the full decode → invoke → encode body.
  ...
```

Each export takes roughly 30-100 lines of generated C. The
result compiles via `leanc` (which is just clang with Lean's
include / library paths preconfigured) to produce the
`.so`.

== Sanity check

After running `lake exe leo4plugin Sample`, inspect
`<pkg>.leo4-shim.c`. A scalar `add(u64, u64) -> u64` should
look like:

```c
LEO4_EXPORT int32_t leo4_call_leo4__sample__Sample__add__u64_u64__h<hash>(
    leo4_arena_t* arena,
    const uint8_t* args_ptr, size_t args_len,
    uint8_t* ret_ptr, size_t ret_cap, size_t* ret_len)
{
    (void)arena;
    size_t off = 0;
    uint64_t a0;
    if (args_len - off < 8u) { *ret_len = 0; return LEO4_ERR_DECODE; }
    leo4_memcpy(&a0, args_ptr + off, 8);
    off += 8u;
    uint64_t a1;
    if (args_len - off < 8u) { *ret_len = 0; return LEO4_ERR_DECODE; }
    leo4_memcpy(&a1, args_ptr + off, 8);
    off += 8u;
    if (off != args_len) { *ret_len = 0; return LEO4_ERR_DECODE; }
    uint64_t r = leo4_lean__leo4__sample__Sample__add__u64_u64__h<hash>(a0, a1);
    size_t out_off = 0;
    if (ret_cap - out_off < 8u) { *ret_len = out_off + 8u; return LEO4_ERR_RETURN_BUF_TOO_SMALL; }
    leo4_memcpy(ret_ptr + out_off, &r, 8);
    out_off += 8u;
    *ret_len = out_off;
    return LEO4_OK;
}
```

Drive `leanc` (or `cc` with the right flags) on this `.c` plus
the Lean wrapper module's `.c` to produce `<pkg>.leo4-shim.so`.
Link the user package's `.so` (from `lake build`'s
`precompileModules`) at RPATH so the wrapper can call into the
user's compiled exports.

= Part VI --- Rust loader and the `import!` macro

You have a shim `.so`, a handshake file, and a mangling table.
The Rust side now binds against them.

== `leo4-native` --- the loader

`crates/leo4-native/` exposes `Lean::open`:

```rust
pub struct Lean { /* libloading::Library + meta */ }

impl Lean {
    pub fn open(
        so_path: impl AsRef<Path>,
        handshake_path: impl AsRef<Path>,
    ) -> Result<Self, LeanError> {
        // 1. Read the handshake JSON; extract schema_hash + wrapper_init_symbol.
        // 2. Verify schema_hash against the Rust-side constant.
        // 3. `Library::new(so_path)` via libloading.
        // 4. Initialise Lean runtime once per process
        //    (`lean_initialize_runtime_module`, then the wrapper's
        //    `initialize_<X>` symbol).
        // 5. Cache function pointers for each mangled symbol.
        ...
    }

    pub fn call_shim(
        &self,
        mangled_body: &str,
        args: &[u8],
        ret: &mut [u8],
    ) -> Result<usize, LeanError> {
        // Look up `leo4_call_<mangled>` via dlsym (cached).
        // Call it with (arena=NULL, args_ptr, args_len, ret_ptr,
        // ret_cap, &ret_len). Convert the int32_t status to a
        // Result.
        ...
    }
}
```

The runtime init is once-per-process via `std::sync::Once`.
The wrapper module's `initialize_<X>` symbol returns
`lean_io_result_is_ok`-style success; check it before
dispatching any user call.

== `leo4-macros-backend` --- the macro expander

`leo4::import!` is a function-procedural macro
(`#[proc_macro]`). It parses an extern-block-like input,
looks each `fn` up in the build-time mangling JSON, and
emits a Rust wrapper:

```rust
pub fn add(lean: &Lean, a: u64, b: u64) -> Result<u64, LeanError> {
    let mut args = Vec::<u8>::with_capacity(16);
    <u64 as LeanMarshal>::canonical_encode(&a, &mut args);
    <u64 as LeanMarshal>::canonical_encode(&b, &mut args);
    let mut ret = [0u8; 8];
    let mut ret_len = 0;
    lean.call_shim(MANGLED_BODY, &args, &mut ret)?;
    let (v, _) = <u64 as LeanMarshal>::canonical_decode(&ret, 0)?;
    Ok(v)
}
```

The macro's job is to choose the right `MANGLED_BODY`. It
reads `LEO4_MANGLING_FILE` (set by `leo4-build`) and matches
the function name + each argument's IDL form (computed via
`rust_type_to_idl`). For multi-instantiation generic exports,
a `#[leo4(args = "u64,str")]` attribute lets the user pick
explicitly.

== `leo4-build` --- the build-script helper

```rust
pub fn wire(lake_build_dir: &str) -> Result<(), String> {
    // Resolve absolute path of shim .so and handshake file.
    // Emit `cargo:rustc-env=LEO4_SHIM_SO=…`
    //       `cargo:rustc-env=LEO4_HANDSHAKE_FILE=…`
    //       `cargo:rustc-env=LEO4_MANGLING_FILE=…`
    //       `cargo:rerun-if-changed=…`
    ...
}
```

This is what makes `env!("LEO4_SHIM_SO")` work in the user's
`main.rs`. The macro reads `LEO4_MANGLING_FILE` (also via
`env!`) to resolve the mangling table.

== Putting it together

A complete consumer crate has:

```
my-app/
  Cargo.toml         # [dependencies] leo4 = "..."; [build-dependencies] leo4-build = "..."
  build.rs           # leo4_build::wire(<path>)
  src/main.rs        # mod sample { leo4::import! { ... } } fn main() { ... }
```

`cargo run` from `my-app/` builds the wrapper macro
expansions, links the shim `.so`, and the runtime call works
end-to-end.

= Part VII --- WIT lowering (optional)

The IDL is a WIT superset; you can lower any leo4 IDL to a
WIT file for consumption by Component Model tools.

== `leo4c lower`

A small Rust CLI (`crates/leo4c`) that reads a
`.leo4-schema` and emits a `.wit` file. The conversion:

- IDL `record R { f: u32 }` → WIT `record r { f: u32 }`.
- IDL `variant V { a, b(string) }` → WIT
  `variant v { a, b(string) }`.
- IDL `resource X` → WIT `resource x`.
- IDL `enum E { a, b }` → WIT `enum e { a, b }`.
- IDL `flags F { x, y }` → WIT `flags f { x, y }`.
- IDL `func f(_0: T) -> R;` → WIT
  `f: func(_0: t) -> r`.

Self-recursive variants in WIT are expressed via a
`resource` type (WIT doesn't allow direct self-recursion in
variant payloads). The lowering detects the recursion and
substitutes accordingly.

Verify the output via:

```
$ wasm-tools component wit <pkg>.wit  # parse + pretty-print
$ wit-bindgen markdown <pkg>.wit       # generate API docs
```

Both should accept the output without errors.

= Part VIII --- Mutual recursion + `Cyc<i>`

Phase 6 of the original project. Up to this point, recursion
goes through `Self` (one declaration recursing on itself).
Mutual recursion needs a way for two declarations to name each
other.

== IDL grammar additions

```
mutual_decl = "mutual" "{" nominal_decl nominal_decl { nominal_decl } "}" ";"
cyc_type    = "Cyc" "<" unsigned_int ">"
```

A `mutual` block contains ≥ 2 nominal declarations sharing a
`Cyc<i>` namespace. Inside any member, `Cyc<i>` refers to the
`i`-th member of the group in source order.

== Mangling rule

`Cyc<i>` → `c<i>c` where `<i>` is the ASCII-decimal index.
The schema hash is computed over the full normalised text
including `Cyc<i>` tokens, so a member-order swap rotates the
hash.

== Plugin work

The Lean plugin detects a mutual cluster via the
`InductiveVal.all` array. If `iv.all.length > 1`, dispatch to a
`walkMutualGroup` function that:

1. Calls `walkUserDecl` for each member with `mutualMembers =
   iv.all` so peer references rewrite to `Cyc<i>`.
2. Wraps the resulting `UserDecl` array in `UserDecl.mutual`.

The shim emitter's variant helper handler picks up `Cyc<i>`
payloads and emits cross-calls to the peer's
`leo4_dec_<seg>` / `leo4_enc_<seg>`. Both helpers live in the
same translation unit; a forward declaration at the top of
the shim header makes them visible at the call site.

The deriving handler emits one `mutual partial def …
end` block per cluster, then one `instance : LeanMarshal X`
per member. Cross-decl payload references route through the
peer's `<peer>._leo4_encode` / `_decode` directly rather than
through typeclass dispatch (which would forward-reference
the unfinished instance).

== Rust derive

Rust accepts forward references between top-level `impl`
blocks in the same module freely. `Box<T>`'s pass-through
`LeanMarshal` impl in `leo4-abi/composites.rs` lets recursive
Rust enum types like `Expr { Lit(u64), Seq(Box<Stmt>) }` be
sized without further macro work. `#[derive(LeanMarshal)]`
handles each enum independently and the cycle resolves at
compile time.

== Sanity check

Land a sample with a mutual cluster:

```lean
mutual
  inductive Expr where
    | lit  (n : UInt64)
    | seq  (s : Stmt)
    deriving LeanMarshal
  inductive Stmt where
    | nop
    | block (e : Expr)
    deriving LeanMarshal
end

@[leo4_export]
def exprIsLit (e : Expr) : Bool := match e with | .lit _ => true | .seq _ => false
```

After `lake exe leo4plugin Sample`, the schema should
contain:

```
mutual { variant Sample.Expr { lit(u64), seq(Cyc<1>) }; variant Sample.Stmt { nop, block(Cyc<0>) }; };
```

The Rust side defines mirror enums + hand-rolled (or derived)
`LeanMarshal` impls and calls `exprIsLit` through the macro.

= Part IX --- Async io<T> + WASIp3

Phase 7. The user-facing API stays sync on both targets (per
the design decision pinned in 2026-05-20); WASIp3 lets a sync
wasm export `block_on` async wasip3 futures internally.

== IDL surface

Lean's `def f : IO α` lowers to `IDLType.io α` in the
plugin's `exprToIDLSubst`. The canonical IDL renders it as
`future<α>` (Phase 7 lift). The Rust schema-idl parser
desugars `future<α>` into `FuncDecl { effect: Async, ret: α }`
at parse time so the round-trip stays symmetric.

== Shim IO unwrap

The Lean wrapper for `IO α` exports returns `lean_io_result α`
at the C level. The shim wraps the call:

```c
lean_object* io_res = leo4_lean__<mangled>(args);
if (!lean_io_result_is_ok(io_res)) {
    lean_dec(io_res); *ret_len = 0;
    return LEO4_ERR_IO_FAILED;
}
RetType r = scalarUnbox(lean_io_result_get_value(io_res));
lean_dec(io_res);
// encode r...
```

`scalarUnbox` dispatches per cType:
`lean_unbox_uint64` / `lean_unbox_uint32` / `lean_unbox` /
`lean_unbox_float` / `lean_unbox_float32`. Signed and
unsigned share the same C width; the cast at the call site
preserves sign interpretation.

== WASIp3 sibling

A standalone Cargo project under `sibling/leo4-wasip3/`,
*not* a member of the main workspace. Pins stable Rust + the
`wasm32-wasip2` target; depends on the `wasip3` crate (which
ships WASIp3 API bindings as compat shims on wasip2's
Component Model).

The sibling implements `leo4_wasip3::Lean::open` analogously
to `leo4_native::Lean::open`, but the dispatch goes through
wasip3 host imports (defined in a WIT file the host
implements). `futures::executor::block_on` drives any async
import while the user-facing Rust API remains sync.

== Sanity check

Land an `IO`-flavoured Sample export:

```lean
@[leo4_export]
def asyncDouble (n : UInt64) : IO UInt64 := return n * 2
```

The schema should show `func asyncDouble(_0: u64) -> future<u64>`.
The Rust caller writes `fn asyncDouble(n: u64) -> u64;` and gets
`asyncDouble(21) == 42`.

= Part X --- Mathlib-compatible carrier types

Phase 8. leo4 stays Mathlib-independent per ROADMAP §8 ---
the runtime library doesn't import Mathlib. But it ships
carrier types (`LeanRat`, `LeanU128/I128`, `LeanComplexF*x2`,
`LeanF16/BF16/F128` nightly) that round-trip to / from the
abstract Mathlib types (`ℚ`, `ZMod (2^128)`, `Complex ℝ`,
`ℝ`).

== Wide ints

`Leo4.LeanU128 { lo : UInt64, hi : UInt64 }` and matching
`LeanI128`. Wire is 16 bytes LE; the field-wise encode from
`deriving LeanMarshal` produces the same byte stream as
Rust's `u128::to_le_bytes()`. Rust's macro maps bare `u128`
to the `Leo4.LeanU128` IDL form via `rust_type_to_idl`.

== Machine complex

`Leo4.LeanComplexF{32,64}x2 { re, im : Float* }`. The naming
convention `F<bits>x<components>` extends to quaternion
(`xN=4`) / octonion (`xN=8`) carriers later.

== Nightly floats

`LeanF16`, `LeanBF16`, `LeanF128` plus the matching complex
carriers, gated behind the `nightly-floats` cargo feature.
Rust's `f16` / `f128` primitives are nightly via
`#![cfg_attr(feature = "nightly-floats", feature(f16, f128))]`;
`bf16` has no native Rust primitive yet, so we carry the bit
pattern as a `u16` newtype.

Lean side has no native `Float16` / `Float128`; the carriers
wrap raw bit patterns (`UInt16` or two `UInt64`s).

== External marshal (`Rat`)

Lean core's `Rat` has proof-carrying fields (`den_nz`,
`reduced`) that the plugin can't lower. The
`UserDecl.externalMarshal` path treats them as opaque blobs
at the IDL level; the shim emitter routes encode / decode
through Lean-emitted C-callable helpers
(`leo4_marshal_Rat_dec` / `leo4_marshal_Rat_enc`) that wrap
`Leo4.LeanMarshal.canonicalDecode/Encode`. The shim does the
`uint8_t* ⇄ ByteArray` glue via `lean_alloc_sarray` +
`leo4_memcpy`.

== Mathlib bridges

Each carrier ships with an opt-in
`Leo4.MathlibBridge.<Sub>` module. The bridges:

- `Wide` — `LeanU128/I128 ↔ Nat / Int / BitVec 128 / ZMod (2^128)`.
- `Complex` — `LeanComplexF{32,64}x2 → ℂ` via `Float.toReal`.
  Reverse direction `ℂ → LeanComplexF*x2` is `noncomputable`
  (Mathlib's ℝ has no constructive `→ Float`).
- `NightlyFloats` — IEEE-754 bit decode `LeanF{16,BF16,128}
  → ℝ` via direct arithmetic on `Nat` field extracts.
  Reverse direction goes through `Rat` (computable subset of
  ℝ) using IEEE-correct round-to-nearest-even.
- `Rat` — Lean core `Rat` → `ℝ` / `ℂ` total embeddings via
  Mathlib's `Rat.cast`.

Rounding-mode policy: IEEE-754 round-to-nearest-even (RTNE).
That's what `Float.div` and the host FPU implement, so the
abstract-Real reverse path stays consistent with the
round-trip that native code already performs.

= Part XI --- Reverse direction (Rust → Lean)

Phase 9 (2026-05-21). leo4's *second* pipeline. Where Parts
I--X build Rust calling Lean, this part builds the inverse:
Lean calling Rust. The build orchestration is mirror-image
(cargo first, then Lake) and the schema_hash discipline is
different (handshake JSON, not mangled-name suffix). The
canonical-ABI wire format is reused unchanged.

== Macro surface (`#[leo4::export]`)

`crates/leo4-macros/src/lib.rs` gains a second proc-macro
attribute. Apply to any function:

```rust
#[leo4::export]
pub fn next_prime(n: u64) -> u64 { … }
```

The macro emits:

1. A per-fn wrapper `leo4_rust__<body>` with the
   canonical-ABI decode → call → encode pattern.
2. A `linkme::distributed_slice` entry that registers the
   fn's mangled body name + a function pointer to the
   wrapper.

`linkme::distributed_slice` collects every entry into a
single static array (`EXPORTS`) at link time. The dispatcher
walks that slice via a `dlsym`'d describer fn
(`leo4_rust_describe_exports`) to enumerate exports without
knowing them at compile time.

== Emit CLI (`crates/leo4-rust-emit`)

After `cargo build --release -p <user_pkg>` produces a
cdylib, the emit CLI does what the Lake plugin does for the
forward direction --- recomputes the schema_hash, writes
the handshake, and emits a typed Lean wrapper:

```
$ leo4-rust-emit --cdylib lib<pkg>.so --emit-lean \
                 --lean-module MyApp.Rust \
                 --out-dir lean/.leo4-emit
```

Output files:

- `<pkg>.leo4-rust-exports.idl` --- canonical IDL.
- `<pkg>.leo4-rust-handshake` --- JSON with schema_hash,
  abi_version, exports list.
- `<pkg>.leo4-rust-imports.lean` --- typed Lean wrappers.

== Worker harness (`crates/leo4-rust-worker`)

A small Rust binary that `dlopen`s the cdylib, recomputes
the schema_hash, sends a 25-byte handshake, then runs a
serial request loop. `LEO4_RUST_HANDSHAKE_PKG` /
`_IFACE` env vars must match what `leo4-rust-emit` used or
the recomputed hash drifts and the wrapper rejects.

== Dispatcher (`shim/leo4_rust_bridge.c`)

A single C TU. C17 baseline, opportunistic C23 upgrade.
`leo4_worker_ops_t` abstracts spawn / kill / reap / send /
recv; backends are stub, POSIX (`posix_spawn` +
`socketpair`), Windows-gnullvm (`CreateProcessA` + named
pipe). `leo4_consume_handshake` *MUST* run immediately
after spawn, before any request frame goes out.
`leo4_dispatch_isolated` is the per-call fresh-worker path
that the `iso:` prefix on a mangled name triggers.

== Lean-side glue shim (`shim/leo4_rust_bridge_lean.c`)

The *only* leo4 C file that includes `<lean/lean.h>`.
Returns an `IO ByteArray` whose first 4 bytes carry a LE
`u32` status and whose remaining bytes carry the
canonical-ABI payload --- avoids the Prod inline-scalar
ABI mismatch that `UInt32 × ByteArray` would create.

== Lake `extern_lib` integration (`lake/Leo4Rust/`)

A separate Lake package that exposes two `extern_lib`s:
`leo4RustBridge` (cargo-built `libleo4_rust_bridge.a`) and
`leo4RustBridgeLean` (leanc-compiled glue shim, ar-wrapped).
User lakefiles `require Leo4Rust` and Lake's `lean_exe`
link step picks both archives up automatically.

== Sanity check

`examples/05-rust-export` exercises every path. After
`just rust-export-05-build` (or `leo4 run` from inside
`examples/05-rust-export/`), the executable prints
prime-related values. If you see "garbage" status values
the dispatcher missed the handshake consume.

== Phase 10 follow-ups

The Phase 9 surface has been smoothed by the Phase 10
substeps that already landed (2026-05-21): `leo4 run` CLI
(D1), `lake run Leo4Rust/regenerate` script (D2),
function-arrow IDL type (B1; runtime in B1.x), reserved
`LeanError` codes 0x02--0x08 with real triggers (F1),
`LEO4_RUST_WORKER_RECYCLE_SECONDS` + restart-flag side-
channel (A4 / A5), variant payload widening (B5).

== Closing

You now have an end-to-end leo4 implementation, including
the reverse direction. The next steps are stretch goals:
WIT lowering refinements, additional Mathlib bridges, the
`wasm32-wasip3` native target when it stabilises, and the
schema-idl `ConstraintExpr<Atom>` typed AST when a consumer
needs it.

The complete reference implementation is at
`github.com/Honey-Be/leo4`. Compare your build against it as
you go; commit messages there name each step and explain why
the design landed where it did.

Happy hacking.
