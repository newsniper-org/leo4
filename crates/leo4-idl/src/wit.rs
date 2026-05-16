//! WIT lowering — Phase 3 of `ROADMAP.md`.
//!
//! Takes a fully-resolved leo4 [`Schema`] and renders the equivalent WIT
//! text, suitable for `wasm-tools component wit` / `wit-bindgen`.
//!
//! Mapping rules (informed by `LEO4-DESIGN.md` §4.1, `SPEC/canonical-abi.md`,
//! and `SPEC/idl-grammar.ebnf`):
//!
//! | leo4 IDL                       | WIT                                |
//! |--------------------------------|------------------------------------|
//! | `u8`…`u64`, `f32`, `f64`, etc. | identical                          |
//! | `i8`…`i64`                     | `s8`…`s64`                         |
//! | `bigint`                       | synthetic alias `bigint`           |
//! | `bignat`                       | synthetic alias `bignat`           |
//! | `string`/`char`/`bool`         | identical                          |
//! | `list<T>`, `option<T>`,        | identical (modulo lowered args)    |
//! |   `result<T>` / `result<T,E>`, |                                    |
//! |   `tuple<…>`                   |                                    |
//! | `io<T>`                        | `result<T, error>` + alias `error` |
//! | dotted FQN nominal types       | kebab-case identifier              |
//! | `Self` / `Self<…>` inside      | the variant's own kebab-case name  |
//! |   variant case payload         |                                    |
//! | generic record/variant args    | dropped (monomorphised forms come  |
//! |                                | through the schema as separate     |
//! |                                | nominal decls with mangled suffix) |
//!
//! Identifier conventions: WIT requires `[a-z][a-z0-9-]*` for names.
//! `kebab_case` converts dotted/snake/camel forms to this shape. The
//! conversion is *not* a round-trip; users who want to recover the
//! original leo4 FQN should consult `.leo4-schema`.

use std::collections::{BTreeMap, BTreeSet};

use crate::idl::*;

/// Convert any identifier-like string to a WIT-friendly kebab-case form.
///
/// Examples:
/// - `Sample.Point`     → `sample-point`
/// - `pointSum`         → `point-sum`
/// - `constantFortyTwo` → `constant-forty-two`
/// - `My.Kv.Pair`       → `my-kv-pair`
/// - `parser_handle`    → `parser-handle`
#[must_use]
pub fn kebab_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let mut prev_lower_or_digit = false;
    for c in s.chars() {
        if c == '.' || c == '_' || c == ' ' || c == '-' {
            if !out.is_empty() && !out.ends_with('-') {
                out.push('-');
            }
            prev_lower_or_digit = false;
        } else if c.is_ascii_uppercase() {
            if prev_lower_or_digit && !out.ends_with('-') {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
            prev_lower_or_digit = false;
        } else if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_lower_or_digit = c.is_ascii_lowercase() || c.is_ascii_digit();
        }
        // Anything else: silently drop. The plugin's emit form is
        // already ASCII so this only matters for hand-written test
        // fixtures.
    }
    while out.starts_with('-') {
        out.remove(0);
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Synthetic side-aliases the lowering may pull in. They are emitted
/// at the top of the interface in deterministic order.
#[derive(Clone, Copy, Eq, Hash, PartialEq, PartialOrd, Ord)]
enum SideAlias {
    Bignat,
    Bigint,
    Error,
}

impl SideAlias {
    fn name(self) -> &'static str {
        match self {
            SideAlias::Bignat => "bignat",
            SideAlias::Bigint => "bigint",
            SideAlias::Error => "error",
        }
    }
    fn rhs(self) -> &'static str {
        match self {
            SideAlias::Bignat => "list<u64>",
            SideAlias::Bigint => "tuple<bool, list<u64>>",
            SideAlias::Error => "record { code: u32, message: string }",
        }
    }
}

