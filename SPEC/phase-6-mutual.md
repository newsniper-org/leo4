# SPEC — Phase 6: Mutual Recursion Between Nominal Types

> Status: **normative**, drafted 2026-05-20. Activates with the Phase 6
> tracking commit; until then v0 `LEO4-DESIGN.md §4.3` rules apply
> (mutual recursion forbidden, users break cycles with `LeanResource`).

This file resolves the four design questions left open in
`ROADMAP.md` "Phase 6 — Mutual recursion between nominal types".
Each section names the chosen answer and the alternative considered.

## 0. Scope

A *mutual group* is a finite, non-empty set of two-or-more nominal
type declarations (`record` / `variant` / `enum` / `flags` /
`resource`) whose definitions reference each other directly. A
singleton group (one declaration self-referencing through `Self`) is
**not** a mutual group — it stays under the v0 `Self` machinery.

Mutual groups appear in three places:
- the IDL source (input to the parser),
- the canonical form of `<pkg>.leo4-schema` (input to the schema
  hash),
- the Lake plugin's `walkUserDecl` output (input to the runtime
  derivation handler).

This SPEC defines the wire-level and naming-level contracts that
binds all three.

## 1. IDL Syntax — explicit `mutual { … }` block

**Chosen:** the IDL grammar grows a `mutual_group` production:

```
mutual_group  = "mutual" , "{" , type_decl , { type_decl } , "}" ;
```

where each `type_decl` is one of the existing nominal decls.

A `mutual_group` may appear anywhere a `type_decl` may appear. Its
members share a `Cyc<i>` namespace (§2). The group is treated as a
single decl by the schema-hash input order (§3).

**Alternative considered:** implicit SCC analysis — the parser
accepts a flat list and the plugin computes strongly-connected
components. Rejected for two reasons: (a) the schema-hash boundary
becomes invisible (adding an apparently unrelated decl can silently
extend the group); (b) the Lean `mutual ... end` block has no
implicit counterpart on the IDL side, so authors would have to
re-derive the grouping from the dependency graph each time.

**Singleton requirement.** A `mutual_group` containing exactly one
declaration is a parse error. The user should drop the brackets and
keep the `Self` recursion.

## 2. Mangling — `Cyc<n>` cycle-breaker token

**Chosen:** every nominal reference *within* a mutual group is
mangled as `Cyc<i>`, where `i` is the 0-based index of the referenced
declaration within the group's source order. References *outside*
the group continue to mangle by FQN (per SPEC/mangling.md §2).

```
Cyc<i>  ↦  c<dec(i)>c            (* dec(i) = ASCII-decimal i, no leading zeros *)
```

The IDL canonical form prints the same token verbatim
(`Cyc<i>`). Outside callers reading the schema therefore see the
group's recursive structure without name-clashing with any specific
member's FQN.

**Self compatibility.** `Self` and `Self<…>` (SPEC/mangling.md §
"Self and Self<…>") remain valid *only* when the enclosing
declaration is in a singleton group (i.e. is not inside any
`mutual` block). When the enclosing declaration is in a multi-member
group, `Self` is a parse error — the user writes `Cyc<i>` to name
"this declaration" explicitly. Authors who want a single uniform
spelling may always write `Cyc<i>` even for the self-only case; the
group of size 1 normalises `Cyc<0>` → `Self` for canonical-form
output.

**Group-level reordering rotates the schema hash.** Reordering the
members of a `mutual` block flips every `Cyc<i>` reference's
identity, which changes the normalised IDL form, which rotates the
schema hash. Authors should treat member order as ABI-significant.

**Alternative considered:** keep references as full FQN of the
peer (no special token). Rejected: makes the mangled body of each
group member quadratic in `m × max_fqn_len`, and the schema hash
input would need to repeat each FQN once per cross-reference. The
`Cyc<i>` token captures the same information in O(log m) bytes per
reference.

## 3. Canonical ABI — group-shared depth counter

**Chosen:** each mutual group has a *single* `max_decode_depth`
counter, shared across every group member's decoder. The default
cap stays at the per-type v0 value (1024 frames); a per-group
override may be added in a later revision.

