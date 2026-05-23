import Lake
open Lake DSL

package leo4Example05 where
  -- The Lean wrapper module that `leo4-rust-emit --emit-lean`
  -- generates lives at `Generated/Leo4ExampleMiniSolverRust.lean`.
  -- Lake picks it up from `srcDir`.
  srcDir := "."

require Leo4 from "../../../lake/Leo4"

@[default_target]
lean_lib Leo4Example05 where
  globs := #[`Leo4Example05]

@[default_target]
lean_exe leo4Example05 where
  root := `Main
