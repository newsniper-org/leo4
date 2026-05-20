-- Leo4.Deriving — `deriving LeanMarshal` handler for structures and inductives.
--
-- Synthesises `instance : LeanMarshal T where canonicalEncode := …; canonicalDecode := …`
-- for any `structure` or `inductive` the user tags. The handler covers three
-- shapes (LEO4-DESIGN.md §10.1):
--
--   • structure (single-ctor inductive) — record encoding, fields in declaration order
--   • all-nullary inductive — IDL enum encoding (u32 discriminator only)
--   • mixed inductive — IDL variant encoding (u32 discriminator + per-case payload)
--
-- As of 2026-05-20 generic structures / inductives ARE supported: the
-- synthesised instance carries one `{α : Type}` implicit binder and one
-- `[Leo4.LeanMarshal α]` instance binder per declared type parameter,
-- and the encode / decode bodies leave field-level marshalling to
-- instance synthesis (which picks up the per-parameter `LeanMarshal`
-- instances naturally).
--
-- Implementation note: we synthesise the instance command as a String and
-- feed it to `Parser.runParserCategory`. This sidesteps `\`(…)`-quotation
-- category-resolution problems we hit when antiquoting term-level idents
-- into command-level decls; the generated source is plain Lean and remains
-- inspectable when we trace `Elab.Deriving.Leo4`.

import Lean
import Leo4.Marshal

namespace Leo4.Deriving

open Lean Elab Command Meta

/-! ## Helpers -/

private def isStructureLike (env : Environment) (declName : Name) : Bool :=
  match env.find? declName with
  | some (.inductInfo iv) => iv.ctors.length == 1
  | _ => false

private def isAllNullary (env : Environment) (indVal : InductiveVal) : Bool :=
  indVal.ctors.all fun cname =>
    match env.find? cname with
    | some (.ctorInfo cv) => cv.numFields == 0
    | _ => false

private def hasTypeParams (indVal : InductiveVal) : Bool :=
  indVal.numParams > 0

/-! ## Generic-binder rendering

For an inductive with `numParams = 0` the head is just `qname` and no
binders are needed. For `numParams ≥ 1` we render:

  - `head`         — `"(Foo α β)"`, applied with the type-param names
  - `instBinders`  — `"[Leo4.LeanMarshal α] [Leo4.LeanMarshal β] "`
  - `defBinders`   — `"{α : Type} {β : Type} [Leo4.LeanMarshal α] [Leo4.LeanMarshal β] "`

`instBinders` is the form that goes on an `instance` declaration;
`defBinders` is the form that goes on a `partial def` (which needs
explicit implicit `Type` binders since there is no surrounding
`instance` to bring them in). Both trail with a single space when
non-empty so the surrounding format strings don't have to special-case
the no-generics case. -/
private def renderGenericFragments
    (qname : String) (params : Array Name) : String × String × String :=
  let lb : String := "{"
  let rb : String := "}"
  let names : List String := params.toList.map (·.toString)
  let head :=
    if names.isEmpty then qname
    else "(" ++ qname ++ " " ++ String.intercalate " " names ++ ")"
  let instBinders :=
    if names.isEmpty then ""
    else
      String.intercalate " "
        (names.map fun n => s!"[Leo4.LeanMarshal {n}]") ++ " "
  let defBinders :=
    if names.isEmpty then ""
    else
      let impl := String.intercalate " " (names.map fun n => lb ++ n ++ " : Type" ++ rb)
      let inst := String.intercalate " " (names.map fun n => s!"[Leo4.LeanMarshal {n}]")
      impl ++ " " ++ inst ++ " "
  (head, instBinders, defBinders)

/-- Extract the user-facing names of the inductive's type parameters
(`α`, `β`, …) by reducing `indVal.type`'s leading `Π` binders. -/
private def getParamBinders (indVal : InductiveVal) : MetaM (Array Name) :=
  Meta.forallTelescopeReducing indVal.type fun args _ => do
    let params := args.extract 0 indVal.numParams
    params.mapM fun fv => fv.fvarId!.getUserName

