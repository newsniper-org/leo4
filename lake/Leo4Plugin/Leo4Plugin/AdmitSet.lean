-- Leo4Plugin.AdmitSet — (α′) admit-set computation.
--
-- Implements LEO4-DESIGN.md §5 step 1 (admit-set per parameter) and step 3
-- (frontier expansion in lazy mode). Step 4 (validation) is partial — depth
-- bound is enforced; constraint sublanguage (∧/∨/¬) evaluation is deferred
-- to Phase 2.
--
-- For Week 2 the supported constraint sources are:
--   • `[Cls T]` instance-implicit binders in the function signature
--   • `scalar` builtin (admit-set is closed: u8…f64)
--
-- `@[leo4_specialize_when …]` is parsed and stored on the export (Step 3 of
-- Week 1) but **not yet consumed** here. When it is, it will OVERRIDE the
-- signature-derived constraint for the named generic.

import Lean
import Leo4

namespace Leo4Plugin

open Lean Lean.Meta

/-- IDL types we currently round-trip. Composite cases (record / variant /
flags / resource) carry an interned name; their structure is recovered on
demand from the Lean side. -/
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
  | record   (name : String) (args : Array IDLType)
  | variant  (name : String) (args : Array IDLType)
  | enumT    (name : String)
  | flagsT   (name : String)
  | resource (name : String)
  | io       (t : IDLType)
  deriving Repr, Inhabited, BEq

/-- Closed-set scalar admit-set (LEO4-DESIGN.md §4.2: `scalar` keyword). -/
def scalarAdmitSet : Array IDLType :=
  #[.u8, .u16, .u32, .u64, .i8, .i16, .i32, .i64, .f32, .f64]

/-- The default admit-set used for a generic parameter that carries **no**
constraint at all. Contains every leo4 primitive we currently round-trip
mechanically — scalars, `bool`, `char`, `string`, `bigint`, `bignat`.

Justification: LEO4-DESIGN.md §5 treats the admit-set as a finite enumeration
in lazy mode; "no constraint" therefore means "any IDL primitive". Composite
types (records, variants, resources, etc.) are excluded because their
admit-set is open-ended at the user-package level — the plugin can't list
them without more context. -/
def unboundedAdmitSet : Array IDLType :=
  #[.bool, .char, .string, .bigint, .bignat,
    .u8, .u16, .u32, .u64, .i8, .i16, .i32, .i64, .f32, .f64]

/-- Map a Lean type's head name to a leo4 IDL primitive, when the mapping is
unambiguous. -/
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
  -- Lean's `Float` is IEEE-754 double; map to f64.
  | ``Float   => some .f64
  | ``Bool    => some .bool
  | ``Char    => some .char
  | ``String  => some .string
  | ``Nat     => some .bignat
  | ``Int     => some .bigint
  | _         => none

/-- Walk through a Pi telescope and return the conclusion (the non-`forallE` tail). -/
partial def stripForall : Expr → Expr
  | .forallE _ _ body _ => stripForall body
  | e => e

/-- Build an `IDLType` from a Lean type `Expr`, when it consists only of types
we already understand. Returns `none` for free variables, custom records,
and anything else we cannot lower mechanically yet. -/
partial def exprToIDL : Expr → Option IDLType := fun e =>
  let head := e.getAppFn
  let args := e.getAppArgs
  match head, args.size with
  | .const ``List _,    1 => (exprToIDL args[0]!).map .list
  | .const ``Option _,  1 => (exprToIDL args[0]!).map .option
  | .const ``Except _,  2 => do
    -- Lean's `Except ε α` has error type first; result<T, E> reverses.
    let tIdl ← exprToIDL args[1]!
    let eIdl ← exprToIDL args[0]!
    return .result tIdl (some eIdl)
  | .const ``Prod _,    2 => do
    let a ← exprToIDL args[0]!
    let b ← exprToIDL args[1]!
    return .tuple #[a, b]
  | .const n _,         0 => leanNameToIDL n
  | _, _ => none

