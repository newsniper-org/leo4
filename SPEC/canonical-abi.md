# leo4 Canonical ABI Specification

> Normative. The byte-level wire format for every type that crosses the
> Rust ↔ Lean boundary. Based on the WIT Canonical ABI, with leo4-specific
> extensions for `bigint`, `bignat`, `io<T>`, and 64-bit resource handles.

## 0. Conventions

- Endianness: **little-endian** everywhere.
- Alignment: natural alignment per primitive; padding bytes are zero on
  write, ignored on read.
- All sizes are in bytes.
- Encoding is "flat" — no length prefixes for fixed-size types, explicit
  lengths for variable-size types.

## 1. Primitive Types

| Type | Size | Encoding |
|---|---|---|
| `u8`  | 1 | one byte |
| `u16` | 2 | two LE bytes |
| `u32` | 4 | four LE bytes |
| `u64` | 8 | eight LE bytes |
| `i8`…`i64` | same as `uN` | two's complement |
| `f32` | 4 | IEEE 754 LE |
| `f64` | 8 | IEEE 754 LE |
| `bool` | 1 | `0x00` for false, `0x01` for true; anything else is a decode error |
| `char` | 4 | Unicode code point, LE u32 |
| `string` | — | see §3 |
| `bigint`, `bignat` | — | see §2 |

## 2. Arbitrary-Precision Integers

`bignat` (unsigned):
```
+--------+--------+- ... -+
| len:u32| limbs (LE u64 each) |
+--------+--------+- ... -+
```
- `len` is the number of `u64` limbs.
- Limbs are little-endian, least significant first.
- The most significant limb is non-zero, unless `len == 0` (meaning the value
  is zero).

`bigint` (signed):
```
+--------+--------+--------+- ... -+
| sign:u8| len:u32| limbs (LE u64 each) |
+--------+--------+--------+- ... -+
```
- `sign` is `0x00` for non-negative, `0x01` for negative.
- For value `0`, `sign` MUST be `0x00` and `len` MUST be `0`.
- Magnitude is encoded like `bignat`.

Decoders MUST reject malformed encodings (leading zero limb when `len > 0`,
sign byte other than `0x00`/`0x01`, etc.).

## 3. Strings

```
+--------+--------+- ... -+
| len:u32| utf8 bytes      |
+--------+--------+- ... -+
```
- `len` is the byte length, not the character count.
- Bytes MUST be valid UTF-8. Invalid UTF-8 is a decode error.

## 4. Lists

`list<T>` for any `T`:
```
+--------+--------+- ... -+
| len:u32| elements (per T encoding) |
+--------+--------+- ... -+
```
- Elements are concatenated with no padding between them.
- For fixed-size `T`, total payload size is `len * sizeof(T)`.

## 5. Option

`option<T>`:
```
discriminator:u8
  | 0x00 (none) -> no payload
  | 0x01 (some) -> payload (per T encoding)
```

## 6. Result

`result<T, E>`:
```
discriminator:u8
  | 0x00 (ok)  -> payload (per T encoding)
  | 0x01 (err) -> payload (per E encoding)
```

`result<T>` (no error type) and `result<>` (neither) follow the same shape
with omitted payloads.

## 7. Tuples

`tuple<T₁, …, Tₙ>`:
- Sequential concatenation of each element's encoding, in declaration order.
- No padding between elements (unlike WIT's "flattened" mode for register
  passing; leo4 uses memory representation throughout).

## 8. Records

`record R { f₁: T₁, …, fₙ: Tₙ }`:
- Fields in **declaration order**. Note: IDL normalization
  (`SPEC/mangling.md` §3) sorts `interface` members, but it does NOT sort
  record/variant fields — those keep their author-given order so that
  encode/decode is stable across source rearrangements that should be
  semantically inert (e.g. doc-comment edits).
- Each field encoded per its type.
- No padding between fields.

### 8.1 Self-recursive records and variants

If a field's type is `Self` (see `SPEC/idl-grammar.ebnf`), encode/decode
recurses into the same record/variant definition. Encoders MUST tolerate
arbitrary recursion depth; decoders MAY enforce a depth cap configured
through `leo4.toml` (`max_decode_depth`, default 256) and return error
code `0x0000_0008` (decode-depth-exceeded) on overflow. `leo4-abi`
exposes the default cap as `MAX_DECODE_DEPTH` together with a
`check_decode_depth(depth) -> Result<usize, LeanError>` helper that
self-recursive decoders MUST call on every recursive site (Phase
10-F1).

`Self` carries no type information of its own on the wire — its layout
is identical to the enclosing record/variant's. Hence `Self` does **not**
appear in the mangled symbol name (it would create infinite expansion);
the schema hash, however, sees the `Self` token literally in the
normalized IDL form, so any change to a self-recursive type's other
fields still rotates every mangled symbol that references it.

