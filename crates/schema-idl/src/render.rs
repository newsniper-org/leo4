//! Canonical IDL rendering. Mirrors `Mangling.renderCanonical` /
//! `userDeclToIDL` / `idlForm` / `collapseWhitespace` on the Lean side.
//!
//! Two outputs of the same data:
//!   * `render_canonical(.., pretty=true)`  → newline-decorated form for
//!     `<pkg>.leo4-schema`.
//!   * `render_canonical(.., pretty=false)` → fully-collapsed form; this
//!     is the *exact* byte stream fed to `Hash::fnv1a64` when computing
//!     the schema hash. Both sides must produce identical bytes for the
//!     same logical input.

use crate::idl::{FuncDecl, IDLType, UserDecl};

/// Newline / indentation toggle.
fn ws(pretty: bool) -> (&'static str, &'static str) {
    if pretty {
        ("\n", "  ")
    } else {
        (" ", "")
    }
}

/// SPEC/handshake.md ordering: type decls first (lex by FQN), then
/// resources (lex by FQN), then functions (lex by name).
fn decl_band(d: &UserDecl) -> u8 {
    match d {
        UserDecl::Record { .. }
        | UserDecl::Enum { .. }
        | UserDecl::Variant { .. }
        | UserDecl::Flags { .. } => 0,
        UserDecl::Resource { .. } => 1,
    }
}

