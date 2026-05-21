# leo4 Mangling Specification

> Normative. Both the Rust side (`leo4-idl`) and the Lean side
> (`Leo4Plugin/Mangling.lean`) implement this independently. The
> cross-impl conformance test in `tests/mangling/` MUST pass.

## 1. Overall Shape

```
mangle(f, [P₁, …, Pₘ]) =
    "leo4__"
  ++ pkg
  ++ "__"
  ++ iface
  ++ "__"
  ++ fname
  ++ "__"
  ++ join("_", map(mangle_type, [P₁, …, Pₘ]))
  ++ "__h"
  ++ schema_hash_prefix
```

### Components

- `pkg`: the IDL `package` declaration's name. Colons **and dashes** in
  the package path are replaced with `_` (e.g., `my:lean` → `my_lean`;
  `leo4-sample` → `leo4_sample`). See `fqn` below — both segments use
  the same dash/dot/colon → underscore normalisation.
- `iface`: the `interface` declaration's name.
- `fname`: the function name as written in IDL.
- `[P₁, …, Pₘ]`: the function's **parameter types**, in declaration order,
  *after generic substitution*. For a non-generic function these are the
  literal parameter types from the IDL; for a generic function `f<T₁,…,Tₙ>`
  invoked at concrete type arguments `(A₁,…,Aₙ)`, every occurrence of `Tᵢ`
  inside the parameter list is replaced by `Aᵢ` and the result is `[P₁,…,Pₘ]`.
  The generic vector `[A₁,…,Aₙ]` itself is **not** part of the mangled name —
  it is carried separately in `<pkg>.leo4-mangling` (see `handshake.md`),
  because the linker only cares about ABI surface, and the ABI surface is
  the parameter list.
- `schema_hash_prefix`: lowercase base32 encoding of the 8 hash bytes
  produced by `fnv1a64` over the normalized IDL form (see §3).

The `__` separator is two underscores everywhere. Single underscores within
type encodings are part of the type encoding and do not collide because
type encodings never start or end with an unbalanced separator.

For a function with **zero parameters** the type list is empty and the
mangled name contains the four consecutive underscores `__` `__h` literally,
e.g. `leo4__pkg__iface__hello____h<hash>`.

### The mangled name is a *body*, not an exported symbol

The string produced by `mangle(f, …)` is the **body** that both ABI
surfaces (shim entry point + Lean-side helper) wrap around. The actual
linker symbols are:

| Exported symbol            | Owner          | ABI                                     |
|----------------------------|----------------|------------------------------------------|
| `leo4_call_<mangled>`      | C shim         | canonical-buffer ABI (canonical-abi.md §14) |
| `leo4_lean__<mangled>`     | Lean wrapper   | Lean native ABI (see §6)                  |

Both prefixes are reserved (§7); user IDL cannot collide. The shared
`<mangled>` body links the two so the shim can resolve its Lean helper
deterministically. Conformance tests compare *bodies*, not exported
symbols (the `<pkg>.leo4-mangling` file carries one `mangled` field per
instantiation, holding the body; downstream consumers re-add whichever
prefix they need).

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

mangle_type(record R)        = "S_" ++ fqn(R) ++ "_s"
mangle_type(record R<T₁,…>)  = "S_" ++ fqn(R) ++ "_" ++ join("_", map(mangle_type, [T₁,…])) ++ "_s"
mangle_type(variant V)       = "V_" ++ fqn(V) ++ "_v"
mangle_type(variant V<T₁,…>) = "V_" ++ fqn(V) ++ "_" ++ join("_", map(mangle_type, [T₁,…])) ++ "_v"
mangle_type(enum E)          = "E_" ++ fqn(E) ++ "_e"
mangle_type(flags F)         = "F_" ++ fqn(F) ++ "_f"
mangle_type(resource R)      = "X_" ++ fqn(R) ++ "_x"
mangle_type(resource R<T₁,…>)= "X_" ++ fqn(R) ++ "_" ++ join("_", map(mangle_type, [T₁,…])) ++ "_x"

