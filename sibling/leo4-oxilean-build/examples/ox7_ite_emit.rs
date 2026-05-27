//! OX7 (ite) emit dump — show what
//! `def constU64 : UInt64 := if true then 1 else 0` currently
//! transpiles to. Helps confirm the `ite(...)` opaque-call
//! symptom and (post-fix) the native `if-expr` lowering.

use leo4_oxilean_build::transpile_source;
use leo4_oxilean_build::leo4_env_bootstrap::bootstrap_env;

fn main() {
    let env = bootstrap_env().expect("env");
    let src = "def constU64 : UInt64 := if true then 1 else 0\n\
               def chooseU64 (b : Bool) (a : UInt64) (c : UInt64) : UInt64 := if b then a else c\n";
    match transpile_source(&env, src) {
        Ok(rs) => {
            println!("--- transpile_source emit ---\n{rs}");
        }
        Err(e) => {
            eprintln!("transpile_source error: {e:?}");
            std::process::exit(1);
        }
    }
}
