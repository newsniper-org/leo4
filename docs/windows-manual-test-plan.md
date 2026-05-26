# Cross-platform Manual Test Plan — C1 (Windows) + C5 (musl) prelim

> Drafted 2026-05-24. Companion to `OS-PORTABILITY.md`.
> Captures the pre-flight audit + actionable test matrix
> for two v1.0 RC blockers:
>
> - **C1 (Windows Tier 2)**: VirtualBox + Windows 11 Pro
>   manual verification pass that precedes Tier 2 CI.
> - **C5 (musl Tier 1+, no-mslean4-no-lake paths only)**:
>   Alpine container or static-binary smoke test for the
>   rust-transpile / scaffold-only / pure-Rust crate
>   surface. (Android = C6, deferred to v1.x.)

## 0. Goal

Verify leo4's runtime behaviour on Windows 11 Pro under
VirtualBox before sinking effort into CI yaml. Manual
testing surfaces:

- Runtime bugs the compile-only sanity check
  (`cargo check`) can't catch.
- Path / extension / IPC misalignments not covered by
  `OS-PORTABILITY.md` §3 audit ledger.
- VirtualBox-specific quirks (clipboard, shared
  folders, virtio-net) that future CI runners may also
  hit.

## 1. Pre-flight audit — gaps found that should be
   considered before testing

Cross-checked against `OS-PORTABILITY.md` §3 + fresh
grep of the codebase. The §3 ledger covers all
build-time / lake-plugin / shim concerns. Additional
items worth knowing before manual testing:

### 1a. `leo4-rust-worker` Windows IPC — known half-done

- **Location**: `crates/leo4-rust-worker/src/main.rs:336`
- **State**: Phase 9-4c landed the shim's C-side Windows
  backend (`CreateProcessA` + `CreateNamedPipeA`), but
  the Rust worker's `open_ipc_channel` Windows branch is
  still a stub returning
  `"Windows named-pipe IPC not yet implemented"`.
- **Impact**: **reverse direction** (Rust cdylib → Lean
  caller) cannot run on Windows yet. Forward direction
  (Lean caller → Rust callee) is unaffected — it doesn't
  spawn the worker.
- **Action**: skip reverse-direction tests until this
  lands, OR mark this as the first issue to fix from
  manual testing.

### 1b. Internal shim naming uses `.so` on all platforms

- **Location**: `lake/Leo4/Leo4/Build.lean:120`
  (`<outName>.so`), `crates/leo4-build/src/lib.rs:79`
  (`{stem}.leo4-shim.so`).
- **State**: leo4-internal convention — the file is named
  `<pkg>.leo4-shim.so` regardless of host OS. On Windows
  the file format is still a PE DLL (gnullvm-clang
  produces it), just with a `.so` suffix.
- **Impact**: `libloading::Library::new(path)` doesn't
  inspect the extension (it hands the path to
  `LoadLibraryW`); the file loads correctly. **No runtime
  break expected.** But Windows users see `.so` files
  which may confuse them.
- **Action**: leave as-is for the manual pass; revisit if
  any tool / Windows Defender / user-facing diagnostic
  surfaces the suffix mismatch.

### 1c. `lean-toolchain` Lean version pin (`v4.29.1`)

- **Location**: `crates/leo4-cli/src/main.rs:657, 679,
  999, 1012` — written into scaffolded `lean/lean-toolchain`
- **State**: `elan` on Windows downloads the same toolchain
  archive. Should work in principle.
- **Action**: verify the toolchain installs on Windows;
  watch for any v4.29.1-specific Windows regression in
  the Lean source.

### 1d. `cargo build` may need explicit target on
   `*-pc-windows-gnullvm`

- **State**: native MSVC toolchain is the Rust default on
  Windows; leo4 specifically targets `gnullvm` per
  `OS-PORTABILITY.md` §1 (clang + lld + UCRT). The
  manual test should pin this via either `rustup target
  add x86_64-pc-windows-gnullvm` + `--target` flag, or
  `rust-toolchain.toml`.
- **Action**: add `x86_64-pc-windows-gnullvm` target on
  the Windows VM before running `cargo build`.

### 1e. Existing OS-PORTABILITY.md §3 entries

Three open audit items (medium priority):
1. `.so` extension hardcoded in `Build.lean:227` —
   library naming layer needed.
