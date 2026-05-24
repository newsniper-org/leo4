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

> **User-facing quickstart:**
> `SPEC/reverse-direction-quickstart.md` — the 60-second tour,
> common pitfalls, isolation modes. This document is the
> normative reference; the quickstart is the introduction.

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

### 3.1 Lean-side glue shim contract

The single C entry above sees byte-pointers. The Lean side
sees a `ByteArray`-returning extern declared with this exact
signature:

```lean
@[extern "leo4_rust_call_lean"]
private opaque leo4RustCallRaw
    (mangled : @& String) (args : @& ByteArray)
    : IO ByteArray
```

**The monad MUST be `IO ByteArray`, not `BaseIO ByteArray`.**
Lean 4's `MonadLift BaseIO IO` instance lifts at the type
level but the lowered C ABI of `BaseIO α` vs `IO α` reads
back the result differently from an `IO` block; declaring the
extern with `IO` directly avoids the runtime mismatch.

The returned ByteArray's layout is **status prefix +
payload**:

- `bytes[0..4]` — `status : UInt32` in little-endian. `0`
  on success; non-zero matches the dispatcher's error
  codes (`SPEC/canonical-abi.md` §13 +
  `SPEC/reverse-direction.md` §10).
- `bytes[4..]` — when `status == 0`, the call's
  canonical-ABI encoded return payload. When `status != 0`,
  empty.

Rationale: the original design returned `IO (UInt32 ×
ByteArray)`, but Lean 4 codegen packs `UInt32` as an inline
scalar field in `Prod`'s ctor (not a boxed `lean_object*`),
which the C-side `lean_alloc_ctor(0, 2, 0)` builder cannot
reproduce. The flat ByteArray with a status prefix sidesteps
the Prod ABI entirely. The generated typed wrapper
(`leo4-rust-emit --emit-lean`) carries a `decodeStatus`
helper that reads the first 4 bytes as LE u32 and a body
that calls `Leo4.LeanMarshal.canonicalDecode (T := …) resp 4`
to decode the payload starting at offset 4.

## 4. Worker Process Lifecycle

### 4.1 Default mode — long-running worker

A single worker process per cdylib. The dispatcher's first call
to `leo4_rust_call` triggers:

1. cdylib path resolution (§9).
2. `posix_spawn` (POSIX) or `CreateProcess` (Windows) of the
   worker binary with the resolved cdylib path as an argument.
3. Worker loads cdylib, recomputes the schema_hash via the
   same FNV-1a-64 + base32lc pipeline `leo4-rust-emit` used
   when writing the handshake JSON (pkg / iface inputs taken
   from `LEO4_RUST_HANDSHAKE_PKG` / `_IFACE` env vars), then
   sends a 25-byte handshake frame (§5.3) containing the
   computed hash + `LEO4_ABI_VERSION` = 1.
4. **Dispatcher MUST consume the handshake frame immediately
   after spawn — before any request frame goes out.** Reading
   the 12-byte header (magic + hash_len + abi_version) then
   the 13-byte hash. Verification against
   `LEO4_RUST_SCHEMA_HASH` env (when set) raises
   `LEO4_ERR_HANDSHAKE_MISMATCH` (0x05). When the env is
   unset, verification is deferred to the typed Lean
   wrapper's compile-time `schemaHash` pin (the wrapper
   raises `IO.userError` itself on observed mismatch).
   Skipping the handshake consume causes the worker's 25
   bytes to pile up in the IPC buffer and the dispatcher's
   subsequent response read decodes them as a response
   header — observed historically as garbage `status`
   values (see CHANGELOG entry for the Phase 9 runtime
   fix, 2026-05-23).

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
worker every call. The Lean wrapper distinguishes isolated
exports at the *mangled-name* layer:

- Default exports pass the raw mangled name to
  `leo4_rust_call` (e.g. `"leo4_rust__add__u64_u64"`).
