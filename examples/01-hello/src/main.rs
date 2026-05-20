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

    Ok(())
}
