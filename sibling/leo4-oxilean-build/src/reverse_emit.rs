//! OX8.2 — reverse direction wrapper emit (RC.5 full-sync with
//! `crates/leo4-rust-emit/src/main.rs`'s RC.2 patches).
//!
//! Drives the rust-transpile path's reverse direction: a Rust
//! cdylib that re-exports `#[leo4::export]` functions via
//! `EXPORTS` (declared in `crates/leo4-abi/src/rust_exports.rs`)
//! becomes a `lean/<Iface>/Rust.lean` file full of
//! `@[extern "<mangled>"]` decls, callable from a Lean source
//! running under `sibling/leo4-oxilean`'s OxiLean evaluator.
//!
//! ## Why a different wrapper shape than mslean4 reverse?
//!
//! `crates/leo4-rust-emit/src/main.rs::render_lean_wrapper` emits
//! *dispatcher-based* wrappers: every Lean call goes through a
//! single `leo4_rust_call_lean` extern that multiplexes
//! mangled-name + canonical-ABI byte buffer. That works because
//! mslean4 has a Rust dispatcher binary (`leo4-rust-worker`)
//! eager-loaded behind the scenes.
//!
//! The rust-transpile path doesn't ship that worker — its design
//! goal is "no lake, no daemon, no separate dispatcher process".
//! So OX8 takes the simpler model: each export gets its *own*
//! `@[extern "<mangled>"]` decl that resolves directly to the
//! cdylib's C-linkage symbol. The OxiLean evaluator (OX8.3,
//! shipped on the fork branch `0.1.3-leo4-ox7`) handles the
//! `libloading` dispatch.
//!
//! ## RC.5 sync surface (2026-05-31)
//!
//! Brings this module to parity with
//! `crates/leo4-rust-emit/src/main.rs`'s RC.2 patches:
//!
//! - **Patch 1** — `lean_type_of_mangle` decodes the five
//!   user-defined-nominal mangle prefixes (`S_/V_/E_/F_/X_`)
//!   plus the `mangle_segment_is_plain_fqn` heuristic that
//!   rejects generic-instantiation mangles. Wrapper signatures
//!   that reference `AdsmtVerdict`-style user types now lower
//!   cleanly to the bare Lean fqn instead of an
//!   `/- unmapped -/` placeholder.
//! - **Patch 2** — `render_reverse_wrapper` takes a
//!   `&[UserTypeView]` alongside `&[ExportEntryView]` and emits a
//!   mirror-decl block (`structure` / `inductive` with
//!   `deriving Leo4.LeanMarshal`) for every user-defined nominal
//!   type the cdylib's `USER_TYPES` distributed slice carries.
//!   `rust_type_to_lean_type` (syn-based AST walk) translates
//!   the macro-emitted Rust source-text of each field to a Lean
//!   type expression. Zero hand-written Lean mirror code.
//!
//! ## Status
//!
//! OX8.2a (initial commit): lib-level renderer + `ExportEntryView`
//! decoupled from the live cdylib.
//! OX8.2b (CLI plumbing): `--mode reverse` flag in the binary
//! `src/bin/leo4-oxilean-build.rs`. RC.5 commit landed this; the
//! binary loads the cdylib via `libloading`, walks both
//! `EXPORTS` and `USER_TYPES` via the FFI introspection entries,
//! and feeds them into `render_reverse_wrapper`.

use leo4_abi::rust_exports::{
    ExportEntry, FieldEntry, UserTypeEntry, UserTypeKind,
};

/// Mirror of `leo4_abi::ExportEntry` (`crates/leo4-abi/src/
/// rust_exports.rs`) with `String` instead of `&'static str`, so
/// callers can decouple this module's tests from a live cdylib
/// load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportEntryView {
    /// User-visible name. Matches Rust `fn` ident.
    pub logical_name: String,
    /// C-linkage symbol the OxiLean evaluator resolves via
    /// `dlsym`. Format:
    /// `leo4_rust__<fname>__<param_mangles>`.
    pub mangled: String,
    /// Parameter type mangles in declaration order.
    pub param_types: Vec<String>,
    /// Return type mangle. Empty string = unit.
    pub ret_type: String,
    /// `#[leo4::export(isolated)]` — informational only;
    /// reverse-emit doesn't change shape based on it (the
    /// rust-transpile reverse runner picks dispatch behaviour
    /// at runtime).
    pub isolated: bool,
    /// ABI version this entry was emitted against. Currently
    /// always `1`.
    pub abi_version: u32,
}

