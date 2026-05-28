//! leo4-macros-backend — proc-macro expansion logic.
//!
//! Split out of the `proc-macro = true` `leo4-macros` crate so the
//! expander is a regular library (compilable without the `proc_macro`
//! linkage, testable as a normal crate, etc.).
//!
//! Public entry: [`expand_import`]. Input is the `TokenStream` body
//! of a `leo4::import! { … }` macro invocation; output is the
//! generated wrapper functions.
//!
//! P5-b₂ scope (minimum viable): scalar-only function signatures
//! against a non-generic Lake-emitted `logical_name`. Generic
//! exports (multi-instantiation) and composite payloads land in
//! P5-b₃.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use schema_idl::{mangle_type, IDLType};
use syn::{
    parse::{Parse, ParseStream, Parser},
    Attribute, FnArg, GenericArgument, LitStr, PathArguments, ReturnType, Signature, Type,
};

/// One entry inside a `leo4::import! { … }` block: a function signature
/// optionally preceded by `#[leo4(...)]` attributes.
struct ImportItem {
    attrs: Vec<Attribute>,
    sig: Signature,
}

impl Parse for ImportItem {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let sig: Signature = input.parse()?;
        // Trailing `;` mandatory (extern-block style); fail locally
        // when the caller forgets it.
        let _: syn::Token![;] = input.parse()?;
        Ok(ImportItem { attrs, sig })
    }
}

/// Parsed body of `leo4::import! { … }`. One [`ImportItem`] per `fn`.
struct ImportBlock {
    items: Vec<ImportItem>,
}

impl Parse for ImportBlock {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut items = Vec::new();
        while !input.is_empty() {
            items.push(input.parse()?);
        }
        Ok(ImportBlock { items })
    }
}

/// Parsed `#[leo4(...)]` import-side attributes (P5-b₃-iv).
#[derive(Default, Debug)]
struct ImportAttrs {
    /// `#[leo4(args = "u64, str, S_Sample_Point_s")]`. Comma-separated
    /// IDL `mangle_type` strings — one per `fn` parameter, in order.
    /// When set, this fully replaces the macro's `rust_type_to_idl`
    /// inference so multi-instantiation exports can be disambiguated
    /// even when the Rust signature uses types the macro can't lower
    /// (newtypes, aliases, user-marshalled wrappers).
    args_mangles: Option<Vec<String>>,
}

fn parse_import_attrs(attrs: &[Attribute]) -> syn::Result<ImportAttrs> {
    let mut out = ImportAttrs::default();
    for attr in attrs {
        if !attr.path().is_ident("leo4") {
            return Err(syn::Error::new_spanned(
                attr,
                "leo4::import! only recognises `#[leo4(...)]` attributes here",
            ));
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("args") {
                let lit: LitStr = meta.value()?.parse()?;
                let parts: Vec<String> = lit
                    .value()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                out.args_mangles = Some(parts);
                Ok(())
            } else {
                Err(meta.error(
                    "unknown leo4 import attribute key; supported: `args = \"…\"`",
                ))
            }
        })?;
    }
    Ok(out)
}

/// Expand a `leo4::import! { … }` block into per-function wrappers.
///
/// The expansion looks each `fn` up in the Lake-emitted mangling
/// JSON pointed to by `LEO4_MANGLING_FILE` (set by `leo4_build::wire`),
/// matches by `logical_name`'s last `::` segment, and emits a wrapper
/// of shape:
///
/// ```ignore
/// pub fn add(lean: &::leo4::Lean, a: u64, b: u64) -> Result<u64, ::leo4::LeanError> {
///     let mut args = Vec::<u8>::with_capacity(16);
///     args.extend_from_slice(&a.to_le_bytes());
///     args.extend_from_slice(&b.to_le_bytes());
///     let mut ret = [0u8; 8];
///     lean.call_shim(MANGLED_BODY, &args, &mut ret)?;
///     Ok(u64::from_le_bytes(ret))
/// }
/// ```
pub fn expand_import(input: TokenStream) -> TokenStream {
    let block = match ImportBlock::parse.parse2(input) {
        Ok(b) => b,
        Err(e) => return e.to_compile_error(),
    };

    let mangling = match load_mangling_json() {
        Ok(j) => j,
        Err(msg) => {
            let lit = proc_macro2::Literal::string(&msg);
            return quote! {
                ::core::compile_error!(#lit);
            };
        }
    };

    let mut out = TokenStream::new();
    for item in block.items {
        match expand_one(&item, &mangling) {
            Ok(ts) => out.extend(ts),
            Err(e) => out.extend(e.to_compile_error()),
        }
    }
    out
}

/// Load the Lake-emitted mangling JSON from `LEO4_MANGLING_FILE`.
/// Returns a compile-error-ready message string on failure.
fn load_mangling_json() -> Result<serde_json::Value, String> {
    let path = std::env::var("LEO4_MANGLING_FILE").map_err(|_| {
        "LEO4_MANGLING_FILE is not set — add `leo4_build::wire(\"<lake-build-dir>\")` to build.rs"
            .to_string()
    })?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("leo4_macros: cannot read {path}: {e}"))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("leo4_macros: cannot parse {path}: {e}"))
}

