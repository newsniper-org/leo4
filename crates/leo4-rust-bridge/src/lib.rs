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

#[cfg(test)]
mod tests {
    // The only test in this module is POSIX-specific
    // (see the `#[cfg(unix)]` gate on the test fn below for
    // the underlying Windows-backend deadlock context). Gate
    // the test-only imports + constants on the same cfg so
    // Windows builds don't trip `unused_imports` /
    // `dead_code` warnings under the gate.
    #[cfg(unix)]
    use super::leo4_rust_call;

    /// Reverse-direction passthrough error codes (`SPEC/canonical-abi.md`
    /// §13 + `SPEC/reverse-direction.md` §10). The dispatcher
    /// surfaces one of these whenever any layer fails.
    #[cfg(unix)]
    const LEO4_ERR_RUST_SPAWN_FAILED: i32 = 0x0002_0003;
    #[cfg(unix)]
    const LEO4_ERR_RUST_CDYLIB_NOT_FOUND: i32 = 0x0002_0004;
    #[cfg(unix)]
    const LEO4_ERR_RUST_IPC_FAILED: i32 = 0x0002_0006;

    /// On a 9-4b POSIX build with no worker binary on PATH and
    /// no real cdylib at `LEO4_RUST_CDYLIB`, the dispatcher
    /// must still error out cleanly — either spawn fails
    /// (ENOENT → `_CDYLIB_NOT_FOUND`), the worker comes up but
    /// crashes immediately on the bogus cdylib (→ `_IPC_FAILED`),
    /// or `posix_spawn` itself fails for some other reason
    /// (→ `_SPAWN_FAILED`). Any non-zero code in the
    /// `0x0002_xxxx` range is acceptable; what matters is that
    /// the dispatcher links, executes, and surfaces an error
    /// rather than crashing the process.
    ///
    /// **Windows skipped** — the Windows backend's
    /// `ConnectNamedPipe(pipe, NULL)` (blocking, no overlapped
    /// I/O) deadlocks when the spawned worker dies before
    /// opening the client end of the pipe, which is exactly
    /// this test's scenario (worker errors out on the empty
    /// `--cdylib ""` cmdline). The Windows backend needs an
    /// overlapped `ConnectNamedPipe` + `WaitForMultipleObjects`
    /// on `[connect event, worker process handle]` to detect
    /// early worker exit — tracked as a separate fix on
    /// `shim/leo4_rust_bridge.c` Windows backend.
    #[cfg(unix)]
    #[test]
    fn dispatcher_links_and_errors_cleanly_on_missing_worker() {
        // Make sure neither override is set so we get a
        // deterministic "missing worker" / "missing cdylib"
        // path rather than accidentally hitting a stale binary.
        // SAFETY: setting env vars in tests is safe here — Cargo
        // runs tests in subprocesses isolated from the dev shell.
        unsafe {
            std::env::remove_var("LEO4_RUST_WORKER_BIN");
            std::env::remove_var("LEO4_RUST_CDYLIB");
        }

        let mangled = b"leo4_rust__add__u64_u64";
        let mut ret = [0u8; 8];
        let mut ret_len: usize = 0;
        // SAFETY: valid pointers / lengths; documented out-params.
        let rc = unsafe {
            leo4_rust_call(
                mangled.as_ptr().cast::<core::ffi::c_char>(),
                mangled.len(),
                std::ptr::null(),
                0,
                ret.as_mut_ptr(),
                ret.len(),
                &raw mut ret_len,
            )
        };
        assert!(
            matches!(
                rc,
                LEO4_ERR_RUST_SPAWN_FAILED
                    | LEO4_ERR_RUST_CDYLIB_NOT_FOUND
                    | LEO4_ERR_RUST_IPC_FAILED
            ),
            "expected a 0x0002_xxxx reverse-direction failure, got 0x{rc:08x}"
        );
        // ret_len reset to 0 on error per the C entry's contract.
        assert_eq!(ret_len, 0);
    }
}
