# Spike 0 — Lake Plugin Hook Stability

> **Status**: not yet started.
> **Owner**: 병익.
> **Goal**: confirm that Lake exposes a stable enough hook for leo4-plugin to
> walk a fully elaborated environment and inspect `@[leo4_export]` definitions.
> **Time budget**: 3-5 working days. If this exceeds 5 days, leo4's
> architecture needs to be revisited before Week 1 starts.

## Why This Spike Exists

leo4's Lake plugin needs to do all of the following, in order, during a Lake
build:

1. Wait until the user's Lean package is **fully elaborated** (so type class
   instances are visible in the environment).
2. Walk the environment to find all definitions tagged `@[leo4_export]`.
3. For each, retrieve its associated `@[leo4_specialize_when]` constraint
   quotation.
4. Call `Lean.Meta.SynthInstance.getInstances` or equivalent to enumerate
   the admit-set per type parameter.
5. Emit IDL, mangling table, handshake, and the C shim source.
6. Drive `cc`/`leanc` to compile the shim and link it with Lean's runtime
   plus the user's compiled Lean code, producing a `.so`.

The question this spike answers: **can Lake's public API support steps 1-2
in a way that survives Lean toolchain updates without breakage every few
months?**

If the answer is no, leo4's plugin architecture has to change (likely to an
external post-build tool that consumes `.olean` files directly, which is
strictly worse).

## Specific Questions to Answer

| # | Question | Method |
|---|---|---|
| Q1 | Does Lake expose a target/facet that runs after all `.olean` files are produced, in the same process, with the elaborated environment in memory? | Read `Lake/Build/Module.lean`, try writing a minimal custom target. |
| Q2 | Can a custom Lake target invoke `Lean.Elab.Frontend` or `Lean.Meta` operations on the user package? | Try it in a toy plugin. |
| Q3 | If Q2 is no, can we re-import the user's `.olean` files and elaborate just enough to inspect attributes? | Try `Lean.Environment.importModules`. |
| Q4 | Is the API surface stable across at least the latest 3 Lean versions? | Diff `Lake/` between recent releases. |
| Q5 | What is the cost of running this hook? (admit-set enumeration is the suspect) | Microbench on a toy package with 5 generic exports. |

## Deliverables

1. **`spike/lake-hook/`** — a working toy Lake plugin that:
   - Defines a custom Lake target `leo4-extract`.
   - Walks the environment of a sample Lean package.
   - Prints all `@[leo4_export]` definitions and their argument types.
   - Calls `getInstances` on a known typeclass (e.g. `ToString`) and prints
     the resulting instance list.
2. **`spike/SPIKE-0-FINDINGS.md`** — a written report answering Q1-Q5 above.
3. A **go / no-go / pivot** recommendation for Week 1.

## Toy Lean Package for the Spike

```lean
-- spike/lake-hook/SampleLean/Sample.lean
import Lean

namespace Sample

-- We pretend `@[leo4_export]` exists by hijacking an existing attribute
-- or registering a real one in the plugin. Use whichever is easier.
@[leo4_export]
def add (a b : UInt64) : UInt64 := a + b

@[leo4_export]
def stringify {T : Type} [ToString T] (x : T) : String := toString x

end Sample
```

The plugin should be able to:

- Locate `Sample.add` and report `[UInt64, UInt64] → UInt64`.
- Locate `Sample.stringify` and report it as generic over `T` with
  constraint `T : ToString`.
- Enumerate the admit-set of `T : ToString` against the current
  environment.

## Stop Conditions

### Green light (proceed to Week 1 as planned)
- Q1 = yes (a clean facet or custom target works)
- Q2 = yes (or Q3 = yes with acceptable overhead)
- Q4 = yes for at least the most recent stable Lean
- Q5 < 1s for the toy package

### Yellow light (proceed but flag risk)
- Q1 partial (custom target works but requires undocumented hooks)
- Q3 yes with elevated overhead
- Q4 questionable but workable with a per-toolchain plugin variant

### Red light (revisit architecture before Week 1)
- Q1 = no AND Q3 = no
- Q4 = no (API churns every release)
- Q5 > 10s for the toy package (architectural problem)

A red light forces consideration of these alternatives:

- **Alternative A**: External tool reads `.olean` files directly. Slower
  development, but Lake-independent. Requires writing `.olean` reader.
- **Alternative B**: Lean side annotates exports manually in a sidecar
  `.json` file; plugin just reads that file. Loses constraint quotation
  power; falls back to a stringified DSL.
- **Alternative C**: Re-implement plugin as a `lean --run` script that
  produces all artifacts, called from `lakefile.lean` as a shell command.
  Loses some integration with Lake's incremental build.

## Findings Template

After running the spike, fill out `spike/SPIKE-0-FINDINGS.md` with:

```markdown
# Spike 0 Findings

**Date**: YYYY-MM-DD
**Lean toolchain tested**: leanprover/lean4:vX.Y.Z
**Recommendation**: GREEN / YELLOW / RED

## Q1: …
**Answer**: …
**Evidence**: …

## Q2: …
[etc.]

## Architectural impact

[If yellow or red, describe the impact.]

## Recommended path

[Concrete next step for Week 1.]
```

## Note

The spike does **not** need to produce production-quality code. It is
disposable. Its only deliverable that survives is the findings document.
Resist the urge to over-build it.