- Isolated exports pass the same body prefixed with `"iso:"`
  (e.g. `"iso:leo4_rust__add__u64_u64"`).

The dispatcher inspects the first few bytes of every mangled
argument with `memcmp`. When `"iso:"` is present, it strips
the prefix and routes through `leo4_dispatch_isolated`:

1. `leo4_worker_ops->spawn` mints a transient worker
   process. The cdylib path comes from the same resolution
   chain as the persistent path (§9); the persistent worker
   slot is untouched.
2. Worker performs its normal init handshake (the dispatcher
   ignores the handshake's `schema_hash` for isolated calls
   in v9.X — schema mismatches surface on the *persistent*
   path which all callers exercise first; tightening this
   is a 9.X follow-on).
3. Send the request frame (§5.1).
4. Receive the response frame (§5.2).
5. Send a magic=0 shutdown frame to the worker.
6. `leo4_worker_ops->reap` blocks on the worker's exit and
   `kill`s + `close`s the IPC end.

Cost per call: one `posix_spawn` / `CreateProcess` (~5–10 ms
on Linux, comparable on Windows) plus the worker's own init
overhead (cdylib `dlopen`, schema_hash recomputation). Use
only for exports whose state contamination would corrupt
later unrelated calls; otherwise the persistent path is the
right default.

The persistent worker and isolated-mode workers do not share
memory or IPC channels. They each load their own copy of the
cdylib.

**Wire / API surface preserved**: the prefix trick adds zero
new dispatcher entry points, no new wire frame fields, no new
opaque type. Backwards-compatible with any 9-5 wrapper
consumer that doesn't tag isolated exports.

### 4.3 Recycle policy

The persistent worker may be configured to terminate and
respawn after N calls. Disabled by default.

Configuration:

- Runtime environment **`LEO4_RUST_WORKER_RECYCLE_CALLS=N`**
  (positive integer). Unset / `0` / non-numeric = recycling
  disabled.
- **`LEO4_RUST_WORKER_RECYCLE_SECONDS=T`** (positive integer
  seconds, Phase 10-A4 2026-05-21). Unset / `0` / non-numeric =
  time-based recycling disabled. The two limits are independent
  and combinable — whichever fires first triggers the recycle.

Implementation (`shim/leo4_rust_bridge.c`):

- `leo4_worker_slot_t` carries an `_Atomic uint64_t
  call_count` field. The dispatcher increments it after each
  successful response.
- It also carries `_Atomic uint64_t spawn_time_s`, stamped at
  `leo4_monotonic_seconds()` once the spawn + handshake-consume
  completes successfully.
- `leo4_recycle_init_once` parses both envs on first call and
  caches them in file-scoped `_Atomic uint64_t`s
  (`leo4_recycle_calls_limit`, `leo4_recycle_seconds_limit`).
- Before each persistent dispatch, the dispatcher checks
  `call_count >= calls_limit` and `(now_s - spawn_time_s) >=
  seconds_limit`. If either fires, it atomically swaps the
  worker pointer out, kills + reaps via the ops table, resets
  both bookkeeping fields, and sets the
  `leo4_persistent_was_restarted` side-channel flag (Phase
  10-A5). The standard lazy-spawn path then spawns the fresh
  worker.
- Time source is `clock_gettime(CLOCK_MONOTONIC)` on POSIX,
  `GetTickCount64() / 1000` on Windows-gnullvm. Clock-skew /
  wall-clock adjustments cannot prematurely fire the recycle
  because `CLOCK_MONOTONIC` is monotone non-decreasing across
  NTP slews / `settimeofday` calls.

Caller-visible behaviour: recycle is **transparent on the
dispatch path** (the next call simply uses the fresh worker
and returns its result). Callers that want to know a recycle
happened poll via:

```c
LEO4_RUST_EXPORT int leo4_rust_bridge_take_restart_flag(void);
```

