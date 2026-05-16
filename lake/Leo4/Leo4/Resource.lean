-- Leo4.Resource — `LeanResource` marker class + `@[leo4_resource]` attribute.
--
-- A `LeanResource` type is transported as an opaque `u64` handle across
-- the leo4 boundary (`SPEC/canonical-abi.md` §12). The type's own bytes
-- never cross; only the handle does. Lifetime is bound to a host-side
-- `LeanRef<'a, T>` (Rust) / `LeanRef α` (Lean) tied to the calling
-- `Arena`.
--
-- Mutually exclusive with `LeanMarshal` (`Leo4.Marshal`). The plugin
-- rejects any type that has instances of both.

import Lean

namespace Leo4

open Lean

/--
Marker class indicating that `T` is an opaque resource handle. No
methods — its presence is the only signal the plugin needs (combined
with the handle being a `u64` in `SPEC/canonical-abi.md` §12).
-/
class LeanResource (T : Type)

/--
`@[leo4_resource]` is the ergonomic shortcut for declaring that a Lean
type lives on the resource side of the boundary. Equivalent to a
hand-written `instance : LeanResource MyType := ⟨⟩`.

The plugin consumes this attribute in two stages:
1. While walking the user package's environment, it collects every
   `@[leo4_resource]`-tagged decl as a resource candidate.
2. It synthesises the matching `LeanResource T` instance entry into the
   admit-set for any `resource` constraint that mentions `T`.
-/
initialize leo4ResourceAttr : TagAttribute ←
  registerTagAttribute `leo4_resource
    "Mark a Lean type as a leo4 resource handle (opaque across the boundary)."

end Leo4
