-- Leo4Plugin.Main — plugin entry point.
--
-- Responsibility (LEO4-DESIGN.md §7, CLAUDE.md "How to Work With the Lake Plugin"):
--   1. importModules on the user package's compiled .olean files     ✓ Week 1
--   2. find every @[leo4_export] definition                          ✓ Week 1
--   3. read its constraints, compute admit-sets per generic          ✓ Week 2 (signature path)
--   4. emit IDL, mangling table, handshake                           ✓ Week 2 (handshake + mangling)
--   5. emit C shim + drive cc/leanc                                  Phase 3
--
-- The plugin is invoked as `lake exe leo4plugin <user-module> [<out-dir>] [<pkg>] [<iface>]`.
-- We do *not* hook Lake.Module.recBuildLean; see SPIKE-0-FINDINGS.md.

import Lean
import Leo4
import Leo4Plugin.AdmitSet
import Leo4Plugin.Mangling
import Leo4Plugin.Emit

open Lean Lean.Meta

namespace Leo4Plugin

/-- Plugin version string, surfaced in `<pkg>.leo4-handshake`. -/
def pluginVersion : String := "0.1.0"

/-! ## Reporting helpers -/

def msStr (ns : Nat) : String :=
  let f : Float := ns.toFloat / 1e6
  let i : Nat := (f * 10).toUInt64.toNat
  s!"{i / 10}.{i % 10}ms"

/-! ## Exporter walking -/

/-- Per-module ext.getModuleEntries walk; sub-ms even for large envs. -/
def gatherExports (env : Environment) : Array Name := Id.run do
  let ext := Leo4.leo4ExportAttr.ext
  let mut acc : Array Name := #[]
  for modIdx in [0 : env.allImportedModuleNames.size] do
    acc := acc ++ ext.getModuleEntries env modIdx
  let curMod := ext.getState (asyncDecl := .anonymous) env
  curMod.foldl (init := acc) fun a n => a.push n

/-! ## Kind detection -/