mangle_type(io<T>)           = "I_" ++ mangle_type(T) ++ "_i"
-- Wire mangle for `io<T>` is unchanged for cross-impl conformance.
-- The canonical IDL renders the same shape as `future<T>` (Phase 7
-- lift), but mangling stays on the `I_…_i` form so the linker
-- symbol table doesn't rotate just because the source spelling
-- changed.

mangle_type(Self)            = "self"   -- only inside a record/variant/resource
                                        -- body; never expands recursively.

mangle_type(Cyc<i>)          = "c" ++ toString i ++ "c"
                              -- only inside a `mutual { … }` group's
                              -- members; the index is 0-based in
                              -- declaration order. See
                              -- SPEC/phase-6-mutual.md §2.
```

### Fully-qualified names

```
fqn(name) = name.replace(".", "_").replace("-", "_")
```

`fqn` is applied to the IDL-side FQN of a nominal type — i.e. its
Lean-side module path joined by `.` and then translated to `_` for
linker friendliness. Both dots and dashes collapse to underscores; the
dash case lets kebab-case names (e.g. `leo4-sample`) survive into the
mangled symbol as valid C / Lean identifiers (Lean's `@[export ident]`
attribute parses an unquoted identifier and rejects dashes). Example:
a record declared in Lean as `Sample.Geom.Point` mangles as
`S_Sample_Geom_Point_s`; a package named `leo4-sample` mangles as
`leo4_sample`.

### Generic records / variants / resources

The mangling of a generic record `R<T₁,…,Tₙ>` includes the substituted
type arguments at the record level (one mangle_type chunk per
argument). Field types are **not** independently mangled into the
record's name — the record-level mangling is sufficient to disambiguate.

If a field of a generic record references one of `R`'s own type
parameters (e.g. `record Pair<α, β> { fst: α, snd: β }`), the field
itself contributes nothing to `R`'s mangling beyond what `[T₁,…,Tₙ]`
already supplies.

### `Self` and `Self<…>`

`Self` mangles to the literal string `self`. It is permitted only as a
type leaf inside the immediately enclosing record, variant, or
resource declaration. The plugin does **not** expand `Self` recursively
into its parent's mangled form — that would loop on self-referential
types like `record Tree { left: Self, right: Self }`. The canonical
ABI handles `Self` by recursive traversal at encode/decode time
(SPEC/canonical-abi.md §8.1).

`Self<T₁,…,Tₙ>` mangles as `self_<mangle_type(T₁)>_<…>_<mangle_type(Tₙ)>_x`,
i.e. like a generic application whose head is the marker `self`. The
recursive reference still does not loop because the head mangles to a
constant `self` token regardless of the enclosing's name; only the
substituted arguments contribute distinct tokens. Bare `Self` is the
identity-substitution sugar and mangles as `self`.

### Higher-kinded type parameters

When a generic parameter has kind `Type -> Type` (or higher arity), its
*type-level uses* in the function signature appear as applications, e.g.
`F<T>`. Such applications mangle exactly as `mangle_type` would on a
named type: the head `F` is itself a generic parameter and is replaced
by its concrete instantiation drawn from the admit-set (a 1-arity record /
variant / resource, or a builtin like `list` / `option`). The result is
then mangled per `mangle_type`. There is no separate `mangle_type(F<T>)`
clause because the substitution erases the HK parameter before mangling.

Example: `func map<F : Type -> Type, A, B>(x: F<A>, f: A -> B) -> F<B>;`
with the instantiation `F = list, A = u32, B = u64` substitutes to
`map(x: list<u32>, f: u32 -> u64) -> list<u64>`; the parameter type
list `[list<u32>, u32 -> u64]` mangles as
`L_u32_l_<arrow-mangling-TBD>`. Function-arrow mangling for callback
parameters is unspecified — the function-pointer ABI is not on the
phase ladder, and no callback-bearing export exists yet. The slot
exists in case a downstream caller needs it.

## 4. Kind discipline

EBNF describes only the *syntactic* shape of the IDL. The kind system
imposes semantic constraints that EBNF cannot express; the plugin
checks them after parsing and rejects ill-kinded declarations with a
diagnostic.

### Judgment

`Γ ⊢ τ :: κ` reads as "in kind environment `Γ`, the IDL type expression
`τ` has kind `κ`."

```
Γ ⊢ u8 :: Type          Γ ⊢ string :: Type          Γ ⊢ bool :: Type
Γ ⊢ bigint :: Type      Γ ⊢ bignat :: Type          Γ ⊢ char :: Type
              ⋯ (every primitive has kind Type) ⋯

