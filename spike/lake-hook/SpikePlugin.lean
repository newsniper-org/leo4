-- Spike 0 plugin entry point.
--
-- Steps (each timed):
--   1. importModules #[Sample, Leo4Export]   (loadExts := true)
--   2. walk env.constants, collect @[leo4_export] decls via TagAttribute.hasTag
--   3. for each export, extract argument binder kinds (instance / implicit / explicit)
--      and report any instance-implicit constraints
--   4. enumerate global instances whose conclusion head is `ToString`
--      (proxy for admit-set enumeration in the real (α′) algorithm)

import Lean
import Leo4Export

open Lean Lean.Meta

/-- Walk a Pi telescope and return the head of the final body. -/
partial def conclHead : Expr → Expr
  | .forallE _ _ body _ => conclHead body
  | e => e.getAppFn

/-- All `@[leo4_export]`-tagged constants in `env`.

Naive O(|env.constants|) scan — for the spike this is the upper bound on cost.
A production plugin would read `leo4ExportAttr.ext.getModuleEntries env modIdx`
per module instead. -/
def gatherExports (env : Environment) : Array Name :=
  env.constants.fold (init := #[]) fun acc n _ =>
    if Leo4.leo4ExportAttr.hasTag env n then acc.push n else acc

/-- Constants registered as global instances whose conclusion head is `cls`. -/
def gatherInstancesOf (env : Environment) (cls : Name) : Array Name :=
  let st := Meta.instanceExtension.getState env
  st.instanceNames.foldl (init := (#[] : Array Name)) fun acc n _ =>
    match env.find? n with
    | none => acc
    | some info =>
      if (conclHead info.type).isConstOf cls then acc.push n else acc

/-- Pretty-print the type of `n` inside MetaM and emit a one-line summary
plus per-binder kinds.  The instance-implicit binders are the constraints
the (α′) algorithm needs to enumerate. -/
def reportExport (n : Name) : MetaM Unit := do
  let env ← getEnv
  let some info := env.find? n | return
  let ppType ← Meta.ppExpr info.type
  IO.println s!"  • {n}"
  IO.println s!"      type: {ppType}"
  forallTelescopeReducing info.type fun args body => do
    if args.size == 0 then
      IO.println s!"      binders: (none)"
    else
      let mut kinds : Array String := #[]
      for a in args do
        let ld ← a.fvarId!.getDecl
        let ty ← Meta.ppExpr ld.type
        let tag :=
          if ld.binderInfo.isInstImplicit then "[inst]"
          else if ld.binderInfo.isImplicit then "{impl}"
          else if ld.binderInfo.isStrictImplicit then "⦃sImpl⦄"
          else "(expl)"
        kinds := kinds.push s!"{tag} {ld.userName} : {ty}"
      IO.println s!"      binders:"
      for k in kinds do
        IO.println s!"        - {k}"
      let ppBody ← Meta.ppExpr body
      IO.println s!"      result: {ppBody}"

def msStr (ns : Nat) : String :=
  let f : Float := ns.toFloat / 1e6
  -- one-decimal-place truncation, good enough for spike timing
  let i : Nat := (f * 10).toUInt64.toNat
  s!"{i / 10}.{i % 10}ms"

/-- Run the report as one MetaM action so that `Meta.ppExpr` and `forallTelescopeReducing`
both work against the freshly imported environment. -/
def runReportMeta (env : Environment) : IO Unit := do
  let action : MetaM Unit := do
    let exports := gatherExports (← getEnv)
    IO.println s!"-- @[leo4_export] decls: {exports.size} --"
    for n in exports do
      reportExport n
    let cls : Name := `ToString
    let t0 ← IO.monoNanosNow
    let insts := gatherInstancesOf (← getEnv) cls
    let t1 ← IO.monoNanosNow
    IO.println s!"-- admit-set proxy: instances of `{cls}`: {insts.size} (enum {msStr (t1 - t0)}) --"
    for n in insts do
      IO.println s!"  • {n}"
  let coreCtx : Core.Context := { fileName := "<spike>", fileMap := FileMap.ofString "" }
  let coreSt  : Core.State   := { env := env }
  let metaCtx : Meta.Context := {}
  let metaSt  : Meta.State   := {}
  let core : CoreM Unit := action.run' metaCtx metaSt
  let _ ← core.toIO' coreCtx coreSt

def main (args : List String) : IO UInt32 := do
  let target : Name :=
    match args with
    | [] => `Sample
    | a :: _ => a.toName
  IO.println s!"target module: {target}"

  -- Lake propagates LEAN_PATH via `lake env`/`lake exe`, but `importModules`
  -- consults `searchPathRef`. Initialise it from LEAN_PATH (and from LEAN_SYSROOT
  -- if available) before importing.
  Lean.initSearchPath (← Lean.findSysroot)

  let t0 ← IO.monoNanosNow
  let env ← Lean.importModules
    (imports := #[{ module := target }, { module := `Leo4Export }])
    (opts := {}) (trustLevel := 0) (loadExts := true)
  let t1 ← IO.monoNanosNow
  IO.println s!"importModules (loadExts=true): {msStr (t1 - t0)}"
  let nConsts := env.constants.fold (init := 0) (fun a _ _ => a + 1)
  IO.println s!"env: {env.allImportedModuleNames.size} imported modules, {nConsts} constants"

  let t2 ← IO.monoNanosNow
  runReportMeta env
  let t3 ← IO.monoNanosNow
  IO.println s!"-- timings --"
  IO.println s!"importModules            : {msStr (t1 - t0)}"
  IO.println s!"report (walk + instances): {msStr (t3 - t2)}"
  IO.println s!"total (wall)             : {msStr (t3 - t0)}"
  return 0
