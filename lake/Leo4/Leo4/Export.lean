-- Leo4.Export — registers the `@[leo4_export]` attribute and (Step 3) the
-- `@[leo4_specialize_when ?]` parametric attribute carrying a `leo4_constraint`
-- payload.
--
-- LEO4-DESIGN.md §4 (IDL) and CLAUDE.md "Lean: All @[leo4_export] attributes go
-- on top-level definitions only."

import Lean
import Leo4.Syntax  -- registers the `leo4_constraint` syntax category (D5)

namespace Leo4

open Lean

/--
`@[leo4_export]` marks a top-level definition for export across the leo4 boundary.

The Lake plugin (`Leo4Plugin.Main`) walks every imported module's serialised
entries for this attribute to discover the export set, then derives IDL,
mangling, and the C shim from each tagged definition's signature.

Validation that boundary signatures stay in `Type 0` and reject either
universe-polymorphic or dependent-type signatures (LEO4-DESIGN.md §4.3, D11,
D12) is the plugin's responsibility, not this attribute's, because at
attribute-application time we do not yet have the full instance environment
needed to give precise diagnostics. The attribute itself just records the tag.
-/
initialize leo4ExportAttr : TagAttribute ←
  registerTagAttribute `leo4_export
    "Mark a top-level definition for export across the leo4 boundary."

/--
Attribute syntax for `@[leo4_specialize_when <constraint>]`.

The `leo4_constraint` argument is parsed but **not elaborated** at attribute-
application time — the Lake plugin elaborates it later, when it walks the
imported environment and computes the admit-set per the (α′) algorithm
(LEO4-DESIGN.md §5).
-/
syntax (name := leo4_specialize_when) "leo4_specialize_when " leo4_constraint : attr

/--
`@[leo4_specialize_when <c>]` attaches a constraint sublanguage payload (D5)
to a `@[leo4_export]`-tagged generic definition. The plugin reads the stored
`Syntax`, evaluates it against the closed-world instance environment, and
materialises one specialisation per admit-set member.

The payload is kept as raw `Syntax` because the constraint language can refer
to constants (`marshal`, user typeclasses, `T : Cls`) that we cannot resolve
without an `Environment`, which we do not have at attribute-set time.

If a `@[leo4_export]` decl carries no `@[leo4_specialize_when]`, the plugin
falls back to reading instance-implicit binders (`[Cls T]`) from the
definition's type signature.
-/
initialize leo4SpecializeWhenAttr : ParametricAttribute Syntax ←
  registerParametricAttribute {
    name := `leo4_specialize_when
    descr := "Constraint sublanguage payload driving the (α′) admit-set enumeration."
    getParam := fun _decl stx => return stx
  }

end Leo4
