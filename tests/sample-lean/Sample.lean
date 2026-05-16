-- Smoke-test "user package" for `lake exe leo4plugin Sample`.
-- Mirrors the toy module from spike/SPIKE-0-lake-hook.md.

import Leo4

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

end Sample
