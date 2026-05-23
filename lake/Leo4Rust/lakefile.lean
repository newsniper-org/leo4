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
