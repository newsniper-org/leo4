//! `leo4` — top-level CLI.
//!
//! Three subcommands:
//!
//! - `leo4 create <direction> <dir>` — **new project**. Creates
//!   the target directory (or expects it to exist + be empty)
//!   and writes a complete buildable skeleton: `Cargo.toml`,
//!   `src/`, `lean/`, `README.md`. Matches `cargo new`'s
//!   ergonomics.
//!
//! - `leo4 init <direction>` — **in-place integration**. Targets
//!   an *existing* Cargo crate (cwd by default, or `--dir`) and
//!   adds the leo4 bits without overwriting the user's
//!   existing `src/`:
//!     * appends a `# leo4 integration` block to `Cargo.toml`
//!       (idempotent — re-running does not duplicate);
//!     * writes `build.rs` (forward only) if absent;
//!     * creates `lean/` with a starter `Sample.lean` +
//!       `lakefile.lean` if absent.
//!
//! - `leo4 run` — **one-shot build + run** (Phase 10-D1).
//!   Detects direction from `Cargo.toml` (`crate-type =
//!   ["cdylib"]` ⇒ reverse, else forward), then executes the
//!   pipeline end-to-end: Lake build, plugin/emit invocation,
//!   Cargo build or final lean_exe build, then runs the binary
//!   with the matching env matrix wired up.
//!
//! Both `create` and `init` support `--forward` (default) and
//! `--reverse` direction. Forward = Lean exports + Rust caller
//! (`@[leo4_export]` / `leo4::import!`). Reverse = Rust cdylib
//! + Lean caller (`#[leo4::export]` / generated Lean wrapper).
//!
//! `create` / `init` do not invoke `cargo` or `lake` — they just
//! edit / write files. `run` is the orchestration entry point.

#![allow(clippy::missing_errors_doc)]

/// Post-OX6 — per-(sub)crate `leo4.toml` config (replaces
/// the historical `--impl <kind>` CLI flag on `create` /
/// `init`). See module doc for the file schema.
pub mod config;

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use clap::{Parser, Subcommand};

use crate::config::{ImplEntry, Leo4Config};

#[derive(Parser, Debug)]
#[command(
    name = "leo4",
    version,
    about = "leo4 — Lean 4 ↔ Rust interop project scaffolder",
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Create a NEW leo4 project in a fresh directory.
    ///
    /// Runtime impl selection moved to the scaffolded
    /// `leo4.toml`. Edit `[[impl]] kind = "..."` post-
    /// create to switch impls or add additional ones —
    /// see `leo4-cli/src/config.rs` module docs.
    Create {
        direction: Direction,
        /// Target directory. Created if absent; must be empty if it
        /// exists.
        dir: PathBuf,
        /// Project name. Defaults to the directory basename.
        #[arg(long)]
        name: Option<String>,
        /// Local checkout path of the leo4 repo. The scaffold's
        /// `Cargo.toml` + `lakefile.lean` reference it. Defaults
        /// to `../leo4` (sibling layout).
        #[arg(long)]
        leo4_root: Option<PathBuf>,
    },

    /// Add leo4 integration to an EXISTING Cargo crate (in cwd
    /// or `--dir`). Edits `Cargo.toml`, writes `build.rs` /
    /// `lean/` only if absent.
    Init {
        direction: Direction,
        /// Target directory containing a `Cargo.toml`. Defaults
        /// to cwd.
        #[arg(long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        leo4_root: Option<PathBuf>,
        /// Lean implementation to target (same semantics as
        /// `leo4 create --impl`). Required.
        #[arg(long, value_parser = parse_impl_kind)]
        r#impl: ImplKind,
    },

    /// Build + run the project end-to-end. Detects direction
    /// automatically from `Cargo.toml`'s `[lib] crate-type`
    /// (`["cdylib"]` ⇒ reverse, else forward). Reads the target
    /// implementation from `<dir>/.leo4-impl` (written by
    /// `leo4 create` / `leo4 init`); override with `--impl`.
    Run {
        /// Override auto-detection of forward vs reverse.
        #[arg(long)]
        direction: Option<Direction>,
        /// Forward: Lean root module name to invoke
        /// `leo4plugin` on (default `Sample`). Reverse: Lean
        /// `lean_lib` that hosts the generated wrapper
        /// (default `CamelCase(crate_name)`).
        #[arg(long)]
        iface: Option<String>,
        /// Path to a checkout of the leo4 repo (helper binaries
        /// + Lake packages live there). Defaults to `../leo4`.
        #[arg(long)]
        leo4_root: Option<PathBuf>,
        /// Project directory containing `Cargo.toml`. Defaults
        /// to cwd.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Override the `<dir>/.leo4-impl` marker. Same accepted
        /// values as `leo4 create --impl`.
        #[arg(long, value_parser = parse_impl_kind)]
        r#impl: Option<ImplKind>,
        /// Extra args forwarded to the final binary.
        #[arg(last = true)]
        args: Vec<String>,
    },
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum Direction {
    /// `@[leo4_export]` on Lean side, `leo4::import!` on Rust side.
    Forward,
    /// `#[leo4::export]` on Rust side, generated Lean wrapper.
    Reverse,
}

/// Lean implementation the scaffold targets. Determines which
/// transport (`SPEC/canonical-abi.md` §14 / `SPEC/wit/leo4-host.wit`
/// / `SPEC/rust-native-lean.md`) the generated project uses.
///
/// Not a `clap::ValueEnum` because `rust` is an alias for
/// `rust-native`. Custom `value_parser` (`parse_impl_kind`) handles
/// both spellings.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ImplKind {
    /// `crates/leo4-mslean4` path — reference Lean 4 via
    /// `<lean/lean.h>` C ABI shim.
    Mslean4,
    /// `leo4-rust-native` adapter path (out-of-tree) — Rust-native
    /// Lean impl via direct in-process Rust call.
    /// Currently deferred (`SPEC/rust-native-lean.md` §8).
    RustNative,
    /// `leo4-oxilean-build` transpile path — OxiLean's
    /// `oxilean-codegen::rust_target_backend` lowers Lean
    /// source to a plain Rust crate at build time; the user
    /// then calls it as ordinary Rust. Bypasses the
    /// evaluator-hook deferral that blocks `RustNative`.
    /// Currently scaffold-only (`SPEC/rust-native-lean.md` §9).
    RustTranspile,
}

