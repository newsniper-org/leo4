//! `leo4-rust-emit` — Phase 9-2 emit CLI.
//!
//! Usage:
//!
//! ```text
//! leo4-rust-emit --cdylib <path> --out-dir <dir> [--pkg <name>] [--iface <name>]
//! ```
//!
//! Loads the user cdylib via `dlopen`, resolves the
//! `leo4_rust_describe_exports` extern, walks the `EXPORTS`
//! slice, computes the canonical IDL form + its FNV-1a-64
//! `schema_hash`, and writes two files into `--out-dir`:
//!
//! - `<pkg>.leo4-rust-exports.idl` — pretty canonical IDL.
//! - `<pkg>.leo4-rust-handshake` — JSON metadata (`schema_hash`,
//!   `abi_version`, `cdylib_path`, exports table).
//!
//! See `SPEC/reverse-direction.md` §7, §8 for the file
//! contracts.

#![allow(clippy::missing_errors_doc)]

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use clap::Parser;
use leo4_abi::rust_exports::{
    ExportEntry, FieldEntry, UserTypeEntry, UserTypeKind,
};

#[derive(Parser, Debug)]
#[command(name = "leo4-rust-emit", version, about = "Emit leo4 reverse-direction IDL + handshake from a built cdylib")]
struct Cli {
    /// Path to the user cdylib produced by `cargo build`. Typical
    /// location: `target/<profile>/lib<crate>.{so,dll,dylib}`.
    #[arg(long)]
    cdylib: PathBuf,

    /// Directory where the two output files will be written. The
    /// directory must already exist.
    #[arg(long)]
    out_dir: PathBuf,

    /// Package basename to use in the output filenames and inside
    /// the canonical IDL `package` directive. Defaults to the
    /// cdylib's stem (`libfoo.so` → `foo`, stripping any
    /// leading `lib`).
    #[arg(long)]
    pkg: Option<String>,

    /// Interface name to use in the canonical IDL `interface`
    /// block. Defaults to a `CamelCase` rendering of `--pkg`.
    #[arg(long)]
    iface: Option<String>,

    /// Toolchain string to record in the handshake JSON. Free
    /// text; the dispatcher does not enforce a specific format.
    #[arg(long, default_value = "rustc-stable")]
    rust_toolchain: String,

    /// Also emit `<pkg>.leo4-rust-imports.lean` — a Lean
    /// wrapper module that exposes one typed `IO` action per
    /// `#[leo4::export]`. The wrapper module imports `Leo4` and
    /// reaches the dispatcher through a single
    /// `@[extern "leo4_rust_call_lean"]` opaque (the Lean-side
    /// glue shim lands with Phase 9-6 / `leanc` link step;
    /// 9-5 produces only the Lean source).
    #[arg(long, default_value_t = false)]
    emit_lean: bool,

    /// Lean module name to use at the top of the generated
    /// wrapper file (`namespace <module>`). Defaults to
    /// `<iface>.Rust` so user code reads e.g.
    /// `open MyRustSmt.Rust` after the import.
    #[arg(long)]
    lean_module: Option<String>,
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(&cli) {
        eprintln!("leo4-rust-emit: {e}");
        std::process::exit(1);
    }
}

fn run(cli: &Cli) -> Result<(), String> {
    let cdylib = cli
        .cdylib
        .canonicalize()
        .map_err(|e| format!("canonicalize {:?}: {e}", cli.cdylib))?;
    if !cli.out_dir.is_dir() {
        return Err(format!(
            "--out-dir {:?} is not a directory",
            cli.out_dir
        ));
    }

    let pkg = cli
        .pkg
        .clone()
        .unwrap_or_else(|| derive_pkg_name(&cdylib));
    let iface = cli.iface.clone().unwrap_or_else(|| camel_case(&pkg));

    let entries = unsafe { load_exports(&cdylib)? };
    // RC.2 patch 2 (2026-05-31) — load the user-defined type
    // schema table alongside EXPORTS. Cdylibs that pre-date the
    // RC.2 macro emit (or that have zero `#[derive(LeanMarshal)]`
    // types) report a missing `leo4_rust_describe_user_types`
    // symbol; treat that as "no user types" rather than fatal,
    // so the wrapper still emits.
    let user_types = unsafe { load_user_types(&cdylib).unwrap_or_default() };

    let canonical_idl = render_canonical_idl(&pkg, &iface, &entries);
    let schema_hash = compute_schema_hash(&canonical_idl);

    let pretty_idl = render_pretty_idl(&pkg, &iface, &entries);
    let handshake_json = build_handshake_json(
        &pkg,
        &iface,
        &cdylib,
        &schema_hash,
        &cli.rust_toolchain,
        &entries,
    );

    let idl_path = cli.out_dir.join(format!("{pkg}.leo4-rust-exports.idl"));
    let handshake_path = cli.out_dir.join(format!("{pkg}.leo4-rust-handshake"));

    atomic_write(&idl_path, pretty_idl.as_bytes())?;
    atomic_write(&handshake_path, handshake_json.as_bytes())?;

    println!("leo4-rust-emit: wrote {} export(s)", entries.len());
    println!("  {}", idl_path.display());
    println!("  {}", handshake_path.display());
    println!("  schema_hash = {schema_hash}");

    if cli.emit_lean {
        let module = cli
            .lean_module
            .clone()
            .unwrap_or_else(|| format!("{iface}.Rust"));
        let lean_src = render_lean_wrapper(&module, &pkg, &iface, &schema_hash, &entries, &user_types)?;
        let lean_path =
            cli.out_dir.join(format!("{pkg}.leo4-rust-imports.lean"));
        atomic_write(&lean_path, lean_src.as_bytes())?;
        println!("  {}", lean_path.display());
    }

    Ok(())
}

// ─── cdylib introspection ────────────────────────────────────────────

/// Borrowed view of a single `ExportEntry`, copied out of the
/// cdylib's address space before the library is unloaded so the
/// caller does not have to juggle pointer lifetimes.
#[derive(Debug, Clone)]
struct EntryView {
    logical_name: String,
    mangled: String,
    param_types: Vec<String>,
    ret_type: String,
    isolated: bool,
    abi_version: u32,
}

/// SAFETY: caller must pass a path that is a leo4-exporting cdylib
/// produced by the same workspace (or any cdylib whose
/// `leo4_rust_describe_exports` symbol matches the `repr(C)`
/// `ExportEntry` layout in this build of `leo4-abi`).
unsafe fn load_exports(cdylib: &Path) -> Result<Vec<EntryView>, String> {
    type Describe = unsafe extern "C" fn(*mut *const ExportEntry, *mut usize) -> i32;

    // SAFETY: opening a shared library is inherently unsafe (we trust
    // the file). The cdylib path was passed by the user.
    let lib = unsafe {
        libloading::Library::new(cdylib)
            .map_err(|e| format!("dlopen {cdylib:?}: {e}"))?
    };

    // SAFETY: the symbol's type must match the C signature emitted by
    // leo4-abi's `leo4_rust_describe_exports`. If the user's cdylib
    // was built against a leo4-abi with a different `ExportEntry`
    // layout, we'll silently mis-read it; the abi_version field
    // guards against that downstream.
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
    // SAFETY: outparams are valid; symbol matches signature.
    let rc = unsafe { sym(&raw mut ptr, &raw mut len) };
    if rc != 0 {
        return Err(format!("leo4_rust_describe_exports returned {rc}"));
    }
    if len > 0 && ptr.is_null() {
        return Err("describe returned non-zero length with null pointer".into());
    }

    let mut out = Vec::with_capacity(len);
    // SAFETY: `ptr`/`len` were produced by `&EXPORTS.as_ptr()` /
    // `.len()` inside the cdylib; both remain valid until we drop
    // `lib` below. We copy strings here so the result is fully
    // owned and the cdylib can be dropped.
    let slice: &[ExportEntry] = unsafe { std::slice::from_raw_parts(ptr, len) };
    for e in slice {
        out.push(EntryView {
            logical_name: e.logical_name.to_owned(),
            mangled: e.mangled.to_owned(),
            param_types: e.param_types.iter().map(|s| (*s).to_owned()).collect(),
            ret_type: e.ret_type.to_owned(),
            isolated: e.isolated,
            abi_version: e.abi_version,
        });
    }

    // Drop the lib *after* we've copied everything out. Pointer
    // lifetime contract preserved.
    drop(lib);
    Ok(out)
}

// ─── User-defined type schema (RC.2 patch 2, 2026-05-31) ────────────

/// Borrowed view of a single `FieldEntry`, owned post-dlclose.
#[derive(Debug, Clone)]
struct FieldView {
    name: String,
    /// IDL-side mangle. Reserved for the RC.3 mangle-first
    /// translation path; RC.2 wrapper-emit reads `rust_type`
    /// exclusively (the macro emits empty `type_mangle` because
    /// proc-macro context doesn't carry IDL access).
    #[allow(dead_code)]
    type_mangle: String,
    rust_type: String,
}

/// Borrowed view of a single `CtorEntry`, owned post-dlclose.
#[derive(Debug, Clone)]
struct CtorView {
    name: String,
    fields: Vec<FieldView>,
}

/// Borrowed view of a single `UserTypeEntry`, owned post-dlclose.
#[derive(Debug, Clone)]
struct UserTypeView {
    fqn: String,
    kind: UserTypeKind,
    fields: Vec<FieldView>,
    ctors: Vec<CtorView>,
}

/// SAFETY: caller must pass a path that is a leo4-exporting cdylib
/// produced by the same workspace (or any cdylib whose
/// `leo4_rust_describe_user_types` symbol matches the `repr(C)`
/// `UserTypeEntry` layout in this build of `leo4-abi`).
unsafe fn load_user_types(cdylib: &Path) -> Result<Vec<UserTypeView>, String> {
    type Describe =
        unsafe extern "C" fn(*mut *const UserTypeEntry, *mut usize) -> i32;

    // SAFETY: opening a shared library is inherently unsafe.
    let lib = unsafe {
        libloading::Library::new(cdylib)
            .map_err(|e| format!("dlopen {cdylib:?}: {e}"))?
    };

    let sym: libloading::Symbol<'_, Describe> = unsafe {
        lib.get(b"leo4_rust_describe_user_types\0").map_err(|e| {
            format!(
                "cdylib does not export `leo4_rust_describe_user_types` ({e}). \
                 Older builds (pre-2026-05-31) didn't emit the user-type \
                 schema slice; wrapper-side mirror Lean decls will be \
                 skipped — user-defined nominal types referenced by \
                 exports will surface as opaque `axiom T : Type` stubs."
            )
        })?
    };

    let mut ptr: *const UserTypeEntry = std::ptr::null();
    let mut len: usize = 0;
    // SAFETY: outparams are valid; symbol matches signature.
    let rc = unsafe { sym(&raw mut ptr, &raw mut len) };
    if rc != 0 {
        return Err(format!("leo4_rust_describe_user_types returned {rc}"));
    }
    if len > 0 && ptr.is_null() {
        return Err(
            "describe_user_types returned non-zero length with null pointer".into(),
        );
    }

    let mut out = Vec::with_capacity(len);
    let slice: &[UserTypeEntry] =
        unsafe { std::slice::from_raw_parts(ptr, len) };
    for e in slice {
        let fields: Vec<FieldView> = e
            .fields
            .iter()
            .map(|f| view_field(f))
            .collect();
        let ctors: Vec<CtorView> = e
            .ctors
            .iter()
            .map(|c| CtorView {
                name: c.name.to_owned(),
                fields: c.fields.iter().map(view_field).collect(),
            })
            .collect();
        out.push(UserTypeView {
            fqn: e.fqn.to_owned(),
            kind: e.kind,
            fields,
            ctors,
        });
    }
    drop(lib);
    Ok(out)
}

