# SPEC — Phase 9: Reverse-Direction Boundary (Rust → Lean Imports)

> Status: **normative design**, drafted 2026-05-21 as part of the
> Phase 9 entry-gate commit (9-0). Implementation lands in
> substeps 9-1 onwards. Until those substeps ship, the
> forward-only model in `LEO4-DESIGN.md §2` applies and no
> `#[leo4::export]` attribute is provided.

This file is the single source of truth for how Rust functions
become callable from Lean through leo4. It complements
`SPEC/mangling.md` (which already covers the mangling rules used
in both directions) and `SPEC/canonical-abi.md` (which covers the
wire format).

The forward direction (`@[leo4_export]` on the Lean side, called
from Rust via `leo4::import!`) is unaffected; this SPEC adds a
second, **independent** pipeline.

## 0. Scope

In-scope (v0, Phase 9 entry):

- A Rust function `pub fn foo(args) -> ret` marked
  `#[leo4::export]` becomes callable from Lean as
  `foo (args) : IO ret` (or pure `ret` if the signature has no
  side effects exposed; v0 surfaces `IO` unconditionally to keep
  the boundary explicit).
- Argument and return types are restricted to whatever
  `LeanMarshal` is implemented for on both sides — scalars,
  `String`, `Vec<T>`, `Option<T>`, `Result<T, E>`, tuples, big
  ints, user-defined `#[derive(LeanMarshal)]` records / enums /
  variants / resources, and the carrier types from Phase 8.
- One worker process per cdylib, long-running by default;
  per-call fresh worker as an opt-in escape hatch.

Out-of-scope (v0):

- Rust function taking a Lean closure / object handle directly
  (function-pointer ABI). The slot for function-arrow mangling
  in `SPEC/mangling.md` stays empty; callbacks are deferred.
- Stronger isolation modes (zygote-fork, wasm sandbox).
  Tracked as Phase 9.X candidates.
- `async fn` exports. Rust async work happens inside the user
  function; the boundary is sync.

## 1. Architecture

```
Lean process
   │
   ├── libleo4_rust_bridge.a       (leo4 ships this; Lake links it statically)
   │     │  single C entry point:
   │     │     int32_t leo4_rust_call(const char* mangled, size_t mangled_len,
   │     │                            const uint8_t* args_ptr, size_t args_len,
   │     │                            uint8_t* ret_ptr, size_t ret_cap,
   │     │                            size_t* ret_len);
   │     │
   │     │  lazy spawn on first call:
   │     ▼
   │  worker process                (one per cdylib; long-running)
   │     │  loads <pkg>.cdylib via dlopen / LoadLibrary
   │     │  handshakes (schema_hash compare) on init
   │     │  receives one request → dispatches via dlsym → replies
```

Three artefacts at runtime:

1. **`libleo4_rust_bridge.a`** — a static C library leo4 ships.
   Lake links it into the Lean executable on every project that
   uses `Leo4`. Defines exactly one external entry point
   (`leo4_rust_call`) plus internal machinery for worker
   lifecycle, IPC, and handshake.
2. **The user cdylib** — built by Cargo. Contains the user's
   `#[leo4::export]`-decorated functions wrapped by per-export
   shims `leo4_rust__<mangled>`. Also exports a compile-time
   constant symbol `LEO4_RUST_SCHEMA_HASH` (13-char base32lc
   string) for handshake.
3. **The worker process** — a tiny harness binary leo4 ships
   alongside `libleo4_rust_bridge.a`. Started by the dispatcher
   on first call. Loads the user cdylib via `dlopen` /
   `LoadLibrary`, then enters a request-loop reading from an IPC
   channel.

The Lean side has zero awareness of any of this. It calls a
generated Lean wrapper that boils down to one `@[extern
"leo4_rust_call"]` declaration plus encode / decode glue.

## 2. Mangling

The mangling rules from `SPEC/mangling.md` §§2–4 apply unchanged.
The only difference is the symbol prefix used at the C linker
level:

| Direction | Shim entry | Lean / Rust helper |
|---|---|---|
| Forward (Lean → Rust calls Lean) | `leo4_call_<body>` | `leo4_lean__<body>` |
| Reverse (Rust → Lean calls Rust) | (none; dispatcher is single-entry) | `leo4_rust__<body>` |

`<body>` is the same string both directions: package, interface,
function name, parameter type mangles, and the schema-hash
suffix. `mangle_type` is identical. The cross-impl conformance
harness (`tests/mangling/`) is the authority — both sides
produce byte-identical `<body>` from the same IDL.

