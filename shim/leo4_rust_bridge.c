/* leo4_rust_bridge.c — Phase 9-4a dispatcher skeleton + stub backend.
 *
 * SPEC: SPEC/reverse-direction.md
 *
 * Single C translation unit. Compiles under clang/gcc with
 * `-std=c17` (or `-std=c2x` when available). The whole library is
 * exposed via the single C entry point `leo4_rust_call`, which the
 * Lean side reaches through `@[extern "leo4_rust_call"]`.
 *
 * Phase 9-4a in-scope:
 *   - `leo4_worker_ops_t` ops-table interface (sec 4.4).
 *   - Stub backend filling every op with always-error
 *     implementations.
 *   - Compile-time backend selection chain (POSIX in 9-4b,
 *     Windows in 9-4c — both stub-only here).
 *   - Dispatcher request loop: lazy-spawn -> send request frame
 *     -> recv response frame -> return status. Calls flow
 *     entirely through `leo4_worker_ops`; no OS syscall named
 *     outside the backend block.
 *   - Lazy worker-handle cache via `_Atomic` slot.
 *   - Frame I/O helpers using only `send` / `recv` from the ops
 *     table.
 *
 * Out of 9-4a (lands in 9-4b/c, 9-5):
 *   - Real POSIX / Windows backends.
 *   - Schema-hash mismatch detection (needs the Lean wrapper to
 *     pin the expected hash, Phase 9-5).
 *   - `isolated`-mode fresh worker per call (Phase 9-X).
 *   - Recycle policy (Phase 9-X).
 *
 * On 9-4a only the stub backend is wired, so `leo4_rust_call`
 * always returns `LEO4_ERR_RUST_SPAWN_FAILED` on the first
 * invocation. This is the correct behaviour for a build that
 * hasn't picked up a real backend yet.
 */

/* POSIX feature flags MUST come before any include — they affect
 * which symbols `<signal.h>` / `<unistd.h>` etc. expose. */
#if defined(__unix__) || defined(__APPLE__)
#  if !defined(_GNU_SOURCE)
#    define _GNU_SOURCE 1
#  endif
#  if defined(__APPLE__) && !defined(_DARWIN_C_SOURCE)
#    define _DARWIN_C_SOURCE 1
#  endif
#endif

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdatomic.h>

#if defined(__unix__) || defined(__APPLE__)
#  include <errno.h>
#  include <fcntl.h>
#  include <signal.h>
#  include <spawn.h>
#  include <sys/socket.h>
#  include <sys/wait.h>
#  include <unistd.h>
extern char** environ;
#endif

#if defined(_WIN32)
#  define WIN32_LEAN_AND_MEAN
#  include <windows.h>
#endif

/* ─── Wire constants (SPEC §5) ────────────────────────────────────── */

#define LEO4_FRAME_MAGIC          0x4C45u  /* 'L', 'E' as 16 LSBs */
#define LEO4_ABI_VERSION          1u
#define LEO4_SCHEMA_HASH_LEN      13u

/* Hard payload ceiling per SPEC §5.4. */
#define LEO4_MAX_PAYLOAD_BYTES    (256u * 1024u * 1024u)

/* ─── Error codes (SPEC §10 / canonical-abi §13) ─────────────────── */

#define LEO4_OK                          0
#define LEO4_ERR_DECODE                  0x00000001
#define LEO4_ERR_HANDSHAKE_MISMATCH      0x00000005
#define LEO4_ERR_BUFFER_TOO_SMALL        0x00000007
#define LEO4_ERR_RUST_PANIC              0x00020001
#define LEO4_ERR_RUST_WORKER_RESTARTED   0x00020002
#define LEO4_ERR_RUST_SPAWN_FAILED       0x00020003
#define LEO4_ERR_RUST_CDYLIB_NOT_FOUND   0x00020004
#define LEO4_ERR_RUST_DLSYM_FAILED       0x00020005
#define LEO4_ERR_RUST_IPC_FAILED         0x00020006

/* ─── Ops table (SPEC §4.4) ────────────────────────────────────────
 *
 * The dispatcher reaches every OS syscall through this struct. Each
 * backend (stub, POSIX, Windows) provides one instance; the
 * `leo4_worker_ops` global below points to whichever the current
 * `#ifdef` chain selects.
 */

typedef struct leo4_worker leo4_worker_t;   /* opaque per-backend */

typedef struct {
    /* lifecycle */
    int  (*spawn)(const char* cdylib_path,
                  leo4_worker_t** out,
                  char*  err_buf, size_t err_cap);
    void (*kill)(leo4_worker_t* w);
    int  (*reap)(leo4_worker_t* w, int* exit_status);

    /* IPC */
    int  (*send)(leo4_worker_t* w, const void* buf, size_t len);
    int  (*recv)(leo4_worker_t* w, void* buf, size_t cap, size_t* out_len);

    /* status (non-blocking) */
    int  (*alive)(leo4_worker_t* w);
} leo4_worker_ops_t;

/* ─── Stub backend ────────────────────────────────────────────────
 *
 * Compiles on every platform. Always errors with
 * `LEO4_ERR_RUST_SPAWN_FAILED` so dispatcher code paths and
 * downstream callers can be exercised even before a real backend
 * lands.
 */

