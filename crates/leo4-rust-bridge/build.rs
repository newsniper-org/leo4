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
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let c_source = manifest_dir.join("../../shim/leo4_rust_bridge.c");

    println!("cargo:rerun-if-changed={}", c_source.display());

    let mut build = cc::Build::new();
    build
        .file(&c_source)
        .warnings(true)
        .extra_warnings(true);

    // Arch-style `musl-clang` wrapper quirk: the wrapper
    // passes `-nostdinc` and only `-isystem`s
    // `/usr/lib/musl/include`, dropping clang's freestanding
    // headers (`stdatomic.h`, `stddef.h`, `stdint.h`, …)
    // from the include path. `shim/leo4_rust_bridge.c`
    // depends on `<stdatomic.h>` (line ~53), so the build
    // fails with `'stdatomic.h' file not found` under
    // Arch's `musl-clang`. Detect the wrapper and add
    // `clang -print-resource-dir`/include back to the
    // include path. Idempotent for non-musl-clang
    // toolchains (the `-isystem` flag is harmless if the
    // dir already happens to be in the path).
    //
    // Rationale for fixing in build.rs rather than docs-
    // only: many users will hit this on Arch without
    // reading the docs first; an auto-fix produces a
    // green build out-of-box where the diagnostic
    // ("'stdatomic.h' file not found") otherwise sends
    // them down a yak-shave.
    add_musl_clang_resource_dir_fixup(&mut build);

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

/// If the resolved C compiler is `musl-clang` (Arch
/// packaging quirk: the wrapper uses `-nostdinc` and
/// only `-isystem`s `/usr/lib/musl/include`, omitting
/// clang's freestanding headers), append
/// `-isystem <resource-dir>/include` so headers like
/// `<stdatomic.h>` resolve again. No-op for any other
/// compiler.
fn add_musl_clang_resource_dir_fixup(build: &mut cc::Build) {
    // Resolve the compiler the same way cc-rs would:
    // CC_<target>, then HOST/TARGET CC, then CC.
    let target = env::var("TARGET").unwrap_or_default();
    let target_underscore = target.replace('-', "_");
    let cc = env::var(format!("CC_{target_underscore}"))
        .or_else(|_| env::var(format!("CC_{target}")))
        .or_else(|_| env::var("CC"))
        .unwrap_or_default();

    if !cc.ends_with("musl-clang") && !cc.contains("/musl-clang") {
        return;
    }

    // Ask clang where its resource-dir lives, then add
    // `<resource-dir>/include` to the include path.
    // `clang` (not the wrapper) is what owns the
    // resource-dir, so call clang directly.
    let Ok(output) = Command::new("clang").arg("-print-resource-dir").output() else {
        // clang not on PATH — nothing we can do; let the
        // user's downstream build surface the diagnostic
        // (musl-clang is itself a `clang` wrapper, so if
        // CC=musl-clang resolved, `clang` MUST be on PATH;
        // this branch is defensive).
        return;
    };
    if !output.status.success() {
        return;
    }
    let Ok(dir) = String::from_utf8(output.stdout) else {
        return;
    };
    let dir = dir.trim();
    if dir.is_empty() {
        return;
    }

    let include_path = format!("{dir}/include");
    build.flag(format!("-isystem{include_path}"));
    println!(
        "cargo:warning=leo4-rust-bridge: musl-clang detected; \
         added -isystem{include_path} to fix `stdatomic.h` lookup \
         (Arch musl-clang wrapper packaging quirk)"
    );
}
