/-
  Leo4Rust — Lake package wiring the reverse-direction (Phase 9)
  bridge into the `leanc` link line declaratively.

  Forward-direction users continue to `require Leo4` only.
  Reverse-direction users add:

      require Leo4Rust from "/abs/path/to/leo4/lake/Leo4Rust"

  and the two `extern_lib`s below get picked up automatically by
  Lake's `lean_exe` link step (Lake/Build/Executable.lean's
  transitive externLibs collection).

  Commit 1 (4a): leo4RustBridge — path resolution only. Caller is
  responsible for `cargo build --release -p leo4-rust-bridge`
  before `lake build`; the body locates the produced `.a`.

  Commit 2 (4b): leo4RustBridgeLean — leanc + ar wrap of
  shim/leo4_rust_bridge_lean.c, with logicutils freshcheck as an
  optional cache layer (falls back to unconditional recompile
  when freshcheck is absent). See SPIKE-1.
-/

import Lake
open Lake DSL System

package Leo4Rust

require Leo4 from ".." / "Leo4"

@[default_target]
lean_lib Leo4Rust where
  -- Empty marker lib so Lake has a default target. The real
  -- artefacts are the two extern_libs below.

/-! ## Phase 10-D2 emit script (2026-05-21)

    `lake exe Leo4Rust/regenerate` collapses the reverse-direction
    pipeline's emit step into one Lake invocation. Run after
    `cargo build --release -p <crate>`; reads:

    - `LEO4_RUST_EMIT_BIN`  — abs path to `leo4-rust-emit` (required;
                              `leo4 run` sets it).
    - `LEO4_RUST_IFACE`     — Lean module prefix; defaults to
                              CamelCase(crate-name).

    Invoked from CWD = the user's `lean/` directory. Looks one
    level up for `Cargo.toml`, derives crate name, finds the
    cdylib under `../target/release/lib<crate>.{so,dylib,dll}`,
    invokes leo4-rust-emit, and moves the generated wrapper to
    `<iface>/Rust.lean`. Exit 0 on success / no-op (no Cargo.toml
    or no env), 1 on configuration error.
-/

private partial def cdylibIn (dir : System.FilePath) (stem : String)
    : IO (Option System.FilePath) := do
  for ext in ["so", "dylib", "dll"] do
    let p := dir / s!"lib{stem}.{ext}"
    if (← p.pathExists) then return some p
  return none

/-- Walk upward from `start` looking for `target/release/lib<stem>.{so,dylib,dll}`.
Stops at the filesystem root. Cargo workspace projects share a single
`target/` at the workspace root, so an example inside a workspace finds
its cdylib several levels up rather than next to its own `Cargo.toml`. -/
private partial def findCdylibUp (start : System.FilePath) (stem : String)
    : IO (Option System.FilePath) := do
  let mut cur := start
  let mut steps : Nat := 0
  while steps < 16 do
    let release := cur / "target" / "release"
    if (← release.pathExists) then
      if let some p ← cdylibIn release stem then return some p
    match cur.parent with
    | some parent => if parent = cur then break else cur := parent
    | none        => break
    steps := steps + 1
  return none

private def extractCargoName (s : String) : Option String := Id.run do
  for line in s.splitOn "\n" do
    let t := (line.trimAscii).toString
    if t.startsWith "name" then
      let after := ((t.drop 4).trimAscii).toString
      if after.startsWith "=" then
        let rest := ((after.drop 1).trimAscii).toString
        if rest.startsWith "\"" then
          let inner := rest.drop 1
          let name : String := (inner.takeWhile (· ≠ '"')).toString
          if !name.isEmpty then return some name
  return none

private def toCamelCase (s : String) : String := Id.run do
  let mut out : String := ""
  let mut capNext : Bool := true
  for ch in s.toList do
    if ch = '_' || ch = '-' || ch = ' ' then
      capNext := true
    else if capNext then
      out := out.push ch.toUpper
      capNext := false
    else
      out := out.push ch
  if out.isEmpty then "App" else out