impl ImplKind {
    fn marker_str(&self) -> &'static str {
        match self {
            ImplKind::Mslean4 => "mslean4",
            ImplKind::RustNative => "rust-native",
            ImplKind::RustTranspile => "rust-transpile",
        }
    }
}

fn parse_impl_kind(s: &str) -> Result<ImplKind, String> {
    match s {
        "mslean4" => Ok(ImplKind::Mslean4),
        "rust-native" | "rust" => Ok(ImplKind::RustNative),
        "rust-transpile" => Ok(ImplKind::RustTranspile),
        other => Err(format!(
            "unknown --impl value `{other}`. Accepted: `mslean4`, \
             `rust-native` (alias `rust`), `rust-transpile`."
        )),
    }
}

fn main() {
    let cli = Cli::parse();
    let res = match cli.cmd {
        Cmd::Create { direction, dir, name, leo4_root } => {
            run_create(direction, dir, name, leo4_root)
        }
        Cmd::Init { direction, dir, leo4_root, r#impl } => {
            run_init(direction, dir, leo4_root, r#impl)
        }
        Cmd::Run { direction, iface, leo4_root, dir, r#impl, args } => {
            run_run(direction, iface, leo4_root, dir, r#impl, args)
        }
    };
    if let Err(e) = res {
        eprintln!("leo4: {e}");
        std::process::exit(1);
    }
}

/// Reject scaffolding under a not-yet-supported impl with a clear
/// pointer at the SPEC. Returns `Ok(())` for supported impls.
fn check_impl_supported(kind: &ImplKind) -> Result<(), String> {
    match kind {
        ImplKind::Mslean4 => Ok(()),
        ImplKind::RustNative => Err(
            "--impl rust-native is currently deferred. The integration \
             contract is pinned at `SPEC/rust-native-lean.md` §2, but \
             no in-tree scaffolding ships yet — you'd need an external \
             adapter crate (`leo4-<impl>`, e.g. `leo4-oxilean`).\n\n\
             See `SPEC/rust-native-lean.md` §8 for the activation plan, \
             or use `--impl mslean4` for the reference Lean 4 path that \
             ships today.".into()
        ),
        ImplKind::RustTranspile => Err(
            "--impl rust-transpile is currently scaffold-only. The \
             transpile path (Lean source → Rust crate via OxiLean's \
             `oxilean-codegen::rust_target_backend`) is pinned at \
             `SPEC/rust-native-lean.md` §9 and the build-side adapter \
             crate exists at `sibling/leo4-oxilean-build/`, but no \
             user-package scaffold layout is wired into `leo4 create` \
             / `leo4 init` yet.\n\n\
             See `SPEC/rust-native-lean.md` §9.6 for the open \
             questions an adapter author still answers per-fixture, \
             or use `--impl mslean4` for the reference Lean 4 path \
             that ships today.".into()
        ),
    }
}

/// **Legacy** — write the `<dir>/.leo4-impl` marker
/// file. Post-OX6 the canonical runtime-impl selector
/// is `<dir>/leo4.toml` (see `write_leo4_toml`); this
/// marker stays around solely so existing projects
/// keep parsing under `leo4 run` until they get
/// migrated by `leo4 init` (chunk 4).
fn write_impl_marker(dir: &Path, kind: &ImplKind) -> Result<(), String> {
    let p = dir.join(".leo4-impl");
    fs::write(&p, format!("{}\n", kind.marker_str()))
        .map_err(|e| format!("write {p:?}: {e}"))
}

/// Read the `<dir>/.leo4-impl` marker if present.
/// Legacy companion to `write_impl_marker`.
fn read_impl_marker(dir: &Path) -> Option<ImplKind> {
    let p = dir.join(".leo4-impl");
    let raw = fs::read_to_string(&p).ok()?;
    parse_impl_kind(raw.trim()).ok()
}

/// Post-OX6 — write a fresh `<dir>/leo4.toml` with a
/// single-impl scaffold. `kind` selects which runtime
/// impl the scaffold targets; users can edit the file
/// post-create to add more `[[impl]]` entries or
/// switch kinds. Default kind for `leo4 create` is
/// `mslean4` (the only fully-shipping path today —
/// matches the historical `--impl mslean4` flow).
fn write_leo4_toml(dir: &Path, kind: &str) -> Result<(), String> {
    let cfg = Leo4Config { impls: vec![ImplEntry::new(kind)] };
    let p = dir.join("leo4.toml");
    fs::write(&p, cfg.render()).map_err(|e| format!("write {p:?}: {e}"))
}

// ─── `leo4 create` (new directory) ──────────────────────────────────

fn run_create(
    direction: Direction,
    dir: PathBuf,
    name: Option<String>,
    leo4_root: Option<PathBuf>,
) -> Result<(), String> {
    let dir = abs(&dir)?;
    if dir.exists() {
        let empty = fs::read_dir(&dir)
            .map_err(|e| format!("read_dir {dir:?}: {e}"))?
            .next()
            .is_none();
        if !empty {
            return Err(format!(
                "create: {dir:?} exists and is not empty. Use `leo4 init` to merge into an existing crate."
            ));
        }
    } else {
        fs::create_dir_all(&dir)
            .map_err(|e| format!("create_dir_all {dir:?}: {e}"))?;
    }

    let project_name = name.unwrap_or_else(|| dir_basename(&dir));
    let leo4_root_str = resolve_leo4_root(leo4_root);

    match direction {
        Direction::Forward => scaffold_forward_full(&dir, &project_name, &leo4_root_str)?,
        Direction::Reverse => scaffold_reverse_full(&dir, &project_name, &leo4_root_str)?,
    }
    // Post-OX6 default impl is `mslean4` — the only
    // fully-shipping path. Users wanting `rust-native`
    // or `rust-transpile` edit `leo4.toml` post-create
    // (and accept that those paths are scaffold-only /
    // deferred per `SPEC/rust-native-lean.md` §8–9).
    write_leo4_toml(&dir, ImplKind::Mslean4.marker_str())?;
    println!(
        "leo4 create: {project_name} ({direction:?}, impl=mslean4) → {dir:?}"
    );
    println!("  edit {}/leo4.toml to switch impl or add more", dir.display());
    println!("  next: cat {}/README.md", dir.display());
    Ok(())
}

// ─── `leo4 init` (existing crate) ───────────────────────────────────

fn run_init(
    direction: Direction,
    dir: Option<PathBuf>,
    leo4_root: Option<PathBuf>,
    impl_kind: ImplKind,
) -> Result<(), String> {
    check_impl_supported(&impl_kind)?;
    let dir = match dir {
        Some(d) => abs(&d)?,
        None => std::env::current_dir()
            .map_err(|e| format!("getcwd: {e}"))?,
    };
    let cargo_toml = dir.join("Cargo.toml");
    if !cargo_toml.exists() {
        return Err(format!(
            "init: no Cargo.toml at {dir:?}. Run `cargo init` first, or use `leo4 create` for a fresh project."
        ));
    }

    // If a marker already exists, require it to match — switching
    // impls mid-project is not a workflow we support.
    if let Some(existing) = read_impl_marker(&dir) {
        if existing != impl_kind {
            return Err(format!(
                "init: {dir:?} already carries `.leo4-impl = {}` but you passed `--impl {}`. \
                 Switching impls is not supported; remove `.leo4-impl` manually to override.",
                existing.marker_str(),
                impl_kind.marker_str(),
            ));
        }
    }

    let pkg_name = read_cargo_pkg_name(&cargo_toml)
        .unwrap_or_else(|| dir_basename(&dir));
    let leo4_root_str = resolve_leo4_root(leo4_root);

    match direction {
        Direction::Forward => integrate_forward(&dir, &pkg_name, &leo4_root_str)?,
        Direction::Reverse => integrate_reverse(&dir, &pkg_name, &leo4_root_str)?,
    }
    write_impl_marker(&dir, &impl_kind)?;
    println!(
        "leo4 init: integrated {direction:?} scaffold (impl={}) into {dir:?}",
        impl_kind.marker_str()
    );
    println!("  Cargo.toml extended; lean/ + (forward) build.rs created if absent.");
    Ok(())
}

fn integrate_forward(dir: &Path, name: &str, leo4_root: &str) -> Result<(), String> {
    extend_cargo_toml(
        dir,
        &format!(
            r#"
# ─── leo4 integration (forward direction) ───────────────────────────
[dependencies]
leo4 = {{ path = "{leo4_root}/crates/leo4" }}

[build-dependencies]
leo4-build = {{ path = "{leo4_root}/crates/leo4-build" }}
"#
        ),
    )?;
    write_if_absent(dir, "build.rs", BUILD_RS_FORWARD)?;
    write_if_absent_dir(dir, "lean/lakefile.lean", &lakefile_forward(name, leo4_root))?;
    write_if_absent_dir(dir, "lean/lean-toolchain", "leanprover/lean4:v4.29.1\n")?;
    write_if_absent_dir(dir, "lean/Sample.lean", SAMPLE_LEAN_FORWARD)?;
    write_if_absent_dir(dir, "lean/.gitignore", GITIGNORE_FORWARD_LEAN)?;
    Ok(())
}

fn integrate_reverse(dir: &Path, name: &str, leo4_root: &str) -> Result<(), String> {
    let iface = camel_case(&name.replace('-', "_"));
    extend_cargo_toml(
        dir,
        &format!(
            r#"
# ─── leo4 integration (reverse direction) ───────────────────────────
[lib]
crate-type = ["cdylib"]

[dependencies]
leo4 = {{ path = "{leo4_root}/crates/leo4", features = ["rust-exports"] }}
"#
        ),
    )?;
    write_if_absent_dir(dir, "lean/lakefile.lean", &lakefile_reverse(name, &iface, leo4_root))?;
    write_if_absent_dir(dir, "lean/lean-toolchain", "leanprover/lean4:v4.29.1\n")?;
    write_if_absent_dir(dir, "lean/Main.lean", &main_lean_reverse(&iface))?;
    write_if_absent_dir(dir, "lean/.gitignore", &gitignore_reverse_lean(&iface))?;
    Ok(())
}

// ─── `leo4 run` (build + execute) ───────────────────────────────────

fn run_run(
    direction_arg: Option<Direction>,
    iface_arg: Option<String>,
    leo4_root: Option<PathBuf>,
    dir: Option<PathBuf>,
    impl_arg: Option<ImplKind>,
    args: Vec<String>,
) -> Result<(), String> {
    let dir = match dir {
        Some(d) => abs(&d)?,
        None => std::env::current_dir()
            .map_err(|e| format!("getcwd: {e}"))?,
    };
    let cargo_toml = dir.join("Cargo.toml");
    if !cargo_toml.exists() {
        return Err(format!(
            "run: no Cargo.toml at {dir:?}. Run from inside a leo4 project."
        ));
    }

    // Resolve --impl: explicit override > .leo4-impl marker > error.
    let impl_kind = match impl_arg {
        Some(k) => k,
        None => read_impl_marker(&dir).ok_or_else(|| {
            format!(
                "run: no `.leo4-impl` marker at {dir:?}. \
                 Either re-run `leo4 init --impl <mslean4|rust-native>` to write one, \
                 or pass `--impl <…>` explicitly to this command."
            )
        })?,
    };
    check_impl_supported(&impl_kind)?;

    let pkg_name = read_cargo_pkg_name(&cargo_toml)
        .ok_or_else(|| format!("run: cannot read package name from {cargo_toml:?}"))?;
    let crate_name = pkg_name.replace('-', "_");
    let direction = direction_arg.unwrap_or_else(|| detect_direction(&cargo_toml));
    let leo4_root_dir = resolve_leo4_root_dir(leo4_root, &dir)?;

    match direction {
        Direction::Forward => {
            let iface = iface_arg.unwrap_or_else(|| "Sample".to_string());
            run_forward(&dir, &iface, &leo4_root_dir, &args)
        }
        Direction::Reverse => {
            let iface = iface_arg.unwrap_or_else(|| camel_case(&crate_name));
            run_reverse(&dir, &pkg_name, &crate_name, &iface, &leo4_root_dir, &args)
        }
    }
}

fn detect_direction(cargo_toml: &Path) -> Direction {
    let s = fs::read_to_string(cargo_toml).unwrap_or_default();
    if s.contains("\"cdylib\"") {
        Direction::Reverse
    } else {
        Direction::Forward
    }
}

fn resolve_leo4_root_dir(p: Option<PathBuf>, project_dir: &Path) -> Result<PathBuf, String> {
    let raw = match p {
        Some(p) if p.is_absolute() => p,
        Some(p) => project_dir.join(p),
        None => project_dir.join("..").join("leo4"),
    };
    raw.canonicalize().map_err(|e| {
        format!("--leo4-root {raw:?}: {e} (pass --leo4-root explicitly if leo4 is not at ../leo4)")
    })
}

fn run_forward(
    dir: &Path,
    iface: &str,
    _leo4_root: &Path,
    args: &[String],
) -> Result<(), String> {
    let lean_dir = dir.join("lean");
    if !lean_dir.exists() {
        return Err(format!("run: no `lean/` directory at {dir:?}"));
    }

    step("[1/3] lake build");
    run_cmd(
        Command::new("lake").arg("build").current_dir(&lean_dir),
        "lake build",
    )?;

    step(&format!("[2/3] lake exe leo4plugin {iface}"));
    run_cmd(
        Command::new("lake")
            .args(["exe", "leo4plugin", iface])
            .current_dir(&lean_dir),
        "leo4plugin",
    )?;

    step("[3/3] cargo run");
    let mut cmd = Command::new("cargo");
    cmd.arg("run").current_dir(dir);
    if !args.is_empty() {
        cmd.arg("--").args(args);
    }
    run_cmd(&mut cmd, "cargo run")
}

fn run_reverse(
    dir: &Path,
    pkg_name: &str,
    crate_name: &str,
    iface: &str,
    leo4_root: &Path,
    args: &[String],
) -> Result<(), String> {
    let lean_dir = dir.join("lean");
    if !lean_dir.exists() {
        return Err(format!("run: no `lean/` directory at {dir:?}"));
    }
    let leo4_target = leo4_root.join("target").join("release");
    let emit_bin = leo4_target.join(bin_name("leo4-rust-emit"));
    let worker_bin = leo4_target.join(bin_name("leo4-rust-worker"));
    let bridge_ar = leo4_target.join("libleo4_rust_bridge.a");

    if !emit_bin.exists() || !worker_bin.exists() || !bridge_ar.exists() {
        step("[helpers] cargo build --release -p leo4-rust-{emit,worker,bridge}");
        run_cmd(
            Command::new("cargo")
                .args([
                    "build", "--release",
                    "-p", "leo4-rust-emit",
                    "-p", "leo4-rust-worker",
                    "-p", "leo4-rust-bridge",
                ])
                .current_dir(leo4_root),
            "cargo build (leo4 helpers)",
        )?;
    }

    step(&format!("[1/4] cargo build --release -p {pkg_name}"));
    run_cmd(
        Command::new("cargo")
            .args(["build", "--release", "-p", pkg_name])
            .current_dir(dir),
        "cargo build (cdylib)",
    )?;

    let cargo_target = dir.join("target").join("release");
    let cdylib = find_cdylib(&cargo_target, crate_name)?;

    step("[2/4] lake run Leo4Rust/regenerate (emit wrapper)");
    // Pre-resolved cdylib path goes through env so the script
    // doesn't have to walk-search the workspace target dir again.
    let mut emit_cmd = Command::new("lake");
    emit_cmd
        .args(["run", "Leo4Rust/regenerate"])
        .current_dir(&lean_dir)
        .env("LEO4_RUST_EMIT_BIN", &emit_bin)
        .env("LEO4_RUST_CDYLIB", &cdylib)
        .env("LEO4_RUST_IFACE", iface);
    run_cmd(&mut emit_cmd, "Leo4Rust/regenerate")?;

    step("[3/4] lake build (auto-links bridge + glue via Leo4Rust extern_libs)");
    run_cmd(
        Command::new("lake").arg("build").current_dir(&lean_dir),
        "lake build",
    )?;

    let exe = lean_dir.join(".lake").join("build").join("bin").join(bin_name(crate_name));
    if !exe.exists() {
        return Err(format!(
            "run: lean exe not found at {exe:?}. Check `lean_exe {crate_name}` is defined in lakefile.lean."
        ));
    }

    step(&format!("[4/4] running {}", exe.display()));
    let mut cmd = Command::new(&exe);
    cmd.env("LEO4_RUST_CDYLIB", &cdylib)
        .env("LEO4_RUST_WORKER_BIN", &worker_bin)
        .env("LEO4_RUST_HANDSHAKE_PKG", crate_name)
        .env("LEO4_RUST_HANDSHAKE_IFACE", iface);
    if !args.is_empty() {
        cmd.args(args);
    }
    run_cmd(&mut cmd, "lean exe")
}

fn step(label: &str) {
    eprintln!("[leo4 run] {label}");
}

fn run_cmd(cmd: &mut Command, label: &str) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|e| format!("{label}: spawn: {e}"))?;
    if !status.success() {
        return Err(format!("{label}: exited {status}"));
    }
    Ok(())
}

