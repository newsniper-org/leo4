//! examples/02-roundtrip — Phase 5 exit demo for List + Nat wires.
//!
//! Exercises three pieces of the canonical ABI that
//! `examples/01-hello/` doesn't touch:
//!
//! 1. `list<T>` as a *parameter*  — `Sample.listSumU64(xs : List UInt64)`,
//!    `Sample.listConcat(xs : List String)`.
//! 2. `list<T>` as a *return type* — `Sample.echoes(xs : List UInt32)
//!    (n : Nat) : List UInt32`, which concatenates `n` copies of
//!    `xs`.
//! 3. `bignat` as a parameter — the `n : Nat` slot of `echoes`.
//!
//! Also reuses the multi-instantiation `Sample.listLen` so the
//! `<#[leo4(args = ...)]>` attribute path lands one more time on a
//! `list<u32>` shape (P5-b₃-iv).
//!
//! Run:
//!
//! ```sh
//! cd tests/sample-lean && lake build && cd ../..
//! just smoke-plugin
//! cargo run -p leo4-example-02-roundtrip
//! ```

mod sample {
    leo4::import! {
        // List of u64 → u64. Pure scalar wire on the return side.
        fn listSumU64(xs: Vec<u64>) -> u64;
        // List of String → String. Variable-size list element + variable-size return.
        fn listConcat(xs: Vec<String>) -> String;
        // List + Nat → List. The Phase 5 headliner.
        fn echoes(xs: Vec<u32>, n: leo4::BigNat) -> Vec<u32>;
        // Multi-instantiation: route to the `list<u32>` instantiation
        // by inferred mangling (no attribute needed because Vec<u32>
        // lowers cleanly via `rust_type_to_idl`).
        fn listLen(xs: Vec<u32>) -> leo4::BigNat;
    }
}

fn main() -> Result<(), leo4::LeanError> {
    let lean = leo4::Lean::open(
        env!("LEO4_SHIM_SO"),
        env!("LEO4_HANDSHAKE_FILE"),
    )?;

    // (1) list<u64> argument, scalar return.
    let sum = sample::listSumU64(&lean, vec![1, 2, 3, 4, 5])?;
    assert_eq!(sum, 15);
    println!("listSumU64([1..=5]) = {sum}");

    // (2) list<string> argument, string return.
    let joined = sample::listConcat(&lean, vec!["leo".into(), "4".into(), "!".into()])?;
    assert_eq!(joined, "leo4!");
    println!("listConcat([leo, 4, !]) = {joined:?}");

    // (3) list<u32> + bignat → list<u32>: the Phase 5 headline path.
    let xs = vec![10u32, 20, 30];
    let three = leo4::BigNat::from_u64(3);
    let got = sample::echoes(&lean, xs.clone(), three)?;
    assert_eq!(got, vec![10, 20, 30, 10, 20, 30, 10, 20, 30]);
    println!("echoes({xs:?}, 3) = {got:?}");

    // n = 0 corner case: empty result.
    let empty = sample::echoes(&lean, vec![42u32, 43], leo4::BigNat::from_u64(0))?;
    assert!(empty.is_empty());
    println!("echoes([42, 43], 0) = {empty:?}");

    // Larger Nat (still single-limb) to exercise the bignat wire
    // format past the trivial 0 / 1 cases.
    let many = sample::echoes(&lean, vec![7u32], leo4::BigNat::from_u64(100))?;
    assert_eq!(many.len(), 100);
    assert!(many.iter().all(|&x| x == 7));
    println!("echoes([7], 100) — length {}, all 7s", many.len());

    // (4) Multi-instantiation generic listLen.
    let n = sample::listLen(&lean, vec![0u32; 17])?;
    assert_eq!(n, leo4::BigNat::from_u64(17));
    println!("listLen(vec[17 × 0u32]) = {n:?}");

    println!("P5-e roundtrip green: list<u32> ↔ list<u32>, list<u64>→u64, list<str>→str, bignat arg");
    Ok(())
}