// ─── RC.5 patch 2 — User-defined type schema views ─────────────

/// Borrowed view of a single `FieldEntry`, owned post-dlclose.
#[derive(Debug, Clone)]
pub struct FieldView {
    pub name: String,
    /// IDL-side mangle. Reserved for a future mangle-first
    /// translation path; today's wrapper emit reads
    /// `rust_type` exclusively (the macro emits empty
    /// `type_mangle`).
    #[allow(dead_code)]
    pub type_mangle: String,
    pub rust_type: String,
}

/// Borrowed view of a single `CtorEntry`, owned post-dlclose.
#[derive(Debug, Clone)]
pub struct CtorView {
    pub name: String,
    pub fields: Vec<FieldView>,
}

/// Borrowed view of a single `UserTypeEntry`, owned post-dlclose.
#[derive(Debug, Clone)]
pub struct UserTypeView {
    pub fqn: String,
    pub kind: UserTypeKind,
    pub fields: Vec<FieldView>,
    pub ctors: Vec<CtorView>,
}

/// Convert a borrowed `FieldEntry` from the cdylib to an
/// owned `FieldView`. Shared by the CLI cdylib loader.
pub fn view_field(f: &FieldEntry) -> FieldView {
    FieldView {
        name: f.name.to_owned(),
        type_mangle: f.type_mangle.to_owned(),
        rust_type: f.rust_type.to_owned(),
    }
}

/// Convert a borrowed `UserTypeEntry` from the cdylib to an
/// owned `UserTypeView`. Shared by the CLI cdylib loader.
pub fn view_user_type(e: &UserTypeEntry) -> UserTypeView {
    UserTypeView {
        fqn: e.fqn.to_owned(),
        kind: e.kind,
        fields: e.fields.iter().map(view_field).collect(),
        ctors: e
            .ctors
            .iter()
            .map(|c| CtorView {
                name: c.name.to_owned(),
                fields: c.fields.iter().map(view_field).collect(),
            })
            .collect(),
    }
}

/// Convert a borrowed `ExportEntry` to an owned `ExportEntryView`.
pub fn view_export(e: &ExportEntry) -> ExportEntryView {
    ExportEntryView {
        logical_name: e.logical_name.to_owned(),
        mangled: e.mangled.to_owned(),
        param_types: e.param_types.iter().map(|s| (*s).to_owned()).collect(),
        ret_type: e.ret_type.to_owned(),
        isolated: e.isolated,
        abi_version: e.abi_version,
    }
}

