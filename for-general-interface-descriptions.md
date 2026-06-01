# Reusing the leo4 IDL for General Interface Descriptions

> Working notes on how to lift the leo4 IDL out of the Lean ↔ Rust interop
> setting and reuse it for other "this is the shape of an interface"
> use cases — most concretely, declaring the I/O of AI model blocks /
> layers / sub-graphs. Originally written against Phase 5 / W7-2c;
> reuse advice unchanged through Phases 6–9 and v1.0 RC.1–RC.4
> (2026-05-31). The reverse-direction expansion (RC.2 typed-enum
> mirror emit via `USER_TYPES` `linkme::distributed_slice`,
> RC.3 multi-candidate `rust_type_to_idl_candidates`, RC.4
> `#[leo4::export]` accepting user-defined types) is leo4-internal
> and does not alter the grammar / canonical-ABI / mangling
> surface this guide reuses.

## 1. What you are actually getting from leo4

leo4's IDL is three loosely-coupled artefacts that happen to ship in
the same repo. They can be reused independently.

| Artefact | What it is | Where it lives | Reuse value for other domains |
|----------|------------|----------------|--------------------------------|
| **Grammar** | EBNF for `package / interface / world / type / constraint / nominal_decl / func` | `SPEC/idl-grammar.ebnf` | Very high — it is a strict superset of WIT, so anything in this shape interops with the Wasm Component Model "for free". |
| **Canonical wire format** | Byte-level encoding of every type | `SPEC/canonical-abi.md` | Domain-dependent. Useful when interfaces actually cross a serialization boundary (RPC, on-disk graph, IPC). Not useful when the only thing crossing the boundary is metadata (e.g. tensor descriptors with the tensors themselves staying in-process). |
| **Mangling + schema hash** | Deterministic linker symbol + FNV-1a-64 schema digest, with `<prefix>_<body>` split for ABI / native-helper symbols | `SPEC/mangling.md` | Very high — the same change-detector pattern works any time you want "did the interface drift?" answered at link time / load time. |

Items orthogonal to the IDL itself but worth knowing:

- **Cross-impl conformance harness** (`tests/mangling/`) — pins two
  independent implementations of the grammar + mangling rules to
  byte-identical output. This is the pattern to copy when you have
  more than one language target.
- **Build-script hook** (`Build.lean`, `Leo4.Build` module) — project /
  per-user / system / fallback discovery order. Generalises to any
  emitter that needs an escape hatch for custom link / packaging
  steps.

## 2. What is leo4-specific and has to be replaced or pulled out

These pieces hard-code Lean and/or the canonical-buffer ABI; reusing
them as-is in another domain would drag in obligations that don't fit:

- **`LeanMarshal` / `LeanResource` typeclasses** (`lake/Leo4/Leo4/Marshal.lean`)
  — these are the Lean-side contract for "this type knows how to encode
  itself onto the wire." For a domain where the wire is conceptually
  different (e.g. a tensor whose representation is dictated by the
  underlying tensor library), they don't carry over and a domain-specific
  trait replaces them.
- **`@[leo4_export]` attribute + Lake plugin walker**
  (`lake/Leo4/Leo4/Export.lean`, `lake/Leo4Plugin/`) — same idea: it is
  Lean-environment introspection. The grammar / mangling / hash are
  language-agnostic; the way you *discover* declarations to feed into
  them is whatever your host environment provides.
- **`leo4_call_` / `leo4_lean__` prefixes** — these names are
  build-internal naming conventions, not part of the ABI surface
  (SPEC/mangling.md §6). Each reuse picks its own prefixes.
- **`SPEC/canonical-abi.md`** — couples encoding to little-endian flat
  layout because that is what shim wire format demands. A domain that
  represents interfaces but never actually serializes values (compile-time
  metadata, dispatcher tables) doesn't need any of §1–§12.
