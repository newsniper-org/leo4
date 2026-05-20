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

open Lean
open Leo4Plugin (IDLType)

/-! ## Type encoding (SPEC/mangling.md §2) -/

private def joinUnderscore (xs : Array String) : String :=
  String.intercalate "_" xs.toList

/-- FQN → mangling-safe segment: dot- and dash-separated module path
joined by `_` (SPEC/mangling.md §2 "Fully-qualified names"). -/
def fqnSeg (fqn : String) : String :=
  (fqn.replace "." "_").replace "-" "_"

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
  | .record fqn args =>
      if args.isEmpty then "S_" ++ fqnSeg fqn ++ "_s"
      else "S_" ++ fqnSeg fqn ++ "_" ++ joinUnderscore (args.map mangleType) ++ "_s"
  | .variant fqn args =>
      if args.isEmpty then "V_" ++ fqnSeg fqn ++ "_v"
      else "V_" ++ fqnSeg fqn ++ "_" ++ joinUnderscore (args.map mangleType) ++ "_v"
  | .enumT fqn  => "E_" ++ fqnSeg fqn ++ "_e"
  | .flagsT fqn => "F_" ++ fqnSeg fqn ++ "_f"
  | .resource fqn args =>
      if args.isEmpty then "X_" ++ fqnSeg fqn ++ "_x"
      else "X_" ++ fqnSeg fqn ++ "_" ++ joinUnderscore (args.map mangleType) ++ "_x"
  | .io t       => "I_" ++ mangleType t ++ "_i"
  | .self       => "self"
  | .selfApp args => "self_" ++ joinUnderscore (args.map mangleType) ++ "_x"
  -- Phase 6 cycle-breaker (SPEC/phase-6-mutual.md §2): `Cyc<i>` →
  -- `c<i>c`, where `<i>` is ASCII-decimal with no leading zeros.
  -- Mirrors `crates/schema-idl/src/mangle.rs`'s `Cyc(i)` arm
  -- byte-for-byte.
  | .cyc i      => "c" ++ toString i ++ "c"

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

/-! ## IDL → Lean type-expression mapping -/

/--
Render an `IDLType` as the source-level Lean type that the canonical
ABI maps it to.  Used by `Leo4Plugin.Main` when emitting the Lean
wrapper file (`<pkg>.leo4-exports.lean`) — every `@[leo4_export]`
monomorphisation gets a thin wrapper whose signature is built from
this function, and whose body forwards to the user's original decl.

Inverse of `Leo4Plugin.AdmitSet.leanNameToIDL` for the cases the
plugin currently rounds-trips.  Composite cases produce parenthesised
forms so the result can be slotted into a binder position without
further wrapping. -/
partial def idlToLeanType : IDLType → String
  | .u8  => "UInt8"  | .u16 => "UInt16" | .u32 => "UInt32" | .u64 => "UInt64"
  | .i8  => "Int8"   | .i16 => "Int16"  | .i32 => "Int32"  | .i64 => "Int64"
  | .f32 => "Float32"
  | .f64 => "Float"
  | .bool   => "Bool"
  | .char   => "Char"
  | .string => "String"
  | .bigint => "Int"
  | .bignat => "Nat"
  | .list t          => "(List " ++ idlToLeanType t ++ ")"
  | .option t        => "(Option " ++ idlToLeanType t ++ ")"
  | .result t none   => "(Except Empty " ++ idlToLeanType t ++ ")"
  | .result t (some e) =>
      "(Except " ++ idlToLeanType e ++ " " ++ idlToLeanType t ++ ")"
  | .tuple ts =>
      "(" ++ String.intercalate " × " (ts.toList.map idlToLeanType) ++ ")"
  | .record fqn args | .variant fqn args | .resource fqn args =>
    if args.isEmpty then fqn
    else
      "(" ++ fqn ++ " " ++
        String.intercalate " " (args.toList.map idlToLeanType) ++ ")"
  | .enumT fqn      | .flagsT fqn   => fqn
  | .io t           => "(IO " ++ idlToLeanType t ++ ")"
  | .self           => "_"      -- only reachable inside a nominal body
  | .selfApp _      => "_"
  -- `Cyc<i>` mirrors `Self` / `Self<…>` for mutual groups: the Lean
  -- wrapper template only cares about the variable's *type identity*
  -- and the deriving handler closes the cycle through its own
  -- `mutual ... end` block, so a wildcard suffices here.
  | .cyc _          => "_"

