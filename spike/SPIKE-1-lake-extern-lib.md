# Spike 1 — Lake `extern_lib` for declarative reverse-direction link

**Date**: 2026-05-23
**Lean toolchain**: leanprover/lean4:v4.29.1 (Lake 5.x)
**Question**: Can we drive the Phase 9 reverse-direction
`libleo4_rust_bridge.a` + `leo4_rust_bridge_lean.o` link
declaratively, so a user lakefile gets the bridge wired in
just by `require Leo4Rust from "…"`?
**Recommendation**: **GREEN, with a posix-spawn-for-cargo
caveat.** The mechanism exists; the safe minimal landing is
*path-resolution-only* `extern_lib`s in a new `Leo4Rust`
Lake package, with `cargo build` of the bridge / worker
remaining the user's responsibility (`just rust-bridge-build`
or the eventual `leo4` CLI). Auto-driving cargo from within
the extern_lib body is *possible* but pulls Lake into a
non-trivial Cargo dependency-tracking dance that v0 of this
spike does not bless.

---

## 1. What `extern_lib` actually is in Lake 5.x

The DSL command:

```lean
extern_lib <name> [pkg] := <body>
```

expands (per `/opt/lean4/src/lean/lake/Lake/DSL/Targets.lean`'s
`expandExternLibCommand`) into:

```lean
target <name>.static [pkg] : FilePath := <body>
family_def <name> : CustomOut (__name__, <name>) := ConfigTarget ExternLib.configKind
def <name> : ExternLibDecl := Lake.DSL.mkExternLibDecl __name__ <name> { getPath := cast (by simp) }
```

The `target` macro itself (`expandTargetCommand` in the same
file) requires the `<body>` to have type

```lean
NPackage pkgName → FetchM (Job FilePath)
```

— per `mkTargetDecl`'s signature. The `pkg` binder is
implicit when omitted from the DSL form. So in practice a
minimal `extern_lib` looks like:

```lean
extern_lib leo4RustBridge := do
  let p : System.FilePath := "/abs/path/to/libleo4_rust_bridge.a"
  return (Pure.pure p)
```

The inner `Pure.pure` is `Job`'s `Pure` instance
(`/opt/lean4/src/lean/lake/Lake/Build/Job/Basic.lean:140`):
`public instance : Pure Job := ⟨Job.pure⟩`. The outer monad
is `FetchM`, which has `MonadLift LogIO JobM` →
`MonadLift JobM FetchM`, so any `IO`/`LogIO` is reachable
via the lift chain.

## 2. How `lean_exe` discovers and links the archive

`/opt/lean4/src/lean/lake/Lake/Build/Executable.lean:49–53`:

```lean
let deps := (← (← self.pkg.transDeps.fetch).await).push self.pkg
for dep in deps do
  for lib in dep.externLibs do
    objJobs := objJobs.push <| ← lib.static.fetch
```

The exe build walks **transitive** package deps (including the
package that owns the `lean_exe`), enumerates each dep's
`externLibs`, fetches each `lib.static` job (which evaluates
the `extern_lib` body), and pushes the resulting FilePath
into the link line as a static object input. The exe's
`weakLinkArgs` + `linkArgs` are appended after.

Concretely: **a user package containing nothing but
`require Leo4Rust from "…"` and a `lean_exe MyApp` declaration
gets the bridge archive automatically.** No `weakLinkArgs`
edit; no `extraDepTargets` plumbing on the user side. This
is the property we need.

`ExternLib.recBuildStatic` (in `Lake/Build/ExternLib.lean`)
turns the user-provided body's `Job FilePath` into the
`static` facet — there's nothing additional to write past
the body.

## 3. Cross-package `pkg.dir` access

`NPackage pkgName` is the Lake structure carrying the
package's resolved root (`pkg.dir`). The leo4Rust extern_lib
body sees the `Leo4Rust` package's own dir, not the
consuming user package's dir. To resolve a path like
`../../target/release/libleo4_rust_bridge.a` the body uses
`pkg.dir / "../../target/release/libleo4_rust_bridge.a"` —
treating `lake/Leo4Rust/` as the anchor and going up two
levels to the leo4 repo root.

Lake exposes the `Workspace` and transitive package map via
`pkg.workspace`, so a fancier resolver could walk other
packages, but for our use case `pkg.dir` + relative paths
is enough.

## 4. The four shapes the `extern_lib` body can take

Ordered by progressively more work the body does:

### 4a. Pure path constant (env-driven discovery)

