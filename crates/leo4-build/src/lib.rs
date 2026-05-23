//! leo4-build — `build.rs` helper for crates that consume a leo4 shim.
//!
//! Typical user:
//!
//! ```ignore
//! // examples/01-hello/build.rs
//! fn main() {
//!     leo4_build::wire("../../tests/sample-lean/.lake/build/leo4")
//!         .expect("leo4 shim wiring");
//! }
//! ```
//!
//! [`wire`] inspects `lake_build_dir` for the Lake-plugin emitted
//! sidecar files (`<pkg>.leo4-{handshake,mangling}` and
//! `<pkg>.leo4-shim.so`), emits `cargo:rustc-env=` lines that point
//! the `leo4_macros` proc-macro and the `leo4_mslean4` loader at the
//! discovered paths, and registers `cargo:rerun-if-changed=` for
//! each so an updated shim re-triggers the consumer crate's build.
//!
//! Three env vars are exposed to the compile environment:
//!
//! - `LEO4_HANDSHAKE_FILE` — `.leo4-handshake` absolute path
//! - `LEO4_MANGLING_FILE`  — `.leo4-mangling` absolute path
//! - `LEO4_SHIM_SO`        — `.leo4-shim.so` (or `.dylib` / `.dll`
//!                            on other tiers) absolute path
//!
//! Downstream code reads them with `env!("LEO4_…")` at compile time
//! (proc-macro) or `std::env::var("LEO4_…")` at runtime (loader).

use std::path::{Path, PathBuf};

/// Wire a leo4 shim into the consuming crate's build environment.
///
/// `lake_build_dir` should point at the directory the Lake plugin
/// emits its artefacts into — typically `<lean-pkg>/.lake/build/leo4/`.
/// Exactly one `<pkg>.leo4-handshake` file must live there; its stem
/// names the rest of the sidecar set.
///
/// # Errors
///
/// Returns `Err` if `lake_build_dir` does not exist, cannot be read,
/// or contains zero or multiple `.leo4-handshake` files. The error
/// message names which the build-script writer should look at.
pub fn wire(lake_build_dir: impl AsRef<Path>) -> std::io::Result<WireOutput> {
    let dir = lake_build_dir.as_ref();
    let dir_abs = dir
        .canonicalize()
        .map_err(|e| io_err(format!("canonicalize {dir:?}: {e}")))?;

    let mut handshakes: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(&dir_abs)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".leo4-handshake"))
        {
            handshakes.push(path);
        }
    }
    if handshakes.is_empty() {
        return Err(io_err(format!(
            "no `.leo4-handshake` file found in {dir_abs:?} — did you run `lake build`?"
        )));
    }
    if handshakes.len() > 1 {
        return Err(io_err(format!(
            "multiple `.leo4-handshake` files in {dir_abs:?}: {handshakes:?} — leo4-build expects exactly one per consumer"
        )));
    }
    let handshake = handshakes.into_iter().next().unwrap();
    let stem = handshake
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| io_err(format!("non-UTF8 stem on {handshake:?}")))?
        .to_string();
    let mangling = dir_abs.join(format!("{stem}.leo4-mangling"));
    let shim_so = dir_abs.join(format!("{stem}.leo4-shim.so"));

    println!(
        "cargo:rustc-env=LEO4_HANDSHAKE_FILE={}",
        handshake.display()
    );
    println!(
        "cargo:rustc-env=LEO4_MANGLING_FILE={}",
        mangling.display()
    );
    println!("cargo:rustc-env=LEO4_SHIM_SO={}", shim_so.display());
    println!("cargo:rerun-if-changed={}", handshake.display());
    println!("cargo:rerun-if-changed={}", mangling.display());

    Ok(WireOutput {
        stem,
        handshake_path: handshake,
        mangling_path: mangling,
        shim_so_path: shim_so,
    })
}

