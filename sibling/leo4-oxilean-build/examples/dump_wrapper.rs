//! `cargo run --example dump_wrapper` — prints the canonical-ABI
//! boundary wrapper `leo4-oxilean-build` synthesises for a fixture
//! transpiled fn. Useful when iterating on §5 wrapper emission.

use leo4_oxilean_build::synthesize_canonical_wrapper;
use oxilean_codegen::rust_target_backend::{RustFn, RustType};

fn main() {
    let f = RustFn::new(
        "addOne",
        vec![("n".to_string(), RustType::U64, false)],
        Some(RustType::U64),
        vec![],
    );
    println!("{}", synthesize_canonical_wrapper(&f).unwrap());
}
