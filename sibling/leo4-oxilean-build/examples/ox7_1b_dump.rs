//! OX7 (1b) elab-output dump — feed `def add (a b : UInt64)
//! : UInt64 := a + b` through the real OxiLean elab pipeline
//! and print the resulting `(ty, val)` shape so we can see
//! exactly what implicit args are getting baked in past the
//! point `to_lcnf` can erase them.

use leo4_oxilean_build::leo4_env_bootstrap::bootstrap_env;
use leo4_oxilean_build::{lean4_normalize, parse_decls_for_transpile, inner_decl};
use oxilean_elab::elab_decl::{elaborate_decl, PendingDecl};
use oxilean_parse::Decl as OxDecl;

fn main() {
    let env = bootstrap_env().expect("env");
    // Monomorphic call — `+` itself would NameNotFound
    // until HAdd typeclass + instances arrive. Use the
    // explicit form to expose what implicit args elab
    // piles on top of the body.
    let src = "def add (a b : UInt64) : UInt64 := UInt64.add a b\n";
    let norm = lean4_normalize(src);
    let parsed = parse_decls_for_transpile(&norm).expect("parse");

    for d in &parsed {
        let inner = inner_decl(d);
        if let OxDecl::Definition { name, .. } = &inner.value {
            println!("=== source decl: {name:?} ===");
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
