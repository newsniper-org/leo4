//! Cross-impl mangling conformance test.
//!
//! Verifies that:
//!   1. `Hash::of_str(canonical_form)` on the Rust side produces the same
//!      `schema_hash` the Lake plugin recorded in `.leo4-handshake`.
//!   2. Every `mangled` field in the Lake-emitted `.leo4-mangling` JSON
//!      is reproducible from the Rust side by running parse → render →
//!      mangle on the matching `.leo4-schema` text.
//!
//! Skips with a warning when the Lake-produced fixtures are missing —
//! run `just smoke-plugin` first to populate them. The full harness
//! (`tests/mangling/run.sh`, wired into `just mangling-test`) does that
//! preflight build itself.

use std::collections::BTreeSet;

use leo4_idl::{mangle, parse, render_canonical, Hash};

const SCHEMA_PATH: &str = "../../tests/sample-lean/.lake/build/leo4/leo4-sample.leo4-schema";
const MANGLING_PATH: &str =
    "../../tests/sample-lean/.lake/build/leo4/leo4-sample.leo4-mangling";
const HANDSHAKE_PATH: &str =
    "../../tests/sample-lean/.lake/build/leo4/leo4-sample.leo4-handshake";

#[test]
fn schema_hash_matches_lake_plugin() {
    let Ok(schema_text) = std::fs::read_to_string(SCHEMA_PATH) else {
        eprintln!("skipping: fixture {SCHEMA_PATH} missing (run `just smoke-plugin`)");
        return;
    };
    let Ok(handshake_text) = std::fs::read_to_string(HANDSHAKE_PATH) else {
        eprintln!("skipping: fixture {HANDSHAKE_PATH} missing");
        return;
    };

    let schema = parse(&schema_text).expect("parse");
    let canonical = render_canonical(
        &schema.package,
        &schema.interface,
        &schema.user_decls,
        &schema.funcs,
        /* pretty */ false,
    );
    let rust_hash = Hash::of_str(&canonical);

    let handshake: serde_json::Value = serde_json::from_str(&handshake_text).expect("handshake json");
    let lean_hash = handshake["schema_hash"].as_str().expect("schema_hash field");
    assert_eq!(
        rust_hash.to_base32lc(),
        lean_hash,
        "schema_hash divergence between Rust and Lean"
    );
}

#[test]
fn mangled_names_match_lake_plugin() {
    let Ok(schema_text) = std::fs::read_to_string(SCHEMA_PATH) else {
        eprintln!("skipping: fixture {SCHEMA_PATH} missing");
        return;
    };
    let Ok(mangling_text) = std::fs::read_to_string(MANGLING_PATH) else {
        eprintln!("skipping: fixture {MANGLING_PATH} missing");
        return;
    };

    let schema = parse(&schema_text).expect("parse");
    let canonical = render_canonical(
        &schema.package,
        &schema.interface,
        &schema.user_decls,
        &schema.funcs,
        false,
    );
    let hash = Hash::of_str(&canonical);

    let mut rust_set: BTreeSet<String> = BTreeSet::new();
    for f in &schema.funcs {
        let args: Vec<_> = f.params.iter().map(|(_, t)| t.clone()).collect();
        rust_set.insert(mangle(&schema.package, &schema.interface, &f.name, &args, hash));
    }

    let lean_json: serde_json::Value = serde_json::from_str(&mangling_text).expect("mangling json");
    let mut lean_set: BTreeSet<String> = BTreeSet::new();
    for entry in lean_json["entries"].as_array().expect("entries") {
        for inst in entry["instantiations"].as_array().expect("instantiations") {
            lean_set.insert(
                inst["mangled"]
                    .as_str()
                    .expect("mangled string")
                    .to_string(),
            );
        }
    }

    assert_eq!(
        rust_set, lean_set,
        "mangled-name set divergence between Rust and Lean"
    );
    assert!(!rust_set.is_empty(), "rust mangled set is empty");
}