/--
True iff `e` is a *kind expression* — `Sort _` or a Pi telescope whose
every domain and conclusion is itself a kind.

  `Type`            → kind                (`isSort = true`)
  `Type → Type`     → kind                (Pi from kind to kind)
  `Nat`             → not a kind          (it's a value type)
  `{n : Nat} → Type` → not a kind         (value-indexed)

Used to distinguish type binders (`{T : Type}`, `{F : Type → Type}`)
from value binders (`{n : Nat}`). LEO4-DESIGN.md §5 enumerates the
admit-set rule for each binder class.
-/
partial def isKindExpr : Expr → Bool
  | .sort _              => true
  | .forallE _ d b _     => isKindExpr d ∧ isKindExpr b
  | _                    => false

/-- True iff the binder type denotes a *higher* kind, i.e. anything
strictly above `Sort _` (`Type → Type`, `Type → Type → Type`, …). -/
def isHigherKind : Expr → Bool
  | .forallE _ d b _     => isKindExpr d ∧ isKindExpr b
  | _                    => false

/-! ## Per-export analysis -/

/-- Result of analysing one `@[leo4_export]` decl. -/
structure ExportAnalysis where
  /-- The decl's name (e.g. `Sample.stringify`). -/
  declName     : Name
  /-- Last name component (`stringify`). -/
  fname        : String
  /-- Generic-parameter user names, in order. -/
  generics     : Array Name
  /-- Per-generic admit-set, in the same order as `generics`. -/
  admitSets    : Array (Array IDLType)
  /-- Per-class admit-sets we encountered, keyed by class name; used to
  populate `<pkg>.leo4-handshake.constraint_universe`. -/
  classAdmits  : Array (Name × Array IDLType)
  /-- `true` at index `i` ⇔ `generics[i]` is a phantom generic, i.e. it
  appears nowhere in the value parameter types or the return type. -/
  phantomMask  : Array Bool
  /-- Concrete instantiations: each carries
      (genericArgs, paramInfos, returnAsIDL).
  `genericArgs` has length = generics.size; phantom positions are `none`.
  `paramInfos` has length = number of value parameters; each entry pairs
  the substituted IDL encoding with the generic indices the parameter's
  template depended on. -/
  resolved     : Array (Array (Option IDLType) × Array Emit.ParamInfo × IDLType)
  /-- Diagnostic notes — printed by the plugin to stdout, not emitted to JSON. -/
  notes        : Array String
  deriving Inhabited

/-- Analyse one tagged decl. Returns `none` if the decl is missing from `env`. -/
def analyzeExport (n : Name) : MetaM (Option ExportAnalysis) := do
  let env ← getEnv
  let some info := env.find? n | return none
  forallTelescopeReducing info.type fun args body => do
    -- Classify binders. A binder counts as a type-level generic if its
    -- type is a *kind* (LEO4-DESIGN.md §5). That catches plain
    -- `{T : Type}` as well as higher-kind binders like
    -- `{F : Type → Type}`.
    let mut generics    : Array (FVarId × Name) := #[]
    let mut higherKinds : Array (FVarId × Name) := #[]
    let mut classes     : Array (Name × Name) := #[]
    let mut paramTypes  : Array Expr := #[]
    for a in args do
      let ld ← a.fvarId!.getDecl
      let ty := ld.type
      if ld.binderInfo.isImplicit ∧ isKindExpr ty then
        generics := generics.push (a.fvarId!, ld.userName)
        if isHigherKind ty then
          higherKinds := higherKinds.push (a.fvarId!, ld.userName)
      else if ld.binderInfo.isInstImplicit then
        let ch := ty.getAppFn
        let ca := ty.getAppArgs
        if let .const clsName _ := ch then
          if ca.size > 0 then
            let argHead := ca[0]!.getAppFn
            if let .fvar fv := argHead then
              let ld2 ← fv.getDecl
              classes := classes.push (ld2.userName, clsName)
      else
        paramTypes := paramTypes.push ty

    -- Reject unconstrained higher-kind generics.
    -- LEO4-DESIGN.md §5 + SPEC/mangling.md "Mandatory check 5".
    -- (The `@[leo4_specialize_when F : oneof {…}]` constraint isn't
    -- elaborated by this plugin yet — Phase 2+ — so for the moment we
    -- reject every HK generic regardless of attribute presence.)
    for (_, gname) in higherKinds do
      let msg :=
        "@[leo4_export] `" ++ n.toString ++
        "`: generic `" ++ gname.toString ++
        "` has higher kind, but the plugin rejects unconstrained higher-kind " ++
        "generics at the boundary (LEO4-DESIGN.md §5). Add an explicit " ++
        "@[leo4_specialize_when <param> : oneof { ... }] (Phase 2+) " ++
        "or refactor the export to be monomorphic."
      throwError msg
    -- Per-parameter generic dependencies, computed once on the *templates*
    -- (i.e. before substitution). Same vector applies to every instantiation.
    let genFvars : Array FVarId := generics.map Prod.fst
    let mut paramOrigins : Array (Array Nat) := #[]
    for ty in paramTypes do
      let st : Lean.CollectFVars.State := {}
      let st := Lean.collectFVars st ty
      let mut idxs : Array Nat := #[]
      for i in [0 : genFvars.size] do
        if st.fvarSet.contains genFvars[i]! then
          idxs := idxs.push i
      paramOrigins := paramOrigins.push idxs
    -- Phantom detection: a generic is *alive* iff its FVar appears in some
    -- parameter type OR in the return type. The return type is checked
    -- separately because `paramOrigins` only covers parameters.
    let retState : Lean.CollectFVars.State := Lean.collectFVars {} body
    let mut phantomMask : Array Bool := #[]
    for i in [0 : genFvars.size] do
      let usedInParams := paramOrigins.any (fun idxs => idxs.contains i)
      let usedInRet    := retState.fvarSet.contains genFvars[i]!
      phantomMask := phantomMask.push (¬ (usedInParams ∨ usedInRet))
    -- Admit-set computation: only for alive generics. Phantom axes are
    -- skipped (LEO4-DESIGN.md §5).
    let mut admitSets     : Array (Array IDLType) := #[]
    let mut classAdmits   : Array (Name × Array IDLType) := #[]
    let mut activeIndices : Array Nat := #[]
    for i in [0 : generics.size] do
      if phantomMask[i]! then continue
      let (_, gname) := generics[i]!
      let clsList := classes.filterMap fun (un, cls) =>
        if un == gname then some cls else none
      if clsList.isEmpty then
        admitSets := admitSets.push unboundedAdmitSet
      else
        let mut acc := classAdmitSet env clsList[0]!
        unless classAdmits.any (·.1 == clsList[0]!) do
          classAdmits := classAdmits.push (clsList[0]!, acc)
        for cls in clsList[1:] do
          let next := classAdmitSet env cls
          unless classAdmits.any (·.1 == cls) do
            classAdmits := classAdmits.push (cls, next)
          acc := acc.filter (next.contains ·)
        admitSets := admitSets.push acc
      activeIndices := activeIndices.push i
    -- If any alive generic has an empty admit-set, emit no instantiations
    -- (a class constraint with no satisfying instance is a real "no" answer).
    let combosActive : Array (Array IDLType) :=
      if admitSets.any (·.isEmpty) then #[]
      else cartesian admitSets
    -- Reconstruct full-length generic_args vectors (one slot per declared
    -- generic). Phantom positions get `none`; alive positions are filled
    -- from the active combo.
    let mut resolved : Array (Array (Option IDLType) × Array Emit.ParamInfo × IDLType) := #[]
    let mut notes    : Array String := #[]
    let placeholder : IDLType := .bool  -- value never observed: phantom FVars don't appear in any target.
    for activeCombo in combosActive do
      -- substMap[i] = type to plug in for generic i; placeholder for phantoms.
      let mut substMap : Array IDLType := Array.replicate generics.size placeholder
      for j in [0 : activeIndices.size] do
        substMap := substMap.set! activeIndices[j]! activeCombo[j]!
      -- Full generic_args, with `none` at phantom positions.
      let mut fullGenArgs : Array (Option IDLType) := Array.replicate generics.size none
      for j in [0 : activeIndices.size] do
        fullGenArgs := fullGenArgs.set! activeIndices[j]! (some activeCombo[j]!)
      -- FVar lookup closure used by exprToIDLSubst.
      let subst : FVarId → Option IDLType := fun fv =>
        Id.run do
          for i in [0 : generics.size] do
            if (generics[i]!).1 == fv then
              return some substMap[i]!
          return none
      let mut ok := true
      let mut paramInfos : Array Emit.ParamInfo := #[]
      for i in [0 : paramTypes.size] do
        let ty := paramTypes[i]!
        match exprToIDLSubst env none subst ty with
        | some idl =>
            paramInfos := paramInfos.push
              { encoded := idl, usesGenerics := paramOrigins[i]! }
        | none =>
            notes := notes.push s!"param type unsupported: {← Meta.ppExpr ty}"
            ok := false
      if !ok then continue
      match exprToIDLSubst env none subst body with
      | some retIDL => resolved := resolved.push (fullGenArgs, paramInfos, retIDL)
      | none =>
          notes := notes.push s!"return type unsupported: {← Meta.ppExpr body}"
    return some {
      declName    := n
      fname       := n.toString.splitOn "." |>.getLast!
      generics    := generics.map Prod.snd
      admitSets, classAdmits, phantomMask, resolved, notes
    }

/-! ## Driver -/

/-- Read the closest `lean-toolchain` file (looking up from cwd) for the
informational `lean_toolchain` handshake field. -/
def findLeanToolchain : IO String := do
  let candidates : Array System.FilePath := #[
    "lean-toolchain", "../lean-toolchain", "../../lean-toolchain", "../../../lean-toolchain"
  ]
  for p in candidates do
    if ← System.FilePath.pathExists p then
      try
        let s ← IO.FS.readFile p
        return s.trimAscii.copy
      catch _ => pure ()
  return "unknown"

