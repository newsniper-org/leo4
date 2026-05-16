-- Spike 0 workspace.
--
-- Three Lake targets:
--   • Leo4Export        — registers `@[leo4_export]` (toy stand-in for lake/Leo4/Export.lean)
--   • Sample            — toy "user package" tagged with `@[leo4_export]`
--   • leo4-spike-plugin — executable that re-loads `Sample` via Lean.importModules,
--                         walks the environment, prints exports, enumerates `ToString`
--                         instances, and reports timings.
--
-- This is the model the production plugin must replicate: re-import .olean,
-- query attributes and the instance extension, emit IDL + mangling table.

import Lake
open Lake DSL

package «leo4-spike-lake-hook» where
  -- nothing special

lean_lib Leo4Export where
  roots := #[`Leo4Export]

lean_lib Sample where
  roots := #[`Sample]

lean_exe «leo4-spike-plugin» where
  root := `SpikePlugin
  -- `supportInterpreter` is required because the exe calls Lean.importModules
  -- and Meta.* at runtime — i.e. it links the Lean elaborator.
  supportInterpreter := true