This atomic exchange-and-clear returns `1` exactly once per
recycle event observed by the dispatcher (call-based OR
time-based), then `0` until the next recycle. Lean wrappers
that want to surface `LEO4_ERR_RUST_WORKER_RESTARTED`
(0x00020002) to their caller bind it via a custom `@[extern]`
shim and check after each call (Phase 10-A5).

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

**Status (2026-05-24):** POSIX (`__unix__ || __APPLE__`) and
Windows (`_WIN32`) backends both shipped on the dispatcher
(C) side; the stub backend stays as the unconditional
fallback for unsupported tiers. The **worker-side**
counterpart — `leo4-rust-worker`'s
`open_ipc_channel` Windows branch — was a stub through
2026-05-23 and was filled 2026-05-24 with a real
`CreateFileW` (via `std::fs::OpenOptions::open`)
implementation that retries 10× on `NotFound` /
`ConnectionRefused` to absorb the spawn-then-register
race; the worker now compiles + cross-compiles clean
on `x86_64-pc-windows-gnullvm`. Windows *runtime*
verification still waits on the Tier 2 CI matrix
(`docs/windows-manual-test-plan.md` holds the manual
prep audit).

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
problem, exactly the same way leo4-mslean4's
`<pkg>.leo4-shim.so` puts the user's Lean code in charge of
its own thread safety.

## 7. Build Orchestration

The forward direction follows `D8` (Lake first, Cargo second).
The reverse direction follows the **mirror** ordering: Cargo
emits a metadata file that Lake reads, but the Rust cdylib
itself is **not** in Lake's build graph (it is loaded at
runtime). Concretely:

1. **Cargo pass** (`cargo build` of the user cdylib):
   - `#[leo4::export]` proc-macro emits per-function wrapper
     shims `leo4_rust__<mangled>` and a static metadata entry
     via the `linkme` crate (Phase 9-1).
   - `leo4-abi`'s `rust_exports` module additionally exports the
     `leo4_rust_describe_exports` C entry that surfaces the
     `EXPORTS` slice's in-process pointer + length to anyone
     who later `dlopen`s the cdylib.
2. **Emit pass** (`leo4-rust-emit` CLI, Phase 9-2):
   - The user invokes
     `leo4-rust-emit --cdylib <path-to-.so/.dylib/.dll>
                     --out-dir <metadata-dir>
                     [--pkg <name>] [--iface <name>]`
     after `cargo build`.
   - The CLI `dlopen`s the cdylib, calls
     `leo4_rust_describe_exports`, copies each `ExportEntry`
     out of the cdylib's address space, computes the
     canonical IDL form + its FNV-1a-64 schema_hash, and
     writes two files into `--out-dir`:
     - `<pkg>.leo4-rust-exports.idl` — pretty canonical IDL.
     - `<pkg>.leo4-rust-handshake` — JSON (§8). `cdylib_path`
       is the absolute path passed via `--cdylib`.
   - The CLI does not modify the cdylib. Re-running it after
     a rebuild is the only way to refresh the metadata.
3. **Lake pass** (`lake build` of the consuming Lean package,
   Phase 9-5):
   - The leo4 Lake plugin reads
     `<pkg>.leo4-rust-exports.idl` and `<pkg>.leo4-rust-handshake`
     from a path the user configures in `lakefile.lean`
     (typically via `Leo4.Build.requireRustExports`).
   - Generates `<pkg>.leo4-rust-imports.lean` — one `opaque`
     declaration per export with a typed wrapper that calls
     into `leo4_rust_call`.
   - Schema-hash mismatch between handshake and the cdylib
     (recomputed by the worker at init time, §1) surfaces as
     `LEO4_ERR_HANDSHAKE_MISMATCH` on the first call rather
     than as a build-time error; this lets `lake build` proceed
     even when the cdylib will be replaced at runtime via
     `LEO4_RUST_CDYLIB` (§9).
