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
use syn::{
    parse::{Parse, ParseStream, Parser},
    FnArg, ItemFn, ReturnType, Signature, Type,
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

    // P5-b₂ minimum: scalar-only. Reject anything else with a clear
    // diagnostic so the failure mode is local.
    let mut arg_kinds: Vec<ScalarKind> = Vec::new();
    let mut arg_idents: Vec<syn::Ident> = Vec::new();
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
                "leo4::import! requires simple `name: T` parameters at P5-b₂",
            ));
        };
        let kind = match classify_scalar(pt.ty.as_ref()) {
            Some(k) => k,
            None => {
                return Err(syn::Error::new_spanned(
                    &pt.ty,
                    "non-scalar parameter — P5-b₂ supports u8/u16/u32/u64/i8..i64/f32/f64/bool/char only; composite / nominal lands in P5-b₃",
                ));
            }
        };
        arg_kinds.push(kind);
        arg_idents.push(ident.ident.clone());
    }
    let ret_kind = match &sig.output {
        ReturnType::Default => {
            return Err(syn::Error::new_spanned(
                sig,
                "leo4::import! requires an explicit return type (use `-> ()` for no result)",
            ));
        }
        ReturnType::Type(_, ty) => match classify_scalar(ty.as_ref()) {
            Some(k) => k,
            None => {
                return Err(syn::Error::new_spanned(
                    ty,
                    "non-scalar return — P5-b₂ supports scalar return types only (P5-b₃ for composites)",
                ));
            }
        },
    };

    // Look up the mangled body. We match on the last `::` segment of
    // `logical_name`; if multiple entries share that name, P5-b₂
    // rejects (requires the user to disambiguate by writing the
    // mangled body explicitly — that path lands with P5-b₃ once
    // generics support is wired).
    let mangled_body = lookup_mangled_body(mangling, &fname).map_err(|e| {
        syn::Error::new_spanned(&sig.ident, e)
    })?;

    let lean = format_ident!("lean");
    let in_size: usize = arg_kinds.iter().map(|k| k.wire_size()).sum();
    let out_size: usize = ret_kind.wire_size();

    // Encode each arg into a single Vec<u8> by repeated to_le_bytes.
    let encode_stmts = arg_idents.iter().zip(&arg_kinds).map(|(name, kind)| {
        let to_bytes = kind.to_le_bytes_call(name);
        quote! { args.extend_from_slice(&#to_bytes); }
    });

    let from_bytes = ret_kind.from_le_bytes_call(quote! { ret });
    let ret_ty = ret_kind.rust_type();

    let arg_decls = arg_idents.iter().zip(&arg_kinds).map(|(name, kind)| {
        let ty = kind.rust_type();
        quote! { #name: #ty }
    });

    let fname_ident = sig.ident.clone();
    let vis = quote! { pub };

    Ok(quote! {
        #vis fn #fname_ident(
            #lean: &::leo4::Lean,
            #(#arg_decls),*
        ) -> ::core::result::Result<#ret_ty, ::leo4::LeanError> {
            let mut args: ::std::vec::Vec<u8> = ::std::vec::Vec::with_capacity(#in_size);
            #(#encode_stmts)*
            let mut ret: [u8; #out_size] = [0u8; #out_size];
            #lean.call_shim(#mangled_body, &args, &mut ret)?;
            ::core::result::Result::Ok(#from_bytes)
        }
    })
}

/// Find the `entries[*].instantiations[0].mangled` for a function
/// whose `logical_name` ends in `::<fname>`. Errors if zero or
/// multiple entries match.
fn lookup_mangled_body(mangling: &serde_json::Value, fname: &str) -> Result<String, String> {
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
    match hits.len() {
        0 => Err(format!("no leo4 export named `{fname}` in mangling JSON")),
        1 => {
            let insts = hits[0]
                .get("instantiations")
                .and_then(|x| x.as_array())
                .ok_or_else(|| format!("entry `{fname}` has no `instantiations`"))?;
            if insts.len() != 1 {
                return Err(format!(
                    "leo4 export `{fname}` has {} instantiations — P5-b₂ requires a single (non-generic) instantiation",
                    insts.len()
                ));
            }
            let m = insts[0]
                .get("mangled")
                .and_then(|x| x.as_str())
                .ok_or_else(|| format!("entry `{fname}` instantiation missing `mangled`"))?;
            Ok(m.to_string())
        }
        n => Err(format!("ambiguous: {n} leo4 exports match `{fname}`")),
    }
}

/// P5-b₂'s supported scalar set.
#[derive(Copy, Clone, Debug)]
enum ScalarKind {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Bool,
    Char,
}

impl ScalarKind {
    fn wire_size(self) -> usize {
        use ScalarKind::*;
        match self {
            U8 | I8 | Bool => 1,
            U16 | I16 => 2,
            U32 | I32 | F32 | Char => 4,
            U64 | I64 | F64 => 8,
        }
    }

    fn rust_type(self) -> TokenStream {
        use ScalarKind::*;
        match self {
            U8 => quote!(u8),
            U16 => quote!(u16),
            U32 => quote!(u32),
            U64 => quote!(u64),
            I8 => quote!(i8),
            I16 => quote!(i16),
            I32 => quote!(i32),
            I64 => quote!(i64),
            F32 => quote!(f32),
            F64 => quote!(f64),
            Bool => quote!(bool),
            Char => quote!(char),
        }
    }

    /// Call-site code that turns the named value into `[u8; N]`
    /// suitable for `extend_from_slice`. Booleans are emitted as
    /// `[0u8]` / `[1u8]` per SPEC/canonical-abi.md §1. `char` is
    /// emitted as `u32::to_le_bytes(codepoint)`.
    fn to_le_bytes_call(self, ident: &syn::Ident) -> TokenStream {
        use ScalarKind::*;
        match self {
            Bool => quote! { [if #ident { 1u8 } else { 0u8 }] },
            Char => quote! { (#ident as u32).to_le_bytes() },
            U8 => quote! { [#ident] },
            I8 => quote! { [#ident as u8] },
            _ => quote! { #ident.to_le_bytes() },
        }
    }

    /// Construction expression: take the buffer `[u8; N]` (named via
    /// the caller-supplied token) and produce a value of `self`'s
    /// Rust type.
    fn from_le_bytes_call(self, buf: TokenStream) -> TokenStream {
        use ScalarKind::*;
        match self {
            Bool => quote! { (#buf[0] != 0u8) },
            Char => quote! {
                core::char::from_u32(u32::from_le_bytes(#buf))
                    .ok_or_else(|| ::leo4::LeanError {
                        code: 1,
                        detail: "invalid char codepoint on the wire".into(),
                    })?
            },
            U8 => quote! { #buf[0] },
            I8 => quote! { #buf[0] as i8 },
            U16 => quote! { u16::from_le_bytes(#buf) },
            I16 => quote! { i16::from_le_bytes(#buf) },
            U32 => quote! { u32::from_le_bytes(#buf) },
            I32 => quote! { i32::from_le_bytes(#buf) },
            U64 => quote! { u64::from_le_bytes(#buf) },
            I64 => quote! { i64::from_le_bytes(#buf) },
            F32 => quote! { f32::from_le_bytes(#buf) },
            F64 => quote! { f64::from_le_bytes(#buf) },
        }
    }
}

fn classify_scalar(ty: &Type) -> Option<ScalarKind> {
    let Type::Path(p) = ty else { return None };
    if p.qself.is_some() || p.path.segments.len() != 1 {
        return None;
    }
    let ident = p.path.segments[0].ident.to_string();
    use ScalarKind::*;
    Some(match ident.as_str() {
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
        _ => return None,
    })
}

#[allow(dead_code)]
fn _itemfn_doc_anchor(_: ItemFn) {}
