use leo4_oxilean_build::{lean4_normalize, leo4_translate};
use oxilean_parse_peg::parse_decls as peg_parse_decls;

fn try_one(label: &str, src: &str) {
    let norm = lean4_normalize(src);
    let peg = match peg_parse_decls(&norm) {
        Ok(p) => p,
        Err(e) => { eprintln!("[{label}] PEG ERR: {e:?}"); return; }
    };
    for (i, d) in peg.iter().enumerate() {
        eprintln!("[{label}] decl #{i}: kind = {:?}", d.kind);
        match leo4_translate::translate_decl(d) {
            Ok(_) => eprintln!("[{label}] translate OK"),
            Err(e) => eprintln!("[{label}] translate ERR: {e}"),
        }
    }
}

fn main() {
    try_one("just_return", "def justReturn : IO Unit := do return ()\n");
}