4. **leanc link**: Lake's link step adds
   `libleo4_rust_bridge.a` (from `Leo4.Build`'s static
   archive). No mention of the user cdylib here; the cdylib is
   discovered at runtime per §9.

The `leo4-build` crate exposes `wire_rust_exports(out_dir)` as a
build-script helper for **Lean-side consumers** (or a thin Cargo
wrapper that re-exports the paths to Lake): it locates the two
metadata files, emits
`cargo:rustc-env=LEO4_RUST_HANDSHAKE_FILE` /
`cargo:rustc-env=LEO4_RUST_EXPORTS_IDL_FILE`, and registers
`cargo:rerun-if-changed=` for both plus
`cargo:rerun-if-env-changed=LEO4_RUST_CDYLIB`. The Rust cdylib
producer's `build.rs` does **not** call this — it has nothing to
wire (the emit step is post-build).

A project containing **both** directions is fine: the two
metadata flows are independent (`<pkg>.leo4-schema` for forward,
`<pkg>.leo4-rust-exports.idl` for reverse). They don't create a
cycle as long as the forward-mode Lean exports don't
syntactically depend on reverse-mode imports of the same project,
which would only happen by accident.

## 8. Handshake File Format

`<pkg>.leo4-rust-handshake` — JSON, atomic write, same
conventions as `<pkg>.leo4-handshake` (`SPEC/handshake.md`).
Required fields (matching what `leo4-rust-emit` 0.1.0 writes,
Phase 9-2):

```json
{
  "leo4_rust_handshake_version": 1,
  "schema_hash": "7i2wz2k5rqhls",
  "abi_version": 1,
  "package": "my_rust_smt",
  "interface": "MyRustSmt",
  "cdylib_path": "/abs/path/to/libmy_rust_smt.so",
  "rust_toolchain": "rustc-stable",
  "leo4_rust_emit_version": "0.1.0",
  "emitted_at": "2026-05-21T07:27:42Z",
  "exports": [
    {
      "logical_name": "solve",
      "mangled": "leo4_rust__solve__str",
      "param_types": ["str"],
      "ret_type": "u64",
      "isolated": false,
      "abi_version": 1
    }
  ]
}
```

The `schema_hash` is FNV-1a-64 of the **collapsed canonical IDL
form** (single-space token separators, no newlines, exports
sorted by mangled name), rendered as 13 base32lc characters
(`abcdefghijklmnopqrstuvwxyz234567`, no padding). This matches
the forward direction's algorithm bit-for-bit; only the input
text differs. The hash spaces of forward and reverse are
independent — a Rust project's `<pkg>.leo4-rust-handshake` hash
has no relation to any Lean project's `<pkg>.leo4-handshake`
hash.

The `exports` array carries one row per `#[leo4::export]`
function, structurally similar to (but distinct from) the
forward direction's `<pkg>.leo4-mangling` shape:

- `logical_name` — the Rust `fn` identifier.
- `mangled` — exact wrapper symbol; the dispatcher resolves
  this via `dlsym` / `GetProcAddress` after the worker loads
  the cdylib.
- `param_types` / `ret_type` — IDL mangle strings from
  `SPEC/mangling.md` §2. `ret_type` is the empty string for
  unit returns.
- `isolated` — `true` iff `#[leo4::export(isolated)]`.
- `abi_version` — currently always `1`; bumps lockstep with
  the `ExportEntry` repr-C layout.

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

## 10a. Re-entrant callbacks (Phase 10-B1 design landing, runtime deferred)

A `#[leo4::export]` Rust function whose signature includes a
function-arrow parameter (e.g. `fn(formula: Formula, ask: fn(SubFormula) -> bool) -> bool`)
will, at runtime, need to **call back into the Lean process** to
invoke the closure that the Lean caller passed. The wire shape for
`fn(…) -> R` is a single `u64 callback_id` (see
`SPEC/canonical-abi.md` §13a); converting that id into an actual
invocation is what this section specifies.

