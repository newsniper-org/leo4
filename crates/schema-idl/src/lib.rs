//! schema-idl
//!
//! Domain-neutral core of the IDL originally developed for leo4
//! (`/for-general-interface-descriptions.md`). The pieces in this
//! crate are independent of any host language and of any specific
//! wire format:
//!
//!   * `idl`      — [`IDLType`], [`UserDecl`], [`FuncDecl`], [`Schema`]
//!   * `parse`    — text → [`Schema`] via [`parse`]
//!   * `render`   — [`Schema`] → canonical / pretty IDL text
//!   * `mangle`   — [`mangle`], [`mangle_type`], [`fqn_seg`] per
//!                  `SPEC/mangling.md`
//!   * `hash`     — FNV-1a-64 over the normalized IDL bytes ([`Hash`])
//!   * `base32`   — lowercase RFC-4648 base32 (no padding)
//!
//! Consumers:
//!   * `leo4-idl` — adds the leo4-specific WIT lowering pass.
//!   * Future domain-specific descriptions (e.g. neural-network block
//!     interfaces) may depend on this crate directly and ship their
//!     own emitter / runtime / discovery layer.
//!
//! Normative references:
//!   * `SPEC/mangling.md`     — type encoding, hash, name format
//!   * `SPEC/idl-grammar.ebnf` — syntax
//!   * `SPEC/handshake.md`    — JSON shapes

pub mod hash;
pub mod base32;
pub mod idl;
pub mod mangle;
pub mod parse;
pub mod render;
pub mod subst;

pub use hash::Hash;
pub use idl::{FuncDecl, IDLType, Schema, UserDecl};
pub use mangle::{fqn_seg, mangle, mangle_type};
pub use parse::{parse, parse_raw, resolve, ParseError, RawSchema, ResolveError};
pub use render::{collapse_whitespace, idl_form, render_canonical, user_decl_to_idl};
pub use subst::{instantiate_record, instantiate_variant, substitute};
