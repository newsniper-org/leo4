//! build.rs for the leo4-example-02-roundtrip demo.
//!
//! Wires the example to the `tests/sample-lean` Lake build outputs.
//! Same wiring as `examples/01-hello/build.rs`; both share the same
//! shim (`leo4_sample.leo4-shim.so`).

fn main() {
    let lake_build_dir = "../../tests/sample-lean/.lake/build/leo4";
    leo4_build::wire(lake_build_dir).expect("leo4 wire");
}
