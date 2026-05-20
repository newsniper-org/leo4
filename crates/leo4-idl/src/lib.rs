//! leo4-idl
//!
//! Rust-side counterpart to `lake/Leo4Plugin/`. Two consumers right now:
//!   * `leo4c` CLI — `leo4c {parse,canonical,mangle,lower} <file>`
//!   * `tests/mangling/` — cross-implementation conformance against the
//!     Lean plugin.
//!
//! Since the schema-idl split, the domain-neutral grammar / parser /
//! `IDLType` / mangling / hash live in the `schema-idl` crate. This crate
//! keeps the leo4-specific bits:
//!
//!   * `wit` — WIT (Wasm Component Model) lowering pass, including the
//!     cyclic-ADT-to-resource fallback. Specific to leo4 because the
//!     wire-format contract is canonical-ABI, which is what lowers
//!     cleanly to WIT's flat layout.
//!
//! and re-exports the `schema-idl` top-level symbols at the names
//! downstream consumers (`leo4c`, cross-impl tests) historically used.
//! Code that wants only the type-system layer should depend on
//! `schema-idl` directly.
//!
//! Normative references:
//!   * `SPEC/mangling.md`     — type encoding, hash, name format
//!   * `SPEC/idl-grammar.ebnf` — syntax
//!   * `SPEC/handshake.md`    — JSON shapes
//!   * `LEO4-DESIGN.md`       — admit-set rules, kind discipline
//!   * `/for-general-interface-descriptions.md` — rationale for the
//!     schema-idl / leo4-idl split

pub mod wit;

// Re-export the schema-idl surface so the original `leo4_idl::…` call
// sites keep working. Downstream consumers may equivalently depend on
// `schema-idl` directly.
pub use schema_idl::{
    collapse_whitespace, fqn_seg, idl_form, instantiate_record, instantiate_variant,
    mangle, mangle_type, parse, parse_raw, render_canonical, resolve, substitute,
    user_decl_to_idl, FuncDecl, Hash, IDLType, ParseError, RawSchema, ResolveError,
    Schema, UserDecl,
};

pub use wit::{kebab_case, lower as lower_to_wit};