The reverse direction does not need its own shim entry symbol
because the dispatcher reaches the worker via IPC, not via
direct C linkage. The `leo4_rust__<body>` symbol is what the
worker resolves with `dlsym` after loading the user cdylib.

## 3. Dispatcher API

`libleo4_rust_bridge.a` exposes exactly one C function:

```c
/* Defined in shim/leo4_rust_bridge.c. */
int32_t leo4_rust_call(
    const char* mangled,        /* pointer to mangled name (UTF-8) */
    size_t mangled_len,         /* length in bytes, no NUL terminator */
    const uint8_t* args_ptr,    /* canonical-ABI encoded argument tuple */
    size_t args_len,
    uint8_t* ret_ptr,           /* caller-provided return buffer */
    size_t ret_cap,
    size_t* ret_len             /* in/out: required size on too-small */
);
```

Return convention is identical to forward direction
(`SPEC/canonical-abi.md` §14): `0` on success, non-zero on
error. Error codes live in `SPEC/canonical-abi.md` §13 (with the
new reverse-direction range defined in this SPEC §10).

The dispatcher must be **re-entrant from a single Lean thread**
(Lean's runtime is single-threaded per `LEO4-DESIGN.md §16`).
Cross-thread calls are out-of-scope; a future multi-Lean
runtime would extend this contract.

## 4. Worker Process Lifecycle

### 4.1 Default mode — long-running worker

A single worker process per cdylib. The dispatcher's first call
to `leo4_rust_call` triggers:

1. cdylib path resolution (§9).
2. `posix_spawn` (POSIX) or `CreateProcess` (Windows) of the
   worker binary with the resolved cdylib path as an argument.
3. Worker loads cdylib, reads `LEO4_RUST_SCHEMA_HASH` from it,
   sends a handshake frame containing that hash + its own
   compile-time `LEO4_RUST_ABI_VERSION`.
4. Dispatcher verifies the hash matches `<pkg>.leo4-rust-handshake`'s
   `schema_hash`. On mismatch returns
   `LEO4_ERR_HANDSHAKE_MISMATCH` (0x05) and tears the worker
   down.

Subsequent calls reuse the same worker. The worker runs each
request **serially** on its main thread:

1. Read request frame (§5).
2. `dlsym(mangled)` (cached after first resolution).
3. Wrap the call in `catch_unwind`.
4. On success, send response frame with status `0` + return
   bytes.
5. On Rust panic, send response frame with status
   `LEO4_ERR_RUST_PANIC` + detail string from the panic info.
   The worker then calls `std::process::abort` (to ensure no
   poisoned state survives). The dispatcher sees the EOF on
   the IPC channel, marks the worker dead, and lazily spawns a
   fresh one on the next call (signalling
   `LEO4_ERR_RUST_WORKER_RESTARTED` on the *first* post-crash
   call so the Lean caller can refresh any persistent state).

cdylib memory + the user function's global state are
**preserved across calls** in this mode. This is deliberate:
SMT solvers with incremental `push/pop`, parsers carrying
interning tables, etc., depend on it.

### 4.2 Isolated mode — per-call fresh worker (opt-in)

A function marked `#[leo4::export(isolated)]` runs in a fresh
worker every call:

1. Dispatcher spawns a transient worker exactly like §4.1 steps
   1–4, but separately from the persistent one.
2. Worker handles exactly one request, sends the response,
   then `_exit`s.
3. Dispatcher waits for the worker to exit, then returns the
   response to the Lean caller.

Cost: one worker spawn per call (`posix_spawn` ~5–10 ms on
Linux, comparable on Windows). Use only for untrusted code or
functions whose state contamination would corrupt later
unrelated calls.

The persistent worker and isolated-mode workers do not share
memory. They each load their own copy of the cdylib.

### 4.3 Optional recycle policy

The persistent worker may be configured to terminate and
respawn after N calls or T seconds. Disabled by default.

Configuration:

- Build-time attribute on the consuming Lean module: pending
  (Phase 9.X candidate).
- Runtime environment: `LEO4_RUST_WORKER_RECYCLE_CALLS=<N>`,
  `LEO4_RUST_WORKER_RECYCLE_SECONDS=<T>`. Unset = OFF.

On recycle the dispatcher behaves like a clean worker death
followed by re-spawn — the *next* call after the recycle
returns `LEO4_ERR_RUST_WORKER_RESTARTED` on success so the
caller can refresh persistent state.

### 4.4 Spawn / IPC abstraction layer

The dispatcher reaches OS spawn and IPC syscalls through a
single internal C interface, defined in the same translation
unit as `leo4_rust_call`. OS-specific code lives behind this
table only; the request loop, handshake verifier, worker-handle
cache, and `catch_unwind` plumbing never name `posix_spawn`,
`CreateProcess`, `socketpair`, or `CreateNamedPipe` directly.

```c
/* internal to libleo4_rust_bridge.a — not exported */
typedef struct leo4_worker leo4_worker_t;   /* opaque per-backend */

typedef struct {
    /* lifecycle */
    int  (*spawn)(const char* cdylib_path,
                  leo4_worker_t** out,
                  char*  err_buf, size_t err_cap);
    void (*kill)(leo4_worker_t* w);                       /* SIGKILL / TerminateProcess */
    int  (*reap)(leo4_worker_t* w, int* exit_status);     /* wait + free */

    /* IPC */
    int  (*send)(leo4_worker_t* w, const void* buf, size_t len);
    int  (*recv)(leo4_worker_t* w, void* buf, size_t cap, size_t* out_len);

    /* status (non-blocking) */
    int  (*alive)(leo4_worker_t* w);
} leo4_worker_ops_t;

extern const leo4_worker_ops_t leo4_worker_ops;   /* compile-time chosen */
```

Two backends ship in the same TU, gated by `#ifdef`:

- **POSIX backend** (`#if defined(__unix__) || defined(__APPLE__)`):
  `posix_spawn` + `socketpair(AF_UNIX, SOCK_STREAM, 0)` + `wait4`.
  The IPC end is handed to the child via
  `posix_spawn_file_actions_adddup2`.
- **Windows backend** (`#if defined(_WIN32)`): `CreateProcess`
  + named pipe (`CreateNamedPipeA` with name
  `\\.\pipe\leo4_rust_<pid>_<nonce>`) + `WaitForSingleObject`.
  Target triple is **`x86_64-pc-windows-gnullvm`** (clang + lld
  + UCRT), so this branch compiles under clang the same way the
  POSIX branch does — no MSVC-intrinsic fork. See
  `LEO4-DESIGN.md §9.1` for the target rationale.

A third **stub backend** ships unconditionally for unknown
platforms: every operation returns `LEO4_ERR_RUST_SPAWN_FAILED`.
This lets `libleo4_rust_bridge.a` link on every platform from
day 1; the bridge is always present, even where reverse-direction
is not yet ported.

The "single C translation unit" promise (§11) is preserved —
all three backends are sections of the same file gated by
`#ifdef`. The dispatcher's request loop calls
`leo4_worker_ops.spawn(...)` and friends, never the raw syscalls.

Future 9.X isolation backends (zygote-fork, wasm sandbox)
implement the same `leo4_worker_ops_t` table; swapping them in
requires zero churn in the request loop, handshake verifier, or
the Lean wrapper.

This layer is the first formally-specified instance of the
leo4-wide **OS-abstraction policy** (`OS-PORTABILITY.md`). Other
OS-specific concerns (library extension, RPATH / DLL search,
visibility attribute, compile flags) are tracked in that
document's audit ledger and will receive analogous layers as
they are touched.

### 4.5 Worker harness binary

leo4 ships a small harness executable `leo4-rust-worker`
(POSIX) / `leo4-rust-worker.exe` (Windows). It is invoked as:

```
leo4-rust-worker --cdylib <path> --ipc-fd <N>
```

(or `--ipc-pipe <name>` on Windows). The harness opens the
cdylib, performs the handshake, then runs the request loop. The
harness source lives in `crates/leo4-rust-worker/` (a regular
workspace crate, not a sibling). It is **the** part of leo4
that may legitimately use unsafe FFI to load arbitrary
cdylibs.

## 5. IPC Wire Format

unix-domain socket (POSIX) or named pipe (Windows). Framed,
little-endian, single round-trip per call.

### 5.1 Request frame (dispatcher → worker)

```
+-------------------+
| u32 magic = 0x4C45 ('LE')   |   sanity / version
+-------------------+
| u32 mangled_len             |   bytes in mangled name
+-------------------+
| u32 args_len                |   bytes in canonical-ABI argument tuple
+-------------------+
| <mangled_len bytes>         |   mangled name (UTF-8, no NUL)
+-------------------+
| <args_len bytes>            |   canonical-ABI argument tuple
+-------------------+
```

### 5.2 Response frame (worker → dispatcher)

```
+-------------------+
| u32 magic = 0x4C45          |
+-------------------+
| i32 status                  |   0 = ok; non-zero = error code
+-------------------+
| u32 ret_len                 |   bytes in canonical-ABI return value
+-------------------+
| u32 detail_len              |   bytes in optional UTF-8 error message
+-------------------+
| <ret_len bytes>             |   return payload (empty on error)
+-------------------+
| <detail_len bytes>          |   panic message / handshake mismatch hint
+-------------------+
```

### 5.3 Handshake frame (worker → dispatcher, first message)

```
+-------------------+
| u32 magic = 0x4C45          |
+-------------------+
| u32 hash_len = 13           |   schema_hash always 13 chars base32lc
+-------------------+
| u32 abi_version             |
+-------------------+
| <13 bytes>                  |   schema_hash from cdylib
+-------------------+
```

Dispatcher sends `0`/`0`/`0` (zero hash_len) to terminate the
worker cleanly when the Lean process exits.

### 5.4 Payload size limits

Hard limit: 256 MiB per single payload (`args_len` or `ret_len`
> 256 MiB ⇒ `LEO4_ERR_DECODE`). Large transfers should use a
resource handle (forward direction) or split across calls.

## 6. Isolation Model — Threat Matrix

leo4 maps three threats to the chosen isolation primitives.
T1, T2, T3 are introduced earlier in the design discussion;
the table below pins which mode handles which.

| Threat | Default (long-running worker) | `isolated` opt-in (fresh worker) |
|---|---|---|
| **T1** memory corruption | Worker process is separate from Lean. OS page tables prevent corruption of Lean memory. **Cross-call accumulation inside the worker is possible — user responsibility.** | Worker `_exit`s after each call; the next call gets a fresh address space. |
| **T2** Rust panic | `catch_unwind` catches; worker reports `LEO4_ERR_RUST_PANIC` and aborts. Lean stack untouched. Next call uses a fresh worker. | Same. |
| **T3** Thread leak | Spawned threads stay in the worker. Recycle policy (§4.3) bounds the lifetime; otherwise persist for the worker's lifetime. | Worker `_exit`s; OS reaps all threads. **No cross-call contamination.** |

leo4 does *not* try to detect or prevent user-side concurrency
inside the worker process. The contract is process-level
isolation — what happens inside the worker is the user's
problem, exactly the same way leo4-native's
`<pkg>.leo4-shim.so` puts the user's Lean code in charge of
its own thread safety.

## 7. Build Orchestration

The forward direction follows `D8` (Lake first, Cargo second).
The reverse direction follows the **mirror** ordering: Cargo
emits a metadata file that Lake reads, but the Rust cdylib
itself is **not** in Lake's build graph (it is loaded at
runtime). Concretely:

