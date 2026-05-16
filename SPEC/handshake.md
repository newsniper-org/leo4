# leo4 Handshake File Format

> Normative. The Lake plugin emits these files; the Rust macro layer
> consumes them.

## Files

| File | Producer | Consumer | Purpose |
|---|---|---|---|
| `<pkg>.leo4-schema` | Lake | `leo4c` (human/tool) | Full IDL in canonical form (see below) |
| `<pkg>.wit` | Lake | wasmtime / others | WIT lowered form (optional) |
| `<pkg>.leo4-handshake` | Lake | Rust build.rs | Schema hash + ABI version |
| `<pkg>.leo4-mangling` | Lake | Rust macro | Complete mangled-name table |
| `<pkg>.leo4-shim.so` | Lake | Rust runtime (linker) | Native shim binary |

All five files share the same `<pkg>` basename per IDL package.

## `<pkg>.leo4-schema`

Plain UTF-8 IDL text in the canonical form defined by `SPEC/mangling.md` §3,
i.e.

1. comments and doc strings stripped,
2. whitespace collapsed to single ASCII spaces (with newlines between
   top-level declarations preserved for human readability — the
   normalisation that feeds the schema hash collapses *all* whitespace,
   but the on-disk file may keep one `\n` between declarations and
   indented members so a human can read it),
3. `use` decls sorted lexicographically by path,
4. `interface` decls sorted lexicographically by name,
5. **type decls before resources before functions** inside each
   interface, lex-sorted within each band,
6. record/variant **fields** kept in declaration order (NOT sorted; see
   `SPEC/canonical-abi.md` §8),
7. `type` and `constraint` aliases inlined.

The file is the authoritative description of the package's IDL. The
schema hash in `<pkg>.leo4-handshake` is the FNV-1a-64 of *exactly the
fully-collapsed form* of this file (whitespace-collapse step applied to
the whole stream), so re-deriving the hash from a `.leo4-schema` on disk
must reproduce the handshake's value byte-for-byte.

## `<pkg>.leo4-handshake`

JSON file. Format:

```json
{
  "version": 1,
  "package": "my:analytics",
  "schema_hash": "k3pq9r2htgmxb",
  "schema_hash_bytes": "abcdef0123456789",
  "abi_version": 1,
  "lean_toolchain": "leanprover/lean4:v4.X.Y",
  "leo4_plugin_version": "0.1.0",
  "emitted_at": "2026-05-16T12:00:00Z",
  "interfaces": [
    {
      "name": "stats",
      "function_count": 3,
      "resource_count": 0
    }
  ],
  "constraint_universe": {
    "scalar": ["u8","u16","u32","u64","i8","i16","i32","i64","f32","f64"],
    "ord": ["u8","u16",…,"f64","string"],
    "marshal": ["…all registered marshalable types…"]
  }
}
```

### Field meanings

- `version`: handshake file format version. Bumped on incompatible changes
  to *this file's* schema. Currently `1`.
- `schema_hash`: the `schema_hash_prefix` from `mangling.md` (13-char base32).
- `schema_hash_bytes`: hex of the 8 bytes (16 hex chars). Used by tools that
  prefer raw bytes over base32.
- `abi_version`: the canonical ABI version. Currently `1`.
- `lean_toolchain`: which Lean toolchain Lake used. Informational.
- `leo4_plugin_version`: Lake plugin version. Informational.
- `emitted_at`: ISO 8601 UTC timestamp. Informational.
- `interfaces`: summary, not authoritative — the IDL is.
- `constraint_universe`: the realized admit-set per constraint, as resolved
  in the current environment. This is what enables lazy mode: every type that
  could appear as a generic parameter is enumerated here.

### Reading from Rust

`leo4-build`'s `build.rs` helper:

```rust
let handshake: Handshake = serde_json::from_reader(
    File::open(handshake_path)?
)?;
println!("cargo:rustc-env=LEO4_SCHEMA_HASH={}", handshake.schema_hash);
println!("cargo:rustc-link-search=native={}", lake_output_dir);
println!("cargo:rustc-link-lib=dylib={}-leo4-shim", pkg_name);
```

The macro layer reads the same file at expansion time to know which
mangled names to declare as `extern "C"` blocks.

## `<pkg>.leo4-mangling`

JSON file. Format:

```json
{
  "version": 1,
  "package": "my:analytics",
  "schema_hash": "k3pq9r2htgmxb",
  "entries": [
    {
      "logical_name": "stats::bucketize",
      "generics": ["T"],
      "instantiations": [
        {
          "generic_args": ["u32"],
          "param_types":  [
            { "encoded": "L_u32_l", "uses_generics": [0] },
            { "encoded": "L_u32_l", "uses_generics": [0] }
          ],
          "mangled": "leo4__my_analytics__stats__bucketize__L_u32_l_L_u32_l__hk3pq9r2htgmxb"
        },
        {
          "generic_args": ["u64"],
          "param_types":  [
            { "encoded": "L_u64_l", "uses_generics": [0] },
            { "encoded": "L_u64_l", "uses_generics": [0] }
          ],
          "mangled": "leo4__my_analytics__stats__bucketize__L_u64_l_L_u64_l__hk3pq9r2htgmxb"
        }
        // … rest of admit-set
      ]
    },
    {
      "logical_name": "stats::sum",
      "generics": [],
      "instantiations": [
        {
          "generic_args": [],
          "param_types": [
            { "encoded": "L_u64_l", "uses_generics": [] }
          ],
          "mangled": "leo4__my_analytics__stats__sum__L_u64_l__hk3pq9r2htgmxb"
        }
      ]
    }
  ]
}
```

### Field meanings

- `logical_name`: `<interface>::<function>`. Lexicographic-sortable.
- `generics`: type-parameter names from the IDL declaration. Empty for
  non-generic functions.
- `instantiations`: every concrete monomorphization Lake has emitted.
  Lazy mode pre-emits the full admit-set Cartesian product (subject to
  `max_depth`).
- `generic_args`: the type arguments used for *this* monomorphization, in
  declaration order, each rendered with `mangle_type` (see `mangling.md` §2).
  Same length as the outer `generics` array. Empty for non-generic functions.
  A position holding JSON `null` indicates a **phantom** generic — one that
  never appears in any parameter type or in the return type, so the plugin
  has not enumerated it. The corresponding `mangled` symbol does not depend
  on this generic's value; consumers should not emit a runtime dispatch on
  it. See LEO4-DESIGN.md §5.
- `param_types`: an entry per value parameter, in declaration order. Each
  entry has:
  - `encoded`: the substituted parameter type rendered with `mangle_type`.
    The `encoded` strings, joined by `_`, form the tokens between the
    function name and `__h<hash>` in `mangled` — i.e. the linker symbol's
    ABI surface.
  - `uses_generics`: indices (ascending, deduplicated) into the enclosing
    entry's `generics` array, identifying which type parameters this slot's
    *template* referenced before substitution. An empty list means the
    parameter is concrete in the function's signature (`encoded` was the
    same string before substitution). A non-empty list means the
    parameter's template was either a generic itself (e.g. `T`) or carried
    a generic inside a composite (e.g. `list<T>`, `option<T>`); the indices
    identify which generics participated.
- `mangled`: the mangled name per `mangling.md`.

The Rust macro:

1. Reads this file at expansion time.
2. For each `entry` referenced in a `#[leo4::import]` block, emits an
   `extern "C"` declaration per `instantiation` with `#[link_name = mangled]`.
3. Emits a generic Rust wrapper that dispatches on the const tag of the
   type parameter; the wrapper picks the right `extern "C"` symbol by
   matching `generic_args` against the call site's `T`.

If a Rust call site uses a `T` not present in any `instantiation`'s
`generic_args`, the macro emits a compile error pointing at the IDL
constraint that needed to be broadened.

## Lifetime and Invalidation

Cargo's `build.rs` MUST emit:

```
cargo:rerun-if-changed=<lake-output-dir>/<pkg>.leo4-handshake
cargo:rerun-if-changed=<lake-output-dir>/<pkg>.leo4-mangling
```

When either file changes, Cargo rebuilds. When the schema hash changes, the
mangled names change, so stale object files fail at link time — this is the
intended behavior, a backstop against silent ABI breaks.

## Atomic Emission

Lake's plugin MUST emit all five files atomically per package:

1. Write all five to temp paths under `<lake-output-dir>/.tmp/`.
2. Rename them into place in this order:
   - `<pkg>.leo4-shim.so` first (largest, slowest)
   - `<pkg>.leo4-mangling` second
   - `<pkg>.leo4-handshake` LAST (Cargo watches this one)

If any step fails, the partial output is left in `.tmp/` and the existing
files in `<lake-output-dir>/` are untouched. This prevents Cargo from
picking up an inconsistent snapshot.

## Forward Compatibility

Future versions of this file format MUST bump `version` on incompatible
changes. The Rust `build.rs` helper supports reading version 1 only.
Newer versions trigger a build error pointing at the version mismatch.

Adding *new* fields to the JSON is a compatible change (consumers ignore
unknown fields).
