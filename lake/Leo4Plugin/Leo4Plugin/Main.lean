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
      let innerCleanup := s!" lean_dec_ref({var}_arr);" ++ cleanup
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
        (if ih1.ownsRef then s!" lean_dec_ref({v1});" else "") ++ cleanup
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

private partial def handlerFor : IDLType → Option TyHandler
  | t =>
    match scalarCType t with
    | some sc => some (scalarHandler sc.c sc.size)
    | none    =>
      match t with
      | .string => some stringHandler
      | .option inner => do
        let ih ← handlerFor inner
        return optionHandler ih
      | .result tOk (some tErr) => do
        let iho ← handlerFor tOk
        let ihe ← handlerFor tErr
        return resultHandler iho ihe
      | .list inner => do
        let ih ← handlerFor inner
        return listHandler ih
      | .tuple ts => do
        if h : ts.size = 2 then
          let ih1 ← handlerFor ts[0]
          let ih2 ← handlerFor ts[1]
          return binaryTupleHandler ih1 ih2
        else
          none
      | .bignat => some bignatHandler
      | .bigint => some bigintHandler
      | _      => none

/-- Render one shim entry point. Signatures whose every slot has a
`TyHandler` get a real wire-format body; the rest fall back to the
`LEO4_ERR_UNIMPLEMENTED` placeholder so the link table is complete. -/
private def renderOneShim
    (cfg : Config) (a : ExportAnalysis) (schemaHash : Hash)
    (params : Array Emit.ParamInfo) (ret : IDLType) : String := Id.run do
  let mangled := mangle cfg.pkg cfg.iface a.fname
                  (params.map (·.encoded)) schemaHash
  let entry  := s!"leo4_call_{mangled}"
  let helper := s!"leo4_lean__{mangled}"
  let paramHs := params.map fun p => handlerFor p.encoded
  let retH?   := handlerFor ret
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
  -- Lean helper extern declaration.
  let externArgsList :=
    if phs.isEmpty then ["void"] else phs.toList.map (·.externCType)
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
      cleanup := s!" lean_dec_ref(a{i});" ++ cleanup
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
    if phs.isEmpty then s!"{helper}()" else s!"{helper}({argApp})"
  -- Post-call cleanup for the return value (only owned types need it).
  let retCleanup := if retH.ownsRef then " lean_dec_ref(r);" else ""
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
    (cfg : Config) (analyses : Array ExportAnalysis) (schemaHash : Hash) : String := Id.run do
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
    "/* Status codes per SPEC/canonical-abi.md §13. */\n" ++
    "#define LEO4_OK                        0\n" ++
    "#define LEO4_ERR_DECODE                0x00000001\n" ++
    "#define LEO4_ERR_RETURN_BUF_TOO_SMALL  0x00000007\n" ++
    "#define LEO4_ERR_UNIMPLEMENTED         0x00000064\n" ++
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
