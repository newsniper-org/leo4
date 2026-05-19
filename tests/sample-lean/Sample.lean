-- Smoke-test "user package" for `lake exe leo4plugin Sample`.
-- Mirrors the toy module from spike/SPIKE-0-lake-hook.md.

import Leo4

open Leo4   -- expose @[leo4_export], `deriving LeanMarshal`, etc., short-form.

namespace Sample

@[leo4_export]
def add (a b : UInt64) : UInt64 := a + b

@[leo4_export]
def stringify {T : Type} [ToString T] (x : T) : String := toString x

@[leo4_export]
def hello : String := "hello, leo4"

@[leo4_export]
def listLen {T : Type} (xs : List T) : Nat := xs.length

-- A use of the constraint sublanguage (Step 3 — registered as a
-- ParametricAttribute Syntax; the plugin does not yet elaborate it).
@[leo4_export, leo4_specialize_when scalar ∧ ord]
def maxScalar {T : Type} [Ord T] (a b : T) : T :=
  match compare a b with
  | .lt => b
  | _   => a

-- Phantom-generic smoke fixture: `T` is declared but never used in the
-- signature, so the plugin should emit a single instantiation with
-- `generic_args = [null]`. The underscore prefix silences Lean's
-- unused-variable linter without altering the plugin's behaviour
-- (the user name is informational only — phantom detection is FVarId-based).
@[leo4_export]
def constantFortyTwo {_T : Type} (_x : UInt32) : UInt32 := 42

-- W3-3 fixtures: deriving LeanMarshal smoke checks.

-- Single-ctor (record) shape.
structure Point where
  x : Float
  y : Float
  deriving LeanMarshal

-- All-nullary inductive → IDL enum.
inductive Color where
  | red
  | green
  | blue
  deriving LeanMarshal

-- Mixed inductive with payload → IDL variant.
-- Self-recursive: `node` references `Self` (i.e. `Tree`).
inductive Tree where
  | leaf
  | node : Tree → Tree → Tree
  deriving LeanMarshal

-- W3-4 boundary exports that take user-defined types.
-- The plugin should mangle these symbols using `S_Sample_Point_s` /
-- `V_Sample_Tree_v` / `E_Sample_Color_e` per SPEC/mangling.md §2.

@[leo4_export]
def pointSum (p : Point) : Float := p.x + p.y

@[leo4_export]
def isLeaf (t : Tree) : Bool :=
  match t with
  | .leaf => true
  | _    => false

@[leo4_export]
def colorName (c : Color) : String :=
  match c with
  | .red   => "red"
  | .green => "green"
  | .blue  => "blue"

-- W3-5: resource fixture. `@[leo4_resource]` marks `ParserHandle` as
-- opaque to the boundary; only a `u64` handle crosses the wire.
@[leo4_resource]
structure ParserHandle where
  raw : UInt64

@[leo4_export]
def parserId (h : ParserHandle) : ParserHandle := h

-- W7-2c-ii fixtures: option / result wire formats.

@[leo4_export]
def safeDiv (a b : UInt64) : Option UInt64 :=
  if b == 0 then none else some (a / b)

@[leo4_export]
def parseU64 (s : String) : Except String UInt64 :=
  match s.toNat? with
  | some n => .ok n.toUInt64
  | none   => .error s!"bad number: {s}"

-- W7-2c-iii fixtures: list / tuple wire formats.

@[leo4_export]
def listSumU64 (xs : List UInt64) : UInt64 :=
  xs.foldl (· + ·) 0

@[leo4_export]
def listConcat (xs : List String) : String :=
  xs.foldl (· ++ ·) ""

@[leo4_export]
def pairAdd (p : UInt64 × UInt32) : UInt64 :=
  p.1 + p.2.toUInt64

end Sample
