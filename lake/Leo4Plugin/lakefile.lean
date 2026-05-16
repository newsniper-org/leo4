-- Lake plugin for leo4.
-- Walks the user package's elaborated environment, extracts
-- @[leo4_export] definitions, computes admit-sets from
-- @[leo4_specialize_when] quotations, and emits IDL, mangling table,
-- handshake, and C shim source.

import Lake
open Lake DSL

package Leo4Plugin where

@[default_target]
lean_lib Leo4Plugin where
  roots := #[`Leo4Plugin]
