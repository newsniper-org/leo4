-- leo4 — Lean 4 ↔ Rust interop, runtime library
--
-- This module is the root of the Leo4 library. Sub-modules:
--   • Leo4.Syntax    — leo4_constraint syntax category
--   • Leo4.Export    — @[leo4_export] attribute
--   • Leo4.Marshal   — `LeanMarshal` typeclass + `LeanError`
--   • Leo4.Resource  — `LeanResource` marker + @[leo4_resource]
--   • Leo4.Builtins  — `LeanMarshal` instances for built-in primitives

import Leo4.Syntax
import Leo4.Export
import Leo4.Marshal
import Leo4.Resource
import Leo4.Builtins
