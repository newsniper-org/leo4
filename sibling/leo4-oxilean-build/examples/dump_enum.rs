//! `cargo run --example dump_enum` prints the Rust enum +
//! `LeanMarshal` impl `leo4-oxilean-build` synthesises for a
//! fixture Lean inductive (OX2 user-records last increment).

use leo4_oxilean_build::{synthesize_enum_type, EnumVariant};
use oxilean_codegen::rust_target_backend::RustType;

fn c(name: &str, fields: Vec<RustType>) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        fields,
    }
}

fn main() {
    let out = synthesize_enum_type(
        "Either",
        &[
            c("left", vec![RustType::Custom("BigNat".to_string())]),
            c("right", vec![RustType::RustString]),
        ],
    )
    .unwrap();
    println!("{out}");
}