fn view_field(f: &FieldEntry) -> FieldView {
    FieldView {
        name: f.name.to_owned(),
        type_mangle: f.type_mangle.to_owned(),
        rust_type: f.rust_type.to_owned(),
    }
}

// ─── IDL rendering ───────────────────────────────────────────────────

fn render_canonical_idl(pkg: &str, iface: &str, entries: &[EntryView]) -> String {
    // Collapsed form (single space between tokens, no newlines).
    // This is the input to the schema_hash. Function order: sorted
    // by mangled-name (stable across re-runs, matches the forward
    // direction's sort discipline in `SPEC/handshake.md`).
    let mut sorted: Vec<&EntryView> = entries.iter().collect();
    sorted.sort_by(|a, b| a.mangled.cmp(&b.mangled));

    let mut s = String::new();
    s.push_str(&format!("package {pkg}; interface {iface} {{ "));
    for e in &sorted {
        s.push_str(&format_func_decl(e));
        s.push(' ');
    }
    s.push('}');
    s
}

fn render_pretty_idl(pkg: &str, iface: &str, entries: &[EntryView]) -> String {
    let mut sorted: Vec<&EntryView> = entries.iter().collect();
    sorted.sort_by(|a, b| a.mangled.cmp(&b.mangled));

    let mut s = String::new();
    s.push_str(&format!("package {pkg};\n"));
    s.push_str(&format!("interface {iface} {{\n"));
    for e in &sorted {
        s.push_str("  ");
        s.push_str(&format_func_decl(e));
        s.push('\n');
    }
    s.push_str("}\n");
    s
}

fn format_func_decl(e: &EntryView) -> String {
    // `func <name>(_0: T0, _1: T1) -> R;`
    // Argument names are positional `_<i>` to match forward
    // direction's canonical form.
    let mut s = String::new();
    s.push_str("func ");
    s.push_str(&e.logical_name);
    s.push('(');
    for (i, p) in e.param_types.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        s.push_str(&format!("_{i}: "));
        s.push_str(&idl_form_of(p));
    }
    s.push(')');
    if !e.ret_type.is_empty() {
        s.push_str(" -> ");
        s.push_str(&idl_form_of(&e.ret_type));
    }
    s.push(';');
    s
}

/// Best-effort prettifier from mangled token (e.g. `u64`,
/// `L_u32_l`, `S_Sample_Point_s`) to IDL surface form (e.g.
/// `u64`, `list<u32>`, `Sample.Point`).
///
/// Phase 9-2 keeps this minimal — it inverts the small subset of
/// `mangle_type` produced by `rust_type_to_idl`. The full
/// inverse lands when 9-5 (Lake-side ingestion) needs it.
fn idl_form_of(mangle: &str) -> String {
    if let Ok((idl, _rest)) = parse_mangle(mangle) {
        idl_to_surface(&idl)
    } else {
        // Fallback: emit the raw mangle so downstream tools can
        // still identify the function even if we can't pretty it.
        mangle.to_string()
    }
}

fn idl_to_surface(t: &schema_idl::IDLType) -> String {
    use schema_idl::IDLType::{U8, U16, U32, U64, I8, I16, I32, I64, F32, F64, Bool, Char, String, BigInt, BigNat, List, Option, Result, Tuple, Record};
    match t {
        U8 => "u8".into(),
        U16 => "u16".into(),
        U32 => "u32".into(),
        U64 => "u64".into(),
        I8 => "i8".into(),
        I16 => "i16".into(),
        I32 => "i32".into(),
        I64 => "i64".into(),
        F32 => "f32".into(),
        F64 => "f64".into(),
        Bool => "bool".into(),
        Char => "char".into(),
        String => "string".into(),
        BigInt => "bigint".into(),
        BigNat => "bignat".into(),
        List(inner) => format!("list<{}>", idl_to_surface(inner)),
        Option(inner) => format!("option<{}>", idl_to_surface(inner)),
        Result(t, e) => match e {
            Some(et) => format!("result<{}, {}>", idl_to_surface(t), idl_to_surface(et)),
            None => format!("result<{}>", idl_to_surface(t)),
        },
        Tuple(ts) => {
            let inner = ts
                .iter()
                .map(idl_to_surface)
                .collect::<Vec<_>>()
                .join(", ");
            format!("tuple<{inner}>")
        }
        Record { fqn, args } => {
            if args.is_empty() {
                fqn.clone()
            } else {
                let inner = args
                    .iter()
                    .map(idl_to_surface)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{fqn}<{inner}>")
            }
        }
        other => format!("{other:?}"),
    }
}

/// Minimal `mangle_type` inverse. Supports the subset
/// `rust_type_to_idl` emits today (scalars, list / option /
/// result / tuple, records with FQN). Returns the parsed IDL
/// plus the unconsumed tail.
fn parse_mangle(s: &str) -> Result<(schema_idl::IDLType, &str), String> {
    use schema_idl::IDLType::{U8, U16, U32, U64, I8, I16, I32, I64, F32, F64, Bool, Char, String, BigInt, BigNat, List, Option, Record};
    let s = s.trim();
    for (tok, ty) in [
        ("u8", U8),
        ("u16", U16),
        ("u32", U32),
        ("u64", U64),
        ("i8", I8),
        ("i16", I16),
        ("i32", I32),
        ("i64", I64),
        ("f32", F32),
        ("f64", F64),
        ("b", Bool),
        ("c", Char),
        ("str", String),
        ("bI", BigInt),
        ("bN", BigNat),
    ] {
        if s == tok {
            return Ok((ty, ""));
        }
    }
    // Composite leaders.
    if let Some(rest) = s.strip_prefix("L_").and_then(|r| r.strip_suffix("_l")) {
        let (inner, _) = parse_mangle(rest)?;
        return Ok((List(Box::new(inner)), ""));
    }
    if let Some(rest) = s.strip_prefix("O_").and_then(|r| r.strip_suffix("_o")) {
        let (inner, _) = parse_mangle(rest)?;
        return Ok((Option(Box::new(inner)), ""));
    }
    if let Some(rest) = s.strip_prefix("S_").and_then(|r| r.strip_suffix("_s")) {
        // For now treat the body as a literal FQN (no generic
        // args). Phase 9-2 keeps this conservative; 9-5 widens it.
        let fqn = rest.replace('_', ".");
        return Ok((
            Record {
                fqn,
                args: vec![],
            },
            "",
        ));
    }
    Err(format!("unrecognised mangle token: `{s}`"))
}

// ─── schema_hash ─────────────────────────────────────────────────────

fn compute_schema_hash(canonical: &str) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut h = OFFSET;
    for &b in canonical.as_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(PRIME);
    }
    let bytes = h.to_be_bytes();
    base32lc(&bytes)
}

const BASE32_LC: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

fn base32lc(input: &[u8]) -> String {
    let mut out = String::with_capacity((input.len() * 8).div_ceil(5));
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &b in input {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buf >> bits) & 0x1f) as usize;
            out.push(char::from(BASE32_LC[idx]));
        }
    }
    if bits > 0 {
        let idx = ((buf << (5 - bits)) & 0x1f) as usize;
        out.push(char::from(BASE32_LC[idx]));
    }
    out
}

// ─── Lean wrapper emission (Phase 9-5) ───────────────────────────────