script regenerate (_args : List String) := do
  let cwd ← IO.currentDir
  let projectRoot := cwd / ".."
  let cargoToml := projectRoot / "Cargo.toml"
  if !(← cargoToml.pathExists) then
    IO.eprintln s!"Leo4Rust/regenerate: no Cargo.toml at {cargoToml} — nothing to do (forward direction?)"
    return 0
  let emitBin? ← IO.getEnv "LEO4_RUST_EMIT_BIN"
  let emitBinStr ← match emitBin? with
    | some s => pure s
    | none   => do
      IO.eprintln "Leo4Rust/regenerate: LEO4_RUST_EMIT_BIN unset.\n  Run via `leo4 run` or `LEO4_RUST_EMIT_BIN=/abs/path/leo4-rust-emit lake exe Leo4Rust/regenerate`."
      return 1
  let emitBin : System.FilePath := emitBinStr
  if !(← emitBin.pathExists) then
    IO.eprintln s!"Leo4Rust/regenerate: LEO4_RUST_EMIT_BIN points to {emitBin}, but that file is absent."
    return 1
  let cargoContent ← IO.FS.readFile cargoToml
  let crateName ← match extractCargoName cargoContent with
    | some n => pure (n.replace "-" "_")
    | none   => do
      IO.eprintln s!"Leo4Rust/regenerate: could not extract `name = \"…\"` from {cargoToml}"
      return 1
  let ifaceEnv ← IO.getEnv "LEO4_RUST_IFACE"
  let iface := ifaceEnv.getD (toCamelCase crateName)
  -- Cdylib search order:
  --   1. $LEO4_RUST_CDYLIB explicit override.
  --   2. <project>/target/release/lib<crate>.{so,dylib,dll}.
  --   3. Walk upward looking for target/release/lib<crate>.* —
  --      handles cargo workspace projects whose target/ lives at
  --      the workspace root, not next to the member crate.
  let cdylibEnv ← IO.getEnv "LEO4_RUST_CDYLIB"
  let cdylib ← match cdylibEnv with
    | some p =>
      let fp : System.FilePath := p
      if !(← fp.pathExists) then
        IO.eprintln s!"Leo4Rust/regenerate: LEO4_RUST_CDYLIB={fp} but file is absent."
        return 1
      pure fp
    | none =>
      let projectReleaseDir := projectRoot / "target" / "release"
      match (← cdylibIn projectReleaseDir crateName) with
      | some p => pure p
      | none   =>
        match (← findCdylibUp projectRoot crateName) with
        | some p => pure p
        | none   => do
          IO.eprintln s!"Leo4Rust/regenerate: cdylib not found for crate `{crateName}`.\n  Searched {projectReleaseDir} and target/release/ walking up from {projectRoot}.\n  Run `cargo build --release -p <crate>` first, or set LEO4_RUST_CDYLIB."
          return 1
  let outDir := cwd / ".leo4-emit"
  IO.FS.createDirAll outDir
  let ifaceDir := cwd / iface
  IO.FS.createDirAll ifaceDir
  let leanModule := s!"{iface}.Rust"
  IO.eprintln s!"Leo4Rust/regenerate: {emitBin} → {ifaceDir}/Rust.lean"
  let r ← IO.Process.output {
    cmd := emitBin.toString,
    args := #[
      "--cdylib", cdylib.toString,
      "--out-dir", outDir.toString,
      "--emit-lean",
      "--lean-module", leanModule,
    ]
  }
  if r.exitCode != 0 then
    IO.eprintln s!"Leo4Rust/regenerate: leo4-rust-emit failed (exit {r.exitCode}):\n{r.stderr}"
    return 1
  let emitted := outDir / s!"{crateName}.leo4-rust-imports.lean"
  let dest := ifaceDir / "Rust.lean"
  if !(← emitted.pathExists) then
    IO.eprintln s!"Leo4Rust/regenerate: emit ran but {emitted} not produced."
    return 1
  if (← dest.pathExists) then
    IO.FS.removeFile dest
  IO.FS.rename emitted dest
  IO.eprintln s!"Leo4Rust/regenerate: ✓ {dest}"
  return 0

/-! ## 4a: leo4RustBridge

    Discovers `libleo4_rust_bridge.a` produced by
    `cargo build -p leo4-rust-bridge`. Search order matches
    SPEC/reverse-direction.md §9 (mirrors LEO4_SHIM_SO):

      1. env LEO4_RUST_BRIDGE_AR    (explicit override)
      2. <leo4_repo>/target/release/libleo4_rust_bridge.a
      3. <leo4_repo>/target/debug/libleo4_rust_bridge.a

    `<leo4_repo>` is resolved relative to this package's
    directory: `lake/Leo4Rust/../..` -> the leo4 repo root.

    Body type required by Lake's `extern_lib` DSL:
        NPackage pkgName -> FetchM (Job FilePath)
