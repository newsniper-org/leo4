-- Leo4.MathlibBridge.Rat — opt-in conversions between Lean-core
-- `Rat` (= Mathlib `ℚ`) and broader Mathlib types Mathlib reasons
-- about (`ℝ`, `ℂ`, `ZMod n`).
--
-- `Leo4.LeanMarshal Rat` lives in `Leo4.MathlibSubset` (Phase 8
-- step 1); on the Lean side we use Lean core's `Rat` directly,
-- which Mathlib treats as `ℚ`. So the "leo4 ↔ Mathlib" bridge
-- here is really `Rat ↔ broader-Mathlib-types`.
--
-- All forward conversions are total: `Rat → ℝ` and `Rat → ℂ`
-- are exact embeddings (Rat values are exactly-representable
-- reals / Gaussian-rationals-with-zero-imaginary). Reverse
-- directions (`ℝ → Rat`, `ℂ → Rat`) are partial (only
-- representable values round-trip); we don't ship those today.
--
-- User packages opt in via `import Leo4.MathlibBridge.Rat`.

import Leo4.MathlibSubset
import Mathlib.Data.Real.Basic
import Mathlib.Data.Complex.Basic

namespace Leo4

/-- Embed `Rat` as a `ℝ` via Mathlib's `Rat.cast`. Total, lossless. -/
def Rat.toReal (q : Rat) : ℝ := (q : ℝ)

/-- Embed `Rat` as a `ℂ` (real part = q, imaginary part = 0). -/
def Rat.toComplex (q : Rat) : ℂ := ⟨(q : ℝ), 0⟩

end Leo4