structure Config where
  target       : Name
  outDir       : System.FilePath
  pkg          : String
  iface        : String
  /-- When `true`, after writing the canonical artefacts, shell out to
  `leo4c lower <schema>` and write the resulting WIT text as
  `<pkg>.wit` in the same `outDir`. Opt-in because the Lake build
  precedes the Cargo build (D8) and `leo4c` therefore is not
  guaranteed to exist; users who want `<pkg>.wit` must already have
  built leo4c. -/
  withLower    : Bool := false

def parseArgs (args : List String) : Config := Id.run do
  let withLower := args.contains "--with-lower"
  let pos := args.filter (· != "--with-lower")
  let target := match pos with
    | []     => `Sample
    | a :: _ => a.toName
  let outDir := match pos with
    | _ :: b :: _ => System.FilePath.mk b
    | _ => System.FilePath.mk ".leo4"
  let pkg := match pos with
    | _ :: _ :: c :: _ => c
    | _ => "leo4-sample"
  let iface := match pos with
    | _ :: _ :: _ :: d :: _ => d
    | _ => target.toString
  return { target, outDir, pkg, iface, withLower }

/-- Collect every user-defined nominal type referenced (directly) by an
analysis's resolved param/return types. Returns the deduplicated FQN
set. Used to drive mutual-exclusion (LeanMarshal ∩ LeanResource) checks
and (Phase 5) `.leo4-schema` declaration emission. -/
private def gatherUserTypes (a : ExportAnalysis) : Array String := Id.run do
  let mut acc : Array String := #[]
  let rec collect (t : IDLType) (acc : Array String) : Array String :=
    match t with
    | .record fqn args | .variant fqn args | .resource fqn args =>
        args.foldl (init := acc.push fqn) (fun a t => collect t a)
    | .enumT fqn  => acc.push fqn
    | .flagsT fqn => acc.push fqn
    | .list t | .option t | .io t => collect t acc
    | .result t none => collect t acc
    | .result t (some e) => collect e (collect t acc)
    | .tuple ts => ts.foldl (init := acc) (fun a t => collect t a)
    | _ => acc
  for (_, paramInfos, ret) in a.resolved do
    for p in paramInfos do
      acc := collect p.encoded acc
    acc := collect ret acc
  -- Dedup.
  let mut seen : Array String := #[]
  for f in acc do
    unless seen.contains f do seen := seen.push f
  seen

/-! ## Lean wrapper emit (W7-1) -/

/-- Sanitise an identifier so it's safe to use inside an
auto-generated Lean `def` name: ASCII alphanumerics + `_`. -/
private def sanitiseIdent (s : String) : String :=
  s.foldl (init := "") fun acc c =>
    if c.isAlphanum || c == '_' then acc.push c
    else acc.push '_'

private def joinByUnderscore (xs : Array String) : String :=
  String.intercalate "_" xs.toList

/-- For one `@[leo4_export]` × instantiation, render the wrapper:

```
@[export leo4_lean__<mangled>]
def _leo4_export_<fname>_<param-suffix> (p0 : T0) (p1 : T1) ... : Ret :=
  <declName> (gname1 := <T1>) ... p0 p1 ...
