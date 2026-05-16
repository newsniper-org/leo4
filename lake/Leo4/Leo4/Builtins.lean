-- Leo4.Builtins — `LeanMarshal` instances for built-in primitives and
-- structural composites.
--
-- Wire format: SPEC/canonical-abi.md. Little-endian throughout.

import Leo4.Marshal

namespace Leo4

open Leo4 (LeanError LeanMarshal)

/-! ## Little-endian byte helpers -/

/-- Append the low `n` bytes of `val`, least significant first. -/
private def pushLE (n : Nat) (val : UInt64) (buf : ByteArray) : ByteArray := Id.run do
  let mut out := buf
  for i in [0:n] do
    let byte : UInt8 := ((val >>> (i * 8).toUInt64) &&& 0xFF).toUInt8
    out := out.push byte
  return out

/-- Read `n` little-endian bytes as a `UInt64`; bounds-check first. -/
private def readLE (n : Nat) (buf : ByteArray) (off : Nat) : Except LeanError (UInt64 × Nat) := do
  if off + n > buf.size then
    throw (LeanError.mk' LeanError.decodeError s!"out of bounds reading {n} bytes at offset {off}")
  let mut acc : UInt64 := 0
  for i in [0:n] do
    acc := acc ||| ((buf.get! (off + i)).toUInt64 <<< (i * 8).toUInt64)
  return (acc, off + n)

/-! ## Unsigned scalars -/

instance : LeanMarshal UInt8 where
  canonicalEncode v buf := buf.push v
  canonicalDecode buf off := do
    if off + 1 > buf.size then
      throw (LeanError.mk' LeanError.decodeError "u8: out of bounds")
    return (buf.get! off, off + 1)

instance : LeanMarshal UInt16 where
  canonicalEncode v buf := pushLE 2 v.toUInt64 buf
  canonicalDecode buf off := do
    let (n, off) ← readLE 2 buf off
    return (n.toUInt16, off)

instance : LeanMarshal UInt32 where
  canonicalEncode v buf := pushLE 4 v.toUInt64 buf
  canonicalDecode buf off := do
    let (n, off) ← readLE 4 buf off
    return (n.toUInt32, off)

instance : LeanMarshal UInt64 where
  canonicalEncode v buf := pushLE 8 v buf
  canonicalDecode buf off := readLE 8 buf off

/-! ## Signed scalars -/

instance : LeanMarshal Int8 where
  canonicalEncode v buf := buf.push v.toUInt8
  canonicalDecode buf off := do
    let (n, off) ← readLE 1 buf off
    return (n.toUInt8.toInt8, off)

instance : LeanMarshal Int16 where
  canonicalEncode v buf := pushLE 2 v.toUInt16.toUInt64 buf
  canonicalDecode buf off := do
    let (n, off) ← readLE 2 buf off
    return (n.toUInt16.toInt16, off)

instance : LeanMarshal Int32 where
  canonicalEncode v buf := pushLE 4 v.toUInt32.toUInt64 buf
  canonicalDecode buf off := do
    let (n, off) ← readLE 4 buf off
    return (n.toUInt32.toInt32, off)

instance : LeanMarshal Int64 where
  canonicalEncode v buf := pushLE 8 v.toUInt64 buf
  canonicalDecode buf off := do
    let (n, off) ← readLE 8 buf off
    return (n.toInt64, off)

/-! ## Floating-point -/

instance : LeanMarshal Float32 where
  canonicalEncode v buf := pushLE 4 v.toBits.toUInt64 buf
  canonicalDecode buf off := do
    let (n, off) ← readLE 4 buf off
    return (Float32.ofBits n.toUInt32, off)

instance : LeanMarshal Float where
  canonicalEncode v buf := pushLE 8 v.toBits buf
  canonicalDecode buf off := do
    let (n, off) ← readLE 8 buf off
    return (Float.ofBits n, off)

/-! ## Bool, Char -/

instance : LeanMarshal Bool where
  canonicalEncode v buf := buf.push (if v then 1 else 0)
  canonicalDecode buf off := do
    if off + 1 > buf.size then
      throw (LeanError.mk' LeanError.decodeError "bool: out of bounds")
    let b := buf.get! off
    if b > 1 then
      throw (LeanError.mk' LeanError.decodeError s!"bool: invalid byte 0x{b.toNat}")
    return (b == 1, off + 1)

instance : LeanMarshal Char where
  canonicalEncode v buf := pushLE 4 v.val.toUInt64 buf
  canonicalDecode buf off := do
    let (n, off) ← readLE 4 buf off
    let cp : UInt32 := n.toUInt32
    if h : cp.isValidChar then
      return (⟨cp, h⟩, off)
    else
      throw (LeanError.mk' LeanError.decodeError s!"char: invalid codepoint 0x{cp.toNat}")

/-! ## String -/

instance : LeanMarshal String where
  canonicalEncode s buf :=
    let bytes := s.toUTF8
    let buf := pushLE 4 bytes.size.toUInt64 buf
    buf ++ bytes
  canonicalDecode buf off := do
    let (len64, off) ← readLE 4 buf off
    let len := len64.toNat
    if off + len > buf.size then
      throw (LeanError.mk' LeanError.decodeError "string: payload truncated")
    let slice := buf.extract off (off + len)
    if let some s := String.fromUTF8? slice then
      return (s, off + len)
    else
      throw (LeanError.mk' LeanError.decodeError "string: invalid UTF-8")

/-! ## Arbitrary-precision integers — bignat, bigint -/

/-- Decompose a `Nat` into 64-bit limbs, least significant first.
The MSB limb is non-zero (unless `n == 0`, in which case the array is empty). -/
private partial def natToLimbs (n : Nat) : Array UInt64 :=
  if n == 0 then #[]
  else
    let limb : UInt64 := (n % (1 <<< 64)).toUInt64
    #[limb] ++ natToLimbs (n >>> 64)

/-- Inverse of `natToLimbs`. -/
private def limbsToNat (limbs : Array UInt64) : Nat := Id.run do
  let mut acc : Nat := 0
  for i in [0:limbs.size] do
    acc := acc ||| (limbs[i]!.toNat <<< (i * 64))
  return acc

instance : LeanMarshal Nat where
  canonicalEncode n buf :=
    let limbs := natToLimbs n
    let buf := pushLE 4 limbs.size.toUInt64 buf
    limbs.foldl (init := buf) fun acc limb => pushLE 8 limb acc
  canonicalDecode buf off := do
    let (lenU, off) ← readLE 4 buf off
    let len := lenU.toNat
    let mut limbs : Array UInt64 := Array.mkEmpty len
    let mut cur := off
    for _ in [0:len] do
      let (limb, off') ← readLE 8 buf cur
      limbs := limbs.push limb
      cur := off'
    return (limbsToNat limbs, cur)

instance : LeanMarshal Int where
  canonicalEncode n buf :=
    let sign : UInt8 := if n < 0 then 1 else 0
    let mag  : Nat   := n.natAbs
    let buf := buf.push sign
    -- magnitude shares bignat layout (u32 len + LE u64 limbs).
    LeanMarshal.canonicalEncode (T := Nat) mag buf
  canonicalDecode buf off := do
    if off + 1 > buf.size then
      throw (LeanError.mk' LeanError.decodeError "bigint: sign byte missing")
    let sign := buf.get! off
    if sign > 1 then
      throw (LeanError.mk' LeanError.decodeError s!"bigint: invalid sign byte 0x{sign.toNat}")
    let (mag, off) ← LeanMarshal.canonicalDecode (T := Nat) buf (off + 1)
    let value : Int := if sign == 1 then -(Int.ofNat mag) else Int.ofNat mag
    -- Reject "negative zero".
    if sign == 1 ∧ mag == 0 then
      throw (LeanError.mk' LeanError.decodeError "bigint: negative zero is illegal")
    return (value, off)

/-! ## Structural composites: option, list, tuple<α, β>, result<α, ε> -/

instance [LeanMarshal α] : LeanMarshal (Option α) where
  canonicalEncode o buf :=
    match o with
    | none   => buf.push 0
    | some x => LeanMarshal.canonicalEncode x (buf.push 1)
  canonicalDecode buf off := do
    if off + 1 > buf.size then
      throw (LeanError.mk' LeanError.decodeError "option: discriminator missing")
    let tag := buf.get! off
    match tag with
    | 0 => return (none, off + 1)
    | 1 =>
      let (x, off) ← LeanMarshal.canonicalDecode (T := α) buf (off + 1)
      return (some x, off)
    | _ => throw (LeanError.mk' LeanError.decodeError s!"option: invalid tag 0x{tag.toNat}")

instance [LeanMarshal α] : LeanMarshal (List α) where
  canonicalEncode xs buf :=
    let buf := pushLE 4 xs.length.toUInt64 buf
    xs.foldl (init := buf) fun acc x => LeanMarshal.canonicalEncode x acc
  canonicalDecode buf off := do
    let (lenU, off) ← readLE 4 buf off
    let len := lenU.toNat
    let mut acc : Array α := Array.mkEmpty len
    let mut cur := off
    for _ in [0:len] do
      let (x, off') ← LeanMarshal.canonicalDecode (T := α) buf cur
      acc := acc.push x
      cur := off'
    return (acc.toList, cur)

instance [LeanMarshal α] [LeanMarshal β] : LeanMarshal (α × β) where
  canonicalEncode p buf :=
    LeanMarshal.canonicalEncode p.2 (LeanMarshal.canonicalEncode p.1 buf)
  canonicalDecode buf off := do
    let (a, off) ← LeanMarshal.canonicalDecode (T := α) buf off
    let (b, off) ← LeanMarshal.canonicalDecode (T := β) buf off
    return ((a, b), off)

/-- `Except ε α` encodes per IDL `result<α, ε>` (SPEC/canonical-abi.md §6):
disc=0 → ok payload (α), disc=1 → err payload (ε). -/
instance [LeanMarshal α] [LeanMarshal ε] : LeanMarshal (Except ε α) where
  canonicalEncode e buf :=
    match e with
    | .ok x  => LeanMarshal.canonicalEncode x  (buf.push 0)
    | .error err => LeanMarshal.canonicalEncode err (buf.push 1)
  canonicalDecode buf off := do
    if off + 1 > buf.size then
      throw (LeanError.mk' LeanError.decodeError "result: discriminator missing")
    let tag := buf.get! off
    match tag with
    | 0 =>
      let (x, off) ← LeanMarshal.canonicalDecode (T := α) buf (off + 1)
      return (.ok x, off)
    | 1 =>
      let (err, off) ← LeanMarshal.canonicalDecode (T := ε) buf (off + 1)
      return (.error err, off)
    | _ => throw (LeanError.mk' LeanError.decodeError s!"result: invalid tag 0x{tag.toNat}")

end Leo4