Γ ⊢ list :: Type -> Type        Γ ⊢ option :: Type -> Type
Γ ⊢ result :: Type -> Type -> Type    Γ ⊢ tuple :: Type -> ... -> Type
                                       -- tuple is variadic; arity fixed at use site.

For a named declaration:
  record R<X₁ : κ₁, …, Xₙ : κₙ> { … }   ⟹   R :: κ₁ -> … -> κₙ -> Type
  (variants and resources analogous)

For a generic parameter binder:
  X : κ          ⟹   Γ, X :: κ ⊢ X :: κ

Application:
  Γ ⊢ f :: κ₁ -> κ₂      Γ ⊢ a :: κ₁
  ──────────────────────────────────
        Γ ⊢ f<a> :: κ₂

Self:
  Inside `record R<X₁ : κ₁, …, Xₙ : κₙ> { … }` the binding
  Self :: κ₁ -> … -> κₙ -> Type   is in scope.
  Bare `Self` (no arguments) is sugar for `Self<X₁, …, Xₙ>`.
```

### Mandatory checks

For every parsed declaration the plugin verifies:

1. Each generic parameter binder is classified as **type_param** or
   **value_param** (SPEC/idl-grammar.ebnf):
   - **type_param**: the `:` annotation, if present, is a kind built from
     `Type` and `->`. Absent annotation defaults to `Type`. Anything else
     is a *kind* error.
   - **value_param**: the `:` annotation is a `type`. The annotation
     itself must be well-kinded `:: Type` (no value-of-HKT, no
     value-of-value).
2. Every type-level use is well-kinded: every application `f<a₁,…,aₙ>`
   has `f :: κ₁ -> … -> κₙ -> κ` for matching `aᵢ :: κᵢ`, and the result
   kind `κ` is consistent with the position (e.g. function parameter
   types must be `Type`, not `Type -> Type`).
3. `Self<…>` arity matches the enclosing declaration's generic_params
   arity exactly.
4. **Dependent codomain rejection**: a function's return type may not
   *syntactically* mention any of the function's value_params — value
   dependence on the return side is forbidden at the boundary
   (LEO4-DESIGN.md §4.3). It is fine for *parameter* types to mention
   value_params (e.g. `Vec n α`), because those types lower to ordinary
   length-prefixed forms (`list<α>`); the value is implicit in the
   wire encoding.
5. **Higher-kind constraint requirement**: any type_param whose kind
   is higher than `Type` (i.e. `Type -> Type`, `Type -> Type -> Type`,
   etc.) MUST carry an explicit constraint that pins its admit-set to
   a closed `oneof` of named type constructors. A bare HK
   type_param with no constraint is rejected with a diagnostic
   pointing at the binder.

   *Rationale*: the unconstrained admit-set for a 1-arity type
   constructor is "every 1-arity inductive in the user package", which
   blows up the cartesian product without describing anything the wire
   contract actually needs. The boundary cross is monomorphic; users
   who genuinely need HK at the boundary express the closed set
   directly, e.g. `@[leo4_specialize_when F : oneof {List, Option}]`.
   General HKT closed-world enumeration is therefore *not* on the
   phase ladder (ROADMAP.md "Future").

Ill-kinded declarations never reach the admit-set enumerator.

### Value-param erasure

A `value_param` carries no contribution to:
- the mangled name (no `__<n>__` token),
- the schema-hash input (the `value_param`'s **name** appears in the
  normalized IDL form, so renaming `n` to `len` rotates the hash, but no
  *value* of `n` is ever folded in),
- the function's wire ABI (the parameter is not transmitted; its value
  is recovered, when needed, from a length prefix elsewhere in the
  encoding).

In effect, value generics are a Lean-side ergonomics feature; from the
boundary's perspective the function behaves exactly as if all
`value_params` had been deleted and the dependent types in the
parameter list had been replaced by their erased counterparts (`Vec n α`
→ `list<α>`, `Fin n` → `u32`, etc.). The IDL emitter performs this
erasure when writing `<pkg>.leo4-schema`.

### Invariants

1. Every encoding has a balanced open/close marker. `L_…_l`, `S_…_s`, etc.
   This makes mangled names unambiguously parseable back to an IDL type.
2. Encodings are deterministic. Field/case order follows IDL declaration order
   AFTER normalization (see §3).
3. Encoding length is bounded by IDL size. No "shortened" or "smart" forms.

## 3. Normalized IDL Form and Hash

The schema hash is `fnv1a64` over the *normalized* form of the IDL file.
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

### Hash construction

`fnv1a64` is the standard FNV-1a 64-bit hash:

```
fnv1a64(bytes) :
    h := 0xcbf29ce484222325
    for b in bytes :
        h := (h XOR (b as u64)) * 0x100000001b3   -- u64 wraparound
    return h