/// Render the full `lean/<Iface>/Rust.lean` wrapper from a list
/// of cdylib exports + the cdylib's `USER_TYPES` schema slice.
///
/// Each `@[extern "<mangled>"] opaque <name> (...) : ...` is a
/// typed binding to one Rust `#[leo4::export]` function. The
/// OxiLean evaluator at call time looks up the mangled symbol in
/// the configured cdylib via `libloading` and dispatches with the
/// canonical-ABI byte buffer.
///
/// `module` is the Lean namespace (typically `<Iface>.Rust`).
/// `user_types` carries one entry per `#[derive(LeanMarshal)]`
/// type the cdylib registers; the renderer emits one mirror Lean
/// declaration per entry (record → `structure`, variant /
/// unit-enum → `inductive`, all with `deriving Leo4.LeanMarshal`)
/// so the wrapper is self-contained — no user-side mirror module
/// required.
///
/// # Errors
/// Currently unused; reserved for future "strict mode" where
/// unmapped mangles abort. The current implementation passes
/// through unmapped mangles as `/- unmapped: <mangle> -/ String`
/// placeholders (matching `leo4-rust-emit`'s behaviour).
pub fn render_reverse_wrapper(
    module: &str,
    entries: &[ExportEntryView],
    user_types: &[UserTypeView],
) -> Result<String, String> {
    let mut s = String::new();
    s.push_str("-- Auto-generated by `leo4-oxilean-build --mode reverse`. Do not edit.\n");
    s.push_str("-- See SPEC/ox8-rust-transpile-reverse.md.\n");
    s.push_str("--\n");
    s.push_str("-- Each `#[leo4::export]` Rust function in the source\n");
    s.push_str("-- cdylib surfaces here as one `@[extern]` opaque. The\n");
    s.push_str("-- OxiLean evaluator (`sibling/leo4-oxilean`) resolves\n");
    s.push_str("-- the mangled symbol at call time via `libloading`.\n");
    s.push_str("--\n");
    s.push_str("-- User-defined nominal types referenced by the exports\n");
    s.push_str("-- get auto-emitted mirror declarations below (RC.5 sync\n");
    s.push_str("-- with `leo4-rust-emit`'s RC.2 patch 2 — synthesised\n");
    s.push_str("-- from the cdylib's `USER_TYPES` distributed slice).\n\n");

    s.push_str(&format!("namespace {module}\n\n"));

    // RC.5 (2026-05-31) — emit mirror Lean declarations for every
    // user-defined nominal type the cdylib's `USER_TYPES` slice
    // carries. Lands above the `@[extern]` decls so the wrappers
    // can reference the names below.
    if !user_types.is_empty() {
        s.push_str(&render_user_type_mirror_block(user_types));
    }

    // RC.6 F3 (2026-05-31) — extract the user-type fqn list
    // so the mangle decoder resolves generic-instantiation
    // mangles via `known_fqns` lookup.
    let known_fqns: Vec<String> =
        user_types.iter().map(|t| t.fqn.clone()).collect();

    let mut sorted: Vec<&ExportEntryView> = entries.iter().collect();
    sorted.sort_by(|a, b| a.logical_name.cmp(&b.logical_name));
    for e in sorted {
        s.push_str(&render_one_extern(e, &known_fqns));
        s.push('\n');
    }

    s.push_str(&format!("end {module}\n"));
    Ok(s)
}

/// Render one `@[extern] opaque` per export. The signature is
/// reconstructed from the entry's `param_types` / `ret_type`
/// mangles via `lean_type_of_mangle_with_user_fqns`.
fn render_one_extern(e: &ExportEntryView, known_fqns: &[String]) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "/-- `{}` (mangled: `{}`). -/\n",
        e.logical_name, e.mangled,
    ));
    s.push_str(&format!("@[extern \"{}\"]\n", e.mangled));

    let fname = lean_safe_ident(&e.logical_name);
    s.push_str(&format!("opaque {fname}"));
    for (i, m) in e.param_types.iter().enumerate() {
        let lean_ty = lean_type_of_mangle_with_user_fqns(m, known_fqns)
            .unwrap_or_else(|| format!("/- unmapped: {m} -/ String"));
        s.push_str(&format!(" (a{i} : {lean_ty})"));
    }
    let ret_ty = if e.ret_type.is_empty() {
        "Unit".to_string()
    } else {
        lean_type_of_mangle_with_user_fqns(&e.ret_type, known_fqns)
            .unwrap_or_else(|| format!("/- unmapped: {} -/ Unit", e.ret_type))
    };
    s.push_str(&format!(" : {ret_ty}\n"));
    s
}

// ─── RC.5 patch 1 — `lean_type_of_mangle` user-defined arms ─────

/// Backward-compat wrapper — calls
/// [`lean_type_of_mangle_with_user_fqns`] with empty fqns.
pub fn lean_type_of_mangle(mangle: &str) -> Option<String> {
    lean_type_of_mangle_with_user_fqns(mangle, &[])
}

