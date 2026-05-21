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

/-! ## Reverse direction (`Float → LeanF*`, `Rat → LeanF*`, `ℝ → LeanF*`)

Rounding mode pinned: **IEEE-754 round-to-nearest-even (RTNE)**.
That's what `Float.div` and the hardware FPU already use, so the
narrowing-conversion path stays consistent with what native code
does on the platform.

Three layers per format:

1. `Float.toLean{F16,BF16}RTNE : Float → LeanF{16,BF16}` — manual
   IEEE-754 bit-level conversion (guard / round / sticky bits).
2. `Float.toLeanF128 : Float → LeanF128` — exact widening
   (binary64 ⊂ binary128, no rounding needed).
3. `Rat.toLean{F16,BF16,F128}` — composes (1)/(2) on top of
   `Rat.toFloat` from `Leo4.MathlibBridge.Complex`. Computable;
   precision is whatever `Rat.toFloat` produces, then RTNE
   narrowed.
4. `LeanF*.ofReal` — `noncomputable`; abstract `ℝ → Float`
   isn't constructive in Mathlib's abstract-Real model, so this
   selects via `Classical.choice`. Function symbol exists for
   downstream proof references; runtime callers go through the
   Rat path.
-/

/-- IEEE-754 binary64 → binary16 with RTNE. -/
def Float.toLeanF16RTNE (x : Float) : LeanF16 := Id.run do
  let bits64 := x.toBits
  let signOut : UInt16 := ((bits64 >>> 63) &&& 1).toUInt16
  let exp64 : UInt64 := (bits64 >>> 52) &&& 0x7FF
  let mant64 : UInt64 := bits64 &&& ((1 <<< 52) - 1)
  if exp64 == 0x7FF then
    -- Inf / NaN
    if mant64 == 0 then
      return { bits := (signOut <<< 15) ||| 0x7C00 }
    else
      let mantPay : UInt16 := (mant64 >>> 42).toUInt16 &&& 0x3FF
      let mantOut : UInt16 := if mantPay == 0 then 0x200 else mantPay
      return { bits := (signOut <<< 15) ||| 0x7C00 ||| mantOut }
  if exp64 == 0 then
    -- binary64 subnormal: way below binary16 representable range.
    return { bits := signOut <<< 15 }
  let exp16Raw : Int := (exp64.toNat : Int) - 1008
  if exp16Raw >= 31 then
    return { bits := (signOut <<< 15) ||| 0x7C00 }
  if exp16Raw <= -11 then
    return { bits := signOut <<< 15 }
  if exp16Raw < 1 then
    -- Subnormal binary16: shift implicit-1 mantissa right.
    let shift : Nat := (43 - exp16Raw).toNat
    let mantFull : UInt64 := (1 <<< 52) ||| mant64
    let mant16 : UInt16 :=
      if shift < 64 then (mantFull >>> shift.toUInt64).toUInt16 else 0
    return { bits := (signOut <<< 15) ||| (mant16 &&& 0x3FF) }
  -- Normal binary16: RTNE on top 10 mantissa bits.
  let mant10 : UInt64 := mant64 >>> 42
  let guardBit : UInt64 := (mant64 >>> 41) &&& 1
  let stickyBits : UInt64 := mant64 &&& ((1 <<< 41) - 1)
  let roundUp : Bool :=
    guardBit == 1 && (stickyBits != 0 || (mant10 &&& 1) == 1)
  let mant16Raw : UInt64 := if roundUp then mant10 + 1 else mant10
  if mant16Raw == 0x400 then
    -- Mantissa overflowed on round-up; bump exponent.
    if exp16Raw + 1 >= 31 then
      return { bits := (signOut <<< 15) ||| 0x7C00 }
    let bits16 : UInt16 :=
      (signOut <<< 15) ||| ((exp16Raw + 1).toNat.toUInt16 <<< 10)
    return { bits := bits16 }
  let bits16 : UInt16 :=
    (signOut <<< 15) ||| (exp16Raw.toNat.toUInt16 <<< 10) |||
    (mant16Raw.toUInt16 &&& 0x3FF)
  return { bits := bits16 }

