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
    use super::{Color, ParserHandle, Point, Tree, Wrapped};
    leo4::import! {
        fn add(a: u64, b: u64) -> u64;
        fn hello() -> String;
        // P5-b₃-iii: nominal user types route through the
        // fname-only single-instantiation lookup since the macro
        // can't infer Lean-side FQNs from a bare Rust ident.
        fn pointSum(p: Point) -> f64;
        fn colorName(c: Color) -> String;
        fn isLeaf(t: Tree) -> bool;
        fn parserId(h: ParserHandle) -> ParserHandle;
        // P5-b₃-iv: `stringify` is generic on the Lean side (15
        // instantiations). `Wrapped` is a Rust newtype that marshals
        // wire-identically to `u64` but doesn't lower via
        // `rust_type_to_idl`, so without an attribute the macro
        // would fall through to the multi-inst error. The
        // `#[leo4(args = "u64")]` hint nails the u64 instantiation.
        #[leo4(args = "u64")]
        fn stringify(x: Wrapped) -> String;
        // Value-param erasure: `Sample.doubleVal {_N : Nat} (x : UInt32)`
        // — the `{_N}` implicit is erased at the boundary by the plugin,
        // so the boundary signature is just `(u32) -> u32`. The Lean
        // wrapper fills `_N := default` so elaboration succeeds.
        fn doubleVal(x: u32) -> u32;
        // Phase 8 step 2b: external-marshal nominal. `Rat`'s wire
        // format is opaque to the IDL (proof-carrying fields); the
        // shim routes encode/decode through Lean-emitted
        // `leo4_marshal_Rat_dec/_enc` helpers.
        fn addRat(a: leo4::LeanRat, b: leo4::LeanRat) -> leo4::LeanRat;
        // Phase 8 #55: bare Rust `u128` routes through `Leo4.LeanU128`
        // on the Lean side via macro auto-mapping. Wire: 16 bytes LE.
        fn addU128(a: u128, b: u128) -> u128;
        // Phase 8 #56: machine complex multiplication. The Rust type
        // is explicit `LeanComplexF64x2` (not a tuple) — `re` and
        // `im` field names line up with the Lean structure so the
        // wire is byte-identical without aliases.
        fn mulComplexF64x2(
            a: leo4::LeanComplexF64x2,
            b: leo4::LeanComplexF64x2,
        ) -> leo4::LeanComplexF64x2;
        // Phase 7 step 2b: `IO α` return. The Lean side declares
        // `def asyncDouble (n : UInt64) : IO UInt64`; the plugin
        // lifts the return to `future<u64>` in the canonical IDL.
        // The shim wraps the wrapper's call in
        // `lean_io_result_is_ok` / `_get_value` so the wire format
        // after unwrap is the same as a sync u64 return.
        fn asyncDouble(n: u64) -> u64;
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

/// Hand-rolled u64 newtype. Marshals wire-identically to `u64` but
/// the macro can't lower a bare `Wrapped` to an IDL type from the
/// signature alone — that's the case `#[leo4(args = "…")]` exists to
/// disambiguate (P5-b₃-iv).
#[derive(Debug, PartialEq, Clone, Copy)]
struct Wrapped(u64);

impl leo4::LeanMarshal for Wrapped {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        <u64 as leo4::LeanMarshal>::canonical_encode(&self.0, buf);
    }
    fn canonical_decode(
        buf: &[u8],
        off: usize,
    ) -> ::core::result::Result<(Self, usize), leo4::AbiError> {
        let (v, off) = <u64 as leo4::LeanMarshal>::canonical_decode(buf, off)?;
        Ok((Wrapped(v), off))
    }
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

    // P5-b₃-iii: nominal user-type wrappers — the macro picks the
    // single-instantiation Lean export for each, the canonical-ABI
    // encode / decode bridges the derived Rust impl with the
    // Lean-side `deriving LeanMarshal` output.
    let sum = sample::pointSum(&lean, Point { x: 1.5, y: 2.5 })?;
    assert_eq!(sum, 4.0);
    println!("pointSum(Point {{ x: 1.5, y: 2.5 }}) = {sum}");

    for (c, expect) in [
        (Color::Red, "red"),
        (Color::Green, "green"),
        (Color::Blue, "blue"),
    ] {
        let name = sample::colorName(&lean, c.clone())?;
        println!("colorName({c:?}) = {name:?} (expected {expect:?})");
        assert_eq!(name, expect);
    }

    let leaf_only = sample::isLeaf(&lean, Tree::Leaf)?;
    let node = sample::isLeaf(
        &lean,
        Tree::Node(Box::new(Tree::Leaf), Box::new(Tree::Leaf)),
    )?;
    assert!(leaf_only);
    assert!(!node);
    println!("isLeaf(Leaf) = {leaf_only}, isLeaf(Node(...)) = {node}");

    let h = sample::parserId(&lean, ParserHandle { raw: 0xc0ffee })?;
    assert_eq!(h.raw, 0xc0ffee);
    println!("parserId(ParserHandle {{ raw: 0xc0ffee }}) = {h:?}");

    // P5-b₃-iv: attribute-routed multi-instantiation pick. Without
    // `#[leo4(args = "u64")]` the macro can't decide which of the 15
    // `Sample.stringify` instantiations to bind. The attribute names
    // the u64 instantiation explicitly.
    let s = sample::stringify(&lean, Wrapped(42))?;
    assert_eq!(s, "42");
    println!("stringify(Wrapped(42)) = {s:?}");

    // Value-param erasure: `Sample.doubleVal` has a phantom
    // `{_N : Nat}` implicit that the plugin erases at the boundary.
    // The boundary signature is just `(u32) -> u32`, identical to
    // any other monomorphic export.
    let d = sample::doubleVal(&lean, 21)?;
    assert_eq!(d, 42);
    println!("doubleVal(21) = {d}");

    // Phase 8 step 2b: external-marshal `Rat` round-trip across the
    // boundary. 1/3 + 1/6 = 1/2 — `Rat`'s smart constructor `mkRat`
    // normalises on the Lean side, so the result is in lowest terms.
    let a = leo4::LeanRat::from_i64_u64(1, 3);
    let b = leo4::LeanRat::from_i64_u64(1, 6);
    let sum = sample::addRat(&lean, a, b)?;
    println!("addRat(1/3, 1/6) = {sum:?}");
    // Expected: 1/2 = (num=1, den=2) after gcd normalisation.
    assert_eq!(sum.num, leo4::BigInt::from_i64(1));
    assert_eq!(sum.den, leo4::BigNat::from_u64(2));

    // Phase 8 #55: stable `u128` round-trip. Cross the u64 boundary
    // intentionally so the high limb participates.
    let big = (1u128 << 64) | 0xdead_beef_cafe_babe;
    let small = 1u128;
    let sum128 = sample::addU128(&lean, big, small)?;
    assert_eq!(sum128, big + small);
    println!("addU128({big:#x}, 1) = {sum128:#x}");

    // Phase 8 #56: machine-complex multiplication.
    // (2 + 3i) · (4 - i) = (8 + 3) + (-2 + 12)i = 11 + 10i
    let a = leo4::LeanComplexF64x2::new(2.0, 3.0);
    let b = leo4::LeanComplexF64x2::new(4.0, -1.0);
    let prod = sample::mulComplexF64x2(&lean, a, b)?;
    assert_eq!(prod.re, 11.0);
    assert_eq!(prod.im, 10.0);
    println!("mulComplexF64x2((2+3i)(4-i)) = {} + {}i", prod.re, prod.im);

    // Phase 7 step 2b: `IO α` boundary call. Wire is identical to a
    // sync u64 return — the shim unwraps `lean_io_result` before
    // encoding the inner value.
    let doubled = sample::asyncDouble(&lean, 21)?;
    assert_eq!(doubled, 42);
    println!("asyncDouble(21) = {doubled}");

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
