# Spike 0 Findings

**Date**: 2026-05-16
**Lean toolchain tested**: leanprover/lean4:v4.29.1 (Lake 5.0.0-src+f72c35b)
**Recommendation**: **GREEN, with architectural commitment** — proceed to Week 1 along the *re-import* path (a `lean_exe` plugin that calls `Lean.importModules` on the user package), **not** along an in-process Lake-internal hook.

The toy plugin lives at `spike/lake-hook/`. The relevant numbers:

```
target module: Sample
importModules (loadExts=true): 477–615 ms (3 runs: 615, 510, 477 ms)
env: 2050 imported modules, 189005 constants
@[leo4_export] decls: 4   (Sample.add, Sample.stringify, Sample.hello, Sample.listLen)
admit-set proxy: 150 ToString instances enumerated in <1 ms
total wall (warm)            : 685–840 ms
```

All four toy exports are correctly recovered, their binder kinds correctly classified (the `[inst] inst._… : ToString T` binder on `Sample.stringify` is the kind of constraint the (α′) algorithm needs).

---

## Q1: Does Lake expose a target/facet that runs after all `.olean` files are produced, in the same process, with the elaborated environment in memory?

**Answer**: **No — not in the strict sense the question asks**. Lake builds `.olean` files in *subprocesses* (`/opt/lean4/bin/lean` per module); the elaborated `Environment` of the user package is **never in memory inside the Lake driver process**. There is no public facet that surfaces it.

What Lake *does* offer is:
- Public DSL macros to declare `package`, `lean_lib`, `lean_exe`, scripts, and custom `target`s (`Lake/DSL/Targets.lean`, 235 lines, **identical signatures across v4.27.0 → v4.29.1**).
- A documented `lake exe <target>` invocation that runs after Lake has built all of `<target>`'s declared dependencies.
- The `extraDepTargets`/`needs` field on `lean_lib`/`lean_exe` to widen the set of pre-built dependencies.

The hook this spike was originally written to investigate — `Lake.Module.recBuildLean` — is **`private` in all three Lean versions checked** (v4.27.0, v4.28.0, v4.29.1; file: `src/lake/Lake/Build/Module.lean`). Reaching into it would mean copying the function body and re-registering a facet on top, which is exactly the kind of brittle hooking we want to avoid.

**Evidence**:
- `grep -n "private def Module.recBuildLean" /tmp/lean-history/{v4.27.0,v4.28.0,v4.29.1}/Module.lean` — all three matches start with `private`.
- The body of `Lake/Build/Module.lean` accumulated 143 diff-lines between v4.27.0 and v4.29.1, but every change is below the private boundary.
- The public DSL surface (`Lake/DSL/Targets.lean`) only changed by 2 diff-lines across the same span.

**What we use instead**: a `lean_exe` target with `supportInterpreter := true`. Lake builds its declared library dependencies first, then runs the exe. Inside the exe we set the search path (`Lean.initSearchPath (← Lean.findSysroot)`) and call `Lean.importModules`. This is the same pattern the Lean compiler itself uses in `lean --run` and in tooling like `doc-gen4`.

This is essentially **Alternative C** from `spike/SPIKE-0-lake-hook.md`, but invoked via `lake exe` rather than a shell-out from `lakefile.lean`. We get full Lake dependency tracking on the *inputs* (user `.lean` files) for free; we trade away Lake incremental tracking of the *plugin output artifacts*, which we will need to rebuild in Week 1 anyway via `cargo:rerun-if-changed=` (see "Architectural impact" below).

---

## Q2: Can a custom Lake target invoke `Lean.Elab.Frontend` or `Lean.Meta` operations on the user package?

**Answer**: **Yes**, but only by going through `Lean.importModules` from inside a `lean_exe` (or equivalent `lean --run` script). The Lake driver process *itself* cannot, because it has no elaborated `Environment` of the user package — those live in the per-module `lean` subprocesses, which exit before Lake's own DSL runs.