private def runSyntheticCommand (src : String) : CommandElabM Unit := do
  let env ← getEnv
  match Parser.runParserCategory env `command src "<leo4-derive>" with
  | .error e => throwError s!"deriving LeanMarshal: synthesised command failed to parse:\n{e}\n--- source ---\n{src}"
  | .ok stx => elabCommand stx

/-- Quote a `Name` as its fully-qualified Lean source form (e.g. `Sample.Point`),
prefixed with `_root_.` so the synthesised command is robust against the
ambient `namespace`. -/
private def qualified (n : Name) : String :=
  "_root_." ++ n.toString

/-! ## Structure (single-ctor) -/

private def mkStructureInstance (declName : Name) (indVal : InductiveVal)
    : CommandElabM Unit := do
  let env := (← getEnv)
  let fields := getStructureFields env declName
  let qname := qualified declName
  let params ← liftTermElabM (getParamBinders indVal)
  let (head, instBinders, _) := renderGenericFragments qname params
  -- Encode body.
  let mut encLets : String := ""
  for f in fields do
    encLets := encLets ++ s!"    let buf := Leo4.LeanMarshal.canonicalEncode v.{f.toString} buf\n"
  -- Decode body.
  let mut decLets : String := ""
  for f in fields do
    decLets := decLets ++ s!"    let ({f.toString}, off) ← Leo4.LeanMarshal.canonicalDecode buf off\n"
  let structFields :=
    String.intercalate ", " (fields.toList.map fun f => s!"{f.toString} := {f.toString}")
  let src : String := s!"\
instance {instBinders}: Leo4.LeanMarshal {head} where
  canonicalEncode v buf := Id.run do
{encLets}    return buf
  canonicalDecode buf off := do
{decLets}    return (\{ {structFields} }, off)
"
  runSyntheticCommand src

/-! ## All-nullary inductive (enum) -/

private def mkEnumInstance (declName : Name) (indVal : InductiveVal) : CommandElabM Unit := do
  let qname := qualified declName
  let ctors := indVal.ctors.toArray
  let params ← liftTermElabM (getParamBinders indVal)
  let (head, instBinders, _) := renderGenericFragments qname params
  -- Encode: match arm per ctor; tag is i.toUInt32.
  let mut encArms : String := ""
  for i in [0:ctors.size] do
    let cq := qualified ctors[i]!
    encArms := encArms ++ s!"    | {cq} => ({i} : UInt32)\n"
  -- Decode: match on UInt32 tag.
  let mut decArms : String := ""
  for i in [0:ctors.size] do
    let cq := qualified ctors[i]!
    decArms := decArms ++ s!"    | {i} => return ({cq}, off)\n"
  decArms := decArms ++ s!"    | t => throw (Leo4.LeanError.mk' Leo4.LeanError.decodeError s!\"{declName.toString}: invalid tag \{t.toNat}\")\n"
  let src : String := s!"\
instance {instBinders}: Leo4.LeanMarshal {head} where
  canonicalEncode v buf :=
    Leo4.LeanMarshal.canonicalEncode (T := UInt32) (match v with
{encArms}    ) buf
  canonicalDecode buf off := do
    let (tag, off) ← Leo4.LeanMarshal.canonicalDecode (T := UInt32) buf off
    match tag with
{decArms}"
  runSyntheticCommand src

/-! ## Mixed inductive (variant) -/

private def mkVariantInstance (declName : Name) (indVal : InductiveVal) : CommandElabM Unit := do
  let env := (← getEnv)
  let qname := qualified declName
  let ctors := indVal.ctors.toArray
  let encFn := s!"{declName.toString}._leo4_encode"
  let decFn := s!"{declName.toString}._leo4_decode"
  let params ← liftTermElabM (getParamBinders indVal)
  let (head, instBinders, defBinders) := renderGenericFragments qname params
  -- Encode arms.
  let mut encArms : String := ""
  let mut decArms : String := ""
  for i in [0:ctors.size] do
    let cname := ctors[i]!
    let cq    := qualified cname
    let some (.ctorInfo cv) := env.find? cname
      | throwError s!"deriving LeanMarshal: ctor info missing for {cname}"
    -- Per-field "is the type the enclosing inductive itself?" mask.
    -- Self-typed fields call the partial-def aux directly so that
    -- instance synthesis doesn't recurse on the unfinished instance.
    let selfMask : Array Bool ← liftTermElabM <|
      Meta.forallTelescopeReducing cv.type fun args _ => do
        let fieldArgs := args.extract indVal.numParams args.size
        fieldArgs.mapM fun a => do
          let argTy ← Meta.inferType a
          return argTy.getAppFn.isConstOf declName
    let argNames : List String :=
      (List.range cv.numFields).map fun k => s!"a{k}"
    let pat :=
      if argNames.isEmpty then cq
      else cq ++ " " ++ String.intercalate " " argNames
    let mut encBody : String :=
      s!"      let buf := Leo4.LeanMarshal.canonicalEncode (T := UInt32) ({i} : UInt32) buf\n"
    for j in [0:argNames.length] do
      let an := argNames[j]!
      if selfMask[j]! then
        encBody := encBody ++ s!"      let buf := {encFn} {an} buf\n"
      else
        encBody := encBody ++ s!"      let buf := Leo4.LeanMarshal.canonicalEncode {an} buf\n"
    encArms := encArms ++
      s!"    | {pat} => Id.run do\n{encBody}      return buf\n"
    let mut decBody : String := ""
    for j in [0:argNames.length] do
      let an := argNames[j]!
      if selfMask[j]! then
        decBody := decBody ++ s!"      let ({an}, off) ← {decFn} buf off\n"
      else
        decBody := decBody ++ s!"      let ({an}, off) ← Leo4.LeanMarshal.canonicalDecode buf off\n"
    let ret :=
      if argNames.isEmpty then cq
      else cq ++ " " ++ String.intercalate " " argNames
    decArms := decArms ++
      s!"    | {i} => do\n{decBody}      return ({ret}, off)\n"
  decArms := decArms ++
    s!"    | t => throw (Leo4.LeanError.mk' Leo4.LeanError.decodeError s!\"{declName.toString}: invalid tag \{t.toNat}\")\n"

  runSyntheticCommand s!"\
partial def {encFn} {defBinders}(v : {head}) (buf : ByteArray) : ByteArray :=
  match v with
{encArms}"
  runSyntheticCommand s!"\
partial def {decFn} {defBinders}(buf : ByteArray) (off : Nat) :
    Except Leo4.LeanError ({head} × Nat) := do
  let (tag, off) ← Leo4.LeanMarshal.canonicalDecode (T := UInt32) buf off
  match tag with
{decArms}"
  runSyntheticCommand s!"\
instance {instBinders}: Leo4.LeanMarshal {head} where
  canonicalEncode := {encFn}
  canonicalDecode := {decFn}"

/-! ## Handler entry point -/

private def mkLeanMarshalHandler (declNames : Array Name) : CommandElabM Bool := do
  let env ← getEnv
  for declName in declNames do
    let some (.inductInfo indVal) := env.find? declName | return false
    if isStructureLike env declName then
      mkStructureInstance declName indVal
    else if isAllNullary env indVal then
      mkEnumInstance declName indVal
    else
      mkVariantInstance declName indVal
  return true

initialize
  registerDerivingHandler ``Leo4.LeanMarshal mkLeanMarshalHandler

end Leo4.Deriving
