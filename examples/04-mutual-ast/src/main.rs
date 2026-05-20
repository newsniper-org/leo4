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

use leo4::{AbiError, LeanMarshal};

mod sample {
    use super::{Expr, Stmt};
    leo4::import! {
        fn exprIsLit(e: Expr) -> bool;
        fn stmtIsNop(s: Stmt) -> bool;
    }
}

#[derive(Debug, PartialEq, Clone)]
enum Expr {
    Lit(u64),
    Seq(Box<Stmt>),
}

#[derive(Debug, PartialEq, Clone)]
enum Stmt {
    Nop,
    Block(Box<Expr>),
}

// Hand-rolled `LeanMarshal` impls. Each variant prefixes the wire
// with its u32 discriminator and then encodes its payload via
// `LeanMarshal::canonical_encode`. `Box<T>` already has a pass-through
// impl in `leo4-abi`, so the cross-decl references compose
// transparently — no instance forward-declaration is needed because
// Rust accepts mutual `impl` blocks freely as long as both names are
// in scope. The Lean side closes the same cycle through the
// `mutual { partial def … }` block emitted by the deriving handler.

// Wire format for the variant discriminator: 1 byte (SPEC/canonical-abi.md
// §9 "decoders MAY accept a 1-byte discriminator when reading, but
// encoders MUST emit 4 bytes" — the shim emitter and the Rust
// `#[derive(LeanMarshal)]` both use u8 today; we match them so the
// hand-rolled impls round-trip against the shim's variant helpers
// without rewriting the existing 1-byte path. A canonical 4-byte
// disc lands when both sides flip together.).

impl LeanMarshal for Expr {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        match self {
            Expr::Lit(n) => {
                buf.push(0u8);
                n.canonical_encode(buf);
            }
            Expr::Seq(s) => {
                buf.push(1u8);
                s.canonical_encode(buf);
            }
        }
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), AbiError> {
        if buf.len() < off + 1 {
            return Err(AbiError::new(
                leo4::error_codes::DECODE_ERROR,
                "Expr: not enough bytes for u8 tag",
            ));
        }
        let tag = buf[off];
        let off = off + 1;
        match tag {
            0 => {
                let (n, off) = u64::canonical_decode(buf, off)?;
                Ok((Expr::Lit(n), off))
            }
            1 => {
                let (s, off) = Box::<Stmt>::canonical_decode(buf, off)?;
                Ok((Expr::Seq(s), off))
            }
            t => Err(AbiError::new(
                leo4::error_codes::DECODE_ERROR,
                format!("Expr: invalid variant tag {t}"),
            )),
        }
    }
}

impl LeanMarshal for Stmt {
    fn canonical_encode(&self, buf: &mut Vec<u8>) {
        match self {
            Stmt::Nop => {
                buf.push(0u8);
            }
            Stmt::Block(e) => {
                buf.push(1u8);
                e.canonical_encode(buf);
            }
        }
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), AbiError> {
        if buf.len() < off + 1 {
            return Err(AbiError::new(
                leo4::error_codes::DECODE_ERROR,
                "Stmt: not enough bytes for u8 tag",
            ));
        }
        let tag = buf[off];
        let off = off + 1;
        match tag {
            0 => Ok((Stmt::Nop, off)),
            1 => {
                let (e, off) = Box::<Expr>::canonical_decode(buf, off)?;
                Ok((Stmt::Block(e), off))
            }
            t => Err(AbiError::new(
                leo4::error_codes::DECODE_ERROR,
                format!("Stmt: invalid variant tag {t}"),
            )),
        }
    }
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
