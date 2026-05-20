-- Leo4.MathlibSubset — `LeanMarshal` instances for the named subset of
-- Mathlib-compatible types (ROADMAP Phase 8). Each instance lands in
-- this file as it's needed; the file imports only what each instance
-- requires so `Leo4` itself doesn't pull Mathlib transitively.
--
-- Convention per ROADMAP Phase 8: the *type* may be Mathlib (or Lean
-- core); the marshal contract lives here, on the leo4 side. User
-- packages opt in by importing `Leo4.MathlibSubset` (already brought
-- in by the top-level `Leo4` import) and writing their own
-- `@[leo4_export]` definitions over these types.
--
-- 2026-05-20 entries:
--   • `Rat` (Lean core `Init.Data.Rat.Basic`) — marshalled as
--     bigint × bignat (num / den). Decode uses `mkRat` so the proof
--     obligations (`den ≠ 0`, `gcd = 1`) are reconstructed by the
--     smart constructor; the wire never carries the proof terms.

import Leo4.Marshal
import Leo4.Builtins

namespace Leo4

/-- `Rat` (Lean core) — wire format: `bigint num` followed by
`bignat den`. SPEC/canonical-abi.md §§5-6. The smart constructor
`mkRat num den` normalises the resulting rational on decode, which
includes the `den = 0 ⇒ 0/1` rule the Lean core API specifies, so
malformed wire payloads degrade to `0` rather than panicking. -/
instance : LeanMarshal Rat where
  canonicalEncode r buf :=
    let buf := LeanMarshal.canonicalEncode (T := Int) r.num buf
    LeanMarshal.canonicalEncode (T := Nat) r.den buf
  canonicalDecode buf off := do
    let (num, off) ← LeanMarshal.canonicalDecode (T := Int) buf off
    let (den, off) ← LeanMarshal.canonicalDecode (T := Nat) buf off
    return (mkRat num den, off)

end Leo4