static int stub_spawn(const char* p, leo4_worker_t** out,
                      char* err_buf, size_t err_cap) {
    (void)p;
    if (out) *out = NULL;
    if (err_buf && err_cap > 0) {
        static const char msg[] =
            "stub backend: no real worker spawn implementation compiled in "
            "(9-4b POSIX / 9-4c Windows pending)";
        size_t n = sizeof msg - 1;
        if (n >= err_cap) n = err_cap - 1;
        memcpy(err_buf, msg, n);
        err_buf[n] = '\0';
    }
    return LEO4_ERR_RUST_SPAWN_FAILED;
}

static void stub_kill(leo4_worker_t* w) { (void)w; }

static int stub_reap(leo4_worker_t* w, int* exit_status) {
    (void)w;
    if (exit_status) *exit_status = -1;
    return LEO4_ERR_RUST_SPAWN_FAILED;
}

static int stub_send(leo4_worker_t* w, const void* buf, size_t len) {
    (void)w; (void)buf; (void)len;
    return LEO4_ERR_RUST_IPC_FAILED;
}

static int stub_recv(leo4_worker_t* w, void* buf, size_t cap, size_t* out_len) {
    (void)w; (void)buf; (void)cap;
    if (out_len) *out_len = 0;
    return LEO4_ERR_RUST_IPC_FAILED;
}

static int stub_alive(leo4_worker_t* w) { (void)w; return 0; }

__attribute__((unused))
static const leo4_worker_ops_t leo4_stub_ops = {
    .spawn = stub_spawn,
    .kill  = stub_kill,
    .reap  = stub_reap,
    .send  = stub_send,
    .recv  = stub_recv,
    .alive = stub_alive,
};

/* ─── POSIX backend (Phase 9-4b) ──────────────────────────────────
 *
 * `posix_spawn` + `socketpair(AF_UNIX, SOCK_STREAM, 0)` + `waitpid`.
 *
 * Worker process model: the dispatcher creates a unix-domain
 * socketpair; one end is retained as `sock_fd`, the other is dup2'd
 * into the child's fd 3 via `posix_spawn_file_actions_adddup2`. The
 * child is invoked as:
 *
 *     leo4-rust-worker --cdylib <path> --ipc-fd 3
 *
 * Worker binary path resolution:
 *   1. env LEO4_RUST_WORKER_BIN  (absolute path override)
 *   2. fallback "leo4-rust-worker" via `posix_spawnp` (PATH search)
 *
 * cdylib path comes from `leo4_cdylib_path()` (env
 * LEO4_RUST_CDYLIB on 9-4a; handshake-baked default in 9-5).
 */

#if defined(__unix__) || defined(__APPLE__)

struct leo4_worker {
    pid_t pid;
    int   sock_fd;   /* parent end of socketpair; -1 once reaped */
};

#define LEO4_WORKER_IPC_FD 3  /* the child sees the socketpair here */

static int posix_spawn_worker(const char* cdylib_path,
                              leo4_worker_t** out,
                              char*  err_buf, size_t err_cap) {
    if (out) *out = NULL;
    int sv[2] = { -1, -1 };
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, sv) != 0) {
        if (err_buf && err_cap > 0) {
            (void)snprintf(err_buf, err_cap, "socketpair: %s", strerror(errno));
        }
        return LEO4_ERR_RUST_SPAWN_FAILED;
    }

    /* Pin the parent end above the well-known stdio fds + the
     * worker's IPC fd so a stray dup2 in the child cannot collide.
     * On modern Linux/macOS sockets already come back at the next
     * free fd, but we guard anyway. */
    int parent_fd = sv[0];
    int child_fd  = sv[1];

    /* The child should NOT inherit the parent end. */
    (void)fcntl(parent_fd, F_SETFD, FD_CLOEXEC);

    posix_spawn_file_actions_t actions;
    if (posix_spawn_file_actions_init(&actions) != 0) {
        (void)close(parent_fd);
        (void)close(child_fd);
        if (err_buf && err_cap > 0) {
            (void)snprintf(err_buf, err_cap,
                           "posix_spawn_file_actions_init: %s",
                           strerror(errno));
        }
        return LEO4_ERR_RUST_SPAWN_FAILED;
    }

    /* Child: dup2 socketpair end onto fd 3; close the high-fd
     * version after the dup so the child only owns fd 3. */
    int rc = 0;
    rc |= posix_spawn_file_actions_adddup2(&actions, child_fd, LEO4_WORKER_IPC_FD);
    if (child_fd != LEO4_WORKER_IPC_FD) {
        rc |= posix_spawn_file_actions_addclose(&actions, child_fd);
    }
    if (rc != 0) {
        posix_spawn_file_actions_destroy(&actions);
        (void)close(parent_fd);
        (void)close(child_fd);
        if (err_buf && err_cap > 0) {
            (void)snprintf(err_buf, err_cap, "posix_spawn_file_actions_add*: %d", rc);
        }
        return LEO4_ERR_RUST_SPAWN_FAILED;
    }

    /* Build child argv. The string buffers below outlive the
     * posix_spawnp call (which copies them into the child's address
     * space). */
    const char* worker_bin = getenv("LEO4_RUST_WORKER_BIN");
    int use_path_search = 0;
    if (!worker_bin || !worker_bin[0]) {
        worker_bin = "leo4-rust-worker";
        use_path_search = 1;
    }
    char fd_arg[16];
    (void)snprintf(fd_arg, sizeof fd_arg, "%d", LEO4_WORKER_IPC_FD);

    char* argv[] = {
        (char*)worker_bin,
        (char*)"--cdylib", (char*)cdylib_path,
        (char*)"--ipc-fd", fd_arg,
        NULL,
    };

    pid_t child_pid = 0;
    int spawn_rc = use_path_search
        ? posix_spawnp(&child_pid, worker_bin, &actions, NULL, argv, environ)
        : posix_spawn(&child_pid, worker_bin, &actions, NULL, argv, environ);

    posix_spawn_file_actions_destroy(&actions);

    if (spawn_rc != 0) {
        (void)close(parent_fd);
        (void)close(child_fd);
        if (err_buf && err_cap > 0) {
            (void)snprintf(err_buf, err_cap,
                           "posix_spawn(%s): %s",
                           worker_bin, strerror(spawn_rc));
        }
        return spawn_rc == ENOENT ? LEO4_ERR_RUST_CDYLIB_NOT_FOUND
                                  : LEO4_ERR_RUST_SPAWN_FAILED;
    }

    /* Parent doesn't need the child end of the socket anymore. */
    (void)close(child_fd);

    leo4_worker_t* w = (leo4_worker_t*)calloc(1, sizeof *w);
    if (!w) {
        (void)close(parent_fd);
        (void)kill(child_pid, SIGKILL);
        (void)waitpid(child_pid, NULL, 0);
        if (err_buf && err_cap > 0) {
            (void)snprintf(err_buf, err_cap, "calloc(leo4_worker_t)");
        }
        return LEO4_ERR_RUST_SPAWN_FAILED;
    }
    w->pid = child_pid;
    w->sock_fd = parent_fd;
    *out = w;
    return LEO4_OK;
}

