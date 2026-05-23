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
//! Two source-line forms are accepted:
//!
//! - **Single-decl form** (legacy):
//!   `source=<lean_file_path> <mangled_name>` — file MUST
//!   contain exactly one top-level decl; the mangled is
//!   bound to that decl.
//!
//! - **Multi-decl form** (OX1 step b):
//!   `source=<lean_file_path>` (no mangled on the source line)
//!   followed by zero or more `bind=<decl_name>=<mangled>`
//!   lines until the next `source=`. The file is parsed
//!   multi-decl; each `@[leo4_export]` `def` looks up its
//!   mangled by name in the binds preceding it. Type-only
//!   decls (`structure` / `inductive`) ignore the binds.
//!
//! Examples:
//!
//! ```text
//! crate_name=my_transpiled
//! schema_hash=0123456789abc
//! leo4_abi_dep={ path = "../leo4-abi" }
//! out_dir=/tmp/transpiled
//!
//! # single-decl form
//! source=lean/Foo.lean abc12345_a
//!
//! # multi-decl form
//! source=lean/Pkg.lean
//! bind=addOne=def67890_a
//! bind=square=fed09876_b
//! ```
//!
//! The lake plugin / leo4-rust-emit precomputes each mangled
//! per `SPEC/mangling.md` §3.
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
    emit_crate, transpile_source_to_unit, transpile_source_to_units,
    Leo4ExportRegistry,
};
use oxilean_kernel::env::Environment;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

/// One source-file entry in the manifest. Single-decl form
/// pairs `(path, Some(mangled))`; multi-decl form pairs
/// `(path, None)` + a non-empty `binds` map.
struct SourceEntry {
    path: PathBuf,
    /// `Some(mangled)` → single-decl source line. The whole
    /// file is treated as one decl; the bind map is ignored.
    /// `None` → multi-decl source line; the bind map's
    /// `decl_name → mangled` entries supply the dispatch keys.
    single_mangled: Option<String>,
    binds: HashMap<String, String>,
}

struct Manifest {
    crate_name: String,
    schema_hash: String,
    leo4_abi_dep: String,
    out_dir: PathBuf,
    sources: Vec<SourceEntry>,
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
         \x20 source=<lean> <mangled>   single-decl source (file = 1 decl)\n\
         \x20 source=<lean>             multi-decl source (binds follow)\n\
         \x20 bind=<name>=<mangled>     per-decl mangled (multi-decl form only)\n\
         \n\
         Exit codes: 0 = success, 1 = transpile failure, 2 = usage / IO error\n"
    );
}

fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let mut crate_name: Option<String> = None;
    let mut schema_hash: Option<String> = None;
    let mut leo4_abi_dep: Option<String> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut sources: Vec<SourceEntry> = Vec::new();

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
                    .ok_or_else(|| format!("line {}: `source` missing path", i + 1))?
                    .trim();
                let single_mangled = parts.next().map(|s| s.trim().to_string());
                sources.push(SourceEntry {
                    path: PathBuf::from(path),
                    single_mangled,
                    binds: HashMap::new(),
                });
            }
            "bind" => {
                let (decl_name, mangled) = val.split_once('=').ok_or_else(|| {
                    format!(
                        "line {}: `bind` value must be `<decl_name>=<mangled>`",
                        i + 1
                    )
                })?;
                let last = sources.last_mut().ok_or_else(|| {
                    format!(
                        "line {}: `bind` appears before any `source=` line",
                        i + 1
                    )
                })?;
                if last.single_mangled.is_some() {
                    return Err(format!(
                        "line {}: `bind` follows a single-decl `source=<path> <mangled>` \
                         line — drop the mangled from the source line to use multi-decl form",
                        i + 1
                    ));
                }
                last.binds
                    .insert(decl_name.trim().to_string(), mangled.trim().to_string());
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

#[allow(clippy::too_many_lines)] // documented: arg parse + manifest + transpile loop + emit
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

    for entry in &manifest.sources {
        let src = std::fs::read_to_string(&entry.path).unwrap_or_else(|e| {
            die(format!("reading source `{}`: {e}", entry.path.display()))
        });
        if let Some(single_mangled) = &entry.single_mangled {
            // Single-decl form.
            match transpile_source_to_unit(&env, &mut registry, &src, single_mangled) {
                Ok(Some(unit)) => units.push(unit),
                Ok(None) => {
                    skipped += 1;
                    eprintln!(
                        "leo4-oxilean-build: skip (no @[leo4_export]): {}",
                        entry.path.display()
                    );
                }
                Err(e) => {
                    errors += 1;
                    eprintln!(
                        "leo4-oxilean-build: ERROR `{}`: 0x{:08x} {}",
                        entry.path.display(),
                        e.code,
                        e.message
                    );
                }
            }
        } else {
            // Multi-decl form.
            match transpile_source_to_units(&env, &mut registry, &src, &entry.binds) {
                Ok(us) => {
                    if us.is_empty() {
                        skipped += 1;
                        eprintln!(
                            "leo4-oxilean-build: skip (no @[leo4_export]s): {}",
                            entry.path.display()
                        );
                    }
                    units.extend(us);
                }
                Err(e) => {
                    errors += 1;
                    eprintln!(
                        "leo4-oxilean-build: ERROR `{}`: 0x{:08x} {}",
                        entry.path.display(),
                        e.code,
                        e.message
                    );
                }
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
