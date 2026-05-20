//! IDL type model. Mirrors `Leo4Plugin.AdmitSet.IDLType` and `UserDecl`.
//!
//! `record`/`variant`/`enum`/`resource` carry the **dotted FQN**
//! (e.g. `Sample.Point`); `mangle` translates dots to underscores per
//! `SPEC/mangling.md` §2.

/// IDL types reachable across the leo4 boundary.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IDLType {
    U8, U16, U32, U64,
    I8, I16, I32, I64,
    F32, F64,
    Bool, Char, String,
    BigInt, BigNat,
    /// `list<T>`
    List(Box<IDLType>),
    /// `option<T>`
    Option(Box<IDLType>),
    /// `result<T>` (Err none) or `result<T, E>` (Err some)
    Result(Box<IDLType>, Option<Box<IDLType>>),
    /// `tuple<T1,…,Tn>`
    Tuple(Vec<IDLType>),
    /// `record FQN<args>` — generic args follow declaration order
    Record { fqn: String, args: Vec<IDLType> },
    /// `variant FQN<args>`
    Variant { fqn: String, args: Vec<IDLType> },
    /// `enum FQN`
    Enum(String),
    /// `flags FQN`
    Flags(String),
    /// `resource FQN<args>` — opaque `u64` handle on the wire
    Resource { fqn: String, args: Vec<IDLType> },
    /// `io<T>` — sync today, lowers to `result<T, error>`
    Io(Box<IDLType>),
    /// `Self` — identity-substitution sugar for `Self<X1, …, Xn>` where
    /// each `Xi` is the enclosing's own generic parameter.
    Self_,
    /// `Self<T1, …, Tn>` — explicit substitution at a self-reference.
    /// Mangles as `self_<…>_x` per SPEC/mangling.md §"Self and Self<…>".
    SelfApp(Vec<IDLType>),
}

/// User-defined nominal type declarations the plugin discovers by walking
/// the user package's `LeanMarshal`/`LeanResource` instances.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum UserDecl {
    Record {
        fqn: String,
        generics: Vec<String>,
        fields: Vec<(String, IDLType)>,
    },
    Enum {
        fqn: String,
        cases: Vec<String>,
    },
    Variant {
        fqn: String,
        generics: Vec<String>,
        cases: Vec<(String, Vec<IDLType>)>,
    },
    Resource {
        fqn: String,
        generics: Vec<String>,
    },
    /// `flags F<…> { read, write, exec }` — bitfield-of-named-flags.
    /// SPEC/idl-grammar.ebnf line 47. The pre-2026-05-20 parser
    /// collapsed `flags` declarations onto `UserDecl::Enum`, losing the
    /// flags/enum distinction; this dedicated variant restores it.
    Flags {
        fqn: String,
        generics: Vec<String>,
        members: Vec<String>,
    },
}

impl UserDecl {
    #[must_use]
    pub fn fqn(&self) -> &str {
        match self {
            UserDecl::Record { fqn, .. }
            | UserDecl::Enum { fqn, .. }
            | UserDecl::Variant { fqn, .. }
            | UserDecl::Resource { fqn, .. }
            | UserDecl::Flags { fqn, .. } => fqn,
        }
    }
}

/// Function-level effect (`SPEC/mangling.md` D-i 2026-05-19,
/// `LEO4-DESIGN.md` D4). Async / streaming are *boundary-only*
/// modifiers; they never appear as `IDLType` variants inside record /
/// variant / list payloads. The parser desugars
/// `func foo(…) -> future<T>;` and `… -> stream<T>;` into
/// `FuncDecl { effect: Async / Stream, ret: T }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Effect {
    #[default]
    Sync,
    Async,
    Stream,
}

/// A function exported across the boundary, post-resolution.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FuncDecl {
    pub name: String,
    pub params: Vec<(String, IDLType)>,
    pub ret: IDLType,
    /// Function-level effect; defaults to `Sync` for pre-Phase-7
    /// code paths. Phase 7 wires this into the shim emitter.
    #[allow(unused)]
    pub effect: Effect,
}

/// A fully-resolved IDL schema. `parse::parse(...)` returns one of these.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Schema {
    pub package: String,
    pub interface: String,
    pub user_decls: Vec<UserDecl>,
    pub funcs: Vec<FuncDecl>,
}
