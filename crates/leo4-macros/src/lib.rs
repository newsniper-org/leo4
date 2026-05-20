//! leo4-macros — proc-macro entry points.
//!
//! All real work lives in `leo4-macros-backend`; this crate is the
//! `proc-macro = true` wrapper that the user (or `leo4` façade)
//! re-exports.
//!
//! Surface:
//!
//! ```ignore
//! leo4::import! {
//!     fn add(a: u64, b: u64) -> u64;
//!     fn max_scalar(a: i64, b: i64) -> i64;
//! }
//! ```
//!
//! See `leo4_macros_backend::expand_import` for the expansion rules
//! and the supported scalar set.

use proc_macro::TokenStream;

/// `leo4::import!` — generate Rust wrappers for `@[leo4_export]`
/// definitions on the Lean side.
#[proc_macro]
pub fn import(input: TokenStream) -> TokenStream {
    leo4_macros_backend::expand_import(input.into()).into()
}
