# schema-idl shortcomings

> Outstanding limitations of the domain-neutral `crates/schema-idl/`
> crate, as of W7-2d (2026-05-19). This file is the single ledger
> for "what's left in schema-idl, ranked." Limitations that belong
> downstream (leo4 plugin's shim emitter, `LeanMarshal` instances,
> WIT lowering, etc.) are listed in `ROADMAP.md` / `LEO4-DESIGN.md`
> and intentionally **not** repeated here.
>
> Companion docs:
> - `for-general-interface-descriptions.md` — reuse guide + decision log
> - `LEO4-DESIGN.md` — leo4-specific design decisions (D1…D13)
> - `SPEC/idl-grammar.ebnf` — normative IDL grammar
> - `SPEC/mangling.md` — normative mangling rules
> - `SPEC/canonical-abi.md` — normative wire format (consumer-side; not
>   schema-idl's concern)

## Index

| #  | Title                                                              | Grammar impact | Status   |
|----|--------------------------------------------------------------------|----------------|----------|
| 1  | `UserDecl::Flags` variant missing + parser collapses flags to Enum | no             | open     |
| 2  | `FuncDecl.effect` field missing (D-i decided 2026-05-19)           | yes            | deferred |
| 3  | `ConstraintExpr<Atom>` typed AST missing (D-ii decided 2026-05-19) | yes            | deferred |
| 4  | Generic type-parameter substitution helper missing                 | no             | landed 2026-05-20; generic-record wire-up demo waits on leo4 `deriving LeanMarshal` handler |
| 5  | `mutual_group` production / cross-decl recursion                   | yes            | Phase 6  |

"Status" legend:
- **open** — nothing blocks landing it; just hasn't been prioritised.
- **deferred** — decision recorded, implementation parked until a
  consuming phase / external need arrives.
- **Phase N** — fix tied to a leo4 roadmap phase entry gate.

---

## #1 `UserDecl::Flags` variant missing + parser collapses `flags` to `Enum`

### Current state

- `IDLType` carries `IDLType::Flags(String)` (a type-level reference
  to a flags FQN), so a function signature *referring to* an
  already-declared flags type round-trips correctly.
- `UserDecl` has variants for `Record`, `Enum`, `Variant`, and
  `Resource` only. There is no `UserDecl::Flags`. A user-declared
  `flags Sample.Perms { read, write, exec };` therefore cannot be
  represented as a *declaration* in the schema; the parser absorbs
  it as if it were an `enum` declaration with a hint, losing the
  flags/enum distinction at the AST level.

### Source location

- `crates/schema-idl/src/idl.rs` — `pub enum UserDecl { … }` (no
  `Flags` arm).
- `crates/schema-idl/src/parse.rs:996` — `parse_flags_decl` exists.
- `crates/schema-idl/src/parse.rs:1017` (comment) —
  `// We re-use RawDecl::Enum for flags at the parse layer; the …`
  is where the parser drops the distinction.

### Severity

Information loss on IDL roundtrip for any package that declares a
`flags` type. leo4's sample doesn't declare flags, so cross-impl
mangling stays byte-identical today — the bug is latent rather than
actively breaking. Once any downstream IDL needs flags (likely AI
permissions / quantization mode bitfields, etc.), the
declaration-level entry is required.

### Grammar impact

None. `nominal_flags_decl` is already a first-class production in
`SPEC/idl-grammar.ebnf` line 47, and the parser does tokenise it. The
fix is entirely below the parser, at the `RawDecl → UserDecl`
resolution layer.

### Downstream impact

- leo4 plugin: cannot emit a `flags` declaration into
  `<pkg>.leo4-schema` correctly. Currently moot because no sample
  exercises it.
- Other schema-idl consumers (AI-block IDL etc.): blocks any domain
  that wants user-declared bitfields.

### Implementation sketch

1. Add `UserDecl::Flags { fqn: String, generics: Vec<String>, members: Vec<String> }`.
2. Add a parallel `RawDecl::Flags` (or a tag inside the existing
   `RawDecl::Enum` so the resolver can switch on it).
3. In `parse_flags_decl`, emit the new raw form rather than reusing
   `RawDecl::Enum`.
4. Update `resolve_decl` to route the new raw form to
   `UserDecl::Flags`.
5. `render::user_decl_to_idl` already needs to handle it; verify the
   canonical form emits `flags <fqn> { … };`.
6. `mangle_type` (`crates/schema-idl/src/mangle.rs`) for
   `IDLType::Flags` is already `"F_<fqn>_f"`; no change there.
7. Add a roundtrip test alongside `parse::tests::*` and the cross-impl
   conformance fixtures.

The `UserDecl::Flags` variant adds a path that is symmetric to
`Enum` plus an optional `generics` field; the code shape is small.

### Cross-references

- `SPEC/idl-grammar.ebnf` line 47 (`nominal_flags_decl`).
- `SPEC/mangling.md` §2 (the `mangle_type(flags F)` rule).

---

## #2 `FuncDecl.effect` field missing (D-i 2026-05-19)

### Current state

`FuncDecl` carries `name`, `generics`, `params: Vec<(String, IDLType)>`,
`ret: Option<IDLType>` — no effect tag. The IDL grammar's
`builtin_generic` allows `future<T>` and `stream<T>` (marked "deferred
to WASIp3"), but with no `effect` field there is no place in the AST
to record "this function returns asynchronously".

### Source location

- `crates/schema-idl/src/idl.rs` — `pub struct FuncDecl { … }`.
- `crates/schema-idl/src/parse.rs` — `parse_func_decl` /
  `parse_func_sig` (no effect-aware branch).
- `SPEC/idl-grammar.ebnf` line 110-111 (`future<T>` / `stream<T>`
  parked in `builtin_generic`).

### Severity

No active impact at v0 (D4 keeps the runtime sync). Required before
Phase 7 enters the wire format. The decision (D-i) closed the design
debate: async/stream are **function-level effects**, never `IDLType`
variants, so the work surface is small and well-bounded.

### Grammar impact

Yes.

- The `future<T>` / `stream<T>` productions stay in
  `builtin_generic`, but become **context-restricted**: only valid
  at the immediate boundary position of a `func_sig`'s return type.
  Anywhere else (inside records, variants, lists, tuples, options,
  generic arguments) is a parse error.
- Context-sensitive rules don't fit cleanly in EBNF. Either:
  - keep the production and add a normative semantic check note,
  - or drop the production and add a `func_sig` prefix qualifier
    (`async` / `stream`) that the parser maps onto the same effect
    field.
- The implementer picks one of those two surface forms at Phase 7
  entry; the AST landing zone (a `FuncDecl::effect` field) is the
  same.

### Downstream impact

- leo4 plugin: needs to start tracking effect from this point so the
  shim emitter can produce a future-aware ABI when Phase 7 fires.
  Before then it just sees `Effect::Sync` and the existing code path
  unchanged.
- Other schema-idl consumers: sync-only consumers (AI-IDL with no
  streaming requirement) ignore the field. Async-savvy consumers
  (RPC schemas, actor model IDLs) get a first-class place to plug in.

### Implementation sketch

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Effect {
    Sync,
    Async,           // boundary `future<T>` (or `async func … -> T`)
    Stream,          // boundary `stream<T>` (or `stream func … -> T`)
    AsyncStream,     // both, if the chosen surface allows it
}

pub struct FuncDecl {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub params:  Vec<(String, IDLType)>,
    pub ret:     Option<IDLType>,
    pub effect:  Effect,   // defaults to Sync in pre-Phase-7 code paths
}
```

Parser changes:
1. In `parse_func_sig`, after parsing the optional `-> type`,
   inspect the parsed type: if it is `IDLType::Future(inner)` /
   `IDLType::Stream(inner)`, **don't** materialise a `Future` /
   `Stream` IDLType (none exists per D-i); instead store
   `effect = Async` / `Stream` and the unwrapped inner as `ret`.
2. Add a recursive walk that rejects `future<T>` / `stream<T>` if
   found anywhere outside that boundary position. Best done in
   `resolve_type`.
3. The two raw surface forms (`-> future<T>` vs `async func …`) can
   both be supported with no AST difference.

Mangling impact: SPEC/mangling.md needs an entry for effect-bearing
functions. Simplest is to include the effect in the mangled name
(e.g. `__async__` or `__stream__` token) so a sync caller cannot
accidentally link against an async helper. This is the only place
schema-idl's mangling rules change.

### Cross-references

- `LEO4-DESIGN.md` D4 (decision body, updated 2026-05-19).
- `ROADMAP.md` Phase 7 "Async IDL" — deliverables updated to reflect
  the function-level effect model.
- `for-general-interface-descriptions.md` §7 D-i.
- `SPEC/idl-grammar.ebnf` line 110-111 — parked productions, with
  the lifting note.

---

## #3 `ConstraintExpr<Atom>` typed AST missing (D-ii 2026-05-19)

### Current state

The constraint sublanguage (`SPEC/idl-grammar.ebnf` §
`constraint_decl`, `constraint_body`, `constraint_atom`,
`constraint_expr`) is recognised by the parser as far as the
*outer* declaration (`constraint Name = body;`), but the body itself
is preserved as an opaque string (`ConstraintDeclRaw.body: String`).
Evaluation lives in the leo4 plugin's admit-set walker, which itself
is currently only partially implemented (LEO4-DESIGN D5 — registered
as a `ParametricAttribute Syntax`, plugin does not yet elaborate it).

D-ii closes the design question of *where* constraint AST should
live: **schema-idl owns the AST shape**, but the atom vocabulary
is pluggable per consumer (leo4 vs. AI-IDL vs. …) so that the schema
core stays domain-neutral.

### Source location

- `crates/schema-idl/src/parse.rs:123–129` — the
  `ConstraintDeclRaw` shape + the parser's "balanced-only, raw body"
  promise.
- `crates/schema-idl/src/idl.rs` — no `ConstraintExpr` /
  `ConstraintAtom` types exist.
- `SPEC/idl-grammar.ebnf` line 60–73 — the productions that ought to
  have a corresponding typed AST.

### Severity

Largest of the five. Constraint elaboration is what drives admit-set
enumeration, which in turn drives the entire mangling table.
Currently the leo4 plugin papers over the gap by treating
`leo4_specialize_when` as a `Syntax` attribute that it parses
separately, but anything downstream that wants to evaluate
constraints uniformly must either reimplement parsing from raw
strings or duplicate the leo4 plugin's elaborator.

### Grammar impact

Yes.

- The hard-coded `constraint_atom` alternatives in EBNF line 67
  (`"scalar" | "ord" | "eq" | "hash" | "pod" | "marshal" | "resource"
  | ident | type ":" ident | type "=" type | "¬" constraint_atom |
  "(" constraint_expr ")"`) become a **default leo4-atom set**.
  Domain-specific atoms are introduced by the consumer's atom
  registry, not by adding alternatives to EBNF.
- EBNF gains one note paragraph clarifying that the literal keywords
  shown are leo4's default; the production semantically depends on
  the atom registry the parser is invoked with.

### Downstream impact

- leo4 plugin: gets a typed `Schema<LeoAtoms>` instead of raw
  strings; the admit-set walker can pattern-match on `ConstraintExpr`
  values directly.
- AI-IDL (hypothetical): can define its own atoms
  (`differentiable`, `quantizable`, `broadcastable`, …) and reuse
  the same `parse_with::<NnAtoms>` entry point. The two consumers'
  schemas remain incompatible at the type level (different `Atom`
  parameter), which is the right level of strictness.

### Implementation sketch

```rust
// In schema-idl::constraint (new module)
pub trait Atom: Sized + Clone + Eq + core::fmt::Debug {
    /// Parse a single bare-word atom keyword (e.g. "scalar", "ord",
    /// or a domain-specific "differentiable"). Returns None when
    /// the keyword is not in this registry; the parser may then
    /// try other atom forms (`type ":" ident`, etc.).
    fn from_keyword(s: &str) -> Option<Self>;
    /// Render back to the source form; used by canonical render.
    fn as_keyword(&self) -> &'static str;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstraintExpr<A: Atom> {
    Builtin(A),
    HasTrait { ty: IDLType, trait_name: String },
    TypeEq    { lhs: IDLType, rhs: IDLType },
    Not(Box<ConstraintExpr<A>>),
    And(Box<ConstraintExpr<A>>, Box<ConstraintExpr<A>>),
    Or (Box<ConstraintExpr<A>>, Box<ConstraintExpr<A>>),
    Oneof(Vec<IDLType>),
    TraitBody(Vec<FuncDecl>),
}

pub struct ConstraintDecl<A: Atom> {
    pub name: String,
    pub body: ConstraintExpr<A>,
}

// Schema becomes generic.
pub struct Schema<A: Atom = LeoAtoms> {
    pub package: String,
    pub interface: String,
    pub user_decls: Vec<UserDecl>,
    pub funcs:      Vec<FuncDecl>,
    pub constraint_decls: Vec<ConstraintDecl<A>>,
    // …
}

// Default atom set for leo4.
pub enum LeoAtoms { Scalar, Ord, Eq, Hash, Pod, Marshal, Resource }
impl Atom for LeoAtoms { … }
```

Parser entry points:

```rust
// Current: pub fn parse(text: &str) -> Result<Schema, ParseError>
// Becomes:
pub fn parse(text: &str) -> Result<Schema<LeoAtoms>, ParseError> { … }
pub fn parse_with<A: Atom>(text: &str) -> Result<Schema<A>, ParseError> { … }
```

`Schema<LeoAtoms>` keeps the existing `Schema` type alias working for
downstream code, so the leo4 plugin and `leo4c` see no signature
change unless they want to switch atom set.

Other AST nodes that hold constraint references (e.g.
`GenericParam` carrying an inline `: constraint_expr` annotation)
also become generic over `A`. Verify the chain doesn't leak `A`
into places it shouldn't (e.g. `mangle()` is constraint-free and
should stay non-generic).

### Cross-references

- `LEO4-DESIGN.md` §5 ("admit-set enumeration") and D5.
- `SPEC/idl-grammar.ebnf` lines 60–73.
- `for-general-interface-descriptions.md` §7 D-ii.

---

## #4 Generic type-parameter substitution helper missing

### Current state

`UserDecl::Record { generics: Vec<String>, fields: … }` and
`UserDecl::Variant { generics: Vec<String>, cases: … }` carry their
parameter binders. Field/case types in those declarations may
reference a binder by name — and they do so via the existing
`IDLType::Record { fqn, args }` form, where `fqn` happens to match a
binder string. There is no `IDLType::TypeVar(String)`; type-variable
references look just like nullary type references.

Substitution — "given `Record { fqn = "Pair", args = [u32, str] }`
look up the declaration, walk its fields, and replace the binder
names with `u32` / `str`" — is currently done inline inside the
leo4 plugin's nominal handler, and only when both
`generics.is_empty()` and `args.is_empty()` (i.e., the
no-substitution case). Hence the shim's "generic-nominal types are
stubbed" limitation.

### Source location

- `crates/schema-idl/src/idl.rs` — `UserDecl` carries `generics` but
  no helpers.
- `crates/schema-idl/src/parse.rs` — resolution does not perform
  substitution.
- `lake/Leo4Plugin/Leo4Plugin/Main.lean:1066+` — handler currently
  bails on non-empty generics.

### Severity

The actual restriction (`generics.is_empty()` check in the shim)
prevents wire-up for any user-defined generic record / variant /
resource. The leo4 sample uses no such generics, so cross-impl
mangling is unaffected. Real consumers — including any meaningful
AI-IDL — will hit this immediately (`Tensor<dt, rank, shape>` is the
canonical example).

### Status (2026-05-20)

- **schema-idl side: landed.** `crates/schema-idl/src/subst.rs`
  exports `substitute(ty, env)`, `instantiate_record(decl, args)`,
  `instantiate_variant(decl, args)`, with 11 unit tests covering
  leaf substitution, nested composites, generic record application,
  Self pass-through, and arity mismatches.
- **leo4-idl re-exports them** at the crate root so existing
  call sites can pick the new helpers up without an extra dependency.
- **Plugin adoption: landed at the code level.**
  `lake/Leo4Plugin/Leo4Plugin/AdmitSet.lean` ships
  `Subst.substIDL` / `Subst.mkEnv` (line-for-line mirror of the Rust
  helpers). `lake/Leo4Plugin/Leo4Plugin/Main.lean::handlerFor`
  swaps the `if !generics.isEmpty || !args.isEmpty then none` bail
  for proper `mkEnv` + `substIDL` walks on the
  `.record` / `.resource` / `.variant` branches. Arity mismatch
  (binders vs args) still returns `none` — that is the
  caller-bug-in-IDL case the kind discipline should catch upstream.
- **End-to-end demo blocked elsewhere.** A sample fixture with a
  user-declared `structure Pair (α β : Type) … deriving LeanMarshal`
  hit `error: deriving LeanMarshal: generic inductive 'Sample.Pair'
  not yet supported` in `lake/Leo4/Leo4/Deriving.lean`. That gap is
  a leo4-runtime-library limitation, not a schema-idl one;
  consequently the generic-record wire-up path is exercised today
  only by the schema-idl Rust unit tests and by code review of the
  mirror in `Subst.substIDL`. Filing a follow-up against
  `lake/Leo4/Leo4/Deriving.lean` to support generic inductives will
  unlock the end-to-end fixture.

### Grammar impact

None. The grammar already accepts generic nominal declarations.

### Downstream impact

- leo4 shim emitter: gain wire-up for generic records, variants, and
  resources. Removes the "isEmpty || isEmpty" condition in
  `handlerFor`.
- AI-IDL: required out of the box — tensor types are parametric on
  dtype + rank + shape.

### Implementation sketch

A pure function in schema-idl that walks `IDLType` and replaces
named references whose fqn matches one of the binder names:

```rust
// In schema-idl::idl (or a new submodule schema-idl::subst).
pub fn substitute(ty: &IDLType, env: &[(String, IDLType)]) -> IDLType {
    use IDLType::*;
    match ty {
        Record { fqn, args } if args.is_empty()
            && env.iter().any(|(n, _)| n == fqn) =>
            env.iter().find(|(n, _)| n == fqn).unwrap().1.clone(),
        Record { fqn, args } => Record {
            fqn: fqn.clone(),
            args: args.iter().map(|a| substitute(a, env)).collect(),
        },
        Variant  { fqn, args } => Variant  { … },
        Resource { fqn, args } => Resource { … },
        List(t)                => List(Box::new(substitute(t, env))),
        Option(t)              => Option(Box::new(substitute(t, env))),
        Result(t, e)           => Result(
            Box::new(substitute(t, env)),
            e.as_ref().map(|x| Box::new(substitute(x, env))),
        ),
        Tuple(ts)              => Tuple(ts.iter().map(|x| substitute(x, env)).collect()),
        Io(t)                  => Io(Box::new(substitute(t, env))),
        // Primitive, Enum(_), Flags(_), Self_, SelfApp(_) → returned verbatim.
        t                      => t.clone(),
    }
}

/// Convenience: given a UserDecl and concrete args, return the
/// concrete record fields / variant cases after substitution.
pub fn instantiate_record(d: &UserDecl, args: &[IDLType]) -> Option<Vec<(String, IDLType)>> { … }
pub fn instantiate_variant(d: &UserDecl, args: &[IDLType]) -> Option<Vec<(String, Vec<IDLType>)>> { … }
```

The "named reference looks like nullary type" identification is the
only subtle point: substitution walks must recognise that an
`IDLType::Record { fqn = "T", args = [] }` whose `fqn` matches a
binder in `env` is in fact a type-variable, not a nullary record.
This is exactly the test the plugin already does implicitly; lifting
it into schema-idl gives every consumer the same semantics.

(Optional, post-decision: introduce a dedicated
`IDLType::TypeVar(String)` variant and have the resolver convert at
parse time. Clean but mangles the cross-impl byte stream — would
need SPEC update. Leave for a future tightening pass.)

### Cross-references

- `LEO4-DESIGN.md` §4.2 ("Constraint-driven instantiation
  enumeration").
- `lake/Leo4Plugin/Leo4Plugin/Main.lean` — `handlerFor` for
  `.record` / `.variant` / `.resource`.

---

## #5 `mutual_group` production / cross-decl recursion

### Current state

`IDLType::Self_` and `IDLType::SelfApp(args)` cover *direct*
self-recursion inside one declaration (the `Sample.Tree` case). The
W7-2d-ii variant handler emits a self-recursive C helper for this
shape. **Cross**-declaration cycles — two nominal types that name
each other — are rejected per LEO4-DESIGN §4.3 ("Forbidden in v0"),
and the shim emitter would not know how to lay them out anyway.

### Source location

- `crates/schema-idl/src/idl.rs` — `Self_` / `SelfApp(args)`, no
  cluster type.
- `crates/schema-idl/src/parse.rs` — no `mutual_group` production
  handling.
- `LEO4-DESIGN.md` §4.3 — the forbidden-features entry that will be
  lifted in Phase 6.

### Severity

Zero today. Phase 6 entry decides whether to lift, at which point
schema-idl gains a `mutual_group` representation and mangling rule
extensions for cluster members that mention each other.

### Grammar impact

Yes, explicitly. ROADMAP.md Phase 6 lists this as the first
deliverable: "`SPEC/idl-grammar.ebnf`: a `mutual_group` production
wrapping a contiguous set of `type_decl`s that the kind-discipline
checker treats as a single recursion frame."

### Downstream impact

- leo4 plugin: per-cluster `walkUserDecl` emit; `deriving LeanMarshal`
  synthesises `mutual` blocks of `partial def`s.
- Mangling: cluster members reference each other by FQN in their
  mangled type encodings, identical to today's nominal references —
  the schema hash, not the per-symbol name, prevents collisions.

### Implementation sketch

1. Add a `Schema.mutual_groups: Vec<Vec<UserDecl>>` (or move
   `user_decls` into a `DeclGroup` enum). Decide whether
   single-decl groups stay in their own field or get rolled into a
   trivial 1-element cluster.
2. Parser: new `mutual { … }` syntactic block. Inside, the regular
   nominal-decl productions are accepted, and the resolver bundles
   them as a single cluster.
3. Kind-discipline check: cluster members visible to each other when
   resolving field/case types.
4. SPEC/mangling.md: add a clause noting that mutual-cluster
   cross-references mangle exactly like ordinary nominal references
   (no special token); the schema hash absorbs the cluster shape via
   the normalised IDL form.
5. SPEC/canonical-abi.md §8.1 (self-recursive depth cap): generalise
   to "any cluster traversal" or note that the cap is per-cluster
   stack frame.

### Cross-references

- `LEO4-DESIGN.md` §4.3.
- `ROADMAP.md` Phase 6 "Mutual recursion between nominal types".
- `SPEC/idl-grammar.ebnf` (target for the new production).

---

## Priority ordering (decision-aware)

After D-i / D-ii landed:

| Rank | Item | Why this rank |
|------|------|---------------|
| **A** | #4 substitution helper | Highest value-to-effort: pure function in schema-idl, no grammar work, immediately unblocks the leo4 shim's generic-nominal restriction and provides AI-IDL reuse for free |
| **B** | #2 `FuncDecl.effect` field | Bounded by D-i; small AST + parser change. Land before Phase 7's entry gate fires so the async work isn't piling on top of unrelated AST surgery |
| **C** | #1 `UserDecl::Flags` | Small isolated PR. Restores IDL roundtrip correctness; sample-fixture pickup waits for a consumer that declares flags |
| **D** | #3 `ConstraintExpr<Atom>` | Largest churn — `Schema<A>` parametric across the crate. Plan: backward-compat alias `type Schema = SchemaG<LeoAtoms>` so existing call sites compile unchanged. Land when the leo4 plugin's elaborator work begins, or when the first non-leo4 atom set has a concrete user |
| **E** | #5 mutual recursion | Phase 6 owns the entry decision; until then the v0 rejection is normative |

## Decision log

| Date       | ID   | Decision                                                                                  |
|------------|------|-------------------------------------------------------------------------------------------|
| 2026-05-19 | D-i  | future/stream are function-level effects (`FuncDecl.effect`), not `IDLType` variants      |
| 2026-05-19 | D-ii | constraint sublanguage uses a typed AST + pluggable atom registry in schema-idl           |

(For leo4-side decisions see `LEO4-DESIGN.md` §2 "Decisions" table.)