static void posix_kill_worker(leo4_worker_t* w) {
    if (!w) return;
    if (w->pid > 0) {
        (void)kill(w->pid, SIGKILL);
    }
}

static int posix_reap_worker(leo4_worker_t* w, int* exit_status) {
    if (!w) return LEO4_ERR_RUST_SPAWN_FAILED;
    int st = -1;
    if (w->pid > 0) {
        (void)waitpid(w->pid, &st, 0);
        w->pid = 0;
    }
    if (w->sock_fd >= 0) {
        (void)close(w->sock_fd);
        w->sock_fd = -1;
    }
    if (exit_status) *exit_status = st;
    free(w);
    return LEO4_OK;
}

static int posix_send_all(leo4_worker_t* w, const void* buf, size_t len) {
    if (!w || w->sock_fd < 0) return LEO4_ERR_RUST_IPC_FAILED;
    const uint8_t* p = (const uint8_t*)buf;
    size_t left = len;
    while (left) {
        ssize_t n = write(w->sock_fd, p, left);
        if (n < 0) {
            if (errno == EINTR) continue;
            return LEO4_ERR_RUST_IPC_FAILED;
        }
        if (n == 0) return LEO4_ERR_RUST_IPC_FAILED;
        p    += (size_t)n;
        left -= (size_t)n;
    }
    return LEO4_OK;
}

static int posix_recv_exact(leo4_worker_t* w, void* buf, size_t cap, size_t* out_len) {
    if (!w || w->sock_fd < 0) {
        if (out_len) *out_len = 0;
        return LEO4_ERR_RUST_IPC_FAILED;
    }
    uint8_t* p = (uint8_t*)buf;
    size_t left = cap;
    while (left) {
        ssize_t n = read(w->sock_fd, p, left);
        if (n < 0) {
            if (errno == EINTR) continue;
            if (out_len) *out_len = cap - left;
            return LEO4_ERR_RUST_IPC_FAILED;
        }
        if (n == 0) {
            /* EOF mid-message; worker likely died. */
            if (out_len) *out_len = cap - left;
            return LEO4_ERR_RUST_IPC_FAILED;
        }
        p    += (size_t)n;
        left -= (size_t)n;
    }
    if (out_len) *out_len = cap;
    return LEO4_OK;
}

static int posix_alive_worker(leo4_worker_t* w) {
    if (!w || w->pid <= 0) return 0;
    int st = 0;
    pid_t r = waitpid(w->pid, &st, WNOHANG);
    if (r == 0) return 1;        /* still running */
    if (r == w->pid) {
        w->pid = 0;               /* mark reaped */
        return 0;
    }
    return 0;                     /* error: treat as dead */
}

static const leo4_worker_ops_t leo4_posix_ops = {
    .spawn = posix_spawn_worker,
    .kill  = posix_kill_worker,
    .reap  = posix_reap_worker,
    .send  = posix_send_all,
    .recv  = posix_recv_exact,
    .alive = posix_alive_worker,
};

#endif /* __unix__ || __APPLE__ */