1. **Cargo pass** (`cargo build --release` of the user cdylib):
   - `#[leo4::export]` proc-macro emits per-function wrapper
     shims `leo4_rust__<mangled>` and a static metadata entry
     via the `linkme` crate.
   - The `leo4-build` crate's `wire_rust_exports()` helper,
     called from the cdylib's `build.rs`, writes:
     - `<pkg>.leo4-rust-exports.idl` — canonical IDL of all
       exports.
     - `<pkg>.leo4-rust-handshake` — JSON with `schema_hash`,
       `abi_version`, `cdylib_path` (absolute), and
       `emitted_at`.
2. **Lake pass** (`lake build` of the consuming Lean package):
   - The leo4 Lake plugin reads
     `<pkg>.leo4-rust-exports.idl` and `<pkg>.leo4-rust-handshake`
     from a path the user configures in `lakefile.lean`
     (typically via `Leo4.Build.requireRustExports`).
   - Generates `<pkg>.leo4-rust-imports.lean` — one `opaque`
     declaration per export with a typed wrapper that calls
     into `leo4_rust_call`.
   - Schema-hash mismatch between handshake and the wrapper
     module's compile-time constant raises a build-time
     error.
3. **leanc link**: Lake's link step adds
   `libleo4_rust_bridge.a` (from `Leo4.Build`'s static
   archive). No mention of the user cdylib here; the cdylib is
   discovered at runtime per §9.