/// Generates `<pkg>.leo4-rust-imports.lean` — a Lean wrapper
/// module that exposes one typed `IO` action per `#[leo4::export]`.
///
/// Wrapper anatomy:
///
/// ```text
/// -- auto-generated; do not edit
/// import Leo4
///
/// namespace <module>
///
/// /-- Compile-time pin: schema_hash for this cdylib. The
///     dispatcher's worker verifies against this on init. -/
/// def schemaHash : String := "<hash>"
///
/// /-- Raw entry — the Lean-side glue shim (Phase 9-6) wraps
///     `leo4_rust_call`; this `@[extern]` reaches it. -/
/// @[extern "leo4_rust_call_lean"]
/// private opaque leo4RustCallRaw
///     (mangled : @& String) (args : @& ByteArray)
///     : BaseIO (UInt32 × ByteArray)
///
/// /-- Per-export typed wrapper. -/
/// def add (a : UInt64) (b : UInt64) : IO UInt64 := do …
///
/// end <module>
/// ```
///
/// The `leo4_rust_call_lean` extern lives in a small leanc-compiled
/// C shim that 9-6 (Lake link step) builds; until then the
/// generated `.lean` file compiles only the Lean side — the
/// `@[extern]` symbol stays unresolved which surfaces as a
/// linker error at user-binary link time, not at this emit
/// time.
fn render_lean_wrapper(
    module: &str,
    _pkg: &str,
    _iface: &str,
    schema_hash: &str,
    entries: &[EntryView],
    user_types: &[UserTypeView],
) -> Result<String, String> {
    let mut s = String::new();
    s.push_str("-- Auto-generated by leo4-rust-emit. Do not edit.\n");
    s.push_str("-- See SPEC/reverse-direction.md.\n");
    s.push_str("--\n");
    s.push_str("-- Each `#[leo4::export]` Rust function in the source\n");
    s.push_str("-- cdylib surfaces here as one typed `IO α` action.\n\n");
    s.push_str("import Leo4\n\n");
    s.push_str(&format!("namespace {module}\n\n"));

    // RC.2 patch 2 (2026-05-31) — emit mirror Lean declarations
    // for every user-defined nominal type referenced via
    // `#[derive(LeanMarshal)]` in the cdylib. Each entry
    // produces either a `structure` (record / tuple-record /
    // unit-struct) or an `inductive` (variant / unit-enum) with
    // a `deriving Leo4.LeanMarshal` clause so the wrapper's
    // `canonicalEncode` / `canonicalDecode` calls resolve via
    // the macro-derived instance. Wire-format parity with the
    // Rust `#[derive(LeanMarshal)]` is the macro's
    // responsibility — `crates/leo4-macros-backend` emits
    // identical encode/decode logic on both sides.
    if !user_types.is_empty() {
        s.push_str(&render_user_type_mirror_block(user_types));
    }

    s.push_str("/-- Compile-time pin: schema_hash for this cdylib.\n");
    s.push_str("    The dispatcher's worker verifies against this on init. -/\n");
    s.push_str(&format!(
        "def schemaHash : String := \"{schema_hash}\"\n\n"
    ));

    s.push_str("/-- Raw dispatcher entry. The Lean-side glue shim\n");
    s.push_str("    (`shim/leo4_rust_bridge_lean.c`, Phase 9-6) routes\n");
    s.push_str("    `mangled` + `args` through `leo4_rust_call` and packs\n");
    s.push_str("    the response into a single ByteArray:\n");
    s.push_str("      * bytes[0..4] — status (LE u32). 0 = success.\n");
    s.push_str("      * bytes[4..]  — canonical-ABI return payload on success;\n");
    s.push_str("                      empty on error.\n");
    s.push_str("    The single-ByteArray representation avoids Lean's Prod\n");
    s.push_str("    inline-scalar codegen — UInt32 in `(UInt32 × ByteArray)`\n");
    s.push_str("    isn't laid out the way a naive C ctor builder expects. -/\n");
    s.push_str("@[extern \"leo4_rust_call_lean\"]\n");
    s.push_str("private opaque leo4RustCallRaw\n");
    s.push_str("    (mangled : @& String) (args : @& ByteArray)\n");
    s.push_str("    : IO ByteArray\n\n");

    s.push_str("/-- Decode the 4-byte LE u32 status prefix of the dispatcher\n");
    s.push_str("    response. -/\n");
    s.push_str("private def decodeStatus (buf : ByteArray) : UInt32 :=\n");
    s.push_str("  if buf.size < 4 then 0xffffffff  -- malformed; treat as error\n");
    s.push_str("  else\n");
    s.push_str("    (buf.get! 0).toUInt32 |||\n");
    s.push_str("    ((buf.get! 1).toUInt32 <<< 8) |||\n");
    s.push_str("    ((buf.get! 2).toUInt32 <<< 16) |||\n");
    s.push_str("    ((buf.get! 3).toUInt32 <<< 24)\n\n");

    // RC.6 F3 (2026-05-31) — extract the cdylib's user-type
    // fqn list so the mangle decoder can resolve generic-
    // instantiation mangles via `known_fqns` lookup.
    let known_fqns: Vec<String> = user_types
        .iter()
        .map(|t| t.fqn.clone())
        .collect();

    let mut sorted: Vec<&EntryView> = entries.iter().collect();
    sorted.sort_by(|a, b| a.logical_name.cmp(&b.logical_name));
    for e in sorted {
        s.push_str(&render_one_export(e, &known_fqns)?);
        s.push('\n');
    }

    s.push_str(&format!("end {module}\n"));
    Ok(s)
}

/// RC.2 patch 2 — emit the mirror Lean declaration block at
/// the top of the wrapper namespace. One `structure` /
/// `inductive` per user-defined type, in declaration order
/// preserved from `USER_TYPES` (linkme order — stable across
/// re-runs because the macro emits one entry per `derive`
/// site).
fn render_user_type_mirror_block(types: &[UserTypeView]) -> String {
    let mut s = String::new();
    s.push_str("/-- ── User-defined nominal types ─────────────────\n");
    s.push_str("    Mirror declarations for every `#[derive(LeanMarshal)]`\n");
    s.push_str("    type referenced by this wrapper's exports. Auto-\n");
    s.push_str("    synthesised from the cdylib's `USER_TYPES`\n");
    s.push_str("    distributed-slice (RC.2 patch 2, 2026-05-31). The\n");
    s.push_str("    `deriving Leo4.LeanMarshal` clause on each decl\n");
    s.push_str("    produces a wire-format-equivalent `LeanMarshal`\n");
    s.push_str("    instance matching the Rust side. -/\n\n");
    for ty in types {
        s.push_str(&render_one_user_type(ty));
        s.push('\n');
    }
    s
}

fn render_one_user_type(ty: &UserTypeView) -> String {
    match ty.kind {
        UserTypeKind::Record => render_record_mirror(ty),
        UserTypeKind::TupleRecord => render_tuple_record_mirror(ty),
        UserTypeKind::Variant => render_variant_mirror(ty),
        UserTypeKind::UnitEnum => render_unit_enum_mirror(ty),
        UserTypeKind::UnitStruct => render_unit_struct_mirror(ty),
    }
}

fn render_record_mirror(ty: &UserTypeView) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "/-- Mirror of Rust `#[derive(LeanMarshal)] struct {0} {{ … }}`. -/\n",
        ty.fqn
    ));
    s.push_str(&format!("structure {} where\n", ty.fqn));
    if ty.fields.is_empty() {
        s.push_str("  -- (empty record)\n");
    } else {
        for f in &ty.fields {
            let lean_ty = rust_type_to_lean_type(&f.rust_type);
            let field_name = if f.name.is_empty() {
                "field0".to_string()
            } else {
                lean_safe_ident(&f.name)
            };
            s.push_str(&format!("  {field_name} : {lean_ty}\n"));
        }
    }
    s.push_str(&format!("deriving Leo4.LeanMarshal\n"));
    s
}

fn render_tuple_record_mirror(ty: &UserTypeView) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "/-- Mirror of Rust `#[derive(LeanMarshal)] struct {0}(…);` (tuple struct). -/\n",
        ty.fqn
    ));
    s.push_str(&format!("structure {} where\n", ty.fqn));
    if ty.fields.is_empty() {
        s.push_str("  -- (empty tuple struct)\n");
    } else {
        for (i, f) in ty.fields.iter().enumerate() {
            let lean_ty = rust_type_to_lean_type(&f.rust_type);
            s.push_str(&format!("  field{i} : {lean_ty}\n"));
        }
    }
    s.push_str(&format!("deriving Leo4.LeanMarshal\n"));
    s
}

fn render_variant_mirror(ty: &UserTypeView) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "/-- Mirror of Rust `#[derive(LeanMarshal)] enum {0} {{ … }}`. -/\n",
        ty.fqn
    ));
    s.push_str(&format!("inductive {} where\n", ty.fqn));
    for c in &ty.ctors {
        let ctor_name = lowercase_first(&c.name);
        let ctor_safe = lean_safe_ident(&ctor_name);
        if c.fields.is_empty() {
            s.push_str(&format!("  | {ctor_safe} : {0}\n", ty.fqn));
        } else {
            s.push_str(&format!("  | {ctor_safe}"));
            for (i, f) in c.fields.iter().enumerate() {
                let lean_ty = rust_type_to_lean_type(&f.rust_type);
                let binder_name = if f.name.is_empty() {
                    format!("_arg{i}")
                } else {
                    lean_safe_ident(&f.name)
                };
                s.push_str(&format!(" ({binder_name} : {lean_ty})"));
            }
            s.push_str(&format!(" : {0}\n", ty.fqn));
        }
    }
    s.push_str(&format!("deriving Leo4.LeanMarshal\n"));
    s
}

fn render_unit_enum_mirror(ty: &UserTypeView) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "/-- Mirror of Rust `#[derive(LeanMarshal)] enum {0} {{ … }}` (all unit variants). -/\n",
        ty.fqn
    ));
    s.push_str(&format!("inductive {} where\n", ty.fqn));
    for c in &ty.ctors {
        let ctor_name = lowercase_first(&c.name);
        let ctor_safe = lean_safe_ident(&ctor_name);
        s.push_str(&format!("  | {ctor_safe} : {0}\n", ty.fqn));
    }
    s.push_str(&format!("deriving Leo4.LeanMarshal\n"));
    s
}

fn render_unit_struct_mirror(ty: &UserTypeView) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "/-- Mirror of Rust `#[derive(LeanMarshal)] struct {0};` (unit struct). -/\n",
        ty.fqn
    ));
    s.push_str(&format!("structure {} where\n", ty.fqn));
    s.push_str(&format!("deriving Leo4.LeanMarshal\n"));
    s
}

/// Lowercase the first ASCII alphabetic char in `s`. Used to map
/// Rust ctor idents (`Sat`, `Unsat`, `Abductive`, `Unknown`) to
/// Lean ctor idents (`sat`, `unsat`, `abductive`, `unknown` —
/// Lean's stdlib convention for inductive ctor names).
fn lowercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_ascii_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// RC.2 patch 2 — translate a Rust source-text type
/// (whitespace-normalised by the derive macro, e.g.
/// `"Vec<(String,String)>"`) into a Lean type expression
/// (`"Array (String × String)"`). Uses `syn::parse_str::<Type>`
/// to get a typed AST, then walks the result.
///
/// Recognised Rust types:
///
/// - Primitives: `u8`/`u16`/.../`u128` → `UInt8`/.../`UInt128`,
///   `i8`/.../`i128` → `Int8`/.../`Int128`, `f32`/`f64` →
///   `Float32`/`Float`, `bool` → `Bool`, `char` → `Char`,
///   `()` → `Unit`.
/// - Strings: `String` → `String`, `&str` / `&'static str` →
///   `String` (Lean has no borrowed-string type at the boundary).
/// - Collections: `Vec<u8>` → `ByteArray`, `Vec<T>` → `Array T`,
///   `Option<T>` → `Option T`, `Result<T, E>` → `Except E T`.
/// - Tuples: `(A, B)` → `A × B`, `(A, B, C)` → `A × B × C` (right-
///   associative).
/// - BigInt / BigNat: `BigInt` → `Int`, `BigNat` → `Nat`.
/// - Box / reference: `Box<T>` → `T` (transparent at the boundary).
/// - Unknown: passes through verbatim — assumes the input is a
///   user-defined nominal type whose mirror decl lives in the
///   same wrapper file.
fn rust_type_to_lean_type(rust_src: &str) -> String {
    match syn::parse_str::<syn::Type>(rust_src) {
        Ok(ty) => translate_syn_type(&ty),
        Err(_) => {
            // Parse failed (the macro emitted an unparseable
            // form). Fall back to the source text; the wrapper
            // user will see a Lean-level type error pointing at
            // the offending line and can hand-correct.
            rust_src.to_string()
        }
    }
}