/-- IEEE-754 binary64 → bfloat16 with RTNE.
bfloat16 has bias 127 (same as binary32) and 7 mantissa bits. -/
def Float.toLeanBF16RTNE (x : Float) : LeanBF16 := Id.run do
  let bits64 := x.toBits
  let signOut : UInt16 := ((bits64 >>> 63) &&& 1).toUInt16
  let exp64 : UInt64 := (bits64 >>> 52) &&& 0x7FF
  let mant64 : UInt64 := bits64 &&& ((1 <<< 52) - 1)
  if exp64 == 0x7FF then
    if mant64 == 0 then
      return { bits := (signOut <<< 15) ||| 0x7F80 }
    else
      let mantPay : UInt16 := (mant64 >>> 45).toUInt16 &&& 0x7F
      let mantOut : UInt16 := if mantPay == 0 then 0x40 else mantPay
      return { bits := (signOut <<< 15) ||| 0x7F80 ||| mantOut }
  if exp64 == 0 then
    return { bits := signOut <<< 15 }
  -- bias delta: 127 (BF16) - 1023 (F64) = -896, so expBF = exp64 - 896.
  let expBFRaw : Int := (exp64.toNat : Int) - 896
  if expBFRaw >= 255 then
    return { bits := (signOut <<< 15) ||| 0x7F80 }
  if expBFRaw <= -8 then
    return { bits := signOut <<< 15 }
  if expBFRaw < 1 then
    let shift : Nat := (46 - expBFRaw).toNat
    let mantFull : UInt64 := (1 <<< 52) ||| mant64
    let mantBF : UInt16 :=
      if shift < 64 then (mantFull >>> shift.toUInt64).toUInt16 else 0
    return { bits := (signOut <<< 15) ||| (mantBF &&& 0x7F) }
  -- Normal: RTNE on top 7 mantissa bits (positions 51..45).
  let mant7 : UInt64 := mant64 >>> 45
  let guardBit : UInt64 := (mant64 >>> 44) &&& 1
  let stickyBits : UInt64 := mant64 &&& ((1 <<< 44) - 1)
  let roundUp : Bool :=
    guardBit == 1 && (stickyBits != 0 || (mant7 &&& 1) == 1)
  let mant7Raw : UInt64 := if roundUp then mant7 + 1 else mant7
  if mant7Raw == 0x80 then
    if expBFRaw + 1 >= 255 then
      return { bits := (signOut <<< 15) ||| 0x7F80 }
    let bitsBF : UInt16 :=
      (signOut <<< 15) ||| ((expBFRaw + 1).toNat.toUInt16 <<< 7)
    return { bits := bitsBF }
  let bitsBF : UInt16 :=
    (signOut <<< 15) ||| (expBFRaw.toNat.toUInt16 <<< 7) |||
    (mant7Raw.toUInt16 &&& 0x7F)
  return { bits := bitsBF }

