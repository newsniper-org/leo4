-- Leo4.MathlibBridge.Wide — opt-in 1-to-1 conversions between leo4's
-- wide-integer carriers (`LeanU128`, `LeanI128`) and the abstract
-- integer types Mathlib reasons about.
--
-- This first cut wires the *Lean core* sides (`Nat`, `Int`,
-- `BitVec 128`); the Mathlib-specific bridges (`ZMod (2^128)`,
-- `Fin (2^128)`, …) land in follow-up commits that import the
-- matching Mathlib modules. The file pulls in `Mathlib.Init` so
-- it lives in the same compile graph as the other Mathlib bridges
-- and the sibling `mathlib-bridge-test` package builds them all
-- together.
--
-- User packages opt in via `import Leo4.MathlibBridge.Wide`. The
-- top-level `Leo4` import does NOT pull this — leo4 core stays
-- Mathlib-independent per ROADMAP §8.
--
-- All conversions are total: `LeanU128` ↔ `BitVec 128` is a
-- bijection (both are 128 bits of payload); `LeanU128` → `Nat` is
-- total but `Nat` → `LeanU128` is partial (Nat values ≥ 2^128 must
-- truncate, exposed as `?` suffix).

import Leo4.Wide
import Mathlib.Init
import Mathlib.Data.ZMod.Basic

namespace Leo4

/-- Pack `LeanU128` as a single `Nat`. Total, lossless. -/
def LeanU128.toNat (v : LeanU128) : Nat :=
  v.hi.toNat * (2 ^ 64) + v.lo.toNat

/-- Recover a `LeanU128` from a `Nat`. Values `≥ 2^128` truncate to
the low 128 bits; the caller checks for overflow via `n < 2^128` if
they care. -/
def LeanU128.ofNat (n : Nat) : LeanU128 where
  lo := (n &&& (2^64 - 1)).toUInt64
  hi := (n >>> 64).toUInt64

/-- Pack `LeanI128` as a `Int`. The sign lives in bit 127 of the
two's-complement layout; we read the raw 128 bits as `Nat` and
adjust for the sign bit. -/
def LeanI128.toInt (v : LeanI128) : Int :=
  let raw : Nat := v.hi.toNat * (2 ^ 64) + v.lo.toNat
  if v.hi.toBitVec.msb then
    (raw : Int) - (2 ^ 128 : Int)
  else
    (raw : Int)

/-- Encode a `Int` value back into `LeanI128`. Values outside
`[-2^127, 2^127)` wrap modulo `2^128` (two's-complement
truncation). -/
def LeanI128.ofInt (i : Int) : LeanI128 :=
  let modded : Int := i % (2 ^ 128 : Int)
  let raw : Nat := if modded < 0 then ((2 ^ 128 : Int) + modded).toNat else modded.toNat
  { lo := (raw &&& (2^64 - 1)).toUInt64
  , hi := (raw >>> 64).toUInt64 }

/-- View `LeanU128` as a `BitVec 128`. Concatenates `hi` (high 64
bits) with `lo` (low 64 bits). The bit layout matches Rust's
`u128::to_le_bytes()` interpretation. -/
def LeanU128.toBitVec128 (v : LeanU128) : BitVec 128 :=
  let hiV : BitVec 64 := BitVec.ofNat 64 v.hi.toNat
  let loV : BitVec 64 := BitVec.ofNat 64 v.lo.toNat
  hiV ++ loV

/-- Recover `LeanU128` from a `BitVec 128`. Inverse of `toBitVec128`. -/
def LeanU128.ofBitVec128 (b : BitVec 128) : LeanU128 where
  lo := (b.extractLsb' 0 64).toNat.toUInt64
  hi := (b.extractLsb' 64 64).toNat.toUInt64

/-- View `LeanI128` as a `BitVec 128`. Bit-identical to
`LeanU128.toBitVec128` modulo the sign interpretation (BitVec has
no sign; consumers apply `BitVec.toInt` if they want signed view). -/
def LeanI128.toBitVec128 (v : LeanI128) : BitVec 128 :=
  let hiV : BitVec 64 := BitVec.ofNat 64 v.hi.toNat
  let loV : BitVec 64 := BitVec.ofNat 64 v.lo.toNat
  hiV ++ loV

/-- Recover `LeanI128` from a `BitVec 128`. -/
def LeanI128.ofBitVec128 (b : BitVec 128) : LeanI128 where
  lo := (b.extractLsb' 0 64).toNat.toUInt64
  hi := (b.extractLsb' 64 64).toNat.toUInt64

/-- View `LeanU128` as `ZMod (2^128)`. Mathlib's `ZMod n` is
`Fin n` for `n > 0`, with arithmetic mod `n`; `(v.toNat :
ZMod (2^128))` invokes the `NatCast` instance. -/
def LeanU128.toZMod (v : LeanU128) : ZMod (2 ^ 128) :=
  (v.toNat : ZMod (2 ^ 128))

/-- Recover a `LeanU128` from `ZMod (2^128)`. `ZMod.val` gives the
canonical representative in `0..2^128 - 1`. -/
def LeanU128.ofZMod (z : ZMod (2 ^ 128)) : LeanU128 :=
  LeanU128.ofNat z.val

/-- View `LeanI128` as `ZMod (2^128)` (unsigned representative).
Consumers can recover the signed view via `ZMod.toIntInRange` or
similar Mathlib helpers. -/
def LeanI128.toZMod (v : LeanI128) : ZMod (2 ^ 128) :=
  ((v.hi.toNat * (2 ^ 64) + v.lo.toNat : Nat) : ZMod (2 ^ 128))

/-- Recover a `LeanI128` from `ZMod (2^128)`. The bit pattern at
position 127 carries the sign per two's complement; callers who
want a signed `Int` view first convert with `LeanI128.ofZMod` and
then `LeanI128.toInt`. -/
def LeanI128.ofZMod (z : ZMod (2 ^ 128)) : LeanI128 :=
  let n := z.val
  { lo := (n &&& (2 ^ 64 - 1)).toUInt64
  , hi := (n >>> 64).toUInt64 }

end Leo4
