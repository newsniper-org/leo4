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
  deriving Repr, Inhabited, BEq

/-- A user-package nominal type declaration. The plugin discovers these by
walking the environment of `@[leo4_export]`-tagged decls and the
`LeanMarshal` / `LeanResource` instance database. -/
inductive UserDecl where
  | record   (fqn : String) (generics : Array Name) (fields : Array (Name × IDLType))
  | enumT    (fqn : String) (cases    : Array Name)
  | variant  (fqn : String) (generics : Array Name) (cases : Array (Name × Array IDLType))
  | resource (fqn : String) (generics : Array Name)
  deriving Repr, Inhabited

namespace UserDecl
def fqn : UserDecl → String
  | .record   f _ _ => f
  | .enumT    f _   => f
  | .variant  f _ _ => f
  | .resource f _   => f
end UserDecl

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
-/
partial def exprToIDLSubst
    (env : Environment) (enclosing : Option Name) (subst : FVarId → Option IDLType)
    : Expr → Option IDLType := fun e =>
  let head := e.getAppFn
  let args := e.getAppArgs
  match head, args.size with
  | .fvar fv, 0 => subst fv
  | .const ``List _,    1 => (exprToIDLSubst env enclosing subst args[0]!).map .list
  | .const ``Option _,  1 => (exprToIDLSubst env enclosing subst args[0]!).map .option
  | .const ``Except _,  2 => do
      let tIdl ← exprToIDLSubst env enclosing subst args[1]!
      let eIdl ← exprToIDLSubst env enclosing subst args[0]!
      pure (.result tIdl (some eIdl))
  | .const ``Prod _,    2 => do
      let a ← exprToIDLSubst env enclosing subst args[0]!
      let b ← exprToIDLSubst env enclosing subst args[1]!
      pure (.tuple #[a, b])
  | .const n _, _ =>
      -- Self-reference? Bare `Tree` = `.self`; `Tree α` = `.selfApp [α']`.
      if enclosing == some n then
        if args.isEmpty then some .self
        else
          let argsIdl? : Option (Array IDLType) :=
            args.foldlM (init := (#[] : Array IDLType)) fun acc a =>
              (exprToIDLSubst env enclosing subst a).map (acc.push ·)
          argsIdl?.map IDLType.selfApp
      -- Primitive?
      else if let some idl := leanNameToIDL n then
        some idl
      else if let some (.inductInfo iv) := env.find? n then
        -- User-defined inductive / structure: build the right shape by
        -- inspecting the inductive value.
        let argsIdl? : Option (Array IDLType) :=
          args.foldlM (init := (#[] : Array IDLType)) fun acc a =>
            (exprToIDLSubst env enclosing subst a).map (acc.push ·)
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

/-- Walk a user-defined `inductive`/`structure` `declName` and synthesise
its `UserDecl`. The plugin classifies the shape (record vs enum vs
variant vs resource) by what instances of `LeanMarshal`/`LeanResource`
the user has provided.

* `LeanResource` → `.resource` (opaque handle, no field walk required).
* Otherwise (`LeanMarshal`, or a `derive`d instance) we walk the
  inductive value: single-ctor → record; all-nullary multi-ctor →
  enumT; else → variant. Self-references become `IDLType.self`.

Returns `none` if `declName` is not an inductive, has no
LeanMarshal/LeanResource opt-in, or its fields contain a type we
cannot lower.

Recursion is bounded: we never re-walk a name already in `inFlight`. -/
partial def walkUserDecl
    (env : Environment) (inFlight : Std.HashSet Name) (declName : Name)
    : MetaM (Option UserDecl) := do
  -- Resource short-circuit.
  if hasLeanResourceInstance env declName then
    let some (.inductInfo iv) := env.find? declName | return none
    let genNames := iv.levelParams.toArray.map (fun _ => Name.anonymous)
    return some (.resource declName.toString genNames)
  -- Marshal required.
  unless hasLeanMarshalInstance env declName do return none
  let some (.inductInfo iv) := env.find? declName | return none
  if inFlight.contains declName then return none
  let _inFlight := inFlight.insert declName  -- not yet threaded through recursive field walks; W3-5
  let fqn := declName.toString
  let ctors := iv.ctors.toArray
  -- All-nullary inductive → enumT.
  let allNullary : Bool := ctors.all fun cname =>
    match env.find? cname with
    | some (.ctorInfo cv) => cv.numFields == 0
    | _ => false
  if ctors.size > 1 ∧ allNullary then
    -- enum
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
    let res ← Meta.forallTelescopeReducing cv.type fun args _body => do
      let fieldArgs := args.extract iv.numParams args.size
      let mut acc : Array (Name × IDLType) := #[]
      for i in [0:fieldArgs.size] do
        let arg := fieldArgs[i]!
        let argTy ← Meta.inferType arg
        match exprToIDLSubst env (some declName) (fun _ => none) argTy with
        | some idl => acc := acc.push (fieldNames[i]!, idl)
        | none => return none
      return some acc
    match res with
    | none => return none
    | some fields => return some (.record fqn #[] fields)
  -- Mixed multi-ctor → variant.
  let mut cases : Array (Name × Array IDLType) := #[]
  for cname in ctors do
    let some (.ctorInfo cv) := env.find? cname | return none
    let caseName := cname.componentsRev.head!
    let payload ← Meta.forallTelescopeReducing cv.type fun args _body => do
      let fieldArgs := args.extract iv.numParams args.size
      let mut acc : Array IDLType := #[]
      for arg in fieldArgs do
        let argTy ← Meta.inferType arg
        match exprToIDLSubst env (some declName) (fun _ => none) argTy with
        | some idl => acc := acc.push idl
        | none => return none
      return some acc
    match payload with
    | none => return none
    | some p => cases := cases.push (caseName, p)
  return some (.variant fqn #[] cases)

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