fn find_cdylib(target_release: &Path, crate_name: &str) -> Result<PathBuf, String> {
    let candidates = [
        format!("lib{crate_name}.so"),
        format!("lib{crate_name}.dylib"),
        format!("{crate_name}.dll"),
    ];
    for c in &candidates {
        let p = target_release.join(c);
        if p.exists() {
            return Ok(p);
        }
    }
    Err(format!(
        "cdylib not found under {}; tried {candidates:?}. Did `cargo build --release` succeed?",
        target_release.display()
    ))
}

fn bin_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

// ─── full scaffold (`create`) ────────────────────────────────────────

fn scaffold_forward_full(dir: &Path, name: &str, leo4_root: &str) -> Result<(), String> {
    write_required(dir, "Cargo.toml", &cargo_toml_forward(name, leo4_root))?;
    write_required(dir, "build.rs", BUILD_RS_FORWARD)?;
    write_required(dir, "src/main.rs", &main_rs_forward(name))?;
    write_required(dir, "lean/lakefile.lean", &lakefile_forward(name, leo4_root))?;
    write_required(dir, "lean/lean-toolchain", "leanprover/lean4:v4.29.1\n")?;
    write_required(dir, "lean/Sample.lean", SAMPLE_LEAN_FORWARD)?;
    write_required(dir, "lean/.gitignore", GITIGNORE_FORWARD_LEAN)?;
    write_required(dir, ".gitignore", GITIGNORE_REVERSE_ROOT)?;
    write_required(dir, "README.md", &readme_forward(name, leo4_root))?;
    Ok(())
}

