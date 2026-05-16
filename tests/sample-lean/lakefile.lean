-- Smoke-test "user package" for the leo4 Lake plugin.
--
-- The plugin is invoked as
--   lake env lake exe leo4plugin Sample
-- against this package's compiled .olean files.  See README.md (TBD).

import Lake
open Lake DSL

package «leo4-sample» where

require Leo4 from ".." / ".." / "lake" / "Leo4"

@[default_target]
lean_lib Sample where
  roots := #[`Sample]
