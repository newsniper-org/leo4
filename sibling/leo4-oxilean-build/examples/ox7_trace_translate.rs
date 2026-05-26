use leo4_oxilean_build::{lean4_normalize, parse_decls_for_transpile, leo4_translate};
use oxilean_parse_peg::parse_decls as peg_parse_decls;

fn main() {
    let src = "def add (a b : UInt64) : UInt64 := a + b\n";
    let norm = lean4_normalize(src);
    let peg = peg_parse_decls(&norm).expect("peg parse");
    eprintln!("[trace] peg parse OK ({} decls)", peg.len());
    for (i, d) in peg.iter().enumerate() {
        match leo4_translate::translate_decl(d) {
            Ok(_) => eprintln!("[trace] translate OK #{i}"),
            Err(e) => eprintln!("[trace] translate ERR #{i}: {e}"),
        }
    }
    eprintln!("[trace] parse_decls_for_transpile:");
    match parse_decls_for_transpile(&norm) {
        Ok(ds) => eprintln!("  ok: {} decls", ds.len()),
        Err(e) => eprintln!("  err: {e:?}"),
    }
}