fn scaffold_reverse_full(dir: &Path, name: &str, leo4_root: &str) -> Result<(), String> {
    let iface = camel_case(&name.replace('-', "_"));
    write_required(dir, "Cargo.toml", &cargo_toml_reverse(name, leo4_root))?;
    write_required(dir, "src/lib.rs", &lib_rs_reverse(name))?;
    write_required(dir, "lean/lakefile.lean", &lakefile_reverse(name, &iface, leo4_root))?;
    write_required(dir, "lean/lean-toolchain", "leanprover/lean4:v4.29.1\n")?;
    write_required(dir, "lean/Main.lean", &main_lean_reverse(&iface))?;
    write_required(dir, "lean/.gitignore", &gitignore_reverse_lean(&iface))?;
    write_required(dir, ".gitignore", GITIGNORE_REVERSE_ROOT)?;
    write_required(dir, "README.md", &readme_reverse(name, &iface, leo4_root))?;
    Ok(())
}

// ─── Templates ──────────────────────────────────────────────────────

const BUILD_RS_FORWARD: &str = r#"// Wire the Lake-built shim into Cargo's compile environment.
fn main() {
    let lake_build = "lean/.lake/build/leo4";
    leo4_build::wire(lake_build).expect("leo4-build: wire shim");
}
"#;

