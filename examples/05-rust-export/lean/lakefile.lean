import Lake
open Lake DSL

package leo4Example05 where
  -- The Lean wrapper module that `leo4-rust-emit --emit-lean`
  -- generates lives at `Leo4ExampleMiniSolverRust/Rust.lean`.
  -- Lake picks it up from `srcDir` via the `.submodules` glob below.
  srcDir := "."

require Leo4     from "../../../lake/Leo4"
require Leo4Rust from "../../../lake/Leo4Rust"
-- `require Leo4Rust` pulls in two extern_libs (Phase 9-6
-- follow-up 1/3 + 2/3) that Lake auto-links into the
-- executable: `libleo4_rust_bridge.a` (cargo-built) and
-- `libleo4_rust_bridge_lean.a` (leanc-compiled glue shim).
-- The manual `leanc -o` final link line is no longer needed.
--
-- Phase 10-D2 (2026-05-21): the Lake-side emit step is exposed as
-- `lake exe Leo4Rust/regenerate` (declared in Leo4Rust's lakefile).
-- Run it after `cargo build --release` to drop the generated
-- wrapper at `Leo4ExampleMiniSolverRust/Rust.lean`, then `lake
-- build`. `leo4 run` (Phase 10-D1) chains both for you.

-- Carries the auto-generated wrapper module produced by
-- `leo4-rust-emit --emit-lean` at
-- `Leo4ExampleMiniSolverRust/Rust.lean`. The `.submodules` glob
-- pulls every file under that directory in.
lean_lib Leo4ExampleMiniSolverRust where
  globs := #[.submodules `Leo4ExampleMiniSolverRust]

@[default_target]
lean_exe leo4Example05 where
  root := `Main
