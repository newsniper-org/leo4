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
-- Generic structures / inductives are *not* handled by this pass; the handler
-- returns `false` and the user gets the standard "no LeanMarshal handler"
-- diagnostic. Generic deriving lands in Phase 4 once IDL-level generics are
-- wired end-to-end.
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

private def mkStructureInstance (declName : Name) : CommandElabM Unit := do
  let env := (← getEnv)
  let fields := getStructureFields env declName
  let qname := qualified declName
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
instance : Leo4.LeanMarshal {qname} where
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
instance : Leo4.LeanMarshal {qname} where
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
partial def {encFn} (v : {qname}) (buf : ByteArray) : ByteArray :=
  match v with
{encArms}"
  runSyntheticCommand s!"\
partial def {decFn} (buf : ByteArray) (off : Nat) :
    Except Leo4.LeanError ({qname} × Nat) := do
  let (tag, off) ← Leo4.LeanMarshal.canonicalDecode (T := UInt32) buf off
  match tag with
{decArms}"
  runSyntheticCommand s!"\
instance : Leo4.LeanMarshal {qname} where
  canonicalEncode := {encFn}
  canonicalDecode := {decFn}"

/-! ## Handler entry point -/

private def mkLeanMarshalHandler (declNames : Array Name) : CommandElabM Bool := do
  let env ← getEnv
  for declName in declNames do
    match env.find? declName with
    | some (.inductInfo iv) =>
        if hasTypeParams iv then
          logWarning s!"deriving LeanMarshal: generic inductive `{declName}` not yet supported"
          return false
    | _ => return false
  for declName in declNames do
    let some (.inductInfo indVal) := env.find? declName | continue
    if isStructureLike env declName then
      mkStructureInstance declName
    else if isAllNullary env indVal then
      mkEnumInstance declName indVal
    else
      mkVariantInstance declName indVal
  return true

initialize
  registerDerivingHandler ``Leo4.LeanMarshal mkLeanMarshalHandler

end Leo4.Deriving
