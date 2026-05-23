use leo4_oxilean_build::transpile_kernel_decl;
use oxilean_kernel::{Expr, Name};
fn main() {
    let nat = Expr::Const(Name::str("Nat"), vec![]);
    let succ = Expr::Const(Name::str("Nat.succ"), vec![]);
    let body = Expr::App(Box::new(succ), Box::new(Expr::BVar(0)));
    let src = transpile_kernel_decl(
        &Name::str("Sample.addOne"),
        &[(Name::str("n"), nat)],
        &body,
    ).unwrap();
    println!("=== emitted Rust source ===\n{src}\n=== end ===");
}
