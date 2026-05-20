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
    FnArg, GenericArgument, PathArguments, ReturnType, Signature, Type,
};

/// Parsed body of `leo4::import! { fn add(a: u64, b: u64) -> u64; … }`.
struct ImportBlock {
    signatures: Vec<Signature>,
}

impl Parse for ImportBlock {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut signatures = Vec::new();
        while !input.is_empty() {
            let sig: Signature = input.parse()?;
            // The trailing `;` is mandatory (extern-block style) but
            // some callers may forget it; require it explicitly so
            // diagnostics are local.
            let _: syn::Token![;] = input.parse()?;
            signatures.push(sig);
        }
        Ok(ImportBlock { signatures })
    }
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
    for sig in block.signatures {
        match expand_one(&sig, &mangling) {
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

fn expand_one(sig: &Signature, mangling: &serde_json::Value) -> syn::Result<TokenStream> {
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

    // Compute the IDL-side mangling of each argument's Rust type so
    // we can disambiguate instantiations of a generic export. Types
    // we can't lower today (nominal user types without an explicit
    // hint, function pointers, references, …) trip `unrecognised`
    // and yield a localized compile error.
    let arg_idls: Vec<IDLType> = arg_types
        .iter()
        .map(|t| {
            rust_type_to_idl(t).ok_or_else(|| {
                syn::Error::new_spanned(
                    t,
                    "leo4::import!: this Rust type isn't recognised as a leo4 IDL type; supported set is u8..u64, i8..i64, f32/f64, bool, char, String, Vec<T>, Option<T>, Result<T,E>, (T1, T2)",
                )
            })
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let arg_mangles: Vec<String> = arg_idls.iter().map(mangle_type).collect();

    let mangled_body = lookup_mangled_body(mangling, &fname, &arg_mangles)
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
    use IDLType::*;
    Some(match name.as_str() {
        "u8" => U8,
        "u16" => U16,
        "u32" => U32,
        "u64" => U64,
        "i8" => I8,
        "i16" => I16,
        "i32" => I32,
        "i64" => I64,
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
        _ => return None,
    })
    .or_else(|| {
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
}