This is technically the Q3 path, but it works cleanly enough that the practical answer to Q2 is "yes via Q3". See evidence under Q3.

---

## Q3: If Q2 is no, can we re-import the user's `.olean` files and elaborate just enough to inspect attributes?

**Answer**: **Yes**, robustly, with all the necessary downstream APIs working.

**Evidence** — the spike's `SpikePlugin.lean`:

```lean
Lean.initSearchPath (← Lean.findSysroot)
let env ← Lean.importModules
  (imports := #[{ module := target }, { module := `Leo4Export }])
  (opts := {}) (trustLevel := 0) (loadExts := true)
```

After that one call, the rest of the spike just works:

- **Attribute reads**: `Leo4.leo4ExportAttr.hasTag env n` correctly returns `true` for the 4 tagged decls in `Sample`. This works *without* `loadExts := true` (TagAttribute reads from serialized per-module data via `getModuleEntries`), but we keep `loadExts := true` because…

- **Instance enumeration**: `Meta.instanceExtension.getState env |>.instanceNames` correctly contains 150 `ToString` instances from the imported environment. This *requires* `loadExts := true`, because `instanceExtension` is a `SimpleScopedEnvExtension` whose state is reconstructed by `addImportedFn` during `finalizeImport`, which only runs when `loadExts := true`.

- **MetaM operations on imported decls**: `Meta.ppExpr`, `Meta.forallTelescopeReducing`, `Meta.inferType` all work after wrapping the work in `CoreM.toIO'` with `{ env := env }` as the initial state. The spike does exactly this to extract binder kinds (`isInstImplicit`, `isImplicit`, `isStrictImplicit`, explicit) per export.

**Operationally important detail**: `lake exe` does *not* push sibling-library `.olean` directories onto `LEAN_PATH` for the exe target by default. Either invoke via `lake env lake exe leo4-spike-plugin` (which sets `LEAN_PATH=<pkg>/.lake/build/lib/lean:/opt/lean4/lib/lean`), or call `Lean.initSearchPath` explicitly inside the exe (we do both, belt-and-braces). Without this, `importModules` fails with `unknown module prefix 'Sample'`.

---

## Q4: Is the API surface stable across at least the latest 3 Lean versions?

**Answer**: **Yes for the APIs we touch**, with a hard "stay-on-public-surface" rule. Diffs across v4.27.0 → v4.28.0 → v4.29.1:

| Surface | File | Diff (v4.27→v4.29.1) | Notes |
|---|---|---|---|
| `Lean.TagAttribute`, `registerTagAttribute`, `hasTag` | `src/Lean/Attributes.lean` | 1 diff-line (within a `ParametricAttribute` internal refactor — does **not** touch `TagAttribute`) | stable |
| `Lean.importModules` signature | `src/Lean/Environment.lean` | identical signature | stable |
| `Lean.Meta.instanceExtension`, `Instances.instanceNames`, `isInstance`, `isInstanceCore`, `getGlobalInstancesIndex` | `src/Lean/Meta/Instances.lean` | 77 diff-lines across v4.27→v4.29.1, **none touching our surface** | stable |
| `Lake.DSL.Targets` (`package`, `lean_lib`, `lean_exe`) | `src/lake/Lake/DSL/Targets.lean` | 2 diff-lines | stable |
| `Lake.Module.recBuildLean` (was: candidate hook) | `src/lake/Lake/Build/Module.lean` | 143 diff-lines, function remains **`private`** all 3 versions | unsafe to depend on |

**Rule we adopt**: the plugin uses **only** the public Lean and Lake surfaces listed in the first four rows. We do not touch `private` Lake internals; we do not import any file inside `Lake/Build/`. This rule is what turns the 143-line `Module.lean` diff from a yellow flag into a no-op for us.

**Evidence**:
- `gh api repos/leanprover/lean4/contents/<path>?ref=<tag>` to fetch each file at each tag.
- Diffs in `/tmp/lean-history/` (not committed; reproducible from the same `gh api` calls).
- `grep "^def importModules " /tmp/lean-history/*/Environment.lean` confirms identical default-arg lists.