/* ─── Windows backend (Phase 9-4c) ────────────────────────────────
 *
 * `CreateProcess` + named pipe + `WaitForSingleObject`.
 *
 * Worker process model:
 *   * Dispatcher creates a duplex named pipe at
 *     `\\.\pipe\leo4_rust_<pid>_<nonce>` (`CreateNamedPipeA`).
 *   * Dispatcher invokes the worker binary with
 *     `--cdylib <path> --ipc-pipe <pipe-name>`.
 *     Worker opens the same name with `CreateFileA` and uses
 *     `ReadFile` / `WriteFile` for I/O.
 *   * No `posix_spawn` / `socketpair` / `dup2` — Windows
 *     subprocess invocation is its own world. Lifecycle uses
 *     `WaitForSingleObject` + `GetExitCodeProcess` instead of
 *     `waitpid`, and `TerminateProcess` instead of `SIGKILL`.
 *
 * This file remains a single C TU; the Windows backend lives
 * behind `#if defined(_WIN32)`. Linux/macOS builds skip it
 * entirely. Verification on Windows happens via the gnullvm
 * Tier 2 CI matrix (when it lands).
 */

#if defined(_WIN32)

struct leo4_worker {
    HANDLE proc;        /* process handle from CreateProcess */
    HANDLE pipe;        /* parent end of duplex named pipe   */
    DWORD  pid;         /* informational only                */
};

static int win_spawn_worker(const char* cdylib_path,
                            leo4_worker_t** out,
                            char*  err_buf, size_t err_cap) {
    if (out) *out = NULL;

    /* Build a unique pipe name: \\.\pipe\leo4_rust_<pid>_<nonce>.
     * Nonce is a simple atomic counter — single-Lean-thread, but
     * defensive in case multiple workers spawn during the same
     * dispatcher session. */
    static _Atomic uint32_t nonce_counter = 0;
    uint32_t nonce = atomic_fetch_add_explicit(&nonce_counter, 1,
                                               memory_order_relaxed);
    char pipe_name[128];
    int n = snprintf(pipe_name, sizeof pipe_name,
                     "\\\\.\\pipe\\leo4_rust_%lu_%u",
                     (unsigned long)GetCurrentProcessId(), nonce);
    if (n < 0 || (size_t)n >= sizeof pipe_name) {
        if (err_buf && err_cap) (void)snprintf(err_buf, err_cap, "pipe name overflow");
        return LEO4_ERR_RUST_SPAWN_FAILED;
    }

    HANDLE pipe = CreateNamedPipeA(
        pipe_name,
        PIPE_ACCESS_DUPLEX,
        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
        1,                          /* one instance */
        65536, 65536,               /* out/in buffer sizes */
        0,                          /* default timeout */
        NULL);                      /* default security */
    if (pipe == INVALID_HANDLE_VALUE) {
        if (err_buf && err_cap)
            (void)snprintf(err_buf, err_cap,
                           "CreateNamedPipeA(%s) failed: GLE=%lu",
                           pipe_name, GetLastError());
        return LEO4_ERR_RUST_SPAWN_FAILED;
    }

    /* Resolve worker binary path. Env override, else fall back to
     * "leo4-rust-worker.exe" and let CreateProcess find it on
     * PATH (NULL lpApplicationName + lpCommandLine == PATH search). */
    const char* worker_bin = getenv("LEO4_RUST_WORKER_BIN");
    if (!worker_bin || !worker_bin[0]) worker_bin = "leo4-rust-worker.exe";

    /* Build the command line. CreateProcess wants a single mutable
     * string. Format:
     *   "<worker-bin>" --cdylib "<cdylib>" --ipc-pipe "<pipe>"  */
    char cmdline[1024];
    int cl = snprintf(cmdline, sizeof cmdline,
                      "\"%s\" --cdylib \"%s\" --ipc-pipe \"%s\"",
                      worker_bin, cdylib_path, pipe_name);
    if (cl < 0 || (size_t)cl >= sizeof cmdline) {
        CloseHandle(pipe);
        if (err_buf && err_cap)
            (void)snprintf(err_buf, err_cap, "cmdline overflow");
        return LEO4_ERR_RUST_SPAWN_FAILED;
    }

    STARTUPINFOA si = { .cb = sizeof si };
    PROCESS_INFORMATION pi = {0};
    BOOL ok = CreateProcessA(
        NULL,              /* lpApplicationName — let cmdline drive */
        cmdline,
        NULL, NULL,        /* default security attrs */
        FALSE,             /* don't inherit handles */
        0,                 /* default creation flags */
        NULL,              /* inherit env */
        NULL,              /* current working dir */
        &si, &pi);
    if (!ok) {
        DWORD gle = GetLastError();
        CloseHandle(pipe);
        if (err_buf && err_cap)
            (void)snprintf(err_buf, err_cap,
                           "CreateProcessA(%s) failed: GLE=%lu",
                           cmdline, gle);
        return gle == ERROR_FILE_NOT_FOUND
            ? LEO4_ERR_RUST_CDYLIB_NOT_FOUND
            : LEO4_ERR_RUST_SPAWN_FAILED;
    }
    CloseHandle(pi.hThread);   /* main thread handle unused */

    /* Wait for the worker to connect. The worker calls
     * `CreateFileA(pipe_name, ...)`; ConnectNamedPipe blocks until
     * the child end is open. */
    BOOL connected = ConnectNamedPipe(pipe, NULL);
    if (!connected && GetLastError() != ERROR_PIPE_CONNECTED) {
        TerminateProcess(pi.hProcess, 1);
        CloseHandle(pi.hProcess);
        CloseHandle(pipe);
        if (err_buf && err_cap)
            (void)snprintf(err_buf, err_cap,
                           "ConnectNamedPipe failed: GLE=%lu",
                           GetLastError());
        return LEO4_ERR_RUST_SPAWN_FAILED;
    }

    leo4_worker_t* w = (leo4_worker_t*)calloc(1, sizeof *w);
    if (!w) {
        TerminateProcess(pi.hProcess, 1);
        CloseHandle(pi.hProcess);
        CloseHandle(pipe);
        return LEO4_ERR_RUST_SPAWN_FAILED;
    }
    w->proc = pi.hProcess;
    w->pipe = pipe;
    w->pid  = pi.dwProcessId;
    *out = w;
    return LEO4_OK;
}