/// `.gitignore` block for a reverse-direction scaffold's `lean/`
/// dir. The `<iface>/` ignore covers the auto-generated wrapper
/// module dir produced by `lake run Leo4Rust/regenerate`.
fn gitignore_reverse_lean(iface: &str) -> String {
    format!(
        r#"# Lake-local build cache + dep manifest (regenerated on every `lake build`).
.lake/
lake-manifest.json

# Phase 9 reverse-direction emit staging dir.
.leo4-emit/

# Auto-generated Lean wrapper module (produced by
# `lake run Leo4Rust/regenerate` / `leo4 run` from the cdylib's
# EXPORTS slice). Regenerated on every build.
{iface}/
"#
    )
}

/// `.gitignore` block for the project root of a reverse-direction
/// scaffold (alongside the Cargo crate). `cargo` already ignores
/// `target/` but we set it explicitly so users running `git init`
/// from a fresh `leo4 create` get a complete starting state.
const GITIGNORE_REVERSE_ROOT: &str = r#"# Cargo
/target/

# IDE
/.idea/
/.vscode/
.DS_Store
"#;

/// `.gitignore` block for the forward-direction scaffold's `lean/`
/// dir. The plugin-emitted `.lake/build/leo4/` files (schema /
/// mangling / handshake) are regenerated every `lake exe leo4plugin`
/// run; the user shouldn't commit them.
const GITIGNORE_FORWARD_LEAN: &str = r#"# Lake-local build cache + dep manifest (regenerated on every `lake build`).
.lake/
lake-manifest.json
"#;

const SAMPLE_LEAN_FORWARD: &str = r#"import Leo4

@[leo4_export]
def hello : String := "hello from Lean"

@[leo4_export]
def add (a b : UInt64) : UInt64 := a + b
"#;

fn cargo_toml_forward(name: &str, leo4_root: &str) -> String {
    format!(
        r#"[package]
name    = "{name}"
version = "0.1.0"
edition = "2024"

[dependencies]
leo4 = {{ path = "{leo4_root}/crates/leo4" }}

[build-dependencies]
leo4-build = {{ path = "{leo4_root}/crates/leo4-build" }}
"#
    )
}