```

The 8 hash bytes are taken from `h` *big-endian* (MSB first), then
base32-encoded with the lowercase RFC 4648 alphabet
`abcdefghijklmnopqrstuvwxyz234567`, no padding. 8 bytes pack into 13
base32 characters; the 13th character carries only 4 bits of payload
(low bit zero).

```
schema_hash_prefix = base32lc( be_bytes( fnv1a64(normalized_idl_bytes) ) )
```

**Why FNV-1a, not a cryptographic hash**: the digest is a *change detector*
for ABI invalidation at link time, not a security primitive. Cargo's
`cargo:rerun-if-changed=` plus the linker between them invalidate every
stale object when the digest changes; that is the whole job. Both the Lean
plugin (`Leo4Plugin.Mangling`) and the Rust IDL crate (`leo4-idl`) MUST
implement this byte-for-byte identically — `tests/mangling/` (Phase 3+)
pins that contract.

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

For the instantiation `T = u8`, the substituted parameter list is
`(xs: list<u8>, bs: list<u8>)`, so `[P₁, P₂] = [list<u8>, list<u8>]`,
mangled as `L_u8_l_L_u8_l`. Mangled names for the admit-set:
```
leo4__my_analytics__stats__bucketize__L_u8_l_L_u8_l__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__L_u16_l_L_u16_l__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__L_u32_l_L_u32_l__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__L_u64_l_L_u64_l__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__L_i8_l_L_i8_l__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__L_i16_l_L_i16_l__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__L_i32_l_L_i32_l__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__L_i64_l_L_i64_l__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__L_f32_l_L_f32_l__hk3pq9r2htgmxb
leo4__my_analytics__stats__bucketize__L_f64_l_L_f64_l__hk3pq9r2htgmxb
```
The corresponding `<pkg>.leo4-mangling` rows carry both the generic
argument vector (e.g. `"generic_args": ["u8"]`) and the substituted
parameter list with per-slot origin info (e.g. `"param_types": [
{ "encoded": "L_u8_l", "uses_generics": [0] },
{ "encoded": "L_u8_l", "uses_generics": [0] }
]`); see `handshake.md`.

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

### Example 3 — record-returning function (FQN)

```
package my:geom;
interface points {
    record My.Geom.Point { x: f64, y: f64 }
    func midpoint(a: My.Geom.Point, b: My.Geom.Point) -> My.Geom.Point;
}
```

`fqn("My.Geom.Point") = "My_Geom_Point"`, so mangled:
```
leo4__my_geom__points__midpoint__S_My_Geom_Point_s_S_My_Geom_Point_s__h<hash>
```

### Example 4 — self-recursive variant

```
package my:syntax;
interface ast {
    variant My.Ast.Tree { leaf, node(Self, Self) }
    func depth(t: My.Ast.Tree) -> u32;
}
```

`Self` mangles to `self` and never expands recursively. The variant
itself mangles via its FQN. So:
```
leo4__my_syntax__ast__depth__V_My_Ast_Tree_v__h<hash>
```

The mangled name of `Tree` itself, were it ever used as a `mangle_type`
target, is `V_My_Ast_Tree_v`. The `Self` token only appears inside the
declaration's body and contributes to the schema hash (via the
normalized IDL form) but not to any function's mangled symbol.

### Example 5 — generic record

```
package my:gen;
interface kv {
    record My.Kv.Pair<α, β> { fst: α, snd: β }
    func swap<A, B>(p: My.Kv.Pair<A, B>) -> My.Kv.Pair<B, A>;
}
```

For the instantiation `A = u32, B = string`:
```
P₁ = My.Kv.Pair<u32, string>  →  S_My_Kv_Pair_u32_str_s
```
So:
```
leo4__my_gen__kv__swap__S_My_Kv_Pair_u32_str_s__h<hash>
```

Note that the fields `fst` and `snd` of `Pair` do **not** contribute
their own mangled tokens — the record's type-argument list `[u32, str]`
already disambiguates the instantiation at the record level.

## 5. Cross-Implementation Conformance

The test in `tests/mangling/` provides:

- `tests/mangling/cases/*.idl` — input IDL files.
- `tests/mangling/expected/*.txt` — expected mangled name table.
- `tests/mangling/run.sh` — invokes both `leo4c mangle` (Rust) and
  `lake build mangling-test` (Lean), diff-compares outputs.

Adding a new mangling rule REQUIRES adding a case to this test.

## 6. Lean-side Native ABI Helper Names

The shim entry point and the Lean-side helper both wrap the §1
mangled body, under their respective prefixes. Concretely, every
`<mangled>` body has **two** companion symbols in `<pkg>.leo4-shim.so`:

| Symbol                              | Owner          | ABI                                          | Linkage  |
|-------------------------------------|----------------|-----------------------------------------------|----------|
| `leo4_call_<mangled>`               | C shim         | canonical-buffer ABI (canonical-abi.md §14)   | external |
| `leo4_lean__<mangled>`              | Lean wrapper   | Lean native ABI                               | external |

The shim's `leo4_call_<mangled>` entry point decodes the canonical
buffer into Lean values via `lean.h`, calls `leo4_lean__<mangled>`
(the `@[export]`-ed Lean wrapper around the user's `@[leo4_export]`
definition), then encodes the return value back into the caller's
output buffer.

Concretely, the Lean side declares:

```lean
@[export leo4_lean__leo4__pkg__iface__fname__P1_P2__h<hash>]
def _leo4_export_<safe>_<param-suffix> (p0 : T0) (p1 : T1) ... : Ret := …
```

Both names live in one `.so`, so there is one link step, not two. The
`<mangled>` body is shared between the two prefixes so the shim can
resolve its Lean-side helper purely by string concatenation
(`"leo4_lean__" ++ mangled`) without consulting any extra mapping.

The `leo4_lean__` prefix is fixed: the prefix never participates in
the schema hash (it is a build-internal naming convention, not part
of the ABI surface) and it is reserved (see §7 below).

The `leo4_lean__` prefix starts with an alphabetic character because
Lean's `@[export ident]` validator (`Lean.isValidCppId`) requires
identifiers to begin with a letter, not an underscore. A leading-`_`
form like `_leo4_lean__…` is rejected at attribute-application time.
The prefix also stays disjoint from the mangled-body namespace:
mangled bodies begin with `leo4__` (double underscore), shim entry
points with `leo4_call_<mangled>`, and helper names with
`leo4_lean__<mangled>`, so no user IDL can collide on any side.

## 7. Reserved Symbols

The following symbols are reserved by leo4 for runtime internals; user IDL
must not produce mangled names that collide:

- `leo4__rt__*` — runtime API
- `leo4__shim__*` — shim helpers
- `leo4__panic_handler` — panic catcher
- `leo4_call_*` — C shim canonical-buffer entry points (see §6)
- `leo4_lean__*` — Lean-side native-ABI helper wrappers (see §6)

Collision is impossible by construction (user IDL must have a `pkg` segment),
but the linker will fail loudly if it ever happens, which is the desired
behavior.
