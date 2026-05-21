-- sibling/mathlib-bridge-test — Lake package that pulls Mathlib +
-- the main `Leo4` library so the `Leo4.MathlibBridge.*` modules can
-- be type-checked end-to-end. Lives in `sibling/` because Mathlib
-- pull / build is heavy (1-2h cold on a fresh machine) and we don't
-- want it on every `just test` invocation.
--
-- Run with `just mathlib-bridge-test` from the repo root. The
-- recipe drives `lake build MathlibBridgeTest`, which compiles every
-- `Leo4.MathlibBridge.*` module against actual Mathlib types.

import Lake
open Lake DSL

package «mathlib-bridge-test» where

-- leo4 runtime library — same path-dep shape as
-- `tests/sample-lean/lakefile.lean`. Brings in `Leo4`, `Leo4.Wide`,
-- `Leo4.MathlibSubset`, etc. without pulling in the Lake plugin.
require Leo4 from ".." / ".." / "lake" / "Leo4"

-- Mathlib — heavy dep. We pin to the same source git/branch the
-- user's `lean-toolchain` is happy with; Mathlib release branches
-- track Lean 4 stable closely so the toolchain in
-- `lean-toolchain` is the single source of truth.
require mathlib from git "https://github.com/leanprover-community/mathlib4.git"

@[default_target]
lean_lib MathlibBridgeTest where
  roots := #[`MathlibBridgeTest]
