-- Leo4.MathlibBridge.Complex — opt-in conversion between leo4's
-- machine-complex carriers (`LeanComplexF32x2`, `LeanComplexF64x2`)
-- and Mathlib's abstract `Complex ℝ`.
--
-- Forward direction (`toComplex`) is total — machine Float values
-- are exact dyadic rationals, and the embedding `Float → ℝ` is
-- well-defined (Mathlib's `Float.toReal`). Reverse direction is
-- rounding-lossy: `Complex ℝ` has infinite precision and most
-- values don't round-trip back through `Float`. The reverse
-- (`ofComplex?` / `ofComplexRound`) is left to a future commit
-- with a concrete rounding-mode decision; today the module
-- provides only the forward maps.
--
-- User packages opt in via `import Leo4.MathlibBridge.Complex`.
-- The top-level `Leo4` import does NOT pull this — leo4 core
-- stays Mathlib-independent per ROADMAP §8.

import Leo4.Wide
import Mathlib.Data.Complex.Basic
import Mathlib.Data.Float

namespace Leo4

/-- View a `LeanComplexF64x2` as a Mathlib `ℂ`. Total: each `Float`
field maps to its exact dyadic-rational `ℝ` value. -/
def LeanComplexF64x2.toComplex (v : LeanComplexF64x2) : ℂ :=
  ⟨v.re.toReal, v.im.toReal⟩

/-- `LeanComplexF32x2 → ℂ` via Float32 → Float → ℝ. Float32 values
embed exactly into Float (binary64 has strictly more precision),
so the conversion stays lossless. -/
def LeanComplexF32x2.toComplex (v : LeanComplexF32x2) : ℂ :=
  ⟨v.re.toFloat.toReal, v.im.toFloat.toReal⟩

/-- Reverse direction `ℂ → LeanComplexF64x2`. `noncomputable`
because `ℝ → Float` rounding isn't constructive at Mathlib's
abstract-Real level: ℝ is a Cauchy-sequence quotient and no
total constructive function picks "the nearest representable
Float" without choosing a rounding mode and machinery for
finding the rounded value.

This stub returns `default` (zero) so the bridge type-checks
end-to-end; a follow-up commit picks **round-to-nearest-even**
(IEEE-754 default) and replaces the body with a real
implementation that goes through `Float.ofRat` on a finite
rational approximation.

Until the rounding mode is pinned, callers requiring the
reverse direction should choose their own rounding policy
manually rather than rely on this stub. -/
noncomputable def LeanComplexF64x2.ofComplex (_c : ℂ) : LeanComplexF64x2 :=
  default

/-- Same caveat as `LeanComplexF64x2.ofComplex` — stubbed
`noncomputable` placeholder until a Float32 rounding policy is
pinned. -/
noncomputable def LeanComplexF32x2.ofComplex (_c : ℂ) : LeanComplexF32x2 :=
  default

end Leo4
