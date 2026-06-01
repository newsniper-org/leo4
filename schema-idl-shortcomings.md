# schema-idl shortcomings

> Outstanding limitations across the IDL stack, last audited
> 2026-05-23 after the Phase 9 reverse-direction pipeline ran
> end-to-end for the first time (declarative Lake `extern_lib`
> integration + handshake-consume + glue-shim ABI all landed
> 2026-05-23). **Re-audited 2026-05-31 against v1.0 RC.1–RC.4:**
> no new schema-idl shortcomings introduced; the RC.2–RC.4
> reverse-direction work (typed-enum mirror via `USER_TYPES`
> `linkme::distributed_slice`, multi-candidate
> `rust_type_to_idl_candidates`, `#[leo4::export]` accepting
> user-defined types) operates entirely on the Rust-side
> macro / emit boundary — schema-idl grammar / IDLType /
> mangling all unchanged. Phases 0–9 are done; this ledger
> keeps the residual schema-idl / plugin / runtime items
> distinct from the released surface. Each item is labelled
> by ownership (`schema-idl` / `leo4-plugin` /
> `leo4-runtime`); the cascade table in §"Dependency graph"
> makes the inter-item order explicit.
>
> Companion docs:
> - `for-general-interface-descriptions.md` — reuse guide + decision log
> - `LEO4-DESIGN.md` — leo4-specific design decisions (D1…D13)
> - `SPEC/idl-grammar.ebnf` — normative IDL grammar
> - `SPEC/mangling.md` — normative mangling rules
> - `SPEC/canonical-abi.md` — normative wire format (consumer-side; not
>   schema-idl's concern)

## Index

| #  | Owner         | Title                                                              | Grammar impact | Status   |
|----|---------------|--------------------------------------------------------------------|----------------|----------|
| 1  | schema-idl    | `UserDecl::Flags` variant missing + parser collapses flags to Enum | no             | Rust side landed 2026-05-20; Lean `UserDecl.flagsT` pending |
| 2  | schema-idl    | `FuncDecl.effect` field missing (D-i decided 2026-05-19)           | yes            | landed end-to-end 2026-05-20 → 2026-05-21 (Phase 7 step 1/2a/2b/2c) |
| 3  | schema-idl    | `ConstraintExpr<Atom>` typed AST missing (D-ii decided 2026-05-19) | yes            | deferred |
| 4  | schema-idl    | Generic type-parameter substitution helper                         | no             | landed 2026-05-20 (Rust + Lean mirror) |
| 5  | schema-idl    | `mutual_group` production / cross-decl recursion                   | yes            | landed Phase 6 (2026-05-20) — SPEC/phase-6-mutual.md |
| 6  | schema-idl    | Parser rejects non-ASCII identifiers (`α`, `β`, …)                 | no             | sidestepped 2026-05-20 by ASCII-positional binder names from plugin (parser unchanged) |
| 7  | schema-idl    | `render::user_decl_to_idl` omits `generic_params` on nominal decls | yes (cosmetic) | landed 2026-05-20 (Rust render + Lean `userDeclToIDL`; parser `RawDecl` now carries `generics`; resolver wires `Shape::TypeVar` for in-scope binders) |
| 8  | leo4-runtime  | `deriving LeanMarshal` generic-inductive support                   | no             | committed 2026-05-20 (`ef40451` + cascade `a8556a8` / `b2df550` / `92a0d9f`) |
| 9  | leo4-plugin   | `walkUserDecl` generic-aware (param names, FVar→placeholder subst) | no             | committed 2026-05-20 (`ef40451`) |
| 10 | leo4-plugin   | `idlToLeanType` renders nominal generic application                | no             | committed 2026-05-20 (`ef40451`, ASCII-positional binders in `a8556a8`) |
| 11 | leo4-plugin   | Admit-set guard against HK type-vars (LEO4-DESIGN §4.2 check #5)   | yes (semantic) | landed 2026-05-20 (AdmitSet.lean's user-inductive enumeration now skips `iv.numParams > 0` heads) |
| 12 | leo4-plugin   | Variant case with non-Self payload (W7-2d-iii)                     | no             | F-step minimum viable landed 2026-05-20: 0-field / all-Self / 1-field (Self ‖ scalar ‖ string) supported, per-instantiation helper emit; multi-field mixed / composite-payload variants still stub |

"Status" legend:
- **open** — nothing blocks landing it; just hasn't been prioritised.
- **open, blocking #4 demo** — must land before a sample-level
  generic-record fixture round-trips through cross-impl.
- **deferred** — decision recorded, implementation parked until a
  consuming phase / external need arrives.
- **landed YYYY-MM-DD** — code in tree; may or may not be committed yet.
- **Phase N** — fix tied to a leo4 roadmap phase entry gate.

## Dependency graph: what blocks the "generic record / variant
wire-up end-to-end" milestone

```text
                        [#4 substitute helper]    landed
                                │
                                ▼
              [#8 deriving generic inductive]    committed ef40451+
                                │
                                ▼
         [#9 walkUserDecl generic-aware]         committed ef40451
                                │
                                ▼
       [#10 idlToLeanType generic apply]         committed ef40451
                                │
              ┌─────────────────┴───────────────────┐
              ▼                                     ▼
[#6 parser non-ASCII idents]           [#7 render generic_params header]
 (Rust schema-idl)                      (leo4-plugin Emit.lean)
              │                                     │
              └────────────────┬────────────────────┘
                               ▼
        [sample monomorphic-instance fixture (Pair u64 u32)] ▶ wire-up ✓

  Separate sub-track (generic export at the boundary, not monomorphic
  use of a generic record):
        [#11 admit-set HK guard]   →  blocks generic `def f<α> …` exports
        [#12 variant non-Self]     →  blocks generic Either-style payloads

  Far downstream:
        [#1 UserDecl::Flags] [#2 effect] [#3 ConstraintExpr<Atom>] [#5 mutual]
        — none of these on the generic-record critical path.
```

Two side-tracks branch from the same trunk:

- **Monomorphic-instance fixture** (`Pair u64 u32` exported by a
  non-generic function such as `pairFstU64U32`). Needs #6 + #7
  before cross-impl byte-identical holds.
- **Generic-export fixture** (`def pairFst<α β>(…) : α`). Needs the
  above plus #11 (kind-mandatory constraints) and #12 (variant
  non-Self payloads) for the full `Either α β` case.

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

### Status (2026-05-20, post-audit)

- **schema-idl side: landed (committed in `5b34aaf`).**
  `crates/schema-idl/src/subst.rs` exports `substitute(ty, env)`,
  `instantiate_record(decl, args)`, `instantiate_variant(decl, args)`,
  with 11 unit tests. leo4-idl re-exports them.
- **Plugin adoption: landed (committed in `6f15756`).**
  `Subst.substIDL` / `Subst.mkEnv` in AdmitSet.lean; the `handlerFor`
  bail on non-empty generics replaced with proper `mkEnv + substIDL`
  walks on `.record / .resource / .variant`.
- **`deriving LeanMarshal` generic-inductive support: landed in
  working tree, not yet committed.** `lake/Leo4/Leo4/Deriving.lean`
  now generates `instance [Leo4.LeanMarshal α] [Leo4.LeanMarshal β]
  : Leo4.LeanMarshal (Pair α β) where …` and the matching generic
  `partial def`s for variants. (See ledger item #8.)
- **Plugin `walkUserDecl` generic-aware: landed in working tree,
  not yet committed.** AdmitSet.lean now extracts type-param binder
  names and threads an FVar→placeholder substitution into
  `exprToIDLSubst`. `UserDecl.record/variant/resource` carry their
  generics arrays. (See ledger item #9.)
- **`idlToLeanType` generic application form: landed in working
  tree, not yet committed.** Mangling.lean's `idlToLeanType` now
  emits `(Sample.Pair α β)` form for `Record { fqn, args }` with
  non-empty args. (See ledger item #10.)
- **Still blocking the end-to-end fixture:**
  - Item #6 — schema-idl parser rejects non-ASCII identifiers (`α`,
    `β`). Plugin emits binder names verbatim from Lean source, so a
    `structure Pair (α β : Type)` lands `α / β` into the schema and
    leo4c chokes on byte 113 of the round-tripped form.
  - Item #7 — `render::user_decl_to_idl` doesn't print
    `generic_params` on nominal-decl headers, so even after #6 lands,
    `record Sample.Pair { fst: α, snd: β }` reads as fields with
    dangling type-var references and the resolver can't pin them.
  - Items #11 / #12 only matter for generic `@[leo4_export]` at the
    boundary (admit-set HK guard + variant non-Self payload). The
    monomorphic-instance fixture (`pairFstU64U32`) doesn't need them.

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

## Priority ordering (post-2026-05-20 audit)

| Rank | Items | Why this rank |
|------|-------|---------------|
| **A**  | (#4 #8 #9 #10 landed) — commit pending | First, finalize what's in-tree. #8/#9/#10 are buildable, do not break cross-impl (sample reverted to the W7-2d baseline), and are needed regardless of whether the demo lands soon |
| **A+** | #6 parser non-ASCII identifiers + binder-name normaliser | One coordinated change: (a) plugin emits ASCII-safe generic names (`T0/T1`) instead of forwarding `α/β` verbatim, or (b) schema-idl parser accepts a wider identifier alphabet. Pick (a) — keeps the SPEC unchanged and the wire format pure ASCII |
| **A++** | #7 nominal-decl `generic_params` render | leo4 plugin `Emit.lean` prints `record Sample.Pair<T0, T1> { … }`. Rust schema-idl parser already accepts the production; the round-trip closes |
| **B**  | Sample fixture re-introduction + cross-impl byte-identical check | landed 2026-05-20. `Pair α β` (record) and `Either α β` (variant) re-introduced; `pairFstU64U32` / `pairSndU64U32` wire-up; `eitherTaggedU64String` remains stub pending #12. Cross-impl 58 byte-identical, schema_hash `4apuhe7gzvtzs` |
| **C**  | #2 `FuncDecl.effect` field (D-i) | Pre-stage for Phase 7 async; small AST work; land before Phase 7 entry |
| **D**  | #1 `UserDecl::Flags` variant | Small isolated PR, roundtrip correctness |
| **E**  | #11 admit-set HK guard | Generic-`@[leo4_export]` boundary path; touches LEO4-DESIGN check #5; mid-size plugin work. Needed for generic-export fixtures (not for monomorphic-instance ones) |
| **F**  | #12 variant non-Self payload (W7-2d-iii) | Plugin variantHandler / helper emission generalisation; needed for `Either`-style payloads |
| **G**  | #3 `ConstraintExpr<Atom>` | Largest churn — Schema<A> parametric across crate. Wait for first atom-set consumer |
| **H**  | #5 mutual recursion | Phase 6 owns entry decision |

The A/A+/A++/B group is the **generic-record critical path**: items
should ship together in a single commit (or two close ones) because
breaking any link in the chain leaves the working tree with
non-buildable sample fixtures. The audit revert (sample fixture
removed) was done precisely so the chain can be commited piecewise
without an interim red state.

## Decision log

| Date       | ID   | Decision                                                                                  |
|------------|------|-------------------------------------------------------------------------------------------|
| 2026-05-19 | D-i  | future/stream are function-level effects (`FuncDecl.effect`), not `IDLType` variants      |
| 2026-05-19 | D-ii | constraint sublanguage uses a typed AST + pluggable atom registry in schema-idl           |

(For leo4-side decisions see `LEO4-DESIGN.md` §2 "Decisions" table.)
