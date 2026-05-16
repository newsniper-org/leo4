# leo4 Mangling Specification

> Normative. Both the Rust side (`leo4-idl`) and the Lean side
> (`Leo4Plugin/Mangling.lean`) implement this independently. The
> cross-impl conformance test in `tests/mangling/` MUST pass.

## 1. Overall Shape

```
mangle(f, [T₁, …, Tₙ]) =
    "leo4__"
  ++ pkg
  ++ "__"
  ++ iface
  ++ "__"
  ++ fname
  ++ "__"
  ++ join("_", map(mangle_type, [T₁, …, Tₙ]))
  ++ "__h"
  ++ schema_hash_prefix
```

### Components

- `pkg`: the IDL `package` declaration's name. Colons in the package path
  are replaced with `_` (e.g., `my:lean` → `my_lean`).
- `iface`: the `interface` declaration's name.
- `fname`: the function name as written in IDL.
- `schema_hash_prefix`: lowercase base32 encoding of the first 8 bytes of
  the BLAKE3 hash of the normalized IDL form (see §3).

The `__` separator is two underscores everywhere. Single underscores within
type encodings are part of the type encoding and do not collide because
type encodings never start or end with an unbalanced separator.

## 2. Type Encoding

```
mangle_type(u8)              = "u8"
mangle_type(u16)             = "u16"
mangle_type(u32)             = "u32"
mangle_type(u64)             = "u64"
mangle_type(i8)              = "i8"
mangle_type(i16)             = "i16"
mangle_type(i32)             = "i32"
mangle_type(i64)             = "i64"
mangle_type(f32)             = "f32"
mangle_type(f64)             = "f64"
mangle_type(bool)            = "b"
mangle_type(char)            = "c"
mangle_type(string)          = "str"
mangle_type(bigint)          = "bI"
mangle_type(bignat)          = "bN"

mangle_type(list<T>)         = "L_" ++ mangle_type(T) ++ "_l"
mangle_type(option<T>)       = "O_" ++ mangle_type(T) ++ "_o"
mangle_type(result<T, E>)    = "Rz_" ++ mangle_type(T) ++ "_" ++ mangle_type(E) ++ "_z"
mangle_type(result<T>)       = "Rz_" ++ mangle_type(T) ++ "__z"
mangle_type(tuple<T₁,…,Tₙ>)  = "T_" ++ join("_", map(mangle_type, [T₁,…,Tₙ])) ++ "_t"

mangle_type(record R)        = "S_" ++ R ++ "_s"
mangle_type(record R<T₁,…>)  = "S_" ++ R ++ "_" ++ join("_", map(mangle_type, [T₁,…])) ++ "_s"
mangle_type(variant V)       = "V_" ++ V ++ "_v"
mangle_type(variant V<T₁,…>) = "V_" ++ V ++ "_" ++ join("_", map(mangle_type, [T₁,…])) ++ "_v"
mangle_type(enum E)          = "E_" ++ E ++ "_e"
mangle_type(flags F)         = "F_" ++ F ++ "_f"
mangle_type(resource R)      = "X_" ++ R ++ "_x"

mangle_type(io<T>)           = "I_" ++ mangle_type(T) ++ "_i"
```

### Invariants

1. Every encoding has a balanced open/close marker. `L_…_l`, `S_…_s`, etc.
   This makes mangled names unambiguously parseable back to an IDL type.
2. Encodings are deterministic. Field/case order follows IDL declaration order
   AFTER normalization (see §3).
3. Encoding length is bounded by IDL size. No "shortened" or "smart" forms.

## 3. Normalized IDL Form

The schema hash is BLAKE3 over the *normalized* form of the IDL file.
Normalization steps, applied in order:

1. **Strip comments**: remove `//` to end of line and `/* … */`.
2. **Strip doc strings**: remove `///`-style doc comments and the body of any
   `@doc(...)` attribute.
3. **Whitespace canonicalize**: collapse all whitespace (including newlines)
   to single ASCII spaces. Remove trailing/leading whitespace on each token.
4. **Sort `use` decls**: lexicographic by path.
5. **Sort `interface` decls**: lexicographic by name.
6. **Sort `interface` members**: type decls first (lex by name), then resources
   (lex by name), then functions (lex by name).
7. **Inline `type` aliases**: replace every reference to an aliased type with
   its definition. Aliases that resolve to themselves are forbidden.
8. **Inline `constraint` aliases**: same.
9. **Re-emit as UTF-8**: standard ASCII representation, no BOM.

The normalized stream is the input to BLAKE3.

```
schema_hash_prefix = base32lc( blake3(normalized_idl_bytes)[0..8] )
```

`base32lc` is RFC 4648 base32 with the lowercase alphabet `abcdefghijklmnopqrstuvwxyz234567`,
no padding. 8 bytes → 13 base32 characters.

## 4. Worked Examples

### Example 1 — scalar generic

IDL fragment:
```
package my:analytics;
interface stats {
    func bucketize<T: scalar>(xs: list<T>, bs: list<T>) -> list<u32>;
}
```

After normalization (whitespace collapsed, members sorted):
```
package my:analytics; interface stats { func bucketize<T: scalar>(xs: list<T>, bs: list<T>) -> list<u32>; }
```

Let `schema_hash_prefix = "k3pq9r2htgmxb"` (hypothetical).

Mangled names for the admit-set:
```
leo4__my_analytics__stats__bucketize__u8__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__u16__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__u32__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__u64__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__i8__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__i16__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__i32__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__i64__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__f32__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__f64__hk3pq9r2htgmxb
```

### Example 2 — non-generic with list

IDL fragment:
```
package my:data;
interface util {
    func sum(xs: list<u64>) -> u64;
}
```

Mangled:
```
leo4__my_data__util__sum__L_u64_l__h<hash>
```

Note that `sum`'s parameter list has one parameter, so the type list
emitted into the symbol is `[list<u64>]`, producing the `L_u64_l` chunk.

### Example 3 — record-returning function

```
package my:geom;
interface points {
    record Point { x: f64, y: f64 }
    func midpoint(a: Point, b: Point) -> Point;
}
```

Mangled:
```
leo4__my_geom__points__midpoint__S_Point_s_S_Point_s__h<hash>
```

## 5. Cross-Implementation Conformance

The test in `tests/mangling/` provides:

- `tests/mangling/cases/*.idl` — input IDL files.
- `tests/mangling/expected/*.txt` — expected mangled name table.
- `tests/mangling/run.sh` — invokes both `leo4c mangle` (Rust) and
  `lake build mangling-test` (Lean), diff-compares outputs.

Adding a new mangling rule REQUIRES adding a case to this test.

## 6. Reserved Symbols

The following symbols are reserved by leo4 for runtime internals; user IDL
must not produce mangled names that collide:

- `leo4__rt__*` — runtime API
- `leo4__shim__*` — shim helpers
- `leo4__panic_handler` — panic catcher

Collision is impossible by construction (user IDL must have a `pkg` segment),
but the linker will fail loudly if it ever happens, which is the desired
behavior.