fn translate_syn_type(ty: &syn::Type) -> String {
    use syn::Type;
    match ty {
        Type::Path(tp) => translate_type_path(tp),
        Type::Tuple(tt) => {
            if tt.elems.is_empty() {
                return "Unit".to_string();
            }
            let parts: Vec<String> = tt
                .elems
                .iter()
                .map(translate_syn_type)
                .collect();
            // Right-associative product. `(A, B, C)` → `A × (B × C)`
            // — matches Lean's Prod convention; parens preserve
            // associativity.
            translate_tuple_right_assoc(&parts)
        }
        Type::Reference(tr) => translate_syn_type(&tr.elem),
        Type::Paren(tp) => format!("({})", translate_syn_type(&tp.elem)),
        Type::Array(ta) => {
            // Rust `[T; N]` → Lean `Array T` (length erased
            // at the boundary; matches `Vec<T>` lowering).
            format!("Array {}", translate_syn_type(&ta.elem))
        }
        Type::Slice(ts) => {
            format!("Array {}", translate_syn_type(&ts.elem))
        }
        _ => quote::ToTokens::to_token_stream(ty).to_string(),
    }
}

fn translate_type_path(tp: &syn::TypePath) -> String {
    let seg = match tp.path.segments.last() {
        Some(s) => s,
        None => return quote::ToTokens::to_token_stream(tp).to_string(),
    };
    let name = seg.ident.to_string();

    // Extract generic args, if any.
    let args: Vec<&syn::Type> = match &seg.arguments {
        syn::PathArguments::AngleBracketed(ab) => ab
            .args
            .iter()
            .filter_map(|a| match a {
                syn::GenericArgument::Type(t) => Some(t),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };

    match (name.as_str(), args.len()) {
        // Primitives.
        ("u8", 0) => "UInt8".to_string(),
        ("u16", 0) => "UInt16".to_string(),
        ("u32", 0) => "UInt32".to_string(),
        ("u64", 0) => "UInt64".to_string(),
        ("u128", 0) => "UInt128".to_string(),
        ("usize", 0) => "USize".to_string(),
        ("i8", 0) => "Int8".to_string(),
        ("i16", 0) => "Int16".to_string(),
        ("i32", 0) => "Int32".to_string(),
        ("i64", 0) => "Int64".to_string(),
        ("i128", 0) => "Int128".to_string(),
        ("isize", 0) => "ISize".to_string(),
        ("f32", 0) => "Float32".to_string(),
        ("f64", 0) => "Float".to_string(),
        ("bool", 0) => "Bool".to_string(),
        ("char", 0) => "Char".to_string(),
        ("String", 0) => "String".to_string(),
        ("str", 0) => "String".to_string(),
        ("BigInt", 0) => "Int".to_string(),
        ("BigNat", 0) => "Nat".to_string(),
        // Containers.
        ("Vec", 1) => {
            let inner = translate_syn_type(args[0]);
            if inner == "UInt8" {
                "ByteArray".to_string()
            } else {
                format!("Array ({inner})")
            }
        }
        ("Option", 1) => format!("Option ({})", translate_syn_type(args[0])),
        ("Result", 2) => {
            // Rust `Result<T, E>` ↔ Lean `Except E T` (Lean
            // puts error before value).
            let t = translate_syn_type(args[0]);
            let e = translate_syn_type(args[1]);
            format!("Except ({e}) ({t})")
        }
        ("Box", 1) => translate_syn_type(args[0]),
        // User-defined / unknown — pass through. Generic
        // user types apply args directly: `Pair<u32, str>` →
        // `Pair UInt32 String`.
        _ => {
            if args.is_empty() {
                name
            } else {
                let arg_strs: Vec<String> =
                    args.iter().map(|a| translate_syn_type(a)).collect();
                let arg_block: Vec<String> = arg_strs
                    .iter()
                    .map(|s| format!("({s})"))
                    .collect();
                format!("{name} {}", arg_block.join(" "))
            }
        }
    }
}

fn translate_tuple_right_assoc(parts: &[String]) -> String {
    match parts.len() {
        0 => "Unit".to_string(),
        1 => parts[0].clone(),
        2 => format!("({} × {})", parts[0], parts[1]),
        _ => {
            let head = &parts[0];
            let tail = translate_tuple_right_assoc(&parts[1..]);
            format!("({head} × {tail})")
        }
    }
}

fn render_one_export(
    e: &EntryView,
    known_fqns: &[String],
) -> Result<String, String> {
    // Map each parameter / return mangle to a Lean type. As of
    // RC.2 (2026-05-31) `lean_type_of_mangle` covers: scalars,
    // `String`, `ByteArray` (= `Vec<u8>`), `Array T`, `Option T`,
    // and all 5 user-defined nominal kinds (record `S_<fqn>_s`,
    // variant `V_<fqn>_v`, enum `E_<fqn>_e`, flags `F_<fqn>_f`,
    // resource `X_<fqn>_x`). The matching mirror Lean
    // declaration is emitted by `render_user_type_mirror_block`
    // via the cdylib's `USER_TYPES` distributed slice (RC.2
    // patch 2), so the wrapper file is fully self-contained.
    // Mangles that still don't decode (generic instantiations
    // of user types — the RC.2 tokeniser is heuristic) fall
    // back to a Lean-level `panic!` body so the build still
    // succeeds.
    let mut param_idents: Vec<String> = Vec::with_capacity(e.param_types.len());
    let mut param_lean_tys: Vec<String> = Vec::with_capacity(e.param_types.len());
    let mut all_mapped = true;
    for (i, m) in e.param_types.iter().enumerate() {
        param_idents.push(format!("a{i}"));
        if let Some(t) = lean_type_of_mangle_with_user_fqns(m, known_fqns) {
            param_lean_tys.push(t)
        } else {
            all_mapped = false;
            param_lean_tys.push(format!("/- unmapped: {m} -/ String"));
        }
    }
    let ret_ty = if e.ret_type.is_empty() {
        Some("Unit".to_string())
    } else {
        lean_type_of_mangle_with_user_fqns(&e.ret_type, known_fqns)
    };
    let ret_ty_str = ret_ty.clone().unwrap_or_else(|| format!("/- unmapped: {} -/ Unit", e.ret_type));
    if ret_ty.is_none() {
        all_mapped = false;
    }

    let fname = lean_safe_ident(&e.logical_name);

    let mut s = String::new();
    s.push_str(&format!("/-- `{}` (mangled: `{}`). -/\n", e.logical_name, e.mangled));
    s.push_str(&format!("def {fname}"));
    for (id, ty) in param_idents.iter().zip(param_lean_tys.iter()) {
        s.push_str(&format!(" ({id} : {ty})"));
    }
    // Wrap the return type in parens so Lean parses
    // `IO (Option UInt64)` rather than `IO Option UInt64`.
    s.push_str(&format!(" : IO ({ret_ty_str}) := do\n"));

    if !all_mapped {
        s.push_str(&format!(
            "  panic! \"leo4-rust-emit (RC.2+): export `{}` has an unmapped parameter or return mangle ({:?}). Most user-defined nominal types and primitives decode cleanly post-RC.2; failures here typically indicate a generic-instantiation mangle the heuristic tokeniser couldn't split (rename to a flat fqn or wait for the full mangle parser)\"\n",
            e.logical_name, e.param_types
        ));
        return Ok(s);
    }

    // Body: encode each arg, single call, decode return.
    s.push_str("  let mut args : ByteArray := ByteArray.empty\n");
    for (id, mangle) in param_idents.iter().zip(e.param_types.iter()) {
        let _ = mangle; // we already know the type maps
        s.push_str(&format!(
            "  args := Leo4.LeanMarshal.canonicalEncode {id} args\n"
        ));
    }
    // Isolated exports get an `iso:` prefix on the mangled name
    // so the dispatcher's `leo4_rust_call` routes them through the
    // per-call fresh worker path (SPEC §4.2). Persistent exports
    // pass the raw mangled name through verbatim.
    let dispatched_mangled = if e.isolated {
        format!("iso:{}", e.mangled)
    } else {
        e.mangled.clone()
    };
    s.push_str(&format!(
        "  let resp ← leo4RustCallRaw \"{dispatched_mangled}\" args\n",
    ));
    s.push_str("  let status := decodeStatus resp\n");
    s.push_str("  if status ≠ 0 then\n");
    s.push_str(&format!(
        "    throw (IO.userError s!\"leo4 rust call `{}` failed with status {{status}}\")\n",
        e.logical_name
    ));
    if e.ret_type.is_empty() {
        s.push_str("  return ()\n");
    } else {
        let lean_ret = lean_type_of_mangle_with_user_fqns(
            &e.ret_type,
            known_fqns,
        ).unwrap();
        // Decode starting at offset 4 (after the status prefix).
        s.push_str(&format!(
            "  match (Leo4.LeanMarshal.canonicalDecode (T := {lean_ret}) resp 4) with\n"
        ));
        s.push_str("  | .ok (v, _) => return v\n");
        s.push_str(&format!(
            "  | .error e => throw (IO.userError s!\"leo4 rust call `{}`: decode failed: {{e.message}}\")\n",
            e.logical_name
        ));
    }
    Ok(s)
}

/// Map an IDL mangle token to a Lean type identifier. Mirrors the
/// `surface_form` table but produces Lean-side names.
/// Backward-compat wrapper — calls
/// [`lean_type_of_mangle_with_user_fqns`] with an empty
/// `known_fqns` list. Generic instantiations of user-defined
/// types still hit the heuristic-only path; for accurate
/// splits the caller should use the `_with_user_fqns` variant
/// directly with the cdylib's `USER_TYPES`-derived fqn list.
#[cfg(test)]
fn lean_type_of_mangle(mangle: &str) -> Option<String> {
    lean_type_of_mangle_with_user_fqns(mangle, &[])
}

/// RC.6 F3 (2026-05-31) — mangle decoder with an explicit
/// list of *known user-defined fqns* (post-`fqn_seg`
/// normalisation, i.e. with dots and dashes already collapsed
/// to underscores). When the input is a record / variant /
/// resource mangle (`S_…_s` / `V_…_v` / `X_…_x` — Enum and
/// Flags are kind-only and carry no generic args by SPEC
/// design), the decoder:
///
/// 1. Greedily matches the longest known fqn that is a prefix
///    of the middle segment (after stripping the kind prefix
///    and suffix).
/// 2. Treats the remainder as the generic-arg list, each arg
///    itself a recursively-decodable mangle.
/// 3. Renders `<fqn> (<arg1>) (<arg2>) ... (<argN>)`.
///
/// When no known fqn matches (or `known_fqns` is empty), falls
/// back to the RC.2 patch 1 `mangle_segment_is_plain_fqn`
/// heuristic. The heuristic rejects mangles with embedded
/// primitive tokens (the generic-instantiation case) and
/// accepts plain alphabetic fqn-only mangles.
///
/// Lossy round-trip: `fqn_seg` collapses `.` and `-` to `_`
/// (`Sample.Color` and `Sample_Color` both mangle to
/// `Sample_Color`). The decoder cannot distinguish them; the
/// `USER_TYPES` slice carries the macro-emitted ident form
/// (typically the flat Rust ident with no dots), so the
/// `known_fqns` match returns whatever the macro recorded.
fn lean_type_of_mangle_with_user_fqns(
    mangle: &str,
    known_fqns: &[String],
) -> Option<String> {
    Some(match mangle {
        "u8" => "UInt8".into(),
        "u16" => "UInt16".into(),
        "u32" => "UInt32".into(),
        "u64" => "UInt64".into(),
        "i8" => "Int8".into(),
        "i16" => "Int16".into(),
        "i32" => "Int32".into(),
        "i64" => "Int64".into(),
        "f32" => "Float32".into(),
        "f64" => "Float".into(),
        "b" => "Bool".into(),
        "c" => "Char".into(),
        "str" => "String".into(),
        "bI" => "Int".into(),
        "bN" => "Nat".into(),
        other => {
            // List<u8> → ByteArray; List<T> → Array T (best effort).
            if let Some(rest) =
                other.strip_prefix("L_").and_then(|r| r.strip_suffix("_l"))
            {
                if rest == "u8" {
                    return Some("ByteArray".into());
                }
                let inner = lean_type_of_mangle_with_user_fqns(rest, known_fqns)?;
                return Some(format!("Array {}", paren_if_multi_token(&inner)));
            }
            if let Some(rest) =
                other.strip_prefix("O_").and_then(|r| r.strip_suffix("_o"))
            {
                let inner = lean_type_of_mangle_with_user_fqns(rest, known_fqns)?;
                return Some(format!("Option {}", paren_if_multi_token(&inner)));
            }
            // User-defined nominal types — RC.6 F3
            // (2026-05-31) lifted RC.2 patch 1's heuristic
            // with a proper tokeniser that consults
            // `known_fqns`. Mangle prefixes per
            // `crates/schema-idl/src/mangle.rs::mangle_type`:
            //
            // - `S_<fqn>_s` / `S_<fqn>_<args>_s` — record.
            // - `V_<fqn>_v` / `V_<fqn>_<args>_v` — variant.
            // - `E_<fqn>_e` — enum (all-unit; no generics
            //   by SPEC design).
            // - `F_<fqn>_f` — flags (no generics by design).
            // - `X_<fqn>_x` / `X_<fqn>_<args>_x` — resource.
            //
            // Strategy: greedy-match the longest known FQN
            // that prefixes `rest` (after stripping kind
            // prefix + suffix). Split rest into FQN +
            // remaining; treat remaining as the `_arg1_arg2_…`
            // generic-arg sequence, each arg itself a
            // recursively-decodable mangle. When no known
            // FQN matches (or `known_fqns` is empty),
            // fall back to the original
            // `mangle_segment_is_plain_fqn` heuristic.
            //
            // Round-trip is lossy on `.`/`-` → `_` collapse;
            // see `lean_type_of_mangle_with_user_fqns` doc.
            if let Some(rest) =
                other.strip_prefix("S_").and_then(|r| r.strip_suffix("_s"))
            {
                return decode_nominal_with_args(rest, known_fqns);
            }
            if let Some(rest) =
                other.strip_prefix("V_").and_then(|r| r.strip_suffix("_v"))
            {
                return decode_nominal_with_args(rest, known_fqns);
            }
            if let Some(rest) =
                other.strip_prefix("E_").and_then(|r| r.strip_suffix("_e"))
            {
                // Enums never carry generic args per SPEC §
                // mangling.md — fall straight back to the
                // plain-fqn or known-fqn match.
                if known_fqns.iter().any(|f| f == rest)
                    || mangle_segment_is_plain_fqn(rest)
                {
                    return Some(rest.to_string());
                }
                return None;
            }
            if let Some(rest) =
                other.strip_prefix("F_").and_then(|r| r.strip_suffix("_f"))
            {
                if known_fqns.iter().any(|f| f == rest)
                    || mangle_segment_is_plain_fqn(rest)
                {
                    return Some(rest.to_string());
                }
                return None;
            }
            if let Some(rest) =
                other.strip_prefix("X_").and_then(|r| r.strip_suffix("_x"))
            {
                return decode_nominal_with_args(rest, known_fqns);
            }
            return None;
        }
    })
}

/// RC.6 F3 helper — wrap a Lean type expression in parens
/// when it contains a space (i.e. it's an App, like
/// `My_Pair (UInt32) (String)`) so outer composite wrappers
/// (`Array T` / `Option T`) parse correctly. Single-token
/// types (`UInt32`, `String`, `AdsmtVerdict`) pass through
/// unwrapped to keep backward compat with the RC.2-era
/// `Array UInt32` / `Option String` test expectations.
fn paren_if_multi_token(t: &str) -> String {
    if t.contains(' ') {
        format!("({t})")
    } else {
        t.to_string()
    }
}

/// RC.6 F3 helper — split a nominal-kind mangle's middle
/// segment (after stripping kind prefix + suffix) into
/// (fqn, [arg_types]). Uses `known_fqns` for greedy longest-
/// match; falls back to the plain-fqn heuristic for no-
/// generics cases.
///
/// Returns `Some(rendered)` when:
///
/// - `rest` exactly matches a known fqn (no args), OR
/// - `rest` exactly matches the plain-fqn heuristic, OR
/// - a known fqn is a prefix of `rest` followed by `_`, and
///   every trailing arg recursively decodes.
///
/// Returns `None` when the split is ambiguous (no known fqn
/// prefix matches, heuristic rejects).
fn decode_nominal_with_args(
    rest: &str,
    known_fqns: &[String],
) -> Option<String> {
    // Case 1: exact known-fqn match (no generic args).
    if known_fqns.iter().any(|f| f == rest) {
        return Some(rest.to_string());
    }
    // Case 2: a known fqn is a prefix followed by `_<args>`.
    // Greedy longest-match — sort by length desc so
    // `My_Pair` wins over `My` when both are registered.
    let mut sorted_fqns: Vec<&String> = known_fqns.iter().collect();
    sorted_fqns.sort_by_key(|f| std::cmp::Reverse(f.len()));
    for fqn in &sorted_fqns {
        if let Some(args_rest) = rest.strip_prefix(fqn.as_str())
            && let Some(args_str) = args_rest.strip_prefix('_')
        {
            // Tokenise args_str into individual arg mangles +
            // recursively decode.
            if let Some(args) = tokenise_arg_list(args_str, known_fqns) {
                let arg_lean: Vec<String> = args
                    .iter()
                    .map(|a| {
                        lean_type_of_mangle_with_user_fqns(a, known_fqns)
                    })
                    .collect::<Option<Vec<_>>>()?;
                let arg_block: Vec<String> =
                    arg_lean.iter().map(|t| format!("({t})")).collect();
                return Some(format!("{fqn} {}", arg_block.join(" ")));
            }
        }
    }
    // Case 3: heuristic fallback — no known fqn knew about
    // this mangle.
    if mangle_segment_is_plain_fqn(rest) {
        return Some(rest.to_string());
    }
    None
}

/// RC.6 F3 helper — split an underscore-joined arg-list
/// string (`u32_str_L_u8_l` etc.) into individual top-level
/// arg mangles. Tracks balanced kind prefix/suffix pairs so a
/// nested `L_u8_l` doesn't get mis-split.
///
/// Returns `Some(args)` when every char is consumed by valid
/// args, `None` if the parse hits a syntactic dead-end.
///
/// Note: balancing tracks `L_/_l`, `O_/_o`, `Rz_/_z`,
/// `T_/_t`, `S_/_s`, `V_/_v`, `E_/_e`, `F_/_f`, `X_/_x`,
/// `I_/_i`, `A_/_a` opening/closing pairs. Primitive tokens
/// (`u8`, `u16`, …, `str`, `bI`, `bN`, `c<i>c`, `self`) are
/// recognised as zero-depth atoms.
fn tokenise_arg_list(
    s: &str,
    known_fqns: &[String],
) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut remaining = s;
    while !remaining.is_empty() {
        let (tok, rest) = take_one_arg_mangle(remaining, known_fqns)?;
        out.push(tok.to_string());
        if rest.is_empty() {
            return Some(out);
        }
        // After consuming one arg, the next byte must be `_`
        // (the separator joining args). Strip it.
        remaining = rest.strip_prefix('_')?;
    }
    Some(out)
}

/// RC.6 F3 helper — take exactly one arg mangle off the front
/// of `s`, returning `(token, rest_after_token)`. Stops at
/// the first valid mangle boundary, respecting balanced
/// kind prefix/suffix pairs.
fn take_one_arg_mangle<'a>(
    s: &'a str,
    known_fqns: &[String],
) -> Option<(&'a str, &'a str)> {
    // Primitives: try the longest first
    // (`bI`/`bN`/`u128`/`u64`/`u32`/`u16`/`u8`/`i128`/`i64`/
    // `i32`/`i16`/`i8`/`f64`/`f32`/`str`/`b`/`c`/`self`).
    for p in &[
        "self", "bI", "bN", "u128", "u64", "u32", "u16", "u8",
        "i128", "i64", "i32", "i16", "i8", "f64", "f32", "str",
        "b", "c",
    ] {
        if let Some(rest) = s.strip_prefix(p) {
            // Boundary check: next char must be `_` or end-
            // of-string (so `u8` doesn't match `u8s_…`).
            if rest.is_empty() || rest.starts_with('_') {
                return Some((&s[..p.len()], rest));
            }
        }
    }
    // Cyc: `c<digits>c`.
    if let Some(rest) = s.strip_prefix('c') {
        let digit_end = rest
            .bytes()
            .position(|b| !b.is_ascii_digit())
            .unwrap_or(rest.len());
        if digit_end > 0 {
            let rest_after_digits = &rest[digit_end..];
            if let Some(after_c) = rest_after_digits.strip_prefix('c') {
                if after_c.is_empty() || after_c.starts_with('_') {
                    return Some((&s[..1 + digit_end + 1], after_c));
                }
            }
        }
    }
    // Kind-prefix mangles: scan for the matching close.
    for (prefix, suffix) in &[
        ("L_", "_l"),
        ("O_", "_o"),
        ("Rz_", "_z"),
        ("T_", "_t"),
        ("S_", "_s"),
        ("V_", "_v"),
        ("E_", "_e"),
        ("F_", "_f"),
        ("X_", "_x"),
        ("I_", "_i"),
        ("A_", "_a"),
    ] {
        if s.starts_with(prefix) {
            if let Some(end) = find_matching_suffix(s, prefix, suffix) {
                return Some((&s[..end], &s[end..]));
            }
            return None;
        }
    }
    // Fallback: known-fqn prefix (bare-no-prefix, only used
    // in nominal-with-args contexts where the outer kind
    // prefix has already been stripped — this branch only
    // fires for recursion into `tokenise_arg_list` when the
    // caller mistakenly forwards a non-mangle bare ident; the
    // SPEC mangle scheme always wraps idents in `S_/V_/X_`
    // kind tags at arg position, so this is dead in
    // practice. Kept for robustness against future SPEC
    // changes.).
    for fqn in known_fqns {
        if let Some(rest) = s.strip_prefix(fqn.as_str()) {
            if rest.is_empty() || rest.starts_with('_') {
                return Some((&s[..fqn.len()], rest));
            }
        }
    }
    None
}

