-- leo4 — Lean 4 ↔ Rust interop, runtime library
--
-- This module is the root of the Leo4 library. Sub-modules:
--   • Leo4.Syntax         — leo4_constraint syntax category
--   • Leo4.Export         — @[leo4_export] attribute
--   • Leo4.Marshal        — `LeanMarshal` typeclass + `LeanError`
--   • Leo4.Resource       — `LeanResource` marker + @[leo4_resource]
--   • Leo4.Builtins       — `LeanMarshal` instances for built-in primitives
--   • Leo4.Build          — user-facing `Build.lean` helpers (M + L surfaces)
--
-- Opt-in modules (not auto-imported by `import Leo4`):
--   • Leo4.MathlibSubset  — Phase 8 marshal contracts for named
--                            Mathlib-compatible types (`Rat`, …). User
--                            packages `import Leo4.MathlibSubset`
--                            explicitly when they need these instances.
--                            Kept opt-in so the auto-discovered class
--                            admit-sets (`@[leo4_export] def f {T}
--                            [ToString T] ...`) don't pull these types
--                            in by accident before the shim emitter
--                            can route their wire format (Phase 8 step 2).

import Leo4.Syntax
import Leo4.Export
import Leo4.Marshal
import Leo4.Resource
import Leo4.Builtins
import Leo4.Deriving
import Leo4.Build
