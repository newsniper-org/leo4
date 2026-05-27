//! `leo4-oxilean-build` — OX1 + OX8 CLI entry for the rust-
//! transpile path. Drives two directions:
//!
//! - **Forward** (`--mode forward`, default; OX1): Lean source
//!   → Cargo crate transpile. Reads a manifest with one or
//!   more `source=<lean_file>` lines and emits a plain Rust
//!   crate at `out_dir/`.
//!
//! - **Reverse** (`--mode reverse`; OX8.2b): Rust cdylib →
//!   `lean/<Iface>/Rust.lean` wrapper emit. Loads the cdylib
//!   via `libloading`, calls `leo4_rust_describe_exports` to
//!   walk the `EXPORTS` slice, and renders one
//!   `@[extern "<mangled>"] opaque …` declaration per
//!   `#[leo4::export]` Rust function.
//!
//! Lake plugin / leo4-rust-emit / a user's `build.rs` can
//! spawn this as a subprocess for either direction.
//!
//! ## Usage
//!
//! ```text
//! # Forward (Lean source → Rust crate; OX1)
//! leo4-oxilean-build --manifest <path>     # read manifest from a file
//! leo4-oxilean-build --manifest -          # read manifest from stdin
//!
//! # Reverse (cdylib → Lean wrapper; OX8.2b)
//! leo4-oxilean-build --mode reverse \
//!     --cdylib <path> --iface <Name> --out <path.lean>
//! # …or carry the same fields via a manifest:
//! leo4-oxilean-build --manifest <path>
//!     # manifest carries `mode=reverse`, `cdylib=…`, `iface=…`, `lean_out=…`
//!
//! leo4-oxilean-build --help
//! ```
//!
//! ## Manifest format
//!
//! Line-oriented, `key=value`. `#` and blank lines ignored.
//! Mode-selection field (added OX8.2b):
//!
//! - `mode=forward` (default; OX1 — same behaviour as
//!   pre-OX8.2b releases).
//! - `mode=reverse` — OX8.2b reverse direction. Additional
//!   required fields: `cdylib=<path>`, `iface=<Name>`,
//!   `lean_out=<path>`. `crate_name` / `out_dir` / `source` /
//!   `bind` lines are ignored in reverse mode.
//!
//! Two source-line forms are accepted in forward mode:
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
//! # Forward
//! crate_name=my_transpiled
//! out_dir=/tmp/transpiled
//! source=lean/Foo.lean abc12345_a
//! source=lean/Pkg.lean
//! bind=addOne=def67890_a
//! bind=square=fed09876_b
//!
//! # Reverse
//! mode=reverse
//! cdylib=target/release/libmycrate.so
//! iface=MyCrate
//! lean_out=lean/MyCrate/Rust.lean
//! ```
//!
//! The lake plugin / leo4-rust-emit precomputes each mangled
//! per `SPEC/mangling.md` §3.
//!
//! Exit codes:
//!
//! - `0`: success.
//! - `1`: transpile / reverse emit failure (parse / elab /
//!   wrapper-synth error in forward mode; cdylib load /
//!   symbol resolve / render error in reverse mode). No
//!   output written.
//! - `2`: usage error (bad args, missing manifest field,
//!   IO error reading manifest / source / out_dir).

use leo4_abi::rust_exports::ExportEntry;
use leo4_oxilean_build::{
    leo4_env_bootstrap::bootstrap_env,
    pure_emit::transpile_sources_to_pure_crate,
    reverse_emit::{render_reverse_wrapper, ExportEntryView},
};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Mode selector (added OX8.2b). Forward is the OX1 default
/// behaviour; reverse is the OX8 cdylib → Lean wrapper emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Forward,
    Reverse,
}