/// Find the byte index in `s` where the matching `suffix` for
/// the opening `prefix` ends. Balances nested kind prefix /
/// suffix pairs by counting depth on every recognised
/// opener / closer pair.
fn find_matching_suffix(s: &str, _prefix: &str, suffix: &str) -> Option<usize> {
    let openers = [
        ("L_", "_l"),
        ("O_", "_o"),
        ("Rz_", "_z"),
        ("T_", "_t"),
        ("S_", "_s"),
        ("V_", "_v"),
        ("E_", "_e"),
        ("F_", "_f"),
        ("X_", "_x"),
        ("I_", "_i"),
        ("A_", "_a"),
    ];
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < s.len() {
        // Check for an opening prefix at position i.
        let tail = &s[i..];
        let mut matched_open = None;
        for (op, _) in &openers {
            if tail.starts_with(op) {
                matched_open = Some(op.len());
                break;
            }
        }
        if let Some(skip) = matched_open {
            depth += 1;
            i += skip;
            continue;
        }
        // Check for a closing suffix at position i. The outer
        // `suffix` we're looking for closes when depth would
        // drop to 0; nested suffixes balance.
        let mut matched_close = None;
        for (op, suf) in &openers {
            if tail.starts_with(suf) {
                // Confirm it's actually a close, not an
                // ident segment containing the letters — by
                // construction `_<letter>` at depth>0
                // closes one level.
                let _ = op;
                matched_close = Some((suf.len(), *suf));
                break;
            }
        }
        if let Some((skip, suf)) = matched_close {
            depth -= 1;
            i += skip;
            if depth == 0 && suf == suffix {
                return Some(i);
            }
            continue;
        }
        // Neither opener nor closer at this byte; advance one
        // char (multibyte-safe via char_indices boundary).
        i += s[i..].chars().next()?.len_utf8();
    }
    None
}

