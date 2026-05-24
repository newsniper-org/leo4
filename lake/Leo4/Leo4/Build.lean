-- Leo4.Build — user-facing build helpers for the `.leo4-shim.so` link
-- step.
--
-- Two surfaces:
--
--   M (default)  : `defaultLink cfg`  — one-shot wrapper→C→.o + shim→.o
--                  + link into `<outName>.so`.
--   L (extended) : the same `defaultLink` plus per-phase helpers
--                  (`compileWrapperToC`, `compileSourceToObj`,
--                  `linkShared`) and `checkExports` validation.
--
-- L is a superset of M: every helper M offers stays callable from L,
-- and L just adds knobs (`BuildCfg.extraCFlags`, `.extraLDFlags`,
-- `.extraObjects`) plus the symbol-table check.
--
-- The plugin uses these helpers itself when no user-supplied
-- `Build.lean` is present (the "default Build.lean"), so the surface
-- the plugin uses and the surface the user can override are the same
-- code path.

import Lean.Data.Json
import Lean.Data.Json.FromToJson
import Leo4.Platform

namespace Leo4.Build

open Leo4.Platform (dynlibExt dynlibPrefix defaultShimSuffix
                    isPlatformDynlib stemOfDynlib linkRpath?)

open Lean (Json ToJson FromJson fromJson? toJson)

/-- Configuration delivered from the plugin to the user's `Build.lean`
(or consumed by `Leo4.Build.defaultLink` when there is no
`Build.lean`).

Field roles:

- `pkg` / `iface` / `schemaHash` — identity-only, surfaced for log lines
  and conditional builds. Not consumed by the default helpers.
- `outDir` / `outName` — where artifacts live and what stem the link
  step uses for the produced `.so` (`<outName>.so`).
- `shimSrc` / `wrapperSrc` — the two emitted sources every shim build
  starts from.
- `manglingPath` / `mangledBodies` — input to `checkExports`. Plugin
  populates both; the file path is canonical, the array is a
  convenience copy.
- `leancPath` — resolved `leanc` binary (lets `Build.lean` reuse it).
- `extraCFlags` / `extraLDFlags` / `extraObjects` — L-tier knobs. M
  callers can leave these empty and the defaults still produce a
  working `.so`. -/
structure BuildCfg where
  pkg            : String
  iface          : String
  schemaHash     : String
  /-- User package root — the directory containing `lakefile.{lean,toml}`
  and (optionally) `Build.lean`. The plugin infers this from `outDir`
  by walking up until it finds the `.lake` directory and returning the
  parent. -/
  pkgRoot        : System.FilePath
  outDir         : System.FilePath
  outName        : String
  shimSrc        : System.FilePath
  wrapperSrc     : System.FilePath
  manglingPath   : System.FilePath
  mangledBodies  : Array String
  leancPath      : System.FilePath
  extraCFlags    : Array String := #[]
  extraLDFlags   : Array String := #[]
  extraObjects   : Array System.FilePath := #[]
deriving Inhabited

-- System.FilePath is a one-field structure around String; round-trip
-- through that field gives a stable JSON representation.
private def fpToJson (p : System.FilePath) : Json := Json.str p.toString
private def fpFromJson? (j : Json) : Except String System.FilePath :=
  j.getStr?.map System.FilePath.mk

instance : ToJson BuildCfg where
  toJson c := Json.mkObj
    [ ("pkg",           toJson c.pkg)
    , ("iface",         toJson c.iface)
    , ("schemaHash",    toJson c.schemaHash)
    , ("pkgRoot",       fpToJson c.pkgRoot)
    , ("outDir",        fpToJson c.outDir)
    , ("outName",       toJson c.outName)
    , ("shimSrc",       fpToJson c.shimSrc)
    , ("wrapperSrc",    fpToJson c.wrapperSrc)
    , ("manglingPath",  fpToJson c.manglingPath)
    , ("mangledBodies", toJson c.mangledBodies)
    , ("leancPath",     fpToJson c.leancPath)
    , ("extraCFlags",   toJson c.extraCFlags)
    , ("extraLDFlags",  toJson c.extraLDFlags)
    , ("extraObjects",  toJson (c.extraObjects.map (·.toString)))
    ]

