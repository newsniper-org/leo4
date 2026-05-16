-- Leo4.Marshal — `LeanMarshal` typeclass + `LeanError` wire-error type.
--
-- See LEO4-DESIGN.md §10.1 for the design, SPEC/canonical-abi.md for the
-- normative wire format, and SPEC/handshake.md for how the plugin emits
-- the mangled symbols that ultimately call into these encode/decode
-- implementations.

import Lean

namespace Leo4

open Lean

/--
Wire-level error returned by canonical-ABI encode/decode operations.
Mirrors the on-the-wire `error` record defined in
`SPEC/canonical-abi.md` §13.
-/
structure LeanError where
  code      : UInt32
  message   : String
  backtrace : Option String := none
  deriving Inhabited

namespace LeanError

-- Reserved leo4-runtime error codes (SPEC/canonical-abi.md §13).
def decodeError         : UInt32 := 0x0000_0001
def encodeError         : UInt32 := 0x0000_0002
def invalidHandle       : UInt32 := 0x0000_0003
def oom                 : UInt32 := 0x0000_0004
def handshakeMismatch   : UInt32 := 0x0000_0005
def unknownFunction     : UInt32 := 0x0000_0006
def bufferTooSmall      : UInt32 := 0x0000_0007
def decodeDepthExceeded : UInt32 := 0x0000_0008

/-- Convenience constructor — no backtrace. -/
def mk' (code : UInt32) (msg : String) : LeanError :=
  { code, message := msg, backtrace := none }

end LeanError

/--
Canonical-ABI marshalling for boundary types.

`canonicalEncode v buf` appends `v`'s little-endian wire encoding to
`buf` and returns the updated buffer (ByteArray's in-place mutation is
preserved when uniquely owned).

`canonicalDecode buf off` reads one `T` starting at byte offset `off`
of `buf`. On success returns the decoded value together with the
offset one past its last byte; on failure returns a `LeanError`
carrying an error code from `LeanError.*`.

Wire formats are normative — `SPEC/canonical-abi.md` enumerates each
type's exact bytes. Two implementations of `LeanMarshal T` for the
same `T` MUST produce identical bytes; the cross-impl conformance
suite in `tests/conformance/` (Phase 4+) will pin that.

`LeanMarshal` and `LeanResource` (`Leo4.Resource`) cover **disjoint**
populations: a type is either marshalled by value (its bytes cross the
boundary) or held by an opaque resource handle (a `u64` handle
crosses; the bytes do not). The plugin enforces mutual exclusion at
admit-set computation time (LEO4-DESIGN.md §10.1).
-/
class LeanMarshal (T : Type) where
  canonicalEncode : T → ByteArray → ByteArray
  canonicalDecode : ByteArray → Nat → Except LeanError (T × Nat)

end Leo4
