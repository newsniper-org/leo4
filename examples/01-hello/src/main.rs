//! examples/01-hello — smallest end-to-end leo4 demo.
//!
//! Calls `Sample.add (a b : UInt64) : UInt64` from Rust by:
//!
//! 1. opening `tests/sample-lean/.lake/build/leo4/leo4_sample.leo4-shim.so`
//!    via `leo4::Lean::open` (handshake + Lean runtime init + wrapper
//!    module init);
//! 2. routing the call through the `leo4::import!`-generated wrapper,
//!    which encodes the two `u64` args as LE bytes and decodes the
//!    LE u64 return.
//!
//! Run:
//!
//! ```sh
//! cd tests/sample-lean && lake build   # produce / refresh shim
//! cd ../..
//! just smoke-plugin                    # produce / refresh .so
//! cargo run -p leo4-example-01-hello
//! ```

mod sample {
    leo4::import! {
        fn add(a: u64, b: u64) -> u64;
        fn hello() -> String;
    }
}

// P5-b₃-ii: derived LeanMarshal instances mirroring the Sample
// package's user-defined nominal types. Round-trip is checked
// against the wire-format the Lean-side `deriving LeanMarshal`
// handler emits.
#[derive(leo4::LeanMarshal, Debug, PartialEq, Clone)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(leo4::LeanMarshal, Debug, PartialEq, Clone)]
enum Color {
    Red,
    Green,
    Blue,
}

#[derive(leo4::LeanMarshal, Debug, PartialEq, Clone)]
enum Tree {
    Leaf,
    Node(Box<Tree>, Box<Tree>),
}

#[derive(leo4::LeanMarshal, Debug, PartialEq, Clone)]
#[leo4(resource)]
struct ParserHandle {
    raw: u64,
}

#[derive(leo4::LeanMarshal, Debug, PartialEq, Clone)]
struct Pair<A, B> {
    fst: A,
    snd: B,
}

fn main() -> Result<(), leo4::LeanError> {
    // env! values come from leo4_build::wire in build.rs.
    let lean = leo4::Lean::open(
        env!("LEO4_SHIM_SO"),
        env!("LEO4_HANDSHAKE_FILE"),
    )?;

    let result = sample::add(&lean, 2, 3)?;
    assert_eq!(result, 5);
    println!("add(2, 3) = {result}");

    let big = sample::add(&lean, u64::MAX - 1, 1)?;
    assert_eq!(big, u64::MAX);
    println!("add(u64::MAX - 1, 1) = {big}");

    // P5-b₂ composite check: String return type. `Sample.hello`
    // exercises the LeanMarshal codegen for variable-size payloads.
    let greeting = sample::hello(&lean)?;
    assert_eq!(greeting, "hello, leo4");
    println!("hello() = {greeting:?}");

    // P5-b₃-ii: derive macro round-trip across the four shapes.
    nominal_derive_check();

    // Phase-5 exit criterion: handshake-mismatch detection. Mutate
    // the handshake JSON's `schema_hash_bytes` and re-open the same
    // shim; the loader must refuse with code 5 instead of running.
    handshake_mismatch_check()?;

    Ok(())
}

/// Pure-Rust derive round-trip across record / enum / variant /
/// resource / generic-record shapes. Uses
/// `leo4::encode`/`leo4::decode` to drive each instance back and
/// forth through the canonical-ABI wire format.
fn nominal_derive_check() {
    // record
    let p = Point { x: 1.0, y: 2.5 };
    let bytes = leo4::encode(&p);
    let p2: Point = leo4::decode(&bytes).expect("Point decode");
    assert_eq!(p, p2);

    // enum (all-unit)
    for c in [Color::Red, Color::Green, Color::Blue] {
        let bytes = leo4::encode(&c);
        let c2: Color = leo4::decode(&bytes).expect("Color decode");
        assert_eq!(c, c2);
    }

    // variant (self-recursive)
    let t = Tree::Node(
        Box::new(Tree::Leaf),
        Box::new(Tree::Node(Box::new(Tree::Leaf), Box::new(Tree::Leaf))),
    );
    let bytes = leo4::encode(&t);
    let t2: Tree = leo4::decode(&bytes).expect("Tree decode");
    assert_eq!(t, t2);

    // resource (opaque u64)
    let h = ParserHandle { raw: 0xdead_beef_cafe_babe };
    let bytes = leo4::encode(&h);
    let h2: ParserHandle = leo4::decode(&bytes).expect("ParserHandle decode");
    assert_eq!(h, h2);

    // generic record
    let pair: Pair<u32, String> = Pair { fst: 7, snd: "leo4".into() };
    let bytes = leo4::encode(&pair);
    let pair2: Pair<u32, String> = leo4::decode(&bytes).expect("Pair decode");
    assert_eq!(pair, pair2);

    println!(
        "derive(LeanMarshal) round-trip: record / enum / variant / resource / generic-record all green"
    );
}

/// Verifies that `Lean::open` rejects a shim whose schema_hash_bytes
/// disagrees with the shim's compiled-in `leo4_schema_hash_be` array.
fn handshake_mismatch_check() -> Result<(), leo4::LeanError> {
    let handshake_path = env!("LEO4_HANDSHAKE_FILE");
    let shim_path = env!("LEO4_SHIM_SO");
    let original = std::fs::read_to_string(handshake_path)
        .expect("read original handshake");
    let mut json: serde_json::Value =
        serde_json::from_str(&original).expect("parse original handshake");
    // Flip the high nibble of the first byte so the first hex digit
    // changes — guaranteed mismatch against the shim's baked-in
    // schema_hash_be.
    let hex = json["schema_hash_bytes"]
        .as_str()
        .expect("schema_hash_bytes")
        .to_string();
    let mut chars: Vec<char> = hex.chars().collect();
    chars[0] = if chars[0] == '0' { 'f' } else { '0' };
    json["schema_hash_bytes"] =
        serde_json::Value::String(chars.into_iter().collect());
    let tampered_path = std::env::temp_dir().join("leo4-01-hello-tampered.handshake");
    std::fs::write(&tampered_path, json.to_string()).expect("write tampered");

    match leo4::Lean::open(shim_path, &tampered_path) {
        Ok(_) => panic!("expected handshake mismatch but Lean::open succeeded"),
        Err(e) if e.code == 5 => {
            println!("handshake-mismatch check: rejected (code={}, detail truncated)", e.code);
            Ok(())
        }
        Err(e) => panic!(
            "expected code 5 (LEO4_ERR_HANDSHAKE_MISMATCH); got code {}: {}",
            e.code, e.detail
        ),
    }
}
