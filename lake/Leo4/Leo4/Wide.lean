-- Leo4.Wide — wide integer / complex / vector carrier types whose wire
-- format leo4 supports unconditionally on stable Rust. Each type is a
-- Lean structure with byte-level fields; the canonical-ABI wire is
-- formed by concatenating field encodings in declaration order, so
-- the byte stream matches a Rust `<T>::to_le_bytes()` for the
-- corresponding primitive.
--
-- 2026-05-20 entries:
--   • `LeanU128` { lo, hi : UInt64 } — pairs with Rust `u128`. Wire:
--     16 bytes LE.
--   • `LeanI128` { lo, hi : UInt64 } — pairs with Rust `i128`. Wire:
--     16 bytes LE two's-complement (the sign lives in the top bit of
--     `hi`; consumers do their own conversion if they need
--     sign-magnitude).
--
-- Auto-imported by `Leo4` so admit-sets and `deriving LeanMarshal`
-- can see them without a separate opt-in.
--
-- Future additions (LeanComplexF{32,64}x2 etc.) land in the same
-- module; nightly-only float types (LeanF16 / BF16 / F128 + their
-- Complex friends) live in `Leo4.NightlyFloats` and stay opt-in.

import Leo4.Marshal
import Leo4.Builtins
import Leo4.Deriving

namespace Leo4

/-- Rust `u128` carrier. Two UInt64 limbs in little-endian order
(`lo` is bytes 0..7, `hi` is bytes 8..15). The plugin lowers this
to an IDL `record Leo4.LeanU128 { lo: u64, hi: u64 }`; the wire
form is byte-identical to `u128::to_le_bytes()`. -/
structure LeanU128 where
  lo : UInt64
  hi : UInt64
  deriving Repr, DecidableEq, Inhabited, LeanMarshal

/-- Rust `i128` carrier. Same layout as `LeanU128` — the sign lives
in the top bit of `hi` per two's complement. Wire form is
byte-identical to `i128::to_le_bytes()`. -/
structure LeanI128 where
  lo : UInt64
  hi : UInt64
  deriving Repr, DecidableEq, Inhabited, LeanMarshal

end Leo4
