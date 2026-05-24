# OS Portability — Policy and Audit Ledger

> Status: living document. Drafted 2026-05-21 alongside the Phase
> 9-0 spawn / IPC abstraction landing (SPEC/reverse-direction.md
> §4.4). Updated as new OS branches are identified or lifted into
> existing layers.

## 0. Why this document exists

leo4 v0.1.0 runs end-to-end on Tier 1 (x86_64 Linux). macOS is
Tier 3 (best-effort, no CI) and Windows is Tier 2 (feature parity
expected, periodic CI). The codebase has accumulated a handful of
Linux-shaped assumptions (`.so` extension, `-Wl,-rpath`,
`__attribute__((visibility))`, etc.) in scattered places. As
Phase 9 introduces a deliberate spawn / IPC abstraction layer
(`SPEC/reverse-direction.md` §4.4), it makes sense to make the
same abstraction discipline a leo4-wide policy and to inventory
where current OS branches live.

This is the home for both.

## 1. Policy

**Rule.** OS-specific code in leo4 must be confined to identified
abstraction layers. The rest of the codebase addresses OS
differences only through:

1. **Standard-library cross-platform APIs** — Rust `std::fs`,
   `std::process`, `std::path::Path`; Lean's `IO.FS` and `System`
   namespaces.
2. **A leo4-internal abstraction layer**, defined per concern,
   with a documented interface (see §2 below).
3. **A third-party crate whose stated purpose is cross-platform
   abstraction** — currently `libloading` (dynamic loading) and
   `linkme` (static metadata collection).

**Tier 2 Windows target choice (adopted 2026-05-21).** leo4
targets `x86_64-pc-windows-gnullvm`, not `*-windows-msvc`. The
gnullvm target uses clang + lld + UCRT, matching the LLVM
toolchain stack leanc drives on every other tier. The practical
consequence for this document: **clang-style C ergonomics**
(`__attribute__((visibility("default")))`, `__builtin_memcpy`,
gcc command-line flags) are available on every tier, so several
concerns that would have needed an abstraction layer under MSVC
do not need one. The §2 table records which concerns remain
under-abstracted given this target choice.

**C ↔ Rust ABI compatibility on Windows.** Per the rustc
platform-support docs for `x86_64-pc-windows-gnullvm`: Rust
binaries on that target are ABI-compatible with C code built
through an **LLVM-based** C toolchain targeting either
`x86_64-pc-windows-gnu` (mingw triple, but compiled with clang)
or `x86_64-pc-windows-gnullvm`. This is *not* automatic with
mingw-w64 `gcc` — the C compiler must be `clang`. leo4 enforces
the LLVM track end-to-end:

1. The forward shim and the reverse-direction dispatcher are
   both C code; leo4's build path drives them through `leanc`,
   which on Windows already wraps clang.
2. User cdylibs (Phase 9 reverse direction) are Rust, built
   for `x86_64-pc-windows-gnullvm`. They cannot link MSVC-ABI
   C++ libraries directly, but they can link any C library
   built with clang on either Windows triple.

The Phase 9 spawn / IPC layer's Windows branch (`CreateProcess`
+ named pipe, see `SPEC/reverse-direction.md` §4.4) is plain
Windows API, available regardless of which Windows target is
chosen — the gnullvm choice is about ABI alignment and C
ergonomics, not about access to system calls.

A new commit that adds a `#[cfg(target_os = …)]`, `cfg(unix)`,
`cfg(windows)`, `cfg(target_family = …)`, or a `System.os` /
`System.Platform`-driven Lean branch **outside an identified
layer** is reviewed under this policy and either:

- moved into an existing layer,
- promoted into a new layer (this document gains a new entry in §2),
- or rejected as accidental Linux-shape leakage.

**Rationale.** OS branches are easy to add and very hard to keep
consistent. Centralising them lets us pay the per-platform tax
once per concern instead of N times.

## 2. Identified layers

Each layer has: a concern, an interface, the
implementations it currently has, and a status. New layers go in
this table.