/// RC.2 patch 1 helper — heuristic check that `rest`
/// looks like an unambiguous FQN with no embedded generic
/// args. Returns `true` when every underscore-separated
/// segment is a plain Lean ident segment (alphanumeric,
/// no leading digit) AND none of the segments matches a
/// primitive mangle token (`u8`/`u16`/.../`f32`/`f64`/
/// `b`/`c`/`str`/`bI`/`bN`) or a composite mangle prefix
/// (`L`/`O`/`Rz`/`T`/`S`/`V`/`E`/`F`/`X`/`I`/`A`).
///
/// Rationale: the mangle scheme collapses `.` and `-` to
/// `_` so `Sample.Color` and `Sample_Color` both encode
/// to `Sample_Color`. The decoder can't distinguish
/// them. But it CAN tell that `My_Kv_Pair_u32_str` has
/// `u32` and `str` segments which are primitive mangles
/// — i.e. it's a generic instantiation, not a plain FQN.
/// Defer those to RC.3.
///
/// False positives possible if a user names a Lean type
/// `Foo.u32` or `bar_str_quux` etc. — they get treated
/// as generic instantiations and return `None` → falls
/// back to stub-panic. Same observable as the pre-RC.2
/// behaviour; users rename or wait for RC.3's tokeniser.
fn mangle_segment_is_plain_fqn(rest: &str) -> bool {
    if rest.is_empty() {
        return false;
    }
    for segment in rest.split('_') {
        if segment.is_empty() {
            return false;
        }
        // Reject if segment is a primitive mangle token.
        if matches!(
            segment,
            "u8" | "u16" | "u32" | "u64"
            | "i8" | "i16" | "i32" | "i64"
            | "f32" | "f64"
            | "b" | "c" | "str" | "bI" | "bN"
            | "self"
            // Composite-mangle terminator letters at
            // standalone segment position would only
            // appear inside a generic-arg list (e.g.
            // `S_Foo_L_u32_l_s` → segments after `Foo`
            // are `L`/`u32`/`l`).
            | "l" | "o" | "z" | "t" | "s" | "v" | "e"
            | "f" | "x" | "i" | "a"
            | "r"
        ) {
            return false;
        }
        // First char of any FQN segment must be a letter
        // (Lean ident grammar) — never a digit. Mangle
        // tokens like `c0c` (Cyc(0)) get rejected here.
        let first = segment.chars().next().unwrap();
        if !first.is_alphabetic() {
            return false;
        }
    }
    true
}

/// Lean identifiers borrow Rust's lexical set, but a tiny set of
/// keywords needs guarding. We rename only the literal collisions;
/// anything else passes through.
fn lean_safe_ident(s: &str) -> String {
    matches!(
        s,
        "def" | "let" | "fun" | "match" | "if" | "then" | "else" |
        "do" | "open" | "namespace" | "section" | "end" | "import" |
        "where" | "with" | "by" | "instance" | "structure" | "inductive"
    )
    .then(|| format!("`{s}`"))
    .unwrap_or_else(|| s.to_string())
}

// ─── handshake JSON ──────────────────────────────────────────────────

fn build_handshake_json(
    pkg: &str,
    iface: &str,
    cdylib: &Path,
    schema_hash: &str,
    rust_toolchain: &str,
    entries: &[EntryView],
) -> String {
    let now = current_utc_iso();
    let exports: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "logical_name": e.logical_name,
                "mangled": e.mangled,
                "param_types": e.param_types,
                "ret_type": e.ret_type,
                "isolated": e.isolated,
                "abi_version": e.abi_version,
            })
        })
        .collect();
    let v = serde_json::json!({
        "leo4_rust_handshake_version": 1,
        "schema_hash": schema_hash,
        "abi_version": 1,
        "package": pkg,
        "interface": iface,
        "cdylib_path": cdylib.display().to_string(),
        "rust_toolchain": rust_toolchain,
        "leo4_rust_emit_version": env!("CARGO_PKG_VERSION"),
        "emitted_at": now,
        "exports": exports,
    });
    serde_json::to_string_pretty(&v).expect("serde_json::to_string_pretty on a Value")
}

fn current_utc_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // No chrono dependency — format manually as RFC 3339 in UTC.
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (year, month, day, hour, minute, second) = secs_to_ymdhms(secs);
    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    )
}

fn secs_to_ymdhms(mut secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let second = (secs % 60) as u32;
    secs /= 60;
    let minute = (secs % 60) as u32;
    secs /= 60;
    let hour = (secs % 24) as u32;
    secs /= 24;
    // Days since 1970-01-01.
    let mut days = secs as i64;
    let mut year: i32 = 1970;
    loop {
        let dy = if is_leap_year(year) { 366 } else { 365 };
        if days < dy {
            break;
        }
        days -= dy;
        year += 1;
    }
    let months_len = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u32;
    for &m in &months_len {
        if days < m {
            break;
        }
        days -= m;
        month += 1;
    }
    let day = (days + 1) as u32;
    #[allow(clippy::cast_sign_loss)] // year >= 1970
    (year as u32, month, day, hour, minute, second)
}

fn is_leap_year(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

// ─── filesystem helpers ─────────────────────────────────────────────

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)
            .map_err(|e| format!("create {tmp:?}: {e}"))?;
        f.write_all(bytes)
            .map_err(|e| format!("write {tmp:?}: {e}"))?;
        f.sync_all()
            .map_err(|e| format!("fsync {tmp:?}: {e}"))?;
    }
    fs::rename(&tmp, path)
        .map_err(|e| format!("rename {tmp:?} -> {path:?}: {e}"))?;
    Ok(())
}

fn derive_pkg_name(cdylib: &Path) -> String {
    let stem = cdylib
        .file_stem().map_or_else(|| "exports".into(), |s| s.to_string_lossy().into_owned());
    stem.strip_prefix("lib").map(str::to_owned).unwrap_or(stem)
}

