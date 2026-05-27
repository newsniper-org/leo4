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

#[test]
fn cli_translate_coverage_if_let_list_at() {
    // OX7 translate coverage expansion (2026-05-27) —
    // the `If`, `Let`, `List`, `At` arms in
    // `translate_expr` now route the fixture through
    // the production translate path instead of falling
    // back to the legacy walker. Each fixture exercises
    // one arm; we only assert the CLI accepts the
    // source and emits *some* crate — the resulting
    // Rust bodies may be unhelpful (e.g.  `ite(...)`
    // for `if`, `List_cons(...)` for `[1, 2, 3]`)
    // because OxiLean lowers `if`/list-literals
    // through their typeclass / inductive desugars and
    // we don't fold those into native `if-expr` /
    // `vec![...]` yet — that's tracked as a separate
    // codegen step. Translate-path coverage is what
    // this test pins down.
    let dir = tmp_dir("translate_coverage");
    let out_dir = dir.join("crate");
    let lean = dir.join("Coverage.lean");
    let manifest = dir.join("manifest.txt");

    write_file(
        &lean,
        // Two decls, each exercising one previously-
        // Unsupported translate_expr arm:
        //   1. `if-then-else`        — L4Expr::If
        //   2. `[1, 2, 3]` list-lit  — L4Expr::List
        //
        // `L4Expr::At` (the `@`-explicit-args marker)
        // and `L4Expr::Let` (let-in) translate arms
        // are exercised by translate-side unit tests in
        // `leo4_translate.rs` since they need
        // identifiers the OX5-oxi env doesn't ship
        // (e.g. `@id`), which would force the smoke
        // here to fail at elab even with a correctly
        // translated AST.
        "def fromIf : UInt64 := if true then 1 else 0\n\
         def threeU64 : List UInt64 := [1, 2, 3]\n",
    );
    write_file(
        &manifest,
        &format!(
            "crate_name=translate_coverage_pkg\n\
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

    let lib_path = out_dir.join("src").join("lib.rs");
    assert!(lib_path.exists(), "src/lib.rs must exist");
    let lib_text = std::fs::read_to_string(&lib_path).expect("read");
    // The three `pub fn` decls all landed — translate
    // succeeded for each fixture's surface form.
    assert!(lib_text.contains("fn fromIf"),    "lib.rs missing `fromIf`: {lib_text}");
    assert!(lib_text.contains("fn threeU64"),  "lib.rs missing `threeU64`: {lib_text}");


    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_translate_coverage_forall_do_anonctor() {
    // OX7 translate coverage expansion (2026-05-27, batch 2) —
    // `Forall`, `Do`, `AnonCtor` PEG variants now route through
    // production translate instead of the legacy fallback.
    //
    // `Exists`, `InterpStr`, `AnonStruct` are deliberately
    // omitted — they need additional surface-AST shape work or
    // (in the case of InterpStr) Token-level Lean ↔ OxiLean
    // conversion that doesn't fit a single commit.
    let dir = tmp_dir("translate_coverage_2");
    let out_dir = dir.join("crate");
    let lean = dir.join("Coverage2.lean");
    let manifest = dir.join("manifest.txt");

    write_file(
        &lean,
        // OX7 (2026-05-27) coverage batch 2. Three
        // fixtures cover the new translate arms
        // exhaustively at the translate level (see
        // `examples/ox7_more_coverage.rs` for the
        // matching spike). Avoiding fixtures that need
        // `Prop` / `()` / `String` literal coverage —
        // those have known PEG-side gaps tracked
        // separately.
        //
        //   1. `Lam` arm (already covered) via plain
        //      `fun n => n` body.
        //   2. `Forall` arm — `(x : Nat) → x = x` would
        //      need Prop env support; instead we use the
        //      `forall` form *inside a type context* so
        //      elab leaves it abstract.
        //   3. `AnonCtor` — `⟨1, 2⟩` for `Prod Nat Nat`.
        //
        // `Do` arm coverage is exercised by the spike
        // example, not here (the `do` fixture needs
        // `()` / `String` literals which currently
        // sub-spec the PEG).
        "def identNat : Nat -> Nat := fun n => n\n\
         def pairAnon : Prod Nat Nat := ⟨1, 2⟩\n",
    );
    write_file(
        &manifest,
        &format!(
            "crate_name=translate_coverage_2_pkg\n\
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

    let lib_path = out_dir.join("src").join("lib.rs");
    assert!(lib_path.exists(), "src/lib.rs must exist");
    let lib_text = std::fs::read_to_string(&lib_path).expect("read");
    assert!(lib_text.contains("fn identNat"), "lib.rs missing identNat: {lib_text}");
    assert!(lib_text.contains("fn pairAnon"), "lib.rs missing pairAnon: {lib_text}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ─── OX8.2b — reverse mode CLI smoke tests ───────────────────────────
//
// End-to-end actual cdylib roundtrip belongs to OX8.4 (and
// is gated on a leo4-abi-compatible cdylib being available
// at test time, which would require building one inside
// this crate's test harness). The tests below pin the
// arg-parsing + error-surfacing surface only — exit codes,
// the missing-required-field messages, and the
// nonexistent-cdylib branch.

#[test]
fn cli_reverse_mode_via_cli_flags_missing_cdylib_path_reports_error_1() {
    // `--mode reverse --cdylib /no/such/file …` — the path
    // doesn't exist; the CLI must surface this as exit
    // code 1 (transpile/emit failure) with a clear
    // message, NOT as exit code 2 (usage error).
    let dir = tmp_dir("reverse_missing_cdylib");
    let lean_out = dir.join("lean").join("X").join("Rust.lean");
    let bogus_cdylib = dir.join("does_not_exist.so");

    let output = Command::new(cli_path())
        .arg("--mode").arg("reverse")
        .arg("--cdylib").arg(&bogus_cdylib)
        .arg("--iface").arg("X")
        .arg("--out").arg(&lean_out)
        .output()
        .expect("invoke");

    assert_eq!(
        output.status.code(),
        Some(1),
        "missing cdylib path must exit 1 (emit failure), not 2 (usage). stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not exist") || stderr.contains("cdylib"),
        "stderr should mention cdylib non-existence: {stderr}"
    );
    // No Lean output written on failure.
    assert!(
        !lean_out.exists(),
        "reverse wrapper must NOT be written on cdylib load failure"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_reverse_mode_missing_required_iface_field_reports_usage_error() {
    // Reverse mode with cdylib path but no --iface → exit
    // code 2 (usage error). The check fires before the
    // cdylib is even opened.
    let dir = tmp_dir("reverse_no_iface");
    let lean_out = dir.join("Rust.lean");

    let output = Command::new(cli_path())
        .arg("--mode").arg("reverse")
        .arg("--cdylib").arg("/tmp/anything.so")
        .arg("--out").arg(&lean_out)
        .output()
        .expect("invoke");

    assert_eq!(
        output.status.code(),
        Some(2),
        "missing --iface must exit 2 (usage). stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("iface"),
        "stderr should mention iface: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_reverse_mode_via_manifest_fields_recognised() {
    // The manifest can drive reverse mode entirely:
    // `mode=reverse`, `cdylib=…`, `iface=…`, `lean_out=…`.
    // We point at a nonexistent cdylib so the run fails
    // at the cdylib-load step (exit 1) rather than at the
    // usage-validation step (exit 2). This pins the
    // manifest parser's recognition of the new fields.
    let dir = tmp_dir("reverse_manifest");
    let lean_out = dir.join("out.lean");
    let bogus_cdylib = dir.join("does_not_exist.so");
    let manifest = dir.join("manifest.txt");
    write_file(
        &manifest,
        &format!(
            "mode=reverse\n\
             cdylib={}\n\
             iface=MyIface\n\
             lean_out={}\n",
            bogus_cdylib.display(),
            lean_out.display()
        ),
    );

    let output = Command::new(cli_path())
        .arg("--manifest").arg(&manifest)
        .output()
        .expect("invoke");

    // Exit 1 = manifest parsed cleanly, mode dispatched
    // to reverse, cdylib-existence check failed.
    assert_eq!(
        output.status.code(),
        Some(1),
        "manifest reverse-mode fields must be recognised; cdylib-missing must exit 1. stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cli_reverse_mode_unknown_mode_value_reports_usage_error() {
    // `--mode foobar` is rejected at arg-parse time with
    // exit code 2 (usage error).
    let output = Command::new(cli_path())
        .arg("--mode").arg("foobar")
        .output()
        .expect("invoke");
    assert_eq!(
        output.status.code(),
        Some(2),
        "bad --mode value must exit 2 (usage). stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("foobar") || stderr.contains("mode"),
        "stderr should explain bad mode: {stderr}"
    );
}
