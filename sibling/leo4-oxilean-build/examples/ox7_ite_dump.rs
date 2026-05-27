//! OX7 (ite) elab-output dump — feed `def constU64 :=
//! if true then 1 else 0` through the real OxiLean elab
//! pipeline and print the resulting `(ty, val)` shape so we
//! can see exactly what `ite` desugars to past the point
//! `to_lcnf` can erase implicit args.

use leo4_oxilean_build::leo4_env_bootstrap::bootstrap_env;
use leo4_oxilean_build::{lean4_normalize, parse_decls_for_transpile, inner_decl};
use oxilean_elab::elab_decl::{elaborate_decl, PendingDecl};
use oxilean_parse::Decl as OxDecl;

fn main() {
    let env = bootstrap_env().expect("env");
    let src = "def constU64 : UInt64 := if true then 1 else 0\n\
               def chooseU64 (b : Bool) (a b' : UInt64) : UInt64 := if b then a else b'\n";
    let norm = lean4_normalize(src);
    let parsed = parse_decls_for_transpile(&norm).expect("parse");

    for d in &parsed {
        let inner = inner_decl(d);
        if let OxDecl::Definition { name, .. } = &inner.value {
            println!("=== source decl: {name:?} ===");
            println!("--- post-translate Decl ---\n{:#?}\n---", inner.value);
            match elaborate_decl(&env, &inner.value) {
                Ok(PendingDecl::Definition { name, ty, val, .. }) => {
                    println!("--- name = {name:?} ---");
                    println!("--- ty   = {ty:#?}");
                    println!("--- val  = {val:#?}");
                }
                Ok(other) => println!("(non-Definition: {other:?})"),
                Err(e) => println!("ELAB ERROR: {e:?}"),
            }
        }
    }
}
