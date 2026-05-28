//! `LeanProc` + `LeanProcInvoker` — trait surface for
//! Rust-native Lean implementations. Pinned at
//! `SPEC/rust-native-lean.md` §2 + §3 (2026-05-21).
//!
//! These traits define what an **adapter crate** (e.g.
//! `leo4-oxilean`, `leo4-<other-rust-native-impl>`) implements
//! against the underlying Lean impl's native Rust API. leo4
//! itself only declares the contract — it doesn't depend on
//! any specific impl. Adapter crates live **outside** the main
//! leo4 workspace (sibling repos or `sibling/leo4-<impl>/`
//! standalone Cargo workspaces).
//!
//! Three transports share the same wire format (canonical-ABI
//! per `SPEC/canonical-abi.md`); this module is the one for
//! the in-process direct-Rust-call case:
//!
//! | Transport | Crate | C ABI? | Adapter location |
//! |---|---|---|---|
//! | leo4-mslean4 | `crates/leo4-mslean4` | yes (`lean.h`) | in-tree |
//! | leo4-wasm | `crates/leo4-wasm` | no | in-tree |
//! | **leo4-rust-native** | `leo4-<impl>` adapter | **no** | **out-of-tree** |

use crate::LeanError;

/// One Rust-native Lean process / context. Owns the impl's
/// environment, elaborator state, and compiled bytecode (or
/// whatever the impl's equivalent abstractions are).
///
/// **Lifetime model**: cheap to construct; cheap to clone
/// (typically `Arc`-cloned). Adapters typically wrap one
/// `oxilean-runtime`-style `Env` per process and `Arc` it.
///
/// **Object-safety**: this trait is object-safe so the
/// downstream consumer can hold a `Box<dyn LeanProc>` and
/// swap impls at runtime without touching its own type
/// parameters.
pub trait LeanProc: Send + Sync {
    /// Recompute the `schema_hash` from the loaded module's
    /// `@[leo4_export]` declarations. Must produce the same
    /// 13-char base32lc value `leo4-rust-emit` (or the impl-
    /// specific build tool that filled this role) recorded
    /// into the corresponding `.leo4-handshake` JSON.
    ///
    /// Used during handshake verification (mirrors what the
    /// native shim and the wasm component's
    /// `verify-handshake` export do for the other two
    /// transports).
    fn schema_hash(&self) -> &str;

    /// `LEO4_ABI_VERSION` the impl was built against
    /// (currently always `1`).
    fn abi_version(&self) -> u32;

    /// Look up + invoke a `@[leo4_export]` body by its
    /// mangled name. `args` is canonical-ABI-encoded per
    /// `SPEC/canonical-abi.md`. Returns canonical-ABI-encoded
    /// result bytes on success.
    ///
    /// # Errors
    /// `LeanError` on dispatch failure (unknown mangled
    /// name → `UNKNOWN_FUNCTION` 0x06) or in-Lean exception
    /// (impl-flavoured passthrough; typically
    /// `LEO4_ERR_RUST_PANIC` `0x0002_0001`).
    fn call(&self, mangled: &str, args: &[u8]) -> Result<Vec<u8>, LeanError>;
}

/// Host-side callback the rust-native impl invokes when its
/// Lean code reaches the moral equivalent of
/// `leo4_rust_call_lean(mangled, args)` — i.e. when the Lean
/// side wants to call a Rust `#[leo4::export]` function.
///
/// Registered once at adapter startup; one callback covers
/// **all** `#[leo4::export]`s of the linked cdylib (dispatch
/// happens inside the callback by looking up `mangled`).
///
/// For `OxiLean` specifically: the adapter calls
/// `oxilean_kernel::ExternRegistry::register(decl)` with
/// `lib_name = "leo4-rust-bridge"` and a closure that
/// delegates to `LeanProcInvoker::invoke`. See
/// `SPEC/rust-native-lean.md` §7.1 for the OxiLean-specific
/// wiring table.
///
/// Re-entrant calls (Lean → Rust → Lean, the Phase 10-B1
/// callback ABI pattern) work transparently — everything is
/// in-process Rust function calls, no IPC frames to design.
pub trait LeanProcInvoker: Send + Sync {
    /// Invoke a `#[leo4::export]` Rust function by its
    /// mangled name. `args` is canonical-ABI-encoded.
    ///
    /// # Errors
    /// `LeanError` for unknown mangled name (`0x06`) or
    /// Rust-side panic caught at the boundary
    /// (`0x0002_0001`).
    fn invoke(&self, mangled: &str, args: &[u8]) -> Result<Vec<u8>, LeanError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sanity check: both traits are object-safe (the SPEC
    // pins this — `Box<dyn LeanProc>` is the type the
    // downstream consumer holds, ditto for the invoker).
    #[allow(dead_code)]
    fn assert_object_safe() {
        let _: Option<Box<dyn LeanProc>> = None;
        let _: Option<Box<dyn LeanProcInvoker>> = None;
    }

    // Demonstrate a minimal impl exists by type-checking
    // alone — proves the contract isn't trivially
    // unimplementable.
    struct DummyProc;
    impl LeanProc for DummyProc {
        fn schema_hash(&self) -> &'static str {
            "0000000000000"
        }
        fn abi_version(&self) -> u32 {
            1
        }
        fn call(&self, _mangled: &str, _args: &[u8]) -> Result<Vec<u8>, LeanError> {
            Err(LeanError::unknown_function("dummy"))
        }
    }

    #[test]
    fn dummy_impl_constructs_and_dispatches() {
        let p: Box<dyn LeanProc> = Box::new(DummyProc);
        assert_eq!(p.schema_hash(), "0000000000000");
        assert_eq!(p.abi_version(), 1);
        let err = p.call("anything", &[]).unwrap_err();
        assert_eq!(err.code, crate::error::error_codes::UNKNOWN_FUNCTION);
    }

    struct DummyInvoker;
    impl LeanProcInvoker for DummyInvoker {
        fn invoke(&self, _mangled: &str, _args: &[u8]) -> Result<Vec<u8>, LeanError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn dummy_invoker_constructs_and_invokes() {
        let inv: Box<dyn LeanProcInvoker> = Box::new(DummyInvoker);
        let out = inv.invoke("anything", &[]).unwrap();
        assert!(out.is_empty());
    }
}
