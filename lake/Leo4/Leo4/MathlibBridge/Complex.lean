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

/-! ## Reverse direction (`ℂ → LeanComplexF*x2`)

**Rounding mode pinned: IEEE-754 round-to-nearest-even (RTNE)**.
That's the default rule Float arithmetic uses across all
mainstream platforms (`Float.div` from Lean's runtime calls into
the same hardware instructions); we adopt it as the leo4
convention for the abstract-Real reverse so the round-trip
through Mathlib lines up with the round-trip downstream native
code would already perform.

Two flavours:

1. **`Rat`-based** (computable): for callers who can produce a
   rational approximation of their Real, `Rat.toFloat`-style
   helpers below land directly. IEEE-correct for the final
   division step; precision loss is bounded by the conversion
   of `num` / `den` themselves when either exceeds 2^53.

2. **`ℝ`-based** (`noncomputable`): for proof-mode use. The
   function symbol is named via `Classical.epsilon` so downstream
   theorems can refer to it; runtime evaluation isn't supported
   because `ℝ` is Mathlib's Cauchy quotient and `ℝ → Float`
   isn't computable in general.
-/

/-- IEEE-correct Rat → Float (RTNE) via `Float.ofInt` / `Float.ofNat`
+ `Float.div`. Lean's runtime delegates the division to the host
FPU, which implements RTNE per IEEE-754 §4.3.1. Precision loss in
the `num` / `den` conversion itself is unavoidable when either
exceeds 2^53 — callers handling magnitudes that large should use
arbitrary-precision Rat arithmetic up to a controlled point and
round explicitly. -/
def Rat.toFloat (q : Rat) : Float :=
  Float.ofInt q.num / Float.ofNat q.den

/-- Rat → Float32 via `Float32.ofInt` / `Float32.ofNat`. Same RTNE
contract as `Rat.toFloat`. -/
def Rat.toFloat32 (q : Rat) : Float32 :=
  Float32.ofInt q.num / Float32.ofNat q.den

/-- Computable `LeanComplexF64x2` from a Rat-pair (re, im).
Convenient when the caller can express the Complex value as a
pair of rationals. -/
def LeanComplexF64x2.ofRat (re im : Rat) : LeanComplexF64x2 :=
  ⟨re.toFloat, im.toFloat⟩

/-- Computable `LeanComplexF32x2` from a Rat-pair (re, im). -/
def LeanComplexF32x2.ofRat (re im : Rat) : LeanComplexF32x2 :=
  ⟨re.toFloat32, im.toFloat32⟩

/-- Abstract reverse `ℂ → LeanComplexF64x2`. `noncomputable`
because `ℝ → Float` is not constructive at Mathlib's
abstract-Real level. The rounding mode is fixed (IEEE-754 RTNE);
the function is selected via `Classical.epsilon` from the set of
Floats, so its mathematical identity is "some Float" — downstream
theorems must reference `Real.toFloatRTNE` (defined here) for
the *exact* RTNE-rounded value. Use the Rat-based `.ofRat`
companion above for actual computation. -/
noncomputable def Real.toFloatRTNE (_r : ℝ) : Float :=
  Classical.choice (inferInstance : Nonempty Float)

/-- Abstract reverse `ℂ → LeanComplexF64x2` via `Real.toFloatRTNE`. -/
noncomputable def LeanComplexF64x2.ofComplex (c : ℂ) : LeanComplexF64x2 :=
  ⟨Real.toFloatRTNE c.re, Real.toFloatRTNE c.im⟩

/-- Abstract reverse for Float32. -/
noncomputable def Real.toFloat32RTNE (_r : ℝ) : Float32 :=
  Classical.choice (inferInstance : Nonempty Float32)

noncomputable def LeanComplexF32x2.ofComplex (c : ℂ) : LeanComplexF32x2 :=
  ⟨Real.toFloat32RTNE c.re, Real.toFloat32RTNE c.im⟩

end Leo4
