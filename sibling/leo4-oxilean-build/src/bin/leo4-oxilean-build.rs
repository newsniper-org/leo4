//! `leo4-oxilean-build` — OX1 CLI entry to drive the
//! Lean-source → Cargo-crate transpile pipeline from outside
//! Rust code. Lake plugin / leo4-rust-emit / a user's
//! `build.rs` can spawn this as a subprocess; it reads a
//! line-oriented manifest, transpiles every source listed,
//! and writes the emitted crate to `out_dir`.
//!
//! ## Usage
//!
//! ```text
//! leo4-oxilean-build --manifest <path>     # read manifest from a file
//! leo4-oxilean-build --manifest -          # read manifest from stdin
//! leo4-oxilean-build --help
//! ```
//!
//! ## Manifest format
//!
//! Line-oriented, `key=value`. `#` and blank lines ignored.
//! Multiple `source=` lines accumulate.
//!
//! ```text
//! crate_name=my_transpiled
//! schema_hash=0123456789abc
//! leo4_abi_dep={ path = "../leo4-abi" }
//! out_dir=/tmp/transpiled
//! source=lean/Foo.lean abc12345_a
//! source=lean/Bar.lean def67890_a
//! ```
//!
//! Each `source` line is `<lean_file_path> <mangled_name>` —
//! the caller (lake plugin / leo4-rust-emit) precomputes the
//! mangled name per `SPEC/mangling.md` §3.
//!
//! Exit codes:
//!
//! - `0`: success — all sources transpiled (or skipped because
//!   they weren't tagged `@[leo4_export]`); crate written.
//! - `1`: at least one source failed transpile (parse / elab /
//!   wrapper-synth error); no crate emitted.
//! - `2`: usage error (bad args, missing manifest field,
//!   IO error reading manifest / source / out_dir).

use leo4_oxilean_build::{
    emit_crate, transpile_source_to_unit, Leo4ExportRegistry,
};
use oxilean_kernel::env::Environment;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

struct Manifest {
    crate_name: String,
    schema_hash: String,
    leo4_abi_dep: String,
    out_dir: PathBuf,
    sources: Vec<(PathBuf, String)>,
}

fn die(msg: impl AsRef<str>) -> ! {
    eprintln!("leo4-oxilean-build: error: {}", msg.as_ref());
    std::process::exit(2);
}

fn print_help() {
    print!(
        "leo4-oxilean-build — Lean source → Cargo crate transpiler (OX1 CLI)\n\
         \n\
         Usage:\n\
         \x20 leo4-oxilean-build --manifest <path>\n\
         \x20 leo4-oxilean-build --manifest -\n\
         \x20 leo4-oxilean-build --help\n\
         \n\
         Manifest fields (line-oriented `key=value`):\n\
         \x20 crate_name=<str>          required\n\
         \x20 schema_hash=<13-char>     required\n\
         \x20 leo4_abi_dep=<toml-frag>  required, e.g. `{{ path = \"../leo4-abi\" }}`\n\
         \x20 out_dir=<path>            required\n\
         \x20 source=<lean> <mangled>   one or more\n\
         \n\
         Exit codes: 0 = success, 1 = transpile failure, 2 = usage / IO error\n"
    );
}

fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let mut crate_name: Option<String> = None;
    let mut schema_hash: Option<String> = None;
    let mut leo4_abi_dep: Option<String> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut sources: Vec<(PathBuf, String)> = Vec::new();

    for (i, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, val) = line
            .split_once('=')
            .ok_or_else(|| format!("line {}: missing '='", i + 1))?;
        let key = key.trim();
        let val = val.trim();
        match key {
            "crate_name" => crate_name = Some(val.to_string()),
            "schema_hash" => schema_hash = Some(val.to_string()),
            "leo4_abi_dep" => leo4_abi_dep = Some(val.to_string()),
            "out_dir" => out_dir = Some(PathBuf::from(val)),
            "source" => {
                let mut parts = val.splitn(2, char::is_whitespace);
                let path = parts
                    .next()
                    .ok_or_else(|| format!("line {}: `source` missing path", i + 1))?;
                let mangled = parts.next().ok_or_else(|| {
                    format!("line {}: `source` missing mangled name", i + 1)
                })?;
                sources.push((PathBuf::from(path.trim()), mangled.trim().to_string()));
            }
            other => return Err(format!("line {}: unknown key '{}'", i + 1, other)),
        }
    }

    Ok(Manifest {
        crate_name: crate_name.ok_or("missing required field `crate_name`")?,
        schema_hash: schema_hash.ok_or("missing required field `schema_hash`")?,
        leo4_abi_dep: leo4_abi_dep
            .ok_or("missing required field `leo4_abi_dep`")?,
        out_dir: out_dir.ok_or("missing required field `out_dir`")?,
        sources,
    })
}

fn main() -> ExitCode {
    let mut manifest_arg: Option<String> = None;
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest" | "-m" => {
                manifest_arg = args.get(i + 1).cloned();
                i += 2;
            }
            "--help" | "-h" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            other => die(format!("unknown arg '{other}'; try --help")),
        }
    }
    let manifest_arg = manifest_arg
        .unwrap_or_else(|| die("--manifest <path|-> required; try --help"));

    let text = if manifest_arg == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .unwrap_or_else(|e| die(format!("reading stdin: {e}")));
        s
    } else {
        std::fs::read_to_string(&manifest_arg)
            .unwrap_or_else(|e| die(format!("reading manifest `{manifest_arg}`: {e}")))
    };

    let manifest = parse_manifest(&text)
        .unwrap_or_else(|e| die(format!("parsing manifest: {e}")));

    let mut registry = Leo4ExportRegistry::new();
    let env = Environment::new();
    let mut units = Vec::with_capacity(manifest.sources.len());
    let mut skipped = 0usize;
    let mut errors = 0usize;

    for (path, mangled) in &manifest.sources {
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| die(format!("reading source `{}`: {e}", path.display())));
        match transpile_source_to_unit(&env, &mut registry, &src, mangled) {
            Ok(Some(unit)) => units.push(unit),
            Ok(None) => {
                skipped += 1;
                eprintln!(
                    "leo4-oxilean-build: skip (no @[leo4_export]): {}",
                    path.display()
                );
            }
            Err(e) => {
                errors += 1;
                eprintln!(
                    "leo4-oxilean-build: ERROR `{}`: 0x{:08x} {}",
                    path.display(),
                    e.code,
                    e.message
                );
            }
        }
    }

    if errors > 0 {
        eprintln!(
            "leo4-oxilean-build: {errors} source(s) failed transpile; no crate emitted"
        );
        return ExitCode::from(1);
    }

    let g = emit_crate(
        &manifest.crate_name,
        &units,
        &manifest.leo4_abi_dep,
        &manifest.schema_hash,
    );
    let written = g.write_to_dir(&manifest.out_dir).unwrap_or_else(|e| {
        die(format!("writing crate to `{}`: {e}", manifest.out_dir.display()))
    });

    eprintln!(
        "leo4-oxilean-build: wrote crate `{}` ({} units, {} skipped, {} bytes) to {}",
        manifest.crate_name,
        units.len(),
        skipped,
        written,
        manifest.out_dir.display()
    );
    ExitCode::SUCCESS
}