2. `-Wl,-rpath` in `Build.lean:233, 262` — library
   search path layer needed.
3. `-shared` linker flag in `Build.lean:259` — same
   layer.

These should fail loudly on Windows during the lake
plugin's shim-link step. **They are the most likely
single point of mslean4-path failure on Windows.**

## 2. Pre-flight `#[cfg(...)]` ledger

Confirmed via grep (`crates/` + `sibling/`):

| File | Branch | Status |
|---|---|---|
| `crates/leo4-rust-worker/src/main.rs:78, 84, 314, 330` | `#[cfg(unix)]` / `#[cfg(windows)]` | unix: full; windows: stub (1a) |
| `crates/leo4-cli/src/main.rs:1556-1559` | `bin_name` adds `.exe` on Windows | adequate |
| `shim/leo4_rust_bridge.c:39, 55, 67, 81, 211, 439, 642, 1040` | `#if defined(_WIN32)` blocks | Phase 9-4c landed; runtime unverified |

No ad-hoc `#[cfg(target_os = ...)]` outside `OS-PORTABILITY.md`
identified layers — the policy is being followed.

## 3. Test matrix (priority order)

Run on a fresh Windows 11 Pro VM with:
- VirtualBox Guest Additions
- `rustup-init.exe` → install `stable-x86_64-pc-windows-gnu`
- `rustup target add x86_64-pc-windows-gnullvm` (if not default)
- `elan` for Windows → `elan toolchain install leanprover/lean4:v4.29.1`
- `git for Windows`
- LLVM/clang (for gnullvm target's C toolchain): MSYS2 mingw-w64-clang or
  standalone LLVM installer
- The leo4 repo: `git clone` to `C:\leo4` (path with spaces
  not recommended for first pass)

**Shell choice — run from MSYS2 ucrt64**: `cargo
install --path crates\leo4-cli` produces a `leo4.exe`
that dynamically links against the mingw runtime DLLs
(`libwinpthread-1.dll`, `libgcc_s_seh-1.dll`, …)
when the rustup default toolchain is one of the
`*-gnu` / `*-gnullvm` flavors. PowerShell's default
`PATH` does NOT include the MSYS2 / mingw bin
directory that holds those DLLs, so invoking
`leo4 ...` from PowerShell fails with exit code
`-1073741515` (`0xC0000135`, `STATUS_DLL_NOT_FOUND`)
before printing anything. The MSYS2 ucrt64 shell
includes `C:\msys64\ucrt64\bin` in its `PATH` by
default, so all `leo4 …` commands here are written
assuming that shell. Alternative: prepend
`C:\msys64\ucrt64\bin` to PowerShell's PATH, or
switch the rustup default to
`stable-x86_64-pc-windows-msvc` and rebuild
(`cargo install --path crates\leo4-cli --force`) for
a self-contained binary.

### T1 — Pure Rust compile-only sanity (forward path crates)

The `examples/0{1,2,4,5}-*` crates' `build.rs` calls
`leo4-build::wire(...)` and canonicalizes the
`tests/sample-lean/.lake/build/leo4` directory. That
directory only exists after `lake build` has run on the
sample fixture — under Linux the path is normally warm
via `just smoke-plugin`; on a fresh Windows VM it does
NOT exist yet, so a bare `cargo build --workspace`
panics in the examples' build scripts. Exclude them
from T1 (they get their own end-to-end test under T8):

```powershell
cd C:\leo4
cargo build --workspace `
    --exclude leo4-example-01-hello `
    --exclude leo4-example-02-roundtrip `
    --exclude leo4-example-04-mutual-ast `
    --exclude leo4-example-05-rust-export `
    --target x86_64-pc-windows-gnullvm
```

(In an MSYS2 ucrt64 bash, swap PowerShell's `` ` `` line
continuations for `\`.)

**Expect**: clean. **Verified 2026-05-25 on a fresh
Windows 11 Pro + MSYS2 ucrt64 VM**: pass. If this fails,
file the diagnostic and stop — everything downstream
depends on it.

### T2 — Pure Rust test suite (no Lean toolchain needed)

Same `--exclude` set as T1 (examples have the
sample-lean lake artifact precondition). Add
`--no-fail-fast` so any single fail doesn't abort the
whole matrix, and `--skip parses_lake_plugin_sample_schema`
because `leo4-idl::tests::round_trip_sample` reads the
same precondition fixture.

```powershell
cd C:\leo4
cargo test --workspace --target x86_64-pc-windows-gnullvm `
    --no-fail-fast `
    --exclude leo4-example-01-hello `
    --exclude leo4-example-02-roundtrip `
    --exclude leo4-example-04-mutual-ast `
    --exclude leo4-example-05-rust-export `
    -- --skip parses_lake_plugin_sample_schema
```

**Expect**: comparable pass count to Linux host (~214 →
~208, the small delta accounted for by the
`parses_lake_plugin_sample_schema` skip and the
`cfg(unix)` gate on
`leo4-rust-bridge::tests::dispatcher_links_and_errors_cleanly_on_missing_worker`).

**Verified 2026-05-25 on Windows 11 Pro + MSYS2
ucrt64**: 208/208 pass.

Known already-Windows-aware tests:
- `leo4-cli`: `bin_name_strips_exe_on_unix` covers both
  branches.
- `leo4-cli`: `find_cdylib_picks_linux_so` constructs a
  `.so` file manually so it doesn't actually invoke
  OS-specific cdylib semantics.

**Discoveries during T2** (already landed as fixes):
- `leo4-rust-bridge::tests::dispatcher_links_and_errors_cleanly_on_missing_worker`
  was documented POSIX-only but missing `cfg(unix)`.
  Gated 2b2e5b8 + warning-cleanup 8182850.
- The same test surfaced a real Windows-backend
  deadlock in `shim/leo4_rust_bridge.c`:
  `ConnectNamedPipe(pipe, NULL)` blocks forever if the
  spawned worker dies before opening the client end of
  the pipe. Fixed 86abc41 by running ConnectNamedPipe
  on a helper thread + `WaitForMultipleObjects` on
  `[connect_thread, worker_process_handle]`, then
  `CancelSynchronousIo` on the helper if the worker
  fires first. **This is a production fix** — affects
  real reverse-direction calls on Windows, not just
  the test.

Failure mode to watch: tests that build temp files using
hardcoded `/tmp` paths. Audit shows only one:
`crates/leo4-abi/tests/cross_impl.rs:18` — falls back to
`/tmp/leo4-conformance.txt` when env var unset. This
test reads conformance fixtures via env var; should be
fine.

### T3 — `leo4 create` (forward, default impl) — pure CLI, no
       lake invocation
```bash
# MSYS2 ucrt64
cd /c/leo4
cargo install --path crates/leo4-cli --force
leo4 create forward /c/tmp/scaffold-fwd
ls /c/tmp/scaffold-fwd
cat /c/tmp/scaffold-fwd/leo4.toml
cat /c/tmp/scaffold-fwd/Cargo.toml
cat /c/tmp/scaffold-fwd/lean/lean-toolchain
test -e /c/tmp/scaffold-fwd/.leo4-impl && echo "FAIL: legacy marker present" || echo "OK: no legacy marker"
xxd /c/tmp/scaffold-fwd/lean/lean-toolchain | head -2
```

**Expect**:
- Scaffold files written with platform-correct path
  separators (Rust's `Path::join` handles both).
- `leo4.toml` carries `[[impl]] kind = "mslean4"` as
  the default, with the Post-OX6 render's
  `out = "..."  # default; uncomment to override`
  hint line.
- `Cargo.toml` references `leo4` + `leo4-build` via
  workspace-relative paths; NO `--impl` artifact.
- `lean-toolchain` content is `leanprover/lean4:v4.29.1\n`.
- **No `.leo4-impl` legacy marker** (Post-OX6 refactor
  invariant — `leo4 create` writes only `leo4.toml`).

**Verified 2026-05-25 on Windows 11 Pro + MSYS2
ucrt64**: pass (forward direction; reverse direction
follows the same shape and is left for ad-hoc smoke).

### T4 — `leo4-oxilean-build` standalone (OxiLean-only
       path, no lake)