impl Mode {
    fn parse(s: &str) -> Result<Self, String> {
        match s {
            "forward" => Ok(Self::Forward),
            "reverse" => Ok(Self::Reverse),
            other => Err(format!(
                "unknown mode `{other}`; expected `forward` or `reverse`"
            )),
        }
    }
}

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
    mode: Mode,
    // ─── Forward-mode fields ─────────────────────────────
    crate_name: Option<String>,
    /// Required for canonical (option B) emit. None
    /// allowed in pure (option A) mode where the
    /// transpiled crate has no `LeanProc` impl that
    /// would carry a schema_hash.
    schema_hash: Option<String>,
    /// Required for canonical emit (the emitted
    /// Cargo.toml gains a `leo4-abi = { … }` line).
    /// Pure mode emits a Cargo.toml with no deps so
    /// this field is unused.
    leo4_abi_dep: Option<String>,
    out_dir: Option<PathBuf>,
    sources: Vec<SourceEntry>,
    // ─── Reverse-mode fields (OX8.2b) ────────────────────
    /// Path to the user cdylib produced by `cargo build`.
    /// Required in reverse mode. Resolved relative to the
    /// process cwd at run time.
    cdylib: Option<PathBuf>,
    /// Lean interface namespace for the generated wrapper —
    /// becomes `namespace <iface>` at the top of the emitted
    /// `.lean` file. Required in reverse mode.
    iface: Option<String>,
    /// Output path for the generated `lean/<Iface>/Rust.lean`
    /// file. Required in reverse mode.
    lean_out: Option<PathBuf>,
}

fn die(msg: impl AsRef<str>) -> ! {
    eprintln!("leo4-oxilean-build: error: {}", msg.as_ref());
    std::process::exit(2);
}

fn print_help() {
    print!(
        "leo4-oxilean-build — leo4 rust-transpile path build driver (OX1 + OX8.2b)\n\
         \n\
         Usage:\n\
         \x20 leo4-oxilean-build --manifest <path>\n\
         \x20 leo4-oxilean-build --manifest -\n\
         \x20 leo4-oxilean-build --mode reverse --cdylib <path> \\\n\
         \x20                    --iface <Name> --out <path.lean>\n\
         \x20 leo4-oxilean-build --help\n\
         \n\
         Modes:\n\
         \x20 forward (default; OX1) — Lean source → Rust crate.\n\
         \x20   Output: a plain Rust crate at `out_dir/` —\n\
         \x20   `Cargo.toml` + `src/lib.rs` with `pub fn`\n\
         \x20   signatures, no canonical-ABI wrapper, no\n\
         \x20   leo4-abi dep, no schema_hash.\n\
         \x20 reverse (OX8.2b) — Rust cdylib → Lean wrapper.\n\
         \x20   Output: one `lean/<Iface>/Rust.lean` file\n\
         \x20   containing `@[extern \"<mangled>\"] opaque …`\n\
         \x20   declarations, one per `#[leo4::export]` fn\n\
         \x20   discovered via the cdylib's\n\
         \x20   `leo4_rust_describe_exports` C entry.\n\
         \n\
         Manifest fields (line-oriented `key=value`):\n\
         Forward mode:\n\
         \x20 crate_name=<str>          required\n\
         \x20 out_dir=<path>            required\n\
         \x20 source=<lean>             one per Lean source file\n\
         \n\
         Reverse mode (set `mode=reverse`):\n\
         \x20 mode=reverse              required to trigger reverse\n\
         \x20 cdylib=<path>             path to the cdylib (required)\n\
         \x20 iface=<Name>              Lean namespace (required)\n\
         \x20 lean_out=<path>           output `.lean` path (required)\n\
         \n\
         CLI args override manifest fields for reverse mode:\n\
         \x20 --mode reverse\n\
         \x20 --cdylib <path>\n\
         \x20 --iface <Name>\n\
         \x20 --out <path.lean>\n\
         \n\
         Legacy manifest fields (silently ignored — carried\n\
         by older lake-plugin-emitted forward manifests):\n\
         \x20 schema_hash=<13-char>\n\
         \x20 leo4_abi_dep=<toml-frag>\n\
         \x20 source=<lean> <mangled>   (mangled suffix ignored)\n\
         \x20 bind=<name>=<mangled>\n\
         \n\
         Exit codes: 0 = success, 1 = transpile / emit failure, 2 = usage / IO error\n"
    );
}

fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let mut mode: Option<Mode> = None;
    let mut crate_name: Option<String> = None;
    let mut schema_hash: Option<String> = None;
    let mut leo4_abi_dep: Option<String> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut sources: Vec<SourceEntry> = Vec::new();
    let mut cdylib: Option<PathBuf> = None;
    let mut iface: Option<String> = None;
    let mut lean_out: Option<PathBuf> = None;

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
            "mode" => mode = Some(Mode::parse(val).map_err(|e| format!("line {}: {e}", i + 1))?),
            "crate_name" => crate_name = Some(val.to_string()),
            "schema_hash" => schema_hash = Some(val.to_string()),
            "leo4_abi_dep" => leo4_abi_dep = Some(val.to_string()),
            "out_dir" => out_dir = Some(PathBuf::from(val)),
            "cdylib" => cdylib = Some(PathBuf::from(val)),
            "iface" => iface = Some(val.to_string()),
            "lean_out" => lean_out = Some(PathBuf::from(val)),
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
        mode: mode.unwrap_or(Mode::Forward),
        crate_name,
        schema_hash,
        leo4_abi_dep,
        out_dir,
        sources,
        cdylib,
        iface,
        lean_out,
    })
}

/// Parsed CLI args. Forward mode uses only `manifest`;
/// reverse mode can supply `mode` / `cdylib` / `iface` /
/// `out` directly without a manifest (or override manifest
/// fields when both are present).
struct CliArgs {
    manifest_arg: Option<String>,
    mode: Option<Mode>,
    cdylib: Option<PathBuf>,
    iface: Option<String>,
    out: Option<PathBuf>,
}

fn parse_cli_args(args: &[String]) -> CliArgs {
    let mut out = CliArgs {
        manifest_arg: None,
        mode: None,
        cdylib: None,
        iface: None,
        out: None,
    };
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest" | "-m" => {
                out.manifest_arg = args.get(i + 1).cloned();
                i += 2;
            }
            "--mode" => {
                let v = args
                    .get(i + 1)
                    .map_or_else(
                        || die("--mode requires a value (`forward` or `reverse`)"),
                        String::as_str,
                    );
                out.mode = Some(Mode::parse(v).unwrap_or_else(|e| die(e)));
                i += 2;
            }
            "--cdylib" => {
                let v = args
                    .get(i + 1)
                    .unwrap_or_else(|| die("--cdylib requires a path"));
                out.cdylib = Some(PathBuf::from(v));
                i += 2;
            }
            "--iface" => {
                let v = args
                    .get(i + 1)
                    .unwrap_or_else(|| die("--iface requires a name"));
                out.iface = Some(v.clone());
                i += 2;
            }
            "--out" => {
                let v = args
                    .get(i + 1)
                    .unwrap_or_else(|| die("--out requires a path"));
                out.out = Some(PathBuf::from(v));
                i += 2;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => die(format!("unknown arg '{other}'; try --help")),
        }
    }
    out
}

#[allow(clippy::too_many_lines)] // documented: arg parse + manifest + transpile loop + emit
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let cli = parse_cli_args(&args);

    // Load manifest if one was provided. In reverse mode the
    // manifest is optional — all four required fields can
    // come from `--mode/--cdylib/--iface/--out` flags
    // instead.
    let mut manifest: Manifest = if let Some(arg) = cli.manifest_arg.as_deref() {
        let text = if arg == "-" {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .unwrap_or_else(|e| die(format!("reading stdin: {e}")));
            s
        } else {
            std::fs::read_to_string(arg)
                .unwrap_or_else(|e| die(format!("reading manifest `{arg}`: {e}")))
        };
        parse_manifest(&text)
            .unwrap_or_else(|e| die(format!("parsing manifest: {e}")))
    } else {
        // No manifest — reverse mode w/ direct flags only.
        Manifest {
            mode: Mode::Forward,
            crate_name: None,
            schema_hash: None,
            leo4_abi_dep: None,
            out_dir: None,
            sources: Vec::new(),
            cdylib: None,
            iface: None,
            lean_out: None,
        }
    };

    // CLI args override manifest fields when both supplied.
    if let Some(m) = cli.mode {
        manifest.mode = m;
    }
    if let Some(c) = cli.cdylib {
        manifest.cdylib = Some(c);
    }
    if let Some(i) = cli.iface {
        manifest.iface = Some(i);
    }
    if let Some(o) = cli.out {
        manifest.lean_out = Some(o);
    }

    // Without --manifest AND without --mode reverse, we
    // can't proceed — forward mode requires a manifest.
    if cli.manifest_arg.is_none() && manifest.mode == Mode::Forward {
        die("--manifest <path|-> required for forward mode; try --help");
    }

    match manifest.mode {
        Mode::Forward => run_forward(&manifest),
        Mode::Reverse => run_reverse(&manifest),
    }
}

