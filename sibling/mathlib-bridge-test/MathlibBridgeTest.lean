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
import Leo4.NightlyFloats
import Leo4.MathlibBridge.Wide
import Leo4.MathlibBridge.Complex
import Leo4.MathlibBridge.NightlyFloats
import Leo4.MathlibBridge.Rat
import Mathlib.Init

namespace MathlibBridgeTest

/-- Smoke check: the leo4 surface is in scope and Mathlib resolves. -/
example : Leo4.LeanU128 := ⟨0, 0⟩

-- Wide-integer bridge round-trips (`Leo4.MathlibBridge.Wide`).

example : Leo4.LeanU128.ofNat 42 |>.toNat = 42 := by decide
example : Leo4.LeanU128.ofNat (2^128 - 1) |>.toNat = 2^128 - 1 := by decide
example : Leo4.LeanI128.toInt (Leo4.LeanI128.ofInt 0) = 0 := by decide
example : Leo4.LeanI128.toInt (Leo4.LeanI128.ofInt (-1)) = -1 := by decide

/-- Round-trip `LeanU128 → BitVec 128 → LeanU128` preserves the
limbs at the bit level. We don't prove general bijectivity here —
the test exists so a compile failure surfaces if the underlying
Lean / Mathlib `BitVec` API drifts. -/
example : (Leo4.LeanU128.ofBitVec128
    (Leo4.LeanU128.toBitVec128 ⟨0x1234, 0x5678⟩)) = ⟨0x1234, 0x5678⟩ := by decide

-- Complex bridge type-check smoke tests
-- (`Leo4.MathlibBridge.Complex`). We don't `decide` arithmetic on
-- `ℂ` here — Complex equality goes through `Real` Cauchy
-- sequences, which `decide` can't crack. Just ensuring the
-- conversions elaborate suffices to catch upstream API drift.

example : Complex :=
  Leo4.LeanComplexF64x2.toComplex ⟨1.0, 2.0⟩

example : Complex :=
  Leo4.LeanComplexF32x2.toComplex ⟨3.0, 4.0⟩

-- Nightly-float bridge type-check smoke
-- (`Leo4.MathlibBridge.NightlyFloats`). The `decide` route is
-- unavailable (Real arithmetic), but elaboration still catches
-- API drift.

example : Real := Leo4.LeanF16.toReal ⟨0⟩
example : Real := Leo4.LeanBF16.toReal ⟨0⟩
example : Real := Leo4.LeanF128.toReal ⟨0, 0⟩
example : Complex := Leo4.LeanComplexF16x2.toComplex ⟨⟨0⟩, ⟨0⟩⟩
example : Complex := Leo4.LeanComplexBF16x2.toComplex ⟨⟨0⟩, ⟨0⟩⟩
example : Complex := Leo4.LeanComplexF128x2.toComplex ⟨⟨0, 0⟩, ⟨0, 0⟩⟩

-- ZMod / reverse-Complex elaborate-only smoke (Wide bridge ZMod
-- adds Mathlib `ZMod (2^128)`; reverse Complex is `noncomputable`
-- stub).

example : ZMod (2 ^ 128) := Leo4.LeanU128.toZMod ⟨1, 2⟩
example : Leo4.LeanU128 :=
  Leo4.LeanU128.ofZMod ((42 : ℕ) : ZMod (2 ^ 128))
example : ZMod (2 ^ 128) := Leo4.LeanI128.toZMod ⟨1, 2⟩

noncomputable example : Leo4.LeanComplexF64x2 :=
  Leo4.LeanComplexF64x2.ofComplex ⟨0, 0⟩
noncomputable example : Leo4.LeanComplexF32x2 :=
  Leo4.LeanComplexF32x2.ofComplex ⟨0, 0⟩

-- `Leo4.MathlibBridge.Rat` — Lean core `Rat` ↔ Mathlib `ℝ` / `ℂ`.

example : Real := Leo4.Rat.toReal (0 : Rat)
example : Complex := Leo4.Rat.toComplex (0 : Rat)

end MathlibBridgeTest
