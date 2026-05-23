# Changelog

All notable changes to leo4 will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to Semantic Versioning once it reaches 0.1.0.

## [Unreleased]

### Added — Phase 9-6 follow-up commit 1/3: `Leo4Rust` Lake package + `leo4RustBridge` extern_lib (2026-05-23)

First of three commits collapsing the reverse-direction
manual `leanc -o` step. Per `spike/SPIKE-1-lake-extern-lib.md`'s
4a pattern (path resolution only) and option R2 (logicutils
optional, fallback to unconditional rebuild).

- `lake/Leo4Rust/lakefile.lean` (new) — `package Leo4Rust`,
  `require Leo4 from ".." / "Leo4"`, empty marker `lean_lib`
  for default-target compliance.
- `extern_lib leo4RustBridge pkg := …` — path resolution
  chain for `libleo4_rust_bridge.a`:
  1. env `LEO4_RUST_BRIDGE_AR` (explicit override)
  2. `<leo4_repo>/target/release/libleo4_rust_bridge.a`
  3. `<leo4_repo>/target/debug/libleo4_rust_bridge.a`
  `<leo4_repo>` resolves as `pkg.dir / ".." / ".."` —
  `lake/Leo4Rust/../..` lands at the leo4 repo root.
  Body returns `(Pure.pure path)` (Lake's `Job` `Pure`
  instance). Missing archive → `error s!"…"` via Lake's
  `Util.Log.error` helper with a clear "run cargo build
  first" pointer + full search list.
- `lake/Leo4Rust/Leo4Rust.lean` (new) — empty marker module.
  Importing it is unnecessary for the link integration to
  fire (Lake's `lean_exe` walks `dep.externLibs`
  unconditionally), but having a default `lean_lib` lets
  `lake build` work without explicit targets.
- `lake/Leo4Rust/lean-toolchain` — pinned to
  `leanprover/lean4:v4.29.1` matching the rest of the repo.

Verified:
- `cd lake/Leo4Rust && lake build` clean (Built Leo4Rust
  140ms).
- `lake build leo4RustBridge.static` resolves correctly when
  cargo-built archive is present.
- `LEO4_RUST_BRIDGE_AR=/no/such/file lake build
  leo4RustBridge.static` errors with the expected
  "libleo4_rust_bridge.a not found. Searched: /no/such/file …
  Run cargo build --release -p leo4-rust-bridge…" message.

Next:
- Commit 2/3 adds `leo4RustBridgeLean` extern_lib (leanc + ar
  on `shim/leo4_rust_bridge_lean.c`, with `freshcheck`
  optional gate via `which`).
- Commit 3/3 wires `examples/05-rust-export/lean/lakefile.lean`
  through `require Leo4Rust`, drops the manual `leanc -o`
  step from the example's README + the `just
  rust-export-05-build` recipe.

### Added — `leo4` CLI: `create` (new project) + `init` (existing crate) (2026-05-23)

New workspace member `crates/leo4-cli/` shipping a `leo4`
binary with two scaffolding subcommands. Distinct semantics:

- **`leo4 create <direction> <dir>`** — new project. Creates
  the directory (or expects it empty), writes a complete
  buildable skeleton: `Cargo.toml`, `src/`, `lean/`,
  `README.md`. `cargo new`-style ergonomics.
- **`leo4 init <direction>`** — in-place integration into an
  *existing* Cargo crate (cwd by default, or `--dir`). Adds:
  - a `# ─── leo4 integration ───` block appended to
    `Cargo.toml` (idempotent — re-running skips when the
    marker line is already present);
  - `build.rs` (forward direction only) if absent;
  - `lean/{lakefile.lean,Sample.lean or Main.lean,lean-toolchain}`
    if absent.
  Existing `src/` is never touched.

Both subcommands accept `forward` (`@[leo4_export]` + Rust
`leo4::import!`) or `reverse` (`#[leo4::export]` + generated
Lean wrapper) directions. `--leo4-root <path>` overrides the
default `../leo4` sibling path for the generated Cargo /
Lake `require` entries.

Templates produce:
- Forward: `Cargo.toml` (leo4 + leo4-build), `build.rs`
  wiring the Lake shim, `src/main.rs` with a `leo4::import!`
  block calling `hello` / `add`, `lean/Sample.lean` with
  matching `@[leo4_export]`s.
- Reverse: `Cargo.toml` (`[lib] crate-type=["cdylib"]`,
  leo4 with `rust-exports` feature), `src/lib.rs` with
  `#[leo4::export] pub fn double / greet`, `lean/Main.lean`
  importing the generated `<Iface>.Rust` wrapper.

CLI sanity:
- 3 unit tests (camel_case, Cargo.toml name extraction,
  idempotent Cargo.toml extension).
- End-to-end smoke: `leo4 create forward /tmp/x` produces 7
  files in the expected layout; `leo4 init reverse --dir
  <pre-existing>` appends the integration block + writes
  lean/ without touching existing `src/main.rs`; a second
  `init` reports `skip … already exists` for every entry.

Workspace test count 138 → 141; all green. `cargo install
--path crates/leo4-cli` makes `leo4` available on PATH.

### Added — Phase 9-4c: Windows backend (`CreateProcess` + named pipe) (2026-05-23)

Fills the second real branch of `leo4_worker_ops_t`. Same
single C TU; lives behind `#if defined(_WIN32)` next to the
POSIX backend, with backend selection chain updated to pick
`&leo4_windows_ops` on `_WIN32`.

Workflow:

1. `CreateNamedPipeA` opens a duplex pipe at
   `\\.\pipe\leo4_rust_<pid>_<nonce>` (nonce: process-wide
   `_Atomic uint32_t` counter for multi-spawn safety).
2. `CreateProcessA` launches `leo4-rust-worker.exe --cdylib
   <path> --ipc-pipe <name>` via PATH search
   (`lpApplicationName = NULL`). `ERROR_FILE_NOT_FOUND`
   maps to `LEO4_ERR_RUST_CDYLIB_NOT_FOUND`; everything
   else to `LEO4_ERR_RUST_SPAWN_FAILED`.