/-- A function's binders, classified for admit-set work. -/
structure ParsedSignature where
  /-- Type-parameter binders (`{T : Type}`); name and position. -/
  generics       : Array Name
  /-- `[Cls X]` instance-implicit binders, recorded as (genericName, className). -/
  classBinders   : Array (Name × Name)
  /-- Explicit and strict-implicit *value* binders, in source order, as a Lean
  `Expr`. We keep them as `Expr` so substitution still works after admit-set
  resolution. -/
  paramTypes     : Array Expr
  /-- Return type. -/
  returnType     : Expr
  deriving Inhabited

/-- Walk the signature of `info`, classifying binders. -/
def parseSignature (info : ConstantInfo) : MetaM ParsedSignature := do
  forallTelescopeReducing info.type fun args body => do
    let mut generics     : Array Name := #[]
    let mut classBinders : Array (Name × Name) := #[]
    let mut paramTypes   : Array Expr := #[]
    -- We need to look up each FVar's declaration to recover binderInfo / userName.
    for a in args do
      let ld ← a.fvarId!.getDecl
      let ty := ld.type
      if ld.binderInfo.isImplicit && ty.isSort then
        generics := generics.push ld.userName
      else if ld.binderInfo.isInstImplicit then
        let ch := ty.getAppFn
        let cargs := ty.getAppArgs
        if let .const clsName _ := ch then
          if cargs.size > 0 then
            -- Try to identify which generic this constraint is on.
            let argHead := cargs[0]!.getAppFn
            if let .fvar fv := argHead then
              let ld2 ← fv.getDecl
              classBinders := classBinders.push (ld2.userName, clsName)
        -- We deliberately do *not* push instance-implicit binders into
        -- paramTypes — those are constraint-resolved, not transport.
      else
        -- Explicit (or strict-implicit) value binder: transported as a parameter.
        paramTypes := paramTypes.push ty
    return { generics, classBinders, paramTypes, returnType := body }

/-- Enumerate global instances whose conclusion is `cls T` for some `T` whose
head maps via `leanNameToIDL`. Returns the de-duplicated admit-set in
`instanceExtension` traversal order; callers that need a canonical order
should sort by `Mangling.mangleType`. -/
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
      if let some idl := leanNameToIDL tName then
        unless acc.contains idl do
          acc := acc.push idl
  acc

/-- The admit-set for a single generic `T`, derived from the class binders
the signature gave us. Currently: intersection of admit-sets per class
constraint (`[Cls₁ T] [Cls₂ T]` ≡ `Cls₁ ∧ Cls₂`).

A generic with no class constraint gets the empty admit-set — the plugin
will warn and skip it. -/
def admitSetForGeneric
    (env : Environment) (sig : ParsedSignature) (g : Name) : Array IDLType := Id.run do
  let clsList := sig.classBinders.filterMap fun (gen, cls) =>
    if gen == g then some cls else none
  if clsList.isEmpty then
    return #[]
  -- Start with the first constraint, intersect with the rest.
  let mut acc := classAdmitSet env clsList[0]!
  for cls in clsList[1:] do
    let next := classAdmitSet env cls
    acc := acc.filter (next.contains ·)
  acc

/-- Cartesian product over a list of axes. The first axis varies slowest,
matching the worked-example order in `SPEC/mangling.md`. -/
partial def cartesian (axes : Array (Array IDLType)) : Array (Array IDLType) :=
  if axes.isEmpty then
    #[#[]]
  else
    let head := axes[0]!
    let rest := cartesian (axes.extract 1 axes.size)
    head.foldl (init := #[]) fun outer t =>
      rest.foldl (init := outer) fun inner row =>
        inner.push (#[t] ++ row)

/-- One concrete instantiation: a vector of types, one per generic. -/
abbrev Instantiation := Array IDLType

/-- All instantiations of `info`'s generics under the closed-world environment. -/
def instantiations (env : Environment) (sig : ParsedSignature) : Array Instantiation :=
  let axes := sig.generics.map (admitSetForGeneric env sig)
  cartesian axes

end Leo4Plugin