-/
extern_lib leo4RustBridge pkg := do
  let leo4Repo : System.FilePath := pkg.dir / ".." / ".."
  let envHit ← IO.getEnv "LEO4_RUST_BRIDGE_AR"
  let candidates : List System.FilePath :=
    match envHit with
    | some p => [System.FilePath.mk p]
    | none   => [
        leo4Repo / "target" / "release" / "libleo4_rust_bridge.a",
        leo4Repo / "target" / "debug"   / "libleo4_rust_bridge.a",
      ]
  let mut found : Option System.FilePath := none
  for c in candidates do
    if (← c.pathExists) then
      found := some c
      break
  match found with
  | some p =>
    return (Pure.pure p)
  | none =>
    let listed := String.intercalate "\n  " (candidates.map (·.toString))
    error <|
      s!"Leo4Rust: libleo4_rust_bridge.a not found.\nSearched:\n  " ++
      listed ++
      "\nRun `cargo build --release -p leo4-rust-bridge` " ++
      "in the leo4 repo first, or set LEO4_RUST_BRIDGE_AR to an absolute path."

/-! ## 4b: leo4RustBridgeLean — leanc-compiled glue shim wrapped in `.a`

    `shim/leo4_rust_bridge_lean.c` is the one place leo4 includes
    `<lean/lean.h>` (SPEC/reverse-direction.md §3, Phase 9-6).
    Lake links it as a static archive, which means the `.o` from
    `leanc -c` must be wrapped via `ar rcs` first.

    Logicutils integration (option R2, optional with fallback):
    when `freshcheck` is available on `$PATH` the body uses it as
    a content-hash gate around the leanc / ar invocation; absent,
    it falls through to unconditional rebuild (4b base). The
    freshcheck path uses BLAKE3 by default (per logicutils'
    `--method=hash`) — robust against `git checkout` timestamp
    churn that a Lake-native cache would mis-handle.

    See SPIKE-1 §4b + the logicutils discussion. README install
    note: `pacman -S logicutils` on Arch / Manjaro;
    `cargo install --git https://github.com/newsniper-org/logicutils logicutils`
    elsewhere; or omit and accept the per-build leanc/ar
    invocation.
-/
extern_lib leo4RustBridgeLean pkg := do
  let workDir : System.FilePath := pkg.buildDir / "leo4rust"
  IO.FS.createDirAll workDir
  let leo4Repo : System.FilePath := pkg.dir / ".." / ".."
  let src    := leo4Repo / "shim" / "leo4_rust_bridge_lean.c"
  let outObj := workDir / "leo4_rust_bridge_lean.o"
  let outAr  := workDir / "libleo4_rust_bridge_lean.a"
  let store  := workDir / ".lu-store"

  if !(← src.pathExists) then
    error s!"Leo4Rust: missing source {src}"

  -- Probe `freshcheck` availability. Logicutils' CLI protocol
  -- exposes `--protocol-version` on every binary; we use it as
  -- a cheap "is the tool installed?" probe.
  let freshcheckAvailable : Bool ←
    try
      let _ ← IO.Process.output {
        cmd := "freshcheck", args := #["--protocol-version"]
      }
      pure true
    catch _ =>
      pure false

  -- Decide rebuild.
  let mustRebuild ← if freshcheckAvailable && (← outAr.pathExists) then do
    let r ← IO.Process.output {
      cmd  := "freshcheck",
      args := #["--method=hash", "--store", store.toString,
                outAr.toString, src.toString],
    }
    pure (r.exitCode != 0)
  else
    pure true

  if mustRebuild then
    -- leanc -c -std=c2x src -o outObj
    let r1 ← IO.Process.output {
      cmd  := "leanc",
      args := #["-c", "-std=c2x", src.toString, "-o", outObj.toString],
    }
    if r1.exitCode != 0 then
      error s!"Leo4Rust: leanc failed compiling {src} (exit {r1.exitCode}):\n{r1.stderr}"

    -- ar rcs outAr outObj  (replace any existing archive contents)
    -- Remove the archive first so `ar rcs` doesn't accumulate
    -- objects across rebuilds.
    if (← outAr.pathExists) then
      IO.FS.removeFile outAr
    let r2 ← IO.Process.output {
      cmd  := "ar",
      args := #["rcs", outAr.toString, outObj.toString],
    }
    if r2.exitCode != 0 then
      error s!"Leo4Rust: ar failed wrapping {outObj} -> {outAr} (exit {r2.exitCode}):\n{r2.stderr}"

    -- Stamp BOTH the source and the rebuilt archive — freshcheck
    -- compares the stored hashes to the current ones, so it needs
    -- a recorded baseline for every dep + target. Tolerate stamp
    -- failure (cache bookkeeping; the rebuild itself was the
    -- load-bearing step).
    if freshcheckAvailable then
      let _ ← try
        IO.Process.output {
          cmd  := "stamp",
          args := #["record", "--store", store.toString,
                    outAr.toString, src.toString],
        }
      catch _ => pure { exitCode := 0, stdout := "", stderr := "" }
      pure ()

  return (Pure.pure outAr)
