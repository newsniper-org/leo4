-- Leo4Plugin.Emit — write `<pkg>.leo4-handshake` and `<pkg>.leo4-mangling`.
--
-- Format: `SPEC/handshake.md`.  Atomic emission per SPEC §"Atomic Emission":
-- write to `<outDir>/.tmp/`, then rename into place.  Handshake renamed LAST
-- so Cargo's `cargo:rerun-if-changed=` sees a consistent snapshot.
--
-- Week 2 emits only two of the five files. `<pkg>.leo4-schema` (full IDL),
-- `<pkg>.wit` (lowered), and `<pkg>.leo4-shim.so` (compiled shim) land later.

import Lean.Data.Json
import Leo4Plugin.Mangling

namespace Leo4Plugin.Emit

open Lean (Json)
open Leo4Plugin (IDLType Hash mangleType mangle normalizePackageSegment)

/-! ## Data shapes -/

/--
One parameter position in an `Instantiation`. Carries both the substituted
IDL encoding (what appears in `mangled`) and a list of generic indices the
parameter's *template* depended on. An empty `usesGenerics` means the
parameter is concrete in the function's signature (no generic substitution
happened for it); a non-empty list points back into the enclosing entry's
`generics` array.
-/
structure ParamInfo where
  encoded      : IDLType
  /-- Indices into the enclosing `ManglingEntry.generics`, ascending, deduplicated. -/
  usesGenerics : Array Nat
  deriving Inhabited

/-- Per-instantiation row inside `<pkg>.leo4-mangling.entries[].instantiations[]`. -/
structure Instantiation where
  /-- Type arguments substituted for the function's generics. Same length as
  the enclosing entry's `generics`. A `none` slot marks a *phantom* generic
  — one that has no observable effect on the function's ABI surface, so the
  plugin did not enumerate it (LEO4-DESIGN.md §5). Empty array for
  non-generic functions. -/
  genericArgs : Array (Option IDLType)
  /-- Parameter slots after generic substitution. The `.encoded` fields,
  joined by `_`, form the tokens between the function name and `__h<hash>`
  in `mangled`. -/
  paramTypes  : Array ParamInfo
  /-- The mangled linker symbol (SPEC/mangling.md §1). -/
  mangled     : String
  deriving Inhabited

/-- Per-function entry in `<pkg>.leo4-mangling`. -/
structure ManglingEntry where
  logicalName    : String                                 -- "<iface>::<fname>"
  generics       : Array String
  instantiations : Array Instantiation
  deriving Inhabited

/-- Per-interface summary in `<pkg>.leo4-handshake.interfaces[]`. -/
structure InterfaceSummary where
  name           : String
  function_count : Nat
  resource_count : Nat                                    -- always 0 for Week 2
  deriving Inhabited

/-- The full bundle to emit. The plugin builds one of these and hands it off
to `emit` below. -/
structure EmitBundle where
  package         : String
  /-- Lean module name the user `@[leo4_export]`s live in (e.g.
  `Sample`). leo4-native uses this at load time to know which
  `initialize_<Module>` symbol to dlsym before any shim entry point
  is called. (P5-a₂ wires this through to the Rust loader.) -/
  targetModule    : String
  /-- Linker-visible `initialize_*` symbol emitted by `lean -c` for
  the auto-generated wrapper file. Loading it (one call from
  leo4-native) transitively initialises `initialize_Init` and the
  user package's `initialize_<targetModule>`, so the loader doesn't
  need to know either of those individually. Computed by the plugin
  from the wrapper file's stem via `manglerLeanModuleName`. -/
  wrapperInitSymbol : String
  schemaHash      : Hash
  /-- `lean-toolchain` file contents, informational. -/
  leanToolchain   : String
  pluginVersion   : String                                -- e.g. "0.1.0"
  emittedAt       : String                                -- ISO 8601 UTC
  interfaces      : Array InterfaceSummary
  /-- Realised admit-set per constraint name.  For Week 2: only `scalar` and
  any Lean class we encountered in `[Cls T]` binders. -/
  constraintUniverse : Array (String × Array IDLType)
  entries         : Array ManglingEntry
  /-- Pretty (newline-decorated) IDL text for the on-disk `<pkg>.leo4-schema`
  file.  Its fully-collapsed form is the input to `schemaHash`. -/
  schemaText      : String
  deriving Inhabited

/-! ## JSON encoders (ordered keys) -/

private def idlJson (t : IDLType) : Json :=
  Json.str (mangleType t)

private def argsJson (xs : Array IDLType) : Json :=
  Json.arr (xs.map idlJson)

