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
 *        (link alongside `libleo4_rust_bridge.a` per Phase 9-6).
 *
 * Lean declaration this file resolves:
 *
 *     @[extern "leo4_rust_call_lean"]
 *     private opaque leo4RustCallRaw
 *         (mangled : @& String) (args : @& ByteArray)
 *         : BaseIO (UInt32 × ByteArray)
 *
 * Lean's compiler lowers this into a C function that takes
 *   (lean_object* mangled, lean_object* args, lean_object* world)
 * and returns a `lean_io_result` wrapping the `(UInt32 × ByteArray)`
 * tuple. The third argument is the IO RealWorld token — borrowed,
 * passed through unchanged.
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

/* Default response buffer (4 KiB). Doubles on
 * BUFFER_TOO_SMALL and retries; the dispatcher passes the
 * required size out via `*ret_len`, so a single retry suffices. */
#define LEO4_RUST_INITIAL_RET_CAP     4096u

/* Build the `(UInt32 × ByteArray)` tuple Lean expects on the
 * happy path.
 *
 * In Lean 4 a product type `(α × β)` lowers to a single ctor with
 * two fields — `Prod.mk α β`, ctor index 0, 2 object pointers, 0
 * scalar bytes. UInt32 is boxed (sub-word values land in
 * lean_object* via `lean_box_uint32`).
 */
static lean_object* leo4_rust_make_result_tuple(uint32_t status,
                                                lean_object* ret_array) {
    lean_object* tuple = lean_alloc_ctor(0, 2, 0);
    lean_ctor_set(tuple, 0, lean_box_uint32(status));
    lean_ctor_set(tuple, 1, ret_array);
    return tuple;
}

/* Lean extern entry. Always returns `lean_io_result_mk_ok(tuple)` —
 * dispatcher / user-function failures are surfaced through the
 * `status` field of the tuple, not through Lean's IO error path.
 * This mirrors the wrapper's design (the typed Lean wrapper
 * emitted by `--emit-lean` checks `status` and raises
 * `IO.userError` itself).
 *
 * Argument annotation rules (`@&`): both `mangled` and `args` are
 * borrowed; we must NOT call `lean_dec` on them. We DO own the
 * `ret_array` we allocate and the boxed status — they pass into
 * the tuple, which the caller drops.
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

    /* Allocate the response buffer. We use `lean_alloc_sarray(1, n, n)`:
     *   elem_size = 1 (byte), size = used count, capacity = same.
     * We pre-set `size` to the initial cap so `lean_sarray_cptr` is
     * usable; after the dispatcher returns we shrink via
     * `lean_sarray_set_size` to the actual `ret_len`. */
    size_t cap = LEO4_RUST_INITIAL_RET_CAP;
    lean_object* ret_array = lean_alloc_sarray(1, cap, cap);
    uint8_t*     r_ptr     = lean_sarray_cptr(ret_array);
    size_t       r_len     = 0;

    int32_t status = leo4_rust_call(m_cstr, m_len, a_ptr, a_len,
                                    r_ptr, cap, &r_len);

    /* BUFFER_TOO_SMALL retry: dispatcher wrote required size into
     * r_len. Re-allocate the response buffer to exactly that size
     * and retry once. Further retries fail with the same code (the
     * worker's wrapper should never bounce twice). */
    if (status == LEO4_ERR_BUFFER_TOO_SMALL) {
        cap = r_len;
        /* Release the too-small buffer. `lean_dec` is the
         * generic drop; for a freshly-allocated sarray with rc=1
         * it frees immediately. */
        lean_dec(ret_array);
        ret_array = lean_alloc_sarray(1, cap, cap);
        r_ptr     = lean_sarray_cptr(ret_array);
        r_len     = 0;
        status    = leo4_rust_call(m_cstr, m_len, a_ptr, a_len,
                                   r_ptr, cap, &r_len);
    }

    /* Shrink the sarray's `m_size` to the actual byte count. The
     * underlying allocation stays at `cap`; this only updates the
     * length the Lean ByteArray reports. */
    if (r_len <= cap) {
        lean_sarray_set_size(ret_array, r_len);
    }
    /* On the (impossible) `r_len > cap` outcome we leave the
     * sarray reporting the over-allocated capacity rather than
     * panic; the dispatcher's contract says this cannot happen. */

    return lean_io_result_mk_ok(
        leo4_rust_make_result_tuple((uint32_t)status, ret_array)
    );
}
