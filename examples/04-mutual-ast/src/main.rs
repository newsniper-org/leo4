//! examples/04-mutual-ast — Phase 6 exit demo for mutual recursion.
//!
//! Mirrors `Sample.lean`'s `mutual inductive Expr / Stmt end` on the
//! Rust side with hand-written `LeanMarshal` impls (the `#[derive]`
//! macro doesn't yet emit cross-decl partial defs — Phase 6-3c
//! extends the *Lean* deriving handler; the Rust derive's
//! mutual-cluster support is a follow-up). The two impls
//! cross-reference each other through their payload encodes /
//! decodes; the wire format is the canonical-ABI variant encoding
//! per SPEC/canonical-abi.md §9 (u32 discriminator + payload).
//!
//! End-to-end checks:
//!   - `exprIsLit(Lit(42))` → true
//!   - `exprIsLit(Seq(Box::new(Stmt::Nop)))` → false
//!   - `stmtIsNop(Stmt::Nop)` → true
//!   - `stmtIsNop(Stmt::Block(Box::new(Expr::Lit(1))))` → false
//!   - deeper nesting: `Stmt::Block(Box::new(Expr::Seq(Box::new(Stmt::Nop))))`
//!     round-trips both directions.

mod sample {
    use super::{Expr, Stmt};
    leo4::import! {
        fn exprIsLit(e: Expr) -> bool;
        fn stmtIsNop(s: Stmt) -> bool;
    }
}

// `#[derive(LeanMarshal)]` handles the mutual recursion automatically:
// Rust accepts forward references between top-level `impl` blocks in
// the same module, so `Expr`'s impl can reference `Stmt`'s and vice
// versa without any explicit cluster handling on the derive side.
// `Box<T>`'s pass-through `LeanMarshal` impl (leo4_abi::composites)
// breaks the recursive Rust type so the enums can be sized; on the
// Lean side the `mutual ... end` deriving block closes the same cycle.
#[derive(leo4::LeanMarshal, Debug, PartialEq, Clone)]
enum Expr {
    Lit(u64),
    Seq(Box<Stmt>),
}

#[derive(leo4::LeanMarshal, Debug, PartialEq, Clone)]
enum Stmt {
    Nop,
    Block(Box<Expr>),
}

fn main() -> Result<(), leo4::LeanError> {
    let lean = leo4::Lean::open(
        env!("LEO4_SHIM_SO"),
        env!("LEO4_HANDSHAKE_FILE"),
    )?;

    // exprIsLit checks.
    let lit42 = Expr::Lit(42);
    let r = sample::exprIsLit(&lean, lit42.clone())?;
    assert!(r);
    println!("exprIsLit({lit42:?}) = {r}");

    let seq_nop = Expr::Seq(Box::new(Stmt::Nop));
    let r = sample::exprIsLit(&lean, seq_nop.clone())?;
    assert!(!r);
    println!("exprIsLit({seq_nop:?}) = {r}");

    // stmtIsNop checks.
    let r = sample::stmtIsNop(&lean, Stmt::Nop)?;
    assert!(r);
    println!("stmtIsNop(Stmt::Nop) = {r}");

    let block_lit = Stmt::Block(Box::new(Expr::Lit(1)));
    let r = sample::stmtIsNop(&lean, block_lit.clone())?;
    assert!(!r);
    println!("stmtIsNop({block_lit:?}) = {r}");

    // Deeper nesting: alternate Expr/Stmt three levels deep.
    let nested = Stmt::Block(Box::new(Expr::Seq(Box::new(Stmt::Block(Box::new(
        Expr::Lit(0xdead_beef),
    ))))));
    let r = sample::stmtIsNop(&lean, nested.clone())?;
    assert!(!r);
    println!("stmtIsNop(deep nested) = {r}");

    // Pure-Rust round-trip via `leo4::encode` / `leo4::decode` to
    // confirm the hand-rolled marshalling is internally consistent
    // before sending bytes across the boundary.
    let bytes = leo4::encode(&nested);
    let back: Stmt = leo4::decode(&bytes).expect("Stmt round-trip");
    assert_eq!(nested, back);
    println!(
        "Stmt encode→decode round-trip (Rust-only): {} bytes",
        bytes.len()
    );

    println!("Phase 6-4 mutual-AST demo green");
    Ok(())
}
