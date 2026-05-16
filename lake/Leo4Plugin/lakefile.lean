-- Lake plugin for leo4.
-- Walks the user package's elaborated environment, extracts
-- @[leo4_export] definitions, computes admit-sets from
-- @[leo4_specialize_when] quotations, and emits IDL, mangling table,
-- handshake, and C shim source.
--
-- Plugin is driven by `lake exe leo4plugin <user-module>`, NOT by a Lake
-- internal facet hook. See spike/SPIKE-0-FINDINGS.md for rationale.

import Lake
open Lake DSL

package Leo4Plugin where

require Leo4 from ".." / "Leo4"

@[default_target]
lean_lib Leo4Plugin where
  roots := #[`Leo4Plugin]

@[default_target]
lean_exe leo4plugin where
  root := `Leo4Plugin.Main
  -- Required: the exe links Lean's elaborator (Lean.importModules + Meta.*).
  supportInterpreter := true