fn run_forward(manifest: &Manifest) -> ExitCode {
    let crate_name = manifest
        .crate_name
        .as_deref()
        .unwrap_or_else(|| die("forward mode: missing required field `crate_name`"));
    let out_dir = manifest
        .out_dir
        .as_ref()
        .unwrap_or_else(|| die("forward mode: missing required field `out_dir`"));

    // Legacy canonical-mode fields silently ignored
    // — kept on Manifest for backward compat with
    // older lake-plugin-emitted manifests. Surface a
    // one-line note when they appear so the user
    // knows they had no effect.
    if manifest.schema_hash.is_some() || manifest.leo4_abi_dep.is_some() {
        eprintln!(
            "leo4-oxilean-build: note — manifest has canonical-mode fields \
             (schema_hash / leo4_abi_dep); the CLI emits pure-Rust only \
             since 2026-05-25, so they are ignored."
        );
    }

    // OX5-oxi: populate env with OxiLean prelude + leo4
    // boundary primitives before elab so the transpile
    // pipeline doesn't choke on `NameNotFound("UInt64")`
    // etc. Zero lake/lean overhead — both layers run
    // in-process against the oxilean-kernel cargo dep.
    let env = bootstrap_env().unwrap_or_else(|e| {
        die(format!("leo4-oxilean-build: env bootstrap failed: {e}"))
    });

    // Read all source files into memory + collect into
    // a single multi-file PureCrate. Legacy single-decl
    // form's `<mangled>` suffix and multi-decl `bind=`
    // lines are silently ignored — pure mode has no
    // mangling.
    let mut source_strings: Vec<String> = Vec::with_capacity(manifest.sources.len());
    for entry in &manifest.sources {
        let src = std::fs::read_to_string(&entry.path).unwrap_or_else(|e| {
            die(format!("reading source `{}`: {e}", entry.path.display()))
        });
        source_strings.push(src);
    }
    let source_refs: Vec<&str> = source_strings.iter().map(String::as_str).collect();

    let pure_crate = match transpile_sources_to_pure_crate(
        &env,
        crate_name,
        &source_refs,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "leo4-oxilean-build: ERROR — pure transpile failed: 0x{:08x} {}",
                e.code, e.message
            );
            return ExitCode::from(1);
        }
    };

    pure_crate.write_to_dir(out_dir).unwrap_or_else(|e| {
        die(format!(
            "writing crate to `{}`: {} {}",
            out_dir.display(),
            e.code,
            e.message
        ))
    });

    eprintln!(
        "leo4-oxilean-build: wrote pure crate `{}` ({} fns) to {}",
        crate_name,
        pure_crate.fns.len(),
        out_dir.display()
    );
    ExitCode::SUCCESS
}

fn run_reverse(manifest: &Manifest) -> ExitCode {
    let cdylib = manifest
        .cdylib
        .as_ref()
        .unwrap_or_else(|| die("reverse mode: missing required field `cdylib` (use --cdylib or `cdylib=<path>` in manifest)"));
    let iface = manifest
        .iface
        .as_deref()
        .unwrap_or_else(|| die("reverse mode: missing required field `iface` (use --iface or `iface=<Name>` in manifest)"));
    let lean_out = manifest
        .lean_out
        .as_ref()
        .unwrap_or_else(|| die("reverse mode: missing required field `lean_out` (use --out or `lean_out=<path>` in manifest)"));

    // Bail before opening the cdylib if the path doesn't
    // exist — surfaces the user error with a sharper
    // message than libloading's "cannot open shared
    // object".
    if !cdylib.exists() {
        eprintln!(
            "leo4-oxilean-build: ERROR — cdylib `{}` does not exist. \
             Did you forget `cargo build --release`?",
            cdylib.display()
        );
        return ExitCode::from(1);
    }

    // SAFETY: the user is responsible for pointing
    // `--cdylib` at a file produced from a leo4-abi-
    // compatible workspace. The repr-C `ExportEntry`
    // layout + the `abi_version=1` field guard against
    // stale builds; mismatches surface as garbled
    // metadata, never as memory unsafety in the read
    // step (which only dereferences once, into owned
    // `String`s).
    let entries = match unsafe { load_exports(cdylib) } {
        Ok(e) => e,
        Err(e) => {
            eprintln!("leo4-oxilean-build: ERROR — reverse cdylib load failed: {e}");
            return ExitCode::from(1);
        }
    };

    let lean_src = match render_reverse_wrapper(iface, &entries) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("leo4-oxilean-build: ERROR — reverse wrapper render failed: {e}");
            return ExitCode::from(1);
        }
    };

    if let Some(parent) = lean_out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).unwrap_or_else(|e| {
            die(format!(
                "creating parent dir `{}`: {e}",
                parent.display()
            ))
        });
    }
    std::fs::write(lean_out, lean_src.as_bytes()).unwrap_or_else(|e| {
        die(format!(
            "writing reverse wrapper to `{}`: {e}",
            lean_out.display()
        ))
    });

    eprintln!(
        "leo4-oxilean-build: wrote reverse wrapper for `{}` ({} export(s)) to {}",
        iface,
        entries.len(),
        lean_out.display()
    );
    ExitCode::SUCCESS
}

