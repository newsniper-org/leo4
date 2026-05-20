-- Leo4Plugin.AdmitSet — (α′) admit-set computation + IDL type model.
--
-- Implements LEO4-DESIGN.md §5 (admit-sets), §4 (IDL), and the v0 mapping
-- from Lean types to IDL types. User-defined `structure`/`inductive`s with
-- `LeanMarshal`/`LeanResource` instances are surfaced into the admit-set
-- alongside primitives.

import Lean
import Leo4

namespace Leo4Plugin

open Lean Lean.Meta

/-! ## IDL type model -/

/--
IDL types we currently round-trip. `record`/`variant`/`enum`/`resource`
carry the type's **FQN** (dotted Lean name, e.g. `Sample.Point`); mangling
replaces dots with underscores per `SPEC/mangling.md` §2.

`self` is the marker for self-recursive field types inside a record /
variant / resource declaration. It carries no further structure — the
encoder/decoder closes the cycle by recursive traversal.
-/
inductive IDLType where
  | u8 | u16 | u32 | u64
  | i8 | i16 | i32 | i64
  | f32 | f64
  | bool | char | string
  | bigint | bignat
  | list     (t : IDLType)
  | option   (t : IDLType)
  | result   (t : IDLType) (e : Option IDLType)
  | tuple    (ts : Array IDLType)
  | record   (fqn : String) (args : Array IDLType)
  | variant  (fqn : String) (args : Array IDLType)
  | enumT    (fqn : String)
  | flagsT   (fqn : String)
  | resource (fqn : String) (args : Array IDLType)
  | io       (t : IDLType)
  /-- Bare `Self` — identity-substitution sugar for `Self<X₁, …, Xₙ>`. -/
  | self
  /-- `Self<T₁, …, Tₙ>` — explicit substitution; mangles as `self_<…>_x`
  (SPEC/mangling.md §"Self and Self<…>"). -/
  | selfApp (args : Array IDLType)
  /-- `Cyc<i>` — Phase 6 cycle-breaker reference to the `i`-th member of
  the enclosing mutual group (SPEC/phase-6-mutual.md §2). `i` is 0-based
  and scoped to the immediately enclosing `mutual { … }` block; the
  resolver rejects `Cyc<i>` outside any group or with `i ≥ group_size`.
  Mangles as `c<i>c`. -/
  | cyc (i : UInt32)
  deriving Repr, Inhabited, BEq

