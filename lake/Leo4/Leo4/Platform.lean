-- Leo4.Platform — the first leo4-Lean OS abstraction layer.
--
-- Per OS-PORTABILITY.md §1 policy, OS-specific code must
-- live in an identified layer. This module covers the
-- linker / dynamic-library file conventions concern that
-- previously had ad-hoc `.so` / `-Wl,-rpath` literals in
-- `Leo4.Build`.
--
-- Layer membership: §2 row "Linker / dynamic-library file
-- conventions" (added 2026-05-24 with the first
-- Tier 2 Windows runtime audit pass).
--
-- Branching uses `System.Platform.isWindows` /
-- `System.Platform.isOSX` (Lean stdlib's
-- `Init.System.Platform`). All branches are evaluated at
-- the Lean process's startup; the produced strings are
-- pure values without per-call overhead.

namespace Leo4.Platform

/-- Filename extension for the dynamic library a `leanc
    -shared` link produces on this platform.
    Linux/POSIX → `"so"`, macOS → `"dylib"`,
    Windows (gnullvm) → `"dll"`.
    Note: leo4's *internal* `<pkg>.leo4-shim.so` naming
    intentionally uses `.so` on every platform — see
    `defaultShimSuffix` for that convention. The
    `dynlibExt` here is the *system* convention used for
    detection / discovery of platform-native libraries
    (the per-package libs Lake's `lake build <Module>:shared`
    produces). -/
def dynlibExt : String :=
  if System.Platform.isWindows then "dll"
  else if System.Platform.isOSX then "dylib"
  else "so"

/-- Filename prefix Lake / clang produce for dynamic
    libraries on this platform. POSIX ELF/Mach-O outputs
    are typically `lib<name>.<ext>`; gnullvm-clang on
    Windows emits bare `<name>.dll` (the import-library
    `lib<name>.dll.a` keeps the `lib` prefix, but that's
    a static archive, not a DLL we'd dlopen). -/
def dynlibPrefix : String :=
  if System.Platform.isWindows then "" else "lib"

/-- True iff `name` matches the platform's dynamic-library
    naming convention.
    Linux: `name` starts with `"lib"` AND ends with `".so"`.
    macOS: `name` starts with `"lib"` AND ends with `".dylib"`.
    Windows (gnullvm): `name` ends with `".dll"` (no `lib`
    prefix requirement — gnullvm-clang doesn't add one). -/
def isPlatformDynlib (name : String) : Bool :=
  let extDot := "." ++ dynlibExt
  if dynlibPrefix.isEmpty then
    name.endsWith extDot
  else
    name.startsWith dynlibPrefix && name.endsWith extDot

/-- Extract the link-stem (`-l<stem>`) for a filename that
    matches `isPlatformDynlib`. Strips the prefix and the
    `.<ext>` suffix.
    Pre: `isPlatformDynlib name = true`. -/
def stemOfDynlib (name : String) : String :=
  let withoutPrefix := name.drop dynlibPrefix.length
  -- Drop the trailing `.<ext>`. `dropRight` in modern
  -- Lean returns a `String.Slice`; project back to
  -- `String` so the result composes with `s!"-l…"`.
  (withoutPrefix.dropEnd (dynlibExt.length + 1)).toString

/-- The platform-conventional internal suffix for a leo4
    shim. **By design** this is `".so"` on every platform
    — the file produced is a PE DLL on Windows, but the
    leo4 readers (`leo4-build` / `leo4-mslean4`) and the
    plugin writer all agree on the `.so` suffix so the
    handshake / mangling / shim file triple stays
    discoverable with a single naming rule. `libloading`
    (Rust) and `LoadLibraryW` (Windows) both accept any
    extension. -/
def defaultShimSuffix : String := ".so"

/-- Render an RPATH linker flag if the platform supports
    runtime library search paths embedded in the produced
    binary.
    Linux / macOS → `some "-Wl,-rpath,<dir>"`. PE binaries
    (Windows) don't have an rpath concept; DLL search is
    governed by `LoadLibrary` resolution order (PATH,
    .exe-adjacent dir, AppPaths, system dirs,
    `AddDllDirectory` API) → returns `none`. The caller
    is responsible for an alternative DLL discovery path
    on Windows (typically: place dependent DLLs next to
    the shim, or set PATH before loading). -/
def linkRpath? (dir : String) : Option String :=
  if System.Platform.isWindows then none
  else some s!"-Wl,-rpath,{dir}"

/-- `name` for `lake build`-produced shared libs in a
    `.lake/build/lib` directory: produced by
    `dynlibPrefix ++ stem ++ "." ++ dynlibExt` formatter,
    expected by `isPlatformDynlib` parser. -/
def dynlibFileName (stem : String) : String :=
  s!"{dynlibPrefix}{stem}.{dynlibExt}"

end Leo4.Platform