/// IDL textual form of one type. Matches `Leo4Plugin.Mangling.idlForm`.
pub fn idl_form(t: &IDLType) -> String {
    use IDLType::*;
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
        List(t) => format!("list<{}>", idl_form(t)),
        Option(t) => format!("option<{}>", idl_form(t)),
        Result(t, None) => format!("result<{}>", idl_form(t)),
        Result(t, Some(e)) => format!("result<{}, {}>", idl_form(t), idl_form(e)),
        Tuple(ts) => format!(
            "tuple<{}>",
            ts.iter().map(idl_form).collect::<Vec<_>>().join(", ")
        ),
        Record { fqn, args } if args.is_empty() => fqn.clone(),
        Record { fqn, args } => format!(
            "{fqn}<{}>",
            args.iter().map(idl_form).collect::<Vec<_>>().join(", ")
        ),
        Variant { fqn, args } if args.is_empty() => fqn.clone(),
        Variant { fqn, args } => format!(
            "{fqn}<{}>",
            args.iter().map(idl_form).collect::<Vec<_>>().join(", ")
        ),
        Enum(fqn) => fqn.clone(),
        Flags(fqn) => fqn.clone(),
        Resource { fqn, args } if args.is_empty() => fqn.clone(),
        Resource { fqn, args } => format!(
            "{fqn}<{}>",
            args.iter().map(idl_form).collect::<Vec<_>>().join(", ")
        ),
        Io(t) => format!("io<{}>", idl_form(t)),
        Self_ => "Self".into(),
        SelfApp(args) => format!(
            "Self<{}>",
            args.iter().map(idl_form).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Render a `UserDecl` as a single line of IDL text. Matches
/// `Leo4Plugin.Mangling.userDeclToIDL`.
/// `<T0, T1>` header that follows the fqn on a nominal declaration.
/// Empty string when the declaration has no generic parameters.
fn generic_header(generics: &[String]) -> String {
    if generics.is_empty() {
        String::new()
    } else {
        format!("<{}>", generics.join(", "))
    }
}

pub fn user_decl_to_idl(d: &UserDecl) -> String {
    match d {
        UserDecl::Record { fqn, generics, fields } => {
            let fs = fields
                .iter()
                .map(|(n, t)| format!("{n}: {}", idl_form(t)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("record {fqn}{} {{ {fs} }}", generic_header(generics))
        }
        UserDecl::Enum { fqn, cases } => {
            let cs = cases.join(", ");
            format!("enum {fqn} {{ {cs} }}")
        }
        UserDecl::Variant { fqn, generics, cases } => {
            let cs = cases
                .iter()
                .map(|(n, payload)| {
                    if payload.is_empty() {
                        n.clone()
                    } else {
                        let ps = payload.iter().map(idl_form).collect::<Vec<_>>().join(", ");
                        format!("{n}({ps})")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("variant {fqn}{} {{ {cs} }}", generic_header(generics))
        }
        UserDecl::Resource { fqn, generics } => {
            format!("resource {fqn}{}", generic_header(generics))
        }
        UserDecl::Flags { fqn, generics, members } => {
            format!(
                "flags {fqn}{} {{ {} }}",
                generic_header(generics),
                members.join(", ")
            )
        }
    }
}

/// Collapse all runs of ASCII whitespace to a single space and trim.
/// Matches `Leo4Plugin.Mangling.collapseWhitespace`.
pub fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut saw_space = true;
    for c in s.chars() {
        if c.is_ascii_whitespace() {
            if !saw_space {
                out.push(' ');
                saw_space = true;
            }
        } else {
            out.push(c);
            saw_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Render the canonical IDL form for the discovered export set + user-type
/// declarations.
///
/// * `pretty = true`  → newline-decorated form for `<pkg>.leo4-schema`.
/// * `pretty = false` → fully-collapsed form (the schema-hash input).
///
/// Mirrors `Leo4Plugin.Mangling.renderCanonical`.
#[must_use]
pub fn render_canonical(
    pkg: &str,
    iface: &str,
    user_decls: &[UserDecl],
    funcs: &[FuncDecl],
    pretty: bool,
) -> String {
    let mut decls_sorted: Vec<&UserDecl> = user_decls.iter().collect();
    decls_sorted.sort_by(|a, b| {
        decl_band(a)
            .cmp(&decl_band(b))
            .then_with(|| a.fqn().cmp(b.fqn()))
    });
    let mut funcs_sorted: Vec<&FuncDecl> = funcs.iter().collect();
    funcs_sorted.sort_by(|a, b| a.name.cmp(&b.name));

    let (nl, ind) = ws(pretty);

    let mut s = String::new();
    s.push_str(&format!("package {pkg};{nl}"));
    s.push_str(&format!("interface {iface} {{{nl}"));
    for d in &decls_sorted {
        s.push_str(ind);
        s.push_str(&user_decl_to_idl(d));
        s.push(';');
        s.push_str(nl);
    }
    for f in &funcs_sorted {
        s.push_str(ind);
        s.push_str("func ");
        s.push_str(&f.name);
        s.push('(');
        for (i, (_pname, t)) in f.params.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&format!("_{i}: {}", idl_form(t)));
        }
        s.push_str(") -> ");
        s.push_str(&idl_form(&f.ret));
        s.push(';');
        s.push_str(nl);
    }
    s.push('}');
    if pretty { s } else { collapse_whitespace(&s) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::idl::*;

    #[test]
    fn idl_form_primitives() {
        assert_eq!(idl_form(&IDLType::U8), "u8");
        assert_eq!(idl_form(&IDLType::Bool), "bool");
        assert_eq!(idl_form(&IDLType::String), "string");
        assert_eq!(idl_form(&IDLType::BigInt), "bigint");
    }

    #[test]
    fn idl_form_composites() {
        let l = IDLType::List(Box::new(IDLType::U32));
        assert_eq!(idl_form(&l), "list<u32>");
        let r = IDLType::Result(
            Box::new(IDLType::U64),
            Some(Box::new(IDLType::String)),
        );
        assert_eq!(idl_form(&r), "result<u64, string>");
    }

    #[test]
    fn collapse_drops_runs_and_trims() {
        assert_eq!(collapse_whitespace("  a  b\nc  "), "a b c");
        assert_eq!(collapse_whitespace(""), "");
        assert_eq!(collapse_whitespace("x"), "x");
    }

    #[test]
    fn canonical_form_is_deterministic() {
        let pkg = "leo4-sample";
        let iface = "Sample";
        let user_decls = vec![UserDecl::Record {
            fqn: "Sample.Point".into(),
            generics: vec![],
            fields: vec![("x".into(), IDLType::F64), ("y".into(), IDLType::F64)],
        }];
        let funcs = vec![FuncDecl {
            name: "pointSum".into(),
            params: vec![(
                "p".into(),
                IDLType::Record {
                    fqn: "Sample.Point".into(),
                    args: vec![],
                },
            )],
            ret: IDLType::F64,
            effect: crate::Effect::Sync,
        }];
        let c1 = render_canonical(pkg, iface, &user_decls, &funcs, false);
        let c2 = render_canonical(pkg, iface, &user_decls, &funcs, false);
        assert_eq!(c1, c2);
        // Collapsed form has no double-spaces or leading/trailing whitespace.
        assert!(!c1.contains("  "));
        assert!(!c1.starts_with(' ') && !c1.ends_with(' '));
    }
}