static void win_kill_worker(leo4_worker_t* w) {
    if (!w) return;
    if (w->proc) TerminateProcess(w->proc, 1);
}

static int win_reap_worker(leo4_worker_t* w, int* exit_status) {
    if (!w) return LEO4_ERR_RUST_SPAWN_FAILED;
    if (w->proc) {
        WaitForSingleObject(w->proc, INFINITE);
        DWORD ec = 0;
        if (exit_status) {
            if (GetExitCodeProcess(w->proc, &ec)) *exit_status = (int)ec;
            else                                  *exit_status = -1;
        }
        CloseHandle(w->proc);
        w->proc = NULL;
    }
    if (w->pipe) {
        CloseHandle(w->pipe);
        w->pipe = NULL;
    }
    free(w);
    return LEO4_OK;
}

static int win_send_all(leo4_worker_t* w, const void* buf, size_t len) {
    if (!w || !w->pipe) return LEO4_ERR_RUST_IPC_FAILED;
    const uint8_t* p = (const uint8_t*)buf;
    size_t left = len;
    while (left) {
        DWORD wrote = 0;
        BOOL ok = WriteFile(w->pipe, p, (DWORD)left, &wrote, NULL);
        if (!ok || wrote == 0) return LEO4_ERR_RUST_IPC_FAILED;
        p    += (size_t)wrote;
        left -= (size_t)wrote;
    }
    return LEO4_OK;
}

static int win_recv_exact(leo4_worker_t* w, void* buf, size_t cap, size_t* out_len) {
    if (!w || !w->pipe) {
        if (out_len) *out_len = 0;
        return LEO4_ERR_RUST_IPC_FAILED;
    }
    uint8_t* p = (uint8_t*)buf;
    size_t left = cap;
    while (left) {
        DWORD got = 0;
        BOOL ok = ReadFile(w->pipe, p, (DWORD)left, &got, NULL);
        if (!ok || got == 0) {
            if (out_len) *out_len = cap - left;
            return LEO4_ERR_RUST_IPC_FAILED;
        }
        p    += (size_t)got;
        left -= (size_t)got;
    }
    if (out_len) *out_len = cap;
    return LEO4_OK;
}

static int win_alive_worker(leo4_worker_t* w) {
    if (!w || !w->proc) return 0;
    DWORD r = WaitForSingleObject(w->proc, 0);
    if (r == WAIT_TIMEOUT)  return 1;   /* still running */
    if (r == WAIT_OBJECT_0) return 0;   /* exited */
    return 0;
}

static const leo4_worker_ops_t leo4_windows_ops = {
    .spawn = win_spawn_worker,
    .kill  = win_kill_worker,
    .reap  = win_reap_worker,
    .send  = win_send_all,
    .recv  = win_recv_exact,
    .alive = win_alive_worker,
};

#endif /* _WIN32 */

/* ─── Backend selection ───────────────────────────────────────────
 *
 * The dispatcher body below does not change when backends light up
 * — only this pointer flips.
 */

#if defined(__unix__) || defined(__APPLE__)
static const leo4_worker_ops_t* const leo4_worker_ops = &leo4_posix_ops;
#elif defined(_WIN32)
static const leo4_worker_ops_t* const leo4_worker_ops = &leo4_windows_ops;
#else
static const leo4_worker_ops_t* const leo4_worker_ops = &leo4_stub_ops;
#endif

/* ─── Worker handle cache (lazy spawn, atomic init) ───────────────
 *
 * 9-4a holds a single persistent worker slot. The
 * `#[leo4::export(isolated)]` path (per-call fresh worker) gets a
 * second code path in 9-X; the slot layout below is intentionally
 * extensible — a `worker_slot_t` array could host pooled workers
 * without changing the dispatcher entry's signature.
 */

typedef struct {
    /* (leo4_worker_t*) when spawned, NULL initially.
     * Stored as uintptr_t so we can use `atomic_compare_exchange`
     * on it portably. */
    _Atomic uintptr_t  worker;

    /* 0 = idle, 1 = spawn currently in progress. Single-Lean-thread
     * invariant means contention is impossible today; the field is
     * here for future multi-Lean models (LEO4-DESIGN §16). */
    _Atomic int        spawn_in_progress;

    /* Recycle policy: number of completed calls since this worker
     * spawned. Compared against `leo4_recycle_calls_limit` after
     * each successful request/response round-trip. */
    _Atomic uint64_t   call_count;
} leo4_worker_slot_t;