## 9. Variants

`variant V { case₁(T₁), case₂(T₂), … }`:
```
discriminator:u32
payload (per case encoding)
```
- Discriminator is a 32-bit LE index into the case list.
- Cases without payload have no bytes after the discriminator.
- Both encoders **and** decoders in tree (Lake plugin shim emitter +
  Rust `#[derive(LeanMarshal)]`) emit / read 4 bytes for byte-identical
  cross-impl conformance, as of 2026-05-20. A permissive 1-byte
  decoder is still allowed by this SPEC for variants with ≤ 256 cases,
  but no in-tree decoder uses it — every wire payload that needs
  cross-impl validation must use the 4-byte form.

## 10. Enums

`enum E { a, b, c }`:
- Encoded as `u32` LE of the case index.

## 11. Flags

`flags F { x, y, z }`:
- Encoded as `u8` if ≤ 8 flags, `u16` if ≤ 16, `u32` if ≤ 32, `u64` if ≤ 64.
- Currently leo4 does not support > 64 flags. Use a `list<bool>` or a
  separate record.

## 12. Resources

`resource R`:
```
handle:u64
```
- `handle` is opaque to the Rust side.
- On native: pointer reinterpreted as `u64`. Zero-extend on 32-bit systems
  (unsupported but specified for completeness).
- On wasm: Component Model resource handle, already a `u64`.
- `handle == 0` is the **reserved null sentinel** (the
  `INVALID_RESOURCE_HANDLE` constant in `leo4-abi`). Encoders MUST
  reject this value with `LEO4_ERR_ENCODE_ERROR` (`0x02`); decoders
  MUST reject it with `LEO4_ERR_INVALID_HANDLE` (`0x03`). The
  reservation gives `leo4-abi` a deterministic trigger path for both
  codes (Phase 10-F1, 2026-05-21).

### Ownership semantics

- A `resource` parameter without modifier is **borrowed**: the callee may
  not retain the handle past the call.
- An `own<R>` modifier (TODO: add to IDL grammar) transfers ownership; the
  callee is responsible for `dec_rc` (or equivalent) eventually.
- Functions returning a `resource` always transfer ownership.

### Reference counting

