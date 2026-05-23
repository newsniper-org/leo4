-- Leo4Rust — Lake package wiring the reverse-direction (Phase 9)
-- bridge into a leanc link. Forward-direction users continue to
-- `require Leo4` only; reverse-direction users add
-- `require Leo4Rust from "/abs/path/to/leo4/lake/Leo4Rust"` and the
-- two extern libs below are automatically picked up by Lake's link
-- step for any `lean_exe` in the consuming package.
--
-- The two extern libs:
--   * leo4RustBridgeLean : compiles `shim/leo4_rust_bridge_lean.c`
--       via leanc -c -std=c2x and wraps the .o in a static archive
--       (`ar rcs`) so Lake can hand a .a to leanc on the link line.
--   * leo4RustBridge     : discovers `libleo4_rust_bridge.a` from a
--       prior `cargo build -p leo4-rust-bridge` run. Search chain
--       mirrors LEO4_SHIM_SO: env LEO4_RUST_BRIDGE_AR, then
--       target/release/, then target/debug/ at the repo root.
--
-- Both target's output paths land in `Leo4Rust/.lake/build/lib/`
-- so Lake's standard link-line discipline finds them.

import Lake
open Lake DSL System

package Leo4Rust where
  -- nothing distinctive; we only contribute the two extern libs below.

require Leo4 from "../Leo4"

-- The repo root, three `..` up from this lakefile.
def leo4Root : System.FilePath :=
  System.mkFilePath ["..", "..", ".."]

target leo4RustBridgeLeanObj pkg : System.FilePath := Job.async do
  let workDir := pkg.buildDir / "leo4rust"
  IO.FS.createDirAll workDir
  let src    := leo4Root / "shim" / "leo4_rust_bridge_lean.c"
  let outObj := workDir / "leo4_rust_bridge_lean.o"
  let res ← IO.Process.output {
    cmd  := "leanc",
    args := #["-c", "-std=c2x", src.toString, "-o", outObj.toString],
  }
  if res.exitCode != 0 then
    throw <| IO.userError <|
      s!"leanc -c {src} failed (exit {res.exitCode}):\n{res.stderr}"
  return outObj

extern_lib leo4RustBridgeLean pkg := do
  let objJob ← leo4RustBridgeLeanObj.fetch
  objJob.bindSync fun obj _ => do
    let workDir := pkg.buildDir / "leo4rust"
    let archive := workDir / "libleo4_rust_bridge_lean.a"
    let res ← IO.Process.output {
      cmd := "ar",
      args := #["rcs", archive.toString, obj.toString],
    }
    if res.exitCode != 0 then
      throw <| IO.userError <|
        s!"ar rcs {archive} failed (exit {res.exitCode}):\n{res.stderr}"
    return (archive, .nilTrace)

extern_lib leo4RustBridge pkg := do
  Job.async do
    -- Discovery chain: env LEO4_RUST_BRIDGE_AR -> release -> debug.
    let candidates : List System.FilePath := Id.run do
      let mut xs := []
      xs := xs ++ [leo4Root / "target" / "debug"   / "libleo4_rust_bridge.a"]
      xs := xs ++ [leo4Root / "target" / "release" / "libleo4_rust_bridge.a"]
      return xs
    let envHit ← IO.getEnv "LEO4_RUST_BRIDGE_AR"
    let envCandidates : List System.FilePath :=
      match envHit with
      | some p => [System.FilePath.mk p]
      | none   => []
    let search := envCandidates ++ candidates
    let mut found : Option System.FilePath := none
    for c in search do
      if (← c.pathExists) then
        found := some c
        break
    match found with
    | some p => return p
    | none =>
      throw <| IO.userError <|
        s!"libleo4_rust_bridge.a not found; run `cargo build --release -p leo4-rust-bridge` first " ++
        s!"(searched: $LEO4_RUST_BRIDGE_AR + {search})"