| Layer | Concern | Interface | Implementations | Status |
|---|---|---|---|---|
| **Spawn / IPC** | worker process lifecycle + IPC for reverse direction | `leo4_worker_ops_t` (`SPEC/reverse-direction.md` §4.4) | stub (9-4a), POSIX (9-4b, `posix_spawn` + `socketpair`), Windows (9-4c, `CreateProcessA` + `CreateNamedPipeA`) | **POSIX + Windows code landed 2026-05-23**; Windows runtime verification follows the Tier 2 CI matrix |
| **Dynamic library loading (Rust)** | open shim `.so` and resolve symbols | `libloading::Library` / `Symbol` | one (cross-platform) | adequate |
| **Dynamic library loading (Lean)** | wrapper-module init at runtime | Lean's `@[extern]` + leanc link step | one (host-platform leanc decides) | adequate |
| **Dynamic library naming** | choose `.so` / `.dylib` / `.dll` for a given package | TBD | Lean side hard-codes `.so` (`lake/Leo4/Leo4/Build.lean:227`) | **needs layer** |
| **C compiler visibility attribute** | mark shim entry points exported | not needed under gnullvm target choice | `__attribute__((visibility("default")))` works on every tier (Linux gcc / clang, gnullvm clang, macOS clang) | covered by Tier 2 target choice |
| **Shared-library RPATH / DLL search path** | resolve `libleanshared` + user `.so` at load time | TBD | `-Wl,-rpath,...` (Linux/macOS) only (`lake/Leo4/Leo4/Build.lean:233, 262`) | **needs layer** |
| **Filesystem atomic write** | handshake / mangling / schema emit | Lean `IO.FS.writeBinFile` + rename-into-place | one (Lean) | adequate on POSIX; needs review on Windows |
| **Path separators / extensions** | constructing build-output paths | Lean `System.FilePath` / Rust `std::path::Path` | std-library | adequate |
| **Environment variable conventions** | runtime cdylib / shim discovery | leo4-defined names (`LEO4_SHIM_SO`, `LEO4_RUST_CDYLIB`, …) | one (leo4) | adequate |

## 3. Audit — current OS-specific branches

Entries to lift into a layer or recheck. Updated as commits land
or new branches are discovered.

| Location | Branch / assumption | Concern | Recommended layer | Priority |
|---|---|---|---|---|
| `lake/Leo4/Leo4/Build.lean:227` | `name.endsWith ".so"` | Dynamic library naming | new "library extension" layer (returns `.so` / `.dylib` / `.dll`) | medium |
| `lake/Leo4/Leo4/Build.lean:233, 262` | `-Wl,-rpath,...` | Runtime library search path | new "library search path" layer (Linux: rpath; macOS: rpath / `@loader_path`; Windows: PATH at load / install dir / nothing) | medium |
| `lake/Leo4/Leo4/Build.lean:259` | `-shared` on `leanc` | shared-library link command | overlap with the above; can share the layer | medium |
| `lake/Leo4Plugin/Leo4Plugin/Main.lean:1792` | `__attribute__((visibility("default")))` in shim source | C compiler visibility | **covered** — gnullvm Tier 2 target choice keeps clang `__attribute__` available on every tier | resolved |
| `lake/Leo4Plugin/Leo4Plugin/Main.lean` (shim emit, generally) | `-fPIC`, gcc/clang command line | C compiler flags | **covered** — same reason; gcc-style flags work on every tier via leanc / clang / gnullvm-clang | resolved |
| `shim/leo4_rust_bridge.c` (Phase 9-4) | `posix_spawn` / `CreateProcessA`, `socketpair` / `CreateNamedPipeA`, dispatcher-side reaping | Spawn / IPC + worker lifecycle | `leo4_worker_ops_t` — POSIX + Windows backends both implemented | resolved |
| `crates/leo4-rust-worker/src/main.rs:330` | Windows `open_ipc_channel` client side (`CreateFileW` on the dispatcher's named pipe + retry on race) | Spawn / IPC — worker side counterpart | `open_windows_pipe` via `std::fs::OpenOptions::open` (CreateFileW under the hood); 10× linear backoff on `NotFound`/`ConnectionRefused` for the narrow worker-spawned-before-pipe-registered race | resolved (cross-compile clean on `x86_64-pc-windows-gnullvm`; runtime verification follows Tier 2 CI) |
| `crates/leo4-build/src/lib.rs:24` (comment) | acknowledges `.so` / `.dylib` / `.dll` exist but only wires `.so` | Dynamic library naming | use the same layer as `lake/Leo4/Leo4/Build.lean:227` | low |

## 4. Conventions for new layers

When a layer is needed:

1. **Pick a name and define the interface in a single place.** For
   C / shim code: a `LEO4_*` macro family or a `leo4_*_ops_t` table
   in `shim/`. For Rust: a trait or function set in a leaf crate.
   For Lean: a module under `lake/Leo4/Leo4/Platform/` (to be
   created when the first layer lands).
2. **Stub backend first.** Every layer ships a fallback that the
   build always compiles, even on unsupported platforms. The
   fallback may be "every operation returns an error" — what matters
   is that the build never fails because a platform is unsupported,
   only the runtime call does.
3. **Cite the layer in the SPEC** if the concern is normative
   (e.g. visibility macros affect the ABI). For purely
   build-time concerns (path layouts) a comment in the Lean / Rust
   source pointing at this document suffices.
4. **Add an entry to §2 of this document** the same commit the
   layer lands. Failing audit entries from §3 move into §2 once
   they have a layer.

## 5. Cross-references

- `SPEC/reverse-direction.md` §4.4 — spawn / IPC abstraction
  (first formally-specified layer).
- `LEO4-DESIGN.md §9.1` — platform tier policy (Tier 1 Linux /
  Tier 2 Windows / Tier 3 macOS).
- `CLAUDE.md` — code conventions, including a pointer back to this
  document.
- `ROADMAP.md` Phase 9 — substeps 9-4a / 9-4b / 9-4c implement
  the spawn layer.