/// Paths surfaced by [`wire`], for callers that want to do additional
/// build-script work beyond what `leo4_build` arranges by default.
#[derive(Debug, Clone)]
pub struct WireOutput {
    pub stem: String,
    pub handshake_path: PathBuf,
    pub mangling_path: PathBuf,
    pub shim_so_path: PathBuf,
}

fn io_err(msg: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, msg.into())
}

// ─── Phase 9-2: reverse-direction (Rust → Lean) build-script helper ──

/// Wire a Rust cdylib's reverse-direction handshake into the
/// consumer's build environment.
///
/// Unlike [`wire`] (forward direction), this helper does **not**
/// reach into the cdylib itself — that's the job of the
/// `leo4-rust-emit` CLI, which the user runs separately after
/// `cargo build`. This function only:
///
/// 1. Resolves the absolute path of `<pkg>.leo4-rust-exports.idl`
///    and `<pkg>.leo4-rust-handshake` inside `out_dir`.
/// 2. Emits `cargo:rustc-env=LEO4_RUST_HANDSHAKE_FILE=…` and
///    `LEO4_RUST_EXPORTS_IDL_FILE=…` so the Lean wrapper Lake will
///    eventually generate (Phase 9-5) can pick the paths up via
///    `env!()`.
/// 3. Emits `cargo:rerun-if-changed=…` for both files plus
///    `rerun-if-env-changed=LEO4_RUST_CDYLIB` so the consumer
///    rebuilds when the cdylib path is overridden at runtime.
///
/// Typical caller is the **Lean-side consumer**'s `build.rs` (or a
/// thin wrapper Cargo crate that re-exports it to Lake). The Rust
/// cdylib *producer* doesn't call this; it just runs `cargo build`
/// followed by `leo4-rust-emit`.
///
/// # Errors
///
/// Returns `Err` if `out_dir` does not exist, cannot be read, or
/// does not contain exactly one `.leo4-rust-handshake` file. The
/// caller is expected to run `leo4-rust-emit` first.
pub fn wire_rust_exports(out_dir: impl AsRef<Path>) -> std::io::Result<RustExportsOutput> {
    let dir = out_dir.as_ref();
    let dir_abs = dir
        .canonicalize()
        .map_err(|e| io_err(format!("canonicalize {dir:?}: {e}")))?;

    let mut handshakes: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(&dir_abs)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".leo4-rust-handshake"))
        {
            handshakes.push(path);
        }
    }
    if handshakes.is_empty() {
        return Err(io_err(format!(
            "no `.leo4-rust-handshake` file in {dir_abs:?} — run `leo4-rust-emit --cdylib … --out-dir {dir_abs:?}` first"
        )));
    }
    if handshakes.len() > 1 {
        return Err(io_err(format!(
            "multiple `.leo4-rust-handshake` files in {dir_abs:?}: {handshakes:?} — one per consumer expected"
        )));
    }
    let handshake = handshakes.into_iter().next().unwrap();
    let stem = handshake
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| io_err(format!("non-UTF8 stem on {handshake:?}")))?
        .to_string();
    let idl = dir_abs.join(format!("{stem}.leo4-rust-exports.idl"));

    println!(
        "cargo:rustc-env=LEO4_RUST_HANDSHAKE_FILE={}",
        handshake.display()
    );
    println!(
        "cargo:rustc-env=LEO4_RUST_EXPORTS_IDL_FILE={}",
        idl.display()
    );
    println!("cargo:rerun-if-changed={}", handshake.display());
    println!("cargo:rerun-if-changed={}", idl.display());
    println!("cargo:rerun-if-env-changed=LEO4_RUST_CDYLIB");

    Ok(RustExportsOutput {
        stem,
        handshake_path: handshake,
        idl_path: idl,
    })
}

/// Paths surfaced by [`wire_rust_exports`].
#[derive(Debug, Clone)]
pub struct RustExportsOutput {
    pub stem: String,
    pub handshake_path: PathBuf,
    pub idl_path: PathBuf,
}
