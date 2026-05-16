-- Top-level Lake workspace for leo4.
-- Individual targets live under lake/Leo4 (runtime library) and
-- lake/Leo4Plugin (Lake build plugin).
--
-- This file declares the workspace and re-exports the two sub-packages.

import Lake
open Lake DSL

package leo4

require Leo4 from "lake" / "Leo4"
require Leo4Plugin from "lake" / "Leo4Plugin"
