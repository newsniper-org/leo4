-- Leo4.Syntax — declares the `leo4_constraint` syntax category (D5).
--
-- The constraint sublanguage mirrors LEO4-DESIGN.md §4.2 and
-- SPEC/idl-grammar.ebnf's `constraint_expr` / `constraint_atom` productions,
-- but is written in Lean source as a payload to `@[leo4_specialize_when …]`.
--
-- This file registers the category and a Week-1 vocabulary. The plugin
-- consumes the resulting `Syntax` tree; it does *not* elaborate it as a
-- Lean term — the category is a quotation, not a typechecking site.

import Lean

namespace Leo4

open Lean

declare_syntax_cat leo4_constraint

-- Closed-set named constraints (LEO4-DESIGN.md §4.2 / SPEC `constraint_atom`).
-- Note `scalar` has a closed admit-set; the others are open type classes whose
-- admit-set is closed by the dependency graph (LEO4-DESIGN.md §5).
syntax "scalar"   : leo4_constraint
syntax "ord"      : leo4_constraint
syntax "eq"       : leo4_constraint
syntax "hash"     : leo4_constraint
syntax "pod"      : leo4_constraint
syntax "marshal"  : leo4_constraint
syntax "resource" : leo4_constraint

-- Type-class membership: `T : Cls`.
syntax (name := leo4_constraint_member) ident " : " ident : leo4_constraint

-- Decidable equality: `T = U`.
syntax (name := leo4_constraint_eq) ident " = " ident : leo4_constraint

-- Boolean connectives. Precedences chosen to match the usual reading
-- ¬ binds tightest, then ∧, then ∨ — i.e. `a ∨ b ∧ ¬ c` parses as `a ∨ (b ∧ (¬ c))`.
syntax:30 leo4_constraint " ∨ " leo4_constraint:31 : leo4_constraint
syntax:35 leo4_constraint " ∧ " leo4_constraint:36 : leo4_constraint
syntax:40 "¬ " leo4_constraint:41 : leo4_constraint

-- Grouping.
syntax "(" leo4_constraint ")" : leo4_constraint

end Leo4
