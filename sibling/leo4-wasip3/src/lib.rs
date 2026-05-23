//! leo4-wasip3 — WASIp3 backend for leo4.
//!
//! Compiles on **stable Rust** targeting `wasm32-wasip2` against
//! the `wasip3` crate's WASIp3 API bindings. The earlier nightly
//! requirement (and the WASIp3 target itself being tier 3) is
//! moot for our purposes — `wasip3` v0.6 ships the WASIp3 surface
//! as compat shims on wasip2's Component Model.
//!
//! ## API surface
//!
//! Per the design crystallised on 2026-05-20, the user-facing
//! Rust API is **sync on both native and wasm targets**. The
//! wasm side uses `futures::executor::block_on` inside the sync
//! export to drive any async wasip3 import — WASIp3 explicitly
//! avoids function coloring so a sync wasm export can call
//! async imports. `leo4::import!` emits the same `fn add(...)`
//! signature on native and wasm; the wasm side wraps an internal
//! async future in `block_on`. No per-target `.await` at the
//! call site, no `async-runtime` dep forced on default users.
//!
//! ## Wire format
//!
//! The canonical-ABI encode / decode layer is shared with the
//! main workspace via the `leo4-abi` path-dep. Only the dispatch
//! layer (host import call vs. libloading `.so`) differs from
//! `crates/leo4-mslean4`.
//!
//! ## Status (2026-05-21)
//!
//! Skeleton with real `wasip3` dep wired in. `Lean::open` still
//! returns an error because the leo4-specific WIT interface
//! describing the host imports hasn't been pinned yet — that's
//! a follow-up spec design task. Once `SPEC/wit/leo4-host.wit`
//! exists, `wit-bindgen` (or `wasip3-bindgen`) generates the
//! imports we wrap here.

#![allow(clippy::missing_panics_doc, clippy::missing_errors_doc)]

pub use leo4_abi::{LeanError, LeanMarshal};

/// Placeholder mirroring the planned `leo4_mslean4::Lean` surface
/// so downstream code targeting both backends can
/// `use leo4_wasip3::Lean` interchangeably under wasm.
///
/// Future fields (when the WIT design lands):
///   - host import handle(s) for the schema-hash / handshake
///     entry point
///   - import handles for each `@[leo4_export]` mangled symbol,
///     resolved through `wasip3` Component Model interfaces
///   - a per-callsite cache analogous to `leo4_mslean4::Lean`'s
///     `Mutex<HashMap>` (but probably just `RefCell` since the
///     wasm guest is single-threaded by default)
pub struct Lean {
    _private: (),
}

impl Lean {
    /// Open the WASIp3 component world that exposes the leo4
    /// wrappers.
    ///
    /// **Stub today.** Returns `Err`. The implementation needs:
    ///
    /// 1. A pinned WIT interface for leo4 host imports (TBD —
    ///    `SPEC/wit/leo4-host.wit`).
    /// 2. `wit-bindgen` / `wasip3-bindgen` invocation in this
    ///    crate's `build.rs` to generate the imports.
    /// 3. Replace the body with:
    ///    `let imports = leo4::host::call_handshake_etc()?;`
    ///    `Ok(Lean { imports })` with `block_on` wrapping any
    ///    async wasip3 call.
    ///
    /// # Errors
    ///
    /// Always returns `Err` until step (2) is done.
    pub fn open() -> Result<Self, LeanError> {
        // Demonstrates the `block_on` pattern that future
        // dispatch sites will use. Replace the future body with
        // the actual wasip3 import call once the WIT is pinned.
        let result: Result<Self, LeanError> =
            futures::executor::block_on(async {
                Err(LeanError::new(
                    leo4_abi::error_codes::DECODE_ERROR,
                    "leo4-wasip3: backend not yet wired (WIT interface design pending — see SPEC/wit/leo4-host.wit follow-up)",
                ))
            });
        result
    }
}