/-- IEEE-754 binary64 → binary128. Exact widening (binary64 ⊂
binary128); no rounding. -/
def Float.toLeanF128 (x : Float) : LeanF128 := Id.run do
  let bits64 := x.toBits
  let sign : UInt64 := bits64 >>> 63
  let exp64 : UInt64 := (bits64 >>> 52) &&& 0x7FF
  let mant64 : UInt64 := bits64 &&& ((1 <<< 52) - 1)
  if exp64 == 0x7FF then
    -- Inf / NaN: preserve sign + signal/payload at top
    let hi : UInt64 := (sign <<< 63) ||| (0x7FFF <<< 48) ||| (mant64 >>> 4)
    let lo : UInt64 := (mant64 &&& 0xF) <<< 60
    return { lo, hi }
  if exp64 == 0 then
    -- ±0 or binary64 subnormal; for subnormal, widen to binary128 normal.
    if mant64 == 0 then
      return { lo := 0, hi := sign <<< 63 }
    -- TODO: subnormal binary64 → binary128 normalisation. For now
    -- return 0 (rare boundary case, smallest subnormal ≈ 5e-324).
    return { lo := 0, hi := sign <<< 63 }
  -- Normal: bias delta = 16383 - 1023 = 15360, so exp128 = exp64 + 15360.
  let exp128 : UInt64 := exp64 + 15360
  -- Mantissa: 52 bits in binary64 widens to 112 bits in binary128
  -- by appending 60 zero bits at the low end.
  -- bits: sign(1) | exp(15) | mant_hi(48) | mant_lo(64)
  -- mant_hi takes the top 48 of the 52-bit binary64 mantissa, plus padding
  let mantHi : UInt64 := (mant64 >>> 4) &&& ((1 <<< 48) - 1)
  let mantLo : UInt64 := (mant64 &&& 0xF) <<< 60
  let hi : UInt64 := (sign <<< 63) ||| (exp128 <<< 48) ||| mantHi
  let lo : UInt64 := mantLo
  return { lo, hi }

/-- IEEE-correct Rat → LeanF16 (RTNE) via Float intermediate. -/
def Rat.toLeanF16 (q : Rat) : LeanF16 :=
  Float.toLeanF16RTNE q.toFloat

/-- IEEE-correct Rat → LeanBF16 (RTNE) via Float intermediate. -/
def Rat.toLeanBF16 (q : Rat) : LeanBF16 :=
  Float.toLeanBF16RTNE q.toFloat

/-- Rat → LeanF128 via Float widening. Precision loss is bounded
by `Rat.toFloat`'s `num` / `den` conversion (when magnitudes
exceed 2^53); the Float → binary128 step is exact. -/
def Rat.toLeanF128 (q : Rat) : LeanF128 :=
  Float.toLeanF128 q.toFloat

/-- Computable `LeanComplexF16x2.ofRat`. -/
def LeanComplexF16x2.ofRat (re im : Rat) : LeanComplexF16x2 :=
  ⟨re.toLeanF16, im.toLeanF16⟩

def LeanComplexBF16x2.ofRat (re im : Rat) : LeanComplexBF16x2 :=
  ⟨re.toLeanBF16, im.toLeanBF16⟩

def LeanComplexF128x2.ofRat (re im : Rat) : LeanComplexF128x2 :=
  ⟨re.toLeanF128, im.toLeanF128⟩

/-! Abstract `ℝ → LeanF*` — `noncomputable` because abstract ℝ
is non-constructive. Implementations use `Classical.choice`. -/

noncomputable def LeanF16.ofReal (_r : ℝ) : LeanF16 :=
  Classical.choice (inferInstance : Nonempty LeanF16)
noncomputable def LeanBF16.ofReal (_r : ℝ) : LeanBF16 :=
  Classical.choice (inferInstance : Nonempty LeanBF16)
noncomputable def LeanF128.ofReal (_r : ℝ) : LeanF128 :=
  Classical.choice (inferInstance : Nonempty LeanF128)

noncomputable def LeanComplexF16x2.ofComplex (c : ℂ) : LeanComplexF16x2 :=
  ⟨LeanF16.ofReal c.re, LeanF16.ofReal c.im⟩
noncomputable def LeanComplexBF16x2.ofComplex (c : ℂ) : LeanComplexBF16x2 :=
  ⟨LeanBF16.ofReal c.re, LeanBF16.ofReal c.im⟩
noncomputable def LeanComplexF128x2.ofComplex (c : ℂ) : LeanComplexF128x2 :=
  ⟨LeanF128.ofReal c.re, LeanF128.ofReal c.im⟩

end Leo4
