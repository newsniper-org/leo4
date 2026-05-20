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

/-- One variant member's elaborated rendering pieces. The `header`
strings cover the rendered Lean source for `(<head> : ByteArray …)`
parts of the synthesised partial defs; `encArms` / `decArms` are the
per-ctor match arms. Used by both the standalone variant emitter
and the Phase 6 mutual-cluster emitter. -/
private structure VariantPieces where
  encFn       : String
  decFn       : String
  head        : String
  instBinders : String
  defBinders  : String
  encArms     : String
  decArms     : String
  declName    : Name

/-- Build a `VariantPieces` for `declName` (a multi-ctor inductive),
routing every cluster-peer reference inside payload fields to a
direct `<peer>._leo4_encode/decode` call. `peers` lists every peer
declaration (including `declName` itself). For a non-mutual variant
this is just `#[declName]`. -/
private def buildVariantPieces
    (peers : Array Name) (declName : Name) (indVal : InductiveVal)
    : CommandElabM VariantPieces := do
  let env := (← getEnv)
  let qname := qualified declName
  let ctors := indVal.ctors.toArray
  let encFn := s!"{declName.toString}._leo4_encode"
  let decFn := s!"{declName.toString}._leo4_decode"
  let params ← liftTermElabM (getParamBinders indVal)
  let (head, instBinders, defBinders) := renderGenericFragments qname params
  -- Encode / decode arms.
  let mut encArms : String := ""
  let mut decArms : String := ""
  for i in [0:ctors.size] do
    let cname := ctors[i]!
    let cq    := qualified cname
    let some (.ctorInfo cv) := env.find? cname
      | throwError s!"deriving LeanMarshal: ctor info missing for {cname}"
    -- Per-field index in `peers` (some idx) or none (non-cluster).
    -- Cluster fields route directly to `<peer>._leo4_encode/decode`
    -- so instance synthesis doesn't recurse on the unfinished
    -- instances. The `declName == peers[i]` case is just the
    -- v0-style self-recursion fast path; cross-decl peers use the
    -- same mechanism, lifted to the whole `iv.all` cluster.
    let crossIdx : Array (Option Nat) ← liftTermElabM <|
      Meta.forallTelescopeReducing cv.type fun args _ => do
        let fieldArgs := args.extract indVal.numParams args.size
        fieldArgs.mapM fun a => do
          let argTy ← Meta.inferType a
          let head := argTy.getAppFn
          return peers.findIdx? (fun p => head.isConstOf p)
    let argNames : List String :=
      (List.range cv.numFields).map fun k => s!"a{k}"
    let pat :=
      if argNames.isEmpty then cq
      else cq ++ " " ++ String.intercalate " " argNames
    let mut encBody : String :=
      s!"      let buf := Leo4.LeanMarshal.canonicalEncode (T := UInt32) ({i} : UInt32) buf\n"
    for j in [0:argNames.length] do
      let an := argNames[j]!
      match crossIdx[j]! with
      | some idx =>
        let peer := peers[idx]!
        encBody := encBody ++ s!"      let buf := {peer.toString}._leo4_encode {an} buf\n"
      | none =>
        encBody := encBody ++ s!"      let buf := Leo4.LeanMarshal.canonicalEncode {an} buf\n"
    encArms := encArms ++
      s!"    | {pat} => Id.run do\n{encBody}      return buf\n"
    let mut decBody : String := ""
    for j in [0:argNames.length] do
      let an := argNames[j]!
      match crossIdx[j]! with
      | some idx =>
        let peer := peers[idx]!
        decBody := decBody ++ s!"      let ({an}, off) ← {peer.toString}._leo4_decode buf off\n"
      | none =>
        decBody := decBody ++ s!"      let ({an}, off) ← Leo4.LeanMarshal.canonicalDecode buf off\n"
    let ret :=
      if argNames.isEmpty then cq
      else cq ++ " " ++ String.intercalate " " argNames
    decArms := decArms ++
      s!"    | {i} => do\n{decBody}      return ({ret}, off)\n"
  decArms := decArms ++
    s!"    | t => throw (Leo4.LeanError.mk' Leo4.LeanError.decodeError s!\"{declName.toString}: invalid tag \{t.toNat}\")\n"
  return {
    encFn, decFn, head, instBinders, defBinders, encArms, decArms, declName
  }

/-- Render `partial def {encFn}` for one variant member. Returns a
self-contained Lean command string. -/
private def renderEncodePartialDef (p : VariantPieces) : String :=
  s!"partial def {p.encFn} {p.defBinders}(v : {p.head}) (buf : ByteArray) : ByteArray :=\n" ++
  s!"  match v with\n{p.encArms}"

