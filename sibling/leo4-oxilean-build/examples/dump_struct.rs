//! `cargo run --example dump_struct` prints the Rust struct +
//! `LeanMarshal` impl `leo4-oxilean-build` synthesises for a fixture
//! Lean structure. Useful when iterating on OX2's user-record path.

use leo4_oxilean_build::{synthesize_struct_type, StructField};
use oxilean_codegen::rust_target_backend::RustType;

fn f(name: &str, ty: RustType) -> StructField {
    StructField {
        name: name.to_string(),
        ty,
    }
}

fn main() {
    let out = synthesize_struct_type(
        "MoneyBag",
        &[
            f("major", RustType::Custom("BigNat".to_string())),
            f("minor", RustType::U32),
            f("label", RustType::RustString),
            f("flags", RustType::Vec(Box::new(RustType::Bool))),
        ],
    )
    .unwrap();
    println!("{out}");
}