instance : FromJson BuildCfg where
  fromJson? j := do
    let get s := j.getObjVal? s
    let pkg           ← (← get "pkg").getStr?
    let iface         ← (← get "iface").getStr?
    let schemaHash    ← (← get "schemaHash").getStr?
    let pkgRoot       ← fpFromJson? (← get "pkgRoot")
    let outDir        ← fpFromJson? (← get "outDir")
    let outName       ← (← get "outName").getStr?
    let shimSrc       ← fpFromJson? (← get "shimSrc")
    let wrapperSrc    ← fpFromJson? (← get "wrapperSrc")
    let manglingPath  ← fpFromJson? (← get "manglingPath")
    let mangledBodies ← fromJson? (← get "mangledBodies")
    let leancPath     ← fpFromJson? (← get "leancPath")
    let extraCFlags   ← fromJson? (← get "extraCFlags")
    let extraLDFlags  ← fromJson? (← get "extraLDFlags")
    let extraObjsArr  : Array String ← fromJson? (← get "extraObjects")
    return {
      pkg, iface, schemaHash, pkgRoot, outDir, outName,
      shimSrc, wrapperSrc, manglingPath, mangledBodies, leancPath,
      extraCFlags, extraLDFlags,
      extraObjects := extraObjsArr.map System.FilePath.mk
    }

namespace BuildCfg

/-- Canonical output path: `<outDir>/<outName>.so`.
The `.so` suffix is **intentionally constant on every
platform** — see `Leo4.Platform.defaultShimSuffix`. The
plugin writer + `leo4-build` + `leo4-mslean4` readers all
agree on this leo4-internal naming so the
handshake/mangling/shim file triple stays discoverable
through a single naming rule. On Windows the file content
is a PE DLL regardless; `libloading` / `LoadLibraryW`
don't inspect the extension. -/
def defaultSoPath (cfg : BuildCfg) : System.FilePath :=
  cfg.outDir / s!"{cfg.outName}{defaultShimSuffix}"

def load (path : System.FilePath) : IO BuildCfg := do
  let text ← IO.FS.readFile path
  match Json.parse text with
  | .error e => throw (IO.userError s!"BuildCfg.load: invalid JSON at {path}: {e}")
  | .ok j =>
    match fromJson? (α := BuildCfg) j with
    | .error e => throw (IO.userError s!"BuildCfg.load: schema mismatch at {path}: {e}")
    | .ok cfg => return cfg

def save (path : System.FilePath) (cfg : BuildCfg) : IO Unit :=
  IO.FS.writeFile path ((toJson cfg).pretty)

end BuildCfg

/-! ## Process helpers -/

private def runOrThrow (cmd : String) (args : Array String) : IO Unit := do
  let res ← IO.Process.output { cmd, args }
  if res.exitCode != 0 then
    throw <| IO.userError <|
      s!"leo4.build: `{cmd} {String.intercalate " " args.toList}` " ++
      s!"exited {res.exitCode}\nstdout:\n{res.stdout}\nstderr:\n{res.stderr}"

/-! ## M API -/

/-- Phase 1: compile the auto-emitted wrapper `.lean` to a `.c` file
via `lake env lean -c`. The output path is `wrapperSrc` with its
`.lean` extension swapped for `.c` (so a wrapper at
`<pkg>.leo4-exports.lean` produces `<pkg>.leo4-exports.c`).

P5-a₃-β (2026-05-20): we pass `--root=<wrapper-dir>` so that the
Lean compiler derives the wrapper's *module name* from
`<wrapper-basename>` (e.g. `leo4_sample.leo4-exports`) rather than
from the absolute path to the file. The plugin then computes the
emitted `initialize_*` symbol from that deterministic module name
and stores it on the handshake JSON so the loader can dlsym it
without knowing where on disk the build happened. -/
def compileWrapperToC (cfg : BuildCfg) : IO System.FilePath := do
  let outC := cfg.wrapperSrc.withExtension "c"
  let rootDir : String :=
    match cfg.wrapperSrc.parent with
    | some p => p.toString
    | none   => "."
  runOrThrow "lake" #[
    "env", "lean",
    "-c", outC.toString,
    "--root=" ++ rootDir,
    cfg.wrapperSrc.toString
  ]
  return outC

