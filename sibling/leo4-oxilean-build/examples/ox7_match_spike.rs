use oxilean_parse_peg::parse_decls;

fn main() {
    let src = "def isZero (n : Nat) : Bool := match n with | 0 => true | _ => false\n";
    match parse_decls(src) {
        Ok(decls) => {
            for d in &decls {
                eprintln!("{:#?}", d);
            }
        }
        Err(e) => eprintln!("ERR: {e:?}"),
    }
}
