/* leo4_rust_bridge_lean.c — Phase 9-6 Lean-side glue shim.
 *
 * SPEC: SPEC/reverse-direction.md §3.
 *
 * Bridges the Lean-side `@[extern "leo4_rust_call_lean"]` declaration
 * (emitted by `leo4-rust-emit --emit-lean`, Phase 9-5) to the
 * dispatcher entry `leo4_rust_call` (Phase 9-4a, library
 * `libleo4_rust_bridge.a`).
 *
 * Compiled separately from `leo4_rust_bridge.c` because this file
 * is the ONE leo4-side place that touches `lean.h`. The dispatcher
 * and its backends stay free of Lean ABI details, matching the
 * forward-direction split (`<pkg>.leo4-shim.c` vs
 * `crates/leo4-native/`).
 *
 * Build: leanc -c shim/leo4_rust_bridge_lean.c -o leo4_rust_bridge_lean.o
 *        (Phase 9-6 follow-up's Leo4Rust Lake package handles this).
 *
 * Lean declaration this file resolves:
 *
 *     @[extern "leo4_rust_call_lean"]
 *     private opaque leo4RustCallRaw
 *         (mangled : @& String) (args : @& ByteArray)
 *         : BaseIO ByteArray
 *
 * Returned ByteArray layout (no Lean Prod, no ABI guessing):
 *
 *   bytes[0..4]  — status (LE u32). 0 = success; non-zero matches
 *                  the dispatcher's error codes
 *                  (SPEC/canonical-abi.md §13 + SPEC/reverse-direction.md §10).
 *   bytes[4..]   — when status == 0, the call's canonical-ABI
 *                  encoded return payload. When status != 0,
 *                  empty.
 *
 * Earlier drafts used `BaseIO (UInt32 × ByteArray)`, but Lean's
 * Prod codegen for `UInt32 × ByteArray` inlines the UInt32 as a
 * scalar field rather than the boxed lean_object* layout the
 * naive `lean_alloc_ctor(0, 2, 0)` / `lean_box_uint32` pair
 * produces — the ABI mismatch surfaced as garbage `status`
 * values in the Lean wrapper. The single-ByteArray return is
 * unambiguous: caller reads the leading 4 LE bytes and slices
 * the tail.
 */

#include <stddef.h>
#include <stdint.h>

#include <lean/lean.h>

/* Dispatcher entry from `libleo4_rust_bridge.a` (Phase 9-4a). */
extern int32_t leo4_rust_call(
    const char* mangled, size_t mangled_len,
    const uint8_t* args_ptr, size_t args_len,
    uint8_t* ret_ptr, size_t ret_cap, size_t* ret_len);

/* Error codes the dispatcher may surface — kept in sync with
 * `shim/leo4_rust_bridge.c` and `SPEC/canonical-abi.md` §13. */
#define LEO4_OK                       0
#define LEO4_ERR_BUFFER_TOO_SMALL     0x00000007

/* Default response buffer (4 KiB minus the 4-byte status
 * prefix). Doubles on BUFFER_TOO_SMALL and retries once; the
 * dispatcher writes the required *return-payload* size into
 * `*ret_len`, so a single retry with that size + 4 always
 * suffices. */
#define LEO4_RUST_INITIAL_RET_CAP     4096u

/* Write a little-endian u32 into `out[0..4]`. */
static inline void leo4_lean_u32_le(uint8_t* out, uint32_t v) {
    out[0] = (uint8_t)(v & 0xffu);
    out[1] = (uint8_t)((v >> 8) & 0xffu);
    out[2] = (uint8_t)((v >> 16) & 0xffu);
    out[3] = (uint8_t)((v >> 24) & 0xffu);
}

/* Lean extern entry. Always returns `lean_io_result_mk_ok(ByteArray)` —
 * dispatcher / user-function failures are surfaced through the
 * 4-byte LE u32 status prefix of the returned ByteArray, not
 * through Lean's IO error path. The typed Lean wrapper emitted
 * by `--emit-lean` parses the prefix and raises `IO.userError`
 * itself on non-zero status.
 *
 * Argument annotation rules (`@&`): both `mangled` and `args` are
 * borrowed; we must NOT call `lean_dec` on them. The ByteArray
 * we allocate passes into the IO result, which the caller drops.
 */
LEAN_EXPORT lean_object* leo4_rust_call_lean(
    b_lean_obj_arg mangled,    /* @& String */
    b_lean_obj_arg args,       /* @& ByteArray */
    lean_object*   /*world*/   /* IO RealWorld token, passed through */
) {
    /* String -> (cstr, size). `lean_string_size` includes the
     * trailing NUL; subtract it so the dispatcher sees the
     * mangled-name length without the terminator. */
    const char* m_cstr  = lean_string_cstr(mangled);
    size_t      m_len   = lean_string_size(mangled);
    if (m_len > 0) m_len -= 1;  /* drop NUL */

    /* ByteArray -> (ptr, size). */
    uint8_t const* a_ptr = lean_sarray_cptr((lean_object*)args);
    size_t         a_len = lean_sarray_size(args);

    /* Allocate the response ByteArray with room for the 4-byte
     * status prefix plus the payload. Capacity here covers BOTH;
     * the dispatcher writes only into the payload region (after
     * the prefix). */
    size_t cap = LEO4_RUST_INITIAL_RET_CAP;
    lean_object* ret_array = lean_alloc_sarray(1, cap, cap);
    uint8_t*     r_base    = lean_sarray_cptr(ret_array);
    size_t       payload_len = 0;

    int32_t status = leo4_rust_call(
        m_cstr, m_len, a_ptr, a_len,
        r_base + 4,                /* payload starts after the 4-byte prefix */
        cap - 4,                   /* payload capacity */
        &payload_len);

    if (status == LEO4_ERR_BUFFER_TOO_SMALL) {
        /* `payload_len` carries the required payload size. Re-alloc
         * to (size + 4) so the prefix fits as well, retry once. */
        size_t needed = payload_len + 4;
        if (needed <= cap) needed = cap * 2;  /* defensive */
        lean_dec(ret_array);
        cap = needed;
        ret_array = lean_alloc_sarray(1, cap, cap);
        r_base    = lean_sarray_cptr(ret_array);
        payload_len = 0;
        status = leo4_rust_call(
            m_cstr, m_len, a_ptr, a_len,
            r_base + 4, cap - 4, &payload_len);
    }

    /* Stamp the status prefix. */
    leo4_lean_u32_le(r_base, (uint32_t)status);

    /* On success the ByteArray's logical size is 4 + payload_len.
     * On error it stays 4 (status prefix only, no payload). */
    size_t total = 4 + (status == LEO4_OK ? payload_len : 0);
    if (total <= cap) {
        lean_sarray_set_size(ret_array, total);
    }

    return lean_io_result_mk_ok(ret_array);
}