leo4 uses Lean's native rc for native backend resources. Each `LeanRef<'a, T>`
on the Rust side corresponds to one rc increment. `Drop` decrements.

## 13. `io<T>` / `future<T>`

`io<T>` is recognised in boundary positions and renders in the
canonical IDL as `future<T>` (Phase 7 lift, 2026-05-20). At the wire
layer the value is encoded as bare `T` — the future-ness is a
function-level effect (D-i 2026-05-19), not a wrapper type — and the
shim unwraps Lean's `lean_io_result` before encoding (see
`LEO4_ERR_IO_FAILED` below).

Sync exports continue to land unchanged; only the `IO α` arm flows
through the effect lift.

The `error` type:
```
record error {
    code: u32,
    message: string,
    backtrace: option<string>,
}
```

Error codes (reserved range `0x0000_0000`..`0xFFFF_FFFF`):

| Range | Owner |
|---|---|
| `0x0000_0000`..`0x0000_FFFF` | leo4 runtime |
| `0x0001_0000`..`0x0001_FFFF` | Lean panic / IO failure passthrough |
| `0x0002_0000`..`0x0002_FFFF` | Rust worker passthrough (reverse direction; see `SPEC/reverse-direction.md` §10) |
| `0x0003_0000`..`0x000F_FFFF` | Reserved |
| `0x0010_0000`..`0xFFFF_FFFF` | User-defined |

leo4 runtime error codes:

| Code | Meaning |
|---|---|
| `0x0000_0001` | Decode error (malformed wire format) |
| `0x0000_0002` | Encode error (value out of range) |
| `0x0000_0003` | Resource handle invalid |
| `0x0000_0004` | Out of memory |
| `0x0000_0005` | Schema handshake mismatch |
| `0x0000_0006` | Unknown function |
| `0x0000_0007` | Return buffer too small (caller retries with larger buffer) |
| `0x0000_0008` | Decode-depth exceeded on a `Self`-recursive type |
| `0x0000_0064` | Shim instantiation not yet implemented (placeholder returned when an export uses a wire shape the shim emitter doesn't cover yet) |

Lean-passthrough error codes (range `0x0001_0000`..`0x0001_FFFF`):

| Code | Meaning |
|---|---|
| `0x0001_0001` | `LEO4_ERR_IO_FAILED` — an `IO α` boundary export returned an error result. The shim unwraps `lean_io_result` and surfaces this code instead of the inner value (Phase 7 step 2b, 2026-05-21). |

Rust-worker passthrough error codes (range `0x0002_0000`..`0x0002_FFFF`,
reverse direction; full table in `SPEC/reverse-direction.md` §10):

| Code | Meaning |
|---|---|
| `0x0002_0001` | `LEO4_ERR_RUST_PANIC` — user Rust function panicked; worker aborted |
| `0x0002_0002` | `LEO4_ERR_RUST_WORKER_RESTARTED` — persistent worker respawned mid-session; persistent state lost |
| `0x0002_0003` | `LEO4_ERR_RUST_SPAWN_FAILED` — dispatcher could not spawn a worker (OS error) |
| `0x0002_0004` | `LEO4_ERR_RUST_CDYLIB_NOT_FOUND` — no resolvable cdylib path |
| `0x0002_0005` | `LEO4_ERR_RUST_DLSYM_FAILED` — handshake passed but `dlsym(mangled)` returned NULL |
| `0x0002_0006` | `LEO4_ERR_RUST_IPC_FAILED` — IPC round-trip failed mid-call |

## 13a. Function-arrow types — `fn(T₁,…,Tₙ) -> R`  (Phase 10-B1)

Wire format:
```
callback_id:u64
```
- `callback_id` is opaque to the receiving side. The producing side
  (the one *passing* a closure across the boundary) is responsible
  for resolving the id back to a concrete callable; the receiving
  side stores it as an opaque token and invokes it via the
  re-entrant dispatch path documented in `SPEC/reverse-direction.md`
  §11 (Lean ↔ Rust closure callbacks).
- `callback_id == 0` is reserved as the null sentinel
  (`INVALID_RESOURCE_HANDLE` shape from §12). Encoders reject with
  `LEO4_ERR_ENCODE_ERROR` (`0x02`); decoders reject with
  `LEO4_ERR_INVALID_HANDLE` (`0x03`).
- IDL surface syntax is `fn(T₁, …, Tₙ) -> R`; mangling is
  `A_<tuple-of-args>_<ret>_a` per `SPEC/mangling.md` §2.
- Lifetime: a `callback_id` is valid only for the duration of the
  enclosing call. The receiving side MUST NOT retain the id past the
  return; doing so produces undefined behaviour. (`own<fn(…) -> R>`
  for transferable closures is reserved but not yet specified.)
- Re-entrant dispatch runtime lands in a Phase 10-B1.x follow-up;
  this section pins the wire shape so cross-impl conformance tests
  (mangling + IDL round-trip) can ship ahead of the runtime.

## 14. Function Call Convention

Every leo4 function, regardless of signature, is callable from Rust via this
C signature:

```c
int32_t leo4_call_<mangled>(
    leo4_arena_t* arena,
    const uint8_t* args_ptr, size_t args_len,
    uint8_t* ret_ptr, size_t ret_cap, size_t* ret_len
);
```

`<mangled>` here is the *body* produced by `SPEC/mangling.md §1`; the
`leo4_call_` prefix is added by the shim (mangling.md §6). The Lean
helper that the shim ultimately calls into uses the same body under
the `leo4_lean__` prefix.

- Return value: `0` for success, non-zero for error (see §13).
- `args_ptr`/`args_len`: encoded argument list (tuple-like concatenation).
- `ret_ptr`/`ret_cap`/`ret_len`: caller-provided buffer; callee writes
  encoded return value and sets `*ret_len`.
- If `*ret_len > ret_cap` on entry, callee MAY write nothing and return
  `0x0000_0007` (buffer-too-small); caller retries with larger buffer.

### Argument tuple encoding

A function with parameters `(p₁: T₁, …, pₙ: Tₙ)` receives an argument
buffer encoded as `tuple<T₁, …, Tₙ>` (§7). A zero-parameter function
receives a zero-length buffer.

### Return encoding

A function returning `T` writes `T` to the return buffer. A function
returning nothing (no `->` clause) writes a zero-length buffer.

## 15. Schema Handshake

Before any function call, the Rust side MUST verify the schema handshake:

```c
extern int32_t leo4_handshake(
    const uint8_t* expected_schema_hash,  // 8 bytes
    uint32_t expected_abi_version,
    char* mismatch_detail_out, size_t detail_cap
);
```

The shim implements `leo4_handshake` to compare against its compiled-in
hash and ABI version. On mismatch, returns `0x0000_0005` and writes a
human-readable explanation.

ABI version is currently `1`.

## 16. Thread Safety

- The Lean runtime is single-threaded by default. A `Lean` token cannot
  be cloned across threads.
- `Arena<'a>` is `!Send` and `!Sync`.
- `LeanRef<'a, T>` is `!Send` and `!Sync`.
- Future versions MAY introduce a `MultiLean` or per-thread runtime.
  This is out of scope for v0.

## 17. Open Questions

- Should we support a "compact" mode that drops `*_len` parameters when
  return size is statically known? Skipped for v0.
- Should `bigint`/`bignat` be ABI-compatible with `num-bigint::BigUint`'s
  representation? Decided in Phase 5 of the roadmap.
