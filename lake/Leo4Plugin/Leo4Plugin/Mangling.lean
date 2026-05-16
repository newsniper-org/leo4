-- Leo4Plugin.Mangling — name mangling and schema hashing.
--
-- Implements `SPEC/mangling.md` §§1-3 on the Lean side. The Rust side
-- (`leo4-idl`) MUST agree byte-for-byte; see `tests/mangling/` (Phase 3+).
--
-- The schema hash is FNV-1a-64 (LEO4-DESIGN.md §6, SPEC/mangling.md §3) —
-- a change detector for ABI invalidation at link time, not a cryptographic
-- primitive. Both sides must implement the algorithm identically.

import Leo4Plugin.AdmitSet

namespace Leo4Plugin

open Leo4Plugin (IDLType)

/-! ## Type encoding (SPEC/mangling.md §2) -/

private def joinUnderscore (xs : Array String) : String :=
  String.intercalate "_" xs.toList

partial def mangleType : IDLType → String
  | .u8  => "u8"  | .u16 => "u16" | .u32 => "u32" | .u64 => "u64"
  | .i8  => "i8"  | .i16 => "i16" | .i32 => "i32" | .i64 => "i64"
  | .f32 => "f32" | .f64 => "f64"
  | .bool   => "b"
  | .char   => "c"
  | .string => "str"
  | .bigint => "bI"
  | .bignat => "bN"
  | .list t          => "L_" ++ mangleType t ++ "_l"
  | .option t        => "O_" ++ mangleType t ++ "_o"
  | .result t none   => "Rz_" ++ mangleType t ++ "__z"
  | .result t (some e) => "Rz_" ++ mangleType t ++ "_" ++ mangleType e ++ "_z"
  | .tuple ts =>
      "T_" ++ joinUnderscore (ts.map mangleType) ++ "_t"
  | .record n args =>
      if args.isEmpty then "S_" ++ n ++ "_s"
      else "S_" ++ n ++ "_" ++ joinUnderscore (args.map mangleType) ++ "_s"
  | .variant n args =>
      if args.isEmpty then "V_" ++ n ++ "_v"
      else "V_" ++ n ++ "_" ++ joinUnderscore (args.map mangleType) ++ "_v"
  | .enumT n    => "E_" ++ n ++ "_e"
  | .flagsT n   => "F_" ++ n ++ "_f"
  | .resource n => "X_" ++ n ++ "_x"
  | .io t       => "I_" ++ mangleType t ++ "_i"

/-! ## Schema hash (SPEC/mangling.md §3) -/

/--
An 8-byte schema digest, stored big-endian inside a `UInt64`. Rendered to
either 13-char base32lc or 16-char hex on demand.
-/
structure Hash where
  /-- Big-endian view: byte 0 = `(value >>> 56) &&& 0xff`. -/
  value : UInt64
  deriving Repr, Inhabited, BEq

namespace Hash

/-- FNV-1a 64-bit hash of `bs`. Offset basis `0xcbf29ce484222325`,
prime `0x100000001b3`, `UInt64` wraparound on multiply. -/
def fnv1a64 (bs : ByteArray) : Hash := Id.run do
  let mut h : UInt64 := 0xcbf29ce484222325
  for i in [0 : bs.size] do
    h := (h ^^^ (bs.get! i).toUInt64) * 0x100000001b3
  return { value := h }

/-- FNV-1a-64 over the UTF-8 encoding of `s`. -/
def ofString (s : String) : Hash :=
  fnv1a64 s.toUTF8

/-- 16-char lowercase hex of the 8 hash bytes, big-endian. -/
def toHex (h : Hash) : String := Id.run do
  let hex : Array Char := #[
    '0','1','2','3','4','5','6','7','8','9','a','b','c','d','e','f'
  ]
  let mut out : String := ""
  for i in [0:8] do
    let s : UInt64 := UInt64.ofNat (8 * (7 - i))
    let byte : Nat := ((h.value >>> s).toNat) &&& 0xff
    out := out.push hex[byte / 16]!
    out := out.push hex[byte % 16]!
  return out

