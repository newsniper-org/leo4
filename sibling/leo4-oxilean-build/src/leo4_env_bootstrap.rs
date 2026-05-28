//! OX5-oxi env bootstrap — leaf-crate re-export shim
//! (task #78, 2026-05-28).
//!
//! Real source: `sibling/leo4-oxilean-bootstrap/src/lib.rs`.
//! This file used to carry the bootstrap + axiom-list
//! implementation directly; the extracted leaf crate
//! `leo4-oxilean-bootstrap` is now the single source of
//! truth, shared with `sibling/leo4-oxilean-runner/`.
//!
//! Existing call sites in `leo4-oxilean-build` keep using
//! `crate::leo4_env_bootstrap::bootstrap_env` etc.
//! unchanged — they resolve through the `pub use` below.

pub use leo4_oxilean_bootstrap::*;