static leo4_worker_slot_t leo4_persistent_slot = {0};

/* Recycle policy (SPEC/reverse-direction.md §4.3).
 *
 * Read once on first call from env LEO4_RUST_WORKER_RECYCLE_CALLS.
 * Value 0 (the default) disables recycling — the persistent worker
 * stays up across the whole Lean process lifetime. Positive values
 * cap the worker at N completed calls; on the N+1-th call the
 * dispatcher reaps the worker and spawns a fresh one before
 * dispatching.
 *
 * Time-based recycle (LEO4_RUST_WORKER_RECYCLE_SECONDS) is a 9.X
 * follow-on; call-based is what ships today.
 */
static _Atomic int      leo4_recycle_initialized = 0;
static _Atomic uint64_t leo4_recycle_calls_limit = 0;

/* Self-contained u64 parser. Avoids `strtoull` because some
 * libc/clang combinations (glibc 2.38+ under newer clang) emit a
 * versioned `__isoc23_strtoull` reference that the older
 * leanc-bundled sysroot can't resolve at link time. The parser
 * accepts ASCII decimal digits only; returns 0 on empty / invalid
 * input (the caller treats 0 as "recycling disabled"). */
static uint64_t leo4_parse_u64_decimal(const char* s) {
    if (!s) return 0;
    uint64_t r = 0;
    int saw_digit = 0;
    while (*s >= '0' && *s <= '9') {
        r = r * 10u + (uint64_t)(*s - '0');
        saw_digit = 1;
        ++s;
    }
    if (!saw_digit || *s != '\0') return 0;
    return r;
}

static void leo4_recycle_init_once(void) {
    int expected = 0;
    if (!atomic_compare_exchange_strong_explicit(
            &leo4_recycle_initialized, &expected, 1,
            memory_order_acquire, memory_order_relaxed)) {
        return;
    }
    const char* env = getenv("LEO4_RUST_WORKER_RECYCLE_CALLS");
    if (!env || !env[0]) {
        atomic_store_explicit(&leo4_recycle_calls_limit, 0, memory_order_release);
        return;
    }
    uint64_t parsed = leo4_parse_u64_decimal(env);
    atomic_store_explicit(&leo4_recycle_calls_limit, parsed, memory_order_release);
}

/* Reap + clear the persistent slot. Caller holds the
 * single-Lean-thread invariant; no atomics for the worker
 * pointer swap. */
static void leo4_recycle_persistent_slot(void) {
    uintptr_t cur = atomic_exchange_explicit(
        &leo4_persistent_slot.worker, 0, memory_order_acq_rel);
    if (cur) {
        leo4_worker_t* w = (leo4_worker_t*)cur;
        leo4_worker_ops->kill(w);
        leo4_worker_ops->reap(w, NULL);
    }
    atomic_store_explicit(&leo4_persistent_slot.call_count, 0,
                          memory_order_release);
}

/* Resolve the cdylib path the dispatcher should hand to the
 * worker. SPEC §9 chain:
 *   1. env LEO4_RUST_CDYLIB
 *   2. handshake JSON's `cdylib_path` (compile-time constant from
 *      the Lake wrapper — Phase 9-5 fills this in)
 *   3. sibling search
 *
 * 9-4a only honours (1); the const string returned in the env-miss
 * case is "" so the stub spawn errors out with a clear message.
 */
static const char* leo4_cdylib_path(void) {
    const char* env = getenv("LEO4_RUST_CDYLIB");
    if (env && env[0]) return env;
    return "";  /* 9-5 will replace this with the baked-in default */
}

/* Returns the persistent worker, lazily spawning on first call.
 * On spawn failure, leaves the slot empty and propagates the
 * error code (the dispatcher entry returns it to the caller).
 */
static int leo4_get_or_spawn_persistent(leo4_worker_t** out_worker) {
    uintptr_t cur = atomic_load_explicit(&leo4_persistent_slot.worker,
                                         memory_order_acquire);
    if (cur) {
        *out_worker = (leo4_worker_t*)cur;
        return LEO4_OK;
    }
    /* Lazy spawn. Single-Lean-thread invariant means we don't need
     * a full mutex; the in-progress flag here is defensive for
     * future multi-Lean models. */
    int expected = 0;
    if (!atomic_compare_exchange_strong_explicit(
            &leo4_persistent_slot.spawn_in_progress, &expected, 1,
            memory_order_acquire, memory_order_relaxed)) {
        /* Another caller is spawning — busy-wait briefly. With one
         * Lean thread this branch is unreachable today. */
        while (atomic_load_explicit(&leo4_persistent_slot.spawn_in_progress,
                                    memory_order_acquire))
        { /* yield-equivalent: do nothing; compiler emits a tight loop */ }
        return leo4_get_or_spawn_persistent(out_worker);
    }

    const char* cdylib = leo4_cdylib_path();
    char err_buf[256] = {0};
    leo4_worker_t* w = NULL;
    int rc = leo4_worker_ops->spawn(cdylib, &w, err_buf, sizeof err_buf);
    if (rc != LEO4_OK) {
        atomic_store_explicit(&leo4_persistent_slot.spawn_in_progress, 0,
                              memory_order_release);
        *out_worker = NULL;
        return rc;
    }
    atomic_store_explicit(&leo4_persistent_slot.worker, (uintptr_t)w,
                          memory_order_release);
    atomic_store_explicit(&leo4_persistent_slot.spawn_in_progress, 0,
                          memory_order_release);
    *out_worker = w;
    return LEO4_OK;
}

