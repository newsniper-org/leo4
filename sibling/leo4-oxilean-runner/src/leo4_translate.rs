//! OX6 step 13 translator — leaf-crate re-export shim
//! (task #78 follow-up, 2026-05-28).
//!
//! Real source: `sibling/leo4-oxilean-translate/src/lib.rs`.
//! Replaces the vendored ~1730-line copy that previously
//! lived here. Single source of truth shared with
//! `sibling/leo4-oxilean-build/src/leo4_translate.rs` (also
//! a shim now). Internal call sites that reference
//! `crate::leo4_translate::translate_decl` etc. resolve
//! through the `pub use` below unchanged.

#[allow(clippy::wildcard_imports)]
pub use leo4_oxilean_translate::*;