### Addendum (2026-05-16, from CI matrix run W7-0a)

The Phase 5 prep matrix (`ci/matrix.sh`) added `v4.30.0-rc2` to the
anchor set and found one *runtime* drift that did not show up in the
static diff above: **`importModules (loadExts := true)` now requires
`Lean.enableInitializersExecution` to have been called first**. The
function itself is defined in `Lean.ImportingFlag` and was present in
every version we sampled (`v4.27.0`, `v4.28.0`, `v4.29.1`, `v4.30.0-rc2`),
but only `v4.30` enforces the call order; earlier versions silently
allowed the call to be skipped.

Fix in the plugin: one line, `unsafe Lean.enableInitializersExecution`,
just before `Lean.importModules`. Safe in every matrix version (no-op
when called too early). The matrix now reports all 4 versions green,
which is the proof that this drift cost us one CI cycle and zero days
of debugging — exactly the value the matrix was added to deliver.

The takeaway: *static API-shape diffing is a necessary but not
sufficient stability signal*. Runtime preconditions (initialization
order, side-effect ordering, default-flag changes) only surface in an
actual cross-version `just test` run. Keep the matrix in CI from
Phase 5 onward.

---

## Q5: What is the cost of running this hook?

**Answer**: **~700–840 ms warm-cache total**, dominated by `importModules`, well below the 1-second green-light threshold for a toy package and well below the 10-second red-light threshold.

| Step | Cost (warm; 3-run min/max) |
|---|---|
| `Lean.importModules` (loadExts=true, 2050 transitive modules) | 477–615 ms |
| env walk (`SMap.fold` over 189k constants for `hasTag`) | ~170 ms |
| `ToString` admit-set enumeration (150 instances) | <1 ms |
| **total wall** | **685–840 ms** |