/// Mirror of `crates/leo4-rust-emit/src/main.rs::lean_type_of_mangle_with_user_fqns`.
/// Duplicated here rather than depending on `leo4-rust-emit`
/// because `leo4-oxilean-build` is a standalone sibling crate
/// (not a workspace member) and pulling in the `leo4-rust-emit`
/// crate would drag the full main-workspace dep graph in.
///
/// Recognises:
///
/// - Scalars (`u8`..`f64`, `bool`, `char`, `String`, `Int`, `Nat`).
/// - `L_<inner>_l` → `Array <inner>`; `L_u8_l` → `ByteArray`.
/// - `O_<inner>_o` → `Option <inner>`.
/// - **RC.5 (2026-05-31)** — `S_<fqn>_s` (record), `V_<fqn>_v`
///   (variant), `E_<fqn>_e` (enum), `F_<fqn>_f` (flags), `X_<fqn>_x`
///   (resource) decode to the bare fqn name (underscored).
/// - **RC.6 F3 (2026-05-31)** — generic instantiations like
///   `S_My_Pair_u32_str_s` resolve when a matching FQN is in
///   `known_fqns`, via greedy longest-match tokeniser. Without
///   `known_fqns` the heuristic-only RC.5 behaviour applies.
pub fn lean_type_of_mangle_with_user_fqns(
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
            // RC.6 F3 — user-defined nominal mangle decoder
            // with `known_fqns`-driven generic-instantiation
            // resolution.
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
/// when it contains a space (App). Single-token types pass
/// through unwrapped to preserve the RC.5-era `Array UInt32`
/// / `Option String` rendering.
fn paren_if_multi_token(t: &str) -> String {
    if t.contains(' ') {
        format!("({t})")
    } else {
        t.to_string()
    }
}

/// RC.6 F3 helper — split a nominal-kind mangle's middle
/// segment into (fqn, [arg_types]). Greedy longest-match
/// against `known_fqns`; falls back to the plain-fqn
/// heuristic. Mirror of `leo4-rust-emit::decode_nominal_with_args`.
fn decode_nominal_with_args(
    rest: &str,
    known_fqns: &[String],
) -> Option<String> {
    if known_fqns.iter().any(|f| f == rest) {
        return Some(rest.to_string());
    }
    let mut sorted_fqns: Vec<&String> = known_fqns.iter().collect();
    sorted_fqns.sort_by_key(|f| std::cmp::Reverse(f.len()));
    for fqn in &sorted_fqns {
        if let Some(args_rest) = rest.strip_prefix(fqn.as_str())
            && let Some(args_str) = args_rest.strip_prefix('_')
        {
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
    if mangle_segment_is_plain_fqn(rest) {
        return Some(rest.to_string());
    }
    None
}

/// RC.6 F3 helper — split an underscore-joined arg list into
/// individual top-level arg mangles. Mirror of
/// `leo4-rust-emit::tokenise_arg_list`.
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
        remaining = rest.strip_prefix('_')?;
    }
    Some(out)
}

/// Take one arg mangle off the front. Mirror of
/// `leo4-rust-emit::take_one_arg_mangle`.
fn take_one_arg_mangle<'a>(
    s: &'a str,
    known_fqns: &[String],
) -> Option<(&'a str, &'a str)> {
    for p in &[
        "self", "bI", "bN", "u128", "u64", "u32", "u16", "u8",
        "i128", "i64", "i32", "i16", "i8", "f64", "f32", "str",
        "b", "c",
    ] {
        if let Some(rest) = s.strip_prefix(p) {
            if rest.is_empty() || rest.starts_with('_') {
                return Some((&s[..p.len()], rest));
            }
        }
    }
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
    for fqn in known_fqns {
        if let Some(rest) = s.strip_prefix(fqn.as_str()) {
            if rest.is_empty() || rest.starts_with('_') {
                return Some((&s[..fqn.len()], rest));
            }
        }
    }
    None
}