/// Lower an `IDLType` to its WIT textual form. The `self_name` argument
/// lets a variant case substitute its own kebab-case name for
/// `IDLType::Self_` / `IDLType::SelfApp(_)`. `needs` collects synthetic
/// side aliases (`bignat`, `bigint`, `error`) the caller must emit at
/// the top of the interface.
fn lower_type(
    t: &IDLType,
    self_name: Option<&str>,
    needs: &mut BTreeSet<SideAlias>,
) -> String {
    use IDLType::*;
    match t {
        U8 => "u8".into(),
        U16 => "u16".into(),
        U32 => "u32".into(),
        U64 => "u64".into(),
        I8 => "s8".into(),
        I16 => "s16".into(),
        I32 => "s32".into(),
        I64 => "s64".into(),
        F32 => "f32".into(),
        F64 => "f64".into(),
        Bool => "bool".into(),
        Char => "char".into(),
        String => "string".into(),
        BigNat => {
            needs.insert(SideAlias::Bignat);
            "bignat".into()
        }
        BigInt => {
            needs.insert(SideAlias::Bigint);
            "bigint".into()
        }
        List(t) => format!("list<{}>", lower_type(t, self_name, needs)),
        Option(t) => format!("option<{}>", lower_type(t, self_name, needs)),
        Result(t, None) => format!("result<{}>", lower_type(t, self_name, needs)),
        Result(t, Some(e)) => format!(
            "result<{}, {}>",
            lower_type(t, self_name, needs),
            lower_type(e, self_name, needs)
        ),
        Tuple(ts) => format!(
            "tuple<{}>",
            ts.iter()
                .map(|t| lower_type(t, self_name, needs))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        // Nominal references: drop generic args (they are already baked
        // into the FQN's monomorphised name on the .leo4-schema side).
        Record { fqn, .. } => kebab_case(fqn),
        Variant { fqn, .. } => kebab_case(fqn),
        Enum(fqn) => kebab_case(fqn),
        Flags(fqn) => kebab_case(fqn),
        Resource { fqn, .. } => kebab_case(fqn),
        Io(t) => {
            needs.insert(SideAlias::Error);
            format!("result<{}, error>", lower_type(t, self_name, needs))
        }
        Self_ | SelfApp(_) => self_name
            .expect("Self / Self<…> used outside a variant/record body")
            .to_string(),
    }
}

/// WIT lowers `variant case(payload₁, payload₂)` as
/// `variant case(tuple<payload₁, payload₂>)` — WIT case payloads are
/// single types.
fn render_variant_case_payload(
    payload: &[IDLType],
    self_name: &str,
    needs: &mut BTreeSet<SideAlias>,
) -> Option<String> {
    if payload.is_empty() {
        return None;
    }
    if payload.len() == 1 {
        Some(lower_type(&payload[0], Some(self_name), needs))
    } else {
        let inner = payload
            .iter()
            .map(|t| lower_type(t, Some(self_name), needs))
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!("tuple<{inner}>"))
    }
}

/// True iff `t` (transitively, but never through a nominal type's body)
/// contains a `Self` / `Self<…>` token.
fn type_contains_self(t: &IDLType) -> bool {
    use IDLType::*;
    match t {
        Self_ | SelfApp(_) => true,
        List(t) | Option(t) | Io(t) => type_contains_self(t),
        Result(t, None) => type_contains_self(t),
        Result(t, Some(e)) => type_contains_self(t) || type_contains_self(e),
        Tuple(ts) => ts.iter().any(type_contains_self),
        Record { args, .. } | Variant { args, .. } | Resource { args, .. } => {
            args.iter().any(type_contains_self)
        }
        _ => false,
    }
}

/// True iff `d`'s body contains a `Self`-reference. WIT does not accept
/// direct self-references; these are lowered to opaque resources
/// (LEO4-DESIGN.md §4.1 "Cyclic ADTs → resources").
fn decl_is_self_recursive(d: &UserDecl) -> bool {
    match d {
        UserDecl::Record { fields, .. } => fields.iter().any(|(_, t)| type_contains_self(t)),
        UserDecl::Variant { cases, .. } => cases
            .iter()
            .any(|(_, payload)| payload.iter().any(type_contains_self)),
        _ => false,
    }
}