/-! ## Function-name mangling (SPEC/mangling.md §1) -/

/-- Per SPEC/mangling.md §1: `pkg` colons (`:`) and dashes (`-`) are
both replaced with `_`. Dashes are normalised so kebab-case package
names (e.g. `leo4-sample`) become valid linker symbols and valid Lean
identifiers — Lean's `@[export ident]` attribute parses an unquoted
identifier and rejects dashes. -/
def normalizePackageSegment (pkg : String) : String :=
  (pkg.replace ":" "_").replace "-" "_"

/-- Lean's C-identifier mangling rule for module names (empirically
determined from the names of `initialize_*` symbols `lean -c` emits):
alphanumerics are preserved, `_` doubles to `__`, and every other
character is encoded as `_x<lowercase-2-digit-hex-codepoint>`. So:

    "leo4_sample.leo4-exports"
       → "leo4" "__" "sample" "_x2e" "leo4" "_x2d" "exports"
       = "leo4__sample_x2eleo4_x2dexports"

Used by the plugin to compute the wrapper module's `initialize_*`
symbol name so the Rust loader can dlsym it deterministically. -/
def manglerLeanModuleName (s : String) : String :=
  s.foldl (init := "") fun acc c =>
    if c.isAlpha || c.isDigit then acc.push c
    else if c == '_' then acc ++ "__"
    else
      let n := c.toNat
      let hi := n / 16
      let lo := n % 16
      let hex (d : Nat) : Char :=
        if d < 10 then Char.ofNat (d + 0x30)        -- '0' + d
        else Char.ofNat (d - 10 + 0x61)              -- 'a' + (d-10)
      acc ++ "_x" ++ String.singleton (hex hi) ++ String.singleton (hex lo)

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

/-- IDL textual form of an `IDLType` (human-readable, matches
`SPEC/idl-grammar.ebnf`). Distinct from `mangleType`, which produces the
linker-symbol form. -/
partial def idlForm : IDLType → String
  | .u8 => "u8" | .u16 => "u16" | .u32 => "u32" | .u64 => "u64"
  | .i8 => "i8" | .i16 => "i16" | .i32 => "i32" | .i64 => "i64"
  | .f32 => "f32" | .f64 => "f64"
  | .bool => "bool" | .char => "char" | .string => "string"
  | .bigint => "bigint" | .bignat => "bignat"
  | .list t          => "list<" ++ idlForm t ++ ">"
  | .option t        => "option<" ++ idlForm t ++ ">"
  | .result t none   => "result<" ++ idlForm t ++ ">"
  | .result t (some e) => "result<" ++ idlForm t ++ ", " ++ idlForm e ++ ">"
  | .tuple ts        => "tuple<" ++ String.intercalate ", " (ts.toList.map idlForm) ++ ">"
  | .record fqn args =>
      if args.isEmpty then fqn
      else fqn ++ "<" ++ String.intercalate ", " (args.toList.map idlForm) ++ ">"
  | .variant fqn args =>
      if args.isEmpty then fqn
      else fqn ++ "<" ++ String.intercalate ", " (args.toList.map idlForm) ++ ">"
  | .enumT fqn       => fqn
  | .flagsT fqn      => fqn
  | .resource fqn args =>
      if args.isEmpty then fqn
      else fqn ++ "<" ++ String.intercalate ", " (args.toList.map idlForm) ++ ">"
  -- Phase 7 (D-i 2026-05-19): lift `io<T>` → `future<T>` in the
  -- canonical IDL surface. The wire mangle (`I_T_i`) is unchanged
  -- for byte-identical cross-impl conformance.
  | .io t            => "future<" ++ idlForm t ++ ">"
  | .self            => "Self"
  | .selfApp args    => "Self<" ++ String.intercalate ", " (args.toList.map idlForm) ++ ">"
  | .cyc i           => "Cyc<" ++ toString i ++ ">"

/-- Render the `<T0, T1>` generic-params suffix that immediately follows
the fqn in a nominal declaration. Empty when there are no generics. -/
private def genericHeader (generics : Array Name) : String :=
  if generics.isEmpty then ""
  else
    let names := generics.map (·.toString)
    "<" ++ String.intercalate ", " names.toList ++ ">"

