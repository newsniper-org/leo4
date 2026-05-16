//! leo4-idl
//!
//! Rust-side counterpart to `lake/Leo4Plugin/`. Two consumers right now:
//!   * `leo4c` CLI — `leo4c {parse,canonical,mangle} <file>`
//!   * `tests/mangling/` — cross-implementation conformance against the
//!     Lean plugin.
//!
//! The on-the-wire artefacts (`<pkg>.leo4-schema`,
//! `<pkg>.leo4-mangling`, `<pkg>.leo4-handshake`) are the integration
//! seam. This crate must produce byte-identical output to
//! `Leo4Plugin.Mangling` for the same canonical IDL bytes.
//!
//! Normative references:
//!   * `SPEC/mangling.md`     — type encoding, hash, name format
//!   * `SPEC/idl-grammar.ebnf` — syntax
//!   * `SPEC/handshake.md`    — JSON shapes
//!   * `LEO4-DESIGN.md`       — admit-set rules, kind discipline

pub mod hash;
pub mod base32;
pub mod idl;
pub mod mangle;
pub mod parse;
pub mod render;

pub use hash::Hash;
pub use idl::{FuncDecl, IDLType, Schema, UserDecl};
pub use mangle::{fqn_seg, mangle, mangle_type};
pub use parse::{parse, parse_raw, resolve, ParseError, RawSchema, ResolveError};
pub use render::{collapse_whitespace, idl_form, render_canonical, user_decl_to_idl};