- **`lake/Leo4Plugin/Leo4Plugin/AdmitSet.lean`** — closed-world admit-set
  enumeration that drives per-generic monomorphisation. The mechanism
  generalises (it is "given a generic and a `oneof` constraint, expand
  to the Cartesian product"), but the implementation is currently
  fused to Lean's `Lean.Meta.SynthInstance` lookup. Lift it by
  parameterising over "how do I enumerate the admit-set".

## 3. Three reuse patterns — pick one

### A. Domain extension on top of leo4 (smallest move)

Take the existing IDL grammar and add a couple of domain-specific
builtins / attributes via the extension points the grammar already has:

- New nominal kind: `tensor`, like `record` / `variant` / `enum` /
  `flags` / `resource`. Add one production to the grammar and one
  `IDLType` variant to both implementations.
- Domain attributes via Lean's `@[…]` mechanism — analogous to
  `@[leo4_export]`, `@[leo4_resource]`, `@[leo4_specialize_when …]`.
  `@[layer_export]`, `@[gradable]`, `@[batched_axis n]` come for free
  as long as your plugin walker knows to look for them.

When this is the right fit: you actually want WIT compatibility,
your domain's wire format can ride on top of leo4's canonical ABI,
and you are happy depending on the leo4 plugin's lifecycle.

### B. Core split — `schema-idl` crate, leo4 as one consumer

Move grammar + parser + IDLType + mangling + schema-hash into a
language-neutral crate (`schema-idl` or similar). leo4 keeps its Lean
plugin, its `LeanMarshal`, its canonical ABI, and depends on
`schema-idl` for the type-system part. The neural-network project
depends on the same crate and provides its own emitter / marshalling
contract.

Concrete steps:

1. Pull `crates/leo4-idl/src/{idl,parse,render,mangle,hash,base32}.rs`
   into a new crate. They are already free of any leo4-specific
   identifiers — just `IDLType`, `UserDecl`, `mangle(...)`, `Hash::of_str`.
2. `crates/leo4-idl/src/wit.rs` (cyclic-ADT-to-resource fallback)
   stays in leo4 because it lowers to WIT-flat-resource which is a
   wire-format choice.
3. The Lake plugin's `Mangling.lean` is similarly portable; the rest
   of the plugin (admit-set, schema/handshake/exports emit) is
   leo4-specific orchestration on top.
4. `SPEC/idl-grammar.ebnf` becomes a sibling document to the new core
   crate; `SPEC/canonical-abi.md` stays in leo4.

When this is the right fit: more than two consumers will share the
type-system layer, and you want a clean dependency graph instead of a
"leo4 fork with a different emitter."

### C. Sibling spec — copy the grammar, diverge the semantics

For a domain where neither WIT compatibility nor binary marshalling
matters — typical AI-graph IDL territory, where interfaces describe
*compile-time shapes* and the runtime moves tensors out-of-band — you
end up reusing essentially three things: the grammar shape, the schema
hash, and the constraint sublanguage. Re-implement them in your
domain's house style; do **not** try to share parsers, because the
constraint vocabulary diverges sharply (`scalar | ord | eq | hash`
in leo4 vs. `differentiable | broadcastable | sparse | quantized` in
NN-land).

What you carry over verbatim:

- `package` / `interface` / `func_decl` skeleton — the WIT-shaped
  declaration framework.
- `nominal_decl`'s short form (`record FQN { … }` etc.) — the
  one-line nominal-type declaration is the most readable feature.
- `generic_params` with the `type_param / value_param` split — see §4.
- `mangle()` + `fnv1a64` schema hash — the link-time change detector.
- The two-tier ABI naming (`<call_prefix>_<mangled>` for the entry
  point, `<lang_prefix>_<mangled>` for the host-language helper).
- `Build.lean`-style discovery order for emission hooks.

What you redefine: `primitive`, `builtin_generic`, the
`constraint_atom` vocabulary, and the canonical-ABI section in toto.

## 4. Why `value_param` is the lever you actually want for AI use cases

The leo4 grammar has a feature that is barely exercised inside leo4
but is the right primitive for tensor shape annotations:

```
generic_param = type_param | value_param ;
value_param   = ident , ":" , type ;             -- (* erased at the boundary *)
```

A `value_param` carries no wire-format payload (SPEC/mangling.md
§"Value-param erasure") but **does** participate in the schema hash —
so the *name* of `n` in `Vec n α` matters, but no concrete value of
`n` ever leaves the host language. For AI interfaces this is exactly
what you want:

```text
interface attention {
    func qkv<batch : usize, seq : usize, dim : usize>
        (q: tensor<f32, [batch, seq, dim]>,
         k: tensor<f32, [batch, seq, dim]>,
         v: tensor<f32, [batch, seq, dim]>)
        -> tensor<f32, [batch, seq, dim]>;
}
```

You get:
- compile-time shape checking on the host side,
- a stable mangled symbol that does not vary with the concrete batch
  size (because value_params don't enter the mangle),
- a schema hash that **does** change if you rename `dim` to `embed`,
  which is the right level of strictness.

leo4 already enforces "dependent codomain rejection" at the
boundary (SPEC/mangling.md §4 "Mandatory checks #4"); copy that
unchanged. Dependent *parameter* types are fine; dependent *return*
types aren't.

## 5. Concrete sketch — what an AI-block IDL on this foundation looks like

```text
package my.nn:transformer;

constraint differentiable = trait { func grad(_: Self) -> Self; };
constraint quantizable    = oneof { f32, f16, bf16, i8 };

record  Tensor<dt: quantizable, rank: usize, shape: list<usize>> {
    storage: opaque<dt>,
}

interface block {
    @[gradable]
    func forward<batch : usize, seq : usize, d : usize>
        (x : Tensor<f32, 3, [batch, seq, d]>)
        -> Tensor<f32, 3, [batch, seq, d]>;

    func init_params<seed : u64>() -> ParamPack;

    resource ParamPack { method save(path : string) -> result<unit>; };
}
```

What is borrowed verbatim from leo4 here:

- the package + interface + record + resource skeleton,
- `@[gradable]` as a domain attribute parallel to `@[leo4_export]`,
- `value_param` (`batch`, `seq`, `d`, `seed`) for shape generics,
- `constraint = oneof { … }` to pin the admit-set of `dt`,
- the trait-style constraint for `differentiable`,
- `result<…>` and `resource` as composite shapes.

What is new:

- `Tensor<dt, rank, shape>` — a domain builtin like `list<T>`,
- `opaque<dt>` — explicitly *not* serialised across the boundary;
  the tensor handle is what crosses, the bytes do not,
- the constraint atoms diverge (`quantizable`, `differentiable`).

Everything else — mangling, schema hash, handshake file format,
discovery hooks — is reused unmodified.

## 6. Pitfalls and open design questions

- **Wire format vs. metadata-only**. leo4's canonical ABI is byte
  marshalling; AI interfaces typically want metadata-only. Don't try
  to retrofit the ABI; mark tensor types as opaque to it and let the
  runtime move the tensor however it wants. The IDL becomes a
  *schema description*, not a serialisation contract.
- **Specialisation explosion**. NN admit-sets are wider than leo4's
  (`dt × rank × shape`). Without an explicit `oneof`, the closed-world
  Cartesian product blows up. The same constraint the leo4 plugin
  enforces today — "HK params MUST carry a constraint" — applies
  doubly here. Borrow SPEC/mangling.md §4 "Mandatory check #5"
  verbatim.
- **Schema hash + weights**. If the schema hash gates ABI
  compatibility, also gate *weight loadability* on it. A renamed
  generic parameter rotating the hash should reject pickled weights,
  same way leo4 rejects a stale shim.
- **Multi-implementation backends** (CPU / GPU / TPU). Treat each as
  an emitter with its own dispatch table, like leo4's `leo4-mslean4`
  vs the future `leo4-wasm`. Mangling stays one source of truth; the
  emitter decides what `<prefix>_<mangled>` resolves to.
- **Forward / backward asymmetry**. leo4 funcs are uni-directional.
  For autodiff, either model `forward` and `backward` as two funcs in
  the same interface (boring but works), or introduce a `coproc` /
  `bidir` declaration kind. The latter is a new EBNF production and a
  new IDLType variant; both implementations must learn it.
- **Versioning policy**. leo4's FNV-1a-64 hash is a change detector,
  not a compatibility classifier — it answers "did anything change?"
  rather than "is this backward compatible?" For AI weights this is
  usually too strict; consider layering a semver-style compatibility
  tag on top of the hash for the *advisory* part of the answer while
  keeping the hash for the *normative* part.

## 7. Decisions recorded so far

Use this section as the canonical source for "we already chose X, don't
re-litigate." Update with the date and a one-line rationale; defer
implementation when the schema-idl change isn't on the critical path
of a current phase.

### D-i (2026-05-19) — `future<T>` / `stream<T>` are function-level effects, not type variants

When `D4` lifts (WASIp3 stabilises), `future<T>` and `stream<T>` enter
the IDL **only at the boundary position of a function declaration's
return type**, not as a new `IDLType` variant.

- Surface form: `func foo(x: T) -> future<U>;` (no IDLType change),
  or alternatively a leading qualifier `async func foo(x: T) -> U;`
  (sugar that parses identically).
- AST shape: `FuncDecl { effect: Sync | Async | Stream | …, params, ret }`.
  `IDLType` itself does not gain a `Future` / `Stream` variant.
- Parse error: `future<T>` / `stream<T>` inside record / variant
  payloads or as a generic argument.
- Rationale: keeps `IDLType` colourless (no effect-wrapper leakage
  into payloads); matches the WIT "function ABI carries the future,
  the value doesn't" mental model; downstream consumers that don't
  need async (e.g. AI-block IDL) ignore the field for free.
- Implementation deferred to leo4 Phase 7.

### D-ii (2026-05-19) — Constraint sublanguage: typed AST with pluggable atom registry

The constraint sublanguage (`constraint_atom` in
`SPEC/idl-grammar.ebnf`) parses into a **typed AST in schema-idl**, not
a raw string the way the current `ConstraintDeclRaw.body` does. The
atom vocabulary itself is **pluggable**: schema-idl defines the
`ConstraintExpr<Atom>` shape (Boolean combinators `∧ / ∨ / ¬`,
`type: trait`, `type = type`, `oneof { … }`, etc.) generically over
the atom set; each downstream consumer supplies its own atoms.

- leo4 atoms: `scalar / ord / eq / hash / pod / marshal / resource`,
  per the current grammar.
- AI-IDL atoms (hypothetical example): `differentiable / quantizable
  / broadcastable / sparse`.
- The registry is a `trait` (or generic parameter) on the parser
  entry point: `parse_with_atoms::<MyAtoms>(text) -> Schema<MyAtoms>`.
- schema-idl ships a `NoAtoms` empty-set implementation so consumers
  that don't use constraints don't take on the registry burden.
- Implementation deferred to whenever the leo4 plugin reworks its
  current "constraint elaboration is parametric-attribute-only"
  state (LEO4-DESIGN.md D5).

## 8. Recommendation

Default to **option B (core split)** unless one of the following holds:

- You are *certain* there will be only one non-leo4 consumer ever →
  option A is cheaper.
- The constraint vocabulary you need is so far from `scalar | ord |
  eq` that sharing a parser would require a settings-driven
  constraint atom registry → option C avoids that complication
  entirely.

The reason B is the default: every piece you'd otherwise re-implement
(grammar, parser, IDLType, mangling, hash, base32lc, normalised-form
fnv1a) is already free of leo4 identity in this tree. Pulling them
into `schema-idl` is a mechanical refactor of ~6 files and gives every
downstream consumer the cross-impl conformance harness for free.

When you do start that split, the boundary to draw is:

```
schema-idl     ← grammar, IDLType, parser, mangling, hash, base32, render
leo4 (kept)    ← Lean plugin, LeanMarshal, canonical-abi, WIT lowering, shim emit
nn-idl (new)   ← tensor builtin, differentiable/quantizable constraints,
                 forward/backward emitter, weight-compat policy
```

Each downstream picks an emitter / runtime / discovery policy. The
type system and the change-detector are shared.