/* ─── Frame I/O helpers (SPEC §5) ─────────────────────────────────
 *
 * `send` / `recv` route through ops. The dispatcher never reads
 * `leo4_worker_t`'s internals; it just hands the opaque pointer
 * back to the ops table.
 */

static int leo4_write_all(leo4_worker_t* w, const void* buf, size_t len) {
    return leo4_worker_ops->send(w, buf, len);
}

static int leo4_read_exact(leo4_worker_t* w, void* buf, size_t cap) {
    size_t got = 0;
    return leo4_worker_ops->recv(w, buf, cap, &got);
    /* NOTE: 9-4b's POSIX `recv` returns LEO4_ERR_RUST_IPC_FAILED on
     * short read so the dispatcher does not need to loop here. The
     * stub backend in 9-4a never reaches this path. */
}

static void leo4_u32_le(uint8_t* out, uint32_t v) {
    out[0] = (uint8_t)(v & 0xffu);
    out[1] = (uint8_t)((v >> 8) & 0xffu);
    out[2] = (uint8_t)((v >> 16) & 0xffu);
    out[3] = (uint8_t)((v >> 24) & 0xffu);
}

static uint32_t leo4_le_u32(const uint8_t* in) {
    return ((uint32_t)in[0])
         | ((uint32_t)in[1] << 8)
         | ((uint32_t)in[2] << 16)
         | ((uint32_t)in[3] << 24);
}

static int leo4_send_request(leo4_worker_t* w,
                             const char* mangled, size_t mangled_len,
                             const uint8_t* args_ptr, size_t args_len)
{
    if (mangled_len > LEO4_MAX_PAYLOAD_BYTES ||
        args_len   > LEO4_MAX_PAYLOAD_BYTES) {
        return LEO4_ERR_DECODE;
    }
    uint8_t header[12];
    leo4_u32_le(header,     LEO4_FRAME_MAGIC);
    leo4_u32_le(header + 4, (uint32_t)mangled_len);
    leo4_u32_le(header + 8, (uint32_t)args_len);
    int rc = leo4_write_all(w, header, sizeof header);
    if (rc != LEO4_OK) return rc;
    if (mangled_len) {
        rc = leo4_write_all(w, mangled, mangled_len);
        if (rc != LEO4_OK) return rc;
    }
    if (args_len) {
        rc = leo4_write_all(w, args_ptr, args_len);
        if (rc != LEO4_OK) return rc;
    }
    return LEO4_OK;
}

static int leo4_recv_response(leo4_worker_t* w,
                              int32_t* status_out,
                              uint8_t* ret_ptr, size_t ret_cap, size_t* ret_len_out,
                              char* detail_buf, size_t detail_cap, size_t* detail_len_out)
{
    uint8_t header[16];
    int rc = leo4_read_exact(w, header, sizeof header);
    if (rc != LEO4_OK) return rc;

    uint32_t magic     = leo4_le_u32(header);
    int32_t  status    = (int32_t)leo4_le_u32(header + 4);
    uint32_t ret_len   = leo4_le_u32(header + 8);
    uint32_t detail_ln = leo4_le_u32(header + 12);

    if (magic != LEO4_FRAME_MAGIC) {
        return LEO4_ERR_RUST_IPC_FAILED;
    }
    if (ret_len > LEO4_MAX_PAYLOAD_BYTES ||
        detail_ln > LEO4_MAX_PAYLOAD_BYTES) {
        return LEO4_ERR_RUST_IPC_FAILED;
    }
    if (ret_len > ret_cap) {
        /* Tell the caller how much they need. Per SPEC §14 the
         * caller may retry with a larger buffer. */
        if (ret_len_out) *ret_len_out = ret_len;
        return LEO4_ERR_BUFFER_TOO_SMALL;
    }

    if (ret_len) {
        rc = leo4_read_exact(w, ret_ptr, ret_len);
        if (rc != LEO4_OK) return rc;
    }
    if (ret_len_out) *ret_len_out = ret_len;

    if (detail_ln) {
        size_t copy_cap = detail_ln;
        if (copy_cap > detail_cap) copy_cap = detail_cap;
        if (detail_buf && copy_cap) {
            rc = leo4_read_exact(w, detail_buf, copy_cap);
            if (rc != LEO4_OK) return rc;
            /* Drain the rest if the caller buffer was smaller. */
            if (detail_ln > copy_cap) {
                uint8_t drain[256];
                size_t left = detail_ln - copy_cap;
                while (left) {
                    size_t chunk = left > sizeof drain ? sizeof drain : left;
                    rc = leo4_read_exact(w, drain, chunk);
                    if (rc != LEO4_OK) return rc;
                    left -= chunk;
                }
            }
        } else {
            uint8_t drain[256];
            size_t left = detail_ln;
            while (left) {
                size_t chunk = left > sizeof drain ? sizeof drain : left;
                rc = leo4_read_exact(w, drain, chunk);
                if (rc != LEO4_OK) return rc;
                left -= chunk;
            }
        }
    }
    if (detail_len_out) *detail_len_out = detail_ln;
    if (status_out)     *status_out     = status;
    return LEO4_OK;
}