fn cargo_toml_reverse(name: &str, leo4_root: &str) -> String {
    format!(
        r#"[package]
name    = "{name}"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
leo4 = {{ path = "{leo4_root}/crates/leo4", features = ["rust-exports"] }}
"#
    )
}

fn main_rs_forward(name: &str) -> String {
    let crate_name = name.replace('-', "_");
    format!(
        r#"//! `{name}` — leo4 forward-direction demo.

mod sample {{
    leo4::import! {{
        fn hello() -> String;
        fn add(a: u64, b: u64) -> u64;
    }}
}}

fn main() -> Result<(), leo4::LeanError> {{
    let lean = leo4::Lean::open(env!("LEO4_SHIM_SO"), env!("LEO4_HANDSHAKE_FILE"))?;
    println!("{{}}", sample::hello(&lean)?);
    println!("2 + 3 = {{}}", sample::add(&lean, 2, 3)?);
    let _ = "{crate_name}";  // used by lake-side IDL generation
    Ok(())
}}
"#
    )
}

fn lib_rs_reverse(name: &str) -> String {
    format!(
        r#"//! `{name}` — leo4 reverse-direction demo. Run
//! `leo4-rust-emit --emit-lean` against the built cdylib to
//! generate the matching Lean wrapper module.

#[leo4::export]
pub fn double(n: u64) -> u64 {{
    n.saturating_mul(2)
}}

#[leo4::export]
pub fn greet(who: String) -> String {{
    format!("hello, {{who}}, from Rust")
}}
"#
    )
}

fn lakefile_forward(name: &str, leo4_root: &str) -> String {
    let crate_name = name.replace('-', "_");
    format!(
        r#"import Lake
open Lake DSL

package {crate_name}

require Leo4       from "{leo4_root}/lake/Leo4"
require Leo4Plugin from "{leo4_root}/lake/Leo4Plugin"

@[default_target]
lean_lib Sample where
  globs := #[`Sample]
"#
    )
}

fn lakefile_reverse(name: &str, iface: &str, leo4_root: &str) -> String {
    let crate_name = name.replace('-', "_");
    format!(
        r#"import Lake
open Lake DSL

package {crate_name} where
  srcDir := "."

require Leo4     from "{leo4_root}/lake/Leo4"
require Leo4Rust from "{leo4_root}/lake/Leo4Rust"
-- `require Leo4Rust` pulls in two `extern_lib`s that Lake
-- auto-links into `lean_exe`: `libleo4_rust_bridge.a` (the
-- cargo-built dispatcher) and `libleo4_rust_bridge_lean.a`
-- (the leanc-compiled glue shim).

-- The generated wrapper lands at `{iface}/Rust.lean`. The
-- `.submodules` glob pulls every file under that directory in.
lean_lib {iface} where
  globs := #[.submodules `{iface}]

@[default_target]
lean_exe {crate_name} where
  root := `Main
"#
    )
}

fn main_lean_reverse(iface: &str) -> String {
    format!(
        r#"-- Generated wrapper module is at `{iface}/Rust.lean`
-- (produced by `leo4-rust-emit --emit-lean`).
import {iface}.Rust

open {iface}.Rust

def main : IO Unit := do
  IO.println s!"schema_hash = {{schemaHash}}"
  IO.println s!"double(21) = {{← double 21}}"
  IO.println s!"greet: {{← greet \"world\"}}"
"#
    )
}

fn readme_forward(name: &str, _leo4_root: &str) -> String {
    format!(
        r#"# {name}

leo4 forward-direction scaffold. Lean exports
`hello : String` and `add (a b : UInt64) : UInt64`; Rust calls
them via `leo4::import!`.

## Build + run (recommended)

```sh
leo4 run
```

This runs the three steps below automatically.

## Build + run (manual, for debugging)

```sh
cd lean && lake build && lake exe leo4plugin Sample && cd ..
cargo run
```

Expected:

```
hello from Lean
2 + 3 = 5
```
"#
    )
}

fn readme_reverse(name: &str, iface: &str, leo4_root: &str) -> String {
    let crate_name = name.replace('-', "_");
    format!(
        r#"# {name}

leo4 reverse-direction scaffold. Rust exposes `double` and
`greet` via `#[leo4::export]`; Lean calls them.

## Build + run (recommended)

```sh
leo4 run --leo4-root {leo4_root}
```

`leo4 run` orchestrates the whole pipeline:
1. Builds the cdylib via `cargo build --release`.
2. Builds the leo4 helper binaries
   (`leo4-rust-{{emit,worker,bridge}}`) under `{leo4_root}`
   if absent.
3. Emits the Lean wrapper module to `lean/{iface}/Rust.lean`.
4. Runs `lake build` (which auto-links the dispatcher +
   glue archives via Lake `extern_lib`s exposed by
   `Leo4Rust`).
5. Executes `lean/.lake/build/bin/{crate_name}` with
   `LEO4_RUST_CDYLIB` / `LEO4_RUST_WORKER_BIN` /
   `LEO4_RUST_HANDSHAKE_PKG` / `LEO4_RUST_HANDSHAKE_IFACE`
   wired up.

## Build + run (manual, for debugging)

```sh
cargo build --release
(cd {leo4_root} && cargo build --release -p leo4-rust-bridge \
                                        -p leo4-rust-worker \
                                        -p leo4-rust-emit)
cd lean
LEO4_RUST_EMIT_BIN={leo4_root}/target/release/leo4-rust-emit \
LEO4_RUST_IFACE={iface} \
  lake run Leo4Rust/regenerate   # Phase 10-D2 Lake-driven emit
lake build
LEO4_RUST_CDYLIB=$(realpath ../target/release/lib{crate_name}.so) \
LEO4_RUST_WORKER_BIN={leo4_root}/target/release/leo4-rust-worker \
LEO4_RUST_HANDSHAKE_PKG={crate_name} \
LEO4_RUST_HANDSHAKE_IFACE={iface} \
  ./.lake/build/bin/{crate_name}
```
"#
    )
}

