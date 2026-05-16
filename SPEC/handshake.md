# leo4 Handshake File Format

> Normative. The Lake plugin emits these files; the Rust macro layer
> consumes them.

## Files

| File | Producer | Consumer | Purpose |
|---|---|---|---|
| `<pkg>.leo4-schema` | Lake | `leo4c` (human/tool) | Full IDL in canonical form |
| `<pkg>.wit` | Lake | wasmtime / others | WIT lowered form (optional) |
| `<pkg>.leo4-handshake` | Lake | Rust build.rs | Schema hash + ABI version |
| `<pkg>.leo4-mangling` | Lake | Rust macro | Complete mangled-name table |
| `<pkg>.leo4-shim.so` | Lake | Rust runtime (linker) | Native shim binary |

All five files share the same `<pkg>` basename per IDL package.

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
          "args": ["u32"],
          "mangled": "leo4__my_analytics__stats__bucketize__u32__hk3pq9r2htgmxb"
        },
        {
          "args": ["u64"],
          "mangled": "leo4__my_analytics__stats__bucketize__u64__hk3pq9r2htgmxb"
        },
        {
          "args": ["f64"],
          "mangled": "leo4__my_analytics__stats__bucketize__f64__hk3pq9r2htgmxb"
        }
        // … rest of admit-set
      ]
    },
    {
      "logical_name": "stats::sum",
      "generics": [],
      "instantiations": [
        {
          "args": [],
          "mangled": "leo4__my_analytics__stats__sum__h<...>"
        }
      ]
    }
  ]
}
```

### Field meanings

- `logical_name`: `<interface>::<function>`. Lexicographic-sortable.
- `generics`: type-parameter names from the IDL declaration.
- `instantiations`: every concrete monomorphization Lake has emitted.
  Lazy mode pre-emits the full admit-set Cartesian product (subject to
  `max_depth`).
- `args`: the concrete type arguments in IDL form.
- `mangled`: the mangled name per `mangling.md`.

The Rust macro:

1. Reads this file at expansion time.
2. For each `entry` referenced in a `#[leo4::import]` block, emits an
   `extern "C"` declaration per `instantiation` with `#[link_name = mangled]`.
3. Emits a generic Rust wrapper that dispatches on the const tag of the
   type parameter.

If a Rust call site uses a `T` not present in `instantiations`, the macro
emits a compile error pointing at the IDL constraint that needed to be
broadened.

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
