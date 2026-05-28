//! OX6 step 13 translator — leaf-crate re-export shim
//! (task #78 follow-up, 2026-05-28).
//!
//! Real source: `sibling/leo4-oxilean-translate/src/lib.rs`.
//! The previous ~1730-line vendor was extracted as a leaf
//! crate to share with `sibling/leo4-oxilean-runner/`.
//! Internal call sites that reference
//! `crate::leo4_translate::translate_decl` etc. resolve
//! through the `pub use` below unchanged.

#[allow(clippy::wildcard_imports)]
pub use leo4_oxilean_translate::*;