// ─── file IO helpers ────────────────────────────────────────────────

fn write_required(dir: &Path, rel: &str, contents: &str) -> Result<(), String> {
    let full = dir.join(rel);
    if let Some(p) = full.parent() {
        fs::create_dir_all(p).map_err(|e| format!("create_dir_all {p:?}: {e}"))?;
    }
    fs::write(&full, contents).map_err(|e| format!("write {full:?}: {e}"))?;
    Ok(())
}

fn write_if_absent(dir: &Path, rel: &str, contents: &str) -> Result<(), String> {
    let full = dir.join(rel);
    if full.exists() {
        println!("  skip {} (already exists)", full.display());
        return Ok(());
    }
    fs::write(&full, contents).map_err(|e| format!("write {full:?}: {e}"))?;
    println!("  + {}", full.display());
    Ok(())
}

fn write_if_absent_dir(dir: &Path, rel: &str, contents: &str) -> Result<(), String> {
    let full = dir.join(rel);
    if let Some(p) = full.parent() {
        fs::create_dir_all(p).map_err(|e| format!("create_dir_all {p:?}: {e}"))?;
    }
    if full.exists() {
        println!("  skip {} (already exists)", full.display());
        return Ok(());
    }
    fs::write(&full, contents).map_err(|e| format!("write {full:?}: {e}"))?;
    println!("  + {}", full.display());
    Ok(())
}

/// Append `block` to `Cargo.toml` unless the marker line already
/// exists (idempotent re-run).
fn extend_cargo_toml(dir: &Path, block: &str) -> Result<(), String> {
    let p = dir.join("Cargo.toml");
    let existing = fs::read_to_string(&p)
        .map_err(|e| format!("read {p:?}: {e}"))?;
    if existing.contains("# ─── leo4 integration") {
        println!("  skip Cargo.toml (leo4 integration block already present)");
        return Ok(());
    }
    let mut f = fs::OpenOptions::new()
        .append(true)
        .open(&p)
        .map_err(|e| format!("open append {p:?}: {e}"))?;
    f.write_all(block.as_bytes())
        .map_err(|e| format!("append {p:?}: {e}"))?;
    println!("  + Cargo.toml (leo4 integration block appended)");
    Ok(())
}

// ─── small utilities ────────────────────────────────────────────────

fn abs(p: &Path) -> Result<PathBuf, String> {
    if p.is_absolute() {
        Ok(p.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|e| format!("getcwd: {e}"))?
            .join(p))
    }
}

fn dir_basename(dir: &Path) -> String {
    dir.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "leo4-app".into())
}

fn resolve_leo4_root(p: Option<PathBuf>) -> String {
    p.map(|x| x.display().to_string())
        .unwrap_or_else(|| "../leo4".to_string())
}