/-- Render `partial def {decFn}` for one variant member. Self-contained
Lean command string. -/
private def renderDecodePartialDef (p : VariantPieces) : String :=
  s!"partial def {p.decFn} {p.defBinders}(buf : ByteArray) (off : Nat) :\n" ++
  s!"    Except Leo4.LeanError ({p.head} × Nat) := do\n" ++
  s!"  let (tag, off) ← Leo4.LeanMarshal.canonicalDecode (T := UInt32) buf off\n" ++
  s!"  match tag with\n{p.decArms}"

private def mkVariantInstance (declName : Name) (indVal : InductiveVal) : CommandElabM Unit := do
  let p ← buildVariantPieces #[declName] declName indVal
  runSyntheticCommand (renderEncodePartialDef p)
  runSyntheticCommand (renderDecodePartialDef p)
  runSyntheticCommand s!"\
instance {p.instBinders}: Leo4.LeanMarshal {p.head} where
  canonicalEncode := {p.encFn}
  canonicalDecode := {p.decFn}"

/-- Phase 6 mutual-cluster deriving handler. Emits one `mutual ... end`
block carrying every member's `partial def _leo4_encode / _leo4_decode`
pair, then one `instance` per member. Peers in the same `iv.all`
group cross-call each other's partial defs directly, breaking the
typeclass-instance dependency cycle. Variant members only — a
cluster containing a record / enum member falls back to the
per-decl loop (and will trip Lean's instance-synthesis if it tries
to forward-reference a peer's marshal instance, surfacing as a
deriving error rather than a silent wrong encoding). -/
private def mkMutualVariantCluster (members : Array Name) : CommandElabM Unit := do
  let env ← getEnv
  -- Gather pieces for every member. Each member must be a multi-ctor
  -- inductive with payload (variant shape).
  let mut pieces : Array VariantPieces := #[]
  for declName in members do
    let some (.inductInfo iv) := env.find? declName
      | throwError s!"deriving LeanMarshal (mutual cluster): not an inductive: {declName}"
    let p ← buildVariantPieces members declName iv
    pieces := pieces.push p
  -- One `mutual ... end` block carrying all 2N partial defs. Lean
  -- requires every member of a `mutual` block to be either a `def` or
  -- a `partial def`, and parses the whole block as one command.
  let body : String :=
    String.intercalate "\n"
      (pieces.toList.flatMap fun p =>
        [renderEncodePartialDef p, renderDecodePartialDef p])
  runSyntheticCommand s!"mutual\n{body}\nend"
  -- One instance per member.
  for p in pieces do
    runSyntheticCommand s!"\
instance {p.instBinders}: Leo4.LeanMarshal {p.head} where
  canonicalEncode := {p.encFn}
  canonicalDecode := {p.decFn}"

/-! ## Handler entry point -/

/-- True iff `declNames` is a genuine Lean mutual cluster: every member's
`iv.all` array matches the input set verbatim and has size > 1. A
`deriving instance` of independent inductives passes them in one
batch too, but their `iv.all` is each singleton, so they slip through
the per-decl branch below. -/
private def isMutualCluster (env : Environment) (declNames : Array Name) : Bool :=
  declNames.size > 1 &&
  declNames.all fun n =>
    match env.find? n with
    | some (.inductInfo iv) =>
      iv.all.length == declNames.size &&
      iv.all.all fun m => declNames.contains m
    | _ => false

/-- True iff every member of the cluster is a variant inductive (≥ 2
ctors, not all-nullary, not a structure). The Phase 6-3c emitter only
covers variant-only clusters; record / enum members in a mutual
cluster fall back to the per-decl loop (which will fail Lean
elaboration with a clear "missing instance" error rather than
silently emit a wrong encoding). -/
private def isAllVariantCluster (env : Environment) (declNames : Array Name) : Bool :=
  declNames.all fun n =>
    match env.find? n with
    | some (.inductInfo iv) =>
      iv.ctors.length > 1 ∧ !isStructureLike env n ∧ !isAllNullary env iv
    | _ => false

private def mkLeanMarshalHandler (declNames : Array Name) : CommandElabM Bool := do
  let env ← getEnv
  -- Phase 6 fast path: every member of a genuine `mutual ... end`
  -- inductive cluster gets a single `mutual { partial def … }` block
  -- so cross-decl encode / decode can recurse through direct function
  -- references rather than forward-undefined typeclass instances.
  if isMutualCluster env declNames && isAllVariantCluster env declNames then
    mkMutualVariantCluster declNames
    return true
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