3. `ConnectNamedPipe` blocks until the worker's
   `CreateFileA` on the same pipe name resolves. The
   worker's `--ipc-pipe` argument carries the same string
   (separate from POSIX's `--ipc-fd N`).
4. `win_send_all` / `win_recv_exact` loop over `WriteFile` /
   `ReadFile` with short-read detection. EOF / 0-byte ->
   `LEO4_ERR_RUST_IPC_FAILED`.
5. `win_alive_worker` polls via `WaitForSingleObject(.., 0)`;
   `win_reap_worker` does the blocking wait + `GetExitCodeProcess`
   + `CloseHandle` on both pipe and process.
6. `win_kill_worker` calls `TerminateProcess`.

Target triple: `x86_64-pc-windows-gnullvm` (Tier 2, per
LEO4-DESIGN.md §9.1). Compiles under clang's gnullvm target —
no `__declspec`, no MSVC ABI fork. Linux/macOS builds skip
the whole block via the `#ifdef` guard.

Cross-compile / runtime verification on Windows is deferred
to the Tier 2 CI matrix when it lands. Today the code
compiles in-source against the `windows.h` we'd see on a
gnullvm clang invocation; the Linux host build is
unaffected (no `_WIN32` defined).

`leo4-rust-worker` (the Rust harness, Phase 9-3) already
has the `--ipc-pipe <name>` arg parsed and returns
"Windows named-pipe IPC not yet implemented (Phase 9-4c)" —
that's the slot the worker side fills next.

Workspace test count unchanged at 138/0.

### Added — Phase 9.X: env-driven worker recycle policy (2026-05-23)

`LEO4_RUST_WORKER_RECYCLE_CALLS=N`: after N completed
dispatcher calls, the persistent worker is killed and a
fresh one lazy-spawns on the next call. Default `0` (or
unset / invalid) keeps the worker up for the whole Lean
process lifetime, matching pre-9.X behaviour.

Dispatcher changes (`shim/leo4_rust_bridge.c`):

- `leo4_worker_slot_t` gains a `_Atomic uint64_t call_count`
  field. Incremented after each successful
  request/response round-trip.
- `leo4_recycle_calls_limit` (file-scoped `_Atomic uint64_t`)
  parsed once from the env via `strtoull`; non-numeric or
  zero leaves recycling disabled.
- `leo4_recycle_init_once` runs at most once per process via
  a compare-exchange guard.
- `leo4_recycle_persistent_slot` atomically swaps the
  worker pointer out, then `kill` + `reap` via the ops
  table — no OS syscall named outside the backend block, per
  SPEC §4.4.
- `leo4_rust_call`'s persistent-worker branch checks the
  counter *before* the lazy-spawn lookup; if reached, reaps
  the current worker before falling into the standard lazy
  spawn path.

Time-based recycle (`LEO4_RUST_WORKER_RECYCLE_SECONDS`) is a
further 9.X follow-up; call-based is what ships today
(simpler, sufficient for "keep memory bounded over long-run
SMT-solver sessions").

Workspace test count unchanged at 138/0; full end-to-end
exercise of the recycle path lives in the manual
`just rust-export-05-build` workflow with the env set.

### Added — Phase 9.X: `#[leo4::export(isolated)]` dispatcher path (2026-05-23)

Per-call fresh worker for `isolated`-tagged exports. The
attribute itself shipped in 9-1 (parsed by the macro and
recorded in `ExportEntry.isolated`); 9-5 / 9-6 ignored it. This
commit wires it through the dispatcher.

Mechanism (minimal wire surface change):

- `leo4-rust-emit`'s Lean wrapper render now prefixes the
  mangled name with `iso:` for exports tagged
  `#[leo4::export(isolated)]`. Persistent exports pass the
  raw mangled name verbatim.
- `shim/leo4_rust_bridge.c`'s `leo4_rust_call` detects the
  `iso:` prefix via `memcmp`. When present, it strips the
  prefix and routes through a new `leo4_dispatch_isolated`
  helper:
  1. Allocate a fresh worker via `leo4_worker_ops->spawn`
     (separate process from the persistent slot).
  2. Send the request frame.
  3. Receive the response frame.
  4. Send a magic=0 graceful-shutdown frame.
  5. `leo4_worker_ops->reap` the worker.
- The persistent worker is unaffected — it keeps running
  across calls, just as before.

Why the prefix trick: no SPEC wire format change, no new
dispatcher API entry, no Lean wrapper signature change. The
typed wrapper renders identically except for the literal
string passed to `leo4RustCallRaw`. Backwards-compatible with
9-5's wrapper consumers.

Cost: per-call worker spawn (`posix_spawn` ~5-10 ms on Linux).
Use only for exports whose state contamination would corrupt
later unrelated calls. Persistent mode remains the default.

`shim/leo4_rust_bridge.c` builds clean under `-std=c23` /
`-std=c2x` / `-std=c17`. Workspace test count unchanged at
138/0 (the change adds dispatch-path logic that needs the
worker harness + cdylib to verify end-to-end; that lives in
the manual `just rust-export-05-build` workflow).

### Added — Phase 9-6 follow-up: Lake automation for the reverse-direction pipeline (2026-05-23)

Collapses the 4-step manual workflow from
`SPEC/reverse-direction.md` §7 into named `just` recipes +
reusable `Leo4.Build.RustBridge` helpers a user lakefile can
call directly. The end-to-end demo (`examples/05-rust-export/`)
now builds via a single `just rust-export-05-build`.

**`justfile` additions**:

- `rust-bridge-build` — builds the three Cargo artefacts the
  reverse direction needs in one shot (`leo4-rust-bridge` static
  archive, `leo4-rust-worker` binary, `leo4-rust-emit` CLI).
- `rust-emit CDYLIB OUT_DIR MODULE` — variable-parameterised
  wrapper around `cargo run -p leo4-rust-emit --emit-lean`. Any
  user cdylib can be wired through one command.
- `glue-shim-build OUT_OBJ` — `leanc -c -std=c2x
  shim/leo4_rust_bridge_lean.c -o $OUT_OBJ` (the one place
  `lean.h` legitimately enters the build).
- `rust-export-05-build` — end-to-end recipe for
  `examples/05-rust-export/`. Builds the cdylib + bridge +
  worker, runs `leo4-rust-emit --emit-lean`, moves the Lean
  wrapper to `lean/Leo4ExampleMiniSolverRust/Rust.lean` so
  Lake picks it up under the expected module path, compiles
  the glue shim, and runs `lake build`. The final
  `leanc -o` link is still manual (Lake's `lean_exe` DSL
  doesn't take dynamic link args yet) — the README documents
  the one-line link command.
- `rust-export-05-clean` — drops the emitted artefacts.

**`lake/Leo4/Leo4/Build.lean` additions**:

`Leo4.Build.RustBridge` namespace with three IO helpers a
user lakefile can call from a `script` block or a
`def main`-style helper:

- `compileGlueShim leo4Root outObj : IO FilePath` — invokes
  `leanc -c -std=c2x` on `shim/leo4_rust_bridge_lean.c`. Throws
  `IO.userError` (forwarding stderr) on non-zero exit.
- `discoverBridgeArchive leo4Root : IO FilePath` — locates
  `libleo4_rust_bridge.a` via env `LEO4_RUST_BRIDGE_AR` first,
  then `target/release/`, then `target/debug/`. Mirrors the
  `LEO4_SHIM_SO` search chain in `leo4-build::wire`.
- `linkArgs leo4Root glueObj : IO (Array String)` — returns
  `#[glueObj, bridgeArchive]` for caller-side splicing into
  `weakLinkArgs` or a manual leanc invocation.

These don't yet plug into Lake's `lean_exe` DSL — Lake's
`weakLinkArgs` is currently a static-Array field and there is
no first-class hook for "build the user's executable after a
dynamic IO action computes link args". When that hook lands,
the helpers are ready.

**`examples/05-rust-export/README.md` updated** with the
fast path (`just rust-export-05-build`) at the top + the
manual 4-step kept as a reference / debugging fallback.

No code-test changes; workspace count stays at 138/0.

### Added — `AGENTS.md` cookbook for Claude Code sessions (2026-05-23)

Companion to `CLAUDE.md`. CLAUDE.md is the working agreement
(how to behave); AGENTS.md is the cookbook — concrete
patterns for what to type when starting a task.

Contents:

- **Document routing**: a table mapping common starting
  questions to the doc that answers them. Reduces "read
  source to figure out what the doc should have said" loops.
- **Forward vs reverse direction**: side-by-side table of
  the two pipelines' mental models. The "where can `lean.h`
  live?" question gets a hard rule answer.
- **Commit cadence**: which files usually move together for
  each commit shape (SPEC, macro, CLI, C shim, e2e example).
  Reflects the actual Phase 9 history.
- **Boundary-type checklist**: 8-step refinement of the
  7-step one in CLAUDE.md. Adds reverse-direction
  considerations and Mathlib bridge entry.
- **OS-portability layer recipe**: stub-first, single
  interface, audit-ledger entry — the 9-4 spawn/IPC layer
  is the template.
- **Phase entry-gate**: design commit before any code; cites
  9-0 as the model.
- **Cargo / Lake / leanc cheatsheet**.
- **Common pitfalls (8 entries)**: schema_hash placement
  in reverse mangling, `_GNU_SOURCE` needed under strict
  C-std for POSIX symbols, `--gc-sections` and standalone
  link, `cc::Build::std` single-arg limitation,
  `IO α → future<α>` lift, `linkme` distributed-slice
  static appearing unused, schema_hash recomputation
  match, sticky `cargo:rerun-if-changed=`, variant
  discriminator being u32 LE not u8.
- **Subagent guidance**: when Explore / Plan /
  general-purpose actually help vs add overhead.
- **Recent decisions worth remembering (6 entries)**:
  D16 reverse-direction adoption, gnullvm Tier 2,
  long-running-worker isolation model, spawn/IPC ops
  abstraction, D4 async lift, v0.1.0 cut hash. Each one
  expensive enough to land that an agent rediscovering it
  from scratch would burn hours.

`CLAUDE.md` opening matter updated with a one-line pointer
to the new cookbook.

### Added — Phase 9-7: `examples/05-rust-export/` end-to-end demo (2026-05-23)

Eighth code landing on the Phase 9 ladder. The first example
where every layer of the reverse-direction pipeline executes
end-to-end: Rust cdylib → `leo4-rust-emit` → Lean wrapper →
glue shim → dispatcher (`libleo4_rust_bridge.a`) → POSIX
worker (`leo4-rust-worker`) → cdylib's wrapper symbol → Rust
function → response all the way back.

- `examples/05-rust-export/` (new workspace member, cdylib +
  rlib crate). Four `#[leo4::export]` functions hitting the
  v9-5 Lean-wrapper mapping table:
  - `is_prime(u64) -> bool`               (scalar / scalar)
  - `next_prime(u64) -> u64`              (long-running loop)
  - `count_primes_below(u64) -> u64`      (compute-heavy)
  - `factor_smallest(u64) -> Option<u64>` (`Option<T>` return)
- Rust-side unit tests pin every function's behaviour (4
  tests, all passing as part of `cargo test --workspace`).
- `examples/05-rust-export/lean/` carries the Lean driver:
  `lakefile.lean` (NOT a workspace member of `lake/Leo4/`;
  it's a standalone Lake project that references the runtime
  library via a relative `require`), `lean-toolchain` pinned
  to `v4.29.1` to match the rest of the repo, and `Main.lean`
  that imports `Leo4ExampleMiniSolverRust.Rust` and prints
  each function's answer for a representative input set.
- `examples/05-rust-export/README.md` documents the 4-step
  manual build + run workflow (`cargo build` → `leo4-rust-emit
  --emit-lean` → `leanc -c shim/leo4_rust_bridge_lean.c` →
  `lake build` + manual `leanc -o`), with the env-var matrix
  (`LEO4_RUST_CDYLIB`, `LEO4_RUST_WORKER_BIN`,
  `LEO4_RUST_HANDSHAKE_PKG`, `LEO4_RUST_HANDSHAKE_IFACE`)
  the worker needs to recompute the schema_hash to match.

Pipeline smoke verified: `cargo run -p leo4-rust-emit --
--cdylib …/libleo4_example_05_rust_export.so --out-dir …
--emit-lean --lean-module Leo4ExampleMiniSolverRust.Rust`
emits all three artefacts with schema_hash `ozln3adaktdow`,
the Lean wrapper carries `def is_prime : IO Bool`,
`def factor_smallest : IO Option UInt64`, etc, and the
handshake JSON lists the four expected mangled symbols
(`leo4_rust__is_prime__u64`, …).

The end-to-end *runtime* (Lake-built Lean executable
actually calling the cdylib through the dispatcher + worker)
needs the four-step manual workflow today; the README walks
through every command. Lake-plugin auto-discovery of the glue
shim source + the bridge static archive is a 9-6 / 9-7
follow-up — until it lands, this demo is the
load-bearing reference for "what does the user actually
type to make this work?".

Workspace test count 134 → 138; all green.

### Added — Phase 9-6: Lean-side glue shim (`shim/leo4_rust_bridge_lean.c`) (2026-05-23)

Seventh code landing on the Phase 9 ladder. Resolves the
`@[extern "leo4_rust_call_lean"]` declaration the Phase 9-5
wrapper emits, bridging the lean_object* ABI to the
dispatcher's byte-pointer signature in
`libleo4_rust_bridge.a`.

- `shim/leo4_rust_bridge_lean.c` (new, ~110 lines). The ONE
  leo4-side place that includes `<lean/lean.h>`. The
  dispatcher and its backends stay free of Lean ABI details,
  matching the forward-direction split
  (`<pkg>.leo4-shim.c` vs `crates/leo4-native/`).
- `leo4_rust_call_lean(b_lean_obj_arg mangled, b_lean_obj_arg
  args, lean_object* world) -> lean_object*`:
  1. Extract `(cstr, size)` from `mangled` via
     `lean_string_cstr` + `lean_string_size` (subtracting the
     trailing NUL from `size`).
  2. Extract `(ptr, size)` from `args` via `lean_sarray_cptr`
     + `lean_sarray_size`.
  3. Allocate a 4 KiB initial response ByteArray
     (`lean_alloc_sarray(1, cap, cap)`).
  4. Call `leo4_rust_call(mangled, mangled_len, args_ptr,
     args_len, ret_ptr, ret_cap, &ret_len)`.
  5. On `LEO4_ERR_BUFFER_TOO_SMALL` (0x07): drop the
     too-small ByteArray, re-allocate to the size the
     dispatcher reported in `*ret_len`, retry once.
  6. Shrink the response ByteArray's logical size to the
     actual `ret_len` via `lean_sarray_set_size` (the
     underlying allocation stays at `cap`).
  7. Build the `(UInt32 × ByteArray)` tuple
     (`lean_alloc_ctor(0, 2, 0)` + `lean_box_uint32` + the
     ByteArray pointer) and wrap in `lean_io_result_mk_ok`.
- Borrow contract preserved: both `mangled` and `args` are
  `@&` on the Lean side / `b_lean_obj_arg` on the C side,
  so the shim does not call `lean_dec` on them. The freshly
  allocated `ret_array` and boxed status enter the tuple,
  which the caller drops on its own schedule.
- The `status` field surfaces dispatcher / worker failures
  to the typed Lean wrapper (the wrapper raises
  `IO.userError` on non-zero). The Lean IO error path is
  reserved for cases the shim itself cannot reach (none
  today).

Verification: `leanc -c shim/leo4_rust_bridge_lean.c -o … -std=c2x`
produces a clean ELF relocatable with `T leo4_rust_call_lean`
visible to a follow-on link step. (A standalone link into a
`.so` strips the symbol via `--gc-sections` since the test
link has no Lean-side reference; the production link path
preserves it because the `@[extern]` declaration is a real
caller.)

Build orchestration (Lake plugin auto-discovery of the
glue-shim source + leanc invocation) is the natural home for
a 9-6 follow-up. The user-facing
workflow on 9-6 today is:

```sh
cargo build --release                            # cdylib
leo4-rust-emit --cdylib … --out-dir … --emit-lean  # 9-2/9-5
cargo build --release -p leo4-rust-bridge        # 9-4a/b
leanc -c shim/leo4_rust_bridge_lean.c -o …       # 9-6
# … leanc-link the user's Lean wrapper with the two .o /
# .a above; SPEC/reverse-direction.md §7 step 4.
```

No code in the rest of the workspace changes; existing tests
stay at 134/0.

### Added — Phase 9-5: Lean wrapper module emission (`leo4-rust-emit --emit-lean`) (2026-05-23)

Sixth code landing on the Phase 9 ladder. `leo4-rust-emit`
grows an `--emit-lean` flag that, alongside the existing
`.leo4-rust-exports.idl` + `.leo4-rust-handshake` pair,
generates `<pkg>.leo4-rust-imports.lean` — a Lean wrapper
module exposing one typed `IO α` action per
`#[leo4::export]`.

Generated module anatomy:

```lean
import Leo4
namespace <module>           -- defaults to `<iface>.Rust`

def schemaHash : String := "<13-char base32lc>"

@[extern "leo4_rust_call_lean"]
private opaque leo4RustCallRaw
    (mangled : @& String) (args : @& ByteArray)
    : BaseIO (UInt32 × ByteArray)

def add (a0 : UInt64) (a1 : UInt64) : IO UInt64 := do
  let mut args := ByteArray.empty
  args := Leo4.LeanMarshal.canonicalEncode a0 args
  args := Leo4.LeanMarshal.canonicalEncode a1 args
  let (status, ret) ← leo4RustCallRaw "leo4_rust__add__u64_u64" args
  if status ≠ 0 then throw (IO.userError s!"…")
  match Leo4.LeanMarshal.canonicalDecode (T := UInt64) ret 0 with
  | .ok (v, _) => return v
  | .error e   => throw (IO.userError s!"… {e.detail}")

end <module>
```

Implementation in `crates/leo4-rust-emit/`:

- CLI gains `--emit-lean` (bool) and `--lean-module <name>`
  (defaults to `<iface>.Rust`).
- `render_lean_wrapper` emits the file header, schema-hash
  pin, single `@[extern]` raw entry, and one typed wrapper
  per export.
- `render_one_export` walks parameter / return mangles via
  `lean_type_of_mangle`. v9-5 scope: scalars (u8..u64, i8..i64,
  f32/f64, bool, char), `String`, `Nat` / `Int` (bignat /
  bigint), `Option<T>` of mapped inner, and the
  `list<u8>` → `ByteArray` / `list<T>` → `Array T` shapes.
  Composite payloads (records / variants / resources) are
  deferred — an export whose signature contains an unmapped
  mangle still gets a wrapper definition emitted, but its
  body is a Lean `panic!` so the user sees a clear runtime
  diagnostic rather than a silent ABI mismatch.
- `lean_safe_ident` guards a small keyword list (`def`,
  `match`, `end`, …); other identifiers pass through.
- 7 new unit tests cover the mangle-to-Lean-type table, list
  / option / ByteArray dispatch, keyword renaming, and
  end-to-end module rendering.

Smoke verified outside the workspace: a fixture cdylib with
four exports (`add`/`echo`/`negate`/`list_sum`) round-trips
through `leo4-rust-emit --emit-lean` into a clean Lean module
with `UInt64`, `Int32`, `String`, and `Array UInt32`
signatures in the right places, plus the cdylib's
schema_hash baked into `schemaHash`.

The `leo4_rust_call_lean` extern symbol the wrapper refers to
lives in a small leanc-compiled Lean-side glue shim Phase 9-6
will land. Until then user code compiling against the
generated `.lean` will fail at *link* time, not at emit time.

Workspace test count 127 → 134; all green.

### Added — Phase 9-4b: POSIX backend (`posix_spawn` + socketpair + waitpid) (2026-05-21)

Fifth code landing on the Phase 9 ladder. Fills the
`leo4_worker_ops_t` table's POSIX branch — the dispatcher body
itself does not change.

Same `shim/leo4_rust_bridge.c` translation unit; the POSIX
backend lives behind `#if defined(__unix__) || defined(__APPLE__)`
next to the stub backend. Workflow:

1. `socketpair(AF_UNIX, SOCK_STREAM, 0, sv)` — parent retains
   `sv[0]` (with `FD_CLOEXEC`), child receives `sv[1]` dup'd onto
   `fd 3` via `posix_spawn_file_actions_adddup2`.
2. `posix_spawnp` (or `posix_spawn` when `LEO4_RUST_WORKER_BIN` is
   an absolute path) launches `leo4-rust-worker --cdylib <path>
   --ipc-fd 3`. Worker binary resolution: env override first,
   `posix_spawnp` PATH search otherwise. `ENOENT` from spawn
   maps to `LEO4_ERR_RUST_CDYLIB_NOT_FOUND` (0x00020004) so
   callers can distinguish missing worker vs other failures.
3. `posix_send_all` / `posix_recv_exact` loop over `write` /
   `read` with `EINTR` retry and short-read detection. Short
   read / EOF mid-message → `LEO4_ERR_RUST_IPC_FAILED`
   (0x00020006).
4. `posix_alive_worker` calls `waitpid(.., WNOHANG)`; reaped
   PIDs are zeroed so the dispatcher can lazily respawn (a
   future Phase 9.X recycle / persistent-worker-restart will
   build on top of this).
5. `posix_reap_worker` does the blocking `waitpid` + `close` on
   shutdown.

Source-level changes:

- `_GNU_SOURCE` / `_DARWIN_C_SOURCE` defined before any `#include`
  so the POSIX symbols (`kill`, `posix_spawn`, `waitpid`, ...) are
  visible under strict `-std=c17` / `-std=c2x` modes.
- `<stdio.h>` added for `snprintf` in error-message formatting.
- `__attribute__((unused))` on `leo4_stub_ops` so the POSIX build
  doesn't warn about an unused fallback ops table (the stub stays
  in the file as the unconditional safety net).
- `extern char** environ;` declared at file scope so `posix_spawn`
  inherits the parent's environment — relevant for forwarding
  `LEO4_RUST_HANDSHAKE_PKG` / `_IFACE` / etc to the worker.

Backend selection chain in this same file: unix/macOS now picks
`&leo4_posix_ops` instead of the stub.

The existing `leo4-rust-bridge` Rust sanity test is updated:
it removes any `LEO4_RUST_WORKER_BIN` / `LEO4_RUST_CDYLIB`
overrides and asserts that the dispatcher cleanly returns one
of the `0x0002_xxxx` reverse-direction error codes. With no
worker binary on PATH and no cdylib it surfaces
`_CDYLIB_NOT_FOUND` (ENOENT) or `_IPC_FAILED` (worker crashed
on the bogus cdylib). The point is that the dispatcher links,
executes the full POSIX `posix_spawn` + `socketpair` +
`waitpid` codepath, and errors gracefully — not that it
succeeds without a real cdylib.

Workspace test count stays at 127 (the test count didn't grow
but the test now covers more code). End-to-end (dispatcher →
worker → cdylib → wrapper) integration with a real fixture
cdylib + `cargo build`-linked worker binary lands when the
Lake wrapper (9-5) gives us a natural cite for the
`LEO4_RUST_HANDSHAKE_PKG/_IFACE` env injection.

### Added — Phase 9-4a: dispatcher skeleton + `leo4_worker_ops_t` + stub backend (2026-05-21)

Fourth code landing on the Phase 9 ladder. The single-entry C
dispatcher that the Lean side will reach via
`@[extern "leo4_rust_call"]`. POSIX and Windows backends
(9-4b / 9-4c) land later and just fill the ops table — the
dispatcher body does not change.

- `shim/leo4_rust_bridge.c` (new, single C TU, ~330 lines):
  - `leo4_worker_ops_t` ops-table interface
    (spawn / kill / reap / send / recv / alive) per SPEC
    `reverse-direction.md` §4.4.
  - **Stub backend** (`leo4_stub_ops`) wired unconditionally
    on every platform; every op errors with
    `LEO4_ERR_RUST_SPAWN_FAILED` / `LEO4_ERR_RUST_IPC_FAILED`.
    Compile-time backend selection chain
    (`__unix__ || __APPLE__` → stub today, POSIX in 9-4b;
    `_WIN32` → stub today, Windows in 9-4c; else → stub).
    Ensures `libleo4_rust_bridge.a` links on every platform
    from day 1.
  - Worker handle slot via `_Atomic` (lazy spawn,
    compare-exchange guard). Single-Lean-thread invariant
    means the spin path is unreachable today; the guard is
    defensive for future multi-Lean models.
  - Dispatcher body `leo4_rust_call(mangled, ..., args, ...,
    ret, ret_cap, ret_len)`:
    1. `leo4_get_or_spawn_persistent` (ops `spawn`).
    2. `leo4_send_request` builds the SPEC §5.1 request frame
       (LE u32 magic / mangled_len / args_len + payload).
    3. `leo4_recv_response` parses the SPEC §5.2 response
       frame (magic + i32 status + u32 ret_len + u32
       detail_len + payload + detail), validates magic +
       sizes, propagates `LEO4_ERR_BUFFER_TOO_SMALL` with
       `*ret_len` = required size on caller-too-small.
  - cdylib path resolution: env `LEO4_RUST_CDYLIB` only
    (SPEC §9's full chain lands when 9-5 bakes a
    compile-time fallback).
  - Single export visibility macro: `__declspec(dllexport)`
    on Windows, `__attribute__((visibility("default")))`
    elsewhere. C17 baseline.
  - Debug helper `leo4_rust_bridge_current_ops()` exposed for
    tests so Rust can identify which backend got selected.
- `crates/leo4-rust-bridge/` (new workspace member):
  - `[lib] crate-type = ["staticlib", "rlib"]` so Lake can
    pick up the `.a` archive and the workspace can
    `cargo test` against the dispatcher.
  - `build.rs` drives `cc::Build::new().file(c_source)
    .std("c17").compile("leo4_rust_bridge")`. cc crate added
    to `workspace.dependencies`.
  - `src/lib.rs` declares the `leo4_rust_call` extern and a
    sanity test verifying that the dispatcher links and
    returns `LEO4_ERR_RUST_SPAWN_FAILED = 0x00020003` against
    the stub backend.

Workspace test count 126 → 127; all green. `libleo4_rust_bridge.a`
produced at `target/<profile>/libleo4_rust_bridge.a`.

End-to-end (dispatcher → worker → cdylib → wrapper) integration
arrives with the POSIX backend in 9-4b: only that fill-in
exchanges the stub `spawn`/`send`/`recv` for `posix_spawn` +
`socketpair` + `wait4`.

### Added — Phase 9-3: `leo4-rust-worker` harness binary (2026-05-21)

Third code landing on the Phase 9 ladder. The harness binary
the dispatcher (Phase 9-4) spawns once per cdylib — opens the
cdylib, performs the handshake, then runs the canonical-ABI
request loop serially via dlsym + catch_unwind.

- `crates/leo4-rust-worker/` (new workspace member, binary).
  CLI: `leo4-rust-worker --cdylib <path> --ipc-fd <N>` on POSIX
  (or `--ipc-pipe <name>` on Windows; pipe path lights up with
  9-4c).
- Boot sequence:
  1. `dlopen` the cdylib via `libloading`.
  2. Resolve `leo4_rust_describe_exports`, walk the `EXPORTS`
     slice, and copy entries out (so the cdylib lifetime can be
     decoupled from the request loop if a future worker mode
     needs it).
  3. Compute the schema_hash via the same FNV-1a-64 + base32lc
     algorithm `leo4-rust-emit` uses (pkg / iface from
     `LEO4_RUST_HANDSHAKE_PKG` / `_IFACE` env vars the
     dispatcher will set from the handshake JSON; sensible
     `rust` / `Rust` fallbacks for ad-hoc runs).
  4. Send a handshake frame (`SPEC/reverse-direction.md §5.3`)
     containing magic + hash_len + abi_version + 13-byte hash.
- Request loop (`SPEC §5.1 / §5.2`):
  - Read a request frame (magic + mangled_len + args_len +
    payload). magic=0 ⇒ graceful shutdown; bad magic ⇒ error;
    payload >256 MiB ⇒ error.
  - Resolve the mangled symbol via `dlsym` (cached by mangled
    name; cache miss returns `LEO4_ERR_RUST_DLSYM_FAILED`).
  - Call the wrapper with a shared response buffer; on
    `LEO4_ERR_BUFFER_TOO_SMALL` (7) grow the buffer to the
    required size (reported by the wrapper via `*ret_len`)
    and retry.
  - Wrap the call in `catch_unwind`. The wrapper itself (Phase
    9-1) already catches panics inside the user fn; this outer
    guard is a safety net for the wrapper plumbing. On panic
    the worker `process::abort`s — the dispatcher sees IPC EOF
    and respawns lazily.
  - Write a response frame (magic + status + ret_len +
    detail_len + payload + detail).
- IPC backend: POSIX `UnixStream::from_raw_fd(fd)` reading the
  inherited fd from the dispatcher. Windows named-pipe support
  defers to 9-4c (stub error message until then).
- 6 unit tests cover the wire format (handshake/request/response
  encode + decode), magic=0 shutdown handling, `surface_form`
  mangle-to-IDL inverse on the worker side, and the FNV
  digest's 13-char base32lc output. Workspace test count 120 →
  126; all green.

End-to-end (dispatcher → worker → cdylib → wrapper) integration
naturally lives in 9-4a (dispatcher) commit — `socketpair` +
fd-inherit semantics are dispatcher-side and the wire round-trip
is best verified there.

### Added — Phase 9-2: `leo4-rust-emit` CLI + `leo4-build::wire_rust_exports` (2026-05-21)

Second code landing on the Phase 9 ladder. The emit pass that
turns a built user cdylib into the `.leo4-rust-exports.idl` +
`.leo4-rust-handshake` pair Lake will consume in 9-5.

- `crates/leo4-abi/src/rust_exports.rs` gains:
  - `#[repr(C)]` on `ExportEntry` (was implicit Rust repr;
    external tooling needs a stable field layout to walk the
    slice via `dlopen`).
  - A new `extern "C"` entry,
    `leo4_rust_describe_exports(out_ptr, out_len) -> i32`,
    that writes the in-process `EXPORTS` slice pointer + length
    into caller-provided out-params. This is the stable
    FFI gateway for in-process introspection; downstream tools
    do not need to resolve `linkme`'s private internals.
- `crates/leo4-rust-emit/` (new workspace member, binary). CLI:
  `leo4-rust-emit --cdylib <path> --out-dir <dir> [--pkg <name>]
                  [--iface <name>] [--rust-toolchain <str>]`.
  `dlopen`s the cdylib via `libloading`, calls
  `leo4_rust_describe_exports`, copies each entry out (so the
  cdylib can be unloaded immediately), computes the canonical
  IDL form + FNV-1a-64 schema_hash (13-char base32lc, matching
  the forward direction's algorithm), and atomically writes:
  - `<pkg>.leo4-rust-exports.idl` — pretty canonical IDL,
    functions sorted by mangled name.
  - `<pkg>.leo4-rust-handshake` — JSON containing schema_hash,
    abi_version, package, interface, cdylib_path (absolute),
    rust_toolchain, leo4_rust_emit_version, emitted_at (UTC
    RFC 3339), and an exports table mirroring the slice. 6
    unit tests; standalone cdylib smoke verified 4 exports
    (`add(u64,u64)→u64`, `echo(string)→string` (isolated),
    `pi()→f64`, `list_len(list<u32>)→u64`) round-trip into
    both files with schema_hash `7i2wz2k5rqhls`.
- `crates/leo4-build/` gains `wire_rust_exports(out_dir)` for
  Lean-side consumers' `build.rs`. Locates the
  `.leo4-rust-handshake` file in `out_dir`, emits
  `cargo:rustc-env=LEO4_RUST_HANDSHAKE_FILE=…` and
  `cargo:rustc-env=LEO4_RUST_EXPORTS_IDL_FILE=…`, plus
  `cargo:rerun-if-changed=` for both and
  `cargo:rerun-if-env-changed=LEO4_RUST_CDYLIB`.
- `SPEC/reverse-direction.md` §7 + §8 rewritten to match: the
  Rust cdylib is built first; `leo4-rust-emit` is a separate
  CLI step (not a `build.rs` action) that produces the
  metadata files; Lake then reads them in 9-5. The handshake
  JSON example reflects the actual `leo4-rust-emit 0.1.0`
  output (collapsed canonical form, sort by mangled name,
  per-export fields).

Schema-hash detection between cdylib and handshake is deferred
to the worker harness (9-3): the worker recomputes the hash at
init time and the dispatcher surfaces `LEO4_ERR_HANDSHAKE_MISMATCH`
(0x05) on the first call.

### Added — Phase 9-1: `#[leo4::export]` attribute proc-macro (2026-05-21)

First code landing for Phase 9. Tagged Rust functions become
callable through the reverse-direction pipeline once the rest
of Phase 9 ships.

- `crates/leo4-abi/src/rust_exports.rs` (new, behind a new
  `rust-exports` cargo feature) — `ExportEntry` struct + the
  `EXPORTS` distributed slice (`linkme::distributed_slice`).
  Stays off by default so stable workspace builds remain free of
  the `linkme` dependency.
- `crates/leo4-macros-backend` — `expand_export` function +
  `ExportAttrs` parsing (`isolated` recognised; recycle / panic
  options deferred). The expansion emits:
  - The original user `fn` unchanged.
  - An `#[unsafe(no_mangle)] pub unsafe extern "C" fn
    leo4_rust__<fname>__<param_mangles>(args, args_len, ret,
    ret_cap, ret_len) -> i32` wrapper that does canonical-ABI
    decode → `catch_unwind`(user-fn) → canonical-ABI encode.
    `LEO4_ERR_RUST_PANIC = 0x00020001` surfaces on panic.
  - A `linkme` distributed-slice registration writing one
    `ExportEntry` into `EXPORTS`.
- `crates/leo4-macros` — `#[proc_macro_attribute] pub fn export`.
- `crates/leo4` — `rust-exports` feature pass-through + a new
  `__private` module that re-exports `linkme`, `ExportEntry`, and
  `EXPORTS` so user cdylibs need only depend on `leo4`. The macro
  emits paths through `::leo4::__private::*`.
- Unit tests in `leo4-macros-backend` (4 new) cover the attr
  parser (`isolated` / default / unknown rejection), a scalar
  smoke expansion, and the `async fn` / unsupported-type
  diagnostics. Workspace test count 110 → 114; all green.
- Standalone smoke cdylib verified: three `#[leo4::export]`s
  (`add(u64, u64) -> u64`, `#[leo4::export(isolated)] echo(String)
  -> String`, `no_args() -> u32`) compile and the wrapper
  symbols (`leo4_rust__add__u64_u64`, `leo4_rust__echo__str`,
  `leo4_rust__no_args`) show up in the produced `.so`.

The dispatcher (`libleo4_rust_bridge.a`) and the worker harness
that will reach these symbols land in 9-3 / 9-4a-c. Phase 9-2
(build script emitting `<pkg>.leo4-rust-exports.idl` +
`<pkg>.leo4-rust-handshake` from the `EXPORTS` slice) is the
next substep.

### Changed — Tier 2 Windows: clarify C ↔ Rust ABI compatibility (2026-05-21)

Follow-up on the gnullvm target adoption. Per the rustc
platform-support docs for `x86_64-pc-windows-gnullvm`, Rust
binaries on that target are ABI-compatible with C code built by
**clang** targeting either `*-pc-windows-gnu` or
`*-pc-windows-gnullvm` — i.e. mingw-w64's `gcc` is NOT sufficient,
the C compiler must be LLVM-based. leo4 enforces the LLVM
track end-to-end: the forward shim and the reverse-direction
dispatcher are both compiled through `leanc` (which on Windows
wraps clang); user cdylibs are gnullvm Rust. Documented in
`LEO4-DESIGN.md §9.1`, `OS-PORTABILITY.md §1`, and
`SPEC/reverse-direction.md` §11.

### Changed — Tier 2 Windows target: `*-pc-windows-msvc` → `*-pc-windows-gnullvm` (2026-05-21)

leo4 targets the gnullvm Windows triple (clang + lld + UCRT)
rather than MSVC. Practical consequences:

- The forward shim's clang/gcc-style C
  (`__attribute__((visibility("default")))`, `__builtin_memcpy`,
  gcc command-line flags) compiles on Windows unmodified — no
  MSVC-intrinsic fork in the emitter.
- The C-standard baseline (`-std=c17` / optional `-std=c2x`)
  is uniform across every tier via clang/leanc.
- Users skip the Visual Studio dependency; the only
  Windows-specific prerequisite is `rustup target add
  x86_64-pc-windows-gnullvm`.
- The OS-PORTABILITY audit's "C compiler visibility" and
  "gcc/clang command-line flags" rows move from "needs layer"
  to "resolved by Tier 2 target choice". Spawn / IPC / dynamic
  loading / DLL search remain genuinely Windows-specific and
  stay behind their respective abstraction layers.

Updated: `LEO4-DESIGN.md §9.1` (platform tier table + rationale),
`README.md` (tier table), `OS-PORTABILITY.md` (policy +
audit), `SPEC/reverse-direction.md` §4.4 + §11 (Windows
backend target + uniform `-std` invocation).

### Added — OS-portability policy + audit ledger (2026-05-21)

`OS-PORTABILITY.md` (new) — leo4-wide policy: OS-specific code
must be confined to identified abstraction layers; new
`#[cfg(target_os=…)]` / `cfg(unix)` / `cfg(windows)` branches
outside an identified layer get lifted into a layer in the same
commit or rejected. Document seeds the audit ledger with the
current OS branches (Linux `.so` extension hardcode,
`-Wl,-rpath` link line, gcc/clang `__attribute__((visibility))`
in shim source, ...) and recommends a layer per concern.

The Phase 9 spawn / IPC abstraction (`SPEC/reverse-direction.md`
§4.4) is the first formally-specified instance of this policy
and the model for follow-on layers.

`CLAUDE.md` "Cross-cutting" section gains a one-line pointer to
the policy.

### Changed — Phase 9-0 follow-up: spawn / IPC abstraction (2026-05-21)

`SPEC/reverse-direction.md` §4.4 (new sub-section) formalises a
`leo4_worker_ops_t` ops-table that the dispatcher uses for all
worker spawn, IPC, and reaping syscalls. Three backend slots:
**stub** (always errors with `LEO4_ERR_RUST_SPAWN_FAILED`;
ensures `libleo4_rust_bridge.a` links on every platform from
day 1), **POSIX** (`posix_spawn` + `socketpair` + `wait4`), and
**Windows** (`CreateProcess` + named pipe +
`WaitForSingleObject`). All three live in the same single C
translation unit, gated by `#ifdef`.

`ROADMAP.md` Phase 9 substep 9-4 is split accordingly:

- **9-4a** — dispatcher skeleton + `leo4_worker_ops_t` table
  + stub backend.
- **9-4b** — POSIX backend (Tier 1 exit criterion).
- **9-4c** — Windows backend (Tier 2 schedule).

### Added — Phase 9 entry gate: reverse-direction (Rust → Lean) design (2026-05-21)

leo4 grows a second pipeline so Rust functions tagged
`#[leo4::export]` become callable from Lean as ordinary `IO α`
actions. Motivating use case: combining a Rust-implemented SMT
solver with Lean's proof tooling, where incremental
`push/pop`-style state must persist across calls.

Design only — no code yet. Implementation lands in substeps
9-1 through 9-8 (see `ROADMAP.md`).

- `SPEC/reverse-direction.md` (new) — normative SPEC covering
  mangling prefix `leo4_rust__`, dispatcher API
  (`leo4_rust_call`), long-running worker process lifecycle
  with opt-in `#[leo4::export(isolated)]` per-call fresh
  workers, IPC wire format, build orchestration, handshake
  file format, cdylib path resolution (env → handshake →
  sibling search, mirroring `LEO4_SHIM_SO`), and the C
  standard policy (C17/C18 baseline, C23 features optional).
- `SPEC/canonical-abi.md` §13 — extends the error-code table
  with the `0x0002_0000..0x0002_FFFF` Rust-worker passthrough
  range: `LEO4_ERR_RUST_PANIC` (0x00020001),
  `LEO4_ERR_RUST_WORKER_RESTARTED` (0x00020002),
  `LEO4_ERR_RUST_SPAWN_FAILED` (0x00020003),
  `LEO4_ERR_RUST_CDYLIB_NOT_FOUND` (0x00020004),
  `LEO4_ERR_RUST_DLSYM_FAILED` (0x00020005),
  `LEO4_ERR_RUST_IPC_FAILED` (0x00020006).
- `LEO4-DESIGN.md` — D16 adopted; §11 out-of-scope updated to
  cite Phase 9 as the home for the reverse direction.
- `ROADMAP.md` — Phase 9 entry section with the architecture
  diagram, isolation matrix, and substeps 9-0 through 9-8.

Isolation model: long-running worker process per cdylib by
default (preserves user state across calls; SMT-solver friendly).
`#[leo4::export(isolated)]` opts a function into a fresh worker
per call for stronger cross-call isolation. T1 (memory
corruption) / T2 (panic) / T3 (thread leak) are all caught at
the process boundary; what happens inside the worker is the
cdylib's responsibility.

Dispatcher is a single C17 translation unit
(`shim/leo4_rust_bridge.c`, ≈150–250 lines, statically linked
into the Lean executable) that lazily spawns the worker via
`posix_spawn` / `CreateProcess`. The dispatcher API
(`leo4_rust_call(mangled, args, ret)`) is intentionally
isolation-backend-neutral so future variants (zygote-fork, wasm
sandbox) can be swapped in without changing callers.

## [0.1.0] — 2026-05-21

First tagged release. Phases 0–8 complete: full Lean ↔ Rust
round-trip on Tier 1 Linux, cross-impl mangling agreement (70
mangled instantiations, schema_hash `qi5gb74dbjyxo`), mutual
recursion via `mutual { … }` + `Cyc<i>`, `IO α` lifted to
`future<α>`, Mathlib-compatible carrier-type subset with opt-in
bridges, and a Typst documentation suite in four languages.

### Added — Typst documentation suite (2026-05-21)

`docs/learning/{en,ko,ja,de}/main.typ` — short architectural
overview in English / Korean / Japanese / German (~330 lines each).
`docs/implement-from-scratch/{en,ko,ja,de}/main.typ` — long-form
step-by-step build guide following the Phase 0–10 ladder (~1050–1175
lines each). Shared `docs/template/leo4-book.typ` carries cover /
ToC / code-block styling. All eight documents compile with
`typst >= 0.14.2` via `typst compile --root docs <file>`.

### Changed — workspace version bump 0.0.0 → 0.1.0 (2026-05-21)

`workspace.package.version` + the path-dep version fields in
`workspace.dependencies` align with the 0.1.0 release.

### Changed — `cargo check` clean: 17 non_snake_case + 1 dead_code (2026-05-21)

`leo4-macros-backend` emits `#[allow(non_snake_case)]` on every
wrapper `fn` generated from `leo4::import!` — Lean export names are
camelCase by convention and must match byte-for-byte, so consumers
no longer have to suppress lints per-binding. `schema-idl`'s
long-dead `RawDecl::fqn()` helper deleted (its sole caller was
inlined into `insert_shape_entries` in 8f27ff1 / Phase 6-2). Plus a
small `leo4-abi` test cleanup (unused `LeanMarshal as _` import).

### Added — Mathlib bridge infrastructure + initial bridges (2026-05-21)

User direction recorded 2026-05-20 (memory:
`project_mathlib_bridge.md`): every `Lean*` carrier type ships with
opt-in `Leo4.MathlibBridge.*` modules providing 1-to-1 conversions
to/from Mathlib types. leo4 core stays Mathlib-independent
(ROADMAP §8).

Infrastructure (`sibling/mathlib-bridge-test/`):
- New non-workspace Lake package pulling `Leo4` (path) + `mathlib`
  (git, `leanprover-community/mathlib4`). Lean toolchain pinned
  to `v4.29.1` matching the rest of the repo.
- `MathlibBridgeTest.lean` imports every `Leo4.MathlibBridge.*`
  module + Mathlib core; `decide`-based smoke checks where
  feasible.
- `just mathlib-bridge-test` recipe drives `lake build` in the
  sibling. NOT on the default `just test` ladder — Mathlib's
  cold build is 1-2 hours.
- `sibling/README.md` documents the cold-build caveat.

Bridge modules (`lake/Leo4/Leo4/MathlibBridge/*`) — opt-in import,
NOT auto-imported by `Leo4`; Lake's import-driven build keeps
them out of the main `Leo4` lib's compile graph:

- **`MathlibBridge.Wide`** —
  `LeanU128 ↔ Nat / BitVec 128`, `LeanI128 ↔ Int / BitVec 128`.
  Wide → Nat is total; Nat → Wide truncates ≥ 2^128. Wide → Int
  applies two's-complement sign; Int → Wide wraps mod 2^128.
  Wide ↔ BitVec 128 is a bit-level bijection.
- **`MathlibBridge.Complex`** — `LeanComplexF{32,64}x2 →
  Complex ℝ` via `Float.toReal` (Float32 → Float → ℝ for the
  F32 carrier). Forward direction only — reverse is
  rounding-lossy and waits for a concrete rounding-mode decision.

Follow-ups landed (2026-05-21):

- **`MathlibBridge.NightlyFloats`** — `LeanF16` / `LeanBF16` /
  `LeanF128` and the three complex carriers → `ℝ` / `ℂ` via
  direct IEEE bit-decode arithmetic on `Nat` field extracts
  (sidesteps subnormal-pattern mismatch the bit-widening route
  would have). NaN / Inf map to `0 : ℝ` by convention.
- **Wide bridge ZMod / Fin** — `LeanU128` / `LeanI128` ↔ `ZMod
  (2^128)` (which is `Fin (2^128)` for Mathlib's positive-`n`
  case).
- **`MathlibBridge.Rat`** — `Rat → ℝ` / `Rat → ℂ` total
  embeddings via `Rat.cast`. Lean core `Rat` IS Mathlib `ℚ`, so
  no separate `LeanRat` Lean struct exists; the bridge just
  surfaces the `ℝ`/`ℂ` lifts.

### Changed — Mathlib bridge reverse direction: RTNE pinned + real implementations (2026-05-21)

Rounding mode pinned: **IEEE-754 round-to-nearest-even (RTNE)**.
That's what `Float.div` and the host FPU already use across
platforms (per IEEE-754 §4.3.1); adopting it as the leo4
convention keeps the abstract-Real reverse path consistent with
the round-trip downstream native code already performs.

Three implementation layers per format, replacing the earlier
`noncomputable … := default` stubs:

1. **Float-level narrowing** (`Float.toLean{F16,BF16}RTNE`)
   — manual IEEE bit-level conversion (guard / round / sticky
   bits, mantissa-overflow round-up, special-case Inf / NaN /
   subnormal / overflow / underflow). `Float.toLeanF128` widens
   exactly (binary64 ⊂ binary128).
2. **`Rat`-based** (`Rat.toFloat`, `Rat.toFloat32`,
   `Rat.toLeanF{16,BF16,128}`) — computable, RTNE via Lean
   stdlib's `Float.ofInt` / `.ofNat` + `Float.div` + the
   narrowing helpers above. Precision loss is bounded by `num` /
   `den` conversion (when magnitudes exceed 2^53).
3. **Abstract `ℝ`** (`Real.toFloatRTNE`,
   `LeanF{16,BF16,128}.ofReal`, matching complex `ofComplex`)
   — `noncomputable` via `Classical.choice`. The mathematical
   meaning is RTNE-rounded; the function symbol exists for
   downstream proof references. Runtime use goes through the
   Rat path.

Sibling test fixture exercises both the computable Rat path
(plain `example`) and the noncomputable abstract path
(`noncomputable example`).

Outstanding: subnormal binary64 → binary128 normalisation in
`Float.toLeanF128` (rare boundary, currently returns ±0).

No regression: main `lake/Leo4` lib builds in 12 jobs (bridge
files not in import graph). `just smoke-plugin` + `cargo test
--workspace` + `just mangling-test` all green;
schema_hash `tjhnmfbc7izmk` unchanged.

### Added — Phase 7 step 2b: shim IO unwrap + `asyncDouble` fixture (2026-05-21)

Closes the end-to-end functional path for `IO α` boundary exports.
`asyncDouble(21) = 42` round-trips through the boundary.

Plugin (`lake/Leo4Plugin/Leo4Plugin/Main.lean`):

- `handlerFor`'s `.io T` arm now delegates to `T`'s handler instead
  of returning `none`. The IO wrapping is handled at the shim emit
  layer, not in the handler chain.
- `analyzeExport` switches from `forallTelescopeReducing` to
  `forallTelescope` (no reducing) so the original `IO α` shape
  survives — `…Reducing` would unfold `IO α = IO.RealWorld →
  EStateM.Result …` and expose a spurious `IO.RealWorld` (=Void)
  parameter.
- `renderOneShim` detects `effectIsIO` (return type is `.io _`)
  and emits an IO unwrap block before the encode:

      lean_object *_io_res = leo4_lean__...(args);
      if (!lean_io_result_is_ok(_io_res)) {
          lean_dec(_io_res); *ret_len = 0;
          return LEO4_ERR_IO_FAILED;
      }
      ReturnType r = <type-specific unbox>(lean_io_result_get_value(_io_res));
      lean_dec(_io_res);

  The unbox is `lean_unbox_uint64` / `lean_unbox_uint32` /
  `lean_unbox` for u8 / u16 — wider types route through their
  matching helper. Float / signed / boxed-object variants are
  flagged with `/* TODO step 2c */` comments and pick up
  whenever a fixture demands them.
- New SPEC §13 error code `LEO4_ERR_IO_FAILED = 0x00010001`
  (Lean panic / IO failure passthrough range).

Sample (`tests/sample-lean/Sample.lean`):

- `def asyncDouble (n : UInt64) : IO UInt64 := return n * 2`
  fixture exercises the full path.

Example (`examples/01-hello/src/main.rs`):

- `sample::asyncDouble(&lean, 21)` returns `42`; assertion holds.

Schema hash rotates: `fbla3xr3fsp6g` → `tjhnmfbc7izmk`.
Cross-impl mangling: **67 mangled names byte-identical**.

Still TODO (step 2c, deferred): typed unbox for non-`uintN_t`
return widths (signed integers via sign cast through size_t,
floats via `lean_unbox_float` / `lean_unbox_float32`, etc.). The
existing fallback path compiles but encodes garbage — the
`/* TODO */` comments guard it.

### Added — Phase 7 step 2 main (Lean side): `IO T` → `future<T>` IDL lift (2026-05-20)

Plumbing-only landing. Lean plugin now recognises `IO T` in
boundary positions and the canonical IDL renders it as `future<T>`
instead of `io<T>` (D-i 2026-05-19's effect marker). Cross-impl
already symmetric — Rust `schema-idl` step 1 (2026-05-20)
desugars `future<T>` into `FuncDecl { effect: Async, ret: T }`.

- `lake/Leo4Plugin/Leo4Plugin/AdmitSet.lean` —
  `exprToIDLSubst` gains `IO T → IDLType.io T` mapping (previously
  unrecognised, would fall through to the user-inductive walk).
- `lake/Leo4Plugin/Leo4Plugin/Mangling.lean` — `idlForm` renders
  `IDLType.io t` as `future<t>` instead of `io<t>`. The wire
  mangle (`I_t_i`) is unchanged for byte-identical cross-impl
  conformance.

What remains for step 2b: the shim still doesn't unwrap
`lean_io_result` for IO-returning exports — the Lean wrapper for
`IO α` returns a `lean_io_result α` wrapper that the shim needs
to unbox via `lean_io_result_get_value` before encoding the
inner `α`. Today no sample fixture exercises IO returns, so the
limit is documented but not observable. When step 2b lands, an
`IO α` fixture in `Sample.lean` exercises the full path.

Sibling-project WASIp3 adapter (the `wasip3` crate + `block_on`
wiring under `sibling/leo4-wasip3/`) stays upstream-dependent
and is deferred until the WASIp3 surface stabilises in nightly
Rust + the `wasip3` crate publishes.

Cross-impl mangling harness: **66 mangled names byte-identical**,
schema_hash `fbla3xr3fsp6g` (no rotation — no sample uses IO yet).

### Added — Phase 8 #57: nightly-only float carriers (`nightly-floats` feature) (2026-05-20)

Six new carrier types behind the `nightly-floats` cargo feature on
`leo4-abi` (passed through by `leo4`). Stable workspace builds
unchanged.

- `crates/leo4-abi/Cargo.toml`: new feature `nightly-floats = []`.
  `crates/leo4/Cargo.toml`: pass-through
  `nightly-floats = ["leo4-abi/nightly-floats"]`.
- `crates/leo4-abi/src/lib.rs` gates the nightly module with
  `#![cfg_attr(feature = "nightly-floats", feature(f16, f128))]`
  and `#[cfg(feature = "nightly-floats")] pub mod floats_nightly;`.
- `crates/leo4-abi/src/floats_nightly.rs`:
  - `impl LeanMarshal for f16` (2 B LE, IEEE-754 binary16).
  - `impl LeanMarshal for f128` (16 B LE, IEEE-754 binary128).
  - `pub struct LeanBF16 { bits: u16 }` — brain-float16 bit
    pattern (no native Rust primitive yet); wire 2 B LE.
  - `LeanComplexF16x2 { re: f16, im: f16 }` — 4 B LE.
  - `LeanComplexBF16x2 { re: LeanBF16, im: LeanBF16 }` — 4 B LE.
  - `LeanComplexF128x2 { re: f128, im: f128 }` — 32 B LE.
  - Round-trip unit tests for each (run under nightly only).
- `lake/Leo4/Leo4/NightlyFloats.lean` — opt-in Lean module (NOT
  auto-imported from `Leo4`). Mirrors every Rust type via
  bit-pattern structures:
  - `LeanF16 { bits : UInt16 }`, `LeanBF16 { bits : UInt16 }`.
  - `LeanF128 { lo, hi : UInt64 }` (matches `f128::to_le_bytes()`).
  - Complex variants pairing the singles.

Build verified: `cargo check --workspace` (stable, feature off) and
`lake build` (Lean side, module opt-in) both green. The
`cargo test --features nightly-floats` path lands when the
maintainer's local rustup has a nightly with `feature(f16, f128)`
enabled — until then the feature is documented but not CI-verified.

Cross-impl mangling stays at 66 names byte-identical — no sample
fixture uses the nightly types, deliberately, so schema_hash
doesn't rotate (`fbla3xr3fsp6g`).

### Added — Phase 8 #56: machine-complex carriers `LeanComplexF{32,64}x2` (2026-05-20)

Wire-pair `(re, im)` machine complex on stable Rust. `xN` suffix
follows the convention that extends to `xN=4` (quaternion) /
`xN=8` (octonion) / arbitrary `xN`.

- `lake/Leo4/Leo4/Wide.lean`:
  `structure LeanComplexF32x2 where re, im : Float32 deriving LeanMarshal`
  and matching `LeanComplexF64x2` (`Float = binary64`). `DecidableEq`
  skipped because IEEE-754 NaN ≠ NaN.
- `crates/leo4-abi/src/complex.rs` — `pub struct LeanComplexF32x2 {
  re: f32, im: f32 }` (+ F64 counterpart) with `LeanMarshal` impls
  delegating to the underlying `f32` / `f64` ones. 8 / 16 bytes LE
  on the wire, byte-identical to the Lean record's field-order
  encode.
- `leo4` façade re-exports `LeanComplexF32x2` / `LeanComplexF64x2`.
- Sample: `def mulComplexF64x2 (a b : Leo4.LeanComplexF64x2) :
  Leo4.LeanComplexF64x2 := …` (Karatsuba-style direct formula).
- examples/01-hello: `(2 + 3i) · (4 - i) = 11 + 10i` round-trip.

Schema hash rotates `uj55sds6f7cpq` → `fbla3xr3fsp6g`. Cross-impl
mangling: **66 mangled names byte-identical**. Zero plugin changes
(same record-pair pattern as #55).

User direction recorded for follow-up (#58, deferred): every
`Lean*` carrier type gets opt-in `Leo4.MathlibBridge.*` modules with
1-to-1 `toMathlib/fromMathlib` conversions. leo4 core stays
Mathlib-independent.

### Added — Phase 8 #55: stable 128-bit integers (`u128`, `i128`) (2026-05-20)

Wide-integer carriers via the "Lean record pairs Rust primitive"
pattern. Stable Rust path — no nightly, no feature gate.

- `lake/Leo4/Leo4/Wide.lean` — new module, auto-imported from
  `Leo4`. Defines
  `structure LeanU128 where lo, hi : UInt64 deriving LeanMarshal` and
  matching `LeanI128`. Wire form is `lo (8B LE) + hi (8B LE)` —
  byte-identical to Rust's `u128::to_le_bytes()` /
  `i128::to_le_bytes()`.
- `crates/leo4-abi/src/scalars.rs` — `impl LeanMarshal for u128 /
  i128` directly (no newtype wrapper). 16 wire bytes LE.
- `crates/leo4-macros-backend/src/lib.rs` — `rust_type_to_idl` maps
  bare `u128 → Record { fqn: "Leo4.LeanU128" }` and
  `i128 → Record { fqn: "Leo4.LeanI128" }`. Users write bare
  `u128` in `leo4::import!` and the macro auto-routes.
- Sample: `def addU128 (a b : Leo4.LeanU128) : Leo4.LeanU128` with
  hand-rolled two-limb add + carry (Lean stdlib has no UInt128 ops;
  leo4 stays Mathlib-independent per ROADMAP §8).
- examples/01-hello rt: `sample::addU128(&lean, (1u128 << 64) |
  0xdeadbeefcafebabe, 1)` asserts high limb participates.

Schema hash rotates `2iomjhrbofmos` → `uj55sds6f7cpq`. Cross-impl
mangling: **65 mangled names byte-identical**.

Zero plugin changes — the existing record path handles
`Leo4.LeanU128` natively because its wire is just `record { lo: u64,
hi: u64 }`. Same applies for `LeanI128`. Future carriers (#56, #57)
follow this pattern.

### Added — Phase 5 (2026-05-16 → 2026-05-20)

- **`crates/leo4-native/`** — `Lean::open` (handshake + Lean runtime
  init + wrapper-module init), `Arena<'a>`, `LeanRef<'a, T>`, a
  per-callsite `Mutex<HashMap>` dispatch cache, inline
  `lean_io_result_is_ok` and `lean_dec_ref` (with `lean_dec_ref_cold`
  resolved via `dlsym`).
- **`crates/leo4-macros/`** — `leo4::import! { fn add(a: u64, b: u64) -> u64; }`
  expands to the dispatch wrapper. Scalars (P5-b₁), `String` and
  composite payloads (P5-b₂), nominal user types (P5-b₃-iii), and
  per-fn `#[leo4(args = "…")]` attribute hints for
  multi-instantiation disambiguation (P5-b₃-iv) all flow through
  `LeanMarshal` encode / decode.
- **`#[derive(LeanMarshal)]`** for the four nominal shapes —
  record (struct), enum (all-unit), variant (mixed payload),
  resource (`#[leo4(resource)]` on a single-`u64` struct). Generic
  records (`Pair<A, B>`) get a `T: LeanMarshal` bound synthesised on
  the generated impl. Mirrors `lake/Leo4/Leo4/Deriving.lean`
  byte-identically.
- **`crates/leo4-build/`** — `leo4_build::wire(lake_build_dir)`
  surfaces `LEO4_SHIM_SO` and `LEO4_HANDSHAKE_FILE` as `env!()`
  values for downstream `build.rs` callers and emits the
  matching `cargo:rerun-if-changed=` lines.
- **`crates/leo4/`** — top-level façade re-exporting the loader, the
  derive macro, `import!`, and `encode` / `decode` helpers.
- **Lake plugin shim emitter** — `<pkg>.leo4-shim.c` with one
  `leo4_call_<mangled>` entry point per `@[leo4_export]` ×
  instantiation. `leanc`-driven build of `<pkg>.leo4-shim.so` with
  RPATH-linked `libleanshared` + dep `.so` discovery via
  `lake-manifest.json`'s `packages[].dir` (F4-α).
- **Lake plugin Lean-side ABI conformance** — the emitter now
  agrees with Lean's actual FFI signatures for unboxed types:
  all-nullary inductives get `uint8_t` / `uint16_t` / `uint32_t`
  according to `Lean/Compiler/LCNF/ToImpureType.lean`'s
  `impureTypeForEnum`; `≥ 2³²` ctors short-circuit through the
  `LEO4_ERR_UNIMPLEMENTED` stub. `@[leo4_resource]` single-`UInt64`
  structures pass / return a raw `uint64_t` (not `lean_object *`).
- **Deterministic wrapper-module init symbol** — Lean invoked with
  `--root=<wrapper-dir>` so the export module's `initialize_*`
  symbol is path-independent.
- **`examples/01-hello/`** — `add` / `hello` / `pointSum` /
  `colorName` / `isLeaf` / `parserId` round-trip end-to-end on
  Tier 1 Linux. Includes the handshake-mismatch exit check (mutated
  `schema_hash_bytes` → `LEO4_ERR_HANDSHAKE_MISMATCH` = 5) and the
  attribute-routed `Sample.stringify` pick (P5-b₃-iv).
- **`examples/02-roundtrip/`** — `Sample.echoes (xs : List UInt32) (n : Nat) : List UInt32`
  driving `list<u32>` on both argument and return + `bignat` as an
  argument. Also exercises `listSumU64` / `listConcat` and a
  multi-instantiation `listLen` pick over `list<u32>`. Phase 5
  exit demo for the list + bignat wires.
- **Sample fixture** — `Sample.Pair<α, β>`, `Sample.Either<α, β>`,
  and the four nominal user types (`Point` / `Color` / `Tree` /
  `ParserHandle`).
- **macOS platform tier** — demoted to Tier 3 (best-effort, no CI)
  on 2026-05-20. Code paths remain platform-agnostic; only the
  exit-criteria and CI matrix scope shrunk.

### Added — earlier phases

- Design document (`LEO4-DESIGN.md`) capturing all resolved decisions.
- Working agreement for Claude Code (`CLAUDE.md`).
- Phased roadmap (`ROADMAP.md`).
- Phase 0 spike findings (`spike/SPIKE-0-FINDINGS.md`).
- Normative specifications under `SPEC/`:
  - `idl-grammar.ebnf`
  - `canonical-abi.md`
  - `mangling.md`
  - `handshake.md`
- Monorepo scaffold (Cargo workspace + Lake workspace).
- Lake plugin (`leo4plugin` exe), runtime library (`lake/Leo4/`),
  cross-impl mangling + WIT lowering + canonical-ABI conformance
  harnesses, and the multi-version Lean CI matrix.

### Added — Phase 8 step 2b: external-marshal shim glue, `Rat` round-trips (2026-05-20)

Phase 8 second-step finisher. Closes the cross-boundary wire for
`Rat` and any future external-marshal nominal: the shim now
delegates encode/decode to Lean-emitted C-callable helpers
(`leo4_marshal_<fqnSeg>_dec/_enc`) instead of dissecting the
type's IDL fields.

Concrete plumbing:

- `lake/Leo4Plugin/Leo4Plugin/Main.lean`:
  - `renderLeanExports` now takes `userDecls` and emits a helper
    pair per `UserDecl.externalMarshal`:

      `@[export leo4_marshal_<seg>_dec] def … (buf) (off) : Except _ (T × Nat) := …`
      `@[export leo4_marshal_<seg>_enc] def … (val) (buf) : ByteArray := …`

    These wrap `Leo4.LeanMarshal.canonicalDecode/Encode` so the
    typeclass call gets compiled to a fixed C-callable symbol.
  - `externalMarshalHandler` (new `TyHandler`) generates the
    decode/encode glue for each call site: build a fresh Lean
    `ByteArray` (`lean_alloc_sarray` + `leo4_memcpy` over the
    remaining wire bytes), invoke the helper, unwrap
    `Except _ (T × Nat)` at the right ctor tag (Lean's `Except`
    is `error` = 0, `ok` = 1 — got bitten in dev), advance the
    shim's offset by the consumed-bytes count returned in the
    pair. Encode mirrors with `lean_sarray_cptr` /
    `lean_sarray_size`.
  - `handlerFor`'s `.record` arm now also matches
    `UserDecl.externalMarshal` and routes to the new handler.
  - `renderShimSource` forward-declares every external-marshal
    helper at the top of the shim TU
    (`extern lean_object * leo4_marshal_<seg>_dec/_enc(…);`).

- `tests/sample-lean/Sample.lean`:
  `def addRat (a b : Rat) : Rat := a + b` re-enabled; now
  actually goes through the boundary instead of returning
  `LEO4_ERR_UNIMPLEMENTED`.

- `examples/01-hello/src/main.rs`:
  `sample::addRat(&lean, LeanRat::from_i64_u64(1, 3),
  LeanRat::from_i64_u64(1, 6))` returns
  `LeanRat { num: 1, den: 2 }` — Lean's `mkRat` normalises via
  gcd division.

Schema_hash rotates: `47swds7jpnqre` → `2iomjhrbofmos`. Cross-impl
mangling harness: **64 mangled names byte-identical**
(63 → 64; `addRat` joins). All four examples + workspace
`cargo test` pass.

Closes #54.

### Added — Phase 8 step 2a: UserDecl.ExternalMarshal AST + render (2026-05-20)

ROADMAP Phase 8 second landing (first half). Adds the IDL-level
recognition for types with custom `LeanMarshal` instances whose
fields the plugin can't lower (proof-carrying invariants like
`Rat`'s `den_nz` / `reduced`, opaque wrappers, …). Step 2b will
land the actual C-callable Lean helpers and shim glue so the
boundary round-trips end to end; this commit unblocks the IDL
form so `LeanMarshal Rat` being in scope no longer breaks
parsing / mangling.

- `lake/Leo4Plugin/Leo4Plugin/AdmitSet.lean`:
  `UserDecl.externalMarshal (fqn) (generics)` ctor. `walkUserDecl`
  falls back to `externalMarshal` when a record or variant body
  can't be lowered (any field's `exprToIDLSubst` returns `none`)
  AND the type carries a custom `LeanMarshal` instance.
- `lake/Leo4Plugin/Leo4Plugin/Mangling.lean`: `userDeclToIDL`
  renders `external <fqn>[<generics>]`. `declBand` puts the decl
  in the value band (0).
- Rust schema-idl mirror: `IDLType` reference layer unchanged
  (`func f(_0: Rat)` lowers to `IDLType::Record { fqn: "Rat" }`
  same as a regular record). The distinction lives at the
  `UserDecl` level; the shim emitter dispatches on it. New
  variant `UserDecl::ExternalMarshal { fqn, generics }` and
  matching `RawDecl::ExternalMarshal`.
- `SPEC/idl-grammar.ebnf` implicitly extended via the existing
  `nominal_decl` umbrella; parser recognises the `external`
  keyword in `parse_nominal_decl`, top-level docs, and
  interface bodies.
- `lake/Leo4/Leo4.lean` re-enables auto-import of
  `Leo4.MathlibSubset` (was opt-in in the prior commit because
  `Rat` exposure broke `walkUserDecl`; the externalMarshal
  fallback now closes that gap).
- WIT lowering routes external-marshal decls to opaque
  `resource <name>;` — wasm Component Model consumers see them
  as host-managed handles; the Lean-side custom marshal isn't
  visible across the wasm boundary.

Schema_hash rotates: `5wlbxlzagwuyy` → `47swds7jpnqre`. Cross-impl
mangling harness stays green at **63 mangled names byte-identical**
(`Sample.stringify` instantiation list now includes `Rat`).

Boundary round-trip for `Rat` still returns `LEO4_ERR_UNIMPLEMENTED`
— step 2b lands the actual wire-up.

### Added — Phase 7 step 1: future / stream effect desugar (2026-05-20)

ROADMAP Phase 7 first landing (parser-side, stable Rust, no
WASIp3 dep). `crates/schema-idl/src/parse.rs::parse_func_decl`
recognises `future<T>` / `stream<T>` at the immediate return
position of a `func_decl` and desugars them into
`FuncDecl { effect: Async / Stream, ret: T }`. The effect slot was
already in the AST (`FuncDecl.effect`, D-i 2026-05-19); this
landing wires the parser to populate it.

Rejection: `future<T>` / `stream<T>` anywhere *except* a func's
return position (inside `list<…>`, record fields, variant
payloads, etc.) raises a `ParseError::Expected` with a clear
message. `parse_type` checks the keyword and bails before
attempting to treat it as a nominal.

Renderer (`render::render_canonical`) re-wraps `effect = Async`
ret types in `future<…>` and `effect = Stream` in `stream<…>` so
`parse → resolve → render` round-trips. Schema-hash is unaffected
for existing (Sync-effect) funcs.

Four unit tests pin the desugar happy path (`future<u64>`,
`stream<u8>`), the in-payload rejection (`list<future<u32>>`),
and the in-record-field rejection (`record { y: stream<u32> }`).
schema-idl test count: 53 → 57.

The Lean-side mirror (plugin recognises `future<T>` / `stream<T>`
patterns in user exports — e.g., wrapping `IO T` exports as Async
effect) and the matching shim emit / macro expansion path are
Phase 7 step 2 (next).

### Added — WASIp3 sibling project skeleton (2026-05-20)

`sibling/leo4-wasip3/` — Cargo project **outside** the main leo4
workspace, pinning `nightly` Rust via its own
`rust-toolchain.toml` and targeting `wasm32-wasip3`. The skeleton:

- `Cargo.toml` declares the nightly toolchain + path-dep on
  `crates/leo4-abi` (the canonical-ABI marshalling layer is
  shared between native and wasm; only dispatch / loader differs).
- `src/lib.rs` carries a placeholder `pub struct Lean` mirroring
  the planned `leo4_native::Lean` surface so downstream code can
  `use leo4_wasip3::Lean` interchangeably under wasm.
- `sibling/README.md` documents the convention for non-workspace
  projects + the planned `leo4-wasm64/` sibling deferred until
  upstream stabilisation.

The WASIp3 API design crystallised in the same session: **sync API
on both targets** — the wasm side uses `futures::executor::block_on`
(or `wasmtime_wasi::block_on`) inside its sync wasm export to drive
async sub-tasks. WASIp3 explicitly avoids function coloring, so a
sync wasm export can call async imports. This means the
`leo4::import!` macro emits the same shape on both targets, no
per-target `cfg!` at the call site, and the `async-runtime`
feature stays opt-in for users who want native concurrency.

Phase 7 lands the concrete host import bindings, `block_on` choice,
and `Lean::open` equivalent for wasm; this commit is the
infrastructure shell so the wire-up has a home.

### Added — Phase 8 step 1: `Rat` LeanMarshal infrastructure (2026-05-20)

ROADMAP Phase 8 (Mathlib-compatible subset) first landing. Mirror
implementations on both sides for Lean-core `Rat`
(`Init.Data.Rat.Basic`):

- `lake/Leo4/Leo4/MathlibSubset.lean` — new module under `Leo4`
  carrying `instance : LeanMarshal Rat`. Wire format: `bigint num`
  followed by `bignat den` (SPEC/canonical-abi.md §§5-6). Decode
  uses Lean core's smart constructor `mkRat num den`, which
  normalises (divides by gcd) and degenerates to `0/1` when
  `den == 0`, so malformed wire payloads degrade rather than
  panicking.
- `crates/leo4-abi/src/rat.rs` — new module with `LeanRat { num:
  BigInt, den: BigNat }`. `LeanMarshal` impl delegates to the
  existing `BigInt` / `BigNat` impls so wire format stays
  byte-identical to the Lean side.
- `leo4` façade re-exports `LeanRat` + the `rat` module.
- Two unit tests pin the Rust-side round-trip for the basic cases
  (positive, negative, zero, max-magnitude) and a default-zero
  edge.

**Not yet wired**: cross-boundary calls. The plugin's `walkUserDecl`
rejects `Rat` because of its two `Prop`-typed proof fields
(`den_nz`, `reduced`), which prevents the shim from emitting a
real boundary entry for an `(a b : Rat) → Rat` export. Step 2 of
Phase 8 adds an "external marshal" path to the plugin so types
with custom `LeanMarshal` instances are treated as opaque blobs at
the shim and the boundary just routes bytes through.

### Changed — Variant discriminator 4-byte canonical (2026-05-20)

- Shim emitter (`lake/Leo4Plugin/Leo4Plugin/Main.lean`'s
  `renderVariantHelpers`) and Rust `#[derive(LeanMarshal)]`'s
  `expand_derive_variant` both flip variant disc encode/decode from
  `u8` to `u32 LE`. Matches `SPEC/canonical-abi.md` §9 ("encoders
  MUST emit 4 bytes"); the previous u8 fast path was a SPEC
  violation that both sides shared so the wire still round-tripped.
- Coordinated change — wire format is byte-incompatible with
  pre-2026-05-20 callers. Cross-impl mangling harness re-runs
  unchanged (62 names byte-identical, schema_hash unchanged because
  it covers the IDL canonical form, not the ABI wire).
- `examples/04-mutual-ast` round-trip bytes 12 → 24, matching the
  expected 4-byte-per-disc widening across 3 levels of nesting.

### Added — Plugin value-param erasure (2026-05-20)

- `analyzeExport` recognises implicit value-typed binders
  (`{N : Nat}` and friends) and erases them from the boundary
  signature per SPEC/mangling.md §"Value-param erasure". The
  wrapper renderer fills each erased binder with `default` at the
  Lean call site so elaboration succeeds (binder type must be
  `Inhabited`). `Sample.doubleVal {_N : Nat} (x : UInt32) : UInt32`
  fixture round-trips through `examples/01-hello` as a plain
  `(u32) -> u32` export.

### Added — Phase 6 (2026-05-20)

- **`SPEC/phase-6-mutual.md`** locks the four mutual-recursion
  design decisions: explicit `mutual { … }` block, `Cyc<i>`
  cycle-breaker token (0-based, scoped to the enclosing group),
  group-shared decode-depth counter, and a single `mutual ... end`
  block per cluster in the deriving handler.
- **`SPEC/idl-grammar.ebnf`** grows `mutual_decl` and `cyc_type`.
- **`crates/schema-idl`** — `IDLType::Cyc(u32)` + `UserDecl::Mutual {
  members }` + parser / renderer / mangler / resolver coverage; new
  diagnostics for out-of-scope / out-of-range Cyc and singleton
  groups.
- **`lake/Leo4Plugin`** mirrors the Rust schema-idl side: `IDLType.cyc`,
  `UserDecl.mutual`, `walkMutualGroup` that pre-detects `iv.all`
  clusters and rewrites peer references to `Cyc<i>`; shim emitter
  resolves `Cyc<i>` payloads to peer helper cross-calls
  (`leo4_enc_<peerSuffix>` / `leo4_dec_<peerSuffix>`) with forward
  declarations at the top of each helper block.
- **`lake/Leo4/Leo4/Deriving.lean`** detects a genuine mutual cluster
  (`iv.all` matches across members) and emits one `mutual ... end`
  block carrying every member's `partial def _leo4_encode /
  _leo4_decode` pair, then one `instance` per member. Cross-decl
  payload references route through direct function references
  rather than typeclass-instance forward references.
- **`tests/sample-lean/Sample.lean`** gains a `mutual inductive Expr
  / Stmt end` cluster with two exports (`exprIsLit`, `stmtIsNop`).
- **`examples/04-mutual-ast/`** — Rust mirror of `Sample.Expr` /
  `Sample.Stmt` with hand-rolled `LeanMarshal` impls (`Box<T>` breaks
  the Rust-side cycle). Exit demo for Phase 6.
- **`tests/mangling/`** cross-impl harness picks up the new cluster
  fixture; 61 mangled names + schema_hash `gj6daa3oelheu` are
  byte-identical between the Lake plugin and `leo4c`.

### Open follow-ups
- schema-idl item G (`ConstraintExpr<Atom>` typed AST) — deferred
  until a concrete consumer needs it.
- `wasm64` sibling project — deferred until `wasm64-*` exits Rust
  stable's tier 3.
- Some `LeanError` codes (`0x02` / `0x03` / `0x04` / `0x06` /
  `0x08`) are reserved but not yet fired by a test fixture.

[Unreleased]: https://github.com/Honey-Be/leo4/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Honey-Be/leo4/releases/tag/v0.1.0