fn expand_one(item: &ImportItem, mangling: &serde_json::Value) -> syn::Result<TokenStream> {
    let attrs = parse_import_attrs(&item.attrs)?;
    let sig = &item.sig;
    if sig.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            sig,
            "async functions are not supported by leo4::import! yet (D-i, Phase 7)",
        ));
    }
    if !sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &sig.generics,
            "generic functions are not supported by leo4::import! at P5-b₂; instantiate them at concrete types or wait for P5-b₃",
        ));
    }

    let fname = sig.ident.to_string();

    // Collect (name, type) pairs. We delegate encoding to leo4-abi's
    // `LeanMarshal` trait (P5-b₂ generalisation): every parameter
    // and the return type must implement it, but we don't introspect
    // the type AST beyond rejecting `self` receivers.
    let mut arg_idents: Vec<syn::Ident> = Vec::new();
    let mut arg_types: Vec<syn::Type> = Vec::new();
    for input in &sig.inputs {
        let FnArg::Typed(pt) = input else {
            return Err(syn::Error::new_spanned(
                input,
                "leo4::import! does not accept `self` receivers",
            ));
        };
        let syn::Pat::Ident(ident) = pt.pat.as_ref() else {
            return Err(syn::Error::new_spanned(
                &pt.pat,
                "leo4::import! requires simple `name: T` parameters",
            ));
        };
        arg_idents.push(ident.ident.clone());
        arg_types.push((*pt.ty).clone());
    }
    let ret_ty = match &sig.output {
        ReturnType::Default => {
            return Err(syn::Error::new_spanned(
                sig,
                "leo4::import! requires an explicit return type (use `-> ()` for no result)",
            ));
        }
        ReturnType::Type(_, ty) => (**ty).clone(),
    };

    // Three-tier disambiguation:
    //   1. `#[leo4(args = "…")]` override — bypasses inference entirely
    //      so the user can name the exact instantiation when their
    //      Rust types don't lower (newtype wrappers, aliases, custom
    //      LeanMarshal impls). The literal is parsed as a
    //      comma-separated list of `mangle_type` strings (same
    //      vocabulary the `.leo4-mangling` JSON uses).
    //   2. Every arg type lowers via `rust_type_to_idl` — match by
    //      the computed mangled arg list.
    //   3. At least one arg type can't be lowered — fall back to a
    //      fname-only single-instantiation lookup. Errors when the
    //      export has multiple instantiations; the user resolves that
    //      with tier 1.
    let mangled_body = if let Some(arg_mangles) = attrs.args_mangles.as_ref() {
        if arg_mangles.len() != arg_types.len() {
            return Err(syn::Error::new_spanned(
                &sig.ident,
                format!(
                    "leo4::import! `#[leo4(args = \"…\")]` declared {} arg(s) but the Rust signature has {}",
                    arg_mangles.len(),
                    arg_types.len()
                ),
            ));
        }
        lookup_mangled_body(mangling, &fname, arg_mangles)
    } else {
        let arg_idls_opt: Vec<Option<IDLType>> =
            arg_types.iter().map(rust_type_to_idl).collect();
        let all_known = arg_idls_opt.iter().all(Option::is_some);
        if all_known {
            let arg_mangles: Vec<String> = arg_idls_opt
                .iter()
                .map(|t| mangle_type(t.as_ref().unwrap()))
                .collect();
            lookup_mangled_body(mangling, &fname, &arg_mangles)
        } else {
            lookup_single_instantiation(mangling, &fname)
        }
    }
    .map_err(|e| syn::Error::new_spanned(&sig.ident, e))?;

    let lean = format_ident!("lean");

    // Encode each arg via `<T as LeanMarshal>::canonical_encode`.
    let encode_stmts = arg_idents.iter().zip(arg_types.iter()).map(|(name, ty)| {
        quote! { <#ty as ::leo4::LeanMarshal>::canonical_encode(&#name, &mut args); }
    });

    let arg_decls = arg_idents.iter().zip(arg_types.iter()).map(|(name, ty)| {
        quote! { #name: #ty }
    });

    let fname_ident = sig.ident.clone();
    let vis = quote! { pub };

    // Grow-on-too-small retry loop. The shim returns
    // LEO4_ERR_RETURN_BUF_TOO_SMALL (code 7) with `*ret_len` set to
    // the required size; `LeanError.detail` carries it as
    // "need <N> bytes". We parse the detail back out for a single
    // retry; further retries fail with the same shim error.
    Ok(quote! {
        #[allow(non_snake_case)]
        #vis fn #fname_ident(
            #lean: &::leo4::Lean,
            #(#arg_decls),*
        ) -> ::core::result::Result<#ret_ty, ::leo4::LeanError> {
            let mut args: ::std::vec::Vec<u8> = ::std::vec::Vec::new();
            #(#encode_stmts)*
            let mut cap: usize = 4096;
            let written = loop {
                let mut ret: ::std::vec::Vec<u8> = ::std::vec::Vec::with_capacity(cap);
                ret.resize(cap, 0u8);
                match #lean.call_shim(#mangled_body, &args, &mut ret) {
                    ::core::result::Result::Ok(written) => {
                        ret.truncate(written);
                        break ret;
                    }
                    ::core::result::Result::Err(e) if e.code == 7 => {
                        // Detail format: "… need <N> bytes, got <M>".
                        let needed = e.detail
                            .split_whitespace()
                            .filter_map(|w| w.parse::<usize>().ok())
                            .next();
                        let next_cap = match needed {
                            ::core::option::Option::Some(n) if n > cap => n,
                            _ => cap.saturating_mul(2),
                        };
                        if next_cap == cap {
                            return ::core::result::Result::Err(e);
                        }
                        cap = next_cap;
                    }
                    ::core::result::Result::Err(e) => return ::core::result::Result::Err(e),
                }
            };
            let (value, _consumed) =
                <#ret_ty as ::leo4::LeanMarshal>::canonical_decode(&written, 0)?;
            ::core::result::Result::Ok(value)
        }
    })
}