**Cost breakdown caveats**:
- The 615 ms `importModules` cost is dominated by loading the *Lean platform* (Init + Std + Lean's own modules). A "toy package" in the spike still has 2050 imports because `Leo4Export` transitively imports `Lean`. A *truly* minimal user package (one that only depends on `Leo4`, not `Lean` directly) might be cheaper, but for any realistic Lean library this is the floor.
- The admit-set enumeration is sub-millisecond because `instanceExtension` already pre-indexes instances by class (via `DiscrTree`). For real (α′) work we will not be enumerating `ToString` — we will be enumerating user-defined `LeanScalar` / `LeanMarshal` instances, which are smaller sets. Cost should remain sub-millisecond.
- The env walk is the *naive* implementation; a production plugin should read `leo4ExportAttr.ext.getModuleEntries env modIdx` per imported module instead, which scales with `|tagged decls|`, not `|env.constants|`. Optimization left for Week 1; current cost is already acceptable.

---

## Architectural impact

### What the spike commits us to

1. **Plugin invocation = `lake exe leo4plugin <user-pkg>`**, not a Lake facet/target hook. The plugin builds the user library as a Lake dependency, then re-imports its `.olean` files via `Lean.importModules (loadExts := true)`.

2. **No private Lake surfaces.** The plugin lives inside the public Lean/Lake API. CLAUDE.md's call-out that `Lake.Module.recBuildLean` hook stability was "the subject of spike 0" resolves to: **we do not hook `recBuildLean` at all; we side-step it.**

3. **`cargo:rerun-if-changed=` granularity** (LEO4-DESIGN.md §12, open question) gains a concrete answer: the Rust side watches the Lake outputs — `.leo4-schema`, `.leo4-handshake`, `.leo4-mangling`, `.leo4-shim.so` — not the user's `.lean` files. Lake handles the `.lean → .olean → plugin output` chain via its own dependency tracking on the `lean_lib` it consumes.

4. **The plugin must call `Lean.initSearchPath`** itself at startup. Lake's `lake env` does set `LEAN_PATH`, but we can't assume the user invokes us through `lake env`, and `importModules` consults the in-process `searchPathRef`, not `LEAN_PATH` directly.

5. **`loadExts := true` is mandatory** for our use of `instanceExtension`. This is a non-default flag; documenting it in `lake/Leo4Plugin/Main.lean` is required so a future contributor doesn't drop it.

### What it does *not* commit us to (still open)

- Lake output location (`target/leo4/` vs `.lake/build/`). The spike uses `.lake/build/lib/lean/` because Lake puts `.olean` there by default; the plugin's *generated* artifacts can go wherever (`LEO4-DESIGN.md §7` shows `target/leo4/<pkg>.*`). Pin in Week 1.
- Whether the plugin is one `lean_exe` per workspace or one per user package. Spike used a single workspace; can revisit.
- How to plumb the user's `lakefile.lean` so that adding `require Leo4Plugin from <git>` is enough to wire up the build. Spike's lakefile is monolithic; real consumption ergonomics are a Week 1 deliverable.

### What changes if v4.30 breaks something

The risk surfaces, ranked by likelihood:
1. `Meta.instanceExtension` internal restructure (the file has been gaining lines steadily: 339 → 337 → 371 across v4.27 → v4.28 → v4.29.1). Mitigation: keep instance enumeration localised to one Lean file with a clear `getInstancesOf : Environment → Name → Array Name` interface. If it breaks, change one file.
2. `Lake.DSL.Targets` macro shape (low risk; very small diff so far). Mitigation: same.
3. `importModules` adding a non-default-valued parameter (low risk; v4.27→v4.29.1 had no such change). Mitigation: pin Lean version via `lean-toolchain` (D10 — already committed in this session: root `lean-toolchain` is now `leanprover/lean4:v4.29.1`).

---

## Recommended path for Week 1

1. **Promote the spike's `SpikePlugin.lean` into `lake/Leo4Plugin/Main.lean`** (the existing placeholder). The 100-line spike covers steps 1–4 of the plugin's responsibility list (CLAUDE.md, "How to Work With the Lake Plugin" section). Steps 5–6 (IDL emit, `cc`/`leanc` driving) come next.
2. **Define the real `@[leo4_export]` attribute in `lake/Leo4/Export.lean`** (the spike's `Leo4Export.lean` is the template — `registerTagAttribute` is the right primitive for v0).
3. **Add `@[leo4_specialize_when ?]` as a `ParametricAttribute Syntax`** in the same file. The spike skipped this because the toy `[ToString T]` constraint was expressible in the function signature; for the real constraint sublanguage (`LEO4-DESIGN.md §4.2`) we need the parametric attribute. The v4.28→v4.29.1 internal refactor of `ParametricAttribute`'s `exportEntriesFnEx` does not affect the registration API; safe to use.
4. **Skip the `recBuildLean` hook entirely.** The original open question is now closed: we drive the plugin from `lake exe`, not from inside Lake's incremental build graph. Treat `LEO4-DESIGN.md §12`'s "open question" item #1 as resolved.
5. **Optimise env walk early** — read `leo4ExportAttr.ext.getModuleEntries env modIdx` instead of folding all 189k constants. The spike's naive walk costs ~170 ms; the indexed walk should be a few hundred microseconds.
6. **Write a regression test** that pins the cost ceiling. The spike measured 685–840 ms wall on this hardware; a Week-3 CI test that asserts `<2 s` on the toy package would catch a 4× regression early.

## Note on the original spike doc

`spike/SPIKE-0-lake-hook.md` Q1 was framed around "same process, with the elaborated environment in memory". The strict-literal answer is "no" (Lake's driver process has no env), but this turns out **not to matter** — the re-import path is fast enough, public-API-only, and stable across the last three Lean releases. The findings reframe Q1 from "is `recBuildLean` stable enough?" to "do we need to hook `recBuildLean` at all?" — answer: no.