A project containing **both** directions is fine: the two
metadata flows are independent (`<pkg>.leo4-schema` for forward,
`<pkg>.leo4-rust-exports.idl` for reverse). They don't create a
cycle as long as the forward-mode Lean exports don't
syntactically depend on reverse-mode imports of the same project,
which would only happen by accident.

## 8. Handshake File Format

`<pkg>.leo4-rust-handshake` — JSON, atomic write, same
conventions as `<pkg>.leo4-handshake` (`SPEC/handshake.md`).
Required fields:

```json
{
  "leo4_rust_handshake_version": 1,
  "schema_hash": "qi5gb74dbjyxo",
  "abi_version": 1,
  "package": "my-rust-smt",
  "cdylib_path": "/abs/path/to/libmy_rust_smt.so",
  "rust_toolchain": "1.85.0",
  "leo4_rust_macros_version": "0.1.0",
  "emitted_at": "2026-05-21T05:43:59Z",
  "exports": [
    { "logical_name": "Smt::solve", "instantiations": [ /* same shape as forward .leo4-mangling */ ] }
  ]
}
```

The `schema_hash` is FNV-1a-64 of the same canonical form the
forward direction uses, computed over the reverse package's
IDL. The hash spaces of forward and reverse are independent —
a Rust project's `<pkg>.leo4-rust-handshake` hash has no
relation to any Lean project's `<pkg>.leo4-handshake` hash.