/// Map a Rust type (as parsed by `syn`) to its IDL counterpart so
/// we can mangle it the same way the Lake plugin does. Returns
/// `None` for types outside the recognised set.
fn rust_type_to_idl(ty: &Type) -> Option<IDLType> {
    if let Type::Tuple(t) = ty {
        if t.elems.is_empty() {
            // unit type `()`; no IDL counterpart at v0.
            return None;
        }
        let inners: ::std::option::Option<Vec<IDLType>> =
            t.elems.iter().map(rust_type_to_idl).collect();
        return inners.map(IDLType::Tuple);
    }
    let Type::Path(p) = ty else { return None };
    if p.qself.is_some() {
        return None;
    }
    let last = p.path.segments.last()?;
    let name = last.ident.to_string();
    let args_of_seg = || -> Vec<Type> {
        let PathArguments::AngleBracketed(ab) = &last.arguments else {
            return Vec::new();
        };
        ab.args
            .iter()
            .filter_map(|a| match a {
                GenericArgument::Type(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    };
    use IDLType::{U8, U16, U32, U64, I8, I16, I32, I64, Record, F32, F64, Bool, Char, String, List, Option, Result, Fn};
    Some(match name.as_str() {
        "u8" => U8,
        "u16" => U16,
        "u32" => U32,
        "u64" => U64,
        "i8" => I8,
        "i16" => I16,
        "i32" => I32,
        "i64" => I64,
        // Stable 128-bit integers (#55). Pair with Lean `Leo4.LeanU128`
        // / `Leo4.LeanI128` records on the wire (16 bytes LE).
        "u128" => Record {
            fqn: "Leo4.LeanU128".to_string(),
            args: vec![],
        },
        "i128" => Record {
            fqn: "Leo4.LeanI128".to_string(),
            args: vec![],
        },
        "f32" => F32,
        "f64" => F64,
        "bool" => Bool,
        "char" => Char,
        "String" => String,
        "Vec" => {
            let args = args_of_seg();
            let inner = rust_type_to_idl(args.first()?)?;
            List(Box::new(inner))
        }
        "Option" => {
            let args = args_of_seg();
            let inner = rust_type_to_idl(args.first()?)?;
            Option(Box::new(inner))
        }
        "Result" => {
            let args = args_of_seg();
            let t = rust_type_to_idl(args.first()?)?;
            let e = args.get(1).and_then(rust_type_to_idl).map(Box::new);
            Result(Box::new(t), e)
        }
        // Phase 10-B1.x — function-arrow params crossing the boundary
        // as Lean closures appear in user exports as
        // `LeanCallback<R, Args>`. The second generic is either a
        // tuple of arg types (n-ary) or a single type (1-arg shorthand).
        "LeanCallback" => {
            let args = args_of_seg();
            let ret = rust_type_to_idl(args.first()?)?;
            let args_param = args.get(1)?;
            let arrow_args: Vec<IDLType> = match args_param {
                Type::Tuple(t) => {
                    let collected: ::std::option::Option<Vec<IDLType>> =
                        t.elems.iter().map(rust_type_to_idl).collect();
                    collected?
                }
                _ => vec![rust_type_to_idl(args_param)?],
            };
            Fn {
                args: arrow_args,
                ret: Box::new(ret),
            }
        }
        _ => return None,
    })
    .or({
        // Fall through to handle Type::Tuple via the outer caller —
        // the closure above returns None for unrecognised idents,
        // and Some(...) for the matched arms; .or_else here is dead.
        None
    })
}

/// Find the mangled body for a leo4 export matching `fname` and the
/// caller's `arg_mangles` (each parameter's IDL `mangle_type` string,
/// in declaration order).
///
/// Match rule: the entry's `logical_name` must end in `::<fname>`;
/// among its `instantiations`, the one whose `param_types[*].encoded`
/// list equals `arg_mangles` wins. Returns an error if no instantiation
/// matches or if multiple entries share the fname (the latter would
/// signal an IDL bug, since `logical_name` is per-export).
fn lookup_mangled_body(
    mangling: &serde_json::Value,
    fname: &str,
    arg_mangles: &[String],
) -> Result<String, String> {
    let entries = mangling
        .get("entries")
        .and_then(|x| x.as_array())
        .ok_or("mangling JSON has no `entries` array")?;
    let mut hits: Vec<&serde_json::Value> = Vec::new();
    for e in entries {
        let logical = e
            .get("logical_name")
            .and_then(|x| x.as_str())
            .unwrap_or_default();
        let last = logical.rsplit("::").next().unwrap_or("");
        if last == fname {
            hits.push(e);
        }
    }
    let entry = match hits.len() {
        0 => return Err(format!("no leo4 export named `{fname}` in mangling JSON")),
        1 => hits[0],
        n => {
            return Err(format!(
                "ambiguous: {n} leo4 exports match `{fname}` — same logical name in different interfaces?"
            ));
        }
    };

    let insts = entry
        .get("instantiations")
        .and_then(|x| x.as_array())
        .ok_or_else(|| format!("entry `{fname}` has no `instantiations`"))?;
    for inst in insts {
        let param_types = inst
            .get("param_types")
            .and_then(|x| x.as_array())
            .ok_or_else(|| format!("entry `{fname}` instantiation missing `param_types`"))?;
        if param_types.len() != arg_mangles.len() {
            continue;
        }
        let inst_matches = param_types.iter().zip(arg_mangles.iter()).all(|(pt, am)| {
            pt.get("encoded")
                .and_then(|x| x.as_str())
                .is_some_and(|enc| enc == am)
        });
        if inst_matches {
            return inst
                .get("mangled")
                .and_then(|x| x.as_str())
                .map(str::to_string)
                .ok_or_else(|| format!("matched instantiation of `{fname}` missing `mangled`"));
        }
    }
    Err(format!(
        "no instantiation of leo4 export `{fname}` matches arg list {arg_mangles:?}"
    ))
}

/// Fallback lookup for the case where the macro can't compute an
/// arg-mangle vector (e.g. a parameter is a user-defined nominal
/// type whose IDL FQN isn't surfaced through the syntactic view).
/// Succeeds only when the named export has exactly one instantiation;
/// multi-instantiation exports require disambiguation that today
/// arrives by writing every arg type in a form `rust_type_to_idl`
/// recognises (P5-b₃-iv will add an explicit attribute hint).
fn lookup_single_instantiation(
    mangling: &serde_json::Value,
    fname: &str,
) -> Result<String, String> {
    let entries = mangling
        .get("entries")
        .and_then(|x| x.as_array())
        .ok_or("mangling JSON has no `entries` array")?;
    let mut hits: Vec<&serde_json::Value> = Vec::new();
    for e in entries {
        let logical = e
            .get("logical_name")
            .and_then(|x| x.as_str())
            .unwrap_or_default();
        let last = logical.rsplit("::").next().unwrap_or("");
        if last == fname {
            hits.push(e);
        }
    }
    let entry = match hits.len() {
        0 => return Err(format!("no leo4 export named `{fname}` in mangling JSON")),
        1 => hits[0],
        n => {
            return Err(format!(
                "ambiguous: {n} leo4 exports match `{fname}`"
            ))
        }
    };
    let insts = entry
        .get("instantiations")
        .and_then(|x| x.as_array())
        .ok_or_else(|| format!("entry `{fname}` has no `instantiations`"))?;
    if insts.len() != 1 {
        return Err(format!(
            "leo4 export `{fname}` has {} instantiations — disambiguate by writing each parameter type in a form `rust_type_to_idl` recognises, or by adding `#[leo4(args = \"<mangled,csv>\")]` above the `fn` (see the package's `.leo4-mangling` JSON for the exact strings)",
            insts.len()
        ));
    }
    insts[0]
        .get("mangled")
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("instantiation of `{fname}` missing `mangled`"))
}


#[cfg(test)]
mod tests {
    use super::*;
    use schema_idl::IDLType;

    #[test]
    fn rust_type_to_idl_basics() {
        let ty: Type = syn::parse_str("u64").unwrap();
        assert_eq!(rust_type_to_idl(&ty), Some(IDLType::U64));

        let ty: Type = syn::parse_str("Vec<u32>").unwrap();
        assert_eq!(
            rust_type_to_idl(&ty),
            Some(IDLType::List(Box::new(IDLType::U32)))
        );

        let ty: Type = syn::parse_str("Option<String>").unwrap();
        assert_eq!(
            rust_type_to_idl(&ty),
            Some(IDLType::Option(Box::new(IDLType::String)))
        );

        let ty: Type = syn::parse_str("(u8, bool)").unwrap();
        assert_eq!(
            rust_type_to_idl(&ty),
            Some(IDLType::Tuple(vec![IDLType::U8, IDLType::Bool]))
        );

        let ty: Type = syn::parse_str("MyCustom").unwrap();
        assert_eq!(rust_type_to_idl(&ty), None);
    }

    #[test]
    fn rust_type_to_idl_lean_callback_single_arg() {
        // `LeanCallback<u64, (u64,)>` (1-tuple wrap of single arg)
        let ty: Type = syn::parse_str("LeanCallback<u64, (u64,)>").unwrap();
        assert_eq!(
            rust_type_to_idl(&ty),
            Some(IDLType::Fn {
                args: vec![IDLType::U64],
                ret: Box::new(IDLType::U64),
            })
        );
    }

    #[test]
    fn rust_type_to_idl_lean_callback_zero_args() {
        // `LeanCallback<String, ()>` — the empty-tuple second
        // generic encodes a 0-arg arrow `fn() -> String`.
        let ty: Type = syn::parse_str("LeanCallback<String, ()>").unwrap();
        assert_eq!(
            rust_type_to_idl(&ty),
            Some(IDLType::Fn {
                args: Vec::new(),
                ret: Box::new(IDLType::String),
            })
        );
    }

    #[test]
    fn rust_type_to_idl_lean_callback_multi_arg() {
        // `LeanCallback<bool, (u32, u32)>` — 2-ary arrow.
        let ty: Type = syn::parse_str("LeanCallback<bool, (u32, u32)>").unwrap();
        assert_eq!(
            rust_type_to_idl(&ty),
            Some(IDLType::Fn {
                args: vec![IDLType::U32, IDLType::U32],
                ret: Box::new(IDLType::Bool),
            })
        );
    }

    #[test]
    fn rust_type_to_idl_lean_callback_bare_single_type() {
        // Sugar: when the second generic is a single type rather
        // than a 1-tuple, treat it as a single-arg arrow.
        // `LeanCallback<u64, u32>` ≡ `LeanCallback<u64, (u32,)>`.
        let ty: Type = syn::parse_str("LeanCallback<u64, u32>").unwrap();
        assert_eq!(
            rust_type_to_idl(&ty),
            Some(IDLType::Fn {
                args: vec![IDLType::U32],
                ret: Box::new(IDLType::U64),
            })
        );
    }

    #[test]
    fn lookup_disambiguates_by_arg_mangle() {
        let m: serde_json::Value = serde_json::from_str(
            r#"{
              "entries": [{
                "logical_name": "Sample::stringify",
                "instantiations": [
                  { "param_types": [{ "encoded": "u64" }],
                    "mangled": "MANGLED_U64" },
                  { "param_types": [{ "encoded": "b" }],
                    "mangled": "MANGLED_BOOL" }
                ]
              }]
            }"#,
        )
        .unwrap();
        assert_eq!(
            lookup_mangled_body(&m, "stringify", &["u64".into()]).unwrap(),
            "MANGLED_U64"
        );
        assert_eq!(
            lookup_mangled_body(&m, "stringify", &["b".into()]).unwrap(),
            "MANGLED_BOOL"
        );
    }

    #[test]
    fn import_attrs_parse_args_override() {
        let attrs: Vec<Attribute> =
            Attribute::parse_outer
                .parse_str("#[leo4(args = \"u64, str , L_u64_l\")]")
                .unwrap();
        let parsed = parse_import_attrs(&attrs).unwrap();
        assert_eq!(
            parsed.args_mangles.unwrap(),
            vec![
                "u64".to_string(),
                "str".to_string(),
                "L_u64_l".to_string()
            ]
        );
    }

    #[test]
    fn import_attrs_rejects_unknown_key() {
        let attrs: Vec<Attribute> = Attribute::parse_outer
            .parse_str("#[leo4(symbol = \"xx\")]")
            .unwrap();
        let err = parse_import_attrs(&attrs).unwrap_err();
        assert!(err.to_string().contains("unknown leo4 import attribute key"));
    }

    #[test]
    fn lookup_rejects_no_match() {
        let m: serde_json::Value = serde_json::from_str(
            r#"{
              "entries": [{
                "logical_name": "Sample::add",
                "instantiations": [
                  { "param_types": [{ "encoded": "u64" }, { "encoded": "u64" }],
                    "mangled": "MANGLED_ADD" }
                ]
              }]
            }"#,
        )
        .unwrap();
        let err = lookup_mangled_body(&m, "add", &["u32".into(), "u32".into()]).unwrap_err();
        assert!(err.contains("no instantiation"), "{err}");
    }

    // ─── #[leo4::export] expansion (Phase 9-1) ─────────────────

    #[test]
    fn export_attrs_default_is_persistent() {
        let parsed = ExportAttrs::parse_from_args(TokenStream::new()).unwrap();
        assert!(!parsed.isolated);
    }

    #[test]
    fn export_attrs_isolated() {
        let args: TokenStream = syn::parse_str("isolated").unwrap();
        let parsed = ExportAttrs::parse_from_args(args).unwrap();
        assert!(parsed.isolated);
    }

    #[test]
    fn export_attrs_rejects_unknown() {
        let args: TokenStream = syn::parse_str("speedrun").unwrap();
        let err = ExportAttrs::parse_from_args(args).unwrap_err();
        assert!(
            err.to_string().contains("unknown #[leo4::export(...)] option"),
            "got: {err}"
        );
    }

    /// Sanity: a trivial scalar-only `#[leo4::export]` expansion
    /// emits the expected mangled wrapper symbol and the `ExportEntry`
    /// metadata. The output is just rendered as text; we don't
    /// type-check it here (proc-macro UI tests on a fixture cdylib
    /// are 9-1's responsibility once an example crate exists).
    #[test]
    fn expand_export_scalar_smoke() {
        let input: TokenStream = syn::parse_str(
            "pub fn add(a: u64, b: u64) -> u64 { a + b }",
        )
        .unwrap();
        let ts = expand_export(TokenStream::new(), input);
        let rendered = ts.to_string();
        assert!(
            rendered.contains("leo4_rust__add__u64_u64"),
            "expected mangled wrapper symbol in: {rendered}"
        );
        assert!(
            rendered.contains("ExportEntry"),
            "expected ExportEntry registration in: {rendered}"
        );
        assert!(
            rendered.contains("\"add\""),
            "expected logical_name literal in: {rendered}"
        );
    }

    #[test]
    fn expand_export_rejects_async() {
        let input: TokenStream = syn::parse_str(
            "pub async fn foo(x: u64) -> u64 { x }",
        )
        .unwrap();
        let ts = expand_export(TokenStream::new(), input);
        let rendered = ts.to_string();
        assert!(
            rendered.contains("does not support `async fn`"),
            "expected diagnostic in: {rendered}"
        );
    }

    #[test]
    fn expand_export_rejects_unsupported_param_type() {
        // `Cow<'_, str>` isn't in `rust_type_to_idl`'s table.
        let input: TokenStream = syn::parse_str(
            "pub fn foo(x: std::borrow::Cow<'static, str>) -> u64 { x.len() as u64 }",
        )
        .unwrap();
        let ts = expand_export(TokenStream::new(), input);
        let rendered = ts.to_string();
        assert!(
            rendered.contains("cannot lower parameter")
                || rendered.contains("rust_type_to_idl"),
            "expected diagnostic in: {rendered}"
        );
    }
}