/-- A user-package nominal type declaration. The plugin discovers these by
walking the environment of `@[leo4_export]`-tagged decls and the
`LeanMarshal` / `LeanResource` instance database. -/
inductive UserDecl where
  | record   (fqn : String) (generics : Array Name) (fields : Array (Name × IDLType))
  | enumT    (fqn : String) (cases    : Array Name)
  | variant  (fqn : String) (generics : Array Name) (cases : Array (Name × Array IDLType))
  | resource (fqn : String) (generics : Array Name)
  /-- Phase 6 cluster (SPEC/phase-6-mutual.md §1). Members reference
  each other via `IDLType.cyc i` (`i` = position in `members`); peers
  *outside* this group remain regular FQN references. Must contain
  ≥ 2 members; singleton groups should be ordinary `record` /
  `variant` / `enumT` decls with `Self` recursion. -/
  | mutual   (members : Array UserDecl)
  /-- Phase 8 step 2: a nominal type with a custom `LeanMarshal`
  instance whose fields the plugin can't (or shouldn't) lower —
  proof-carrying invariants (`Rat`'s `den_nz`, `reduced`), opaque
  wrapper structs, etc. The shim treats the wire format as opaque
  and routes through C-callable Lean helpers
  (`leo4_marshal_<T>_dec/enc`, emitted in step 2b). -/
  | externalMarshal (fqn : String) (generics : Array Name)
  deriving Repr, Inhabited

namespace UserDecl
/-- The single declaration's FQN. Returns `""` for a `mutual` group —
callers that need a flat list of leaves use `leaves` instead. -/
partial def fqn : UserDecl → String
  | .record   f _ _ => f
  | .enumT    f _   => f
  | .variant  f _ _ => f
  | .resource f _   => f
  | .externalMarshal f _ => f
  | .mutual   _     => ""

/-- Flatten a `UserDecl` to its leaves: a non-mutual decl yields a
singleton `#[d]`; a `mutual` group yields its members in source order.
Used by emit / handler-lookup passes that don't care about the
clustering, only about the per-decl shape. -/
partial def leaves : UserDecl → Array UserDecl
  | .mutual ms => ms
  | d          => #[d]
end UserDecl

/-! ## Type-parameter substitution

Mirrors `crates/schema-idl/src/subst.rs::substitute` byte-for-byte
semantically. Used by the shim emitter (`Main.lean::handlerFor`) to
walk a generic `UserDecl`'s field / case types after binding the
binder names to concrete `IDLType` arguments. -/
namespace Subst

/-- Substitute every binder reference in `t` per `env`. A bare nullary
`record fqn #[]` whose `fqn` matches a binder name resolves to the
bound type; everything else recurses through its component types. -/
partial def substIDL (env : Array (String × IDLType)) : IDLType → IDLType
  | .record fqn args =>
    if args.isEmpty then
      match env.find? (·.1 == fqn) with
      | some (_, t) => t
      | none        => .record fqn #[]
    else
      .record fqn (args.map (substIDL env))
  | .variant fqn args  => .variant fqn (args.map (substIDL env))
  | .resource fqn args => .resource fqn (args.map (substIDL env))
  | .list inner        => .list (substIDL env inner)
  | .option inner      => .option (substIDL env inner)
  | .result tOk tErr   =>
    .result (substIDL env tOk) (tErr.map (substIDL env))
  | .tuple ts          => .tuple (ts.map (substIDL env))
  | .io inner          => .io (substIDL env inner)
  | .selfApp args      => .selfApp (args.map (substIDL env))
  | other              => other

/-- Zip a declaration's `generics : Array Name` with concrete `args :
Array IDLType`. Returns `none` when arities disagree. -/
def mkEnv (generics : Array Name) (args : Array IDLType)
    : Option (Array (String × IDLType)) :=
  if generics.size != args.size then none
  else some <| (generics.zip args).map (fun (g, a) => (g.toString, a))

end Subst

/-! ## Admit-set seeds -/

/-- Closed-set scalar admit-set (LEO4-DESIGN.md §4.2: `scalar` keyword). -/
def scalarAdmitSet : Array IDLType :=
  #[.u8, .u16, .u32, .u64, .i8, .i16, .i32, .i64, .f32, .f64]

/-- The default admit-set used for a generic parameter that carries no
constraint at all. Contains every leo4 primitive we round-trip
mechanically. Composite user types are NOT included — they must be
opted in via an explicit `[Cls T]` constraint that enumerates them.
LEO4-DESIGN.md §5. -/
def unboundedAdmitSet : Array IDLType :=
  #[.bool, .char, .string, .bigint, .bignat,
    .u8, .u16, .u32, .u64, .i8, .i16, .i32, .i64, .f32, .f64]

/-! ## Lean Name → IDL primitive mapping -/

/-- Map a Lean type's head name to a leo4 IDL primitive. -/
def leanNameToIDL : Name → Option IDLType
  | ``UInt8   => some .u8
  | ``UInt16  => some .u16
  | ``UInt32  => some .u32
  | ``UInt64  => some .u64
  | ``Int8    => some .i8
  | ``Int16   => some .i16
  | ``Int32   => some .i32
  | ``Int64   => some .i64
  | ``Float32 => some .f32
  | ``Float   => some .f64
  | ``Bool    => some .bool
  | ``Char    => some .char
  | ``String  => some .string
  | ``Nat     => some .bignat
  | ``Int     => some .bigint
  | _         => none

/-! ## Lean Expr walking -/

partial def stripForall : Expr → Expr
  | .forallE _ _ body _ => stripForall body
  | e => e

/-- Detect whether `declName` has any `LeanMarshal _` instance. -/
def hasLeanMarshalInstance (env : Environment) (declName : Name) : Bool := Id.run do
  let st := Meta.instanceExtension.getState env
  for (n, _) in st.instanceNames do
    let some info := env.find? n | continue
    let concl := stripForall info.type
    unless concl.getAppFn.isConstOf ``Leo4.LeanMarshal do continue
    let cargs := concl.getAppArgs
    if cargs.isEmpty then continue
    if cargs[0]!.getAppFn.isConstOf declName then
      return true
  return false

/-- Detect whether `declName` has any `LeanResource _` instance (or carries
the `@[leo4_resource]` attribute). -/
def hasLeanResourceInstance (env : Environment) (declName : Name) : Bool := Id.run do
  if Leo4.leo4ResourceAttr.hasTag env declName then return true
  let st := Meta.instanceExtension.getState env
  for (n, _) in st.instanceNames do
    let some info := env.find? n | continue
    let concl := stripForall info.type
    unless concl.getAppFn.isConstOf ``Leo4.LeanResource do continue
    let cargs := concl.getAppArgs
    if cargs.isEmpty then continue
    if cargs[0]!.getAppFn.isConstOf declName then
      return true
  return false

/--
Convert a Lean type `Expr` to `IDLType`, substituting generic FVars via
`subst`. Built-in composites (`List`, `Option`, `Except`, `Prod`) are
handled here. *User-defined* `structure`/`inductive` references are
turned into bare `.record fqn #[]` / `.variant fqn #[]` / `.enumT fqn`
/ `.resource fqn #[]` references — the corresponding `UserDecl` (with
fields/cases) is discovered separately by `walkUserDecl` below.

`enclosing` is the FQN of the inductive currently being walked, if
any; an `Expr` that references the same declaration becomes
`IDLType.self` (Self-recursion marker, SPEC/idl-grammar.ebnf).

`mutualMembers`, when non-empty, lists the peers of the enclosing
declaration in source order (`iv.all` for the current walker); a
reference to `mutualMembers[i]` (other than `enclosing`, which still
goes through the `Self` short-circuit) becomes `IDLType.cyc i`
(Phase 6, SPEC/phase-6-mutual.md §2).
-/
partial def exprToIDLSubst
    (env : Environment) (enclosing : Option Name) (subst : FVarId → Option IDLType)
    (mutualMembers : Array Name := #[])
    : Expr → Option IDLType := fun e =>
  let head := e.getAppFn
  let args := e.getAppArgs
  match head, args.size with
  | .fvar fv, 0 => subst fv
  | .const ``List _,    1 =>
      (exprToIDLSubst env enclosing subst mutualMembers args[0]!).map .list
  | .const ``Option _,  1 =>
      (exprToIDLSubst env enclosing subst mutualMembers args[0]!).map .option
  | .const ``Except _,  2 => do
      let tIdl ← exprToIDLSubst env enclosing subst mutualMembers args[1]!
      let eIdl ← exprToIDLSubst env enclosing subst mutualMembers args[0]!
      pure (.result tIdl (some eIdl))
  | .const ``Prod _,    2 => do
      let a ← exprToIDLSubst env enclosing subst mutualMembers args[0]!
      let b ← exprToIDLSubst env enclosing subst mutualMembers args[1]!
      pure (.tuple #[a, b])
  | .const n _, _ =>
      -- Self-reference? Bare `Tree` = `.self`; `Tree α` = `.selfApp [α']`.
      if enclosing == some n then
        if args.isEmpty then some .self
        else
          let argsIdl? : Option (Array IDLType) :=
            args.foldlM (init := (#[] : Array IDLType)) fun acc a =>
              (exprToIDLSubst env enclosing subst mutualMembers a).map (acc.push ·)
          argsIdl?.map IDLType.selfApp
      -- Phase 6: peer in the current mutual group? Args (if any) are
      -- erased on the wire — Cyc<i> closes the cycle through index
      -- alone, exactly like `Self` does for the singleton case.
      else if mutualMembers.contains n then
        match mutualMembers.findIdx? (· == n) with
        | some idx => some (.cyc idx.toUInt32)
        | none     => none  -- unreachable given `contains` above
      -- Primitive?
      else if let some idl := leanNameToIDL n then
        some idl
      else if let some (.inductInfo iv) := env.find? n then
        -- User-defined inductive / structure: build the right shape by
        -- inspecting the inductive value.
        let argsIdl? : Option (Array IDLType) :=
          args.foldlM (init := (#[] : Array IDLType)) fun acc a =>
            (exprToIDLSubst env enclosing subst mutualMembers a).map (acc.push ·)
        argsIdl?.bind fun argsIdl =>
          if hasLeanResourceInstance env n then
            some (.resource n.toString argsIdl)
          else if iv.ctors.length == 1 then
            some (.record n.toString argsIdl)
          else if iv.ctors.all (fun cname =>
                   match env.find? cname with
                   | some (.ctorInfo cv) => cv.numFields == 0
                   | _ => false) then
            some (.enumT n.toString)
          else
            some (.variant n.toString argsIdl)
      else
        none
  | _, _ => none

/-! ## Walking structures and inductives -/

/-- Synthesise ASCII-safe positional names (`T0`, `T1`, …) for `iv`'s
first `numParams` type parameters. The Lean-source binders may be
Greek letters like `α / β`, which `schema-idl`'s grammar
(`SPEC/idl-grammar.ebnf` `ident`) rejects. Positional names keep the
wire form pure ASCII while preserving binder *count*; original
Lean-source names remain recoverable from the inductive's `type`
field if a downstream tool needs them. -/
private def paramBinderNames (iv : InductiveVal) : MetaM (Array Name) :=
  pure <| (Array.range iv.numParams).map fun i => Name.mkSimple s!"T{i}"

/-- Substitution function for `exprToIDLSubst` that recognises one of
`iv`'s declared type parameters by FVarId and lowers it to a nullary
`IDLType.record <param-name> #[]` — the same shape the IDL parser
emits for a bare named-type reference. `Subst.substIDL` then matches
that fqn against an instantiation environment to finish the job. -/
private def paramSubst (typeParams : Array Expr) (genNames : Array Name)
    : FVarId → Option IDLType := fun fvId =>
  match typeParams.findIdx? (fun fv => fv.fvarId! == fvId) with
  | some idx => some (.record genNames[idx]!.toString #[])
  | none     => none

/-- Walk a user-defined `inductive`/`structure` `declName` and synthesise
its `UserDecl`. The plugin classifies the shape (record vs enum vs
variant vs resource) by what instances of `LeanMarshal`/`LeanResource`
the user has provided.

* `LeanResource` → `.resource` (opaque handle, no field walk required).
* Otherwise (`LeanMarshal`, or a `derive`d instance) we walk the
  inductive value: single-ctor → record; all-nullary multi-ctor →
  enumT; else → variant. Self-references become `IDLType.self`.
  Generic parameters become a placeholder `IDLType.record <name> #[]`
  inside the field/case types and the declared binder names are
  recorded in `UserDecl.generics` so callers can substitute.

Returns `none` if `declName` is not an inductive, has no
LeanMarshal/LeanResource opt-in, or its fields contain a type we
cannot lower.

Recursion is bounded: we never re-walk a name already in `inFlight`.

When `mutualMembers` is non-empty, any peer reference (a member of the
same `iv.all` group, other than `declName` itself) lowers to
`IDLType.cyc i` instead of a plain FQN nominal — Phase 6
(`SPEC/phase-6-mutual.md` §5). Callers from outside a group leave the
parameter at its default. -/
partial def walkUserDecl
    (env : Environment) (inFlight : Std.HashSet Name) (declName : Name)
    (mutualMembers : Array Name := #[])
    : MetaM (Option UserDecl) := do
  -- Resource short-circuit.
  if hasLeanResourceInstance env declName then
    let some (.inductInfo iv) := env.find? declName | return none
    let genNames ← paramBinderNames iv
    return some (.resource declName.toString genNames)
  -- Marshal required.
  unless hasLeanMarshalInstance env declName do return none
  let some (.inductInfo iv) := env.find? declName | return none
  if inFlight.contains declName then return none
  let _inFlight := inFlight.insert declName  -- not yet threaded through recursive field walks; W3-5
  let fqn := declName.toString
  let ctors := iv.ctors.toArray
  -- All-nullary inductive → enumT. (Enum cases carry no payload, so
  -- the user-facing form is irrelevant — but we still preserve
  -- the binder count for round-trip / mangle parity if the user
  -- ever writes a phantom-generic `enum Foo (α : Type) { … }`.)
  let allNullary : Bool := ctors.all fun cname =>
    match env.find? cname with
    | some (.ctorInfo cv) => cv.numFields == 0
    | _ => false
  if ctors.size > 1 ∧ allNullary then
    let caseNames := ctors.map fun n => n.componentsRev.head!
    return some (.enumT fqn caseNames)
  if ctors.size == 1 then
    -- structure (record-like)
    let cname := ctors[0]!
    let some (.ctorInfo cv) := env.find? cname | return none
    let fieldNames : Array Name :=
      match Lean.getStructureInfo? env declName with
      | some info => info.fieldNames
      | none => (Array.range cv.numFields).map fun i => Name.mkSimple s!"_{i}"
    let genNames ← paramBinderNames iv
    let res ← Meta.forallTelescopeReducing cv.type fun args _body => do
      let typeParams := args.extract 0 iv.numParams
      let subst := paramSubst typeParams genNames
      let fieldArgs := args.extract iv.numParams args.size
      let mut acc : Array (Name × IDLType) := #[]
      for i in [0:fieldArgs.size] do
        let arg := fieldArgs[i]!
        let argTy ← Meta.inferType arg
        match exprToIDLSubst env (some declName) subst mutualMembers argTy with
        | some idl => acc := acc.push (fieldNames[i]!, idl)
        | none => return none
      return some acc
    match res with
    -- Phase 8 step 2 fallback: a structure with proof / unlowerable
    -- fields *but* a custom `LeanMarshal` instance becomes
    -- `externalMarshal`. The shim treats the wire format as opaque
    -- and routes through C-callable Lean helpers (Phase 8 step 2b).
    | none => return some (.externalMarshal fqn genNames)
    | some fields => return some (.record fqn genNames fields)
  -- Mixed multi-ctor → variant. Gather generic names from the first
  -- ctor (every ctor shares the same enclosing binders).
  let genNames ← paramBinderNames iv
  let mut cases : Array (Name × Array IDLType) := #[]
  for cname in ctors do
    let some (.ctorInfo cv) := env.find? cname | return none
    let caseName := cname.componentsRev.head!
    let payload ← Meta.forallTelescopeReducing cv.type fun args _body => do
      let typeParams := args.extract 0 iv.numParams
      let subst := paramSubst typeParams genNames
      let fieldArgs := args.extract iv.numParams args.size
      let mut acc : Array IDLType := #[]
      for arg in fieldArgs do
        let argTy ← Meta.inferType arg
        match exprToIDLSubst env (some declName) subst mutualMembers argTy with
        | some idl => acc := acc.push idl
        | none => return none
      return some acc
    match payload with
    -- Phase 8 step 2 fallback for variants too.
    | none => return some (.externalMarshal fqn genNames)
    | some p => cases := cases.push (caseName, p)
  return some (.variant fqn genNames cases)

/-- Walk a *mutual cluster* whose members are listed in `iv.all` order.
Each member becomes a regular `UserDecl` (record / enumT / variant)
with peer references inside it rewritten to `IDLType.cyc i`. The
whole cluster is wrapped in `UserDecl.mutual`. Returns `none` if any
member fails to walk (missing LeanMarshal, unlowerable field, …). -/
partial def walkMutualGroup
    (env : Environment) (inFlight : Std.HashSet Name) (members : Array Name)
    : MetaM (Option UserDecl) := do
  let mut acc : Array UserDecl := #[]
  for m in members do
    match ← walkUserDecl env inFlight m members with
    | some d => acc := acc.push d
    | none   => return none
  return some (.mutual acc)

/-! ## Class admit-set enumeration -/

/-- Lean's `List`/`Option`/`Except`/`Prod` are *builtin generics* in our
IDL (`list<T>` etc.), not nominal user types. Their full admit-set is
"every primitive wrapped in `list<…>`" which is unbounded; we exclude
them from the class admit-set on purpose. Callers who want
`ToString (List u32)` etc. must lift via an explicit `[LeanMarshal T]`
constraint on `T`. -/
private def isBuiltinGenericHead : Name → Bool
  | ``List => true  | ``Option => true
  | ``Except => true | ``Prod   => true
  | _ => false

/-- Enumerate global instances whose conclusion's head is `cls`. The
target type's head is mapped to an `IDLType`:
* primitives → `leanNameToIDL`
* user types → `.record`/`.variant`/`.enumT`/`.resource` by inductive
  shape (see `exprToIDLSubst`'s shape selection)
* builtin generics (`List`, `Option`, `Except`, `Prod`) → skipped here
  (see `isBuiltinGenericHead`).

Duplicates are de-duped by `BEq`. -/
def classAdmitSet (env : Environment) (cls : Name) : Array IDLType := Id.run do
  let st := Meta.instanceExtension.getState env
  let mut acc : Array IDLType := #[]
  for (n, _) in st.instanceNames do
    let some info := env.find? n | continue
    let concl := stripForall info.type
    let head  := concl.getAppFn
    unless head.isConstOf cls do continue
    let cargs := concl.getAppArgs
    if cargs.isEmpty then continue
    let tHead := cargs[0]!.getAppFn
    if let .const tName _ := tHead then
      if isBuiltinGenericHead tName then
        continue  -- list / option / etc. are not standalone admit-set members
      -- Primitive?
      if let some idl := leanNameToIDL tName then
        unless acc.contains idl do acc := acc.push idl
      else if hasLeanResourceInstance env tName then
        let idl : IDLType := .resource tName.toString #[]
        unless acc.contains idl do acc := acc.push idl
      else if hasLeanMarshalInstance env tName then
        -- Pick the right shape from the inductive value.
        match env.find? tName with
        | some (.inductInfo iv) =>
          -- LEO4-DESIGN §4.2 mandatory check #5: higher-kinded
          -- (`numParams > 0`) inductives are NOT type-instances on
          -- their own. Adding `IDLType.record fqn #[]` for a generic
          -- `Foo : Type → Type` would lower to a wrapper that treats
          -- `Foo` as `Type`, then the elaborator would see
          -- `(Foo : Type)` and reject the application. Such admit-set
          -- members must come from an explicit `oneof` constraint
          -- that pins concrete type arguments, not from this
          -- closed-world unconstrained enumeration. Skip them here.
          unless iv.numParams > 0 do
            let idl : IDLType :=
              if iv.ctors.length == 1 then .record tName.toString #[]
              else if iv.ctors.all (fun cname =>
                       match env.find? cname with
                       | some (.ctorInfo cv) => cv.numFields == 0
                       | _ => false) then .enumT tName.toString
              else .variant tName.toString #[]
            unless acc.contains idl do acc := acc.push idl
        | _ => pure ()
  acc

/-! ## Cartesian product (unchanged) -/

partial def cartesian (axes : Array (Array IDLType)) : Array (Array IDLType) :=
  if axes.isEmpty then
    #[#[]]
  else
    let head := axes[0]!
    let rest := cartesian (axes.extract 1 axes.size)
    head.foldl (init := #[]) fun outer t =>
      rest.foldl (init := outer) fun inner row =>
        inner.push (#[t] ++ row)

end Leo4Plugin
