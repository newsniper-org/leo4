use leo4_idl::parse;

#[test]
fn parses_lake_plugin_sample_schema() {
    let path = "../../tests/sample-lean/.lake/build/leo4/leo4_sample.leo4-schema";
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("schema fixture missing at {path}; run `just smoke-plugin` first"));
    let schema = parse(&text).expect("parse should succeed");
    assert_eq!(schema.package, "leo4-sample");
    assert_eq!(schema.interface, "Sample");
    // Sample declares: Point (record), Color (enum), Tree (variant),
    // ParserHandle (resource), Pair (generic record), Either (generic
    // variant), Rat (external-marshal — Phase 8 step 2), the Phase 6
    // `mutual { Expr; Stmt }` cluster, plus `Leo4.LeanU128` (Phase 8
    // #55 wide-integer carrier) — nine entries total.
    assert_eq!(schema.user_decls.len(), 9, "expected 9 user decls");
    assert!(schema.funcs.len() >= 9, "expected at least 9 funcs, got {}", schema.funcs.len());
}