private def optIdlJson : Option IDLType → Json
  | some t => idlJson t
  | none   => Json.null

private def optArgsJson (xs : Array (Option IDLType)) : Json :=
  Json.arr (xs.map optIdlJson)

private def paramInfoJson (p : ParamInfo) : Json :=
  Json.mkObj [
    ("encoded",       idlJson p.encoded),
    ("uses_generics", Json.arr (p.usesGenerics.map fun n => Json.num (Int.ofNat n)))
  ]

private def instantiationJson (i : Instantiation) : Json :=
  Json.mkObj [
    ("generic_args", optArgsJson i.genericArgs),
    ("param_types",  Json.arr (i.paramTypes.map paramInfoJson)),
    ("mangled",      Json.str i.mangled)
  ]

private def manglingEntryJson (e : ManglingEntry) : Json :=
  Json.mkObj [
    ("logical_name",   Json.str e.logicalName),
    ("generics",       Json.arr (e.generics.map Json.str)),
    ("instantiations", Json.arr (e.instantiations.map instantiationJson))
  ]

private def manglingTableJson (b : EmitBundle) : Json :=
  Json.mkObj [
    ("version",     Json.num (1 : Int)),
    ("package",     Json.str b.package),
    ("schema_hash", Json.str b.schemaHash.toBase32lc),
    ("entries",     Json.arr (b.entries.map manglingEntryJson))
  ]

private def interfaceJson (i : InterfaceSummary) : Json :=
  Json.mkObj [
    ("name",           Json.str i.name),
    ("function_count", Json.num (Int.ofNat i.function_count)),
    ("resource_count", Json.num (Int.ofNat i.resource_count))
  ]

private def handshakeJson (b : EmitBundle) : Json :=
  let constraints : Json :=
    Json.mkObj (b.constraintUniverse.map (fun (k, vs) =>
      (k, Json.arr (vs.map idlJson))) |>.toList)
  Json.mkObj [
    ("version",             Json.num (1 : Int)),
    ("package",             Json.str b.package),
    ("target_module",       Json.str b.targetModule),
    ("wrapper_init_symbol", Json.str b.wrapperInitSymbol),
    ("schema_hash",         Json.str b.schemaHash.toBase32lc),
    ("schema_hash_bytes",   Json.str b.schemaHash.toHex),
    ("abi_version",         Json.num (1 : Int)),
    ("lean_toolchain",      Json.str b.leanToolchain),
    ("leo4_plugin_version", Json.str b.pluginVersion),
    ("emitted_at",          Json.str b.emittedAt),
    ("interfaces",          Json.arr (b.interfaces.map interfaceJson)),
    ("constraint_universe", constraints)
  ]

/-! ## Atomic file emission -/

private def writeAtomic (outDir : System.FilePath) (basename : String) (contents : String) : IO Unit := do
  let tmpDir := outDir / ".tmp"
  IO.FS.createDirAll tmpDir
  let tmpPath := tmpDir / basename
  IO.FS.writeFile tmpPath contents
  IO.FS.rename tmpPath (outDir / basename)

/--
Write the bundle to `<outDir>/<pkg>.leo4-schema`,
`<outDir>/<pkg>.leo4-mangling`, and `<outDir>/<pkg>.leo4-handshake`
atomically.  Order per SPEC/handshake.md §"Atomic Emission" — handshake
last, so Cargo (which watches handshake) only sees a fully consistent
snapshot.
-/
def emit (outDir : System.FilePath) (b : EmitBundle) : IO Unit := do
  IO.FS.createDirAll outDir
  let stem := normalizePackageSegment b.package
  let schemaText    := b.schemaText
  let manglingText  := (manglingTableJson b).pretty
  let handshakeText := (handshakeJson b).pretty
  -- schema first (largest text), mangling, handshake last.
  writeAtomic outDir s!"{stem}.leo4-schema"    schemaText
  writeAtomic outDir s!"{stem}.leo4-mangling"  manglingText
  writeAtomic outDir s!"{stem}.leo4-handshake" handshakeText

/-! ## Time helper -/

/-- Best-effort ISO 8601 UTC timestamp via `date -u +%FT%TZ`.
On `date` failure returns the string `"unknown"`. The field is documented as
informational in `SPEC/handshake.md`. -/
def isoNow : IO String := do
  try
    let out ← IO.Process.run { cmd := "date", args := #["-u", "+%FT%TZ"] }
    return out.trimAscii.copy
  catch _ => return "unknown"

end Leo4Plugin.Emit