// ─── cdylib introspection (OX8.2b) ───────────────────────────────────
//
// Mirrors `crates/leo4-rust-emit/src/main.rs::load_exports` —
// same C entry point (`leo4_rust_describe_exports`), same
// `repr(C) ExportEntry` layout. The only difference: we
// project into this crate's `ExportEntryView` (declared in
// the lib's `reverse_emit` module) rather than a private
// `EntryView`, so `render_reverse_wrapper` can consume the
// result directly.

/// Load the cdylib's `EXPORTS` table via `libloading` and
/// copy each entry into an owned [`ExportEntryView`].
///
/// # Safety
///
/// Caller must pass a path to a leo4-abi-compatible cdylib
/// (one built with the `rust-exports` feature on `leo4-abi`,
/// transitively via the `leo4` facade's `rust-exports`
/// feature). Layout mismatch surfaces as garbled metadata —
/// the `abi_version` field on each entry is the user's
/// guard against version skew.
unsafe fn load_exports(cdylib: &Path) -> Result<Vec<ExportEntryView>, String> {
    type Describe = unsafe extern "C" fn(*mut *const ExportEntry, *mut usize) -> i32;

    // SAFETY: opening a shared library is inherently unsafe
    // (we trust the file). The cdylib path was supplied by
    // the user via --cdylib / manifest.
    let lib = unsafe {
        libloading::Library::new(cdylib)
            .map_err(|e| format!("dlopen `{}`: {e}", cdylib.display()))?
    };

    // SAFETY: the symbol's type must match the C signature
    // emitted by leo4-abi's `leo4_rust_describe_exports`.
    let sym: libloading::Symbol<'_, Describe> = unsafe {
        lib.get(b"leo4_rust_describe_exports\0").map_err(|e| {
            format!(
                "cdylib does not export `leo4_rust_describe_exports` ({e}). \
                 Did you forget to enable the `rust-exports` feature on `leo4`?"
            )
        })?
    };

    let mut ptr: *const ExportEntry = std::ptr::null();
    let mut len: usize = 0;
    // SAFETY: outparams are stack locals — valid for the
    // duration of the call. Signature matches.
    let rc = unsafe { sym(&raw mut ptr, &raw mut len) };
    if rc != 0 {
        return Err(format!("leo4_rust_describe_exports returned {rc}"));
    }
    if len > 0 && ptr.is_null() {
        return Err("describe returned non-zero length with null pointer".into());
    }

    // SAFETY: `ptr`/`len` were produced inside the cdylib
    // from `&EXPORTS.as_ptr()` / `.len()`. The data
    // remains valid until `lib` is dropped — we copy each
    // string out before that happens.
    let slice: &[ExportEntry] = unsafe { std::slice::from_raw_parts(ptr, len) };
    let mut out = Vec::with_capacity(len);
    for e in slice {
        out.push(ExportEntryView {
            logical_name: e.logical_name.to_owned(),
            mangled: e.mangled.to_owned(),
            param_types: e.param_types.iter().map(|s| (*s).to_owned()).collect(),
            ret_type: e.ret_type.to_owned(),
            isolated: e.isolated,
            abi_version: e.abi_version,
        });
    }

    // Drop the lib *after* we've copied everything out.
    drop(lib);
    Ok(out)
}
