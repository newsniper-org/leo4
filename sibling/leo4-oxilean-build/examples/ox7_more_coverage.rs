//! Spike for additional translate_expr arms — Forall,
//! Do, By, InterpStr, AnonStruct, AnonCtor, DotFn.

use leo4_oxilean_build::{lean4_normalize, leo4_translate};
use oxilean_parse_peg::parse_decls as peg_parse_decls;

fn try_fixture(label: &str, src: &str) {
    let norm = lean4_normalize(src);
    let peg = match peg_parse_decls(&norm) {
        Ok(d) => d,
        Err(e) => {
            println!("[{label}] PEG ERR: {e:?}");
            return;
        }
    };
    for (i, d) in peg.iter().enumerate() {
        match leo4_translate::translate_decl(d) {
            Ok(_) => println!("[{label}] decl #{i}: OK"),
            Err(e) => println!("[{label}] decl #{i}: ERR — {e}"),
        }
    }
}

fn main() {
    try_fixture("forall", "def p : Prop := ∀ (x : Nat), x = x\n");
    try_fixture("forall_ascii", "def q : Prop := forall (x : Nat), x = x\n");
    try_fixture("exists", "def r : Prop := ∃ (x : Nat), x = 0\n");
    try_fixture("do_block", "def m : IO Unit := do IO.println \"hi\"\n");
    try_fixture("interp_str", "def s (n : Nat) : String := s!\"n is {n}\"\n");
    try_fixture("anon_struct", "def pt : Point := { x := 1, y := 2 }\n");
    try_fixture("anon_ctor", "def pair : Prod Nat Nat := ⟨1, 2⟩\n");
    try_fixture("dot_fn", "def f (n : Nat) : Bool := n.isPrime\n");
    try_fixture("raw", "def r : Foo := bar baz qux\n");
}
