//! `build.rs` — compiles `shim/leo4_rust_bridge.c` into the
//! `libleo4_rust_bridge.a` static archive that the Lake plugin
//! eventually statically links into the Lean executable (Phase
//! 9-6). The `cc` crate handles the C compiler driver (clang
//! on every tier per `LEO4-DESIGN.md §9.1` / the gnullvm follow-up).
//!
//! C standard target: C17 baseline; bump to `-std=c2x` when the
//! detected compiler accepts it. C11 is rejected per
//! `SPEC/reverse-direction.md` §11.

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let c_source = manifest_dir.join("../../shim/leo4_rust_bridge.c");

    println!("cargo:rerun-if-changed={}", c_source.display());

    let mut build = cc::Build::new();
    build
        .file(&c_source)
        .warnings(true)
        .extra_warnings(true);

    // C standard selection per SPEC/reverse-direction.md §11:
    // C17 is the baseline; upgrade to C23 when the compiler
    // accepts it. The `-std=` flag is last-wins on every supported
    // compiler (clang, gcc), so we add the baseline first and let
    // `flag_if_supported` drop in the higher upgrade if available.
    // Result on common toolchains:
    //   * clang ≥ 18 / gcc ≥ 14 → -std=c23
    //   * clang 16-17 / gcc 13  → -std=c2x
    //   * older                 → -std=c17 (baseline)
    build.flag("-std=c17");
    build.flag_if_supported("-std=c2x");
    build.flag_if_supported("-std=c23");

    // Targets:
    //   * Linux / macOS: rely on clang/gcc defaults (visibility =
    //     hidden by default if the user runs leanc, but the C
    //     source marks the single export with
    //     __attribute__((visibility("default")))).
    //   * Windows / gnullvm: same clang driver; the C source
    //     resolves to __declspec(dllexport) via the LEO4_RUST_EXPORT
    //     macro.
    if cfg!(target_os = "linux") || cfg!(target_os = "macos") {
        build.flag_if_supported("-fvisibility=hidden");
    }

    build.compile("leo4_rust_bridge");
}
