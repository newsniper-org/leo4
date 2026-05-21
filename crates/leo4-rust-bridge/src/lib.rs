//! Rust-side façade over `libleo4_rust_bridge.a` (built by
//! `build.rs` from `shim/leo4_rust_bridge.c`).
//!
//! Phase 9-4a exposes just the single C entry point so the
//! workspace can exercise the dispatcher from Rust tests. The
//! Lean wrapper (Phase 9-5) reaches the same symbol via
//! `@[extern "leo4_rust_call"]`.

#![allow(clippy::missing_safety_doc)]

use core::ffi::c_char;

unsafe extern "C" {
    /// Dispatcher entry. See `SPEC/reverse-direction.md` §3.
    ///
    /// Returns 0 on success, non-zero on error. Error codes in
    /// the canonical-ABI table (`SPEC/canonical-abi.md` §13) and
    /// the reverse-direction sub-range
    /// (`SPEC/reverse-direction.md` §10).
    pub fn leo4_rust_call(
        mangled: *const c_char,
        mangled_len: usize,
        args_ptr: *const u8,
        args_len: usize,
        ret_ptr: *mut u8,
        ret_cap: usize,
        ret_len: *mut usize,
    ) -> i32;
}

/// Phase 9-4a sanity test: the dispatcher's stub backend errors
/// out on `spawn`, so any call to `leo4_rust_call` returns
/// `LEO4_ERR_RUST_SPAWN_FAILED = 0x00020003`. POSIX and Windows
/// backends (9-4b/c) flip this.
#[cfg(test)]
mod tests {
    use super::leo4_rust_call;

    /// Matches `LEO4_ERR_RUST_SPAWN_FAILED` in
    /// `shim/leo4_rust_bridge.c`.
    const LEO4_ERR_RUST_SPAWN_FAILED: i32 = 0x0002_0003;

    #[test]
    fn dispatcher_links_and_returns_spawn_failed_on_stub() {
        let mangled = b"leo4_rust__add__u64_u64";
        let mut ret = [0u8; 8];
        let mut ret_len: usize = 0;
        // SAFETY: we pass valid pointers / lengths; the dispatcher
        // is a regular C function with documented out-params.
        let rc = unsafe {
            leo4_rust_call(
                mangled.as_ptr().cast::<core::ffi::c_char>(),
                mangled.len(),
                std::ptr::null(),
                0,
                ret.as_mut_ptr(),
                ret.len(),
                &mut ret_len,
            )
        };
        // On 9-4a (stub backend) every call fails at spawn time.
        assert_eq!(
            rc, LEO4_ERR_RUST_SPAWN_FAILED,
            "expected stub-backend spawn failure (0x{LEO4_ERR_RUST_SPAWN_FAILED:08x}), got 0x{rc:08x}"
        );
        // ret_len reset to 0 on error per the C entry's contract.
        assert_eq!(ret_len, 0);
    }
}