/-- Phase 2 (generic): compile any `.c` source to a `.o` next to it,
using `leancPath`, `BuildCfg.extraCFlags`, and any caller-supplied
`extra` flags. Returns the produced `.o` path. -/
def compileSourceToObj (cfg : BuildCfg) (src : System.FilePath)
    (extra : Array String := #[]) : IO System.FilePath := do
  let obj := src.withExtension "o"
  let args := #["-c", src.toString, "-o", obj.toString] ++ cfg.extraCFlags ++ extra
  runOrThrow cfg.leancPath.toString args
  return obj

/-- Phase 2 (wrapper-flavoured): like `compileSourceToObj`, but adds
`-DLEAN_EXPORTING` so the `LEAN_EXPORT` macro in `lean.h` expands to
default-visibility attributes. Without this, the wrapper's `@[export]`
symbols get the hidden-by-default visibility that `leanc --print-cflags`
enforces, and the resulting `.so` ships no `leo4_lean__*` entries. -/
def compileWrapperObj (cfg : BuildCfg) (src : System.FilePath) : IO System.FilePath :=
  compileSourceToObj cfg src (extra := #["-DLEAN_EXPORTING"])

/-- Phase 3: link the given objects into a shared library at `out`,
honouring `BuildCfg.extraLDFlags`.

The link line explicitly requests `libleanshared.so`. Without it,
`leanc -shared` (the bundled clang) produces a `.so` that has no
DT_NEEDED entry for the Lean runtime, and the loader fails at
`dlsym lean_initialize_runtime_module` (the runtime's symbols are
*static* inside `leanc`-built executables but not exported across a
shared-library boundary). Linking the shared runtime gives the
loader a stable surface to `dlsym` against and makes
`Lean::open` work from any Rust process. -/
def linkShared (cfg : BuildCfg) (objs : Array System.FilePath) (out : System.FilePath) : IO Unit := do
  -- `cfg.leancPath` is `<sysroot>/bin/leanc`; walk up two to find
  -- `<sysroot>/lib/lean` so we can `-L` it.
  let sysroot : Option System.FilePath := do
    let bin ← cfg.leancPath.parent
    bin.parent
  let leanLibDir : String := match sysroot with
    | some s => (s / "lib" / "lean").toString
    | none   => "."
  -- Auto-detect the user package's per-module shared libraries
  -- (Lake's `lake build <Module>:shared` output) and link against
  -- them. The shim's wrapper init transitively calls
  -- `initialize_<Module>`, which only exists in these libs — without
  -- them the shim loads but `dlsym` on the wrapper init's first
  -- transitive callee fails. We pick up every `lib<…>.so` we find
  -- both in the user package and in each direct dependency listed
  -- in `lake-manifest.json` (transitive Lean modules like Leo4 own
  -- their own `initialize_*` symbols too); RPATH covers the
  -- load-time search path.
  -- Per Leo4.Platform: detect `lib*.so` (Linux),
  -- `lib*.dylib` (macOS), `*.dll` (Windows gnullvm). The
  -- platform layer encapsulates both the file-naming
  -- detection (`isPlatformDynlib` / `stemOfDynlib`) and
  -- the rpath flag emission (`linkRpath?` is `none` on
  -- Windows since PE binaries don't carry rpath).
  let collectLibDir (dir : System.FilePath) : IO (Array String) := do
    if !(← dir.pathExists) then return #[]
    let entries ← dir.readDir
    let mut libs : Array String := #[]
    for entry in entries do
      let name := entry.fileName
      if isPlatformDynlib name then
        libs := libs.push s!"-l{stemOfDynlib name}"
    if libs.isEmpty then return #[]
    let rpathFlag : Array String := match linkRpath? dir.toString with
      | some flag => #[flag]
      | none      => #[]  -- Windows: rely on PATH / .exe-adjacent search
    return #["-L", dir.toString] ++ libs ++ rpathFlag
  let userLibDir : System.FilePath := cfg.pkgRoot / ".lake" / "build" / "lib"
  let mut userLibArgs : Array String ← collectLibDir userLibDir
  -- Walk lake-manifest.json for direct dep packages and pick up
  -- their `.lake/build/lib/` too.
  let manifest := cfg.pkgRoot / "lake-manifest.json"
  if (← manifest.pathExists) then
    let text ← IO.FS.readFile manifest
    match Lean.Json.parse text with
    | .error _ => pure ()
    | .ok j =>
      let pkgs : Array Lean.Json :=
        (j.getObjVal? "packages").toOption.bind (·.getArr?.toOption) |>.getD #[]
      for pkg in pkgs do
        let dir? : Option String :=
          (pkg.getObjVal? "dir").toOption.bind (·.getStr?.toOption)
        match dir? with
        | none     => pure ()
        | some dir =>
          let depRoot : System.FilePath := cfg.pkgRoot / dir
          let depLibDir := depRoot / ".lake" / "build" / "lib"
          userLibArgs := userLibArgs ++ (← collectLibDir depLibDir)
  -- RPATH so the dynamic linker resolves the Lean
  -- runtime DLL at load time without the consumer
  -- setting `LD_LIBRARY_PATH`. POSIX (Linux / macOS):
  -- emit `-Wl,-rpath,<dir>`. Windows (gnullvm): rpath
  -- doesn't apply to PE; the dependent DLL
  -- (`leanshared.dll`) must be reachable via PATH /
  -- adjacent dir / `AddDllDirectory` — the loader
  -- (`leo4-mslean4`) is responsible for arranging that
  -- on Windows.
  let leanRpath : Array String := match linkRpath? leanLibDir with
    | some flag => #[flag]
    | none      => #[]
  let args := #["-shared", "-o", out.toString]
            ++ objs.map (·.toString)
            ++ #["-L", leanLibDir, "-lleanshared"]
            ++ leanRpath
            ++ userLibArgs
            ++ cfg.extraLDFlags
  runOrThrow cfg.leancPath.toString args

/-- Default end-to-end link: wrapper → `.c`, wrapper → `.o` (with
`-DLEAN_EXPORTING`), shim → `.o`, plus any `cfg.extraObjects`, link
into `cfg.defaultSoPath`. -/
def defaultLink (cfg : BuildCfg) : IO Unit := do
  let wrapC ← compileWrapperToC cfg
  let wrapO ← compileWrapperObj cfg wrapC
  let shimO ← compileSourceToObj cfg cfg.shimSrc
  let objs := #[wrapO, shimO] ++ cfg.extraObjects
  linkShared cfg objs cfg.defaultSoPath

/-! ## L extension: post-build symbol check -/

/-- L-tier validation: for every mangled body in `cfg.mangledBodies`,
assert that the produced shared object exports both
`leo4_call_<body>` and `leo4_lean__<body>`. Uses `nm -D --defined-only`
on a Linux/glibc system; falls back to `nm -gU` semantics on macOS.

Returns an error listing missing pairs if any. Pure read-only; safe
to invoke from any `Build.lean` after a link step. -/
def checkExports (cfg : BuildCfg) (sofile : System.FilePath) : IO Unit := do
  let res ← IO.Process.output { cmd := "nm", args := #["-D", "--defined-only", sofile.toString] }
  if res.exitCode != 0 then
    -- macOS fallback: BSD nm doesn't accept --defined-only / -D.
    let res2 ← IO.Process.output { cmd := "nm", args := #["-gU", sofile.toString] }
    if res2.exitCode != 0 then
      throw <| IO.userError s!"leo4.build: nm failed on {sofile} (stderr:\n{res.stderr})"
    return ← checkFromNm cfg res2.stdout sofile
  return ← checkFromNm cfg res.stdout sofile
where
  checkFromNm (cfg : BuildCfg) (out : String) (sofile : System.FilePath) : IO Unit := do
    let syms : Array String := (out.splitOn "\n").toArray.filterMap fun line =>
      -- `String.trimAscii` returns a `String.Slice`; project back to
      -- `String` before reaching for `splitOn`.
      match (line.trimAscii.toString.splitOn " ").reverse with
      | name :: _ => some name
      | _         => none
    let present : Std.HashSet String := Std.HashSet.ofArray syms
    let mut missing : Array String := #[]
    for body in cfg.mangledBodies do
      let call  := s!"leo4_call_{body}"
      let lean  := s!"leo4_lean__{body}"
      unless present.contains call  do missing := missing.push call
      unless present.contains lean  do missing := missing.push lean
    if !missing.isEmpty then
      throw <| IO.userError <|
        s!"leo4.build.checkExports: {missing.size} symbol(s) missing from {sofile}:\n  " ++
        String.intercalate "\n  " missing.toList
    IO.println s!"ok  {sofile} carries {cfg.mangledBodies.size * 2} expected symbols"

end Leo4.Build

/-! ## Reverse-direction (Phase 9-6 follow-up) — Rust bridge helpers

Helpers a user lakefile may call to compile the Lean-side glue
shim (`shim/leo4_rust_bridge_lean.c`) and discover
`libleo4_rust_bridge.a` for the leanc link step. Real Lake DSL
integration (so `lean_exe`'s `weakLinkArgs` can pull them in
declaratively) lands when Lake exposes a stable hook; until
then a user can invoke these from a `script` block or a
`def main`-style helper.
-/

namespace Leo4.Build.RustBridge

/-- Compile `<leo4Root>/shim/leo4_rust_bridge_lean.c` into the
    object at `outObj`. Uses `leanc -c -std=c2x` matching the
    Phase 9-6 build line.

    Returns `outObj` on success; throws an `IO.userError` if
    leanc exits non-zero (stderr forwarded). -/
def compileGlueShim (leo4Root : System.FilePath) (outObj : System.FilePath) :
    IO System.FilePath := do
  let src := leo4Root / "shim" / "leo4_rust_bridge_lean.c"
  if !(← src.pathExists) then
    throw <| IO.userError s!"Leo4.Build.RustBridge.compileGlueShim: missing source {src}"
  let res ← IO.Process.output {
    cmd  := "leanc",
    args := #["-c", "-std=c2x", src.toString, "-o", outObj.toString],
  }
  if res.exitCode != 0 then
    throw <| IO.userError <|
      s!"leanc failed compiling {src} (exit {res.exitCode}):\n{res.stderr}"
  return outObj

/-- Resolve the `libleo4_rust_bridge.a` static archive.

    Search order (mirrors `LEO4_SHIM_SO`'s chain in
    `leo4-build::wire`):
      1. env `LEO4_RUST_BRIDGE_AR`              — explicit override
      2. `<leo4Root>/target/release/libleo4_rust_bridge.a`
      3. `<leo4Root>/target/debug/libleo4_rust_bridge.a`

    Caller is responsible for ensuring the archive has been
    built (`cargo build -p leo4-rust-bridge`); this function
    only locates it, it does NOT invoke cargo. -/
def discoverBridgeArchive (leo4Root : System.FilePath) : IO System.FilePath := do
  match ← IO.getEnv "LEO4_RUST_BRIDGE_AR" with
  | some p =>
    let fp : System.FilePath := p
    if ← fp.pathExists then return fp
    throw <| IO.userError s!"LEO4_RUST_BRIDGE_AR points at non-existent path: {p}"
  | none =>
    let release := leo4Root / "target" / "release" / "libleo4_rust_bridge.a"
    if ← release.pathExists then return release
    let debug := leo4Root / "target" / "debug" / "libleo4_rust_bridge.a"
    if ← debug.pathExists then return debug
    throw <| IO.userError <|
      s!"libleo4_rust_bridge.a not found. Searched:\n" ++
      s!"  $LEO4_RUST_BRIDGE_AR (unset)\n" ++
      s!"  {release}\n" ++
      s!"  {debug}\n" ++
      "Run `cargo build --release -p leo4-rust-bridge` first."

/-- Linker args a `lean_exe` needs to consume reverse-direction
    Lean wrapper modules (one entry per `#[leo4::export]` group).

    Returns `#[<glueShimObj>, <bridgeArchive>]` so the caller can
    splice them into `weakLinkArgs` or pass them to a manual
    `leanc` invocation. -/
def linkArgs (leo4Root : System.FilePath) (glueObj : System.FilePath) :
    IO (Array String) := do
  let ar ← discoverBridgeArchive leo4Root
  return #[glueObj.toString, ar.toString]

end Leo4.Build.RustBridge