fn read_cargo_pkg_name(p: &Path) -> Option<String> {
    let s = fs::read_to_string(p).ok()?;
    for line in s.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("name") {
            // `name = "foo"` or `name="foo"`.
            let rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '=');
            let rest = rest.trim();
            let rest = rest.trim_matches('"');
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

fn camel_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut cap_next = true;
    for ch in s.chars() {
        if ch == '_' || ch == '-' || ch == ' ' {
            cap_next = true;
            continue;
        }
        if cap_next {
            out.extend(ch.to_uppercase());
            cap_next = false;
        } else {
            out.push(ch);
        }
    }
    if out.is_empty() {
        "App".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_case_basic() {
        assert_eq!(camel_case("foo-bar-baz"), "FooBarBaz");
        assert_eq!(camel_case("solver_lib"), "SolverLib");
        assert_eq!(camel_case("App"), "App");
        assert_eq!(camel_case(""), "App");
    }

    #[test]
    fn parse_impl_kind_accepts_canonical_names() {
        assert_eq!(parse_impl_kind("mslean4").unwrap(), ImplKind::Mslean4);
        assert_eq!(parse_impl_kind("rust-native").unwrap(), ImplKind::RustNative);
        assert_eq!(
            parse_impl_kind("rust-transpile").unwrap(),
            ImplKind::RustTranspile
        );
    }

    #[test]
    fn parse_impl_kind_accepts_rust_alias() {
        assert_eq!(parse_impl_kind("rust").unwrap(), ImplKind::RustNative);
    }

    #[test]
    fn parse_impl_kind_rejects_unknown() {
        let err = parse_impl_kind("oxilean").unwrap_err();
        assert!(err.contains("mslean4"), "{err}");
        assert!(err.contains("rust-native"), "{err}");
        assert!(err.contains("rust-transpile"), "{err}");
    }

    #[test]
    fn check_impl_supported_passes_mslean4() {
        assert!(check_impl_supported(&ImplKind::Mslean4).is_ok());
    }

    #[test]
    fn check_impl_supported_rejects_rust_native_with_pointer_to_spec() {
        let err = check_impl_supported(&ImplKind::RustNative).unwrap_err();
        assert!(err.contains("rust-native"), "{err}");
        assert!(err.contains("rust-native-lean.md"), "{err}");
    }

    #[test]
    fn check_impl_supported_rejects_rust_transpile_with_pointer_to_spec_section_9() {
        let err = check_impl_supported(&ImplKind::RustTranspile).unwrap_err();
        assert!(err.contains("rust-transpile"), "{err}");
        assert!(err.contains("§9"), "{err}");
        assert!(err.contains("leo4-oxilean-build"), "{err}");
    }

    #[test]
    fn impl_kind_marker_strings_unique_and_consistent() {
        let kinds = [
            ImplKind::Mslean4,
            ImplKind::RustNative,
            ImplKind::RustTranspile,
        ];
        let markers: Vec<_> = kinds.iter().map(ImplKind::marker_str).collect();
        assert_eq!(markers, ["mslean4", "rust-native", "rust-transpile"]);
        for m in &markers {
            assert_eq!(
                parse_impl_kind(m).unwrap().marker_str(),
                *m,
                "round-trip failed for {m}"
            );
        }
    }

    #[test]
    fn impl_marker_round_trip() {
        let dir = tempdir();
        write_impl_marker(&dir, &ImplKind::Mslean4).unwrap();
        assert_eq!(read_impl_marker(&dir), Some(ImplKind::Mslean4));
        // Overwrite with the other value.
        write_impl_marker(&dir, &ImplKind::RustNative).unwrap();
        assert_eq!(read_impl_marker(&dir), Some(ImplKind::RustNative));
    }

    #[test]
    fn read_impl_marker_absent_returns_none() {
        let dir = tempdir();
        assert_eq!(read_impl_marker(&dir), None);
    }

    #[test]
    fn read_cargo_pkg_name_extracts() {
        let dir = tempdir();
        let p = dir.join("Cargo.toml");
        fs::write(&p, r#"[package]
name = "my-app"
version = "0.1.0"
"#).unwrap();
        assert_eq!(read_cargo_pkg_name(&p).as_deref(), Some("my-app"));
    }

    #[test]
    fn detect_direction_picks_reverse_when_cdylib() {
        let dir = tempdir();
        let p = dir.join("Cargo.toml");
        fs::write(&p, r#"[package]
name = "x"
[lib]
crate-type = ["cdylib"]
"#).unwrap();
        assert!(matches!(detect_direction(&p), Direction::Reverse));
    }

    #[test]
    fn detect_direction_defaults_forward() {
        let dir = tempdir();
        let p = dir.join("Cargo.toml");
        fs::write(&p, r#"[package]
name = "x"
"#).unwrap();
        assert!(matches!(detect_direction(&p), Direction::Forward));
    }

    #[test]
    fn find_cdylib_picks_linux_so() {
        let dir = tempdir();
        let so = dir.join("libfoo.so");
        fs::write(&so, b"\x7fELF").unwrap();
        assert_eq!(find_cdylib(&dir, "foo").unwrap(), so);
    }

    #[test]
    fn find_cdylib_errors_when_missing() {
        let dir = tempdir();
        assert!(find_cdylib(&dir, "foo").is_err());
    }

    #[test]
    fn bin_name_strips_exe_on_unix() {
        let n = bin_name("leo4-rust-emit");
        #[cfg(windows)]
        assert_eq!(n, "leo4-rust-emit.exe");
        #[cfg(not(windows))]
        assert_eq!(n, "leo4-rust-emit");
    }

    #[test]
    fn lakefile_reverse_requires_leo4rust_and_uses_submodules() {
        let s = lakefile_reverse("my-app", "MyApp", "../leo4");
        assert!(s.contains("require Leo4Rust"), "lakefile must require Leo4Rust");
        assert!(s.contains(".submodules `MyApp"), "lib glob must be .submodules");
        assert!(s.contains("lean_exe my_app"), "lean_exe name = crate_name");
    }

    #[test]
    fn extend_cargo_toml_is_idempotent() {
        let dir = tempdir();
        fs::write(dir.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let block = "\n# ─── leo4 integration (test) ────\n[dependencies]\nleo4 = \"*\"\n";
        extend_cargo_toml(&dir, block).unwrap();
        let after1 = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        extend_cargo_toml(&dir, block).unwrap();
        let after2 = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert_eq!(after1, after2, "second run should be a no-op");
    }

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "leo4-cli-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&base).unwrap();
        base
    }

    // ─── Post-OX6 chunk 2: leo4 create writes leo4.toml ───

    #[test]
    fn write_leo4_toml_emits_valid_config_with_default_kind() {
        let dir = tempdir();
        write_leo4_toml(&dir, "mslean4").unwrap();
        let raw = fs::read_to_string(dir.join("leo4.toml")).unwrap();
        assert!(raw.contains("[[impl]]"), "{raw}");
        assert!(raw.contains("kind = \"mslean4\""), "{raw}");
        // Round-trip via the config parser to confirm
        // the emitted TOML is valid + validates clean.
        let cfg = crate::config::Leo4Config::parse_str(&raw)
            .expect("emitted leo4.toml must reparse");
        assert_eq!(cfg.impls.len(), 1);
        assert_eq!(cfg.impls[0].kind, "mslean4");
    }

    #[test]
    fn run_create_forward_writes_leo4_toml_not_leo4_impl_marker() {
        let dir = tempdir().join("scaffold-fwd");
        let leo4_root = tempdir();
        // Synthesize a fake leo4 root just so the
        // scaffold writers' path-substitution succeeds.
        run_create(
            Direction::Forward,
            dir.clone(),
            Some("scaffold-fwd".into()),
            Some(leo4_root),
        )
        .expect("run_create must succeed");
        assert!(dir.join("leo4.toml").exists(), "leo4.toml must be present");
        assert!(
            !dir.join(".leo4-impl").exists(),
            "post-OX6 create must NOT write the legacy .leo4-impl marker"
        );
        let raw = fs::read_to_string(dir.join("leo4.toml")).unwrap();
        assert!(raw.contains("kind = \"mslean4\""));
    }

    #[test]
    fn run_create_reverse_writes_leo4_toml() {
        let dir = tempdir().join("scaffold-rev");
        let leo4_root = tempdir();
        run_create(
            Direction::Reverse,
            dir.clone(),
            Some("scaffold-rev".into()),
            Some(leo4_root),
        )
        .expect("run_create must succeed");
        assert!(dir.join("leo4.toml").exists());
    }
}
