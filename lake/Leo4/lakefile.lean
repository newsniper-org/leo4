-- Lean runtime library for leo4.
-- Provides the @[leo4_export] attribute, the constraint syntax category,
-- and the Marshal type class.

import Lake
open Lake DSL

package Leo4 where

@[default_target]
lean_lib Leo4 where
  roots := #[`Leo4]