/-- Render one `UserDecl` as a single line of IDL text. Field/case order is
the order in the `UserDecl` (which the walker preserves — declaration
order, per SPEC/canonical-abi.md §8). -/
partial def userDeclToIDL : UserDecl → String
  | .record fqn generics fields =>
      let fieldStrs := fields.map fun (n, t) => s!"{n.toString}: {idlForm t}"
      "record " ++ fqn ++ genericHeader generics ++ " { "
        ++ String.intercalate ", " fieldStrs.toList ++ " }"
  | .enumT fqn cases =>
      let caseStrs := cases.map (·.toString)
      "enum " ++ fqn ++ " { " ++ String.intercalate ", " caseStrs.toList ++ " }"
  | .variant fqn generics cases =>
      let caseStrs := cases.map fun (cn, payload) =>
        if payload.isEmpty then cn.toString
        else cn.toString ++ "(" ++ String.intercalate ", " (payload.toList.map idlForm) ++ ")"
      "variant " ++ fqn ++ genericHeader generics ++ " { "
        ++ String.intercalate ", " caseStrs.toList ++ " }"
  | .resource fqn generics =>
      "resource " ++ fqn ++ genericHeader generics
  | .externalMarshal fqn generics =>
      -- Phase 8 step 2: opaque-marshal nominal. The wire format is
      -- whatever the user's custom `LeanMarshal` impl produces;
      -- the IDL declares it so cross-impl renderers know the FQN
      -- is a valid nominal head.
      "external " ++ fqn ++ genericHeader generics
  | .mutual members =>
      -- Each inner nominal_decl carries its own terminating `;`
      -- (matching the grammar's `nominal_decl = … , ";" ;`); the
      -- outer `renderCanonical` then adds the `mutual_decl`'s own
      -- `;` on the closing `}`. SPEC/phase-6-mutual.md §1.
      let memberStrs := members.toList.map (fun m => userDeclToIDL m ++ ";")
      "mutual { " ++ String.intercalate " " memberStrs ++ " }"

/-- Sort key tag for SPEC/handshake.md `<pkg>.leo4-schema` ordering:
type decls first, then resources, then functions; lex by FQN within each band.
A `mutual` cluster lands in the value band (0); its source order is preserved
because `Cyc<i>` indices into the group are position-sensitive. -/
private def declBand : UserDecl → Nat
  | .record _ _ _  => 0
  | .enumT _ _     => 0
  | .variant _ _ _ => 0
  | .resource _ _  => 1
  | .mutual _      => 0
  | .externalMarshal _ _ => 0

/--
Render the canonical IDL form for the discovered export set + user-type
declarations.

* `pretty := true`  → newline-decorated form for `<pkg>.leo4-schema`.
* `pretty := false` → fully-collapsed form (the schema-hash input).

The two forms are byte-identical after the whitespace-collapsing step
described in `SPEC/handshake.md`; that is the contract between the
on-disk schema file and the handshake's schema hash.

`SPEC/handshake.md` ordering: type decls first (lex by FQN), then
resources (lex by FQN), then functions (lex by fname).
-/
def renderCanonical
    (pkg : String) (iface : String)
    (userDecls : Array UserDecl)
    (members : Array (String × Array IDLType × IDLType))
    (pretty : Bool := false) : String := Id.run do
  -- Sort user decls: band first, then FQN.
  let sortedDecls := userDecls.qsort fun a b =>
    if declBand a == declBand b then a.fqn < b.fqn
    else declBand a < declBand b
  let sortedFuncs := members.qsort fun (a, _, _) (b, _, _) => a < b
  let nl    : String := if pretty then "\n" else " "
  let ind   : String := if pretty then "  " else ""
  let mut s : String := "package " ++ pkg ++ ";" ++ nl
  s := s ++ "interface " ++ iface ++ " {" ++ nl
  for d in sortedDecls do
    s := s ++ ind ++ userDeclToIDL d ++ ";" ++ nl
  for (fname, params, ret) in sortedFuncs do
    s := s ++ ind ++ "func " ++ fname ++ "("
    for i in [0 : params.size] do
      if i > 0 then s := s ++ ", "
      s := s ++ "_" ++ toString i ++ ": " ++ idlForm params[i]!
    s := s ++ ") -> " ++ idlForm ret ++ ";" ++ nl
  s := s ++ "}"
  return if pretty then s else collapseWhitespace s

/-- Hash the canonical (fully collapsed) form via FNV-1a-64. -/
def schemaHashOf (canonical : String) : Hash :=
  Hash.ofString canonical

end Leo4Plugin
