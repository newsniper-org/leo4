use leo4_oxilean_build::lean4_normalize;
fn main() {
    let src = std::fs::read_to_string("/home/ybi/leo4/tests/sample-lean/Sample.lean").unwrap();
    print!("{}", lean4_normalize(&src));
}