/* ─── Dispatcher entry (SPEC §3) ──────────────────────────────────
 *
 * Single C ABI entry. Lean reaches it via @[extern "leo4_rust_call"].
 */

#if defined(_WIN32)
#  define LEO4_RUST_EXPORT __declspec(dllexport)
#elif defined(__GNUC__) || defined(__clang__)
#  define LEO4_RUST_EXPORT __attribute__((visibility("default")))
#else
#  define LEO4_RUST_EXPORT
#endif

/* Spawn a transient (one-shot) worker, send a single request,
 * collect its response, then graceful-shutdown the worker (magic=0
 * request) and reap. Used for `#[leo4::export(isolated)]` exports.
 * See SPEC/reverse-direction.md §4.2.
 */
static int32_t leo4_dispatch_isolated(
    const char* mangled, size_t mangled_len,
    const uint8_t* args_ptr, size_t args_len,
    uint8_t* ret_ptr, size_t ret_cap, size_t* ret_len)
{
    const char* cdylib = leo4_cdylib_path();
    char err_buf[256] = {0};
    leo4_worker_t* w = NULL;
    int rc = leo4_worker_ops->spawn(cdylib, &w, err_buf, sizeof err_buf);
    if (rc != LEO4_OK) return rc;

    /* Send the call request. */
    rc = leo4_send_request(w, mangled, mangled_len, args_ptr, args_len);
    if (rc != LEO4_OK) {
        leo4_worker_ops->kill(w);
        leo4_worker_ops->reap(w, NULL);
        return rc;
    }
    int32_t status = 0;
    char detail[256] = {0};
    size_t detail_len = 0;
    rc = leo4_recv_response(w, &status,
                            ret_ptr, ret_cap, ret_len,
                            detail, sizeof detail, &detail_len);

    /* Graceful shutdown: magic=0 request signals the worker to exit
     * cleanly. We don't read its response (there isn't one). */
    uint8_t shutdown_header[12] = {0};
    (void)leo4_write_all(w, shutdown_header, sizeof shutdown_header);
    leo4_worker_ops->reap(w, NULL);

    if (rc != LEO4_OK) return rc;
    return status;
}

LEO4_RUST_EXPORT
int32_t leo4_rust_call(
    const char* mangled, size_t mangled_len,
    const uint8_t* args_ptr, size_t args_len,
    uint8_t* ret_ptr, size_t ret_cap, size_t* ret_len)
{
    if (!mangled || !ret_len) {
        return LEO4_ERR_RUST_IPC_FAILED;
    }
    *ret_len = 0;

    /* Detect the `iso:` prefix the Lean wrapper emits for
     * `#[leo4::export(isolated)]` exports (SPEC §4.2). Strip it
     * before forwarding the real mangled name to the worker. */
    static const char ISO_PREFIX[] = "iso:";
    static const size_t ISO_PREFIX_LEN = sizeof ISO_PREFIX - 1;
    int isolated = 0;
    if (mangled_len >= ISO_PREFIX_LEN
        && memcmp(mangled, ISO_PREFIX, ISO_PREFIX_LEN) == 0) {
        isolated = 1;
        mangled     += ISO_PREFIX_LEN;
        mangled_len -= ISO_PREFIX_LEN;
    }

    if (isolated) {
        return leo4_dispatch_isolated(mangled, mangled_len,
                                      args_ptr, args_len,
                                      ret_ptr, ret_cap, ret_len);
    }

    /* Persistent-worker path (default). */
    leo4_recycle_init_once();

    /* Recycle the persistent worker if call_count has reached the
     * limit. Reap before the lazy spawn happens inside
     * `leo4_get_or_spawn_persistent`. */
    uint64_t limit = atomic_load_explicit(&leo4_recycle_calls_limit,
                                          memory_order_acquire);
    if (limit != 0) {
        uint64_t cnt = atomic_load_explicit(&leo4_persistent_slot.call_count,
                                            memory_order_acquire);
        if (cnt >= limit) {
            leo4_recycle_persistent_slot();
        }
    }

    leo4_worker_t* worker = NULL;
    int rc = leo4_get_or_spawn_persistent(&worker);
    if (rc != LEO4_OK) {
        return rc;
    }

    rc = leo4_send_request(worker, mangled, mangled_len, args_ptr, args_len);
    if (rc != LEO4_OK) {
        return rc;
    }

    int32_t status = 0;
    char detail[256] = {0};
    size_t detail_len = 0;
    rc = leo4_recv_response(worker, &status,
                            ret_ptr, ret_cap, ret_len,
                            detail, sizeof detail, &detail_len);
    if (rc != LEO4_OK) {
        return rc;
    }

    /* Bump call count for recycle bookkeeping. */
    atomic_fetch_add_explicit(&leo4_persistent_slot.call_count, 1,
                              memory_order_acq_rel);
    return status;
}

/* ─── Helpers exposed for unit tests (Rust side reaches these via
 * the staticlib's link surface). Not part of the stable Lean ABI. */

LEO4_RUST_EXPORT
const leo4_worker_ops_t* leo4_rust_bridge_current_ops(void) {
    return leo4_worker_ops;
}
