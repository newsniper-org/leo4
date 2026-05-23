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
