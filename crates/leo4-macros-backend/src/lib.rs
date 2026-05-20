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
    FnArg, ReturnType, Signature,
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

    // Look up the mangled body. P5-b₂ keeps the simple "single
    // instantiation by fname" match — generic exports come back at
    // P5-b₃ with full arg-type-based disambiguation.
    let mangled_body = lookup_mangled_body(mangling, &fname).map_err(|e| {
        syn::Error::new_spanned(&sig.ident, e)
    })?;

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

    // P5-b₂ minimum: fixed 4096-byte return buffer. P5-b₃ adds a
    // grow-on-LEO4_ERR_RETURN_BUF_TOO_SMALL retry loop.
    Ok(quote! {
        #vis fn #fname_ident(
            #lean: &::leo4::Lean,
            #(#arg_decls),*
        ) -> ::core::result::Result<#ret_ty, ::leo4::LeanError> {
            let mut args: ::std::vec::Vec<u8> = ::std::vec::Vec::new();
            #(#encode_stmts)*
            let mut ret: ::std::vec::Vec<u8> = ::std::vec::Vec::with_capacity(4096);
            ret.resize(4096, 0u8);
            let written = #lean.call_shim(#mangled_body, &args, &mut ret)?;
            ret.truncate(written);
            let (value, _consumed) =
                <#ret_ty as ::leo4::LeanMarshal>::canonical_decode(&ret, 0)?;
            ::core::result::Result::Ok(value)
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

