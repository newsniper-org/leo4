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

#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <stdatomic.h>

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

static const leo4_worker_ops_t leo4_stub_ops = {
    .spawn = stub_spawn,
    .kill  = stub_kill,
    .reap  = stub_reap,
    .send  = stub_send,
    .recv  = stub_recv,
    .alive = stub_alive,
};

/* ─── Backend selection ───────────────────────────────────────────
 *
 * 9-4a wires the stub everywhere. 9-4b fills in the POSIX branch
 * with `posix_spawn` + `socketpair` + `wait4`; 9-4c fills in the
 * Windows branch with `CreateProcess` + named pipe. The dispatcher
 * body below does not change when backends light up — only this
 * pointer flips.
 */

#if defined(__unix__) || defined(__APPLE__)
/* TODO(9-4b): replace with `&leo4_posix_ops` once that backend
 * lands in this file under a parallel `#ifdef`. */
static const leo4_worker_ops_t* const leo4_worker_ops = &leo4_stub_ops;
#elif defined(_WIN32)
/* TODO(9-4c): replace with `&leo4_windows_ops`. */
static const leo4_worker_ops_t* const leo4_worker_ops = &leo4_stub_ops;
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
} leo4_worker_slot_t;

static leo4_worker_slot_t leo4_persistent_slot = {0};

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

    leo4_worker_t* worker = NULL;
    int rc = leo4_get_or_spawn_persistent(&worker);
    if (rc != LEO4_OK) {
        return rc;
    }
    /* 9-4b will exchange the handshake frame here on first call
     * and verify the schema_hash against a compile-time pinned
     * value (filled by the Lake wrapper, 9-5). For now we just
     * proceed straight to the request loop — the stub backend's
     * `spawn` already errored out, so this branch is unreachable
     * on the 9-4a build. */

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
    return status;
}

/* ─── Helpers exposed for unit tests (Rust side reaches these via
 * the staticlib's link surface). Not part of the stable Lean ABI. */

LEO4_RUST_EXPORT
const leo4_worker_ops_t* leo4_rust_bridge_current_ops(void) {
    return leo4_worker_ops;
}
