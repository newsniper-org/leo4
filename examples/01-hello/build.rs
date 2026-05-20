//! build.rs for the leo4-example-01-hello demo.
//!
//! Wires the example to the `tests/sample-lean` Lake build outputs.
//! Run `cd tests/sample-lean && lake build && cd ../.. && just smoke-plugin`
//! (or whatever incantation produces the shim) before `cargo run -p
//! leo4-example-01-hello`.

fn main() {
    // The path is relative to this crate's manifest dir
    // (`examples/01-hello/`), so we hop up two levels to the
    // workspace root and back down into the sample's Lake output.
    let lake_build_dir = "../../tests/sample-lean/.lake/build/leo4";
    leo4_build::wire(lake_build_dir).expect("leo4 wire");
}