```lean
extern_lib leo4RustBridge := do
  let env ← (System.getEnv "LEO4_RUST_BRIDGE_AR" : IO _)
  let p : System.FilePath := match env with
    | some v => v
    | none   => pkg.dir / "../../target/release/libleo4_rust_bridge.a"
  if !(← p.pathExists) then
    error s!"leo4_rust_bridge.a not found at {p}; \
             run `cargo build --release -p leo4-rust-bridge` first"
  return (Pure.pure p)
```

Cargo build remains a user step. Lake just resolves the
already-built archive. **This is the safe v0 of the spike.**

### 4b. ar wrapping `.o` → `.a` (for the glue shim)

The Lean-side glue shim is a `.o`; Lake wants a `.a`. The
body compiles + wraps:

```lean
extern_lib leo4RustBridgeLean := do
  let workDir := pkg.buildDir / "leo4rust"
  IO.FS.createDirAll workDir
  let src := pkg.dir / "../../../shim/leo4_rust_bridge_lean.c"
  let outObj := workDir / "leo4_rust_bridge_lean.o"
  let outAr  := workDir / "libleo4_rust_bridge_lean.a"
  -- (a) leanc -c on the C source
  let r1 ← IO.Process.output {
    cmd  := "leanc",
    args := #["-c", "-std=c2x", src.toString, "-o", outObj.toString],
  }
  if r1.exitCode != 0 then error s!"leanc failed: {r1.stderr}"
  -- (b) ar rcs to wrap .o into .a
  let r2 ← IO.Process.output {
    cmd  := "ar",
    args := #["rcs", outAr.toString, outObj.toString],
  }
  if r2.exitCode != 0 then error s!"ar failed: {r2.stderr}"
  return (Pure.pure outAr)
```

This is the second extern_lib that lakefile.lean would
ship — same package `Leo4Rust`, alongside `leo4RustBridge`.

### 4c. ar wrapping, with `buildFileUnlessUpToDate` cache

Lake's `Job.mapM` + `buildFileUnlessUpToDate'` together
give the body cache-aware rebuilds. The body wraps the IO
in a Job that registers traces, so Lake skips the leanc /
ar invocation when the C source hasn't changed. Pattern in
`Lake/Build/ExternLib.lean:36` is the model.

### 4d. Auto-driving cargo from inside the body

```lean
extern_lib leo4RustBridge := do
  -- Trigger cargo build of the upstream crate.
  let r ← IO.Process.output {
    cmd  := "cargo",
    args := #["build", "--release", "-p", "leo4-rust-bridge",
              "--manifest-path", (pkg.dir / "../../Cargo.toml").toString],
  }
  if r.exitCode != 0 then error s!"cargo build leo4-rust-bridge failed: {r.stderr}"
  -- Locate the produced .a.
  return (Pure.pure (pkg.dir / "../../target/release/libleo4_rust_bridge.a"))
```

Cleanest from the user's POV (`lake build` does everything),
but couples Lake's incremental cache to Cargo's. Cargo
*does* track changed Rust sources, so a second `cargo build`
is fast, but it's still an unconditional invocation per
`lake build` — not how Lake usually wants its targets to
behave. Tracking individual `*.rs` files as inputs to the
Job's trace gets unwieldy fast (the Rust source tree is
large and Cargo's effective input set is non-trivial).

## 5. The D8 build-order question

`LEO4-DESIGN.md` D8 pins **Lake-first, Cargo-second**. Reverse
direction inverts that — the cdylib must exist (cargo) before
emit / Lake-link can run. We've already lived with this
inversion in Phase 9 (the manual workflow steps cargo before
lake). The extern_lib design doesn't change the order; it
just makes the *consuming side*'s Lake-link declarative.

Recommendation: **leave D8 unchanged** but record an explicit
exception in `SPEC/reverse-direction.md` §7 — the reverse
pipeline has its own ordering (cargo → emit → lake) and the
`extern_lib`s in `Leo4Rust` assume cargo has already
produced the inputs. If the inputs are missing, `lake build`
fails with a clear "run cargo build first" error — same
contract as the forward-direction `LEO4_SHIM_SO` resolution.

## 6. Risk inventory

| Risk | Severity | Mitigation |
|---|---|---|
| Body's IO action fails silently (Cargo not run yet) | medium | Fail explicitly with a "run cargo build first" message + pointer to `just rust-bridge-build`. |
| Path resolution differs per build profile (debug vs release) | medium | Search both `target/release/` and `target/debug/`; env override (`LEO4_RUST_BRIDGE_AR`) wins. |
| Lake's incremental cache and Cargo's diverge | medium-high (only if we auto-drive cargo) | Skip cargo auto-drive in v0 of the spike. Users invoke cargo. |
| `pkg.dir / "../../target/..."` brittle across `require` strategies | low | The leo4 repo's Cargo target is at a fixed location relative to `lake/Leo4Rust/`; document the assumption. Users with non-default layouts use the env override. |
| ar/leanc not on PATH | low | The harness binary worker already requires leanc; failure mode is identical to forward-direction shim builds. |
| Cross-platform `ar` differences (BSD vs GNU) | low | `ar rcs` flags work on every supported tier (POSIX + gnullvm-clang ar). Windows uses LLVM ar via gnullvm bundle. |
| Race when two `lake build`s of different consumers re-run the leanc compile in parallel | low | Each consumer has its own `pkg.buildDir`; outputs land separately. |

## 7. Recommended v0 landing plan

**Goal**: a user writes

```lean
require Leo4Rust from "<abs leo4 path>/lake/Leo4Rust"

