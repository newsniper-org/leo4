use leo4_oxilean_build::{lean4_normalize, parse_decls_for_transpile, leo4_translate};
use oxilean_parse_peg::parse_decls as peg_parse_decls;

fn main() {
    let src = "def maxU64 (a b : UInt64) : UInt64 := if a < b then b else a\n";
    let norm = lean4_normalize(src);
    let peg = peg_parse_decls(&norm).expect("peg parse");
    for d in &peg {
        match leo4_translate::translate_decl(d) {
            Ok(decl) => eprintln!("[trace] translate OK: {decl:#?}"),
            Err(e) => eprintln!("[trace] translate ERR: {e}"),
        }
    }
}
