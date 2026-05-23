//! `leo4` — top-level CLI.
//!
//! Two scaffolding subcommands with distinct semantics:
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
//! Both flavours support `--forward` (default) and `--reverse`
//! direction. Forward = Lean exports + Rust caller (`@[leo4_export]`
//! / `leo4::import!`). Reverse = Rust cdylib + Lean caller
//! (`#[leo4::export]` / generated Lean wrapper).
//!
//! The CLI does not invoke `cargo` or `lake` — it just edits /
//! writes files. The README in each scaffold walks the user
//! through the build commands.

#![allow(clippy::missing_errors_doc)]

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use clap::{Parser, Subcommand};

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
    },
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum Direction {
    /// `@[leo4_export]` on Lean side, `leo4::import!` on Rust side.
    Forward,
    /// `#[leo4::export]` on Rust side, generated Lean wrapper.
    Reverse,
}

fn main() {
    let cli = Cli::parse();
    let res = match cli.cmd {
        Cmd::Create { direction, dir, name, leo4_root } => {
            run_create(direction, dir, name, leo4_root)
        }
        Cmd::Init { direction, dir, leo4_root } => {
            run_init(direction, dir, leo4_root)
        }
    };
    if let Err(e) = res {
        eprintln!("leo4: {e}");
        std::process::exit(1);
    }
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
    println!("leo4 create: {project_name} ({direction:?}) → {dir:?}");
    println!("  next: cat {}/README.md", dir.display());
    Ok(())
}

// ─── `leo4 init` (existing crate) ───────────────────────────────────

fn run_init(
    direction: Direction,
    dir: Option<PathBuf>,
    leo4_root: Option<PathBuf>,
) -> Result<(), String> {
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

    let pkg_name = read_cargo_pkg_name(&cargo_toml)
        .unwrap_or_else(|| dir_basename(&dir));
    let leo4_root_str = resolve_leo4_root(leo4_root);

    match direction {
        Direction::Forward => integrate_forward(&dir, &pkg_name, &leo4_root_str)?,
        Direction::Reverse => integrate_reverse(&dir, &pkg_name, &leo4_root_str)?,
    }
    println!("leo4 init: integrated {direction:?} scaffold into {dir:?}");
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
    Ok(())
}

// ─── full scaffold (`create`) ────────────────────────────────────────

fn scaffold_forward_full(dir: &Path, name: &str, leo4_root: &str) -> Result<(), String> {
    write_required(dir, "Cargo.toml", &cargo_toml_forward(name, leo4_root))?;
    write_required(dir, "build.rs", BUILD_RS_FORWARD)?;
    write_required(dir, "src/main.rs", &main_rs_forward(name))?;
    write_required(dir, "lean/lakefile.lean", &lakefile_forward(name, leo4_root))?;
    write_required(dir, "lean/lean-toolchain", "leanprover/lean4:v4.29.1\n")?;
    write_required(dir, "lean/Sample.lean", SAMPLE_LEAN_FORWARD)?;
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

require Leo4 from "{leo4_root}/lake/Leo4"

@[default_target]
lean_lib {iface} where
  globs := #[`{iface}]

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

## Build + run

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

## Build + run

```sh
# 1. Build the cdylib + leo4 helper binaries.
cargo build --release
(cd {leo4_root} && cargo build --release -p leo4-rust-bridge \
                                        -p leo4-rust-worker \
                                        -p leo4-rust-emit)

# 2. Emit IDL / handshake / Lean wrapper.
CDYLIB=$(realpath target/release/lib{crate_name}.so)
mkdir -p lean/{iface}
{leo4_root}/target/release/leo4-rust-emit \
  --cdylib $CDYLIB --out-dir lean/.leo4-emit --emit-lean \
  --lean-module {iface}.Rust
mv lean/.leo4-emit/{crate_name}.leo4-rust-imports.lean lean/{iface}/Rust.lean

# 3. Lean-side glue shim.
leanc -c -std=c2x {leo4_root}/shim/leo4_rust_bridge_lean.c \
  -o lean/.leo4-emit/glue.o

# 4. Lake build + manual leanc -o link.
cd lean && lake build
leanc .lake/build/lib/Main.olean.o \
      .lake/build/lib/{iface}/Rust.olean.o \
      {leo4_root}/lake/Leo4/.lake/build/lib/Leo4.olean.o \
      .leo4-emit/glue.o \
      {leo4_root}/target/release/libleo4_rust_bridge.a \
      -o {crate_name}

# 5. Run with env matrix.
LEO4_RUST_CDYLIB=$CDYLIB \
LEO4_RUST_WORKER_BIN={leo4_root}/target/release/leo4-rust-worker \
LEO4_RUST_HANDSHAKE_PKG={crate_name} \
LEO4_RUST_HANDSHAKE_IFACE={iface} \
  ./{crate_name}
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
}