This section is the **design landing** that pins the protocol shape
so cross-impl IDL / mangling conformance can ship now (Phase
10-B1). The runtime implementation (frames, dispatcher state
machine, Lean-side closure registry) is deferred to a Phase
10-B1.x follow-up.

### Wire-level frame extension

The IPC protocol gains two new frame kinds in the worker → main
direction, interleaved with normal `RESPONSE` frames during an
in-flight `REQUEST`:

| Magic | Direction | Payload |
|---|---|---|
| `0x4C45 4351 'LECQ'` | worker → main | u64 callback_id, u32 args_len, args_bytes |
| `0x4C45 4352 'LECR'` | main → worker | u32 status, u32 ret_len, ret_bytes |

(`LECQ` = "leo4 callback query", `LECR` = "leo4 callback
response". The earlier-defined `LEAN`/`LEAR` magics remain the
outer request/response envelope.)

While a worker is executing a `REQUEST` that includes function-arrow
arguments, every invocation of the closure inside Rust code blocks
the worker on `read()` until the main process answers `LECR`. The
main process side, on dispatching the original request, enters a
**callback-receiving loop** that handles any number of `LECQ`
frames before the final `LEAR` carrying the outer request's
return value.

### Lean-side closure registry

The Lean wrapper that generates the outbound call allocates a
fresh `u64` from a thread-local counter for each closure being
sent, stores `id → IO α` in a `HashMap`, then deallocates the
entry as soon as the outer call returns (success OR error). Reusing
an id across calls is permitted; the registry is per-call-scope.

### Rust-side closure thunks

The `leo4-macros::export` proc-macro recognises function-arrow
parameter types and substitutes a typed `LeanCallback<R, Args>`
wrapper struct holding the opaque `callback_id` and a method
`invoke(args) -> Result<R, LeanError>` that:
1. encodes the args via canonical-ABI,
2. emits a `LECQ` frame on the IPC channel,
3. blocks on `LECR`,
4. decodes the return.

The macro statically rejects function-arrow parameters that
themselves contain `Self` / `Cyc<i>` (no nested boundary
recursion for v0).

### Lifetime + cleanup invariants

- The Lean main process MUST deregister the closure id immediately
  after the outer call returns, even if the worker crashes.
  `LEO4_RUST_WORKER_RESTARTED` (Phase 10-A5 follow-up) is the
  signal that triggers this purge.
- The Rust worker MUST NOT retain a `LeanCallback` past the body
  of the export it was passed to. Holding one until later (e.g.
  in a `static`) is undefined behaviour and the runtime is
  permitted to abort the worker on detection.
- Schema hash includes the function-arrow type and its
  args/return in the same way as any other type, via the
  mangling above. A `fn(u8) -> u8` parameter rotates the schema
  hash compared to its absence.

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
| `tests/conformance/` reverse-direction byte parity | partial — pipeline emit verified in 9-7; per-primitive harness 9.X |
| Windows backend (dispatcher `CreateProcess` + `CreateNamedPipeA`; worker `CreateFileW` via `OpenOptions::open` with 10× retry) | ✅ code on both sides as of 2026-05-24; Tier 2 runtime CI follows |
| `#[leo4::export(isolated)]` opt-in mode | ✅ |
| Recycle policy — call-based (`LEO4_RUST_WORKER_RECYCLE_CALLS`) | ✅ |
| Recycle policy — time-based (`LEO4_RUST_WORKER_RECYCLE_SECONDS`) | ✅ (Phase 10-A4) |
| Declarative Lake `extern_lib` integration | Out (9.X — needs Lake 5.x API spike) |
| `LEO4_ERR_RUST_WORKER_RESTARTED` surfacing on recycle | ✅ (Phase 10-A5 side-channel via `leo4_rust_bridge_take_restart_flag`) |
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
