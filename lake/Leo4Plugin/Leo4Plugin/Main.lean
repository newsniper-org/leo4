-- Leo4Plugin.Main — plugin entry point.
--
-- Responsibility (LEO4-DESIGN.md §7, CLAUDE.md "How to Work With the Lake Plugin"):
--   1. importModules on the user package's compiled .olean files
--   2. find every @[leo4_export] definition
--   3. (Phase 2) read its @[leo4_specialize_when] payload and enumerate the admit-set
--   4. (Phase 2) emit IDL, mangling table, handshake, shim source
--
-- Steps 3-4 are stubs in this Week 1 cut. Steps 1-2 are functional and replace
-- spike/lake-hook/SpikePlugin.lean.
--
-- The plugin is invoked as `lake exe leo4plugin <user-module>`. We do *not* hook
-- Lake.Module.recBuildLean; see SPIKE-0-FINDINGS.md for why.

import Lean
import Leo4

open Lean Lean.Meta

namespace Leo4Plugin

/-- Walk a Pi telescope and return the head of the final body. -/
partial def conclHead : Expr → Expr
  | .forallE _ _ body _ => conclHead body
  | e => e.getAppFn

/-- All `@[leo4_export]`-tagged constants visible in `env`.

Walks each imported module's serialized tag entries; sub-ms for typical packages
(replaces the spike's O(|env.constants|) fold). -/
def gatherExports (env : Environment) : Array Name := Id.run do
  let ext := Leo4.leo4ExportAttr.ext
  let mut acc : Array Name := #[]
  for modIdx in [0 : env.allImportedModuleNames.size] do
    acc := acc ++ ext.getModuleEntries env modIdx
  -- Plus anything tagged in the *current* module (none, since the plugin
  -- exe doesn't define exports — but keep the path for correctness).
  let curMod := ext.getState (asyncDecl := .anonymous) env
  curMod.foldl (init := acc) fun a n => a.push n

/-- Constants registered as global instances whose conclusion head is `cls`.

This is the closed-world instance enumeration that the (α′) algorithm consumes;
for now we expose it on the plugin side so admit-sets for `[ToString T]`-shaped
constraints can be derived. -/
def gatherInstancesOf (env : Environment) (cls : Name) : Array Name :=
  let st := Meta.instanceExtension.getState env
  st.instanceNames.foldl (init := (#[] : Array Name)) fun acc n _ =>
    match env.find? n with
    | none => acc
    | some info =>
      if (conclHead info.type).isConstOf cls then acc.push n else acc

/-- Format one tenth-of-a-millisecond nanosecond delta. -/
def msStr (ns : Nat) : String :=
  let f : Float := ns.toFloat / 1e6
  let i : Nat := (f * 10).toUInt64.toNat
  s!"{i / 10}.{i % 10}ms"

private structure BinderKindReport where
  kind     : String   -- "[inst]" / "{impl}" / "(expl)" / "⦃sImpl⦄"
  userName : Name
  typePP   : Format

/-- Pretty-print the type of `n` and the binder kinds in its signature.
The instance-implicit binders are the constraint payload the real (α′)
algorithm enumerates against; for Week 1 we just report them. -/
def reportExport (n : Name) : MetaM Unit := do
  let env ← getEnv
  let some info := env.find? n | return
  let ppType ← Meta.ppExpr info.type
  IO.println s!"  • {n}"
  IO.println s!"      type: {ppType}"
  forallTelescopeReducing info.type fun args body => do
    if args.size == 0 then
      IO.println "      binders: (none)"
      return
    let mut rows : Array BinderKindReport := #[]
    for a in args do
      let ld ← a.fvarId!.getDecl
      let ppTy ← Meta.ppExpr ld.type
      let kind :=
        if ld.binderInfo.isInstImplicit  then "[inst]"
        else if ld.binderInfo.isStrictImplicit then "⦃sImpl⦄"
        else if ld.binderInfo.isImplicit then "{impl}"
        else "(expl)"
      rows := rows.push { kind, userName := ld.userName, typePP := ppTy }
    IO.println "      binders:"
    for r in rows do
      IO.println s!"        - {r.kind} {r.userName} : {r.typePP}"
    let ppBody ← Meta.ppExpr body
    IO.println s!"      result: {ppBody}"

/-- Run the report as one MetaM action so `ppExpr` and `forallTelescopeReducing`
both work against the freshly imported environment. -/
def runReport (env : Environment) : IO Unit := do
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
  let coreCtx : Core.Context := { fileName := "<leo4plugin>", fileMap := FileMap.ofString "" }
  let coreSt  : Core.State   := { env := env }
  let core : CoreM Unit := action.run' {} {}
  let _ ← core.toIO' coreCtx coreSt

/-- Plugin entry point invoked by `lake exe leo4plugin <user-module>`. -/
def main (args : List String) : IO UInt32 := do
  let target : Name :=
    match args with
    | [] => `Sample
    | a :: _ => a.toName
  IO.println s!"leo4plugin: loading module {target}"

  -- Lake propagates LEAN_PATH via `lake env`/`lake exe`, but `importModules`
  -- consults `searchPathRef`. Initialise it from LEAN_PATH (and sysroot).
  Lean.initSearchPath (← Lean.findSysroot)

  let t0 ← IO.monoNanosNow
  let env ← Lean.importModules
    (imports := #[{ module := target }, { module := `Leo4 }])
    (opts := {}) (trustLevel := 0) (loadExts := true)
  let t1 ← IO.monoNanosNow

  let t2 ← IO.monoNanosNow
  let exports := gatherExports env
  let t3 ← IO.monoNanosNow

  IO.println s!"env: {env.allImportedModuleNames.size} imported modules"
  IO.println s!"importModules (loadExts=true): {msStr (t1 - t0)}"
  IO.println s!"gatherExports (per-module ext.getModuleEntries): {msStr (t3 - t2)}"
  IO.println s!"  ({exports.size} exports found)"

  let t4 ← IO.monoNanosNow
  runReport env
  let t5 ← IO.monoNanosNow

  IO.println "-- timings --"
  IO.println s!"importModules : {msStr (t1 - t0)}"
  IO.println s!"gatherExports : {msStr (t3 - t2)}"
  IO.println s!"report (Meta) : {msStr (t5 - t4)}"
  IO.println s!"total (wall)  : {msStr (t5 - t0)}"
  return 0

end Leo4Plugin

/-- C-callable entry point so Lake's `lean_exe` can find `main`. -/
def main (args : List String) : IO UInt32 := Leo4Plugin.main args
