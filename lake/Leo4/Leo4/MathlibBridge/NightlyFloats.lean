-- Leo4.MathlibBridge.NightlyFloats — opt-in IEEE-754 bit-decode
-- conversions from leo4's nightly-only float carriers to Mathlib's
-- `ℝ` (and `ℂ` for the matching complex carriers).
--
-- The carriers live in `Leo4.NightlyFloats` (also opt-in); the Rust
-- side that produces / consumes them is behind the `nightly-floats`
-- cargo feature. The bridge ties the bit-pattern carrier to its
-- exact dyadic-rational value in `ℝ` — total in the *source* sense,
-- but NaN / Inf inputs map to `0 : ℝ` by convention (ℝ has no NaN
-- and `Real.toInhabited` of an exceptional float is undefined).
--
-- The decode is computed by arithmetic on `Nat` field extracts, not
-- by bit-widening into `Float` (which would mishandle subnormal
-- inputs because binary64's subnormal encoding pattern differs from
-- a simple shift of the smaller format's bits). This stays
-- Mathlib-agnostic for the field-extract math; only the final
-- multiplication uses `(2 : ℝ) ^ (z : ℤ)` from Mathlib.
--
-- Forward direction only — reverse `ℝ → LeanF*` is rounding-lossy
-- and waits for a rounding-mode design pass.
--
-- User packages opt in with two imports:
--   `import Leo4.NightlyFloats`
--   `import Leo4.MathlibBridge.NightlyFloats`

import Leo4.NightlyFloats
import Mathlib.Data.Real.Basic
import Mathlib.Data.Complex.Basic

namespace Leo4

private def signFactor (sign : Nat) : ℝ := if sign = 1 then -1 else 1

/-- IEEE-754 binary16 (1 sign / 5 exp / 10 mantissa, bias 15). -/
def LeanF16.toReal (v : LeanF16) : ℝ :=
  let bn   : Nat := v.bits.toNat
  let sign : Nat := bn / 0x8000
  let exp  : Nat := (bn / 0x400) % 0x20
  let mant : Nat := bn % 0x400
  if exp = 0 then
    if mant = 0 then 0
    else
      -- subnormal: sign · mant · 2^(-24)
      signFactor sign * (mant : ℝ) * ((2 : ℝ) ^ (-24 : ℤ))
  else if exp = 0x1F then
    0  -- NaN / Inf — mapped to 0 (ℝ has no NaN; design choice)
  else
    -- normal: sign · (2^10 + mant) · 2^(exp - 15 - 10)
    let mantFull : Nat := 0x400 + mant
    let e : Int := (exp : Int) - 15 - 10
    signFactor sign * (mantFull : ℝ) * ((2 : ℝ) ^ e)

/-- brain-float16 (bfloat16) — 1 sign / 8 exp / 7 mantissa, bias 127. -/
def LeanBF16.toReal (v : LeanBF16) : ℝ :=
  let bn   : Nat := v.bits.toNat
  let sign : Nat := bn / 0x8000
  let exp  : Nat := (bn / 0x80) % 0x100
  let mant : Nat := bn % 0x80
  if exp = 0 then
    if mant = 0 then 0
    else
      -- subnormal: sign · mant · 2^(-133)
      signFactor sign * (mant : ℝ) * ((2 : ℝ) ^ (-133 : ℤ))
  else if exp = 0xFF then
    0
  else
    -- normal: sign · (2^7 + mant) · 2^(exp - 127 - 7)
    let mantFull : Nat := 0x80 + mant
    let e : Int := (exp : Int) - 127 - 7
    signFactor sign * (mantFull : ℝ) * ((2 : ℝ) ^ e)

/-- IEEE-754 binary128 (1 sign / 15 exp / 112 mantissa, bias 16383).
The carrier is `LeanF128 { lo, hi : UInt64 }`; the full 128 bits
pack as `(hi << 64) | lo`. We extract sign / exp from `hi` and
recombine the 112-bit mantissa across `hi`'s low 48 bits and the
full 64 bits of `lo`. -/
def LeanF128.toReal (v : LeanF128) : ℝ :=
  let hi : Nat := v.hi.toNat
  let lo : Nat := v.lo.toNat
  let sign : Nat := hi / (2 ^ 63)
  let exp  : Nat := (hi / (2 ^ 48)) % 0x8000
  -- mantissa: lower 48 bits of hi + full 64 bits of lo = 112 bits
  let mantHi : Nat := hi % (2 ^ 48)
  let mantFull112 : Nat := mantHi * (2 ^ 64) + lo
  if exp = 0 then
    if mantFull112 = 0 then 0
    else
      signFactor sign * (mantFull112 : ℝ) * ((2 : ℝ) ^ (-16494 : ℤ))
      -- subnormal: sign · mant · 2^(-16382 - 112) = · 2^(-16494)
  else if exp = 0x7FFF then
    0
  else
    -- normal: sign · (2^112 + mant) · 2^(exp - 16383 - 112)
    let mantFull : Nat := (2 ^ 112) + mantFull112
    let e : Int := (exp : Int) - 16383 - 112
    signFactor sign * (mantFull : ℝ) * ((2 : ℝ) ^ e)

/-- Machine complex `(f16, f16) → ℂ`. -/
def LeanComplexF16x2.toComplex (v : LeanComplexF16x2) : ℂ :=
  ⟨v.re.toReal, v.im.toReal⟩

/-- Machine complex `(bf16, bf16) → ℂ`. -/
def LeanComplexBF16x2.toComplex (v : LeanComplexBF16x2) : ℂ :=
  ⟨v.re.toReal, v.im.toReal⟩

/-- Machine complex `(f128, f128) → ℂ`. -/
def LeanComplexF128x2.toComplex (v : LeanComplexF128x2) : ℂ :=
  ⟨v.re.toReal, v.im.toReal⟩

/-! ## Reverse direction stubs (`ℝ → LeanF*`, `ℂ → LeanComplex*x2`)

All `noncomputable`. Rationale: `ℝ → Float`-style rounding is not
constructive at Mathlib's abstract-Real level (Cauchy quotient).
A real implementation requires pinning a rounding mode
(IEEE-754 round-to-nearest-even / truncate / toward zero / …)
and machinery to find the rounded value. Future commits land
each as the use case demands.

The stubs return `default` so the bridge type-checks end-to-end
and downstream proofs can reference the function symbol. They
are NOT for runtime use — callers needing actual rounding
should pick a policy and implement their own. -/

noncomputable def LeanF16.ofReal (_r : ℝ) : LeanF16 := default
noncomputable def LeanBF16.ofReal (_r : ℝ) : LeanBF16 := default
noncomputable def LeanF128.ofReal (_r : ℝ) : LeanF128 := default

noncomputable def LeanComplexF16x2.ofComplex (_c : ℂ) : LeanComplexF16x2 := default
noncomputable def LeanComplexBF16x2.ofComplex (_c : ℂ) : LeanComplexBF16x2 := default
noncomputable def LeanComplexF128x2.ofComplex (_c : ℂ) : LeanComplexF128x2 := default

end Leo4
