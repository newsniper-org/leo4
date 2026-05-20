//! leo4-wasip3 — WASIp3 backend for leo4.
//!
//! This sibling project is intentionally outside the main leo4 Cargo
//! workspace so it can pin nightly Rust + the `wasip3` crate without
//! perturbing the main workspace's stable-Rust contract. Its
//! `rust-toolchain.toml` and `Cargo.toml` declare the nightly +
//! wasm32-wasip3 target requirements.
//!
//! ## API surface
//!
//! Per the discussion crystallised on 2026-05-20, the user-facing
//! Rust API is **sync on both native and wasm targets**. The wasm
//! side uses `futures::executor::block_on` (or `wasmtime_wasi::block_on`
//! depending on host) inside the sync export to drive any
//! `wasip3` async sub-task — WASIp3 explicitly avoids function
//! coloring so a sync wasm export can call async imports. This lets
//! the macros emitted by `leo4::import!` keep the same shape on
//! both targets (no per-target `cfg!` at the call site, no
//! `async-runtime` runtime dep imposed on default users).
//!
//! ## Wire format
//!
//! The canonical-ABI encode / decode layer is shared with the main
//! workspace via the `leo4-abi` path-dep. Only the dispatch layer
//! (host import call, lifetime / arena semantics) differs from
//! `crates/leo4-native`.
//!
//! ## Status (2026-05-20)
//!
//! Skeleton only. The `wasip3` crate and `wasm32-wasip3` target are
//! upstream-pending stabilisation; this file describes the planned
//! shape so the wire-up is unambiguous when those land. Concrete
//! glue (host import bindings, `block_on` choice, `Lean::open`
//! equivalent for wasm) arrives in the Phase 7 landing.

#![allow(clippy::missing_panics_doc, clippy::missing_errors_doc)]

pub use leo4_abi::{LeanError, LeanMarshal};

/// Placeholder. Mirrors the planned `leo4_native::Lean` surface so
/// downstream code targeting both backends can `use leo4_wasip3::Lean`
/// (under wasm) or `use leo4_native::Lean` (under native) interchangeably.
///
/// **Not yet implemented.** Activates with Phase 7's WASIp3 wire-up.
pub struct Lean {
    _private: (),
}

impl Lean {
    /// Open the WASIp3 component world that exposes the leo4 wrappers.
    ///
    /// # Errors
    ///
    /// Always returns `Err` today — the WASIp3 host import surface
    /// isn't wired up yet.
    pub fn open() -> Result<Self, LeanError> {
        Err(LeanError::new(
            leo4_abi::error_codes::DECODE_ERROR,
            "leo4-wasip3: backend not yet implemented (skeleton only)",
        ))
    }
}