/// Find the byte index where the matching outer suffix
/// closes. Balances nested kind prefix/suffix pairs.
fn find_matching_suffix(
    s: &str,
    _prefix: &str,
    suffix: &str,
) -> Option<usize> {
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
        let mut matched_close = None;
        for (_, suf) in &openers {
            if tail.starts_with(suf) {
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
        i += s[i..].chars().next()?.len_utf8();
    }
    None
}

/// RC.5 patch 1 helper — same heuristic as `leo4-rust-emit`'s
/// `mangle_segment_is_plain_fqn`. Returns `true` only when every
/// underscore-separated segment is a plain alphabetic Lean ident
/// segment AND none collides with a primitive mangle token or a
/// composite mangle prefix letter. Rejects generic instantiations
/// (`S_My_Pair_u32_str_s` — the `u32` / `str` segments give it
/// away); those defer to a future full mangle tokeniser.
fn mangle_segment_is_plain_fqn(rest: &str) -> bool {
    if rest.is_empty() {
        return false;
    }
    for segment in rest.split('_') {
        if segment.is_empty() {
            return false;
        }
        if matches!(
            segment,
            "u8" | "u16" | "u32" | "u64"
            | "i8" | "i16" | "i32" | "i64"
            | "f32" | "f64"
            | "b" | "c" | "str" | "bI" | "bN"
            | "self"
            | "l" | "o" | "z" | "t" | "s" | "v" | "e"
            | "f" | "x" | "i" | "a"
            | "r"
        ) {
            return false;
        }
        let first = segment.chars().next().unwrap();
        if !first.is_alphabetic() {
            return false;
        }
    }
    true
}

// ─── RC.5 patch 2 — Mirror Lean decl emit ──────────────────────

/// Emit the mirror Lean declaration block at the top of the
/// wrapper namespace. One `structure` / `inductive` per
/// user-defined type, in declaration order preserved from
/// `USER_TYPES` (linkme order — stable across re-runs).
fn render_user_type_mirror_block(types: &[UserTypeView]) -> String {
    let mut s = String::new();
    s.push_str("/-- ── User-defined nominal types ─────────────────\n");
    s.push_str("    Mirror declarations for every `#[derive(LeanMarshal)]`\n");
    s.push_str("    type referenced by this wrapper's exports.\n");
    s.push_str("    Auto-synthesised from the cdylib's `USER_TYPES`\n");
    s.push_str("    distributed slice (RC.5 sync with leo4-rust-emit's\n");
    s.push_str("    RC.2 patch 2, 2026-05-31). The `deriving\n");
    s.push_str("    Leo4.LeanMarshal` clause on each decl produces a\n");
    s.push_str("    wire-format-equivalent `LeanMarshal` instance\n");
    s.push_str("    matching the Rust side. -/\n\n");
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
    s.push_str("deriving Leo4.LeanMarshal\n");
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
    s.push_str("deriving Leo4.LeanMarshal\n");
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
    s.push_str("deriving Leo4.LeanMarshal\n");
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
    s.push_str("deriving Leo4.LeanMarshal\n");
    s
}

fn render_unit_struct_mirror(ty: &UserTypeView) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "/-- Mirror of Rust `#[derive(LeanMarshal)] struct {0};` (unit struct). -/\n",
        ty.fqn
    ));
    s.push_str(&format!("structure {} where\n", ty.fqn));
    s.push_str("deriving Leo4.LeanMarshal\n");
    s
}

/// Lowercase the first ASCII alphabetic char in `s`. Maps Rust
/// ctor idents (`Sat`, `Unsat`) to Lean ctor idents (`sat`,
/// `unsat` — Lean's stdlib convention for inductive ctor names).
fn lowercase_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_ascii_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

// ─── RC.5 patch 2 — Rust source-text → Lean type translator ────

/// Translate a Rust source-text type (whitespace-normalised by
/// the `#[derive(LeanMarshal)]` macro, e.g. `"Vec<(String,String)>"`)
/// into a Lean type expression (`"Array (String × String)"`).
/// syn-based AST walk.
///
/// Recognised Rust types:
///
/// - Primitives: `u8`..`u128` → `UInt8`..`UInt128`, `i8`..`i128`
///   → `Int8`..`Int128`, `f32`/`f64` → `Float32`/`Float`, `bool`
///   → `Bool`, `char` → `Char`, `()` → `Unit`.
/// - Strings: `String` → `String`, `&str` / `&'static str` →
///   `String` (Lean has no borrowed-string type at the boundary).
/// - Collections: `Vec<u8>` → `ByteArray`, `Vec<T>` → `Array T`,
///   `Option<T>` → `Option T`, `Result<T, E>` → `Except E T`.
/// - Tuples: `(A, B)` → `A × B`, `(A, B, C)` → `A × (B × C)`
///   (right-associative).
/// - BigInt / BigNat: `BigInt` → `Int`, `BigNat` → `Nat`.
/// - Box / reference: `Box<T>` → `T` (transparent at boundary).
/// - Unknown idents pass through verbatim — assume user-defined
///   nominal type whose mirror decl lives in the same wrapper
///   file.
pub fn rust_type_to_lean_type(rust_src: &str) -> String {
    match syn::parse_str::<syn::Type>(rust_src) {
        Ok(ty) => translate_syn_type(&ty),
        Err(_) => rust_src.to_string(),
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
            let parts: Vec<String> =
                tt.elems.iter().map(translate_syn_type).collect();
            translate_tuple_right_assoc(&parts)
        }
        Type::Reference(tr) => translate_syn_type(&tr.elem),
        Type::Paren(tp) => format!("({})", translate_syn_type(&tp.elem)),
        Type::Array(ta) => format!("Array {}", translate_syn_type(&ta.elem)),
        Type::Slice(ts) => format!("Array {}", translate_syn_type(&ts.elem)),
        _ => quote::ToTokens::to_token_stream(ty).to_string(),
    }
}