// ─── #[derive(LeanMarshal)] backend ────────────────────────────────

/// Expand `#[derive(LeanMarshal)]` over a struct or enum. Four
/// shapes:
///
/// - struct with `#[leo4(resource)]`: u64-handle wire (SPEC §12).
///   Body must be a single `raw: u64` field — encoder writes 8 LE
///   bytes, decoder reads them back.
/// - struct (no attr): record (SPEC §8) — each field encoded in
///   declaration order.
/// - enum where every variant is unit: IDL enum (SPEC §10) — u32 LE
///   tag in declaration order.
/// - enum with mixed-payload variants: IDL variant (SPEC §9) — u8 LE
///   discriminator + per-case payload.
///
/// Generic parameters carry through; each generic gets a
/// `: ::leo4::LeanMarshal` bound on the synthesised impl.
#[must_use] 
pub fn expand_derive_lean_marshal(input: TokenStream) -> TokenStream {
    let derive_input: syn::DeriveInput = match syn::parse2(input) {
        Ok(d) => d,
        Err(e) => return e.to_compile_error(),
    };

    let is_resource = derive_input.attrs.iter().any(is_leo4_resource);

    match (&derive_input.data, is_resource) {
        (syn::Data::Struct(s), true)  => expand_derive_resource(&derive_input, s),
        (syn::Data::Struct(s), false) => expand_derive_record(&derive_input, s),
        (syn::Data::Enum(e), _) => {
            let all_unit = e.variants.iter().all(|v| matches!(v.fields, syn::Fields::Unit));
            if all_unit {
                expand_derive_enum(&derive_input, e)
            } else {
                expand_derive_variant(&derive_input, e)
            }
        }
        (syn::Data::Union(_), _) => syn::Error::new_spanned(
            &derive_input.ident,
            "leo4 LeanMarshal cannot be derived for unions",
        )
        .to_compile_error(),
    }
}

