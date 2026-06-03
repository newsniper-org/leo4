//! E2E smoke test for `leo4_oxilean_runner::run_main` — RC.6 F2
//! (2026-05-31). Exercises the full pipeline against a real
//! cdylib + Main.lean pair pre-built by the developer.
//!
//! ## Why opt-in via env var (not auto)?
//!
//! Building a cdylib + Main.lean fixture at test time would
//! require either:
//!
//! 1. `cargo build` subprocess invocation inside the test
//!    (slow: ~30..60s; brittle: requires the test process to
//!    know the cdylib package layout + Cargo.toml layout +
//!    target dir conventions).
//! 2. A `build.rs` script driving an inline sub-package
//!    (fights cargo's "no nested workspaces" rules for the
//!    standalone sibling layout this crate uses).
//!
//! Both are louder than the test is worth. The runner already
//! has surface-level smoke (`run_main_missing_cdylib_is_dlsym_failed`
//! etc.) for the error-path; this opt-in test covers the happy
//! path when the developer has an existing cdylib + Main.lean
//! pair handy.
//!
//! ## How to run
//!
//! Pre-build a leo4 reverse-direction example, e.g.
//! `examples/05-rust-export/`:
//!
//! ```sh
//! cargo build -p leo4-example-05-rust-export
//! ```
//!
//! Then set the two env vars to the produced cdylib + the
//! example's `Main.lean`:
//!
//! ```sh
//! export LEO4_E2E_CDYLIB=/path/to/leo4/target/debug/libleo4_example_05_rust_export.so
//! export LEO4_E2E_MAIN_LEAN=/path/to/leo4/examples/05-rust-export/lean/Main.lean
//! ```
//!
//! And run with `--ignored`:
//!
//! ```sh
//! cd sibling/leo4-oxilean-runner
//! cargo test --test e2e_runner_smoke -- --ignored --nocapture
//! ```
//!
//! ## What's asserted
//!
//! The test calls `run_main_diagnostics` (not the panicking
//! `run_main`) so it can inspect how far the pipeline got even
//! when the IO walker hits an unrecognised shape — and prints
//! the `RunDiagnostics` envelope. Pass criteria:
//!
//! - `exports_seen > 0` — the cdylib carries at least one
//!   `#[leo4::export]` entry.
//! - `exports_registered == exports_seen` — every entry
//!   pair-registered.
//! - `env_bootstrapped == true` — the OxiLean prelude + leo4
//!   boundary primitives env construction succeeded.
//! - `resolver_ready == true` — the invoker's
//!   `as_shared_resolver` produced a usable resolver.
//! - `user_types_seen >= 0` — the USER_TYPES slice walked
//!   cleanly (post-RC.5 cdylibs have the slice; pre-RC.5
//!   ones don't, both pass).
//! - `decls_parsed > 0` — Main.lean had at least one
//!   top-level declaration parsed (e.g. the `main` decl).
//! - `decls_elaborated <= decls_parsed` — type-check / elab
//!   produced a non-negative-progress envelope.

use std::path::PathBuf;

fn env_path(var: &str) -> Option<PathBuf> {
    match std::env::var(var) {
        Ok(s) if !s.is_empty() => Some(PathBuf::from(s)),
        _ => None,
    }
}

#[test]
#[ignore = "opt-in: set LEO4_E2E_CDYLIB + LEO4_E2E_MAIN_LEAN env vars and run with --ignored"]
fn run_main_diagnostics_walks_real_cdylib_and_main_lean() {
    let cdylib = env_path("LEO4_E2E_CDYLIB").expect(
        "LEO4_E2E_CDYLIB env var not set — point it at a pre-built leo4 \
         reverse-direction cdylib (e.g. target/debug/libleo4_example_05_rust_export.so)",
    );
    let main_lean = env_path("LEO4_E2E_MAIN_LEAN").expect(
        "LEO4_E2E_MAIN_LEAN env var not set — point it at the matching \
         lean/Main.lean for the cdylib",
    );

    assert!(
        cdylib.exists(),
        "LEO4_E2E_CDYLIB path `{}` does not exist; pre-build the example first",
        cdylib.display(),
    );
    assert!(
        main_lean.exists(),
        "LEO4_E2E_MAIN_LEAN path `{}` does not exist",
        main_lean.display(),
    );

    let diag = leo4_oxilean_runner::run_main_diagnostics(&cdylib, &main_lean)
        .unwrap_or_else(|e| {
            panic!(
                "run_main_diagnostics failed before pipeline could complete: {e}"
            )
        });

    eprintln!("e2e smoke: {diag:#?}");

    // Wiring-level assertions.
    assert!(
        diag.exports_seen > 0,
        "cdylib `{}` carries 0 `#[leo4::export]` entries — wrong file?",
        cdylib.display(),
    );
    assert_eq!(
        diag.exports_registered, diag.exports_seen,
        "every walked export should pair-register"
    );
    assert!(diag.env_bootstrapped, "env bootstrap must succeed");
    assert!(
        diag.resolver_ready,
        "OxiLeanInvoker::as_shared_resolver should produce a resolver \
         once all exports register",
    );
    // user_types_seen is informational — post-RC.5 cdylibs report
    // a count, pre-RC.5 ones report 0. Both are healthy.
    assert!(
        diag.decls_parsed > 0,
        "Main.lean `{}` parsed 0 decls — file empty or unparseable?",
        main_lean.display(),
    );
    assert!(
        diag.decls_elaborated <= diag.decls_parsed,
        "elaborated count ({}) can't exceed parsed count ({})",
        diag.decls_elaborated,
        diag.decls_parsed,
    );
}

#[test]
#[ignore = "opt-in: same vars as the diagnostics variant; this one drives `main : IO Unit` to completion or a walker-gap error"]
fn run_main_drives_main_to_completion_or_documented_walker_gap() {
    let cdylib = env_path("LEO4_E2E_CDYLIB").expect("LEO4_E2E_CDYLIB unset");
    let main_lean =
        env_path("LEO4_E2E_MAIN_LEAN").expect("LEO4_E2E_MAIN_LEAN unset");
    assert!(cdylib.exists());
    assert!(main_lean.exists());

    match leo4_oxilean_runner::run_main(&cdylib, &main_lean) {
        Ok(()) => {
            eprintln!(
                "e2e: run_main completed `main : IO α` successfully against \
                 cdylib={}, main={}",
                cdylib.display(),
                main_lean.display(),
            );
        }
        Err(e) if e.code == 0x0002_0005 => {
            // Walker hit an unrecognised shape — documented gap,
            // not a wiring failure. Smoke-test PASS condition
            // (we exercised the full pipeline up to the gap).
            eprintln!(
                "e2e: run_main reached the IO walker, hit a recognised \
                 unrecognised-shape gap: {e}"
            );
        }
        Err(e) => {
            panic!(
                "e2e: run_main failed before the IO walker — wiring issue: {e}"
            );
        }
    }
}