/// Render a `UserDecl` as a WIT type/resource declaration.
fn render_user_decl(d: &UserDecl, needs: &mut BTreeSet<SideAlias>) -> String {
    // Cyclic ADT short-circuit: WIT cannot express direct self-recursion
    // in a record or variant body. Lower to an opaque resource. The
    // canonical-ABI marshal path on the Lean side is unaffected — the
    // resource handle here is only the *wire* representation a WASM
    // consumer would see.
    if decl_is_self_recursive(d) {
        let name = kebab_case(d.fqn());
        return format!("  resource {name};");
    }
    match d {
        UserDecl::Record { fqn, fields, .. } => {
            let name = kebab_case(fqn);
            let body = fields
                .iter()
                .map(|(fname, ty)| {
                    format!(
                        "    {}: {},",
                        kebab_case(fname),
                        lower_type(ty, Some(&name), needs)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("  record {name} {{\n{body}\n  }}")
        }
        UserDecl::Enum { fqn, cases } => {
            let name = kebab_case(fqn);
            let body = cases
                .iter()
                .map(|c| kebab_case(c))
                .collect::<Vec<_>>()
                .join(", ");
            format!("  enum {name} {{ {body} }}")
        }
        UserDecl::Variant { fqn, cases, .. } => {
            let name = kebab_case(fqn);
            let body = cases
                .iter()
                .map(|(cname, payload)| {
                    let cn = kebab_case(cname);
                    match render_variant_case_payload(payload, &name, needs) {
                        None => format!("    {cn},"),
                        Some(p) => format!("    {cn}({p}),"),
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!("  variant {name} {{\n{body}\n  }}")
        }
        UserDecl::Resource { fqn, .. } => {
            let name = kebab_case(fqn);
            format!("  resource {name};")
        }
    }
}

/// Render a fully-resolved [`Schema`] as a WIT package + interface.
///
/// Determinism: declarations are sorted (type decls first by FQN, then
/// resources by FQN, then funcs by name); synthetic side-aliases are
/// emitted in fixed enum order. Identical input → identical output bytes.
#[must_use]
pub fn lower(schema: &Schema) -> String {
    let mut needs: BTreeSet<SideAlias> = BTreeSet::new();

    // Sort user decls: types (record/enum/variant) first by FQN, then
    // resources by FQN. Inside this section, render to a Vec<String> so we
    // can collect `needs` from each, then we know the final alias set.
    let mut decl_records: Vec<&UserDecl> = schema
        .user_decls
        .iter()
        .filter(|d| !matches!(d, UserDecl::Resource { .. }))
        .collect();
    decl_records.sort_by(|a, b| a.fqn().cmp(b.fqn()));

    let mut decl_resources: Vec<&UserDecl> = schema
        .user_decls
        .iter()
        .filter(|d| matches!(d, UserDecl::Resource { .. }))
        .collect();
    decl_resources.sort_by(|a, b| a.fqn().cmp(b.fqn()));

    let mut decl_strs: Vec<String> = Vec::new();
    for d in decl_records.iter().chain(decl_resources.iter()) {
        decl_strs.push(render_user_decl(d, &mut needs));
    }

    // Funcs, sorted by name. Inputs/outputs may also pull side aliases.
    let mut funcs: Vec<&FuncDecl> = schema.funcs.iter().collect();
    funcs.sort_by(|a, b| a.name.cmp(&b.name));

    let mut func_strs: Vec<String> = Vec::with_capacity(funcs.len());
    let mut counters: BTreeMap<String, u32> = BTreeMap::new();
    for f in funcs {
        let name = kebab_case(&f.name);
        // Disambiguate overloaded monomorphised names by appending an
        // index. The Lake plugin emits one `func` line per monomorph
        // (e.g. listLen 15 times); WIT has no overloading. We mangle the
        // type list into a stable suffix using `mangle_type` so the
        // resulting WIT name is deterministic and free of conflicts.
        let count = counters.entry(name.clone()).or_insert(0);
        let unique = if schema.funcs.iter().filter(|g| g.name == f.name).count() > 1 {
            let suffix = f
                .params
                .iter()
                .map(|(_, t)| crate::mangle::mangle_type(t))
                .collect::<Vec<_>>()
                .join("-");
            *count += 1;
            format!("{name}-{}", kebab_case(&suffix))
        } else {
            name
        };

        let params = f
            .params
            .iter()
            .enumerate()
            .map(|(i, (_pname, ty))| {
                format!("p{i}: {}", lower_type(ty, None, &mut needs))
            })
            .collect::<Vec<_>>()
            .join(", ");
        // Distinguish a "no return" from a "returns unit-shaped tuple".
        // Phase 1 emit form represents missing `-> T` as an empty Tuple;
        // collapse that back to a WIT-style omitted return for clarity.
        let ret_text = match &f.ret {
            IDLType::Tuple(t) if t.is_empty() => String::new(),
            other => format!(" -> {}", lower_type(other, None, &mut needs)),
        };
        func_strs.push(format!("  {unique}: func({params}){ret_text};"));
    }

    // Compose. Package name: leo4 uses kebab already most of the time
    // (`leo4-sample`); WIT package name must be `namespace:name`. If
    // the leo4 package has no namespace, fabricate `leo4:<pkg>`.
    let wit_pkg = if schema.package.contains(':') {
        kebab_pkg_path(&schema.package)
    } else {
        format!("leo4:{}", kebab_case(&schema.package))
    };
    let iface_name = if schema.interface.is_empty() {
        "default".to_string()
    } else {
        kebab_case(&schema.interface)
    };

    let mut out = String::new();
    out.push_str(&format!("package {wit_pkg};\n\n"));
    out.push_str(&format!("interface {iface_name} {{\n"));

    // Side aliases first.
    for a in &needs {
        out.push_str(&format!("  type {} = {};\n", a.name(), a.rhs()));
    }
    if !needs.is_empty() && (!decl_strs.is_empty() || !func_strs.is_empty()) {
        out.push('\n');
    }

    // Type and resource decls.
    for d in &decl_strs {
        out.push_str(d);
        out.push('\n');
    }
    if !decl_strs.is_empty() && !func_strs.is_empty() {
        out.push('\n');
    }

    // Funcs.
    for f in &func_strs {
        out.push_str(f);
        out.push('\n');
    }

    out.push_str("}\n");

    // Default `world` that exports the lone interface. `wit-bindgen`
    // refuses a package with no world; this is the simplest shape that
    // makes the file usable end-to-end. Authors who hand-write IDL
    // with their own world(s) can re-emit via the Lake plugin's escape
    // hatch (Phase 3+).
    out.push_str(&format!("\nworld {iface_name}-world {{\n"));
    out.push_str(&format!("  export {iface_name};\n"));
    out.push_str("}\n");

    out
}

/// Apply `kebab_case` to each `name:ns` / path segment.
fn kebab_pkg_path(pkg: &str) -> String {
    pkg.split(':')
        .map(kebab_case)
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kebab_case_basics() {
        assert_eq!(kebab_case("Sample.Point"), "sample-point");
        assert_eq!(kebab_case("pointSum"), "point-sum");
        assert_eq!(kebab_case("constantFortyTwo"), "constant-forty-two");
        assert_eq!(kebab_case("My.Kv.Pair"), "my-kv-pair");
        assert_eq!(kebab_case("parser_handle"), "parser-handle");
        assert_eq!(kebab_case("UPPER"), "upper");
        assert_eq!(kebab_case("u32"), "u32");
    }

    #[test]
    fn lower_primitives() {
        let mut needs = BTreeSet::new();
        assert_eq!(lower_type(&IDLType::U8, None, &mut needs), "u8");
        assert_eq!(lower_type(&IDLType::I32, None, &mut needs), "s32");
        assert_eq!(lower_type(&IDLType::Bool, None, &mut needs), "bool");
        assert!(needs.is_empty());
    }

    #[test]
    fn lower_bignat_emits_alias() {
        let mut needs = BTreeSet::new();
        assert_eq!(lower_type(&IDLType::BigNat, None, &mut needs), "bignat");
        assert!(needs.contains(&SideAlias::Bignat));
    }

    #[test]
    fn lower_self_in_variant_uses_variant_name() {
        let mut needs = BTreeSet::new();
        let s = lower_type(&IDLType::Self_, Some("tree"), &mut needs);
        assert_eq!(s, "tree");
        let s2 = lower_type(
            &IDLType::SelfApp(vec![IDLType::U32]),
            Some("tree"),
            &mut needs,
        );
        assert_eq!(s2, "tree");
    }

    #[test]
    fn empty_schema_lowers_to_empty_interface() {
        let schema = Schema {
            package: "leo4-sample".into(),
            interface: "Sample".into(),
            user_decls: vec![],
            funcs: vec![],
        };
        let out = lower(&schema);
        assert!(out.starts_with("package leo4:leo4-sample;"));
        assert!(out.contains("interface sample {"));
    }

    #[test]
    fn record_and_func_round_trip_through_wasm_tools() {
        // Pure unit test — `tests/wit/run.sh` invokes wasm-tools to validate.
        let schema = Schema {
            package: "leo4-sample".into(),
            interface: "Sample".into(),
            user_decls: vec![UserDecl::Record {
                fqn: "Sample.Point".into(),
                generics: vec![],
                fields: vec![
                    ("x".into(), IDLType::F64),
                    ("y".into(), IDLType::F64),
                ],
            }],
            funcs: vec![FuncDecl {
                name: "pointSum".into(),
                params: vec![(
                    "p".into(),
                    IDLType::Record {
                        fqn: "Sample.Point".into(),
                        args: vec![],
                    },
                )],
                ret: IDLType::F64,
            }],
        };
        let out = lower(&schema);
        assert!(out.contains("record sample-point"));
        assert!(out.contains("x: f64,"));
        assert!(out.contains("point-sum: func(p0: sample-point) -> f64;"));
    }
}