@[default_target]
lean_exe myApp where
  root := `Main
```

and `lake build` produces an executable with
`libleo4_rust_bridge.a` + glue-shim `.a` automatically
linked.

**Three commits, in order**:

1. **`Leo4Rust` package skeleton** — new
   `lake/Leo4Rust/{lakefile.lean,lean-toolchain}`. Just the
   `package` + `require Leo4`. No extern_lib yet. Confirms
   `cd lake/Leo4Rust && lake build` runs clean.

2. **`leo4RustBridge` extern_lib** — pattern 4a (path
   resolution only). User runs `just rust-bridge-build`
   before `lake build`. The body fails-fast with a clear
   message if the archive is absent.

3. **`leo4RustBridgeLean` extern_lib** — pattern 4b (leanc
   + ar). Body invokes leanc -c -std=c2x on
   `shim/leo4_rust_bridge_lean.c`, then ar rcs into a `.a`
   inside `pkg.buildDir`.

After all three:

- `examples/05-rust-export/lean/lakefile.lean` adds
  `require Leo4Rust from "../../../lake/Leo4Rust"` and drops
  its manual `leanc -o` invocation (the `just
  rust-export-05-build` recipe's step 4 collapses).
- README + AGENTS.md update to reflect the simplification.

Pattern 4c (cache-aware Job mapM) is a later optimisation.
Pattern 4d (auto-drive cargo) stays out of v0.

## 8. Open question for 병익 before commit

- **Should `Leo4Rust` be a separate Lake package, or
  merged into `Leo4`?** Separate is cleaner — forward-only
  users don't need the cargo-built bridge archive, and a
  separate package keeps the failure surface local to
  reverse-direction consumers. But it adds one more
  `require` line to user lakefiles. The audit-ledger
  recommendation: **separate**. Forward-only users matter.

- **Should the body's IO error path emit a structured
  `LEO4_ERR_RUST_*` code, or just `IO.userError`?**
  `extern_lib` body errors propagate as Lake build
  failures — the user sees them in the lake error output.
  Plain `IO.userError s!"…"` is sufficient; the rest of the
  pipeline's error codes are for runtime, not build time.

## 9. What this spike did NOT verify

- **`buildFileUnlessUpToDate'` trace integration**: Pattern
  4c's incremental cache. Works in `recBuildStatic` for
  Lake's builtin facets, but tying our body's traces into
  it correctly needs a small standalone harness build
  before landing.
- **Windows path quoting**: `\\` vs `/` in `pkg.dir / "../.."`
  resolution. Lake's `FilePath` arithmetic handles this on
  every platform per `System.FilePath`, but our specific
  pattern needs a Tier 2 verification.
- **`require Leo4Rust from "<abs path>"` with a Lake reservoir
  in between**: When the user is in a *very* deep monorepo,
  the relative `pkg.dir / "../.."` may not point at the leo4
  repo root. Env override is the fallback.

## 10. Conclusion

The mechanism works. The minimal safe landing is patterns
**4a + 4b** in a separate `Leo4Rust` Lake package, with cargo
build of the bridge / worker remaining a user-invoked step
(or wrapped by the `leo4` CLI, future). After landing, the
4-step manual workflow from `SPEC/reverse-direction.md` §7
collapses to:

```
cargo build  &&  leo4-rust-emit  &&  lake build
```

— three commands, no manual `leanc -o`. The remaining
`leo4-rust-emit` step is the obvious next target for further
automation (a `lean_exe` extra dep target that drives it
from a built cdylib path), but that's a separate spike.
