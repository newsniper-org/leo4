//! `cargo run --example dump_crate` — prints the full
//! `Cargo.toml` + `src/lib.rs` an emitted `leo4-oxilean-build`
//! crate would contain for a two-export fixture. Useful when
//! iterating on §6 crate-emit shape.

use leo4_oxilean_build::{
    emit_crate, synthesize_canonical_wrapper, TranspileUnit,
};
use oxilean_codegen::rust_target_backend::{RustFn, RustType};

fn unit(name: &str, mangled: &str) -> TranspileUnit {
    let f = RustFn::new(
        name.to_string(),
        vec![("n".to_string(), RustType::U64, false)],
        Some(RustType::U64),
        vec![],
    );
    let wrapper = synthesize_canonical_wrapper(&f).unwrap();
    TranspileUnit {
        type_decls: Vec::new(),
        fn_src: f.emit(),
        wrapper_src: wrapper,
        fn_name: name.to_string(),
        mangled: mangled.to_string(),
    }
}

fn main() {
    let units = vec![
        unit("Sample_addOne", "abc12345_ab_a"),
        unit("Sample_double", "def67890_ab_a"),
    ];
    let g = emit_crate(
        "sample_transpiled",
        &units,
        "{ path = \"../../crates/leo4-abi\" }",
        "0123456789abc",
    );
    println!("=== Cargo.toml ===\n{}", g.manifest);
    println!("=== src/lib.rs ===\n{}", g.lib_rs);
}
