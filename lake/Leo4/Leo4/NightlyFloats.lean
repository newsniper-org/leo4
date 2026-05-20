-- Leo4.NightlyFloats — opt-in carrier types for floats whose Rust
-- counterpart requires nightly (`f16`, `f128`) plus `bf16` (no
-- native Rust primitive yet — carried as bit pattern).
--
-- User packages opt in via `import Leo4.NightlyFloats`; the
-- top-level `Leo4` module does NOT auto-import this so the default
-- surface stays stable-Rust only. The corresponding Rust types live
-- in `crates/leo4-abi/src/floats_nightly.rs` behind the
-- `nightly-floats` cargo feature.
--
-- Lean core has none of these float widths as primitives (as of
-- v4.29.1) — `Float` is binary64 and `Float32` is binary32. The
-- carrier structures wrap raw bit patterns (UInt16 or two UInt64s)
-- so the wire is byte-identical to the Rust nightly representation
-- via `<float>::to_le_bytes()`.
--
-- Wire summary:
--   • LeanF16            — 2 B LE (UInt16 bits)
--   • LeanBF16           — 2 B LE (UInt16 bits)
--   • LeanF128           — 16 B LE (lo UInt64 ‖ hi UInt64)
--   • LeanComplexF16x2   — 4 B LE
--   • LeanComplexBF16x2  — 4 B LE
--   • LeanComplexF128x2  — 32 B LE

import Leo4.Marshal
import Leo4.Builtins
import Leo4.Deriving

namespace Leo4

/-- IEEE-754 binary16 carrier. Wire: 2 bytes LE. -/
structure LeanF16 where
  bits : UInt16
  deriving Repr, DecidableEq, Inhabited, LeanMarshal

/-- brain-float16 (bfloat16) carrier. Same on-wire shape as
`LeanF16` but different bit-layout semantics (1 sign / 8 exponent /
7 mantissa). Wire: 2 bytes LE. -/
structure LeanBF16 where
  bits : UInt16
  deriving Repr, DecidableEq, Inhabited, LeanMarshal

/-- IEEE-754 binary128 carrier. Two UInt64 limbs in LE order
(`lo` = bytes 0..7, `hi` = bytes 8..15) — matches Rust's
`f128::to_le_bytes()`. Wire: 16 bytes LE. -/
structure LeanF128 where
  lo : UInt64
  hi : UInt64
  deriving Repr, DecidableEq, Inhabited, LeanMarshal

/-- `(f16, f16)` machine complex. Wire: 4 B LE. -/
structure LeanComplexF16x2 where
  re : LeanF16
  im : LeanF16
  deriving Repr, DecidableEq, Inhabited, LeanMarshal

/-- `(bf16, bf16)` machine complex. Wire: 4 B LE. -/
structure LeanComplexBF16x2 where
  re : LeanBF16
  im : LeanBF16
  deriving Repr, DecidableEq, Inhabited, LeanMarshal

/-- `(f128, f128)` machine complex. Wire: 32 B LE. -/
structure LeanComplexF128x2 where
  re : LeanF128
  im : LeanF128
  deriving Repr, DecidableEq, Inhabited, LeanMarshal

end Leo4