```

The bare `<mangled>` name is reserved for the C shim's canonical-buffer
entry point (SPEC/mangling.md §6). The Lean wrapper exports the
*native-ABI* helper that the shim calls into, hence the `leo4_lean__`
prefix.

`genericArgs` slots whose value is `none` correspond to phantom
generics — they don't appear in the substitution; Lean's inference
fills them. Non-phantom generic args are passed by name so the
user's original definition can stay free of `@`-application
gymnastics. -/
private def renderOneWrapper
    (cfg : Config) (a : ExportAnalysis) (schemaHash : Hash)
    (gargs : Array (Option IDLType)) (params : Array Emit.ParamInfo)
    (ret : IDLType) : String := Id.run do
  let mangled := mangle cfg.pkg cfg.iface a.fname
                  (params.map (·.encoded)) schemaHash
  let paramSeg := joinByUnderscore (params.map fun p => mangleType p.encoded)
  let wrapperName := s!"_leo4_export_{sanitiseIdent a.fname}_{sanitiseIdent paramSeg}"
  -- Parameter signatures `(p0 : T0)` + arg references `p0`.
  let mut paramSigs : Array String := #[]
  let mut paramApps : Array String := #[]
  for i in [0 : params.size] do
    let encStr := idlToLeanType params[i]!.encoded
    paramSigs := paramSigs.push s!"(p{i} : {encStr})"
    paramApps := paramApps.push s!"p{i}"
  let paramApp := String.intercalate " " paramApps.toList
  let paramSigsLine := String.intercalate " " paramSigs.toList
  -- Named generic args. Phantom slots (no observable effect on the
  -- body) are filled with `Unit` so Lean has *some* witness to pin the
  -- implicit; their value is by definition unobservable.
  let mut namedGargs : Array String := #[]
  for i in [0 : a.generics.size] do
    if i < gargs.size then
      let rhs := match gargs[i]! with
        | none     => "Unit"
        | some idl => idlToLeanType idl
      namedGargs := namedGargs.push s!"({a.generics[i]!} := {rhs})"
  let gargsApp := String.intercalate " " namedGargs.toList
  let body :=
    if namedGargs.isEmpty then
      s!"{a.declName.toString} {paramApp}"
    else
      s!"{a.declName.toString} {gargsApp} {paramApp}"
  -- Lean's `@[export ident]` parses an *unquoted* identifier; our
  -- mangled name is already a valid Lean ident after the §1 dash →
  -- underscore normalisation. The bare `<mangled>` symbol is reserved
  -- for the C shim's canonical-buffer entry point (SPEC §6); the Lean
  -- wrapper carries the `leo4_lean__` prefix so the shim can call into
  -- a single deterministic native-ABI helper.
  let header := s!"@[export leo4_lean__{mangled}]\n"
  let sigLine := s!"def {wrapperName} {paramSigsLine} : {idlToLeanType ret} :=\n"
  return header ++ sigLine ++ s!"  {body}\n"

/-- Render the full `<pkg>.leo4-exports.lean` text. The file imports
the user package's target module, then emits one wrapper per
analysis × resolved instantiation. -/
def renderLeanExports
    (cfg : Config) (analyses : Array ExportAnalysis) (schemaHash : Hash) : String := Id.run do
  let banner : String :=
    "-- Auto-generated by `leo4plugin` (W7-1).\n" ++
    "-- Do not edit by hand.\n" ++
    "--\n" ++
    "-- Each entry re-exports one `@[leo4_export]` monomorphisation under\n" ++
    "-- `leo4_lean__<mangled>` — the native-ABI helper symbol that the\n" ++
    "-- C shim calls into. The bare `<mangled>` name is reserved for the\n" ++
    "-- shim's canonical-buffer entry point (SPEC/mangling.md §6).\n" ++
    "--\n" ++
    s!"-- Schema hash : {schemaHash.toBase32lc}\n" ++
    s!"-- Package     : {cfg.pkg}\n" ++
    s!"-- Interface   : {cfg.iface}\n\n"
  let imports := s!"import {cfg.target}\n\n"
  let mut body := ""
  for a in analyses do
    for (gargs, params, ret) in a.resolved do
      body := body ++ renderOneWrapper cfg a schemaHash gargs params ret ++ "\n"
  return banner ++ imports ++ body

/-! ## C shim source emit (W7-2a) -/

/-- Width and canonical-ABI C-type of an IDL primitive that the shim
can wire end-to-end in W7-2a (scalars only). Returns `none` for
non-scalar types — those get the `LEO4_ERR_UNIMPLEMENTED` stub
treatment until W7-2c/W7-2d. -/
private structure ScalarCType where
  c    : String   -- canonical (signed-aware) C type, e.g. `int64_t`
  size : Nat      -- bytes on the wire (== canonical ABI fixed-width)
deriving Inhabited

private def scalarCType : IDLType → Option ScalarCType
  | .u8   => some { c := "uint8_t",  size := 1 }
  | .u16  => some { c := "uint16_t", size := 2 }
  | .u32  => some { c := "uint32_t", size := 4 }
  | .u64  => some { c := "uint64_t", size := 8 }
  | .i8   => some { c := "int8_t",   size := 1 }
  | .i16  => some { c := "int16_t",  size := 2 }
  | .i32  => some { c := "int32_t",  size := 4 }
  | .i64  => some { c := "int64_t",  size := 8 }
  | .f32  => some { c := "float",    size := 4 }
  | .f64  => some { c := "double",   size := 8 }
  | .bool => some { c := "uint8_t",  size := 1 }
  | .char => some { c := "uint32_t", size := 4 }
  | _    => none

/-- C type Lean's code generator picks for a scalar in the native ABI
(matches what `lean -c` prints for `@[export]` signatures). Signed and
unsigned widths share the same C type; floats and char map naturally. -/
private def leanNativeCType : IDLType → Option String
  | .u8 | .i8 | .bool => some "uint8_t"
  | .u16 | .i16        => some "uint16_t"
  | .u32 | .i32 | .char => some "uint32_t"
  | .u64 | .i64        => some "uint64_t"
  | .f32               => some "float"
  | .f64               => some "double"
  | _                  => none

private def cTypeOfIDL (t : IDLType) : String :=
  match leanNativeCType t with
  | some c => c
  | none   => "lean_object*"

/-- Render one shim entry point. Scalar-only signatures get an actual
wire-format encode/decode body that hands off to
`leo4_lean__<mangled>`. Anything that mentions a composite or nominal
type returns the W7-2a placeholder so the link table stays complete
while the encoder grows out in subsequent steps. -/
private def renderOneShim
    (cfg : Config) (a : ExportAnalysis) (schemaHash : Hash)
    (params : Array Emit.ParamInfo) (ret : IDLType) : String := Id.run do
  let mangled := mangle cfg.pkg cfg.iface a.fname
                  (params.map (·.encoded)) schemaHash
  let entry  := s!"leo4_call_{mangled}"
  let helper := s!"leo4_lean__{mangled}"
  let paramScs := params.map fun p => scalarCType p.encoded
  let retSc?   := scalarCType ret
  let allScalar := paramScs.all (·.isSome) && retSc?.isSome
  let paramTyStr := String.intercalate ", "
    (params.toList.map (fun p => cTypeOfIDL p.encoded))
  let retTyStr := cTypeOfIDL ret
  let banner :=
    s!"// {a.fname} :: ({paramTyStr}) -> {retTyStr}\n"
  if !allScalar then
    return banner ++
      s!"int32_t {entry}(\n" ++
      "    leo4_arena_t* arena,\n" ++
      "    const uint8_t* args_ptr, size_t args_len,\n" ++
      "    uint8_t* ret_ptr, size_t ret_cap, size_t* ret_len)\n" ++
      "{\n" ++
      "    (void)arena; (void)args_ptr; (void)args_len;\n" ++
      "    (void)ret_ptr; (void)ret_cap;\n" ++
      "    *ret_len = 0;\n" ++
      "    return LEO4_ERR_UNIMPLEMENTED;  /* W7-2a placeholder */\n" ++
      "}\n"
  let scs := paramScs.map (·.get!)
  let retSc := retSc?.get!
  let inLen := scs.foldl (fun acc p => acc + p.size) 0
  let outLen := retSc.size
  -- Lean native helper extern declaration.
  let leanRetC := (leanNativeCType ret).get!
  let leanArgs := (params.toList.map fun p => (leanNativeCType p.encoded).get!)
  let externArgs :=
    if leanArgs.isEmpty then "void" else String.intercalate ", " leanArgs
  let externDecl := s!"extern {leanRetC} {helper}({externArgs});\n"
  -- Decode args from buffer (memcpy at running offset).
  let mut decode := ""
  let mut off : Nat := 0
  for i in [0 : params.size] do
    let ty := (leanNativeCType params[i]!.encoded).get!
    let sz := scs[i]!.size
    decode := decode ++
      s!"    {ty} a{i}; memcpy(&a{i}, args_ptr + {off}, {sz});\n"
    off := off + sz
  let argApp := String.intercalate ", "
    ((List.range params.size).map (fun i => s!"a{i}"))
  let invoke :=
    if params.isEmpty then s!"{helper}()" else s!"{helper}({argApp})"
  -- `s!"…{{…}}…"` does not accept the doubled-brace escape; we feed
  -- single-character literals via plain concatenation instead.
  let lb : String := "{"
  let rb : String := "}"
  let body :=
    s!"int32_t {entry}(\n" ++
    "    leo4_arena_t* arena,\n" ++
    "    const uint8_t* args_ptr, size_t args_len,\n" ++
    "    uint8_t* ret_ptr, size_t ret_cap, size_t* ret_len)\n" ++
    lb ++ "\n" ++
    "    (void)arena;\n" ++
    s!"    if (args_len != {inLen}u) return LEO4_ERR_DECODE;\n" ++
    decode ++
    s!"    {leanRetC} r = {invoke};\n" ++
    s!"    if (ret_cap < {outLen}u) " ++ lb ++ s!" *ret_len = {outLen}u; return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
    s!"    memcpy(ret_ptr, &r, {outLen});\n" ++
    s!"    *ret_len = {outLen}u;\n" ++
    "    return LEO4_OK;\n" ++
    rb ++ "\n"
  return banner ++ externDecl ++ body

/-- Render the full `<pkg>.leo4-shim.c` text. One translation unit
contains every shim entry point for the package; the matching
`leo4_lean__<mangled>` helpers live in the user's `.olean`-derived
object files and are resolved at `leanc` link time (W7-2b). -/
def renderShimSource
    (cfg : Config) (analyses : Array ExportAnalysis) (schemaHash : Hash) : String := Id.run do
  let banner : String :=
    "// Auto-generated by `leo4plugin` (W7-2a). Do not edit by hand.\n" ++
    "//\n" ++
    "// One translation unit per package; every `@[leo4_export]` ×\n" ++
    "// monomorphisation gets one `leo4_call_<mangled>` entry point\n" ++
    "// (canonical-buffer ABI; SPEC/canonical-abi.md §14). The matching\n" ++
    "// Lean-side helper is `leo4_lean__<mangled>` (SPEC/mangling.md §6).\n" ++
    "//\n" ++
    "// W7-2a scope: scalar-only signatures are wired end-to-end; any\n" ++
    "// composite or nominal type returns LEO4_ERR_UNIMPLEMENTED so the\n" ++
    "// link table stays complete while encoder/decoder coverage grows\n" ++
    "// in W7-2c (composites) and W7-2d (nominal types).\n" ++
    "//\n" ++
    s!"// Schema hash : {schemaHash.toBase32lc}\n" ++
    s!"// Package     : {cfg.pkg}\n" ++
    s!"// Interface   : {cfg.iface}\n" ++
    "\n" ++
    "#include <lean/lean.h>\n" ++
    "#include <stdint.h>\n" ++
    "#include <stddef.h>\n" ++
    "#include <string.h>\n" ++
    "\n" ++
    "/* Status codes per SPEC/canonical-abi.md §13. */\n" ++
    "#define LEO4_OK                        0\n" ++
    "#define LEO4_ERR_DECODE                0x00000001\n" ++
    "#define LEO4_ERR_RETURN_BUF_TOO_SMALL  0x00000007\n" ++
    "#define LEO4_ERR_UNIMPLEMENTED         0x00000064\n" ++
    "\n" ++
    "/* Opaque arena pointer; the W7-2a scalar path doesn't touch it,\n" ++
    "   but the §14 signature reserves the slot. */\n" ++
    "typedef void leo4_arena_t;\n" ++
    "\n"
  let mut body := ""
  for a in analyses do
    for (_gargs, params, ret) in a.resolved do
      body := body ++ renderOneShim cfg a schemaHash params ret ++ "\n"
  return banner ++ body

def runPlugin (cfg : Config) (env : Environment) : IO Unit := do
  let exports := gatherExports env

  -- Analyze every export under one MetaM action.
  let analyses : IO (Array ExportAnalysis) := do
    let action : MetaM (Array ExportAnalysis) := do
      let mut out : Array ExportAnalysis := #[]
      for n in exports do
        match ← analyzeExport n with
        | some a => out := out.push a
        | none   => pure ()
      return out
    let coreCtx : Core.Context := { fileName := "<leo4plugin>", fileMap := FileMap.ofString "" }
    let coreSt  : Core.State   := { env := env }
    let core    : CoreM (Array ExportAnalysis) := action.run' {} {}
    let (xs, _) ← core.toIO coreCtx coreSt
    return xs
  let analyses ← analyses

  IO.println s!"-- analysis (pkg = {cfg.pkg}, iface = {cfg.iface}) --"
  for a in analyses do
    let instCount := a.resolved.size
    IO.println s!"  • {a.declName}  generics={a.generics.size}  instantiations={instCount}"
    for note in a.notes do
      IO.println s!"      note: {note}"

  -- Collect every user type referenced by an analysis (param/return).
  let mut allUserTypes : Array String := #[]
  for a in analyses do
    for t in gatherUserTypes a do
      unless allUserTypes.contains t do allUserTypes := allUserTypes.push t

  -- Mutual-exclusion check: any user type that has both LeanMarshal and
  -- LeanResource instances violates LEO4-DESIGN.md §10.1.
  for fqn in allUserTypes do
    let nm := fqn.toName
    if hasLeanMarshalInstance env nm ∧ hasLeanResourceInstance env nm then
      IO.eprintln s!"  ⚠  {fqn}: declared as both LeanMarshal and LeanResource — \
        v0 requires them to be disjoint (LEO4-DESIGN.md §10.1). \
        The plugin keeps the LeanResource interpretation; please remove one."

  -- Walk every referenced user type and synthesise its IDL declaration.
  let userDeclsIO : IO (Array UserDecl) := do
    let action : MetaM (Array UserDecl) := do
      let mut out : Array UserDecl := #[]
      for fqn in allUserTypes do
        match ← walkUserDecl (← getEnv) {} fqn.toName with
        | some d => out := out.push d
        | none   => pure ()
      return out
    let coreCtx : Core.Context := { fileName := "<leo4plugin>", fileMap := FileMap.ofString "" }
    let coreSt  : Core.State   := { env := env }
    let core    : CoreM (Array UserDecl) := action.run' {} {}
    let (xs, _) ← core.toIO coreCtx coreSt
    return xs
  let userDecls ← userDeclsIO

  IO.println s!"-- user types: {userDecls.size} --"
  for d in userDecls do
    IO.println s!"  • {userDeclToIDL d}"

  -- Build the canonical IDL form (declarations + functions).
  let mut members : Array (String × Array IDLType × IDLType) := #[]
  for a in analyses do
    for (_, paramInfos, ret) in a.resolved do
      members := members.push (a.fname, paramInfos.map (·.encoded), ret)
  let canonical  := renderCanonical cfg.pkg cfg.iface userDecls members (pretty := false)
  let schemaText := renderCanonical cfg.pkg cfg.iface userDecls members (pretty := true)
  let schemaHash := schemaHashOf canonical
  IO.println s!"schema_hash (base32lc): {schemaHash.toBase32lc}"
  IO.println s!"schema_hash (hex)     : {schemaHash.toHex}"

  -- Build mangling entries. Per SPEC/mangling.md §1 the mangled name carries
  -- the *parameter types* after generic substitution; the generic argument
  -- vector and per-parameter origin info are both surfaced in JSON
  -- (SPEC/handshake.md `instantiations[]`).
  let manglingEntries : Array Emit.ManglingEntry := analyses.map fun a =>
    let insts := a.resolved.map fun (gargs, paramInfos, _ret) =>
      ({ genericArgs := gargs
         paramTypes  := paramInfos
         mangled     := mangle cfg.pkg cfg.iface a.fname
                          (paramInfos.map (·.encoded)) schemaHash
       } : Emit.Instantiation)
    { logicalName    := cfg.iface ++ "::" ++ a.fname
      generics       := a.generics.map toString
      instantiations := insts }

  -- constraint_universe: scalar + every class we encountered.
  let mut cu : Array (String × Array IDLType) := #[("scalar", scalarAdmitSet)]
  for a in analyses do
    for (cls, ax) in a.classAdmits do
      unless cu.any (·.1 == cls.toString) do
        cu := cu.push (cls.toString, ax)

  let resourceCount : Nat := userDecls.foldl (init := 0) fun acc d =>
    match d with
    | .resource _ _ => acc + 1
    | _ => acc
  let ifaceSummary : Emit.InterfaceSummary :=
    { name := cfg.iface
      function_count := analyses.size
      resource_count := resourceCount }

  let bundle : Emit.EmitBundle := {
    package            := cfg.pkg
    schemaHash         := schemaHash
    leanToolchain      := ← findLeanToolchain
    pluginVersion      := pluginVersion
    emittedAt          := ← Emit.isoNow
    interfaces         := #[ifaceSummary]
    constraintUniverse := cu
    entries            := manglingEntries
    schemaText         := schemaText
  }

  Emit.emit cfg.outDir bundle
  let stem := normalizePackageSegment cfg.pkg
  let schemaPath    := cfg.outDir / s!"{stem}.leo4-schema"
  let manglingPath  := cfg.outDir / s!"{stem}.leo4-mangling"
  let handshakePath := cfg.outDir / s!"{stem}.leo4-handshake"
  IO.println s!"wrote {schemaPath}"
  IO.println s!"wrote {manglingPath}"
  IO.println s!"wrote {handshakePath}"

  -- Emit the Lean-side wrapper source for the C shim. Each
  -- `@[leo4_export]` × monomorphisation produces one
  --   `@[export leo4_lean__<mangled>]`
  --   `def _leo4_export_<safe-name> (p0 : T0) … : Ret := <fqn> p0 …;`
  -- so the user package's compiled .olean carries the native-ABI
  -- helper symbols the shim will call into (W7-1). The bare
  -- `<mangled>` symbol is reserved for the shim's canonical-buffer
  -- entry point (SPEC §6). The user adds this file to a `lean_lib`
  -- and rebuilds before invoking the shim (W7-2+).
  let leanExportsPath := cfg.outDir / s!"{stem}.leo4-exports.lean"
  let leanExportsText := renderLeanExports cfg analyses schemaHash
  IO.FS.writeFile leanExportsPath leanExportsText
  IO.println s!"wrote {leanExportsPath}"

  -- Emit the C shim source (W7-2a). One translation unit per package
  -- carrying every `leo4_call_<mangled>` entry point. Scalar-only
  -- instantiations are wired through to `leo4_lean__<mangled>`; the
  -- rest get LEO4_ERR_UNIMPLEMENTED placeholders (filled in
  -- W7-2c/W7-2d). The `leanc`-driven compile of this source into
  -- `<pkg>.leo4-shim.so` lands in W7-2b.
  let shimPath := cfg.outDir / s!"{stem}.leo4-shim.c"
  let shimText := renderShimSource cfg analyses schemaHash
  IO.FS.writeFile shimPath shimText
  IO.println s!"wrote {shimPath}"

  -- Optional: lower each emitted `.leo4-schema` to `.wit` via the
  -- leo4c CLI. The shell-out is opt-in (`--with-lower`) precisely to
  -- preserve D8's Lake-then-Cargo build order: the Lake plugin itself
  -- has no Cargo dependency, so a plain `lake exe leo4plugin` still
  -- works on a fresh checkout. When the user wants WIT, they pre-build
  -- leo4c (or trust their PATH) and add the flag.
  if cfg.withLower then
    let schemas : Array System.FilePath := #[schemaPath]
    for sp in schemas do
      let witPath := sp.withExtension "wit"
      try
        let out ← IO.Process.run {
          cmd := "leo4c"
          args := #["lower", sp.toString]
        }
        IO.FS.writeFile witPath out
        IO.println s!"wrote {witPath}"
      catch e =>
        IO.eprintln s!"  ⚠  --with-lower: leo4c invocation failed for {sp}: {e}"
        IO.eprintln "      ensure `cargo build -p leo4c` (or release) and `leo4c` is on PATH"

def main (args : List String) : IO UInt32 := do
  let cfg := parseArgs args
  IO.println s!"leo4plugin: target={cfg.target} outDir={cfg.outDir} pkg={cfg.pkg} iface={cfg.iface}"

  Lean.initSearchPath (← Lean.findSysroot)

  -- Required since Lean v4.30.0-rc2 before `importModules (loadExts := true)`;
  -- present (and a no-op until called) in v4.27.0+. We call it
  -- unconditionally so the same plugin binary works across the
  -- supported toolchain matrix. `unsafeIO` wrapping reflects the
  -- function's `unsafe` signature in `Lean.ImportingFlag`.
  unsafe Lean.enableInitializersExecution

  let t0 ← IO.monoNanosNow
  let env ← Lean.importModules
    (imports := #[{ module := cfg.target }, { module := `Leo4 }])
    (opts := {}) (trustLevel := 0) (loadExts := true)
  let t1 ← IO.monoNanosNow

  IO.println s!"env: {env.allImportedModuleNames.size} imported modules"
  IO.println s!"importModules (loadExts=true): {msStr (t1 - t0)}"

  let t2 ← IO.monoNanosNow
  runPlugin cfg env
  let t3 ← IO.monoNanosNow

  IO.println "-- timings --"
  IO.println s!"importModules : {msStr (t1 - t0)}"
  IO.println s!"runPlugin     : {msStr (t3 - t2)}"
  IO.println s!"total (wall)  : {msStr (t3 - t0)}"
  return 0

end Leo4Plugin

/-- C-callable entry point so Lake's `lean_exe` can find `main`. -/
def main (args : List String) : IO UInt32 := Leo4Plugin.main args