## 9. cdylib Path Resolution

The dispatcher resolves the cdylib path in this order:

1. Environment variable **`LEO4_RUST_CDYLIB`** — runtime
   override. If set, used directly; resolution stops.
2. `cdylib_path` field in `<pkg>.leo4-rust-handshake`,
   surfaced as a compile-time constant string in the generated
   Lean wrapper.
3. **Sibling search** — same directory as the handshake file
   (or the Lean executable, on installed binaries), looking
   for `<package_basename>.{so,dylib,dll}` with the convention
   matching the build's `crate-type = ["cdylib"]` output name.

If none resolve, the dispatcher returns
`LEO4_ERR_RUST_CDYLIB_NOT_FOUND` on the first call.

This mirrors the forward-direction `LEO4_SHIM_SO` resolution
in `leo4-build`'s `wire()`.

## 10. Error Codes

The reverse-direction failure carrier extends
`SPEC/canonical-abi.md` §13. The Lean passthrough range was
formerly reserved exclusively for forward errors; the
reserved sub-range `0x0002_0000..0x0002_FFFF` is now defined
for Rust worker passthrough.

| Code | Meaning |
|---|---|
| `0x0000_0005` | Handshake mismatch (re-used; same meaning, applies to both directions) |
| `0x0002_0001` | `LEO4_ERR_RUST_PANIC` — user Rust function panicked; worker aborted; detail string carries the panic message |
| `0x0002_0002` | `LEO4_ERR_RUST_WORKER_RESTARTED` — the persistent worker died (or was recycled) between calls; the call succeeded after a fresh respawn, but any persistent state is gone |
| `0x0002_0003` | `LEO4_ERR_RUST_SPAWN_FAILED` — the dispatcher could not spawn a worker (OS error; detail string carries the system errno / message) |
| `0x0002_0004` | `LEO4_ERR_RUST_CDYLIB_NOT_FOUND` — none of the resolution paths in §9 produced a loadable cdylib |
| `0x0002_0005` | `LEO4_ERR_RUST_DLSYM_FAILED` — `dlsym` returned NULL for the requested mangled name (handshake passed but the symbol isn't in the cdylib; usually a stale binary) |
| `0x0002_0006` | `LEO4_ERR_RUST_IPC_FAILED` — IPC round-trip failed mid-call (worker died between sending the request and receiving the reply; partial state) |

Codes outside the `0x0002_0000..0x0002_FFFF` range that the
dispatcher may surface include: `0x05` (handshake mismatch),
`0x07` (return buffer too small — propagated identically to
forward direction).

## 11. C Standard for `libleo4_rust_bridge.a`

The dispatcher is a single C translation unit. **Baseline:
C17/C18** (ISO/IEC 9899:2018). When the host compiler exposes
C23 (ISO/IEC 9899:2024) and the build environment opts into
it, leo4 may use C23-specific niceties (`[[nodiscard]]`,
`constexpr` for integer literals where useful, `auto` for
internal locals, `nullptr`) — these stay non-load-bearing so
the C17 build path remains functional.

The leo4 Lake plugin / `leanc` driver passes `-std=c2x` when
the compiler reports support (gcc ≥ 13, clang ≥ 16), falling
back to `-std=c17`. C11 is rejected — the
dispatcher uses `_Atomic` heavily for the worker-handle
cache and the `static_assert` semantics that C17 cleaned up.

Because the Tier 2 Windows target is `x86_64-pc-windows-gnullvm`
(clang + lld + UCRT, see `LEO4-DESIGN.md §9.1`), the same
`-std=c17` / `-std=c2x` invocation works on Windows as on Linux
— no MSVC-specific compiler driver branch.

Per the rustc platform-support note for gnullvm, Rust code on
`*-pc-windows-gnullvm` is ABI-compatible with C code built by
clang for either `*-pc-windows-gnu` or `*-pc-windows-gnullvm`,
**as long as the C side also goes through an LLVM-based
toolchain**. leo4 enforces this end-to-end: the dispatcher is
compiled with clang (via `leanc` or directly with
`--target=x86_64-pc-windows-gnu`), and user cdylibs are built
for `*-pc-windows-gnullvm`. The two end up on the same C ABI
even though their Rust / C target triples are not literally
identical. C++ is never involved.

Mandatory C standard library headers:

- POSIX path: `<unistd.h>`, `<spawn.h>`, `<sys/socket.h>`,
  `<sys/wait.h>`, `<dlfcn.h>`, `<errno.h>`.
- Windows path: `<windows.h>` only.
- Both: `<stdint.h>`, `<stddef.h>`, `<string.h>`,
  `<stdatomic.h>`, `<stdlib.h>`.

The dispatcher must not require any third-party C library; the
build is `cc <single-file>` on every supported platform.

## 12. v0 In-Scope vs Deferred

| Feature | v0 |
|---|---|
| Long-running worker (POSIX, `posix_spawn` + unix-domain socket) | ✅ |
| `catch_unwind` + `abort` + automatic respawn | ✅ |
| Handshake (`schema_hash` compare on worker init) | ✅ |
| `#[leo4::export]` proc-macro | ✅ |
| `linkme`-based metadata collection | ✅ |
| `<pkg>.leo4-rust-exports.idl` / `<pkg>.leo4-rust-handshake` emit | ✅ |
| Lake plugin Rust-IDL ingestion + Lean wrapper emit | ✅ |
| `examples/05-rust-export/` end-to-end demo | ✅ |
| `tests/conformance/` reverse-direction byte parity | ✅ |
| Windows path (`CreateProcess` + named pipe) | Design in-scope; implementation per Tier 2 schedule |
| `#[leo4::export(isolated)]` opt-in mode | Design in-scope; implementation may slip to 9.X |
| Recycle policy (env-driven) | Design in-scope; implementation may slip to 9.X |
| Callback / function-arrow ABI | Out (9.X candidate) |
| Stronger isolation (zygote-fork, wasm sandbox) | Out (9.X candidate) |
| `async fn` reverse exports | Out (no concrete consumer yet) |

## 13. Future Work

Recorded here so the v0 design hatches stay open:

- **9.X — function-arrow ABI** — for Rust functions that need
  to invoke a Lean closure mid-call (e.g. SMT solver asking
  the host for sub-formula simplification). Pulls in
  function-pointer mangling (`SPEC/mangling.md` §3's TBD
  slot) plus a re-entrant dispatcher (Rust worker calls back
  into Lean while the original call is in flight).
- **9.X — alternative isolation backends** — process-pool /
  zygote-fork variants for callers with stricter isolation
  needs. The dispatcher API (§3) is backend-neutral, so
  swapping the implementation does not break callers.
- **9.X — wasm sandbox backend** — for callers willing to
  build the cdylib as wasm. Uses the existing wasm Component
  Model sibling (`sibling/leo4-wasip3/`) infrastructure.

## 14. Cross-references

- `LEO4-DESIGN.md §1` D16 — design decision record.
- `LEO4-DESIGN.md §2` — architecture diagram (the
  `#[leo4_import]` half is realised by this SPEC).
- `LEO4-DESIGN.md §16` — thread-safety policy, single-Lean-thread
  invariant.
- `SPEC/mangling.md` §§2–4 — mangling rules (shared with
  forward direction).
- `SPEC/canonical-abi.md` §13 — error-code table (extended by
  this SPEC §10).
- `SPEC/canonical-abi.md` §14 — function call convention (the
  `leo4_rust_call` API in §3 mirrors it).
- `SPEC/handshake.md` — JSON file conventions
  (`<pkg>.leo4-rust-handshake` follows them).
- `ROADMAP.md` Phase 9 — substep plan.
