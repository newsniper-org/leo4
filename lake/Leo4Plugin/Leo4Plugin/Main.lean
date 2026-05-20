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
  /-- Implicit value-typed binders erased from the boundary signature
  (SPEC/mangling.md §"Value-param erasure"). The wrapper fills these
  with `default` at the call site — the binder's type must be
  `Inhabited`. Truly phantom (no inference from value params), so the
  wrapper must spell every erased name explicitly. -/
  erasedImplicits : Array Name
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
    let mut erasedImplicits : Array Name := #[]
    for a in args do
      let ld ← a.fvarId!.getDecl
      let ty := ld.type
      if ld.binderInfo.isImplicit ∧ isKindExpr ty then
        generics := generics.push (a.fvarId!, ld.userName)
        if isHigherKind ty then
          higherKinds := higherKinds.push (a.fvarId!, ld.userName)
      else if ld.binderInfo.isImplicit then
        -- Implicit value-typed binder (e.g., `{N : Nat}`):
        -- erased at the boundary per SPEC/mangling.md §"Value-param erasure".
        -- The value never crosses the wire; the wrapper renderer fills
        -- the implicit at the wrapper's call site via `(name := default)`
        -- so Lean's elaborator can pin it (the binder's type must be
        -- `Inhabited`). For binders that Lean *can* infer from a later
        -- parameter (e.g., `{n : Nat} (xs : Vec α n)`), `default` is
        -- overridden by inference — the explicit fill is harmless.
        erasedImplicits := erasedImplicits.push ld.userName
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
        match exprToIDLSubst env none subst #[] ty with
        | some idl =>
            paramInfos := paramInfos.push
              { encoded := idl, usesGenerics := paramOrigins[i]! }
        | none =>
            notes := notes.push s!"param type unsupported: {← Meta.ppExpr ty}"
            ok := false
      if !ok then continue
      match exprToIDLSubst env none subst #[] body with
      | some retIDL => resolved := resolved.push (fullGenArgs, paramInfos, retIDL)
      | none =>
          notes := notes.push s!"return type unsupported: {← Meta.ppExpr body}"
    return some {
      declName    := n
      fname       := n.toString.splitOn "." |>.getLast!
      generics    := generics.map Prod.snd
      admitSets, classAdmits, phantomMask, resolved, notes
      erasedImplicits
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
  -- Parameter signatures `(p0 : T0)` + arg references `p0`. When
  -- there are no parameters we insert a dummy `(_unit : Unit)` so
  -- Lean's code generator emits the wrapper as a *function* rather
  -- than as a constant (const-folding hello-style 0-arg definitions
  -- into a `const lean_object*` variable, which the shim then can't
  -- call). The shim's 0-arg entry point passes `lean_box(0)` for
  -- this dummy slot.
  let mut paramSigs : Array String := #[]
  let mut paramApps : Array String := #[]
  if params.isEmpty then
    paramSigs := paramSigs.push "(_unit : Unit)"
    paramApps := paramApps.push "_unit"
  else
    for i in [0 : params.size] do
      let encStr := idlToLeanType params[i]!.encoded
      paramSigs := paramSigs.push s!"(p{i} : {encStr})"
      paramApps := paramApps.push s!"p{i}"
  -- For the user-decl call we never pass the unit dummy along.
  let mut callApps : Array String := #[]
  for i in [0 : params.size] do
    callApps := callApps.push s!"p{i}"
  let paramApp := String.intercalate " " callApps.toList
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
  -- Erased implicit value-typed binders (SPEC §"Value-param erasure"):
  -- the boundary signature drops them, but Lean's elaborator still needs
  -- a value at the wrapper's call site. We fill with `default` —
  -- requires the binder's type to be `Inhabited`. For binders Lean can
  -- infer from later parameters (Vec n α-style), `default` is shadowed
  -- by inference and harmless.
  let mut namedErased : Array String := #[]
  for name in a.erasedImplicits do
    namedErased := namedErased.push s!"({name} := default)"
  let allNamed := namedGargs ++ namedErased
  let gargsApp := String.intercalate " " allNamed.toList
  let body :=
    if allNamed.isEmpty then
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

/-- Render one external-marshal helper pair for type `fqn` (Phase 8 step
2b). The pair exposes Lean's `LeanMarshal` instance for `fqn` under
deterministic C-callable names — `leo4_marshal_<fqnSeg>_dec` and
`_enc` — that the shim's external-marshal handler `extern`-declares
and calls. -/
private def renderExternalMarshalHelpers (fqn : String) : String :=
  let seg := fqnSeg fqn
  let dec := s!"leo4_marshal_{seg}_dec"
  let enc := s!"leo4_marshal_{seg}_enc"
  "@[export " ++ dec ++ "]\n" ++
  "def _" ++ dec ++ " (buf : ByteArray) (off : Nat) : Except Leo4.LeanError (" ++ fqn ++ " × Nat) :=\n" ++
  "  Leo4.LeanMarshal.canonicalDecode buf off\n\n" ++
  "@[export " ++ enc ++ "]\n" ++
  "def _" ++ enc ++ " (val : " ++ fqn ++ ") (buf : ByteArray) : ByteArray :=\n" ++
  "  Leo4.LeanMarshal.canonicalEncode val buf\n\n"

/-- Render the full `<pkg>.leo4-exports.lean` text. The file imports
the user package's target module, then emits one wrapper per
analysis × resolved instantiation. Phase 8 step 2b adds per-type
external-marshal helper pairs for every `UserDecl.externalMarshal`
in `userDecls`. -/
def renderLeanExports
    (cfg : Config) (userDecls : Array UserDecl)
    (analyses : Array ExportAnalysis) (schemaHash : Hash) : String := Id.run do
  let banner : String :=
    "-- Auto-generated by `leo4plugin` (W7-1).\n" ++
    "-- Do not edit by hand.\n" ++
    "--\n" ++
    "-- Each entry re-exports one `@[leo4_export]` monomorphisation under\n" ++
    "-- `leo4_lean__<mangled>` — the native-ABI helper symbol that the\n" ++
    "-- C shim calls into. The bare `<mangled>` name is reserved for the\n" ++
    "-- shim's canonical-buffer entry point (SPEC/mangling.md §6).\n" ++
    "--\n" ++
    "-- External-marshal helpers (Phase 8 step 2b) live alongside the\n" ++
    "-- export wrappers; the shim's external-marshal handler\n" ++
    "-- `extern`-declares them and bridges `uint8_t* ⇄ ByteArray`.\n" ++
    "--\n" ++
    s!"-- Schema hash : {schemaHash.toBase32lc}\n" ++
    s!"-- Package     : {cfg.pkg}\n" ++
    s!"-- Interface   : {cfg.iface}\n\n"
  let imports := s!"import {cfg.target}\n\n"
  let mut body := ""
  -- External-marshal helpers first so the wrappers below can rely
  -- on the helper names being in scope.
  for d in userDecls.flatMap (·.leaves) do
    match d with
    | .externalMarshal fqn _ =>
      body := body ++ renderExternalMarshalHelpers fqn
    | _ => pure ()
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

-- Wire and Lean native ABI agree on bit pattern (signed/unsigned share
-- a C width), so we represent every scalar's local C variable with the
-- *unsigned* counterpart. This keeps `boxExpr`/`unboxExpr` and the
-- Lean wrapper's `extern` declaration aligned on a single C type per
-- IDL width.
private def scalarCType : IDLType → Option ScalarCType
  | .u8  | .i8   => some { c := "uint8_t",  size := 1 }
  | .u16 | .i16  => some { c := "uint16_t", size := 2 }
  | .u32 | .i32  => some { c := "uint32_t", size := 4 }
  | .u64 | .i64  => some { c := "uint64_t", size := 8 }
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

/-- Per-IDL-type code-emission contract for the shim.
    See `handlerFor` below for the supported set; types not handled
    here fall through to the `LEO4_ERR_UNIMPLEMENTED` stub. -/
private structure TyHandler where
  /-- C type of the local variable that holds the decoded native value. -/
  cType        : String
  /-- C type the Lean wrapper expects for this slot in its extern decl
      (same as `cType` for the handled scalar+string set; will diverge
      for future composites that need a boxed Lean object). -/
  externCType  : String
  /-- True when the local variable owns a Lean refcount that must be
      `lean_dec_ref`'d on cleanup. -/
  ownsRef      : Bool
  /-- For scalar types, the suffix of the `lean_ctor_{get,set}_<X>`
      accessor family (`"uint8"`, `"uint16"`, `"uint32"`, `"uint64"`,
      `"float"`, `"float32"`). `none` when the value is a boxed
      `lean_object*` that uses `lean_ctor_get` / `lean_ctor_set` (one
      indexed slot per object field). Used by composite handlers when
      they store the inner value as a constructor field. -/
  scalarKind   : Option String
  /-- Scalar field width contribution to `lean_alloc_ctor`'s
      `scalar_sz` argument. `0` when the value is a boxed
      `lean_object*` (it goes into a `num_objs` slot instead). -/
  ctorScalarSz : Nat
  /-- Emits a decode block: declares `var` of `cType`, reads from
      `args_ptr` at `off`, advances `off`. On failure the block runs
      `cleanup` (which dec_ref's prior owned args), resets
      `*ret_len = 0`, and returns a status code. -/
  decodeBlock  : (var : String) → (cleanup : String) → String
  /-- Emits an encode block for a local variable `var` (typically `r`
      for the return slot, or a per-ctor-field temporary for
      composites). On too-small: `*ret_len = out_off + need`, run
      `cleanup`, return LEO4_ERR_RETURN_BUF_TOO_SMALL. On success:
      write into `ret_ptr + out_off`, advance `out_off`, run `cleanup`. -/
  encodeBlock  : (var : String) → (cleanup : String) → String
  /-- Box a native C value into a `lean_object *`. For scalars: calls
      the right `lean_box_*` helper. For already-boxed values: the
      identity. The returned string is a C *expression*. -/
  boxExpr      : (var : String) → String
  /-- Inverse of `boxExpr`. The returned string is a C *expression*. -/
  unboxExpr    : (var : String) → String
deriving Inhabited

/-- `lean_ctor_{get,set}_<suffix>` family for a scalar's native ABI C
type. Matches the lean.h naming exactly. -/
private def scalarCtorKind (c : String) : String :=
  match c with
  | "uint8_t"  => "uint8"
  | "uint16_t" => "uint16"
  | "uint32_t" => "uint32"
  | "uint64_t" => "uint64"
  | "float"    => "float32"   -- lean_ctor_set_float32 / lean_ctor_get_float32
  | "double"   => "float"     -- lean_ctor_set_float / lean_ctor_get_float
  | _         => "ptr"

/-- Native-C → boxed `lean_object *` expression. Matches the lean.h
boxing helpers and the immediate-tag scheme for small ints. -/
private def scalarBox (c : String) (var : String) : String :=
  match c with
  | "uint8_t"  => s!"lean_box((size_t)({var}))"
  | "uint16_t" => s!"lean_box((size_t)({var}))"
  | "uint32_t" => s!"lean_box_uint32({var})"
  | "uint64_t" => s!"lean_box_uint64({var})"
  | "float"    => s!"lean_box_float32({var})"
  | "double"   => s!"lean_box_float({var})"
  | _         => var

/-- Boxed `lean_object *` C expression → native-C value of `c` type. -/
private def scalarUnbox (c : String) (var : String) : String :=
  match c with
  | "uint8_t"  => s!"(uint8_t)lean_unbox({var})"
  | "uint16_t" => s!"(uint16_t)lean_unbox({var})"
  | "uint32_t" => s!"lean_unbox_uint32({var})"
  | "uint64_t" => s!"lean_unbox_uint64({var})"
  | "float"    => s!"lean_unbox_float32({var})"
  | "double"   => s!"lean_unbox_float({var})"
  | _         => var

private def scalarHandler (c : String) (size : Nat) : TyHandler :=
  let lb := "{"
  let rb := "}"
  { cType        := c
    externCType  := c
    ownsRef      := false
    scalarKind   := some (scalarCtorKind c)
    ctorScalarSz := size
    decodeBlock  := fun var cleanup =>
      s!"    {c} {var};\n" ++
      s!"    if (args_len - off < {size}u) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      s!"    leo4_memcpy(&{var}, args_ptr + off, {size});\n" ++
      s!"    off += {size}u;\n"
    encodeBlock  := fun var cleanup =>
      s!"    if (ret_cap - out_off < {size}u) " ++ lb ++
        s!" *ret_len = out_off + {size}u;{cleanup} return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
      s!"    leo4_memcpy(ret_ptr + out_off, &{var}, {size});\n" ++
      s!"    out_off += {size}u;\n" ++
      cleanup
    boxExpr      := scalarBox c
    unboxExpr    := scalarUnbox c
  }

private def stringHandler : TyHandler :=
  let lb := "{"
  let rb := "}"
  { cType        := "lean_object *"
    externCType  := "lean_object *"
    ownsRef      := true
    scalarKind   := none
    ctorScalarSz := 0
    decodeBlock  := fun var cleanup =>
      s!"    lean_object *{var} = NULL;\n" ++
      "    " ++ lb ++ "\n" ++
      s!"        int32_t st = leo4_decode_string(args_ptr, args_len, &off, &{var});\n" ++
      "        if (st != LEO4_OK) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return st; " ++ rb ++ "\n" ++
      "    " ++ rb ++ "\n"
    encodeBlock  := fun var cleanup =>
      "    " ++ lb ++ "\n" ++
      s!"        size_t need = leo4_encoded_size_string({var});\n" ++
      "        if (ret_cap - out_off < need) " ++ lb ++
        s!" *ret_len = out_off + need;{cleanup} return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
      s!"        leo4_write_string({var}, ret_ptr, &out_off);\n" ++
      "    " ++ rb ++ "\n" ++
      cleanup
    boxExpr      := id
    unboxExpr    := id
  }

/-! ## Composite handlers (W7-2c-ii / W7-2c-iii) -/

/-- Generate a fresh local variable suffix from the var name. Composite
handlers nest into local scopes (their own `{ … }` blocks), so suffix
collisions only need to avoid the enclosing `aN`/`r` names. We use
`<var>_inner` for nested locals — single-level nesting is enough
because each handler opens its own block. -/
private def innerName (var : String) : String := var ++ "_inner"

/-- `lean_ctor_set_<kind>(o, slot, val)` invocation. For boxed values
(`scalarKind = none`) the slot is an object index; for scalars it is a
byte offset within the constructor's scalar area. -/
private def ctorSetCall
    (h : TyHandler) (objSlot : Nat) (scalarOff : Nat)
    (obj : String) (val : String) : String :=
  match h.scalarKind with
  | none     => s!"lean_ctor_set({obj}, {objSlot}, {val})"
  | some "ptr" => s!"lean_ctor_set({obj}, {objSlot}, {val})"
  | some kind => s!"lean_ctor_set_{kind}({obj}, {scalarOff}, {val})"

private def ctorGetCall
    (h : TyHandler) (objSlot : Nat) (scalarOff : Nat) (obj : String) : String :=
  match h.scalarKind with
  | none     => s!"lean_ctor_get({obj}, {objSlot})"
  | some "ptr" => s!"lean_ctor_get({obj}, {objSlot})"
  | some kind => s!"lean_ctor_get_{kind}({obj}, {scalarOff})"

/-- A unary-payload constructor encoding helper used by both
`optionHandler` (Some) and `resultHandler` (Ok / Err). Generates code
that, given an inner handler `ih`, takes one wire byte and an inner
value, allocates the appropriate Lean ctor, and stuffs the inner value
into it. -/
private def emitAllocCtorWithField
    (ih : TyHandler) (tagLean : Nat) (objVar : String) (innerVar : String) : String :=
  let numObjs   := if ih.ownsRef then 1 else 0
  let scalarSz  := ih.ctorScalarSz
  s!"        lean_object *{objVar} = lean_alloc_ctor({tagLean}, {numObjs}, {scalarSz});\n" ++
  s!"        {ctorSetCall ih 0 0 objVar innerVar};\n"

/-- `option<T>` per SPEC §5: u8 discriminator (0=none, 1=some) + payload.
    Lean ctor index matches the wire (`Option.none` = 0, `Option.some`
    = 1), so no remapping is needed. -/
private def optionHandler (ih : TyHandler) : TyHandler :=
  let lb := "{"
  let rb := "}"
  { cType        := "lean_object *"
    externCType  := "lean_object *"
    ownsRef      := true
    scalarKind   := none
    ctorScalarSz := 0
    decodeBlock  := fun var cleanup =>
      let inner := innerName var
      s!"    lean_object *{var} = NULL;\n" ++
      "    " ++ lb ++ "\n" ++
      s!"        if (args_len - off < 1u) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      s!"        uint8_t disc = args_ptr[off]; off += 1u;\n" ++
      "        if (disc == 0u) " ++ lb ++ s!" {var} = lean_box(0); " ++ rb ++ "\n" ++
      "        else if (disc == 1u) " ++ lb ++ "\n" ++
      ih.decodeBlock inner cleanup ++
      emitAllocCtorWithField ih 1 var inner ++
      "        " ++ rb ++ " else " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      "    " ++ rb ++ "\n"
    encodeBlock  := fun var cleanup =>
      let inner := innerName var
      "    " ++ lb ++ "\n" ++
      "        if (ret_cap - out_off < 1u) " ++ lb ++
        s!" *ret_len = out_off + 1u;{cleanup} return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
      s!"        unsigned _tag = lean_obj_tag({var});\n" ++
      "        if (_tag == 0u) " ++ lb ++ "\n" ++
      "            ret_ptr[out_off] = 0u; out_off += 1u;\n" ++
      "        " ++ rb ++ " else " ++ lb ++ "\n" ++
      "            ret_ptr[out_off] = 1u; out_off += 1u;\n" ++
      -- `lean_ctor_get` returns a borrowed reference; the field stays
      -- owned by the parent constructor. The inner `encodeBlock` only
      -- reads, so we pass `""` for its cleanup and skip lean_inc/dec
      -- entirely.
      s!"            {ih.cType} {inner} = ({ih.cType}){ctorGetCall ih 0 0 var};\n" ++
      ih.encodeBlock inner "" ++
      "        " ++ rb ++ "\n" ++
      "    " ++ rb ++ "\n" ++
      cleanup
    boxExpr      := id
    unboxExpr    := id
  }

/-- `result<T, E>` per SPEC §6: u8 discriminator (0=ok, 1=err) + payload.
    Lean `Except`: ctor 0 = `.error`, ctor 1 = `.ok` — wire and ctor
    indices are **inverted**, so the decoder allocates ctor 1 for wire
    0 and ctor 0 for wire 1; the encoder reads `lean_obj_tag` and
    flips accordingly. -/
private def resultHandler (ihOk : TyHandler) (ihErr : TyHandler) : TyHandler :=
  let lb := "{"
  let rb := "}"
  { cType        := "lean_object *"
    externCType  := "lean_object *"
    ownsRef      := true
    scalarKind   := none
    ctorScalarSz := 0
    decodeBlock  := fun var cleanup =>
      let inner := innerName var
      s!"    lean_object *{var} = NULL;\n" ++
      "    " ++ lb ++ "\n" ++
      s!"        if (args_len - off < 1u) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      s!"        uint8_t disc = args_ptr[off]; off += 1u;\n" ++
      "        if (disc == 0u) " ++ lb ++ "\n" ++
      ihOk.decodeBlock inner cleanup ++
      emitAllocCtorWithField ihOk 1 var inner ++
      "        " ++ rb ++ " else if (disc == 1u) " ++ lb ++ "\n" ++
      ihErr.decodeBlock inner cleanup ++
      emitAllocCtorWithField ihErr 0 var inner ++
      "        " ++ rb ++ " else " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      "    " ++ rb ++ "\n"
    encodeBlock  := fun var cleanup =>
      let inner := innerName var
      "    " ++ lb ++ "\n" ++
      "        if (ret_cap - out_off < 1u) " ++ lb ++
        s!" *ret_len = out_off + 1u;{cleanup} return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
      s!"        unsigned _tag = lean_obj_tag({var});\n" ++
      -- `lean_ctor_get` returns borrowed references on both branches.
      -- The inner `encodeBlock` only reads, so we pass `""` for its
      -- cleanup and skip lean_inc/dec entirely.
      "        if (_tag == 1u) " ++ lb ++ "\n" ++
      "            ret_ptr[out_off] = 0u; out_off += 1u;\n" ++
      s!"            {ihOk.cType} {inner} = ({ihOk.cType}){ctorGetCall ihOk 0 0 var};\n" ++
      ihOk.encodeBlock inner "" ++
      "        " ++ rb ++ " else " ++ lb ++ "\n" ++
      "            ret_ptr[out_off] = 1u; out_off += 1u;\n" ++
      s!"            {ihErr.cType} {inner} = ({ihErr.cType}){ctorGetCall ihErr 0 0 var};\n" ++
      ihErr.encodeBlock inner "" ++
      "        " ++ rb ++ "\n" ++
      "    " ++ rb ++ "\n" ++
      cleanup
    boxExpr      := id
    unboxExpr    := id
  }

/-- `list<T>` per SPEC §4:
       u32 len + T encodings concatenated.
    Lean `List α` is a singly-linked chain of `cons` ctors (tag 1, two
    fields: head as `lean_object *` boxed, tail), terminated by `nil`
    (tag 0, no fields → `lean_box(0)`). The decoder collects all
    elements into a fresh `lean_array_object` (each element boxed via
    `ih.boxExpr`) and converts to a `List` with `lean_array_to_list`.
    The encoder walks the cons chain, unboxes each head via
    `ih.unboxExpr`, and delegates to `ih.encodeBlock`. -/
private def listHandler (ih : TyHandler) : TyHandler :=
  let lb := "{"
  let rb := "}"
  { cType        := "lean_object *"
    externCType  := "lean_object *"
    ownsRef      := true
    scalarKind   := none
    ctorScalarSz := 0
    decodeBlock  := fun var cleanup =>
      let inner := innerName var
      let innerCleanup := s!" lean_dec({var}_arr);" ++ cleanup
      s!"    lean_object *{var} = NULL;\n" ++
      "    " ++ lb ++ "\n" ++
      s!"        if (args_len - off < 4u) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      s!"        uint32_t {var}_len;\n" ++
      s!"        leo4_memcpy(&{var}_len, args_ptr + off, 4);\n" ++
      s!"        off += 4u;\n" ++
      s!"        lean_object *{var}_arr = lean_alloc_array((size_t){var}_len, (size_t){var}_len);\n" ++
      s!"        lean_object **{var}_slots = lean_array_cptr({var}_arr);\n" ++
      s!"        for (size_t {var}_i = 0; {var}_i < (size_t){var}_len; {var}_i++) {var}_slots[{var}_i] = lean_box(0);\n" ++
      s!"        for (size_t {var}_i = 0; {var}_i < (size_t){var}_len; {var}_i++) " ++ lb ++ "\n" ++
      ih.decodeBlock inner innerCleanup ++
      s!"            {var}_slots[{var}_i] = {ih.boxExpr inner};\n" ++
      "        " ++ rb ++ "\n" ++
      s!"        {var} = lean_array_to_list({var}_arr);\n" ++
      "    " ++ rb ++ "\n"
    encodeBlock  := fun var cleanup =>
      let inner := innerName var
      "    " ++ lb ++ "\n" ++
      "        if (ret_cap - out_off < 4u) " ++ lb ++
        s!" *ret_len = out_off + 4u;{cleanup} return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
      s!"        size_t {var}_len_off = out_off; out_off += 4u;\n" ++
      s!"        uint32_t {var}_count = 0;\n" ++
      s!"        lean_object *{var}_cur = {var};\n" ++
      s!"        while (lean_obj_tag({var}_cur) == 1u) " ++ lb ++ "\n" ++
      s!"            lean_object *{var}_head = lean_ctor_get({var}_cur, 0);\n" ++
      s!"            {ih.cType} {inner} = {ih.unboxExpr s!"{var}_head"};\n" ++
      ih.encodeBlock inner "" ++
      s!"            {var}_cur = lean_ctor_get({var}_cur, 1);\n" ++
      s!"            {var}_count++;\n" ++
      "        " ++ rb ++ "\n" ++
      s!"        leo4_memcpy(ret_ptr + {var}_len_off, &{var}_count, 4);\n" ++
      "    " ++ rb ++ "\n" ++
      cleanup
    boxExpr      := id
    unboxExpr    := id
  }

/-- `tuple<T₁, T₂>` per SPEC §7, restricted to binary tuples (the only
    arity Leo4 currently marshals: `LeanMarshal (α × β)`). Wire format:
    `T₁` encoding followed by `T₂` encoding, no padding. Lean
    representation: `Prod α β` = `lean_alloc_ctor(0, num_objs, scalar_sz)`
    with `fst` at object slot/scalar offset 0 and `snd` at the next
    slot/offset, in declaration order. -/
private def binaryTupleHandler (ih1 : TyHandler) (ih2 : TyHandler) : TyHandler :=
  let lb := "{"
  let rb := "}"
  -- Lean ctor layout for Prod:
  --   object fields come first (indexed by appearance order among objects),
  --   then scalar fields packed in declaration order.
  let numObjs := (if ih1.ownsRef then 1 else 0) + (if ih2.ownsRef then 1 else 0)
  let scalarSz := ih1.ctorScalarSz + ih2.ctorScalarSz
  -- For each component: where it lives in the ctor.
  let obj1Slot := 0
  let obj2Slot := if ih1.ownsRef then 1 else 0
  let scalar1Off := 0
  let scalar2Off := ih1.ctorScalarSz
  -- Builders.
  let setExpr (h : TyHandler) (objSlot scalarOff : Nat) (obj val : String) : String :=
    ctorSetCall h objSlot scalarOff obj val
  let getExpr (h : TyHandler) (objSlot scalarOff : Nat) (obj : String) : String :=
    ctorGetCall h objSlot scalarOff obj
  { cType        := "lean_object *"
    externCType  := "lean_object *"
    ownsRef      := true
    scalarKind   := none
    ctorScalarSz := 0
    decodeBlock  := fun var cleanup =>
      let v1 := s!"{var}_fst"
      let v2 := s!"{var}_snd"
      let cleanupAfterFirst :=
        (if ih1.ownsRef then s!" lean_dec({v1});" else "") ++ cleanup
      s!"    lean_object *{var} = NULL;\n" ++
      "    " ++ lb ++ "\n" ++
      ih1.decodeBlock v1 cleanup ++
      ih2.decodeBlock v2 cleanupAfterFirst ++
      s!"        {var} = lean_alloc_ctor(0, {numObjs}, {scalarSz});\n" ++
      s!"        {setExpr ih1 obj1Slot scalar1Off var v1};\n" ++
      s!"        {setExpr ih2 obj2Slot scalar2Off var v2};\n" ++
      "    " ++ rb ++ "\n"
    encodeBlock  := fun var cleanup =>
      let v1 := s!"{var}_fst"
      let v2 := s!"{var}_snd"
      "    " ++ lb ++ "\n" ++
      s!"        {ih1.cType} {v1} = ({ih1.cType}){getExpr ih1 obj1Slot scalar1Off var};\n" ++
      ih1.encodeBlock v1 "" ++
      s!"        {ih2.cType} {v2} = ({ih2.cType}){getExpr ih2 obj2Slot scalar2Off var};\n" ++
      ih2.encodeBlock v2 "" ++
      "    " ++ rb ++ "\n" ++
      cleanup
    boxExpr      := id
    unboxExpr    := id
  }

/-- `bignat` per SPEC §2: `u32 len | LE u64 limbs`. v0 shim supports
    single-limb (`len ∈ {0, 1}`) only — multi-limb encode/decode
    require mpz-level limb extraction that `lean.h` does not expose,
    so we return `LEO4_ERR_UNIMPLEMENTED` in that path until the
    post-v0 follow-up. -/
private def bignatHandler : TyHandler :=
  let lb := "{"
  let rb := "}"
  { cType        := "lean_object *"
    externCType  := "lean_object *"
    ownsRef      := true
    scalarKind   := none
    ctorScalarSz := 0
    decodeBlock  := fun var cleanup =>
      s!"    lean_object *{var} = NULL;\n" ++
      "    " ++ lb ++ "\n" ++
      "        if (args_len - off < 4u) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      s!"        uint32_t {var}_len;\n" ++
      s!"        leo4_memcpy(&{var}_len, args_ptr + off, 4);\n" ++
      "        off += 4u;\n" ++
      s!"        if ({var}_len == 0u) " ++ lb ++ s!" {var} = lean_box(0); " ++ rb ++ "\n" ++
      s!"        else if ({var}_len == 1u) " ++ lb ++ "\n" ++
      "            if (args_len - off < 8u) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      s!"            uint64_t {var}_limb;\n" ++
      s!"            leo4_memcpy(&{var}_limb, args_ptr + off, 8);\n" ++
      "            off += 8u;\n" ++
      s!"            {var} = lean_uint64_to_nat({var}_limb);\n" ++
      "        " ++ rb ++ " else " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_UNIMPLEMENTED; " ++ rb ++ "\n" ++
      "    " ++ rb ++ "\n"
    encodeBlock  := fun var cleanup =>
      "    " ++ lb ++ "\n" ++
      s!"        if (!lean_is_scalar({var})) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_UNIMPLEMENTED; " ++ rb ++ "\n" ++
      s!"        size_t {var}_v = lean_unbox({var});\n" ++
      s!"        if ({var}_v == 0u) " ++ lb ++ "\n" ++
      "            if (ret_cap - out_off < 4u) " ++ lb ++
        s!" *ret_len = out_off + 4u;{cleanup} return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
      "            uint32_t _len = 0u;\n" ++
      "            leo4_memcpy(ret_ptr + out_off, &_len, 4);\n" ++
      "            out_off += 4u;\n" ++
      "        " ++ rb ++ " else " ++ lb ++ "\n" ++
      "            if (ret_cap - out_off < 12u) " ++ lb ++
        s!" *ret_len = out_off + 12u;{cleanup} return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
      "            uint32_t _len = 1u;\n" ++
      "            leo4_memcpy(ret_ptr + out_off, &_len, 4);\n" ++
      "            out_off += 4u;\n" ++
      s!"            uint64_t _limb = (uint64_t){var}_v;\n" ++
      "            leo4_memcpy(ret_ptr + out_off, &_limb, 8);\n" ++
      "            out_off += 8u;\n" ++
      "        " ++ rb ++ "\n" ++
      "    " ++ rb ++ "\n" ++
      cleanup
    boxExpr      := id
    unboxExpr    := id
  }

/-- `bigint` per SPEC §2: `u8 sign | u32 len | LE u64 limbs`. v0 shim
    handles single-limb only (`len ∈ {0, 1}`); multi-limb signals
    `LEO4_ERR_UNIMPLEMENTED`. Lean's small Int range is roughly
    `[-2³¹, 2³¹)`, well inside a single limb. Larger magnitudes route
    through `lean_int_big_*` (mpz) — same limitation as `bignat`. -/
private def bigintHandler : TyHandler :=
  let lb := "{"
  let rb := "}"
  { cType        := "lean_object *"
    externCType  := "lean_object *"
    ownsRef      := true
    scalarKind   := none
    ctorScalarSz := 0
    decodeBlock  := fun var cleanup =>
      s!"    lean_object *{var} = NULL;\n" ++
      "    " ++ lb ++ "\n" ++
      "        if (args_len - off < 5u) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      s!"        uint8_t {var}_sign = args_ptr[off];\n" ++
      "        off += 1u;\n" ++
      s!"        if ({var}_sign > 1u) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      s!"        uint32_t {var}_len;\n" ++
      s!"        leo4_memcpy(&{var}_len, args_ptr + off, 4);\n" ++
      "        off += 4u;\n" ++
      s!"        if ({var}_len == 0u) " ++ lb ++ "\n" ++
      s!"            if ({var}_sign != 0u) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      s!"            {var} = lean_int_to_int(0);\n" ++
      s!"        " ++ rb ++ s!" else if ({var}_len == 1u) " ++ lb ++ "\n" ++
      "            if (args_len - off < 8u) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      s!"            uint64_t {var}_limb;\n" ++
      s!"            leo4_memcpy(&{var}_limb, args_ptr + off, 8);\n" ++
      "            off += 8u;\n" ++
      s!"            if ({var}_sign == 0u) " ++ lb ++ "\n" ++
      s!"                lean_object *_n = lean_uint64_to_nat({var}_limb);\n" ++
      s!"                {var} = lean_nat_to_int(_n);\n" ++
      "            " ++ rb ++ " else " ++ lb ++ "\n" ++
      s!"                if ({var}_limb > 9223372036854775808ULL) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_UNIMPLEMENTED; " ++ rb ++ "\n" ++
      s!"                int64_t _v = ({var}_limb == 9223372036854775808ULL)\n" ++
      s!"                              ? (-9223372036854775807LL - 1LL)\n" ++
      s!"                              : -((int64_t){var}_limb);\n" ++
      s!"                {var} = lean_int64_to_int(_v);\n" ++
      "            " ++ rb ++ "\n" ++
      "        " ++ rb ++ " else " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_UNIMPLEMENTED; " ++ rb ++ "\n" ++
      "    " ++ rb ++ "\n"
    encodeBlock  := fun var cleanup =>
      "    " ++ lb ++ "\n" ++
      s!"        if (!lean_is_scalar({var})) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_UNIMPLEMENTED; " ++ rb ++ "\n" ++
      s!"        int64_t {var}_v = lean_scalar_to_int64({var});\n" ++
      s!"        if ({var}_v == 0) " ++ lb ++ "\n" ++
      "            if (ret_cap - out_off < 5u) " ++ lb ++
        s!" *ret_len = out_off + 5u;{cleanup} return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
      "            ret_ptr[out_off] = 0u; out_off += 1u;\n" ++
      "            uint32_t _len = 0u;\n" ++
      "            leo4_memcpy(ret_ptr + out_off, &_len, 4); out_off += 4u;\n" ++
      "        " ++ rb ++ " else " ++ lb ++ "\n" ++
      "            if (ret_cap - out_off < 13u) " ++ lb ++
        s!" *ret_len = out_off + 13u;{cleanup} return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
      s!"            uint8_t _sign; uint64_t _limb;\n" ++
      s!"            if ({var}_v > 0) " ++ lb ++
        s!" _sign = 0u; _limb = (uint64_t){var}_v; " ++ rb ++ "\n" ++
      s!"            else " ++ lb ++
        s!" _sign = 1u; _limb = ({var}_v == (-9223372036854775807LL - 1LL))\n" ++
        s!"                       ? 9223372036854775808ULL\n" ++
        s!"                       : (uint64_t)(-{var}_v); " ++ rb ++ "\n" ++
      "            ret_ptr[out_off] = _sign; out_off += 1u;\n" ++
      "            uint32_t _len = 1u;\n" ++
      "            leo4_memcpy(ret_ptr + out_off, &_len, 4); out_off += 4u;\n" ++
      "            leo4_memcpy(ret_ptr + out_off, &_limb, 8); out_off += 8u;\n" ++
      "        " ++ rb ++ "\n" ++
      "    " ++ rb ++ "\n" ++
      cleanup
    boxExpr      := id
    unboxExpr    := id
  }

/-- Pick the unboxed C scalar type Lean's compiler uses for an
    all-nullary inductive with `numCases` ctors. Mirrors Lean's
    `impureTypeForEnum` in `Lean/Compiler/LCNF/ToImpureType.lean`:
    < 2⁸ → uint8, < 2¹⁶ → uint16, < 2³² → uint32. `numCases == 1`
    Lean represents as `tagged` (lean_object*), but our IDL emits
    `record` for single-ctor inductives, so we never call this with
    `numCases == 1`. `≥ 2³²` is `tagged` too — unreachable for any
    sane IDL. -/
private structure EnumScalar where
  /-- C type Lean uses at the FFI boundary. -/
  cType        : String
  /-- `lean_ctor_get_<kind>` / `lean_ctor_set_<kind>` infix. -/
  scalarKind   : String
  /-- Width in bytes for ctor scalar layout. -/
  size         : Nat

private def enumScalar (numCases : Nat) : Option EnumScalar :=
  if numCases < 256 then
    some { cType := "uint8_t",  scalarKind := "uint8",  size := 1 }
  else if numCases < 65536 then
    some { cType := "uint16_t", scalarKind := "uint16", size := 2 }
  else if numCases < 4294967296 then
    some { cType := "uint32_t", scalarKind := "uint32", size := 4 }
  else
    none

/-- `enum F { c₀, c₁, … }` per SPEC §10: `u32 tag` on the wire.
    Lean's IR unboxes an all-nullary inductive to the smallest
    unsigned scalar that fits its ctor count: `uint8_t` for ≤ 255,
    `uint16_t` for ≤ 65535, `uint32_t` for < 2³². At or above 2³²
    Lean falls back to `tagged` (boxed `lean_object *`) — the boxed
    path isn't wired yet, so we return `none` and let the caller fall
    back to the `LEO4_ERR_UNIMPLEMENTED` stub rather than mismatching
    Lean's actual FFI signature. Wire format stays u32 LE either way. -/
private def enumHandler (numCases : Nat) : Option TyHandler := do
  let lb := "{"
  let rb := "}"
  let s ← enumScalar numCases
  some {
    cType        := s.cType
    externCType  := s.cType
    ownsRef      := false
    scalarKind   := some s.scalarKind
    ctorScalarSz := s.size
    decodeBlock  := fun var cleanup =>
      s!"    {s.cType} {var} = 0;\n" ++
      "    " ++ lb ++ "\n" ++
      "        if (args_len - off < 4u) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      s!"        uint32_t {var}_tag;\n" ++
      s!"        leo4_memcpy(&{var}_tag, args_ptr + off, 4);\n" ++
      "        off += 4u;\n" ++
      s!"        if ({var}_tag >= {numCases}u) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      s!"        {var} = ({s.cType}){var}_tag;\n" ++
      "    " ++ rb ++ "\n"
    encodeBlock  := fun var cleanup =>
      "    " ++ lb ++ "\n" ++
      "        if (ret_cap - out_off < 4u) " ++ lb ++
        s!" *ret_len = out_off + 4u;{cleanup} return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
      s!"        uint32_t _tag = (uint32_t){var};\n" ++
      "        leo4_memcpy(ret_ptr + out_off, &_tag, 4);\n" ++
      "        out_off += 4u;\n" ++
      "    " ++ rb ++ "\n" ++
      cleanup
    boxExpr      := id
    unboxExpr    := id
  }

/-- `record R { f₁: T₁, … }` per SPEC §8: fields concatenated in
    declaration order. Lean ctor layout: object fields first (indexed
    by appearance order among objects), then scalar fields packed in
    declaration order at running byte offsets. -/
private def recordHandler (fields : Array TyHandler) : TyHandler :=
  let lb := "{"
  let rb := "}"
  -- Compute ctor layout: per-field (objSlot, scalarOff).
  let layout : Array (Nat × Nat) := Id.run do
    let mut objIdx : Nat := 0
    let mut scalarOff : Nat := 0
    let mut out : Array (Nat × Nat) := #[]
    for h in fields do
      if h.ownsRef then
        out := out.push (objIdx, 0)
        objIdx := objIdx + 1
      else
        out := out.push (0, scalarOff)
        scalarOff := scalarOff + h.ctorScalarSz
    return out
  let numObjs := fields.foldl (init := 0) fun acc h => acc + (if h.ownsRef then 1 else 0)
  let scalarSz := fields.foldl (init := 0) fun acc h => acc + h.ctorScalarSz
  { cType        := "lean_object *"
    externCType  := "lean_object *"
    ownsRef      := true
    scalarKind   := none
    ctorScalarSz := 0
    decodeBlock  := fun var cleanup => Id.run do
      let mut body : String := s!"    lean_object *{var} = NULL;\n    " ++ lb ++ "\n"
      -- LIFO cleanup of fields already decoded into local temporaries.
      let mut localCleanup : String := cleanup
      for i in [0 : fields.size] do
        let h := fields[i]!
        let v := s!"{var}_f{i}"
        body := body ++ h.decodeBlock v localCleanup
        if h.ownsRef then
          localCleanup := s!" lean_dec({v});" ++ localCleanup
      body := body ++ s!"        {var} = lean_alloc_ctor(0, {numObjs}, {scalarSz});\n"
      for i in [0 : fields.size] do
        let h := fields[i]!
        let (oslot, soff) := layout[i]!
        let v := s!"{var}_f{i}"
        body := body ++ s!"        {ctorSetCall h oslot soff var v};\n"
      body := body ++ "    " ++ rb ++ "\n"
      return body
    encodeBlock  := fun var cleanup => Id.run do
      let mut body : String := "    " ++ lb ++ "\n"
      for i in [0 : fields.size] do
        let h := fields[i]!
        let (oslot, soff) := layout[i]!
        let v := s!"{var}_f{i}"
        body := body ++ s!"        {h.cType} {v} = ({h.cType}){ctorGetCall h oslot soff var};\n"
        body := body ++ h.encodeBlock v ""
      body := body ++ "    " ++ rb ++ "\n" ++ cleanup
      return body
    boxExpr      := id
    unboxExpr    := id
  }

/-- `resource R` per SPEC §12: an opaque `u64` handle on the wire.
    A `@[leo4_resource]` structure has exactly one `UInt64` field so
    Lean's IR elaborates it as a *transparent* single-field record —
    the FFI boundary sees just a raw `uint64_t`, not a `lean_object *`
    (see `lp_<x>_parserId(uint64_t)` in the compiled `.c`). The handler
    therefore reads / writes 8 wire bytes straight into the `uint64_t`. -/
private def resourceHandler : TyHandler :=
  let lb := "{"
  let rb := "}"
  { cType        := "uint64_t"
    externCType  := "uint64_t"
    ownsRef      := false
    scalarKind   := some "uint64"
    ctorScalarSz := 8
    decodeBlock  := fun var cleanup =>
      s!"    uint64_t {var} = 0;\n" ++
      "    " ++ lb ++ "\n" ++
      "        if (args_len - off < 8u) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      s!"        leo4_memcpy(&{var}, args_ptr + off, 8);\n" ++
      "        off += 8u;\n" ++
      "    " ++ rb ++ "\n"
    encodeBlock  := fun var cleanup =>
      "    " ++ lb ++ "\n" ++
      "        if (ret_cap - out_off < 8u) " ++ lb ++
        s!" *ret_len = out_off + 8u;{cleanup} return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
      s!"        leo4_memcpy(ret_ptr + out_off, &{var}, 8);\n" ++
      "        out_off += 8u;\n" ++
      "    " ++ rb ++ "\n" ++
      cleanup
    boxExpr      := id
    unboxExpr    := id
  }

/-- Phase 8 step 2b: external-marshal nominal handler. Wire format is
opaque to the shim — it routes through C-callable Lean helpers
`leo4_marshal_<fqnSeg>_dec/_enc` (emitted in `renderLeanExports`).

Decode flow: copy the *remaining* bytes from `args_ptr + off` into a
fresh Lean `ByteArray` (`lean_alloc_sarray` + `memcpy`), call the
Lean decoder with `(buf, 0)`, unwrap `Except _ (T × Nat)` — on `.ok`
extract the `(val, new_off_nat)` pair and add the decoder's consumed
length to the shim's running offset; on `.error` return `LEO4_ERR_DECODE`.

Encode flow: build an empty `ByteArray`, call the Lean encoder with
`(val, empty_ba)`, extract the resulting `ByteArray`'s bytes via
`lean_sarray_cptr` / `lean_sarray_size`, and `memcpy` into the shim's
ret buffer with the standard short-buffer / overflow checks. -/
private def externalMarshalHandler (fqn : String) : TyHandler :=
  let lb := "{"
  let rb := "}"
  let seg := fqnSeg fqn
  let dec := s!"leo4_marshal_{seg}_dec"
  let enc := s!"leo4_marshal_{seg}_enc"
  { cType        := "lean_object *"
    externCType  := "lean_object *"
    ownsRef      := true
    scalarKind   := none
    ctorScalarSz := 0
    decodeBlock  := fun var cleanup =>
      -- Lean's `Except ε α` ctor tags: `error` = 0, `ok` = 1
      -- (Init/Prelude.lean). The decoder returns `Except _ (T × Nat)`
      -- with the `ok` payload (a `Prod`) at tag 1.
      s!"    lean_object *{var} = NULL;\n" ++
      "    " ++ lb ++ "\n" ++
      s!"        size_t remaining = args_len - off;\n" ++
      s!"        lean_object *ba = lean_alloc_sarray(1, remaining, remaining);\n" ++
      s!"        if (remaining > 0) leo4_memcpy(lean_sarray_cptr(ba), args_ptr + off, remaining);\n" ++
      s!"        lean_object *off_nat = lean_unsigned_to_nat(0);\n" ++
      s!"        lean_object *res = {dec}(ba, off_nat);\n" ++
      s!"        if (lean_obj_tag(res) != 1) " ++ lb ++
        s!" lean_dec(res); *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n" ++
      s!"        lean_object *pair = lean_ctor_get(res, 0);\n" ++
      s!"        {var} = lean_ctor_get(pair, 0);\n" ++
      s!"        lean_inc({var});\n" ++
      s!"        lean_object *new_off = lean_ctor_get(pair, 1);\n" ++
      s!"        size_t consumed = (size_t)lean_uint64_of_nat(new_off);\n" ++
      s!"        off += consumed;\n" ++
      s!"        lean_dec(res);\n" ++
      "    " ++ rb ++ "\n"
    encodeBlock  := fun var cleanup =>
      "    " ++ lb ++ "\n" ++
      s!"        lean_object *empty_ba = lean_alloc_sarray(1, 0, 0);\n" ++
      s!"        lean_object *result_ba = {enc}({var}, empty_ba);\n" ++
      s!"        size_t result_len = lean_sarray_size(result_ba);\n" ++
      s!"        if (ret_cap - out_off < result_len) " ++ lb ++
        s!" *ret_len = out_off + result_len; lean_dec(result_ba);{cleanup} return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
      s!"        leo4_memcpy(ret_ptr + out_off, lean_sarray_cptr(result_ba), result_len);\n" ++
      s!"        out_off += result_len;\n" ++
      s!"        lean_dec(result_ba);\n" ++
      "    " ++ rb ++ "\n" ++
      cleanup
    boxExpr      := id
    unboxExpr    := id
  }

/-- Look up a `UserDecl` by FQN. Linear scan — sample sets have a
handful of decls; if real consumers ship hundreds, swap in a
`Std.HashMap`. Phase 6: a `UserDecl.mutual` cluster is opened up and
its members are searched directly — handler resolution doesn't care
about the bracketing. -/
private def findUserDecl (decls : Array UserDecl) (fqn : String) : Option UserDecl :=
  let leaves : Array UserDecl := decls.flatMap (·.leaves)
  leaves.find? fun d => d.fqn == fqn

/-- Phase 6: if `fqn` is a member of any `UserDecl.mutual` cluster,
return the cluster's member helper-suffixes in source order; otherwise
return `#[]`. Used by `renderVariantHelpers` to resolve a payload's
`Cyc<i>` token to the matching peer's `leo4_enc_/dec_` helper. Today
this only handles monomorphic clusters (no generic args at the member
level) — generic-mutual clusters arrive with Phase 6-5. -/
private def findMutualPeers (decls : Array UserDecl) (fqn : String) : Array String :=
  Id.run do
    for d in decls do
      match d with
      | .mutual members =>
        if members.any (fun m => m.fqn == fqn) then
          return members.map (fun m => fqnSeg m.fqn)
      | _ => pure ()
    return #[]

/-- True iff the given variant case is in the F-step "minimum-viable"
shape that `renderVariantHelpers` knows how to emit:

  * zero fields                       — `lean_box(disc)`
  * all-Self N fields                 — recursive helper call
  * single field of `Self`            — recursive helper call (subsumed above)
  * single field of a scalar          — wire-format memcpy + ctor scalar slot
  * single field of `string`          — `leo4_decode_string` + ctor object slot
  * single field of `Cyc<i>` (mutual) — cross-call to peer's helper
    (`leo4_enc_<peerSuffix>` / `leo4_dec_<peerSuffix>`), Phase 6.

Everything richer (multi-field with non-Self payloads, single
composite like list/option/record, etc.) still falls back to the
LEO4_ERR_UNIMPLEMENTED stub. `peers` lists the helper suffix of each
member of the current mutual cluster (or empty for non-mutual). -/
private def variantCaseSupported (fields : Array IDLType) (peers : Array String) : Bool :=
  if fields.isEmpty then true
  else if fields.all (fun t => match t with | .self => true | _ => false) then true
  else if fields.size == 1 then
    match fields[0]! with
    | .string => true
    | .cyc i => i.toNat < peers.size
    | t       => (scalarCType t).isSome
  else false

private def variantAllCasesSupported
    (cases : Array (Name × Array IDLType)) (peers : Array String) : Bool :=
  cases.all fun (_, fields) => variantCaseSupported fields peers

/-- C-safe suffix encoding for a list of `IDLType` arguments. Reuses
the leo4-mangling type encoding (`SPEC/mangling.md` §2) so the
generated helper name is deterministic, byte-identical to what the
Rust side would compute, and unique per instantiation. -/
private def variantHelperSuffix (fqn : String) (args : Array IDLType) : String :=
  let safe := fqnSeg fqn
  if args.isEmpty then safe
  else safe ++ "_" ++ String.intercalate "_" (args.toList.map mangleType)

/-- Classification of a variant case for the F-step helper emitter. -/
private inductive CaseKind
  | empty                       -- 0 fields
  | allSelf (n : Nat)           -- n Self fields (subsumes single Self)
  | scalar (c : String) (sz : Nat) (kind : String)  -- single scalar (cType, wire size, ctor accessor suffix)
  | str                         -- single `string`
  | cyc (peerSuffix : String)   -- single `Cyc<i>` field, resolved to a peer's helper suffix
deriving Inhabited

private def classifyCase
    (fields : Array IDLType) (peers : Array String) : Option CaseKind :=
  if fields.isEmpty then some .empty
  else if fields.all (fun t => match t with | .self => true | _ => false) then
    some (.allSelf fields.size)
  else if h : fields.size = 1 then
    match fields[0] with
    | .string => some .str
    | .cyc i =>
        if h : i.toNat < peers.size then some (.cyc peers[i.toNat]) else none
    | t       =>
      match scalarCType t with
      | some sc => some (.scalar sc.c sc.size (scalarCtorKind sc.c))
      | none    => none
  else none

/-- Emit forward decls + definitions for the self-recursive
encoder/decoder helpers of one variant declaration. F-step shape
predicate above (`variantCaseSupported`) governs which variants
qualify; `none` is returned for anything richer (mixed-field
payload with non-Self entries, single composite field, etc.).

`peers` is the helper-suffix array of the enclosing mutual cluster
(empty for a non-mutual variant); `Cyc<i>` payload fields turn into
cross-calls to `leo4_enc_<peers[i]>` / `leo4_dec_<peers[i]>`. -/
private def renderVariantHelpers
    (fqn : String) (args : Array IDLType)
    (cases : Array (Name × Array IDLType))
    (peers : Array String := #[]) : Option String := Id.run do
  if !variantAllCasesSupported cases peers then return none
  let suffix := variantHelperSuffix fqn args
  let dec := s!"leo4_dec_{suffix}"
  let enc := s!"leo4_enc_{suffix}"
  let lb := "{"
  let rb := "}"
  -- Decoder. Per SPEC/canonical-abi.md §9 the discriminator is `u32`
  -- LE on the wire; the spec permits a u8 fast path for ≤ 256 cases
  -- but the canonical encoder MUST emit 4 bytes, so the decoder reads
  -- 4 bytes for byte-identical cross-impl conformance.
  let mut decBody : String :=
    s!"static int32_t {dec}(const uint8_t *buf, size_t buf_len, size_t *off, lean_object **out) " ++ lb ++ "\n" ++
    "    *out = NULL;\n" ++
    "    if (buf_len - *off < 4u) return LEO4_ERR_DECODE;\n" ++
    "    uint32_t disc;\n" ++
    "    leo4_memcpy(&disc, buf + *off, 4);\n" ++
    "    *off += 4u;\n"
  for i in [0 : cases.size] do
    let (_, fields) := cases[i]!
    let kind := (classifyCase fields peers).get!
    decBody := decBody ++ s!"    if (disc == {i}u) " ++ lb ++ "\n"
    match kind with
    | .empty =>
      decBody := decBody ++ s!"        *out = lean_box({i});\n" ++
                "        return LEO4_OK;\n" ++
                "    " ++ rb ++ "\n"
    | .allSelf n =>
      let mut accCleanup : String := ""
      for j in [0 : n] do
        decBody := decBody ++ s!"        lean_object *f{j};\n" ++
          s!"        " ++ lb ++ s!" int32_t st = {dec}(buf, buf_len, off, &f{j});\n" ++
          s!"          if (st) " ++ lb ++ accCleanup ++ s!" return st; " ++ rb ++ s!" " ++ rb ++ "\n"
        accCleanup := s!" lean_dec(f{j});" ++ accCleanup
      decBody := decBody ++ s!"        lean_object *r = lean_alloc_ctor({i}, {n}, 0);\n"
      for j in [0 : n] do
        decBody := decBody ++ s!"        lean_ctor_set(r, {j}, f{j});\n"
      decBody := decBody ++ "        *out = r;\n" ++
                "        return LEO4_OK;\n" ++
                "    " ++ rb ++ "\n"
    | .scalar c sz ctorKind =>
      decBody := decBody ++
        s!"        if (buf_len - *off < {sz}u) return LEO4_ERR_DECODE;\n" ++
        s!"        {c} f0; leo4_memcpy(&f0, buf + *off, {sz}); *off += {sz}u;\n" ++
        s!"        lean_object *r = lean_alloc_ctor({i}, 0, {sz});\n" ++
        s!"        lean_ctor_set_{ctorKind}(r, 0, f0);\n" ++
        "        *out = r;\n" ++
        "        return LEO4_OK;\n" ++
        "    " ++ rb ++ "\n"
    | .str =>
      decBody := decBody ++
        s!"        lean_object *f0;\n" ++
        s!"        " ++ lb ++ s!" int32_t st = leo4_decode_string(buf, buf_len, off, &f0);\n" ++
        s!"          if (st) return st; " ++ rb ++ "\n" ++
        s!"        lean_object *r = lean_alloc_ctor({i}, 1, 0);\n" ++
        s!"        lean_ctor_set(r, 0, f0);\n" ++
        "        *out = r;\n" ++
        "        return LEO4_OK;\n" ++
        "    " ++ rb ++ "\n"
    | .cyc peerSuffix =>
      decBody := decBody ++
        s!"        lean_object *f0;\n" ++
        s!"        " ++ lb ++ s!" int32_t st = leo4_dec_{peerSuffix}(buf, buf_len, off, &f0);\n" ++
        s!"          if (st) return st; " ++ rb ++ "\n" ++
        s!"        lean_object *r = lean_alloc_ctor({i}, 1, 0);\n" ++
        s!"        lean_ctor_set(r, 0, f0);\n" ++
        "        *out = r;\n" ++
        "        return LEO4_OK;\n" ++
        "    " ++ rb ++ "\n"
  decBody := decBody ++ "    return LEO4_ERR_DECODE;\n" ++ rb ++ "\n"
  -- Encoder. SPEC/canonical-abi.md §9 mandates u32 LE disc on the
  -- wire ("encoders MUST emit 4 bytes"); we write 4 bytes here so
  -- byte-identical cross-impl conformance lines up.
  let mut encBody : String :=
    s!"static int32_t {enc}(lean_object *v, uint8_t *buf, size_t cap, size_t *off, size_t *needed_out) " ++ lb ++ "\n" ++
    "    if (cap - *off < 4u) " ++ lb ++ " *needed_out = *off + 4u; return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
    "    uint32_t tag = (uint32_t)lean_obj_tag(v);\n" ++
    "    leo4_memcpy(buf + *off, &tag, 4);\n" ++
    "    *off += 4u;\n"
  for i in [0 : cases.size] do
    let (_, fields) := cases[i]!
    let kind := (classifyCase fields peers).get!
    encBody := encBody ++ s!"    if (tag == {i}u) " ++ lb ++ "\n"
    match kind with
    | .empty =>
      encBody := encBody ++ "        return LEO4_OK;\n" ++
                "    " ++ rb ++ "\n"
    | .allSelf n =>
      for j in [0 : n] do
        encBody := encBody ++ s!"        lean_object *f{j} = lean_ctor_get(v, {j});\n" ++
          s!"        " ++ lb ++ s!" int32_t st = {enc}(f{j}, buf, cap, off, needed_out);\n" ++
          s!"          if (st) return st; " ++ rb ++ "\n"
      encBody := encBody ++ "        return LEO4_OK;\n" ++
                "    " ++ rb ++ "\n"
    | .scalar c sz ctorKind =>
      encBody := encBody ++
        s!"        if (cap - *off < {sz}u) " ++ lb ++ s!" *needed_out = *off + {sz}u; return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
        s!"        {c} f0 = lean_ctor_get_{ctorKind}(v, 0);\n" ++
        s!"        leo4_memcpy(buf + *off, &f0, {sz}); *off += {sz}u;\n" ++
        "        return LEO4_OK;\n" ++
        "    " ++ rb ++ "\n"
    | .str =>
      encBody := encBody ++
        s!"        lean_object *f0 = lean_ctor_get(v, 0);\n" ++
        "        size_t need = leo4_encoded_size_string(f0);\n" ++
        "        if (cap - *off < need) " ++ lb ++ " *needed_out = *off + need; return LEO4_ERR_RETURN_BUF_TOO_SMALL; " ++ rb ++ "\n" ++
        "        leo4_write_string(f0, buf, off);\n" ++
        "        return LEO4_OK;\n" ++
        "    " ++ rb ++ "\n"
    | .cyc peerSuffix =>
      encBody := encBody ++
        s!"        lean_object *f0 = lean_ctor_get(v, 0);\n" ++
        s!"        " ++ lb ++ s!" int32_t st = leo4_enc_{peerSuffix}(f0, buf, cap, off, needed_out);\n" ++
        s!"          if (st) return st; " ++ rb ++ "\n" ++
        "        return LEO4_OK;\n" ++
        "    " ++ rb ++ "\n"
  encBody := encBody ++ "    return LEO4_ERR_DECODE;\n" ++ rb ++ "\n"
  -- Forward decls: this decl's helpers + every peer's helpers in the
  -- cluster. The peers' definitions come later in the same TU when
  -- their own `renderVariantHelpers` runs; C requires the declaration
  -- to be in scope at the cross-call site, so emit them upfront here.
  let mut fwd : String :=
    s!"static int32_t {dec}(const uint8_t *buf, size_t buf_len, size_t *off, lean_object **out);\n" ++
    s!"static int32_t {enc}(lean_object *v, uint8_t *buf, size_t cap, size_t *off, size_t *needed_out);\n"
  for peerSuffix in peers do
    let pdec := s!"leo4_dec_{peerSuffix}"
    let penc := s!"leo4_enc_{peerSuffix}"
    -- Skip self-reference (already emitted above).
    if pdec != dec then
      fwd := fwd ++
        s!"static int32_t {pdec}(const uint8_t *buf, size_t buf_len, size_t *off, lean_object **out);\n" ++
        s!"static int32_t {penc}(lean_object *v, uint8_t *buf, size_t cap, size_t *off, size_t *needed_out);\n"
  return some (fwd ++ "\n" ++ decBody ++ "\n" ++ encBody ++ "\n")

/-- `variant V` per SPEC §9, with F-step shape support. Delegates
to the per-instantiation `leo4_dec_<safe-fqn>_<args-mangle>` /
`leo4_enc_<safe-fqn>_<args-mangle>` helpers emitted at the
translation-unit level. Each (fqn, args) tuple gets its own
helper pair so substituted case payloads (different at each
instantiation) can specialise. -/
private def variantHandler (fqn : String) (args : Array IDLType) : TyHandler :=
  let suffix := variantHelperSuffix fqn args
  let dec := s!"leo4_dec_{suffix}"
  let enc := s!"leo4_enc_{suffix}"
  let lb := "{"
  let rb := "}"
  { cType        := "lean_object *"
    externCType  := "lean_object *"
    ownsRef      := true
    scalarKind   := none
    ctorScalarSz := 0
    decodeBlock  := fun var cleanup =>
      s!"    lean_object *{var} = NULL;\n" ++
      "    " ++ lb ++ "\n" ++
      s!"        int32_t st = {dec}(args_ptr, args_len, &off, &{var});\n" ++
      s!"        if (st != LEO4_OK) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return st; " ++ rb ++ "\n" ++
      "    " ++ rb ++ "\n"
    encodeBlock  := fun var cleanup =>
      "    " ++ lb ++ "\n" ++
      "        size_t needed_after = 0;\n" ++
      s!"        int32_t st = {enc}({var}, ret_ptr, ret_cap, &out_off, &needed_after);\n" ++
      "        if (st == LEO4_ERR_RETURN_BUF_TOO_SMALL) " ++ lb ++
        s!" *ret_len = needed_after;{cleanup} return st; " ++ rb ++ "\n" ++
      "        if (st != LEO4_OK) " ++ lb ++
        s!" *ret_len = 0;{cleanup} return st; " ++ rb ++ "\n" ++
      "    " ++ rb ++ "\n" ++
      cleanup
    boxExpr      := id
    unboxExpr    := id
  }

private partial def handlerFor (userDecls : Array UserDecl) : IDLType → Option TyHandler
  | t =>
    match scalarCType t with
    | some sc => some (scalarHandler sc.c sc.size)
    | none    =>
      match t with
      | .string => some stringHandler
      | .option inner => do
        let ih ← handlerFor userDecls inner
        return optionHandler ih
      | .result tOk (some tErr) => do
        let iho ← handlerFor userDecls tOk
        let ihe ← handlerFor userDecls tErr
        return resultHandler iho ihe
      | .list inner => do
        let ih ← handlerFor userDecls inner
        return listHandler ih
      | .tuple ts => do
        if h : ts.size = 2 then
          let ih1 ← handlerFor userDecls ts[0]
          let ih2 ← handlerFor userDecls ts[1]
          return binaryTupleHandler ih1 ih2
        else
          none
      | .bignat => some bignatHandler
      | .bigint => some bigintHandler
      | .enumT fqn => do
        match findUserDecl userDecls fqn with
        | some (.enumT _ cases) => enumHandler cases.size
        | _ => none
      | .record fqn args => do
        match findUserDecl userDecls fqn with
        | some (.record _ generics fields) =>
          let env ← Subst.mkEnv generics args
          let mut fieldHandlers : Array TyHandler := #[]
          for (_, fty) in fields do
            let fh ← handlerFor userDecls (Subst.substIDL env fty)
            fieldHandlers := fieldHandlers.push fh
          return recordHandler fieldHandlers
        | some (.externalMarshal _ _) =>
          -- Phase 8 step 2b: opaque-marshal nominal. Wire format
          -- is whatever the user's `LeanMarshal` instance produces;
          -- the shim routes through Lean-emitted C-callable helpers.
          some (externalMarshalHandler fqn)
        | _ => none
      | .resource fqn args => do
        match findUserDecl userDecls fqn with
        | some (.resource _ generics) =>
          -- Resource wire format is an opaque u64 handle regardless of
          -- generic args (SPEC §12); we only need an arity check.
          let _env ← Subst.mkEnv generics args
          some resourceHandler
        | _ => none
      | .variant fqn args => do
        match findUserDecl userDecls fqn with
        | some (.variant _ generics cases) =>
          let env ← Subst.mkEnv generics args
          -- After substitution, each case's payload may have changed
          -- shape — check support against the substituted form, not
          -- the original.
          let cases' := cases.map fun (n, fs) => (n, fs.map (Subst.substIDL env))
          let peers := findMutualPeers userDecls fqn
          if !variantAllCasesSupported cases' peers then none
          else some (variantHandler fqn args)
        | _ => none
      | _      => none

/-- Render one shim entry point. Signatures whose every slot has a
`TyHandler` get a real wire-format body; the rest fall back to the
`LEO4_ERR_UNIMPLEMENTED` placeholder so the link table is complete. -/
private def renderOneShim
    (cfg : Config) (userDecls : Array UserDecl) (a : ExportAnalysis) (schemaHash : Hash)
    (params : Array Emit.ParamInfo) (ret : IDLType) : String := Id.run do
  let mangled := mangle cfg.pkg cfg.iface a.fname
                  (params.map (·.encoded)) schemaHash
  let entry  := s!"leo4_call_{mangled}"
  let helper := s!"leo4_lean__{mangled}"
  let paramHs := params.map fun p => handlerFor userDecls p.encoded
  let retH?   := handlerFor userDecls ret
  let allHandled := paramHs.all (·.isSome) && retH?.isSome
  let paramTyStr := String.intercalate ", "
    (params.toList.map (fun p => cTypeOfIDL p.encoded))
  let retTyStr := cTypeOfIDL ret
  let banner := s!"// {a.fname} :: ({paramTyStr}) -> {retTyStr}\n"
  let lb := "{"
  let rb := "}"
  if !allHandled then
    return banner ++
      s!"LEO4_EXPORT int32_t {entry}(\n" ++
      "    leo4_arena_t* arena,\n" ++
      "    const uint8_t* args_ptr, size_t args_len,\n" ++
      "    uint8_t* ret_ptr, size_t ret_cap, size_t* ret_len)\n" ++
      lb ++ "\n" ++
      "    (void)arena; (void)args_ptr; (void)args_len;\n" ++
      "    (void)ret_ptr; (void)ret_cap;\n" ++
      "    *ret_len = 0;\n" ++
      "    return LEO4_ERR_UNIMPLEMENTED;\n" ++
      rb ++ "\n"
  let phs := paramHs.map (·.get!)
  let retH := retH?.get!
  -- Lean helper extern declaration. The Lean wrapper takes a dummy
  -- `Unit` (passed as `lean_box(0)`) when there are no real params,
  -- so that Lean's code generator emits it as a function rather than
  -- a `const lean_object*` constant (renderOneWrapper does the same
  -- on the Lean side).
  let externArgsList :=
    if phs.isEmpty then ["lean_object *"] else phs.toList.map (·.externCType)
  let externDecl :=
    s!"extern {retH.externCType} {helper}(" ++
    String.intercalate ", " externArgsList ++ ");\n"
  -- Decode each param; carry an LIFO cleanup string of dec_refs for
  -- already-decoded owned args, threaded into each subsequent
  -- failure path.
  let mut decode := ""
  let mut cleanup := ""
  for i in [0 : phs.size] do
    let h := phs[i]!
    decode := decode ++ h.decodeBlock s!"a{i}" cleanup
    if h.ownsRef then
      cleanup := s!" lean_dec(a{i});" ++ cleanup
  -- After all decodes, the buffer must be fully consumed.
  let lenCheck :=
    "    if (off != args_len) " ++ lb ++
      s!" *ret_len = 0;{cleanup} return LEO4_ERR_DECODE; " ++ rb ++ "\n"
  -- Invocation: ownership of decoded `lean_object *` args transfers to
  -- the Lean wrapper, so the decode-time `cleanup` is *not* run after a
  -- successful call.
  let argApp := String.intercalate ", "
    ((List.range phs.size).map (fun i => s!"a{i}"))
  let invoke :=
    if phs.isEmpty then s!"{helper}(lean_box(0))" else s!"{helper}({argApp})"
  -- Post-call cleanup for the return value (only owned types need it).
  let retCleanup := if retH.ownsRef then " lean_dec(r);" else ""
  let body :=
    s!"LEO4_EXPORT int32_t {entry}(\n" ++
    "    leo4_arena_t* arena,\n" ++
    "    const uint8_t* args_ptr, size_t args_len,\n" ++
    "    uint8_t* ret_ptr, size_t ret_cap, size_t* ret_len)\n" ++
    lb ++ "\n" ++
    "    (void)arena;\n" ++
    "    size_t off = 0;\n" ++
    decode ++
    lenCheck ++
    s!"    {retH.externCType} r = {invoke};\n" ++
    "    size_t out_off = 0;\n" ++
    retH.encodeBlock "r" retCleanup ++
    "    *ret_len = out_off;\n" ++
    "    return LEO4_OK;\n" ++
    rb ++ "\n"
  return banner ++ externDecl ++ body

/-- Render the full `<pkg>.leo4-shim.c` text. One translation unit
contains every shim entry point for the package; the matching
`leo4_lean__<mangled>` helpers live in the user's `.olean`-derived
object files and are resolved at `leanc` link time (W7-2b). -/
def renderShimSource
    (cfg : Config) (userDecls : Array UserDecl)
    (analyses : Array ExportAnalysis) (schemaHash : Hash) : String := Id.run do
  let banner : String :=
    "// Auto-generated by `leo4plugin` (W7-2a). Do not edit by hand.\n" ++
    "//\n" ++
    "// One translation unit per package; every `@[leo4_export]` ×\n" ++
    "// monomorphisation gets one `leo4_call_<mangled>` entry point\n" ++
    "// (canonical-buffer ABI; SPEC/canonical-abi.md §14). The matching\n" ++
    "// Lean-side helper is `leo4_lean__<mangled>` (SPEC/mangling.md §6).\n" ++
    "//\n" ++
    "// Coverage: scalar primitives and `string` are wired end-to-end\n" ++
    "// (W7-2a + W7-2c-i). Other composites and nominal types still\n" ++
    "// return LEO4_ERR_UNIMPLEMENTED; coverage grows in W7-2c-ii..iv\n" ++
    "// (composites) and W7-2d (nominal types).\n" ++
    "//\n" ++
    s!"// Schema hash : {schemaHash.toBase32lc}\n" ++
    s!"// Package     : {cfg.pkg}\n" ++
    s!"// Interface   : {cfg.iface}\n" ++
    "\n" ++
    "#include <lean/lean.h>\n" ++
    "#include <stdint.h>\n" ++
    "#include <stddef.h>\n" ++
    "\n" ++
    "/* `memcpy` via the compiler builtin so the translation unit\n" ++
    "   compiles under leanc's bundled clang without depending on\n" ++
    "   the host's system <string.h> search path. */\n" ++
    "#define leo4_memcpy __builtin_memcpy\n" ++
    "\n" ++
    "/* leanc compiles with `-fvisibility=hidden` (see `leanc --print-cflags`),\n" ++
    "   so the shim's entry points need an explicit default-visibility\n" ++
    "   attribute to land in the .so's dynamic symbol table. */\n" ++
    "#if defined(_WIN32) || defined(__CYGWIN__)\n" ++
    "#define LEO4_EXPORT __declspec(dllexport)\n" ++
    "#elif defined(__GNUC__) || defined(__clang__)\n" ++
    "#define LEO4_EXPORT __attribute__((visibility(\"default\")))\n" ++
    "#else\n" ++
    "#define LEO4_EXPORT\n" ++
    "#endif\n" ++
    "\n" ++
    "/* Status codes per SPEC/canonical-abi.md sec 13. */\n" ++
    "#define LEO4_OK                          0\n" ++
    "#define LEO4_ERR_DECODE                  0x00000001\n" ++
    "#define LEO4_ERR_HANDSHAKE_MISMATCH      0x00000005\n" ++
    "#define LEO4_ERR_RETURN_BUF_TOO_SMALL    0x00000007\n" ++
    "#define LEO4_ERR_UNIMPLEMENTED           0x00000064\n" ++
    "\n" ++
    "/* ABI version per SPEC/canonical-abi.md sec 14. */\n" ++
    "#define LEO4_ABI_VERSION                 1u\n" ++
    "\n" ++
    "/* Opaque arena pointer; the W7-2a scalar path doesn't touch it,\n" ++
    "   but the §14 signature reserves the slot. */\n" ++
    "typedef void leo4_arena_t;\n" ++
    "\n" ++
    "/* ---------- decode/encode helpers (W7-2c+) ---------- */\n" ++
    "\n" ++
    "/* `string` per SPEC/canonical-abi.md §3:\n" ++
    "       len:u32 | utf8 bytes\n" ++
    "   `lean_mk_string_from_bytes` validates UTF-8 and returns a\n" ++
    "   fresh, owned `lean_object*`. The wire byte count equals\n" ++
    "   `lean_string_size(o) - 1` because lean_string_object's m_size\n" ++
    "   includes the trailing NUL terminator. */\n" ++
    "static inline int32_t leo4_decode_string(\n" ++
    "    const uint8_t *buf, size_t buf_len, size_t *off, lean_object **out)\n" ++
    "{\n" ++
    "    if (buf_len - *off < 4u) return LEO4_ERR_DECODE;\n" ++
    "    uint32_t slen;\n" ++
    "    leo4_memcpy(&slen, buf + *off, 4);\n" ++
    "    *off += 4u;\n" ++
    "    if (buf_len - *off < (size_t)slen) return LEO4_ERR_DECODE;\n" ++
    "    *out = lean_mk_string_from_bytes((char const *)(buf + *off), (size_t)slen);\n" ++
    "    *off += (size_t)slen;\n" ++
    "    return LEO4_OK;\n" ++
    "}\n" ++
    "\n" ++
    "static inline size_t leo4_encoded_size_string(lean_object *s) {\n" ++
    "    return 4u + (size_t)(lean_string_size(s) - 1u);\n" ++
    "}\n" ++
    "\n" ++
    "static inline void leo4_write_string(\n" ++
    "    lean_object *s, uint8_t *buf, size_t *off)\n" ++
    "{\n" ++
    "    size_t plen = lean_string_size(s) - 1u;\n" ++
    "    uint32_t slen32 = (uint32_t)plen;\n" ++
    "    leo4_memcpy(buf + *off, &slen32, 4);\n" ++
    "    *off += 4u;\n" ++
    "    leo4_memcpy(buf + *off, lean_string_cstr(s), plen);\n" ++
    "    *off += plen;\n" ++
    "}\n" ++
    "\n" ++
    -- Schema handshake entry point per SPEC/canonical-abi.md sec 15.
    -- Compiled-in 8-byte schema hash (big-endian view of the FNV-1a-64
    -- digest, same byte order as the `schema_hash_bytes` field in
    -- `<pkg>.leo4-handshake`).
    "/* Schema handshake. SPEC/canonical-abi.md sec 15. The hash is\n" ++
    "   the 8-byte big-endian view of the FNV-1a-64 digest baked in at\n" ++
    "   shim build time; the loader passes the bytes it parsed from\n" ++
    "   <pkg>.leo4-handshake. ABI-version mismatch and hash mismatch\n" ++
    "   both report LEO4_ERR_HANDSHAKE_MISMATCH. mismatch_detail_out\n" ++
    "   is reserved for human-readable explanation; for now we leave\n" ++
    "   it untouched (loader formats its own error message). */\n" ++
    (Id.run do
      let bytes := (Array.range 8).map fun i =>
        let shift : Nat := (7 - i) * 8
        let b : Nat := (schemaHash.value >>> shift.toUInt64).toNat &&& 0xff
        let hi := Nat.toDigits 16 (b >>> 4) |>.head!
        let lo := Nat.toDigits 16 (b &&& 0xf) |>.head!
        s!"0x{hi}{lo}"
      let bytesStr := String.intercalate ", " bytes.toList
      pure (
        "static const uint8_t leo4_schema_hash_be[8] = {\n" ++
        s!"    {bytesStr}\n" ++
        "};\n" ++
        "\n" ++
        "LEO4_EXPORT int32_t leo4_handshake(\n" ++
        "    const uint8_t *expected_schema_hash,\n" ++
        "    uint32_t expected_abi_version,\n" ++
        "    char *mismatch_detail_out, size_t detail_cap)\n" ++
        "{\n" ++
        "    (void)mismatch_detail_out; (void)detail_cap;\n" ++
        "    if (expected_abi_version != LEO4_ABI_VERSION) return LEO4_ERR_HANDSHAKE_MISMATCH;\n" ++
        "    for (int i = 0; i < 8; i++) {\n" ++
        "        if (leo4_schema_hash_be[i] != expected_schema_hash[i]) return LEO4_ERR_HANDSHAKE_MISMATCH;\n" ++
        "    }\n" ++
        "    return LEO4_OK;\n" ++
        "}\n" ++
        "\n"))
  -- Per-(variant decl × generic instantiation) helper function pair
  -- (W7-2d-ii + F-step generalisation). Each (fqn, args) pair gets
  -- one helper, so substituted case payloads — different at each
  -- instantiation of a generic variant — get specialised emit.
  -- Variant instantiations are collected by walking every entry
  -- point's parameter / return IDLType tree and recording the
  -- unique (fqn, args) tuples encountered.
  let rec collectVariants : IDLType → Array (String × Array IDLType) → Array (String × Array IDLType)
    | t, acc =>
      let acc := match t with
        | .variant fqn args =>
          if acc.any (fun (f, a) => f == fqn && a == args) then acc
          else acc.push (fqn, args)
        | _ => acc
      match t with
      | .variant _ args | .record _ args | .resource _ args | .selfApp args =>
        args.foldl (init := acc) (fun a x => collectVariants x a)
      | .list inner | .option inner | .io inner =>
        collectVariants inner acc
      | .result tOk tErr =>
        let acc := collectVariants tOk acc
        match tErr with
        | some e => collectVariants e acc
        | none   => acc
      | .tuple ts => ts.foldl (init := acc) (fun a x => collectVariants x a)
      | _ => acc
  let mut variantInsts : Array (String × Array IDLType) := #[]
  for a in analyses do
    for (_gargs, params, ret) in a.resolved do
      for p in params do variantInsts := collectVariants p.encoded variantInsts
      variantInsts := collectVariants ret variantInsts
  let mut variantHelpers : String := ""
  for (fqn, args) in variantInsts do
    match findUserDecl userDecls fqn with
    | some (.variant _ generics cases) =>
      match Subst.mkEnv generics args with
      | some env =>
        let cases' := cases.map fun (n, fs) => (n, fs.map (Subst.substIDL env))
        let peers := findMutualPeers userDecls fqn
        if let some hs := renderVariantHelpers fqn args cases' peers then
          variantHelpers := variantHelpers ++ hs
      | none => ()
    | _ => ()
  -- Phase 8 step 2b: forward-declare the Lean-side external-marshal
  -- helpers (`leo4_marshal_<seg>_dec/_enc`) at the top of the TU so
  -- every shim entry point's body can call them. Each helper takes
  -- two `lean_object *` args (decode: `(buf, off_nat) → Except _ (T,
  -- Nat)`; encode: `(val, buf) → ByteArray`) and returns a
  -- `lean_object *`.
  let mut externalDecls : String := ""
  for d in userDecls.flatMap (·.leaves) do
    match d with
    | .externalMarshal fqn _ =>
      let seg := fqnSeg fqn
      externalDecls := externalDecls ++
        s!"extern lean_object * leo4_marshal_{seg}_dec(lean_object *, lean_object *);\n" ++
        s!"extern lean_object * leo4_marshal_{seg}_enc(lean_object *, lean_object *);\n"
    | _ => pure ()
  let banner := banner ++ variantHelpers ++ externalDecls ++ "\n"
  let mut body := ""
  for a in analyses do
    for (_gargs, params, ret) in a.resolved do
      body := body ++ renderOneShim cfg userDecls a schemaHash params ret ++ "\n"
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
  -- Phase 6: deduplicate by `iv.all` so mutually-recursive inductives
  -- are grouped into one `UserDecl.mutual` cluster rather than emitted
  -- as N independent decls with cross-references via FQN.
  let userDeclsIO : IO (Array UserDecl) := do
    let action : MetaM (Array UserDecl) := do
      let env ← getEnv
      let mut out : Array UserDecl := #[]
      let mut emitted : Std.HashSet Name := {}
      for fqn in allUserTypes do
        let nm := fqn.toName
        if emitted.contains nm then continue
        let groupMembers : Array Name :=
          match env.find? nm with
          | some (.inductInfo iv) =>
            if iv.all.length > 1 then iv.all.toArray else #[nm]
          | _ => #[nm]
        if groupMembers.size > 1 then
          match ← walkMutualGroup env {} groupMembers with
          | some d => out := out.push d
          | none   => pure ()
          for m in groupMembers do
            emitted := emitted.insert m
        else
          match ← walkUserDecl env {} nm with
          | some d => out := out.push d
          | none   => pure ()
          emitted := emitted.insert nm
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

  let resourceCount : Nat := (userDecls.flatMap (·.leaves)).foldl (init := 0) fun acc d =>
    match d with
    | .resource _ _ => acc + 1
    | _ => acc
  let ifaceSummary : Emit.InterfaceSummary :=
    { name := cfg.iface
      function_count := analyses.size
      resource_count := resourceCount }

  -- Wrapper init symbol: file stem of the emitted wrapper, run through
  -- Lean's C-identifier escape rule. The wrapper file is
  -- `<normalizedPkg>.leo4-exports.lean` (emitted later in this
  -- function, but the path is deterministic so we compute the symbol
  -- name up-front and attach it to the handshake bundle).
  let wrapperInitSymbol :=
    let stemForMangle := s!"{normalizePackageSegment cfg.pkg}.leo4-exports"
    s!"initialize_{Leo4Plugin.manglerLeanModuleName stemForMangle}"
  let bundle : Emit.EmitBundle := {
    package            := cfg.pkg
    targetModule       := cfg.target.toString
    wrapperInitSymbol  := wrapperInitSymbol
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
  let leanExportsText := renderLeanExports cfg userDecls analyses schemaHash
  IO.FS.writeFile leanExportsPath leanExportsText
  IO.println s!"wrote {leanExportsPath}"

  -- Emit the C shim source (W7-2a). One translation unit per package
  -- carrying every `leo4_call_<mangled>` entry point. Scalar-only
  -- instantiations are wired through to `leo4_lean__<mangled>`; the
  -- rest get LEO4_ERR_UNIMPLEMENTED placeholders (filled in
  -- W7-2c/W7-2d). The `leanc`-driven compile of this source into
  -- `<pkg>.leo4-shim.so` lands in W7-2b.
  let shimPath := cfg.outDir / s!"{stem}.leo4-shim.c"
  let shimText := renderShimSource cfg userDecls analyses schemaHash
  IO.FS.writeFile shimPath shimText
  IO.println s!"wrote {shimPath}"

  -- W7-2b: shim build orchestration. Hand `Leo4.Build.BuildCfg` to
  -- either the user's `Build.lean` (override path) or this process's
  -- in-place `Leo4.Build.defaultLink` (default path). The plugin's
  -- default path uses exactly the same helpers a user's `Build.lean`
  -- would call into, so the two paths are coherent by construction
  -- (M ⊂ L).
  let sysroot ← Lean.findSysroot
  let leancPath : System.FilePath := sysroot / "bin" / "leanc"
  let mangledBodies : Array String :=
    manglingEntries.foldl (init := #[]) fun acc e =>
      acc ++ e.instantiations.map (·.mangled)
  -- Paths in the cfg JSON are absolute so that user `Build.lean`
  -- scripts (and `checkExports`, etc.) work regardless of the cwd
  -- they're invoked from.
  let outDirAbs       ← IO.FS.realPath cfg.outDir
  let shimPathAbs     ← IO.FS.realPath shimPath
  let leanExportsAbs  ← IO.FS.realPath leanExportsPath
  let manglingPathAbs ← IO.FS.realPath manglingPath
  -- Infer the user package root by walking up from `outDir` until a
  -- `.lake` directory turns up; the root is its parent. Falls back to
  -- cwd when no `.lake` is anywhere on the path (shouldn't happen in
  -- a normal Lake invocation).
  let pkgRoot : System.FilePath ← Id.run do
    let mut p := outDirAbs
    for _ in [0 : 10] do
      if p.fileName == some ".lake" then
        match p.parent with
        | some parent => return pure parent
        | none        => break
      match p.parent with
      | some parent => p := parent
      | none        => break
    return IO.currentDir
  let buildCfg : Leo4.Build.BuildCfg := {
    pkg            := cfg.pkg
    iface          := cfg.iface
    schemaHash     := schemaHash.toBase32lc
    pkgRoot        := pkgRoot
    outDir         := outDirAbs
    outName        := s!"{stem}.leo4-shim"
    shimSrc        := shimPathAbs
    wrapperSrc     := leanExportsAbs
    manglingPath   := manglingPathAbs
    mangledBodies  := mangledBodies
    leancPath      := leancPath
  }
  let cfgJsonPath := cfg.outDir / s!"{stem}.leo4-build-cfg.json"
  Leo4.Build.BuildCfg.save cfgJsonPath buildCfg
  IO.println s!"wrote {cfgJsonPath}"

  -- Build-script discovery order:
  --   1. <cwd>/Build.lean                              (project override)
  --   2. ${HOME}/.local/leo4/DefaultBuild.lean         (per-user default)
  --   3. /etc/leo4/DefaultBuild.lean                   (system default)
  --   4. in-process `Leo4.Build.defaultLink` fallback  (fresh checkout)
  --
  -- All four go through the same `BuildCfg` JSON, so the user can move
  -- a `Build.lean` between scopes without changing its `main`.
  let mut candidates : Array (String × System.FilePath) :=
    #[("project Build.lean", pkgRoot / "Build.lean")]
  if let some home ← IO.getEnv "HOME" then
    let userDef : System.FilePath :=
      (System.FilePath.mk home) / ".local" / "leo4" / "DefaultBuild.lean"
    candidates := candidates.push ("user DefaultBuild.lean", userDef)
  candidates := candidates.push
    ("system DefaultBuild.lean", ("/etc/leo4/DefaultBuild.lean" : System.FilePath))
  let mut chosen : Option (String × System.FilePath) := none
  for (label, path) in candidates do
    if chosen.isNone && (← path.pathExists) then
      chosen := some (label, path)
  match chosen with
  | some (label, path) =>
    IO.println s!"using {label}: {path}"
    -- `lean --run` forwards every positional argument after the script
    -- path verbatim — including a literal `--`. We omit the separator
    -- so `Build.lean`'s `main args` sees `cfgJsonPath` at `args[0]!`,
    -- matching the unix tradition `main` user code typically expects.
    let res ← IO.Process.output {
      cmd := "lake"
      args := #["env", "lean", "--run", path.toString, cfgJsonPath.toString]
    }
    IO.print res.stdout
    IO.eprint res.stderr
    if res.exitCode != 0 then
      throw <| IO.userError s!"{label} exited with code {res.exitCode}"
  | none =>
    IO.println "no Build.lean found in project/user/system scopes; running in-process Leo4.Build.defaultLink"
    Leo4.Build.defaultLink buildCfg
    IO.println s!"linked → {buildCfg.defaultSoPath}"

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