```
DecodeCtx {
    depth: u32,            // increments on every group-member entry
    max_depth: u32,        // group-level cap
    ...
}
```

A frame *outside* the group does not increment the counter; only
decode calls into a group member do.

**Alternative considered:** keep each declaration's own
`max_decode_depth`. Rejected: a malicious payload could bounce
between `Expr.decode` and `Stmt.decode` forever, incrementing
neither counter past its own cap. The shared counter trips after
1024 cross-decl frames regardless of which side initiates.

**Wire format unchanged.** Group-shared depth tracking is a *decoder*
contract; the bytes on the wire are identical to what a flat (non-
cyclic) `Cyc<i>`-free encoding of the same logical structure would
produce.

## 4. `deriving LeanMarshal` — one `mutual ... end` block per group

**Chosen:** the deriving handler accepts an `Array Name` where every
name belongs to the same mutual group (Lean's `deriving` already
passes the whole `mutual ... deriving` cluster as one array). It
synthesises:

1. One `mutual ... end` block containing one `partial def
   <Decl>._leo4_encode` and one `partial def
   <Decl>._leo4_decode` per group member, with cross-calls
   resolved by direct function reference (not via the
   `LeanMarshal` typeclass) to avoid instance-search recursion on
   the unfinished instance.
2. One `instance : Leo4.LeanMarshal <Decl>` per member, each
   pointing at the matching `_leo4_encode` / `_leo4_decode`.

The handler's existing variant code path (`mkVariantInstance`)
already emits `partial def ... encFn` / `partial def ... decFn` for
self-recursive variants; the mutual-group expansion is a `mutual
... end` wrapping of N such pairs.

**Singleton-group degenerate case.** For a group of size 1 the
`mutual ... end` wrapper is dropped (Lean disallows single-member
`mutual` blocks). The output reduces to the existing self-recursive
variant codegen.

## 5. Plugin walk

`Leo4Plugin.Main.walkUserDecl` will:

1. Detect a `mutual` block by looking for `Lean.ConstantInfo`
   entries that share the same `InductiveVal.all` array of size > 1.
2. Emit one IDL `mutual_group` whose members follow `InductiveVal.all`
   order — *the same order Lean uses internally*, which the schema
   hash freezes.
3. For each member, substitute its peers' references with the
   matching `Cyc<i>` token before lowering to `IDLType`.

`Lean.ConstantInfo.inductInfo.all` is the canonical group order;
authors do not get to renumber it without re-`mutual`-grouping in
source.

## 6. Examples

```
mutual {
    variant Expr {
        lit(u64),
        neg(Cyc<0>),
        plus(Cyc<0>, Cyc<0>),
        seq(Cyc<1>),
    };
    variant Stmt {
        nop,
        assign(u32, Cyc<0>),
        block(list<Cyc<1>>),
    };
}
```

- `Cyc<0>` inside `Expr` = `Expr` itself.
- `Cyc<0>` inside `Stmt` = the first decl in the group = `Expr`.
- `Cyc<1>` inside either decl = `Stmt`.
- Outside the group, a function param of type `Cyc<0>` is a parse
  error (the token is scoped to its enclosing group only).

## 7. Implementation phase ladder

This SPEC unblocks Phase 6 work in roughly four landings:

1. **schema-idl** — add `UserDecl::Mutual { members: Vec<UserDecl> }`
   (or equivalent), grow the parser / renderer / mangle / hash paths
   to handle `Cyc<i>`.
2. **leo4-abi** — group-shared `DecodeCtx.depth` counter on the Rust
   side; `LeanMarshal` impls compose without changing the wire
   format.
3. **lake/Leo4 + leo4-plugin** — `walkUserDecl` lifts
   `InductiveVal.all`, the deriving handler emits `mutual ... end`.
4. **examples/04-mutual-ast/** + `tests/mangling/cases/mutual.leo4` —
   the exit demo and the cross-impl harness.

Each landing rotates `tests/sample-lean`'s schema hash because the
canonical-form output gains a `mutual { … }` rendering even for
groups that didn't change in content; the regression check in
landing 4 confirms the rotation is intentional and identical
across both impls.