fn camel_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut cap_next = true;
    for ch in s.chars() {
        if ch == '-' || ch == '_' || ch == ' ' {
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
        "Iface".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_hash_is_stable_13char_base32lc() {
        let h = compute_schema_hash("package leo4-sample; interface Sample {  }");
        assert_eq!(h.len(), 13);
        assert!(h.chars().all(|c| BASE32_LC.contains(&(c as u8))));
    }

    #[test]
    fn schema_hash_changes_on_input_change() {
        let a = compute_schema_hash("package foo;");
        let b = compute_schema_hash("package bar;");
        assert_ne!(a, b);
    }

    #[test]
    fn parse_mangle_scalars() {
        use schema_idl::IDLType::*;
        assert!(matches!(parse_mangle("u64").unwrap().0, U64));
        assert!(matches!(parse_mangle("str").unwrap().0, String));
        assert!(matches!(parse_mangle("bN").unwrap().0, BigNat));
    }

    #[test]
    fn parse_mangle_list_of_u32() {
        let (t, _) = parse_mangle("L_u32_l").unwrap();
        let surface = idl_to_surface(&t);
        assert_eq!(surface, "list<u32>");
    }

    #[test]
    fn derive_pkg_strips_lib_prefix() {
        assert_eq!(
            derive_pkg_name(Path::new("/tmp/target/libfoo_bar.so")),
            "foo_bar"
        );
        // No `lib` prefix → returned verbatim.
        assert_eq!(derive_pkg_name(Path::new("/x/foo.dll")), "foo");
        assert_eq!(derive_pkg_name(Path::new("/x/libcvc5.dylib")), "cvc5");
    }

    #[test]
    fn camel_case_basic() {
        assert_eq!(camel_case("foo-bar"), "FooBar");
        assert_eq!(camel_case("solver"), "Solver");
        assert_eq!(camel_case(""), "Iface");
    }

    // ─── Lean wrapper emission (9-5) ─────────────────────────────

    #[test]
    fn lean_type_of_mangle_scalars() {
        assert_eq!(lean_type_of_mangle("u64").as_deref(), Some("UInt64"));
        assert_eq!(lean_type_of_mangle("i32").as_deref(), Some("Int32"));
        assert_eq!(lean_type_of_mangle("f64").as_deref(), Some("Float"));
        assert_eq!(lean_type_of_mangle("b").as_deref(), Some("Bool"));
        assert_eq!(lean_type_of_mangle("str").as_deref(), Some("String"));
        assert_eq!(lean_type_of_mangle("bN").as_deref(), Some("Nat"));
        assert_eq!(lean_type_of_mangle("bI").as_deref(), Some("Int"));
    }

    #[test]
    fn lean_type_of_mangle_list_bytes_is_bytearray() {
        assert_eq!(
            lean_type_of_mangle("L_u8_l").as_deref(),
            Some("ByteArray")
        );
        assert_eq!(
            lean_type_of_mangle("L_u32_l").as_deref(),
            Some("Array UInt32")
        );
        assert_eq!(
            lean_type_of_mangle("O_str_o").as_deref(),
            Some("Option String")
        );
    }

    #[test]
    fn lean_type_of_mangle_unknown_returns_none() {
        assert_eq!(lean_type_of_mangle("garbage"), None);
        // RC.2 patch 1 lifted the `S_<fqn>_s` arm — see
        // `lean_type_of_mangle_user_defined_*` tests below.
    }

    // ─── RC.2 patch 1 — user-defined nominal types ───
    // 2026-05-31. `lean_type_of_mangle` now decodes the
    // five user-defined-nominal mangle prefixes
    // (`S_<fqn>_s`, `V_<fqn>_v`, `E_<fqn>_e`,
    // `F_<fqn>_f`, `X_<fqn>_x`) to the underscored FQN
    // form. Generic instantiations stay `None`-fallback
    // (defer to RC.3 tokeniser).

    #[test]
    fn lean_type_of_mangle_user_defined_record_no_generics() {
        // `structure Point where x : UInt32, y : UInt32`
        // → mangle `S_Point_s` → Lean type `Point`.
        assert_eq!(
            lean_type_of_mangle("S_Point_s").as_deref(),
            Some("Point"),
        );
    }

    #[test]
    fn lean_type_of_mangle_user_defined_variant_no_generics() {
        // The flagship typed-enum case the RC.2 patch
        // exists for: `enum AdsmtVerdict { Sat { … },
        // Unsat { … }, Abductive { … }, Unknown { … } }`
        // → mangle `V_AdsmtVerdict_v` → Lean type
        // `AdsmtVerdict`. The matching `inductive`
        // decl lands via Patch 2.
        assert_eq!(
            lean_type_of_mangle("V_AdsmtVerdict_v").as_deref(),
            Some("AdsmtVerdict"),
        );
    }

    #[test]
    fn lean_type_of_mangle_user_defined_namespaced_fqn() {
        // Lean FQN `Sample.Color` mangles to
        // `E_Sample_Color_e` (dots → underscores).
        // Decoder returns `Sample_Color` (underscored;
        // round-trip dot-restoration is lossy). Users
        // expecting `Sample.Color` need to either rename
        // their Lean type or wait for the RC.3 mangle
        // tokeniser.
        assert_eq!(
            lean_type_of_mangle("E_Sample_Color_e").as_deref(),
            Some("Sample_Color"),
        );
    }

    #[test]
    fn lean_type_of_mangle_user_defined_flags() {
        assert_eq!(
            lean_type_of_mangle("F_Perms_f").as_deref(),
            Some("Perms"),
        );
    }

    #[test]
    fn lean_type_of_mangle_user_defined_resource() {
        assert_eq!(
            lean_type_of_mangle("X_ParserHandle_x").as_deref(),
            Some("ParserHandle"),
        );
    }

    #[test]
    fn lean_type_of_mangle_generic_instantiation_returns_none_without_known_fqns() {
        // Without `known_fqns`, generic instantiations still
        // fall to the heuristic path which rejects on
        // embedded primitive-mangle tokens. Backward-compat
        // with the RC.2-era behaviour.
        assert_eq!(
            lean_type_of_mangle("S_My_Kv_Pair_u32_str_s"),
            None,
        );
        assert_eq!(
            lean_type_of_mangle("V_Result2_u32_v"),
            None,
        );
    }

    // ─── RC.6 F3 — generic-instantiation tokeniser ─────

    #[test]
    fn lean_type_of_mangle_generic_record_resolves_with_known_fqns() {
        // `My.Kv.Pair<u32, String>` mangles to
        // `S_My_Kv_Pair_u32_str_s`. With `known_fqns=["My_Kv_Pair"]`
        // (the `fqn_seg`-normalised form), the tokeniser
        // splits the FQN from the generic args and emits
        // `My_Kv_Pair (UInt32) (String)`.
        let known = vec!["My_Kv_Pair".to_string()];
        assert_eq!(
            lean_type_of_mangle_with_user_fqns(
                "S_My_Kv_Pair_u32_str_s",
                &known,
            )
            .as_deref(),
            Some("My_Kv_Pair (UInt32) (String)"),
        );
    }

    #[test]
    fn lean_type_of_mangle_generic_variant_resolves_with_known_fqns() {
        let known = vec!["Result2".to_string()];
        assert_eq!(
            lean_type_of_mangle_with_user_fqns(
                "V_Result2_u32_v",
                &known,
            )
            .as_deref(),
            Some("Result2 (UInt32)"),
        );
    }

    #[test]
    fn lean_type_of_mangle_no_generics_resolves_with_known_fqns_exact_match() {
        // No-generics case (`S_AdsmtVerdict_s`) — exact match
        // in known_fqns wins, no args.
        let known = vec!["AdsmtVerdict".to_string()];
        assert_eq!(
            lean_type_of_mangle_with_user_fqns(
                "S_AdsmtVerdict_s",
                &known,
            )
            .as_deref(),
            Some("AdsmtVerdict"),
        );
        assert_eq!(
            lean_type_of_mangle_with_user_fqns(
                "V_AdsmtVerdict_v",
                &known,
            )
            .as_deref(),
            Some("AdsmtVerdict"),
        );
    }

    #[test]
    fn lean_type_of_mangle_nested_generic_in_list_resolves() {
        // `Vec<My_Pair<u32, String>>` → `L_S_My_Pair_u32_str_s_l`.
        // Outer is List → recurse on inner; inner is the
        // generic record. Result: `Array (My_Pair (UInt32)
        // (String))` — outer paren added by
        // `paren_if_multi_token` since inner has spaces.
        let known = vec!["My_Pair".to_string()];
        assert_eq!(
            lean_type_of_mangle_with_user_fqns(
                "L_S_My_Pair_u32_str_s_l",
                &known,
            )
            .as_deref(),
            Some("Array (My_Pair (UInt32) (String))"),
        );
    }

    #[test]
    fn lean_type_of_mangle_greedy_longest_match_wins() {
        // `S_My_Pair_u32_s` with both `My` and `My_Pair` in
        // known_fqns — longest match wins: `My_Pair (UInt32)`,
        // not `My _Pair_u32` (the latter would fail to
        // tokenise).
        let known = vec!["My".to_string(), "My_Pair".to_string()];
        assert_eq!(
            lean_type_of_mangle_with_user_fqns(
                "S_My_Pair_u32_s",
                &known,
            )
            .as_deref(),
            Some("My_Pair (UInt32)"),
        );
    }

    #[test]
    fn lean_type_of_mangle_unknown_generic_falls_back_to_none() {
        // Even with the tokeniser, unknown user types are
        // not resolved without `known_fqns` entry.
        let known = vec!["OtherType".to_string()];
        assert_eq!(
            lean_type_of_mangle_with_user_fqns(
                "S_My_Pair_u32_str_s",
                &known,
            ),
            None,
        );
    }

    #[test]
    fn tokenise_arg_list_splits_primitives() {
        let r = tokenise_arg_list("u32_str", &[]).unwrap();
        assert_eq!(r, vec!["u32".to_string(), "str".to_string()]);
        let r = tokenise_arg_list("u8_u16_u32_u64", &[]).unwrap();
        assert_eq!(r, vec!["u8", "u16", "u32", "u64"]);
    }

    #[test]
    fn tokenise_arg_list_splits_nested_composites() {
        // `L_u8_l_str` → two args: `L_u8_l` and `str`.
        let r = tokenise_arg_list("L_u8_l_str", &[]).unwrap();
        assert_eq!(r, vec!["L_u8_l".to_string(), "str".to_string()]);
    }

    #[test]
    fn paren_if_multi_token_wraps_only_apps() {
        assert_eq!(paren_if_multi_token("UInt32"), "UInt32");
        assert_eq!(paren_if_multi_token("AdsmtVerdict"), "AdsmtVerdict");
        assert_eq!(
            paren_if_multi_token("My_Pair (UInt32) (String)"),
            "(My_Pair (UInt32) (String))",
        );
    }

    #[test]
    fn lean_type_of_mangle_segment_starting_with_digit_returns_none() {
        // Lean idents can't start with a digit. A mangle
        // like `S_1Foo_s` violates the heuristic → None.
        assert_eq!(lean_type_of_mangle("S_1Foo_s"), None);
    }


    #[test]
    fn lean_safe_ident_avoids_keywords() {
        assert_eq!(lean_safe_ident("add"), "add");
        assert_eq!(lean_safe_ident("solve_smt"), "solve_smt");
        assert_eq!(lean_safe_ident("end"), "`end`");
        assert_eq!(lean_safe_ident("match"), "`match`");
    }

    #[test]
    fn render_one_export_emits_typed_wrapper() {
        let e = EntryView {
            logical_name: "add".into(),
            mangled: "leo4_rust__add__u64_u64".into(),
            param_types: vec!["u64".into(), "u64".into()],
            ret_type: "u64".into(),
            isolated: false,
            abi_version: 1,
        };
        let s = render_one_export(&e, &[]).unwrap();
        assert!(s.contains("def add (a0 : UInt64) (a1 : UInt64) : IO (UInt64)"));
        assert!(s.contains("leo4_rust__add__u64_u64"));
        assert!(s.contains("canonicalEncode a0"));
        assert!(s.contains("canonicalEncode a1"));
        assert!(s.contains("UInt64"));
    }

    #[test]
    fn render_one_export_stubs_unmapped_signature() {
        // RC.2 patch 1 (2026-05-31): `S_Sample_Point_s`
        // now maps to a Lean type (`Sample_Point`); pick a
        // mangle that genuinely doesn't decode (generic
        // instantiation — RC.3 tokeniser territory) so
        // the stub-panic path still gets coverage.
        let e = EntryView {
            logical_name: "solve".into(),
            mangled: "leo4_rust__solve__S_My_Pair_u32_str_s".into(),
            param_types: vec!["S_My_Pair_u32_str_s".into()],
            ret_type: "u32".into(),
            isolated: false,
            abi_version: 1,
        };
        let s = render_one_export(&e, &[]).unwrap();
        // Wrapper signature is still emitted, but the body
        // panic!s rather than dispatching with garbage.
        assert!(s.contains("def solve"), "wrapper definition missing: {s}");
        assert!(s.contains("panic!"), "unmapped wrapper must panic!: {s}");
    }

    #[test]
    fn render_lean_wrapper_includes_schema_hash_and_extern() {
        let entries = vec![EntryView {
            logical_name: "ping".into(),
            mangled: "leo4_rust__ping".into(),
            param_types: vec![],
            ret_type: String::new(),
            isolated: false,
            abi_version: 1,
        }];
        let s = render_lean_wrapper("My.Rust", "my", "My", "abcdefghijklm", &entries, &[]).unwrap();
        assert!(s.contains("namespace My.Rust"));
        assert!(s.contains("def schemaHash : String := \"abcdefghijklm\""));
        assert!(s.contains("@[extern \"leo4_rust_call_lean\"]"));
        // New signature: IO ByteArray (status prefix decoded by
        // the typed wrapper).
        assert!(s.contains(": IO ByteArray"));
        assert!(s.contains("private def decodeStatus"));
        assert!(s.contains("def ping") && s.contains("IO (Unit)"));
        assert!(s.contains("end My.Rust"));
    }

    // ─── RC.2 patch 2 — mirror decl emit ───────────────

    #[test]
    fn rust_type_to_lean_type_scalars() {
        assert_eq!(rust_type_to_lean_type("u8"), "UInt8");
        assert_eq!(rust_type_to_lean_type("u64"), "UInt64");
        assert_eq!(rust_type_to_lean_type("i32"), "Int32");
        assert_eq!(rust_type_to_lean_type("f64"), "Float");
        assert_eq!(rust_type_to_lean_type("bool"), "Bool");
        assert_eq!(rust_type_to_lean_type("char"), "Char");
        assert_eq!(rust_type_to_lean_type("String"), "String");
        assert_eq!(rust_type_to_lean_type("()"), "Unit");
    }

    #[test]
    fn rust_type_to_lean_type_vec_special_cases() {
        // Vec<u8> → ByteArray (special-cased for the common
        // binary-blob pattern that wire-decodes faster as
        // ByteArray than via Array UInt8).
        assert_eq!(rust_type_to_lean_type("Vec<u8>"), "ByteArray");
        // Vec<T> for general T → Array T.
        assert_eq!(rust_type_to_lean_type("Vec<u32>"), "Array (UInt32)");
        assert_eq!(rust_type_to_lean_type("Vec<String>"), "Array (String)");
    }

    #[test]
    fn rust_type_to_lean_type_option_result_box() {
        assert_eq!(
            rust_type_to_lean_type("Option<u64>"),
            "Option (UInt64)"
        );
        assert_eq!(
            rust_type_to_lean_type("Result<u32,String>"),
            "Except (String) (UInt32)"
        );
        // Box is transparent at the boundary.
        assert_eq!(rust_type_to_lean_type("Box<u64>"), "UInt64");
    }

    #[test]
    fn rust_type_to_lean_type_tuples_right_assoc() {
        assert_eq!(
            rust_type_to_lean_type("(u32,u64)"),
            "(UInt32 × UInt64)"
        );
        assert_eq!(
            rust_type_to_lean_type("(u32,u64,String)"),
            "(UInt32 × (UInt64 × String))"
        );
        // The flagship nested case from the user's
        // AdsmtVerdict.Sat — Vec<(String, String)>.
        assert_eq!(
            rust_type_to_lean_type("Vec<(String,String)>"),
            "Array ((String × String))"
        );
    }

    #[test]
    fn rust_type_to_lean_type_user_defined_passes_through() {
        // Unknown idents are assumed to be user-defined
        // nominal types — the wrapper's mirror block puts
        // them in scope.
        assert_eq!(
            rust_type_to_lean_type("AdsmtVerdict"),
            "AdsmtVerdict"
        );
        assert_eq!(
            rust_type_to_lean_type("Vec<AdsmtVerdict>"),
            "Array (AdsmtVerdict)"
        );
        // Generic user-types apply args as Lean App-style.
        assert_eq!(
            rust_type_to_lean_type("Pair<u32,String>"),
            "Pair (UInt32) (String)"
        );
    }

    #[test]
    fn render_unit_enum_mirror_emits_inductive_with_deriving() {
        // C-style enum mirror.
        let ty = UserTypeView {
            fqn: "Color".into(),
            kind: UserTypeKind::UnitEnum,
            fields: vec![],
            ctors: vec![
                CtorView { name: "Red".into(), fields: vec![] },
                CtorView { name: "Green".into(), fields: vec![] },
                CtorView { name: "Blue".into(), fields: vec![] },
            ],
        };
        let s = render_one_user_type(&ty);
        assert!(s.contains("inductive Color where"));
        // Lowercase Lean ctor names.
        assert!(s.contains("| red : Color"));
        assert!(s.contains("| green : Color"));
        assert!(s.contains("| blue : Color"));
        assert!(s.contains("deriving Leo4.LeanMarshal"));
    }

    #[test]
    fn render_record_mirror_emits_structure_with_fields() {
        let ty = UserTypeView {
            fqn: "Point".into(),
            kind: UserTypeKind::Record,
            fields: vec![
                FieldView { name: "x".into(), type_mangle: "".into(), rust_type: "u32".into() },
                FieldView { name: "y".into(), type_mangle: "".into(), rust_type: "u32".into() },
            ],
            ctors: vec![],
        };
        let s = render_one_user_type(&ty);
        assert!(s.contains("structure Point where"));
        assert!(s.contains("x : UInt32"));
        assert!(s.contains("y : UInt32"));
        assert!(s.contains("deriving Leo4.LeanMarshal"));
    }

    #[test]
    fn render_variant_mirror_emits_full_typed_enum_for_adsmt_verdict() {
        // The flagship typed-enum case the RC.2 patches
        // exist for. End-to-end check that:
        // 1. Inductive is emitted with the right name.
        // 2. Each ctor lowercases the first char.
        // 3. Each named field gets its name as a binder.
        // 4. Nested Vec<(String,String)> translates to
        //    `Array ((String × String))`.
        // 5. Vec<AbductiveCandidate> translates to
        //    `Array (AbductiveCandidate)`.
        // 6. `deriving Leo4.LeanMarshal` ends the decl.
        let ty = UserTypeView {
            fqn: "AdsmtVerdict".into(),
            kind: UserTypeKind::Variant,
            fields: vec![],
            ctors: vec![
                CtorView {
                    name: "Sat".into(),
                    fields: vec![FieldView {
                        name: "model".into(),
                        type_mangle: "".into(),
                        rust_type: "Vec<(String,String)>".into(),
                    }],
                },
                CtorView {
                    name: "Unsat".into(),
                    fields: vec![
                        FieldView { name: "core".into(), type_mangle: "".into(), rust_type: "Vec<String>".into() },
                        FieldView { name: "cert".into(), type_mangle: "".into(), rust_type: "String".into() },
                    ],
                },
                CtorView {
                    name: "Abductive".into(),
                    fields: vec![FieldView {
                        name: "candidates".into(),
                        type_mangle: "".into(),
                        rust_type: "Vec<AbductiveCandidate>".into(),
                    }],
                },
                CtorView {
                    name: "Unknown".into(),
                    fields: vec![FieldView {
                        name: "reason".into(),
                        type_mangle: "".into(),
                        rust_type: "String".into(),
                    }],
                },
            ],
        };
        let s = render_one_user_type(&ty);
        assert!(s.contains("inductive AdsmtVerdict where"), "missing inductive header: {s}");
        // Sat with nested Vec<(String,String)>.
        assert!(
            s.contains("| sat (model : Array ((String × String))) : AdsmtVerdict"),
            "missing sat arm: {s}"
        );
        // Unsat with two named fields.
        assert!(
            s.contains("| unsat (core : Array (String)) (cert : String) : AdsmtVerdict"),
            "missing unsat arm: {s}"
        );
        // Abductive with Vec<AbductiveCandidate>.
        assert!(
            s.contains("| abductive (candidates : Array (AbductiveCandidate)) : AdsmtVerdict"),
            "missing abductive arm: {s}"
        );
        // Unknown with single String field.
        assert!(
            s.contains("| unknown (reason : String) : AdsmtVerdict"),
            "missing unknown arm: {s}"
        );
        assert!(s.contains("deriving Leo4.LeanMarshal"));
    }

    #[test]
    fn render_lean_wrapper_emits_mirror_block_for_typed_enum_export() {
        // End-to-end: an export with a typed-enum
        // parameter, along with the matching USER_TYPES
        // entry, round-trips through render_lean_wrapper.
        // The wrapper emits both the mirror inductive AND
        // the typed `def solve` wrapper signature.
        let entries = vec![EntryView {
            logical_name: "solve".into(),
            mangled: "leo4_rust__solve__V_AdsmtVerdict_v".into(),
            param_types: vec!["V_AdsmtVerdict_v".into()],
            ret_type: "str".into(),
            isolated: false,
            abi_version: 1,
        }];
        let user_types = vec![UserTypeView {
            fqn: "AdsmtVerdict".into(),
            kind: UserTypeKind::Variant,
            fields: vec![],
            ctors: vec![
                CtorView {
                    name: "Sat".into(),
                    fields: vec![FieldView {
                        name: "model".into(),
                        type_mangle: "".into(),
                        rust_type: "Vec<(String,String)>".into(),
                    }],
                },
                CtorView {
                    name: "Unknown".into(),
                    fields: vec![FieldView {
                        name: "reason".into(),
                        type_mangle: "".into(),
                        rust_type: "String".into(),
                    }],
                },
            ],
        }];
        let s = render_lean_wrapper(
            "My.Rust",
            "my",
            "My",
            "0123456789abc",
            &entries,
            &user_types,
        )
        .unwrap();
        // Mirror inductive present.
        assert!(s.contains("inductive AdsmtVerdict where"));
        assert!(s.contains("| sat (model : Array ((String × String))) : AdsmtVerdict"));
        assert!(s.contains("| unknown (reason : String) : AdsmtVerdict"));
        assert!(s.contains("deriving Leo4.LeanMarshal"));
        // Typed export signature.
        assert!(s.contains("def solve (a0 : AdsmtVerdict) : IO (String)"));
        // No stub panic — patch 1 mapped the variant.
        assert!(!s.contains("has an unmapped parameter"));
    }
}
