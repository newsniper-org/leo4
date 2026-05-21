-- MathlibBridgeTest — smoke test for `Leo4.MathlibBridge.*` modules.
--
-- Each `Leo4.MathlibBridge.<Sub>` module ships per-type 1-to-1
-- conversion functions between leo4's carrier structures (`LeanU128`,
-- `LeanComplexF64x2`, `LeanRat`, …) and the matching Mathlib types
-- (`BitVec 128`, `Complex ℝ`, `ℚ`, …). This file imports them all so
-- a `lake build` here proves every bridge module elaborates against
-- the current Mathlib release.
--
-- Today this file is a placeholder — the actual bridge modules land
-- in follow-up commits. The infrastructure is in place so the first
-- bridge commit just adds an `import Leo4.MathlibBridge.<Sub>` line
-- and a couple of `example` checks.

import Leo4
-- Mathlib is pulled by the lakefile; once individual bridges land we
-- import `Leo4.MathlibBridge.<Sub>` here. For now we just confirm
-- the build environment wires together.
import Mathlib.Init

namespace MathlibBridgeTest

/-- Smoke check: the leo4 surface is in scope and Mathlib resolves. -/
example : Leo4.LeanU128 := ⟨0, 0⟩

end MathlibBridgeTest
