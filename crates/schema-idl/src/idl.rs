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
    /// `Cyc<i>` — cycle-breaker reference to the `i`-th member of the
    /// enclosing mutual group (Phase 6, `SPEC/phase-6-mutual.md` §2).
    /// `i` is 0-based and scoped to the immediately enclosing
    /// `mutual { … }` block; resolvers reject `Cyc<i>` references that
    /// escape their group or that point past the group's last member.
    Cyc(u32),
    /// `fn(T1, …, Tn) -> R` — first-class function-arrow type (Phase
    /// 10-B1, 2026-05-21). Wire format is a single `u64` callback_id
    /// per `SPEC/canonical-abi.md` §15. Mangles as
    /// `A_<tuple-of-args>_<ret>_a` per `SPEC/mangling.md` §2 (filling
    /// the previously-TBD function-arrow slot).
    ///
    /// The IDL parser accepts the surface syntax `fn(T1, T2) -> R`
    /// at any type position. Re-entrant dispatch (Lean ↔ Rust
    /// callbacks) is a runtime concern landed in a later substep
    /// (B1.x); the IDL / mangling layer is callable on its own for
    /// cross-impl conformance.
    Fn { args: Vec<IDLType>, ret: Box<IDLType> },
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
    /// `mutual { … }` block — Phase 6 cross-decl recursion cluster
    /// (`SPEC/phase-6-mutual.md` §1). Members share a `Cyc<i>`
    /// namespace, i.e. `IDLType::Cyc(i)` references inside any
    /// member resolve to `members[i as usize]`. A `Mutual` group
    /// must have ≥ 2 members; the singleton case is a parse error
    /// and authors keep using `Self`.
    Mutual {
        members: Vec<UserDecl>,
    },
    /// Phase 8 step 2: a nominal type with a custom `LeanMarshal`
    /// instance whose fields the plugin can't (or shouldn't) lower
    /// — proof-carrying invariants (`Rat`'s `den_nz`, `reduced`),
    /// opaque wrappers, etc. The shim routes encode/decode through
    /// C-callable Lean helpers (`leo4_marshal_<T>_dec/enc`) emitted
    /// in step 2b; at the *type-reference* level (function params /
    /// return), the IDL still uses `IDLType::Record { fqn, args }`,
    /// so the mangling and IDL form for refs match a regular record.
    /// The distinction lives at this `UserDecl` level and is what
    /// the shim emitter dispatches on.
    ExternalMarshal {
        fqn: String,
        generics: Vec<String>,
    },
}

impl UserDecl {
    /// Member FQN for non-mutual decls. Returns `""` for a `Mutual`
    /// group — call `members()` and pick the desired index instead.
    #[must_use]
    pub fn fqn(&self) -> &str {
        match self {
            UserDecl::Record { fqn, .. }
            | UserDecl::Enum { fqn, .. }
            | UserDecl::Variant { fqn, .. }
            | UserDecl::Resource { fqn, .. }
            | UserDecl::Flags { fqn, .. }
            | UserDecl::ExternalMarshal { fqn, .. } => fqn,
            UserDecl::Mutual { .. } => "",
        }
    }

    /// Iterator over the leaf decls — a non-mutual decl yields just
    /// itself; a `Mutual` group yields its member decls in source
    /// order. Other passes that need a flat list of nominal types
    /// (e.g. handler resolution) should iterate through this.
    pub fn leaves(&self) -> Box<dyn Iterator<Item = &UserDecl> + '_> {
        match self {
            UserDecl::Mutual { members } => Box::new(members.iter()),
            other => Box::new(std::iter::once(other)),
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
