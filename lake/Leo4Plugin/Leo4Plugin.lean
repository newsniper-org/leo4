-- leo4 Lake plugin entry point.
--
-- See LEO4-DESIGN.md §7 (build orchestration) and SPEC/handshake.md.
-- Phase 0 (spike): determine whether the hook we need into Lake's build
-- pipeline is stable enough. See spike/SPIKE-0-lake-hook.md.

import Leo4Plugin.AdmitSet
import Leo4Plugin.Mangling
import Leo4Plugin.Emit
import Leo4Plugin.Main
