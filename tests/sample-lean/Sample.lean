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

-- P5-e fixture: List + Nat + List return. Exercises `list<u32>` on
-- both the argument and the return side, plus `bignat` as a runtime
-- `n : Nat` value parameter (the SPEC's value-param erasure rule
-- only kicks in for `{N : Nat}`-style implicit binders; here `n` is
-- an ordinary explicit parameter, transmitted on the wire as bignat).
@[leo4_export]
def echoes (xs : List UInt32) (n : Nat) : List UInt32 :=
  (List.replicate n xs).foldl (· ++ ·) []

-- Generic user nominal types (re-enabled 2026-05-20 after the A/A+/A++
-- chain landed: deriving generic-inductive, walkUserDecl generic-aware,
-- idlToLeanType nominal application, ASCII-positional binder
-- normaliser, render generic_params header, and the resolver
-- TypeVar shape).
--
-- `Pair` exercises the record substitution path end-to-end (record
-- handler builds field handlers via Subst.substIDL). `Either`
-- exercises the variant-decl emission with generic params; its
-- monomorphic export remains stubbed at the shim entry point because
-- variant `non-Self` payloads are gated by #12 (W7-2d-iii) —
-- record wire-up vs. variant wire-up split is the intended demo
-- of where the cascade currently stops.

structure Pair (α : Type) (β : Type) where
  fst : α
  snd : β
  deriving LeanMarshal

@[leo4_export]
def pairFstU64U32 (p : Pair UInt64 UInt32) : UInt64 := p.fst

@[leo4_export]
def pairSndU64U32 (p : Pair UInt64 UInt32) : UInt32 := p.snd

inductive Either (α : Type) (β : Type) where
  | left  : α → Either α β
  | right : β → Either α β
  deriving LeanMarshal

@[leo4_export]
def eitherTaggedU64String (e : Either UInt64 String) : Bool :=
  match e with
  | .left  _ => false
  | .right _ => true

-- Phase 6 fixture: mutual recursion between two nominal types
-- (`SPEC/phase-6-mutual.md` §1). Lean's `mutual ... end` block ties
-- Expr / Stmt into one `iv.all` group; the plugin emits one
-- `mutual { variant Sample.Expr { … }; variant Sample.Stmt { … }; }`
-- block where peer references use `Cyc<i>` instead of FQN, and the
-- deriving handler emits one `mutual { partial def … }` block so
-- cross-decl marshalling routes through direct function references
-- rather than typeclass-instance forward references.
mutual
  inductive Expr where
    | lit  (n : UInt64)
    | seq  (s : Stmt)
    deriving LeanMarshal
  inductive Stmt where
    | nop
    | block (e : Expr)
    deriving LeanMarshal
end

@[leo4_export]
def exprIsLit (e : Expr) : Bool :=
  match e with
  | .lit _ => true
  | .seq _ => false

@[leo4_export]
def stmtIsNop (s : Stmt) : Bool :=
  match s with
  | .nop     => true
  | .block _ => false

-- Value-param erasure fixture (SPEC/mangling.md §"Value-param erasure").
-- `{_N : Nat}` is a phantom implicit value-typed binder; the plugin
-- omits it from the boundary signature so the wrapper exports
-- `(x : UInt32) -> UInt32`. Lean fills the implicit at the wrapper's
-- call site via instance synthesis on `Inhabited Nat` (default = 0).
@[leo4_export]
def doubleVal {_N : Nat} (x : UInt32) : UInt32 := 2 * x

-- ROADMAP Phase 8: `LeanMarshal Rat` lives in
-- `lake/Leo4/Leo4/MathlibSubset.lean`; step 2 wires the boundary
-- via `UserDecl.externalMarshal` + Lean-side
-- `@[export leo4_marshal_Rat_dec/_enc]` helpers in the
-- `<pkg>.leo4-exports.lean` file. End-to-end:
@[leo4_export]
def addRat (a b : Rat) : Rat := a + b


end Sample
