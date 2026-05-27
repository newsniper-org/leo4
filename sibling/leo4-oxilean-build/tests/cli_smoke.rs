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
fn cli_pure_mode_emits_pub_fn_for_every_def() {
    // Pure mode (option A, default since 2026-05-25):
    // every top-level `def` lands as `pub fn` in the
    // emitted crate's src/lib.rs. No @[leo4_export]
    // filter — pure mode has no dispatcher to query
    // for the tag.
    let dir = tmp_dir("pure_basic");
    let out_dir = dir.join("crate");
    let lean = dir.join("Helper.lean");
    let manifest = dir.join("manifest.txt");

    write_file(&lean, "def helper : Nat -> Nat := fun n -> n\n");
    write_file(
        &manifest,
        &format!(
            "crate_name=pure_basic_pkg\n\
             out_dir={}\n\
             source={}\n",
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
        "CLI must succeed; exit={:?} stderr={stderr}",
        output.status.code()
    );

    // Verify emitted files.
    let cargo_path = out_dir.join("Cargo.toml");
    let lib_path = out_dir.join("src").join("lib.rs");
    assert!(cargo_path.exists(), "Cargo.toml must exist");
    assert!(lib_path.exists(), "src/lib.rs must exist");

    let cargo_text = std::fs::read_to_string(&cargo_path).expect("read Cargo.toml");
    // Pure mode invariant: no leo4-abi / no dispatcher dep.
    assert!(!cargo_text.contains("leo4-abi"), "Cargo.toml must not pull leo4-abi");
    assert!(!cargo_text.contains("[dependencies]"), "Cargo.toml must have no deps");

    let lib_text = std::fs::read_to_string(&lib_path).expect("read lib.rs");
    // The def lands as a pub fn; no dispatcher / no mangling.
    assert!(lib_text.contains("fn helper"), "lib.rs missing helper fn");
    assert!(!lib_text.contains("Leo4OxileanProc"), "pure mode emits no dispatcher");
    assert!(!lib_text.contains("leo4_call"), "pure mode emits no canonical entry");

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
fn cli_pure_mode_ignores_legacy_bind_lines_and_emits_all_defs() {
    // Pure mode silently ignores legacy manifest fields
    // (`schema_hash=`, `leo4_abi_dep=`, `bind=`, the
    // single-decl form's mangled suffix). Multiple `def`s
    // in one source all land as `pub fn` regardless of
    // any @[leo4_export] tag. Type-only decls
    // (`structure`/`inductive`) are NOT emitted today —
    // pure mode is fn-only in v0.
    let dir = tmp_dir("pure_multi");
    let out_dir = dir.join("crate");
    let lean = dir.join("MultiDecl.lean");
    let manifest = dir.join("manifest.txt");

    write_file(
        &lean,
        "def first : Nat -> Nat := fun n -> n\n\
         def second : Nat -> Nat := fun n -> n\n",
    );
    write_file(
        &manifest,
        &format!(
            "crate_name=pure_multi_pkg\n\
             schema_hash=ababababababa\n\
             leo4_abi_dep=\"0.1\"\n\
             out_dir={}\n\
             source={}\n\
             bind=first=ignored_mangled\n",
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
        "pure mode must succeed; stderr={stderr}"
    );
    // Note message about legacy canonical fields being ignored.
    assert!(
        stderr.contains("canonical-mode fields"),
        "expected legacy-fields note in stderr; got:\n{stderr}"
    );

    let lib = std::fs::read_to_string(out_dir.join("src").join("lib.rs"))
        .expect("read lib.rs");
    assert!(lib.contains("fn first"), "lib.rs missing `first`");
    assert!(lib.contains("fn second"), "lib.rs missing `second`");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_bind_before_source_rejected() {
    let dir = tmp_dir("badbind");
    let manifest = dir.join("manifest.txt");
    write_file(
        &manifest,
        "crate_name=x\n\
         schema_hash=000\n\
         leo4_abi_dep=\"0.1\"\n\
         out_dir=/tmp/x\n\
         bind=f=mangled\n",
    );
    let output = Command::new(cli_path())
        .arg("--manifest")
        .arg(&manifest)
        .output()
        .expect("invoke");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("before any `source=`"),
        "expected explanatory error; got:\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_bind_after_single_source_rejected() {
    let dir = tmp_dir("mixform");
    let manifest = dir.join("manifest.txt");
    write_file(
        &manifest,
        "crate_name=x\n\
         schema_hash=000\n\
         leo4_abi_dep=\"0.1\"\n\
         out_dir=/tmp/x\n\
         source=/tmp/foo.lean abc_a\n\
         bind=f=def_a\n",
    );
    let output = Command::new(cli_path())
        .arg("--manifest")
        .arg(&manifest)
        .output()
        .expect("invoke");
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("single-decl"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_reports_transpile_error_with_nonzero_exit() {
    // Reference to an undefined identifier — elab fails
    // even with the OX5-oxi bootstrapped env. CLI must
    // surface this as exit code 1 (transpile failure),
    // not 2 (usage), and must NOT emit a crate.
    let dir = tmp_dir("elab_err");
    let out_dir = dir.join("crate");
    let lean = dir.join("Bad.lean");
    let manifest = dir.join("manifest.txt");

    write_file(
        &lean,
        "def f : TypeDefinitelyNotInBootstrappedEnv -> Nat := fun n -> n\n",
    );
    write_file(
        &manifest,
        &format!(
            "crate_name=bad_pkg\n\
             out_dir={}\n\
             source={}\n",
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
        "Cargo.toml must NOT exist on transpile failure"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_native_binop_smoke_all_arith_and_comparison_ops() {
    // OX7 typeclass step (2026-05-27) — every Lean
    // stdlib primitive arithmetic / comparison operator
    // backed by an `HXxx.xXxx` / `LT.lt` / etc.
    // projection is expected to emit as a native Rust
    // BinOp on the resulting `pub fn` body. This is the
    // first multi-decl, multi-op smoke that exercises
    // both the leo4-side translate desugar
    // (`arith_op_to_tc_projection`) and the fork-side
    // codegen fold (`try_builtin_app` +
    // `tc_projection_to_rust_binop`).
    let dir = tmp_dir("native_binops");
    let out_dir = dir.join("crate");
    let lean = dir.join("Ops.lean");
    let manifest = dir.join("manifest.txt");

    write_file(
        &lean,
        "def addU64 (a b : UInt64) : UInt64 := a + b\n\
         def subU64 (a b : UInt64) : UInt64 := a - b\n\
         def mulU64 (a b : UInt64) : UInt64 := a * b\n\
         def divU64 (a b : UInt64) : UInt64 := a / b\n\
         def modU64 (a b : UInt64) : UInt64 := a % b\n\
         def ltU64  (a b : UInt64) : Bool   := a < b\n\
         def leU64  (a b : UInt64) : Bool   := a <= b\n\
         def eqU64  (a b : UInt64) : Bool   := a == b\n",
    );
    write_file(
        &manifest,
        &format!(
            "crate_name=native_binops_pkg\n\
             out_dir={}\n\
             source={}\n",
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
        "CLI must succeed; exit={:?} stderr={stderr}",
        output.status.code()
    );

    let lib_text = std::fs::read_to_string(out_dir.join("src").join("lib.rs"))
        .expect("read lib.rs");

    // Each op must lower to its native Rust counterpart.
    // The exact whitespace inside the parens is stable
    // from `RustExpr::BinOp`'s emit — `(lhs op rhs)`.
    let cases: &[(&str, &str)] = &[
        ("addU64", "+"),
        ("subU64", "-"),
        ("mulU64", "*"),
        ("divU64", "/"),
        ("modU64", "%"),
        ("ltU64",  "<"),
        ("leU64",  "<="),
        ("eqU64",  "=="),
    ];
    for (fn_name, op) in cases {
        let needle = format!("fn {fn_name}");
        assert!(
            lib_text.contains(&needle),
            "lib.rs missing `{needle}` declaration:\n{lib_text}"
        );
        let body_needle = format!("(_x0 {op} _x1)");
        assert!(
            lib_text.contains(&body_needle),
            "fn {fn_name} body must contain `{body_needle}`:\n{lib_text}"
        );
        // Negative invariant — the typeclass-projection
        // mangled name must NOT leak into the emit; the
        // backend's `try_builtin_app` folded it.
        let projection_mangled = match *op {
            "+"  => "HAdd_hAdd",
            "-"  => "HSub_hSub",
            "*"  => "HMul_hMul",
            "/"  => "HDiv_hDiv",
            "%"  => "HMod_hMod",
            "<"  => "LT_lt",
            "<=" => "LE_le",
            "==" => "BEq_beq",
            _ => unreachable!(),
        };
        assert!(
            !lib_text.contains(projection_mangled),
            "fn {fn_name}: `{projection_mangled}` leaked into emit:\n{lib_text}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}
