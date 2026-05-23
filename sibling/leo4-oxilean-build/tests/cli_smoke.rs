//! OX1 integration tests — subprocess-invoke the
//! `leo4-oxilean-build` CLI binary and verify the manifest →
//! Cargo-crate transpile path works end-to-end.
//!
//! cargo populates `CARGO_BIN_EXE_leo4-oxilean-build` with
//! the binary's absolute path during test builds, so these
//! tests don't need to know where the workspace target dir
//! lives.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn cli_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_leo4-oxilean-build"))
}

/// Unique tmp dir under `CARGO_TARGET_TMPDIR` so cleanup is
/// hermetic + multiple test runs don't collide.
fn tmp_dir(label: &str) -> PathBuf {
    let base = std::env::var("CARGO_TARGET_TMPDIR")
        .map_or_else(
            |_| std::env::temp_dir().join("leo4-oxilean-build-cli"),
            PathBuf::from,
        );
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = base.join(format!("{label}-{}-{nanos}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("tmp_dir mkdir");
    dir
}

fn write_file(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("write_file mkdir");
    }
    std::fs::write(path, contents).expect("write_file");
}

#[test]
fn cli_help_exits_success() {
    let output = Command::new(cli_path())
        .arg("--help")
        .output()
        .expect("invoke CLI");
    assert!(output.status.success(), "--help must exit 0; stderr={}",
        String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("leo4-oxilean-build"));
    assert!(stdout.contains("Manifest"));
}

#[test]
fn cli_missing_manifest_arg_exits_usage_error() {
    let output = Command::new(cli_path()).output().expect("invoke");
    // Exit code 2 = usage error.
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--manifest"));
}

#[test]
fn cli_skip_path_emits_dispatcher_only() {
    let dir = tmp_dir("skip");
    let out_dir = dir.join("crate");
    let lean = dir.join("Skip.lean");
    let manifest = dir.join("manifest.txt");

    write_file(&lean, "def helper : Nat -> Nat := fun n -> n\n");
    write_file(
        &manifest,
        &format!(
            "crate_name=skip_pkg\n\
             schema_hash=0000000000000\n\
             leo4_abi_dep=\"0.1\"\n\
             out_dir={}\n\
             source={} abc_a\n",
            out_dir.display(),
            lean.display()
        ),
    );

    let output = Command::new(cli_path())
        .arg("--manifest")
        .arg(&manifest)
        .output()
        .expect("invoke");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "CLI must succeed on skip path; exit={:?} stderr={stderr}",
        output.status.code()
    );
    assert!(stderr.contains("skip"));
    assert!(stderr.contains("0 units, 1 skipped"));

    // Verify emitted files.
    let manifest_path = out_dir.join("Cargo.toml");
    let lib_path = out_dir.join("src").join("lib.rs");
    assert!(manifest_path.exists(), "Cargo.toml must exist");
    assert!(lib_path.exists(), "src/lib.rs must exist");

    let lib_text = std::fs::read_to_string(&lib_path).expect("read lib.rs");
    // Even with zero exports, the dispatcher is emitted with
    // only its default arm — the consumer still gets a
    // usable `Leo4OxileanProc`.
    assert!(lib_text.contains("Leo4OxileanProc"));
    assert!(lib_text.contains("unknown_function(mangled)"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_reads_manifest_from_stdin() {
    let dir = tmp_dir("stdin");
    let out_dir = dir.join("crate");
    let lean = dir.join("Stdin.lean");

    write_file(&lean, "def x : Nat -> Nat := fun n -> n\n");

    let manifest = format!(
        "crate_name=stdin_pkg\n\
         schema_hash=1111111111111\n\
         leo4_abi_dep=\"0.1\"\n\
         out_dir={}\n\
         source={} abc_a\n",
        out_dir.display(),
        lean.display()
    );

    let mut child = Command::new(cli_path())
        .arg("--manifest")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("stdin pipe")
        .write_all(manifest.as_bytes())
        .expect("write manifest");
    let output = child.wait_with_output().expect("wait");

    assert!(
        output.status.success(),
        "stdin manifest must succeed; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out_dir.join("Cargo.toml").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_rejects_bogus_manifest_field() {
    let dir = tmp_dir("bad");
    let manifest = dir.join("manifest.txt");
    write_file(&manifest, "crate_name=x\nbogus_field=42\n");

    let output = Command::new(cli_path())
        .arg("--manifest")
        .arg(&manifest)
        .output()
        .expect("invoke");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("bogus_field") || stderr.contains("unknown key"),
        "stderr should explain the bad field: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_reports_transpile_error_with_nonzero_exit() {
    // Empty env can't resolve `Nat` — elab fails. CLI must
    // surface this as exit code 1 (transpile failure), not 2
    // (usage), and must NOT emit a crate.
    let dir = tmp_dir("elab_err");
    let out_dir = dir.join("crate");
    let lean = dir.join("Bad.lean");
    let manifest = dir.join("manifest.txt");

    write_file(&lean, "@[leo4_export] def f : Nat -> Nat := fun n -> n\n");
    write_file(
        &manifest,
        &format!(
            "crate_name=bad_pkg\n\
             schema_hash=2222222222222\n\
             leo4_abi_dep=\"0.1\"\n\
             out_dir={}\n\
             source={} abc_a\n",
            out_dir.display(),
            lean.display()
        ),
    );

    let output = Command::new(cli_path())
        .arg("--manifest")
        .arg(&manifest)
        .output()
        .expect("invoke");

    assert_eq!(output.status.code(), Some(1), "exit must be 1 for transpile failure");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ERROR"));
    // No crate emitted on transpile failure.
    assert!(
        !out_dir.join("Cargo.toml").exists(),
        "manifest must NOT exist on transpile failure"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