fn is_leo4_resource(attr: &syn::Attribute) -> bool {
    if !attr.path().is_ident("leo4") {
        return false;
    }
    let mut hit = false;
    let _ = attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("resource") {
            hit = true;
        }
        Ok(())
    });
    hit
}

fn add_lean_marshal_bound(generics: &syn::Generics) -> syn::Generics {
    let mut g = generics.clone();
    for param in &mut g.params {
        if let syn::GenericParam::Type(tp) = param {
            tp.bounds
                .push(syn::parse_quote!(::leo4::LeanMarshal));
        }
    }
    g
}

fn expand_derive_record(input: &syn::DeriveInput, s: &syn::DataStruct) -> TokenStream {
    let name = &input.ident;
    let bounded = add_lean_marshal_bound(&input.generics);
    let (impl_g, ty_g, where_g) = bounded.split_for_impl();
    let (encode_block, decode_block) = match &s.fields {
        syn::Fields::Named(named) => {
            let names: Vec<_> = named.named.iter().map(|f| f.ident.clone().unwrap()).collect();
            let types: Vec<_> = named.named.iter().map(|f| f.ty.clone()).collect();
            let enc = names.iter().zip(types.iter()).map(|(n, t)| {
                quote! { <#t as ::leo4::LeanMarshal>::canonical_encode(&self.#n, buf); }
            });
            let dec_idents: Vec<_> = names.clone();
            let dec_steps = names.iter().zip(types.iter()).map(|(n, t)| {
                quote! {
                    let (#n, __off) = <#t as ::leo4::LeanMarshal>::canonical_decode(buf, __off)?;
                }
            });
            (
                quote! { #(#enc)* },
                quote! {
                    let mut __off = off;
                    #(#dec_steps)*
                    Ok((Self { #(#dec_idents),* }, __off))
                },
            )
        }
        syn::Fields::Unnamed(unnamed) => {
            let count = unnamed.unnamed.len();
            let types: Vec<_> = unnamed.unnamed.iter().map(|f| f.ty.clone()).collect();
            let indices: Vec<syn::Index> = (0..count).map(syn::Index::from).collect();
            let temps: Vec<syn::Ident> = (0..count)
                .map(|i| format_ident!("__f{}", i))
                .collect();
            let enc = indices.iter().zip(types.iter()).map(|(i, t)| {
                quote! { <#t as ::leo4::LeanMarshal>::canonical_encode(&self.#i, buf); }
            });
            let dec_steps = temps.iter().zip(types.iter()).map(|(temp, t)| {
                quote! {
                    let (#temp, __off) = <#t as ::leo4::LeanMarshal>::canonical_decode(buf, __off)?;
                }
            });
            (
                quote! { #(#enc)* },
                quote! {
                    let mut __off = off;
                    #(#dec_steps)*
                    Ok((Self(#(#temps),*), __off))
                },
            )
        }
        syn::Fields::Unit => (
            quote! {},
            quote! { Ok((Self, off)) },
        ),
    };
    quote! {
        impl #impl_g ::leo4::LeanMarshal for #name #ty_g #where_g {
            fn canonical_encode(&self, buf: &mut ::std::vec::Vec<u8>) {
                #encode_block
            }
            fn canonical_decode(buf: &[u8], off: usize)
                -> ::core::result::Result<(Self, usize), ::leo4::AbiError>
            {
                #decode_block
            }
        }
    }
}

fn expand_derive_resource(input: &syn::DeriveInput, s: &syn::DataStruct) -> TokenStream {
    let name = &input.ident;
    // Single `raw: u64` (or unnamed u64) field required.
    let raw_acc: TokenStream = match &s.fields {
        syn::Fields::Named(named) if named.named.len() == 1 => {
            let f = named.named.first().unwrap();
            let n = f.ident.clone().unwrap();
            quote! { self.#n }
        }
        syn::Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => quote! { self.0 },
        _ => {
            return syn::Error::new_spanned(
                &input.ident,
                "leo4 LeanMarshal `#[leo4(resource)]` expects a single field of type u64",
            )
            .to_compile_error();
        }
    };
    let ctor: TokenStream = match &s.fields {
        syn::Fields::Named(named) => {
            let n = named.named.first().unwrap().ident.clone().unwrap();
            quote! { Self { #n: handle } }
        }
        syn::Fields::Unnamed(_) => quote! { Self(handle) },
        _ => unreachable!(),
    };
    let bounded = add_lean_marshal_bound(&input.generics);
    let (impl_g, ty_g, where_g) = bounded.split_for_impl();
    quote! {
        impl #impl_g ::leo4::LeanMarshal for #name #ty_g #where_g {
            fn canonical_encode(&self, buf: &mut ::std::vec::Vec<u8>) {
                buf.extend_from_slice(&(#raw_acc).to_le_bytes());
            }
            fn canonical_decode(buf: &[u8], off: usize)
                -> ::core::result::Result<(Self, usize), ::leo4::AbiError>
            {
                if buf.len() < off + 8 {
                    return ::core::result::Result::Err(::leo4::AbiError::new(
                        ::leo4::error_codes::DECODE_ERROR,
                        "leo4 resource: not enough bytes for u64 handle",
                    ));
                }
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&buf[off..off + 8]);
                let handle = u64::from_le_bytes(bytes);
                ::core::result::Result::Ok((#ctor, off + 8))
            }
        }
    }
}

fn expand_derive_enum(input: &syn::DeriveInput, e: &syn::DataEnum) -> TokenStream {
    let name = &input.ident;
    let bounded = add_lean_marshal_bound(&input.generics);
    let (impl_g, ty_g, where_g) = bounded.split_for_impl();
    let enc_arms = e.variants.iter().enumerate().map(|(i, v)| {
        let vn = &v.ident;
        let tag = i as u32;
        quote! { Self::#vn => #tag, }
    });
    let dec_arms = e.variants.iter().enumerate().map(|(i, v)| {
        let vn = &v.ident;
        let tag = i as u32;
        quote! { #tag => Self::#vn, }
    });
    quote! {
        impl #impl_g ::leo4::LeanMarshal for #name #ty_g #where_g {
            fn canonical_encode(&self, buf: &mut ::std::vec::Vec<u8>) {
                let tag: u32 = match self {
                    #(#enc_arms)*
                };
                buf.extend_from_slice(&tag.to_le_bytes());
            }
            fn canonical_decode(buf: &[u8], off: usize)
                -> ::core::result::Result<(Self, usize), ::leo4::AbiError>
            {
                if buf.len() < off + 4 {
                    return ::core::result::Result::Err(::leo4::AbiError::new(
                        ::leo4::error_codes::DECODE_ERROR,
                        "leo4 enum: not enough bytes for u32 tag",
                    ));
                }
                let mut bytes = [0u8; 4];
                bytes.copy_from_slice(&buf[off..off + 4]);
                let tag = u32::from_le_bytes(bytes);
                let value = match tag {
                    #(#dec_arms)*
                    _ => return ::core::result::Result::Err(::leo4::AbiError::new(
                        ::leo4::error_codes::DECODE_ERROR,
                        format!("leo4 enum: invalid tag {tag}"),
                    )),
                };
                ::core::result::Result::Ok((value, off + 4))
            }
        }
    }
}

fn expand_derive_variant(input: &syn::DeriveInput, e: &syn::DataEnum) -> TokenStream {
    let name = &input.ident;
    let bounded = add_lean_marshal_bound(&input.generics);
    let (impl_g, ty_g, where_g) = bounded.split_for_impl();
    let mut enc_arms = TokenStream::new();
    let mut dec_arms = TokenStream::new();
    // SPEC/canonical-abi.md §9: variant discriminator is `u32 LE` on
    // the wire. Encoders MUST emit 4 bytes; permissive decoders MAY
    // accept a 1-byte form (we don't — strict 4-byte for byte-identical
    // cross-impl conformance).
    for (i, v) in e.variants.iter().enumerate() {
        let vn = &v.ident;
        let disc = i as u32;
        match &v.fields {
            syn::Fields::Unit => {
                enc_arms.extend(quote! {
                    Self::#vn => { buf.extend_from_slice(&(#disc).to_le_bytes()); }
                });
                dec_arms.extend(quote! {
                    #disc => ::core::result::Result::Ok((Self::#vn, off + 4)),
                });
            }
            syn::Fields::Unnamed(unnamed) => {
                let temps: Vec<syn::Ident> = (0..unnamed.unnamed.len())
                    .map(|j| format_ident!("__f{}", j))
                    .collect();
                let types: Vec<_> =
                    unnamed.unnamed.iter().map(|f| f.ty.clone()).collect();
                let enc_steps = temps.iter().zip(types.iter()).map(|(t, ty)| {
                    quote! { <#ty as ::leo4::LeanMarshal>::canonical_encode(#t, buf); }
                });
                enc_arms.extend(quote! {
                    Self::#vn(#(#temps),*) => {
                        buf.extend_from_slice(&(#disc).to_le_bytes());
                        #(#enc_steps)*
                    }
                });
                let dec_steps = temps.iter().zip(types.iter()).map(|(t, ty)| {
                    quote! {
                        let (#t, __off) =
                            <#ty as ::leo4::LeanMarshal>::canonical_decode(buf, __off)?;
                    }
                });
                dec_arms.extend(quote! {
                    #disc => {
                        let mut __off = off + 4;
                        #(#dec_steps)*
                        ::core::result::Result::Ok((Self::#vn(#(#temps),*), __off))
                    }
                });
            }
            syn::Fields::Named(named) => {
                let names: Vec<_> = named.named.iter().map(|f| f.ident.clone().unwrap()).collect();
                let types: Vec<_> = named.named.iter().map(|f| f.ty.clone()).collect();
                let enc_steps = names.iter().zip(types.iter()).map(|(n, ty)| {
                    quote! { <#ty as ::leo4::LeanMarshal>::canonical_encode(#n, buf); }
                });
                enc_arms.extend(quote! {
                    Self::#vn { #(#names),* } => {
                        buf.extend_from_slice(&(#disc).to_le_bytes());
                        #(#enc_steps)*
                    }
                });
                let dec_steps = names.iter().zip(types.iter()).map(|(n, ty)| {
                    quote! {
                        let (#n, __off) =
                            <#ty as ::leo4::LeanMarshal>::canonical_decode(buf, __off)?;
                    }
                });
                dec_arms.extend(quote! {
                    #disc => {
                        let mut __off = off + 4;
                        #(#dec_steps)*
                        ::core::result::Result::Ok((Self::#vn { #(#names),* }, __off))
                    }
                });
            }
        }
    }
    quote! {
        impl #impl_g ::leo4::LeanMarshal for #name #ty_g #where_g {
            fn canonical_encode(&self, buf: &mut ::std::vec::Vec<u8>) {
                match self {
                    #enc_arms
                }
            }
            fn canonical_decode(buf: &[u8], off: usize)
                -> ::core::result::Result<(Self, usize), ::leo4::AbiError>
            {
                if buf.len() < off + 4 {
                    return ::core::result::Result::Err(::leo4::AbiError::new(
                        ::leo4::error_codes::DECODE_ERROR,
                        "leo4 variant: not enough bytes for u32 discriminator",
                    ));
                }
                let disc = u32::from_le_bytes(
                    buf[off..off + 4].try_into().expect("4-byte slice"),
                );
                match disc {
                    #dec_arms
                    _ => ::core::result::Result::Err(::leo4::AbiError::new(
                        ::leo4::error_codes::DECODE_ERROR,
                        format!("leo4 variant: invalid disc {disc}"),
                    )),
                }
            }
        }
    }
}

// =====================================================================
// Phase 9-1 — `#[leo4::export]` attribute proc-macro expansion.
//
// Input: an `ItemFn` (the user's tagged Rust function) and an
// `args` token stream from the attribute (empty for default mode;
// `isolated` for per-call fresh-worker mode).
//
// Output: the original function (unchanged) + a generated
// `extern "C"` wrapper named `leo4_rust__<fname>__<param_mangles>`
// that performs canonical-ABI decode → catch_unwind(call) →
// canonical-ABI encode, plus a `linkme` distributed-slice entry
// registering the export's metadata.
//
// The wrapper symbol intentionally omits the `__h<schema_hash>`
// suffix that forward-direction mangling carries — schema_hash is
// not known at macro-expand time, and it lives in the handshake
// file + cdylib constant only. See `SPEC/reverse-direction.md` §2.
// =====================================================================

/// `#[leo4::export]` — see `SPEC/reverse-direction.md`.
///
/// Expansion (sketch):
///
/// ```ignore
/// // input
/// #[leo4::export]
/// pub fn add(a: u64, b: u64) -> u64 { a + b }
///
/// // output
/// pub fn add(a: u64, b: u64) -> u64 { a + b }
///
/// #[unsafe(no_mangle)]
/// pub unsafe extern "C" fn leo4_rust__add__u64_u64(
///     args_ptr: *const u8, args_len: usize,
///     ret_ptr: *mut u8, ret_cap: usize, ret_len: *mut usize,
/// ) -> i32 { /* decode -> catch_unwind(add(...)) -> encode */ }
///
/// #[::linkme::distributed_slice(::leo4::__private::EXPORTS)]
/// #[allow(non_upper_case_globals)]
/// static __LEO4_EXPORT_add: ::leo4::__private::ExportEntry = …;
/// ```
#[must_use] 
pub fn expand_export(args: TokenStream, input: TokenStream) -> TokenStream {
    let attrs = match ExportAttrs::parse_from_args(args.clone()) {
        Ok(a) => a,
        Err(e) => return e.to_compile_error(),
    };
    let item_fn: syn::ItemFn = match syn::parse2(input.clone()) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error(),
    };

    match expand_export_inner(&attrs, &item_fn) {
        Ok(ts) => ts,
        Err(e) => {
            // Keep the original `fn` so downstream type-checking
            // proceeds (gives the user a clearer diagnostic than
            // a "function not found" cascade).
            let original = quote! { #item_fn };
            let err = e.to_compile_error();
            quote! { #original #err }
        }
    }
}

#[derive(Default, Debug)]
struct ExportAttrs {
    isolated: bool,
}

impl ExportAttrs {
    fn parse_from_args(args: TokenStream) -> syn::Result<Self> {
        if args.is_empty() {
            return Ok(ExportAttrs::default());
        }
        // Comma-separated key list. Today only `isolated` is
        // recognised; the recycle / panic-abort options stay
        // deferred per ROADMAP.
        struct Parser_(ExportAttrs);
        impl Parse for Parser_ {
            fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
                let mut out = ExportAttrs::default();
                let punct: syn::punctuated::Punctuated<syn::Ident, syn::Token![,]> =
                    syn::punctuated::Punctuated::parse_terminated(input)?;
                for ident in punct {
                    match ident.to_string().as_str() {
                        "isolated" => out.isolated = true,
                        other => {
                            return Err(syn::Error::new(
                                ident.span(),
                                format!(
                                    "unknown #[leo4::export(...)] option: `{other}` \
                                     (recognised: `isolated`)"
                                ),
                            ));
                        }
                    }
                }
                Ok(Parser_(out))
            }
        }
        let Parser_(parsed) = syn::parse2(args)?;
        Ok(parsed)
    }
}

fn expand_export_inner(
    attrs: &ExportAttrs,
    item_fn: &syn::ItemFn,
) -> syn::Result<TokenStream> {
    let sig = &item_fn.sig;
    let fname = sig.ident.clone();
    let fname_str = fname.to_string();

    if sig.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            sig.fn_token,
            "`#[leo4::export]` does not support `async fn` in v0; the boundary stays sync",
        ));
    }
    if sig.unsafety.is_some() {
        return Err(syn::Error::new_spanned(
            sig.fn_token,
            "`#[leo4::export]` does not accept `unsafe fn`; wrap unsafety inside the body",
        ));
    }
    if !sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &sig.generics,
            "`#[leo4::export]` does not support generic functions in v0 — monomorphise at the boundary",
        ));
    }

    // Collect parameter types in declaration order, rejecting `self`.
    let mut param_idents: Vec<syn::Ident> = Vec::new();
    let mut param_types: Vec<Type> = Vec::new();
    for input in &sig.inputs {
        match input {
            FnArg::Receiver(r) => {
                return Err(syn::Error::new_spanned(
                    r,
                    "`#[leo4::export]` does not support `self` receivers",
                ));
            }
            FnArg::Typed(pt) => {
                let ident = match &*pt.pat {
                    syn::Pat::Ident(pi) => pi.ident.clone(),
                    other => {
                        return Err(syn::Error::new_spanned(
                            other,
                            "`#[leo4::export]` parameter patterns must be plain identifiers",
                        ));
                    }
                };
                param_idents.push(ident);
                param_types.push((*pt.ty).clone());
            }
        }
    }

    // Compute IDL form for each parameter via the existing
    // `rust_type_to_idl` helper (shared with the forward
    // direction's lookup path).
    let mut param_mangles: Vec<String> = Vec::with_capacity(param_types.len());
    for (i, ty) in param_types.iter().enumerate() {
        let idl = rust_type_to_idl(ty).ok_or_else(|| {
            syn::Error::new_spanned(
                ty,
                format!(
                    "`#[leo4::export]`: cannot lower parameter #{i} type to IDL — \
                     `rust_type_to_idl` doesn't recognise it (only scalars, \
                     `String`, `Vec<T>`, `Option<T>`, `Result<T, E>`, tuples, \
                     and the LeanU128/I128/Complex* carriers are wired in v9-1)"
                ),
            )
        })?;
        param_mangles.push(mangle_type(&idl));
    }

    // Return type: `()` is unit (mangle as empty string).
    let (ret_ty_tokens, ret_mangle, ret_is_unit): (TokenStream, String, bool) = match &sig.output {
        ReturnType::Default => (quote! { () }, String::new(), true),
        ReturnType::Type(_, ty) => {
            let idl = rust_type_to_idl(ty).ok_or_else(|| {
                syn::Error::new_spanned(
                    ty,
                    "`#[leo4::export]`: cannot lower return type to IDL — same restriction as parameters",
                )
            })?;
            let tokens = quote! { #ty };
            (tokens, mangle_type(&idl), false)
        }
    };

    // Mangled wrapper symbol — no `__h<hash>` suffix in reverse
    // direction (SPEC/reverse-direction.md §2).
    let mangled = if param_mangles.is_empty() {
        format!("leo4_rust__{fname_str}")
    } else {
        format!("leo4_rust__{fname_str}__{}", param_mangles.join("_"))
    };
    let wrapper_ident = format_ident!("{}", mangled);
    let entry_ident = format_ident!("__LEO4_EXPORT_{}", fname_str);

    let isolated_lit = attrs.isolated;

    // Decode statements: one per parameter, threading the offset.
    let mut decode_stmts: Vec<TokenStream> = Vec::with_capacity(param_idents.len());
    for (ident, ty) in param_idents.iter().zip(param_types.iter()) {
        decode_stmts.push(quote! {
            let (#ident, __leo4_off) = match
                <#ty as ::leo4::LeanMarshal>::canonical_decode(__leo4_args, __leo4_off)
            {
                ::core::result::Result::Ok(v) => v,
                ::core::result::Result::Err(_) => return ::core::result::Result::Err(
                    ::leo4::error_codes::DECODE_ERROR as i32,
                ),
            };
        });
    }

    let call_expr = quote! { #fname(#(#param_idents),*) };

    let encode_block: TokenStream = if ret_is_unit {
        // Unit return: wrapper writes a zero-length response.
        quote! {
            let _ = #call_expr;
            let mut __leo4_buf: ::std::vec::Vec<u8> = ::std::vec::Vec::new();
            ::core::result::Result::<::std::vec::Vec<u8>, i32>::Ok(__leo4_buf)
        }
    } else {
        quote! {
            let __leo4_ret: #ret_ty_tokens = #call_expr;
            let mut __leo4_buf: ::std::vec::Vec<u8> = ::std::vec::Vec::new();
            <#ret_ty_tokens as ::leo4::LeanMarshal>::canonical_encode(&__leo4_ret, &mut __leo4_buf);
            ::core::result::Result::<::std::vec::Vec<u8>, i32>::Ok(__leo4_buf)
        }
    };

    // Per-parameter IDL string literals for the linkme entry.
    let param_type_lits = param_mangles.iter().map(std::string::String::as_str);

    Ok(quote! {
        // Original user function — kept unchanged so the user
        // can still call it from Rust as a normal `fn`.
        #item_fn

        // Canonical-ABI wrapper. `#[unsafe(no_mangle)]` (2024 edition)
        // makes the symbol name predictable; `extern "C"` pins the
        // calling convention. The dispatcher reaches this symbol via
        // `dlsym` / `GetProcAddress` after loading the cdylib.
        #[unsafe(no_mangle)]
        #[allow(non_snake_case)]
        #[allow(clippy::missing_safety_doc)]
        pub unsafe extern "C" fn #wrapper_ident(
            __leo4_args_ptr: *const u8,
            __leo4_args_len: usize,
            __leo4_ret_ptr: *mut u8,
            __leo4_ret_cap: usize,
            __leo4_ret_len: *mut usize,
        ) -> i32 {
            // SAFETY: dispatcher contract — `args_ptr` is valid for
            // `args_len` bytes of read; `ret_ptr` for `ret_cap` bytes
            // of write; `ret_len` is a valid `&mut usize`.
            let __leo4_args: &[u8] = if __leo4_args_len == 0 {
                &[]
            } else {
                unsafe { ::core::slice::from_raw_parts(__leo4_args_ptr, __leo4_args_len) }
            };

            let __leo4_result: ::core::result::Result<
                ::core::result::Result<::std::vec::Vec<u8>, i32>,
                ::std::boxed::Box<dyn ::core::any::Any + ::core::marker::Send + 'static>,
            > = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                let __leo4_off: usize = 0;
                #(#decode_stmts)*
                if __leo4_off != __leo4_args.len() {
                    return ::core::result::Result::Err(
                        ::leo4::error_codes::DECODE_ERROR as i32,
                    );
                }
                #encode_block
            }));

            match __leo4_result {
                ::core::result::Result::Ok(::core::result::Result::Ok(__leo4_buf)) => {
                    if __leo4_buf.len() > __leo4_ret_cap {
                        unsafe { *__leo4_ret_len = __leo4_buf.len(); }
                        return ::leo4::error_codes::BUFFER_TOO_SMALL as i32;
                    }
                    if !__leo4_buf.is_empty() {
                        unsafe {
                            ::core::ptr::copy_nonoverlapping(
                                __leo4_buf.as_ptr(),
                                __leo4_ret_ptr,
                                __leo4_buf.len(),
                            );
                        }
                    }
                    unsafe { *__leo4_ret_len = __leo4_buf.len(); }
                    0_i32
                }
                ::core::result::Result::Ok(::core::result::Result::Err(__code)) => {
                    unsafe { *__leo4_ret_len = 0; }
                    __code
                }
                ::core::result::Result::Err(_) => {
                    // Rust panic — dispatcher's contract is to abort
                    // the worker, but we also signal LEO4_ERR_RUST_PANIC
                    // so the caller has a code to log. The harness
                    // (Phase 9-3) handles the abort.
                    unsafe { *__leo4_ret_len = 0; }
                    0x0002_0001_u32 as i32
                }
            }
        }

        // Metadata entry — picked up by `leo4-build` at cdylib
        // build time (Phase 9-2) and by Lake (Phase 9-5) via the
        // emitted `<pkg>.leo4-rust-exports.idl`. The `linkme` path
        // routes through `leo4::__private` so user cdylibs need
        // only depend on `leo4`, not on `linkme` directly.
        #[::leo4::__private::linkme::distributed_slice(::leo4::__private::EXPORTS)]
        #[linkme(crate = ::leo4::__private::linkme)]
        #[allow(non_upper_case_globals)]
        static #entry_ident: ::leo4::__private::ExportEntry =
            ::leo4::__private::ExportEntry {
                logical_name: #fname_str,
                mangled: #mangled,
                param_types: &[#(#param_type_lits),*],
                ret_type: #ret_mangle,
                isolated: #isolated_lit,
                abi_version: 1,
            };
    })
}
