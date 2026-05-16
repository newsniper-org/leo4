-- Spike-only stand-in for `lake/Leo4/Export.lean`.
-- The production module will live there and may carry additional payload
-- (e.g. a `leo4_specialize_when` ParametricAttribute).

import Lean

namespace Leo4

open Lean

initialize leo4ExportAttr : TagAttribute ←
  registerTagAttribute `leo4_export
    "Mark a definition as exported across the leo4 boundary."

end Leo4