/-- 13-char lowercase base32 (RFC 4648 alphabet, no padding) of the 8 hash
bytes, big-endian-packed. -/
def toBase32lc (h : Hash) : String := Id.run do
  let alph : Array Char := #[
    'a','b','c','d','e','f','g','h','i','j','k','l','m',
    'n','o','p','q','r','s','t','u','v','w','x','y','z',
    '2','3','4','5','6','7'
  ]
  let mut out : String := ""
  for i in [0:12] do
    let s : UInt64 := UInt64.ofNat (59 - 5 * i)
    let idx : Nat := ((h.value >>> s).toNat) &&& 0x1F
    out := out.push alph[idx]!
  -- Final char: the 4 LSB bits of the 64-bit packed value, left-aligned in 5.
  let last4 : Nat := (h.value.toNat) &&& 0x0F
  out := out.push alph[last4 * 2]!
  return out

end Hash

/-! ## Function-name mangling (SPEC/mangling.md §1) -/

/-- Per SPEC: `pkg` colons (`:`) are replaced with `_`. -/
def normalizePackageSegment (pkg : String) : String :=
  pkg.replace ":" "_"

/-- Full mangled name. See `SPEC/mangling.md` §1. -/
def mangle
    (pkg : String) (iface : String) (fname : String)
    (args : Array IDLType) (schemaHash : Hash) : String :=
  let pkgSeg  := normalizePackageSegment pkg
  let argsSeg := joinUnderscore (args.map mangleType)
  "leo4__" ++ pkgSeg ++ "__" ++ iface ++ "__" ++ fname
    ++ "__" ++ argsSeg
    ++ "__h" ++ schemaHash.toBase32lc

/-! ## IDL normalisation (SPEC/mangling.md §3) -/

/-- Collapse runs of ASCII whitespace to single spaces and trim leading/trailing. -/
def collapseWhitespace (s : String) : String := Id.run do
  let mut out : String := ""
  let mut sawSpace : Bool := true   -- treat leading whitespace as collapsed
  for c in s.toList do
    if c.isWhitespace then
      unless sawSpace do
        out := out.push ' '
        sawSpace := true
    else
      out := out.push c
      sawSpace := false
  -- `out` may still have one trailing space if the input ended with whitespace.
  if out.endsWith " " then out := (out.dropEnd 1).copy
  return out

/--
Render a synthetic *canonical IDL form* for a discovered export set. This is
intentionally a subset of the full grammar (`SPEC/idl-grammar.ebnf`): we
emit only what the Week 2 plugin currently understands — `package`,
`interface`, and `func` declarations. The hash computed over this string is
stable across plugin runs given the same exports.

When the IDL emitter (`<pkg>.leo4-schema`) lands in Phase 3, that file's
normalised form should produce the same hash.
-/
def renderCanonical
    (pkg : String) (iface : String)
    (members : Array (String × Array IDLType × IDLType)) : String := Id.run do
  -- Sort members by function name (SPEC/mangling.md §3 step 6).
  let sorted := members.qsort fun (a, _, _) (b, _, _) => a < b
  let mut s : String := "package " ++ pkg ++ "; interface " ++ iface ++ " { "
  for (fname, params, ret) in sorted do
    s := s ++ "func " ++ fname ++ "("
    for i in [0 : params.size] do
      if i > 0 then s := s ++ ", "
      s := s ++ "_" ++ toString i ++ ": " ++ mangleType params[i]!
    s := s ++ ") -> " ++ mangleType ret ++ "; "
  s := s ++ "}"
  return collapseWhitespace s

/-- Hash the canonical form via FNV-1a-64 (SPEC/mangling.md §3). -/
def schemaHashOf (canonical : String) : Hash :=
  Hash.ofString canonical

end Leo4Plugin
