-- tests/conformance/gen.lean — Lean-side encoder fixture generator.
--
-- For each (name, value) pair below, emits one `name=<hex bytes>` line.
-- The Rust integration test (`crates/leo4-abi/tests/cross_impl.rs`)
-- parses the file, re-encodes the named value on the Rust side, and
-- asserts byte-for-byte equality.
--
-- Run via `tests/conformance/run.sh` (`just conformance-test`).

import Leo4

open Leo4

private def hexDigit (n : Nat) : Char :=
  if n < 10 then Char.ofNat (n + 0x30) else Char.ofNat (n - 10 + 0x61)

private def byteToHex (b : UInt8) : String :=
  let n := b.toNat
  String.ofList [hexDigit (n / 16), hexDigit (n % 16)]

private def hexEncode (bs : ByteArray) : String := Id.run do
  let mut out := ""
  for i in [0 : bs.size] do
    out := out ++ byteToHex (bs.get! i)
  return out

private def emit [LeanMarshal α] (name : String) (v : α) : IO Unit := do
  let bs := LeanMarshal.canonicalEncode v ByteArray.empty
  IO.println s!"{name}={hexEncode bs}"

def main : IO Unit := do
  -- unsigned ints
  emit "u8/0x00" (0   : UInt8)
  emit "u8/0xab" (171 : UInt8)
  emit "u8/0xff" (255 : UInt8)
  emit "u16/0xcafe" (0xcafe : UInt16)
  emit "u32/0xdeadbeef" (0xdeadbeef : UInt32)
  emit "u64/0x123456789abcdef0" (0x123456789abcdef0 : UInt64)
  -- signed ints
  emit "i8/0" (0    : Int8)
  emit "i8/-1" (-1  : Int8)
  emit "i8/min" Int8.minValue
  emit "i8/max" Int8.maxValue
  emit "i16/-1" (-1 : Int16)
  emit "i32/min" Int32.minValue
  emit "i64/max" Int64.maxValue
  -- floats
  emit "f32/0.0" (0.0 : Float32)
  emit "f32/-0.0" (-0.0 : Float32)
  emit "f64/pi" (3.141592653589793 : Float)
  -- bool
  emit "bool/false" false
  emit "bool/true"  true
  -- char
  emit "char/A" 'A'
  emit "char/han" '한'
  -- string
  emit "string/empty" ""
  emit "string/hello" "hello"
  emit "string/han" "안녕"
  -- nat / int (BigNat / BigInt wire form)
  emit "nat/0" (0 : Nat)
  emit "nat/1" (1 : Nat)
  emit "nat/u64max" ((0xFFFFFFFFFFFFFFFF : Nat))
  emit "int/0" (0 : Int)
  emit "int/-1" (-1 : Int)
  emit "int/+0xdeadbeefcafebabe" (0xdeadbeefcafebabe : Int)