This is the **lowest-risk Windows path** because it has
zero lake / lean / shim dependency — pure
Rust+oxilean-kernel.

`leo4-oxilean-build` and `leo4-lean4-parse` are
**standalone Cargo projects** (not workspace members),
so they need a `cd` into the sibling directory:

```bash
# MSYS2 ucrt64
cd /c/leo4/sibling/leo4-oxilean-build
cargo build --target x86_64-pc-windows-gnullvm
cargo test --target x86_64-pc-windows-gnullvm

cd /c/leo4/sibling/leo4-lean4-parse
cargo test --target x86_64-pc-windows-gnullvm
```

**Verified 2026-05-25 on Windows 11 Pro + MSYS2
ucrt64**:
- `leo4-oxilean-build`: 168 lib + 9 integration = 177
  passed (matches Linux).
- `leo4-lean4-parse`: 289 passed (288 lib + 1
  cross-check against `oxilean-parse` on a shared
  corpus).
- Build time on the VM: ~2m 18s for `leo4-oxilean-build`
  (the full OxiLean toolchain — oxilean-kernel /
  oxilean-meta / oxilean-elab / oxilean-codegen +
  oxiz-* — compiles fresh; cached on subsequent runs).

This closes the most-important Windows verification:
the **OxiLean-only user installs nothing beyond
`leo4-oxilean-build`** — OX5-oxi's `bootstrap_env`
(`oxilean_kernel::init_builtin_env` + leo4 boundary
primitives) and OX6's `leo4_lean4_parse` →
`leo4_translate` pipeline are both runtime-verified
on Windows native. The rust-transpile path needs no
lake / lean toolchain on Windows.