fn translate_type_path(tp: &syn::TypePath) -> String {
    let seg = match tp.path.segments.last() {
        Some(s) => s,
        None => return quote::ToTokens::to_token_stream(tp).to_string(),
    };
    let name = seg.ident.to_string();
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
            let t = translate_syn_type(args[0]);
            let e = translate_syn_type(args[1]);
            format!("Except ({e}) ({t})")
        }
        ("Box", 1) => translate_syn_type(args[0]),
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

/// Avoid Lean keyword collisions. Anything not in the list passes
/// through unchanged — Lean identifiers otherwise overlap Rust's
/// lexical set.
fn lean_safe_ident(s: &str) -> String {
    if matches!(
        s,
        "def" | "let" | "fun" | "match" | "if" | "then" | "else" |
        "do" | "open" | "namespace" | "section" | "end" | "import" |
        "where" | "with" | "by" | "instance" | "structure" | "inductive"
    ) {
        format!("`{s}`")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_entry(name: &str, mangled: &str, params: &[&str], ret: &str) -> ExportEntryView {
        ExportEntryView {
            logical_name: name.to_string(),
            mangled: mangled.to_string(),
            param_types: params.iter().map(|s| s.to_string()).collect(),
            ret_type: ret.to_string(),
            isolated: false,
            abi_version: 1,
        }
    }

    #[test]
    fn render_two_arith_exports_emits_namespace_and_decls() {
        let entries = vec![
            mk_entry("add", "leo4_rust__add__u64_u64", &["u64", "u64"], "u64"),
            mk_entry("dbl", "leo4_rust__dbl__u64", &["u64"], "u64"),
        ];
        let out = render_reverse_wrapper("Sample.Rust", &entries, &[]).unwrap();
        assert!(out.contains("namespace Sample.Rust\n"));
        assert!(out.contains("end Sample.Rust\n"));
        assert!(out.contains("@[extern \"leo4_rust__add__u64_u64\"]"));
        assert!(out.contains("opaque add (a0 : UInt64) (a1 : UInt64) : UInt64"));
        assert!(out.contains("@[extern \"leo4_rust__dbl__u64\"]"));
        assert!(out.contains("opaque dbl (a0 : UInt64) : UInt64"));
        let pos_add = out.find("opaque add").unwrap();
        let pos_dbl = out.find("opaque dbl").unwrap();
        assert!(pos_add < pos_dbl, "exports must sort by logical_name");
    }

    #[test]
    fn render_handles_unit_return_and_bool_param() {
        let entries = vec![mk_entry("check", "leo4_rust__check__b", &["b"], "")];
        let out = render_reverse_wrapper("X", &entries, &[]).unwrap();
        assert!(out.contains("opaque check (a0 : Bool) : Unit"));
    }

    #[test]
    fn render_handles_bytearray_via_list_u8() {
        let entries = vec![mk_entry(
            "hash",
            "leo4_rust__hash__L_u8_l",
            &["L_u8_l"],
            "L_u8_l",
        )];
        let out = render_reverse_wrapper("X", &entries, &[]).unwrap();
        assert!(out.contains("opaque hash (a0 : ByteArray) : ByteArray"));
    }

    #[test]
    fn render_handles_option_wrapper() {
        let entries = vec![mk_entry(
            "find",
            "leo4_rust__find__str",
            &["str"],
            "O_u64_o",
        )];
        let out = render_reverse_wrapper("X", &entries, &[]).unwrap();
        assert!(
            out.contains("opaque find (a0 : String) : Option UInt64"),
            "got:\n{out}"
        );
    }

    #[test]
    fn render_keyword_name_gets_raw_escape() {
        let entries = vec![mk_entry("end", "leo4_rust__end__", &[], "")];
        let out = render_reverse_wrapper("X", &entries, &[]).unwrap();
        assert!(out.contains("opaque `end` : Unit"), "got:\n{out}");
    }

    #[test]
    fn render_unmapped_mangle_surfaces_as_placeholder() {
        // RC.5 — generic-instantiation mangles still hit the
        // unmapped path. `S_My_Pair_u32_str_s` contains
        // `u32`/`str` primitive tokens → heuristic rejects.
        let entries = vec![mk_entry(
            "solve",
            "leo4_rust__solve__S_My_Pair_u32_str_s",
            &["S_My_Pair_u32_str_s"],
            "S_My_Pair_u32_str_s",
        )];
        let out = render_reverse_wrapper("X", &entries, &[]).unwrap();
        assert!(out.contains("/- unmapped: S_My_Pair_u32_str_s -/"));
    }

    #[test]
    fn empty_exports_emits_namespace_skeleton() {
        let out = render_reverse_wrapper("Nothing", &[], &[]).unwrap();
        assert!(out.contains("namespace Nothing"));
        assert!(out.contains("end Nothing"));
    }

    // ─── RC.5 patch 1 — `lean_type_of_mangle` user-defined arms

    #[test]
    fn lean_type_of_mangle_user_defined_record_no_generics() {
        assert_eq!(
            lean_type_of_mangle("S_Point_s").as_deref(),
            Some("Point"),
        );
    }

    #[test]
    fn lean_type_of_mangle_user_defined_variant_no_generics() {
        assert_eq!(
            lean_type_of_mangle("V_AdsmtVerdict_v").as_deref(),
            Some("AdsmtVerdict"),
        );
    }

    #[test]
    fn lean_type_of_mangle_user_defined_namespaced_fqn() {
        // `Sample.Color` → mangle `E_Sample_Color_e` →
        // decode `Sample_Color` (round-trip lossy).
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
    fn lean_type_of_mangle_generic_instantiation_returns_none() {
        assert_eq!(lean_type_of_mangle("S_My_Pair_u32_str_s"), None);
        assert_eq!(lean_type_of_mangle("V_Result2_u32_v"), None);
    }

    // ─── RC.5 patch 2 — Rust-type → Lean-type translator + mirror

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
        assert_eq!(rust_type_to_lean_type("Vec<u8>"), "ByteArray");
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
        assert_eq!(
            rust_type_to_lean_type("Vec<(String,String)>"),
            "Array ((String × String))"
        );
    }

    #[test]
    fn rust_type_to_lean_type_user_defined_passes_through() {
        assert_eq!(
            rust_type_to_lean_type("AdsmtVerdict"),
            "AdsmtVerdict"
        );
        assert_eq!(
            rust_type_to_lean_type("Vec<AdsmtVerdict>"),
            "Array (AdsmtVerdict)"
        );
    }

    #[test]
    fn render_unit_enum_mirror_emits_inductive_with_deriving() {
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
        // Flagship typed-enum case — same as leo4-rust-emit's
        // test, verifying the rust-transpile path emits the
        // identical mirror inductive shape.
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
        assert!(s.contains("inductive AdsmtVerdict where"));
        assert!(s.contains("| sat (model : Array ((String × String))) : AdsmtVerdict"));
        assert!(s.contains("| unsat (core : Array (String)) (cert : String) : AdsmtVerdict"));
        assert!(s.contains("| abductive (candidates : Array (AbductiveCandidate)) : AdsmtVerdict"));
        assert!(s.contains("| unknown (reason : String) : AdsmtVerdict"));
        assert!(s.contains("deriving Leo4.LeanMarshal"));
    }

    #[test]
    fn render_reverse_wrapper_emits_mirror_block_for_typed_enum_export() {
        // End-to-end: rust-transpile reverse direction with
        // typed-enum export. Wrapper file contains both the
        // mirror inductive AND the `@[extern] opaque`
        // signature; no `unmapped` placeholders.
        let entries = vec![ExportEntryView {
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
        let out = render_reverse_wrapper("My.Rust", &entries, &user_types).unwrap();
        assert!(out.contains("inductive AdsmtVerdict where"));
        assert!(out.contains("| sat (model : Array ((String × String))) : AdsmtVerdict"));
        assert!(out.contains("| unknown (reason : String) : AdsmtVerdict"));
        assert!(out.contains("deriving Leo4.LeanMarshal"));
        assert!(out.contains("opaque solve (a0 : AdsmtVerdict) : String"));
        assert!(!out.contains("/- unmapped"));
    }
}
