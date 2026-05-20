//! build.rs for the leo4-example-04-mutual-ast demo. Same wiring as
//! the other examples — all three share the sample's shim.

fn main() {
    let lake_build_dir = "../../tests/sample-lean/.lake/build/leo4";
    leo4_build::wire(lake_build_dir).expect("leo4 wire");
}
