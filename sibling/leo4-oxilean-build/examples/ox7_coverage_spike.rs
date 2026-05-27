//! Coverage spike — test which Expr variants translate_decl
//! still fails on for common control-flow patterns.

use leo4_oxilean_build::{lean4_normalize, leo4_translate};
use oxilean_parse_peg::parse_decls as peg_parse_decls;

fn try_fixture(label: &str, src: &str) {
    let norm = lean4_normalize(src);
    let peg = match peg_parse_decls(&norm) {
        Ok(d) => d,
        Err(e) => {
            println!("[{label}] PEG parse ERR: {e:?}");
            return;
        }
    };
    for (i, d) in peg.iter().enumerate() {
        match leo4_translate::translate_decl(d) {
            Ok(_) => println!("[{label}] decl #{i}: translate OK"),
            Err(e) => println!("[{label}] decl #{i}: translate ERR — {e}"),
        }
    }
}

fn main() {
    try_fixture(
        "if_then_else",
        "def maxU64 (a b : UInt64) : UInt64 := if a < b then b else a\n",
    );
    try_fixture(
        "let_in",
        "def double (n : UInt64) : UInt64 := let m := n + n; m\n",
    );
    try_fixture(
        "match",
        "def isZero (n : Nat) : Bool := match n with | 0 => true | _ => false\n",
    );
    try_fixture(
        "list_literal",
        "def threeNats : List Nat := [1, 2, 3]\n",
    );
    try_fixture(
        "explicit_at",
        "def f : Nat := @id Nat 0\n",
    );
}