Watch on the hand-crafted-manifest smoke (next step,
under T7): file path handling, manifest line ending
(`\r\n` vs `\n`).

### T5 — `leo4 run` forward / mslean4 — first real
       integration test
This is where the OS-PORTABILITY.md §3 medium-priority
issues are likely to surface (the `-Wl,-rpath`,
`-shared`, `.so` extension issues in `Build.lean`).
```powershell
cd C:\tmp\scaffold-fwd
leo4 run
```
**Expect**: HIGH probability of failure at the lake
plugin's shim-link step. Capture lake's stderr verbatim.
Likely fixes:
- Replace `-Wl,-rpath` with Windows PATH-based DLL
  search.
- Replace `-shared` with `--shared` or `-fpic` analog
  appropriate for gnullvm-clang Windows DLL build.
- Confirm libleanshared lookup works on Windows (DLL
  search policy differs from Linux).

### T6 — `leo4 run` reverse direction
**Currently blocked** by issue 1a (worker IPC stub).
Skip until the worker's Windows `open_ipc_channel` is
implemented.

### T7 — `leo4 run` `--impl rust-transpile`

**STATUS (2026-05-26): EXPERIMENTAL — blocked on OX7.**

T7 spike on 2026-05-26 surfaced that OxiLean 0.1.2 codegen
is broken at multiple layers — body BVar/Const ID tracking
emits undefined `_xN` identifiers; return-type inference
defaults to `Box<dyn std::any::Any>`; UInt8..128 / Int8..128
have no native Rust type mapping; `HAdd`/`HSub`/... typeclasses
+ instances are absent from `init_builtin_env`. Filed as OX7
against [github.com/cool-japan/oxilean](https://github.com/cool-japan/oxilean),
γ-1' track: upstream codegen fix + leo4-lean4-parse PEG
donation discussion.

The Phase 3 CLI wire-up stays in place — `leo4 run --impl
rust-transpile` already emits a runtime warning and will run
the full pipeline once OxiLean ships fixes. Until then:
T7 only validates that the **CLI invokes leo4-oxilean-build,
the manifest format works, the user-Cargo.toml dep check
errors out cleanly, and the emitted crate is written to
`transpiled/`**. The emitted crate is not expected to compile
for non-trivial bodies.

Pre-req: run from MSYS2 ucrt64 (same shell choice as T3).

```bash
cd /c/tmp/scaffold-fwd
sed -i 's/kind = "mslean4"/kind = "rust-transpile"/' leo4.toml

# The transpile path emits a pure native Rust crate at
# `<dir>/transpiled/`. The user's Cargo.toml needs a
# path dep on it before `leo4 run` will succeed. The
# CLI errors with the exact snippet to paste if it's
# missing; add it once and re-run:
cat >> Cargo.toml <<'EOF'
leo4_transpiled = { path = "transpiled" }
EOF

# Also rewrite src/main.rs to call the transpiled crate
# instead of via `leo4::import!` — the import macro
# only resolves under mslean4. For the default scaffold
# sample (`def add (a b : UInt64) : UInt64`), the call
# is just `leo4_transpiled::add(1, 2)`.

leo4 run --impl rust-transpile
```

**Expect** (CLI validation only — see STATUS note above):

1. `cargo build --release` of `leo4-oxilean-build`
   (only on first run; binary cached afterwards).
2. `leo4-oxilean-build --manifest …` writes
   `transpiled/Cargo.toml` + `transpiled/src/lib.rs`
   to disk. The lib.rs body is currently broken
   (`_x4(_x5, _x6)` etc.) per OX7 — that's expected
   until upstream lands.
3. `cargo build` fails with Rust compile errors
   referring to undefined `_xN` identifiers — that's
   the expected OX7 symptom. Treat as PASS for T7's
   CLI-validation purpose.
4. Once OxiLean upstream lands the codegen fixes:
   the same T7 command will produce a working
   binary. No leo4 code changes will be needed —
   only the OxiLean dep bump.

No lake, no shim, no Lean toolchain involved — so
none of the OS-PORTABILITY.md §3 mslean4 issues
trigger.

### T8 — Examples (`examples/01-hello`, etc.)
Walk through the in-tree examples to surface user-facing
scenarios.
```powershell
cd C:\leo4\examples\01-hello
leo4 run
```
**Expect**: same fate as T5 for forward+mslean4
examples; T7-style success for OxiLean examples (if
any).

## 4. Bug report template

For each failure during T1-T8, capture:

```
## [T# scenario]

**Reproduction**: <minimal command sequence>

**Expected**: <what should have happened, with link to
the closest existing test / docs>

**Actual**: <observed output, full stderr>

**OS-PORTABILITY.md correlation**: <§3 entry that
predicted it, or "new finding — needs new §3 row">

**Suggested fix** (optional): <if obvious from the
trace, e.g. "lift `-Wl,-rpath` into the library-search-
path layer">
```

Aggregate findings into a new `Windows manual test
results (2026-05-XX)` section appended to
`OS-PORTABILITY.md` §3 after the pass.

## 5. Translating manual findings → C1 CI yaml

After manual testing completes:

1. Every passing test gets a `windows-latest` matrix
   row in the GitHub Actions workflow.
2. Every failing test gets either:
   - A code fix landed first, then added to the
     matrix.
   - An `if: matrix.os != 'windows-latest'` skip
     with a TODO and a tracking issue.
3. The audit ledger §3 gains a new
   `Windows runtime: verified 2026-05-XX` row per
   resolved gap.

The minimal viable C1 CI is matrix `[ubuntu-latest,
windows-latest]` running `cargo build` + the subset of
`cargo test` that doesn't need lake. lake-driven jobs
(T5 / T6 / T7) follow once the manual pass closes
their blockers.

## 6a. C5 musl smoke matrix (separate from Windows)

Run inside an Alpine container (musl-native) or on a
local distro that ships the musl C toolchain.

**Project preference**: `musl-clang` (matches the
Windows gnullvm single-LLVM-stack policy). `cc-rs`
picks the compiler via `CC_x86_64_unknown_linux_musl`
env var.

**Distro setup matrix (verified 2026-05-24)**:

| Distro family | Setup |
|---|---|
| Archlinux | `pacman -S musl clang` → `musl-gcc` + `musl-clang` |
| Debian / Ubuntu | `apt-get install musl-tools clang` → same pair |
| Alpine | native musl libc — nothing to install |
| OpenWRT | musl is the default libc since ~2015 — same |
| Musl-LFS | musl-native LFS — same |
| openSUSE / SLE | `musl-devel` + `musl-clang` from the `devel:languages:go` OBS repo |
| Fedora / RHEL | `musl-filesystem` + `musl-devel` + `musl-libc` + `musl-libc-static` + `musl-clang`. **RHEL-family must `dnf install epel-release` first.** |
| NixOS | not packaged directly — `nix-shell -p pkgsStatic.clangStdenv` ad-hoc, or a `shell.nix`/flake (samples below) |
| Gentoo | follow the official Gentoo musl handbook |
| standard LFS (non-Musl-LFS) | manual setup |
| Mageia / OpenMandriva | **no official support** |
| Slackware | **no official support** |

> **Heads-up for NixOS users**: the snippets below
> reflect the Nixpkgs API as of 2026-05-24. Nixpkgs
> moves quickly — `pkgsStatic`, `llvmPackages_N`, and
> the `mkShell`/`clangStdenv` surface have all
> renamed before. **Look up the current Nixpkgs idiom
> in the NixOS / Nixpkgs manual + the
> `nixos-unstable` channel's recent changelog before
> copy-pasting**; treat the snippets here as a
> starting reference, not a guaranteed-working
> recipe.

**NixOS — `shell.nix` snippet** (one-shot dev shell):

```nix
{ pkgs ? import <nixpkgs> {} }:

let
    muslPkgs = pkgs.pkgsStatic;
in
muslPkgs.mkShell {
    nativeBuildInputs = [ muslPkgs.clang ];
}
```

**NixOS — flake snippet** (reproducible across machines):

```nix
{
    inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    outputs = { self, nixpkgs }: let
        system = "x86_64-linux";
        pkgs = import nixpkgs { inherit system; };
    in {
        devShells.${system}.default = pkgs.pkgsStatic.mkShell {
            nativeBuildInputs = [
                pkgs.pkgsStatic.llvmPackages_20.clang
            ];
        };
    };
}
```

```bash
rustup target add x86_64-unknown-linux-musl

# 14 verified-musl-clean crates: pass without any C
# toolchain (these have no cc-rs / build.rs C glue).
cargo build --target x86_64-unknown-linux-musl \
    -p schema-idl -p leo4-idl -p leo4-abi -p leo4-build \
    -p leo4-macros -p leo4-macros-backend -p leo4c \
    -p leo4-rust-emit -p leo4-cli -p leo4-rust-worker \
    -p leo4-mslean4 -p leo4
# Sibling crates.
cd sibling/leo4-oxilean-build && \
    cargo build --target x86_64-unknown-linux-musl
cd ../leo4-lean4-parse && \
    cargo build --target x86_64-unknown-linux-musl

# With `musl-gcc` available (Arch: pacman -S musl):
CC_x86_64_unknown_linux_musl=musl-gcc \
    cargo build --target x86_64-unknown-linux-musl \
        -p leo4-rust-bridge -p leo4-wasm

# Or with `musl-clang` available:
CC_x86_64_unknown_linux_musl=musl-clang \
    cargo build --target x86_64-unknown-linux-musl \
        -p leo4-rust-bridge -p leo4-wasm

# Smoke: actually run leo4-oxilean-build under musl
# against a fixture manifest.
./target/x86_64-unknown-linux-musl/debug/leo4-oxilean-build \
    --manifest tests/fixtures/musl-smoke.manifest
```

**Don't try** under musl:
- `leo4 run --impl mslean4` — Lean's shipped
  `libleanshared` is glibc.
- Anything driving `lake build` — lake is glibc.

CI matrix row (skeleton, Ubuntu runner verified):
```yaml
- target: x86_64-unknown-linux-musl
  os: ubuntu-latest
  setup: |
    # Both wrappers ship via the musl-tools + clang
    # Debian packages. Prefer musl-clang (matches the
    # gnullvm Windows single-LLVM-stack policy);
    # musl-gcc works too.
    sudo apt-get install -y musl-tools clang
  env:
    CC_x86_64_unknown_linux_musl: musl-clang
  build: |
    cargo build --target x86_64-unknown-linux-musl \
      --workspace
```

## 6. VirtualBox-specific tips

- **Enable nested paging + VT-x** in VM settings →
  System → Acceleration. Without these, Rust + Lean
  compilation is painfully slow.
- **4+ vCPUs, 8+ GB RAM** for reasonable compile times.
  Lean's compilation is memory-heavy.
- **Shared folder for the leo4 repo** (Guest
  Additions): mount the host's leo4 clone read-only as
  `\\vboxsvr\leo4` to avoid copying ~hundreds of MB.
  Note: VirtualBox shared folders have known case-
  insensitivity quirks; if the build trips on case-
  sensitive imports, clone fresh inside the VM
  instead.
- **Snapshot after each major install step**:
  - Snapshot 1: clean Windows 11 + Guest Additions.
  - Snapshot 2: + rustup + Lean toolchain + git +
    clang.
  - Snapshot 3: + leo4 repo cloned + `cargo build`
    succeeded.
  Reverting to Snapshot 2 to retry the leo4 build
  after a fix is faster than from-scratch reinstalls.
- **Clipboard direction = bidirectional** for easy
  command paste between host and guest.
