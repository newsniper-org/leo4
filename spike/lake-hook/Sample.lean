-- Toy "user package" consumed by `leo4-spike-plugin`.
-- Mirrors the sketch in `spike/SPIKE-0-lake-hook.md` (the Sample section).

import Leo4Export

namespace Sample

@[leo4_export]
def add (a b : UInt64) : UInt64 := a + b

@[leo4_export]
def stringify {T : Type} [ToString T] (x : T) : String := toString x

@[leo4_export]
def hello : String := "hello, leo4"

@[leo4_export]
def listLen {T : Type} (xs : List T) : Nat := xs.length

end Sample
