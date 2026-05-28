//! Recursive-descent parser for the on-disk `.leo4-schema` text format
//! emitted by `Leo4Plugin.Emit`. Two-pass: raw parse first, then a
//! shape-resolution pass that turns `Named { fqn }` references into
//! concrete `IDLType::Record`/`Variant`/`Enum`/`Resource`.
//!
//! Scope notes:
//!   * Comments / doc strings are not expected in the canonical form
//!     (the emitter strips them); the parser tolerates `// …` line
//!     comments defensively but `/* … */` is not yet supported.
//!   * Full `SPEC/idl-grammar.ebnf` (top-level `world`, `use`, free
//!     `type`, `constraint_decl`) is *not* covered yet — only what the
//!     Lake plugin emits, which is enough for `tests/mangling/`.
//!   * Higher-kind generics (`F : Type -> Type`) are out of scope of
//!     the text emit (the plugin rejects them at admit-set time); this
//!     parser similarly does not accept kind annotations.

use std::collections::HashMap;
use std::fmt;

use crate::idl::{Effect, FuncDecl, IDLType, Schema, UserDecl};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    UnexpectedEof { at: usize },
    Expected { at: usize, what: String },
    InvalidIdent { at: usize },
    UnknownTypeKeyword { at: usize, found: String },
    TrailingInput { at: usize },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnexpectedEof { at } => write!(f, "unexpected EOF at byte {at}"),
            ParseError::Expected { at, what } => {
                write!(f, "expected {what} at byte {at}")
            }
            ParseError::InvalidIdent { at } => write!(f, "invalid identifier at byte {at}"),
            ParseError::UnknownTypeKeyword { at, found } => {
                write!(f, "unknown type keyword `{found}` at byte {at}")
            }
            ParseError::TrailingInput { at } => write!(f, "trailing input after schema at byte {at}"),
        }
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    UnknownNominal { fqn: String },
    EnumWithArgs { fqn: String },
    FlagsWithArgs { fqn: String },
    /// `future<T>` / `stream<T>` only valid at function-boundary
    /// position (D-i 2026-05-19). Found inside a payload type.
    EffectInPayload { kind: &'static str },
    /// `Cyc<i>` used outside any `mutual { … }` block. The token is
    /// scoped to the enclosing group only (SPEC/phase-6-mutual.md §2).
    CycOutsideMutual { index: u32 },
    /// `Cyc<i>` with `i ≥ group_size` of the enclosing mutual group.
    CycIndexOutOfRange { index: u32, group_size: usize },
    /// A `mutual { … }` block with fewer than two members. Singletons
    /// should drop the brackets and use `Self` (§1).
    MutualGroupTooSmall { size: usize },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::UnknownNominal { fqn } => {
                write!(f, "unknown nominal type `{fqn}` (no record/enum/variant/resource/flags decl)")
            }
            ResolveError::EnumWithArgs { fqn } => {
                write!(f, "enum `{fqn}` cannot take generic arguments")
            }
            ResolveError::FlagsWithArgs { fqn } => {
                write!(f, "flags `{fqn}` cannot take generic arguments")
            }
            ResolveError::EffectInPayload { kind } => {
                write!(
                    f,
                    "`{kind}<T>` is only valid at a function's return position, not inside a payload"
                )
            }
            ResolveError::CycOutsideMutual { index } => {
                write!(
                    f,
                    "`Cyc<{index}>` used outside any `mutual {{ … }}` block"
                )
            }
            ResolveError::CycIndexOutOfRange { index, group_size } => {
                write!(
                    f,
                    "`Cyc<{index}>` references position {index} but the enclosing mutual group has {group_size} member(s)"
                )
            }
            ResolveError::MutualGroupTooSmall { size } => {
                write!(
                    f,
                    "`mutual {{ … }}` block has {size} member(s); a group must have at least 2 (use `Self` for singletons)"
                )
            }
        }
    }
}

impl std::error::Error for ResolveError {}

/// Pre-resolution type expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawType {
    /// Builtin shapes (primitives, list/option/result/tuple/io, `Self`).
    /// Recursion through these doesn't go through nominal lookup.
    Builtin(IDLType),
    /// A nominal reference: `Sample.Point`, `My.Kv.Pair<u32, string>`, …
    Named { fqn: String, args: Vec<RawType> },
    /// `Cyc<n>` — Phase 6 cycle-breaker token. Resolution verifies that
    /// the enclosing scope is a `mutual_decl` member and that `n` is in
    /// range; bare top-level / non-mutual scopes are rejected.
    Cyc(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawDecl {
    Record {
        fqn: String,
        generics: Vec<String>,
        fields: Vec<(String, RawType)>,
    },
    Enum {
        fqn: String,
        cases: Vec<String>,
    },
    Variant {
        fqn: String,
        generics: Vec<String>,
        cases: Vec<(String, Vec<RawType>)>,
    },
    Resource {
        fqn: String,
        generics: Vec<String>,
    },
    Flags {
        fqn: String,
        generics: Vec<String>,
        members: Vec<String>,
    },
    /// `mutual { decl₀; decl₁; … }` — Phase 6 cluster of ≥ 2 nominal
    /// decls sharing a `Cyc<i>` namespace. Bracket-form mirrors Lean's
    /// `mutual … end`. Singletons are rejected by the resolver.
    Mutual {
        members: Vec<RawDecl>,
    },
    /// `external <fqn>[<generics>];` — Phase 8 step 2. Type with a
    /// custom `LeanMarshal` instance whose wire format is opaque to
    /// the IDL layer. References to this type at usage sites
    /// (`func f(_0: Foo) -> Foo`) parse as bare nominal refs.
    ExternalMarshal {
        fqn: String,
        generics: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawFunc {
    pub name: String,
    pub params: Vec<(String, RawType)>,
    pub ret: RawType,
    /// Function-level effect (D-i 2026-05-19). Set by
    /// `parse_func_decl` when the return position is wrapped in
    /// `future<…>` (`Async`) or `stream<…>` (`Stream`); `Sync`
    /// otherwise. The inner `T` is unwrapped into `ret`. Effects
    /// are NOT allowed inside payload types — `parse_type` rejects
    /// `future` / `stream` keywords anywhere except a func's
    /// immediate return position.
    pub effect: Effect,
}

/// A `use path [as ident];` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UseDeclRaw {
    /// Dotted/colon-segmented path, raw text.
    pub path: String,
    /// Optional rename via `as <ident>`.
    pub alias: Option<String>,
}

/// A `type Name [generic_params] = type;` alias.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeAliasRaw {
    pub name: String,
    pub generics: Vec<GenericParamRaw>,
    pub body: RawType,
}

/// A `constraint Name = body;` declaration. v0 keeps the body raw text;
/// constraint evaluation lives in the plugin / Phase 4.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstraintDeclRaw {
    pub name: String,
    /// Verbatim body text (whitespace-collapsed). Parsed lazily by the
    /// plugin's constraint-evaluator; the parser only confirms balanced
    /// braces.
    pub body: String,
}

/// A single `interface Name [generic_params] { … }` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterfaceRaw {
    pub name: String,
    pub generics: Vec<GenericParamRaw>,
    pub decls: Vec<RawDecl>,
    pub funcs: Vec<RawFunc>,
    pub use_decls: Vec<UseDeclRaw>,
}

/// A `world Name { … }` block. Currently parsed but not used by the
/// downstream resolver — Phase 3+ WIT lowering revisits this.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldRaw {
    pub name: String,
    /// Verbatim body lines, post-whitespace collapse.
    pub items: Vec<String>,
}

/// `generic_params` entry, pre-resolution. The body fields keep raw
/// text for the `:` annotation; the Phase 1 `Schema` collapses this to
/// just the param name (kind/value erasure lives at admit-set time).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenericParamRaw {
    /// `T : kind` or bare `T` (defaults to `Type`).
    Type {
        name: String,
        kind: Option<RawKind>,
        constraint: Option<String>,
    },
    /// `n : <value-type>` — dependent value generic, erased at the boundary.
    Value { name: String, ty: RawType },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawKind {
    Type,
    Arrow(Box<RawKind>, Box<RawKind>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawSchema {
    pub package: String,
    /// Convenience: name of the first interface, if any (else "").
    pub interface: String,
    /// Convenience: all nominal decls from top level and from the first
    /// interface, in source order with top-level first.
    pub decls: Vec<RawDecl>,
    /// Convenience: funcs from the first interface.
    pub funcs: Vec<RawFunc>,
    /// Full document detail (Phase 2+).
    pub use_decls: Vec<UseDeclRaw>,
    pub type_decls: Vec<TypeAliasRaw>,
    pub constraint_decls: Vec<ConstraintDeclRaw>,
    pub interfaces: Vec<InterfaceRaw>,
    pub worlds: Vec<WorldRaw>,
}

/// Top-level entry: parse and resolve in one shot.
pub fn parse(input: &str) -> Result<Schema, Box<dyn std::error::Error>> {
    let raw = parse_raw(input)?;
    let schema = resolve(&raw)?;
    Ok(schema)
}

/// Parse to `RawSchema` without resolving named types. Accepts the full
/// `SPEC/idl-grammar.ebnf` `document` shape (`package_decl`, `use_decl`,
/// `interface_decl`, `world_decl`, top-level `type_decl`/`constraint_decl`/
/// `nominal_decl`). The Lake plugin currently emits exactly one
/// interface; multiple interfaces / worlds are accepted at parse time
/// for tools that hand-write IDL.
pub fn parse_raw(input: &str) -> Result<RawSchema, ParseError> {
    let mut p = Parser::new(input);
    p.skip_ws();

    // ── package_decl (required, exactly one) ─────────────────────────
    p.expect_keyword("package")?;
    let pkg_name = p.parse_ident_or_kebab()?;
    p.skip_ws();
    let pkg_subnamespace = if p.peek_char() == Some(':') {
        p.pos += 1;
        Some(p.parse_ident_or_kebab()?)
    } else {
        None
    };
    p.skip_ws();
    if p.peek_char() == Some('@') {
        // semver suffix; parsed but not retained — informational on the wire.
        p.pos += 1;
        let _ver = p.parse_semver_segment()?;
    }
    p.expect_char(';')?;

    let package = match pkg_subnamespace {
        Some(sub) => format!("{pkg_name}:{sub}"),
        None => pkg_name,
    };

    // ── top-level items, until EOF ────────────────────────────────────
    let mut top_use_decls: Vec<UseDeclRaw> = Vec::new();
    let mut top_type_decls: Vec<TypeAliasRaw> = Vec::new();
    let mut top_constraint_decls: Vec<ConstraintDeclRaw> = Vec::new();
    let mut top_nominal_decls: Vec<RawDecl> = Vec::new();
    let mut interfaces: Vec<InterfaceRaw> = Vec::new();
    let mut worlds: Vec<WorldRaw> = Vec::new();

    loop {
        p.skip_ws();
        if p.at_end() {
            break;
        }
        if p.peek_keyword("use") {
            top_use_decls.push(p.parse_use_decl()?);
        } else if p.peek_keyword("interface") {
            interfaces.push(p.parse_interface_decl()?);
        } else if p.peek_keyword("world") {
            worlds.push(p.parse_world_decl()?);
        } else if p.peek_keyword("type") {
            top_type_decls.push(p.parse_type_alias_decl()?);
        } else if p.peek_keyword("constraint") {
            top_constraint_decls.push(p.parse_constraint_decl()?);
        } else if p.peek_keyword("record")
            || p.peek_keyword("variant")
            || p.peek_keyword("enum")
            || p.peek_keyword("resource")
            || p.peek_keyword("flags")
            || p.peek_keyword("mutual")
            || p.peek_keyword("external")
        {
            top_nominal_decls.push(p.parse_nominal_decl()?);
        } else {
            return Err(ParseError::Expected {
                at: p.pos,
                what: "top-level item (`use`/`interface`/`world`/`type`/`constraint`/nominal decl)".into(),
            });
        }
    }

    // For backward-compat with the rest of this crate (which expects one
    // interface), pick the first interface as *the* interface and inline
    // its nominal/func decls. Hand-written IDL with multiple interfaces
    // is fine for `parse_raw`; downstream callers that need the full
    // structure read `RawSchema.interfaces` directly.
    let (interface_name, decls, funcs) = match interfaces.first() {
        Some(iface) => {
            let mut decls: Vec<RawDecl> = top_nominal_decls.clone();
            decls.extend(iface.decls.iter().cloned());
            (iface.name.clone(), decls, iface.funcs.clone())
        }
        None => (String::new(), top_nominal_decls.clone(), Vec::new()),
    };

    Ok(RawSchema {
        package,
        interface: interface_name,
        decls,
        funcs,
        use_decls: top_use_decls,
        type_decls: top_type_decls,
        constraint_decls: top_constraint_decls,
        interfaces,
        worlds,
    })
}

/// Resolve all `Named { fqn, args }` references against the user
/// declarations the schema itself provides.
pub fn resolve(raw: &RawSchema) -> Result<Schema, ResolveError> {
    let shapes = build_shape_map(&raw.decls);
    let user_decls = raw
        .decls
        .iter()
        .map(|d| resolve_decl(d, &shapes, None))
        .collect::<Result<Vec<_>, _>>()?;
    let funcs = raw
        .funcs
        .iter()
        .map(|f| {
            let params = f
                .params
                .iter()
                .map(|(n, t)| Ok::<_, ResolveError>((n.clone(), resolve_type(t, &shapes, None)?)))
                .collect::<Result<Vec<_>, _>>()?;
            let ret = resolve_type(&f.ret, &shapes, None)?;
            Ok::<_, ResolveError>(FuncDecl {
                name: f.name.clone(),
                params,
                ret,
                // D-i 2026-05-19: function-level effect. Phase 7
                // step 1 (2026-05-20) wires the parser-side
                // `future<T>` / `stream<T>` desugar into this slot.
                effect: f.effect,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Schema {
        package: raw.package.clone(),
        interface: raw.interface.clone(),
        user_decls,
        funcs,
    })
}

#[derive(Copy, Clone, Debug)]
enum Shape {
    Record,
    Enum,
    Variant,
    Resource,
    Flags,
    /// In-scope generic type parameter of the enclosing nominal decl.
    /// `resolve_decl` injects entries of this shape into a local copy
    /// of the shape map before walking field / case types so a bare
    /// `T0` / `T1` reference resolves to a nullary
    /// `IDLType::Record { fqn: <name>, args: vec![] }` placeholder,
    /// matching the form `Subst.substIDL` (and its Rust mirror) later
    /// substitutes against concrete instance args.
    TypeVar,
}

fn build_shape_map(decls: &[RawDecl]) -> HashMap<String, Shape> {
    let mut map = HashMap::with_capacity(decls.len());
    for d in decls {
        insert_shape_entries(&mut map, d);
    }
    map
}

fn insert_shape_entries(map: &mut HashMap<String, Shape>, d: &RawDecl) {
    match d {
        RawDecl::Record { fqn, .. } => {
            map.insert(fqn.clone(), Shape::Record);
        }
        RawDecl::Enum { fqn, .. } => {
            map.insert(fqn.clone(), Shape::Enum);
        }
        RawDecl::Variant { fqn, .. } => {
            map.insert(fqn.clone(), Shape::Variant);
        }
        RawDecl::Resource { fqn, .. } => {
            map.insert(fqn.clone(), Shape::Resource);
        }
        RawDecl::Flags { fqn, .. } => {
            map.insert(fqn.clone(), Shape::Flags);
        }
        // External-marshal nominal: type references at function
        // sites resolve as Record-shaped IDLType refs (the IDL
        // doesn't carry layout info). Shape tag mirrors that.
        RawDecl::ExternalMarshal { fqn, .. } => {
            map.insert(fqn.clone(), Shape::Record);
        }
        // Mutual groups: register each member's FQN/shape under the
        // top-level shape map so peer references via FQN (when the
        // author uses an explicit name instead of `Cyc<i>`) resolve
        // correctly. The Cyc form is preferred but the FQN path
        // remains valid for nominal references *into* the group from
        // outside it.
        RawDecl::Mutual { members } => {
            for m in members {
                insert_shape_entries(map, m);
            }
        }
    }
}

/// Phase 6 mutual-group scope passed down through `resolve_type` /
/// `resolve_decl`. `Some(n)` ⇒ we are inside a mutual block of `n`
/// members; `None` ⇒ outside, `Cyc<i>` is a hard error.
type MutualCtx = Option<usize>;

fn resolve_type(
    t: &RawType,
    shapes: &HashMap<String, Shape>,
    mctx: MutualCtx,
) -> Result<IDLType, ResolveError> {
    match t {
        RawType::Builtin(b) => Ok(b.clone()),
        RawType::Cyc(i) => match mctx {
            None => Err(ResolveError::CycOutsideMutual { index: *i }),
            Some(group_size) => {
                if (*i as usize) < group_size {
                    Ok(IDLType::Cyc(*i))
                } else {
                    Err(ResolveError::CycIndexOutOfRange {
                        index: *i,
                        group_size,
                    })
                }
            }
        },
        RawType::Named { fqn, args } => {
            let args_idl = args
                .iter()
                .map(|a| resolve_type(a, shapes, mctx))
                .collect::<Result<Vec<_>, _>>()?;
            match shapes.get(fqn) {
                Some(Shape::Record) => Ok(IDLType::Record {
                    fqn: fqn.clone(),
                    args: args_idl,
                }),
                Some(Shape::Enum) => {
                    if !args_idl.is_empty() {
                        return Err(ResolveError::EnumWithArgs { fqn: fqn.clone() });
                    }
                    Ok(IDLType::Enum(fqn.clone()))
                }
                Some(Shape::Variant) => Ok(IDLType::Variant {
                    fqn: fqn.clone(),
                    args: args_idl,
                }),
                Some(Shape::Resource) => Ok(IDLType::Resource {
                    fqn: fqn.clone(),
                    args: args_idl,
                }),
                Some(Shape::Flags) => {
                    if !args_idl.is_empty() {
                        return Err(ResolveError::FlagsWithArgs { fqn: fqn.clone() });
                    }
                    Ok(IDLType::Flags(fqn.clone()))
                }
                Some(Shape::TypeVar) => Ok(IDLType::Record {
                    fqn: fqn.clone(),
                    args: vec![],
                }),
                None => Err(ResolveError::UnknownNominal { fqn: fqn.clone() }),
            }
        }
    }
}

/// Augment `shapes` with `TypeVar` entries for the enclosing decl's
/// generic-parameter binders. Used so a field type that references
/// `T0` resolves to a nullary placeholder rather than `UnknownNominal`.
fn shapes_with_typars(
    base: &HashMap<String, Shape>,
    generics: &[String],
) -> HashMap<String, Shape> {
    let mut map = base.clone();
    for g in generics {
        map.insert(g.clone(), Shape::TypeVar);
    }
    map
}

fn resolve_decl(
    d: &RawDecl,
    shapes: &HashMap<String, Shape>,
    mctx: MutualCtx,
) -> Result<UserDecl, ResolveError> {
    match d {
        RawDecl::Record { fqn, generics, fields } => {
            let local = shapes_with_typars(shapes, generics);
            let fields = fields
                .iter()
                .map(|(n, t)| Ok::<_, ResolveError>((n.clone(), resolve_type(t, &local, mctx)?)))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(UserDecl::Record {
                fqn: fqn.clone(),
                generics: generics.clone(),
                fields,
            })
        }
        RawDecl::Enum { fqn, cases } => Ok(UserDecl::Enum {
            fqn: fqn.clone(),
            cases: cases.clone(),
        }),
        RawDecl::Variant { fqn, generics, cases } => {
            let local = shapes_with_typars(shapes, generics);
            let cases = cases
                .iter()
                .map(|(n, payload)| {
                    let payload = payload
                        .iter()
                        .map(|t| resolve_type(t, &local, mctx))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok::<_, ResolveError>((n.clone(), payload))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(UserDecl::Variant {
                fqn: fqn.clone(),
                generics: generics.clone(),
                cases,
            })
        }
        RawDecl::Resource { fqn, generics } => Ok(UserDecl::Resource {
            fqn: fqn.clone(),
            generics: generics.clone(),
        }),
        RawDecl::Flags { fqn, generics, members } => Ok(UserDecl::Flags {
            fqn: fqn.clone(),
            generics: generics.clone(),
            members: members.clone(),
        }),
        RawDecl::ExternalMarshal { fqn, generics } => Ok(UserDecl::ExternalMarshal {
            fqn: fqn.clone(),
            generics: generics.clone(),
        }),
        RawDecl::Mutual { members } => {
            if members.len() < 2 {
                return Err(ResolveError::MutualGroupTooSmall {
                    size: members.len(),
                });
            }
            let group_size = members.len();
            let inner_ctx: MutualCtx = Some(group_size);
            // Phase 6: every member's field / case types resolve with
            // the mutual context set to the group's size so any
            // `Cyc<i>` inside the block can be bounds-checked. Nested
            // `mutual` blocks are caught by `parse_mutual_decl` so we
            // don't need to guard against them here.
            let resolved = members
                .iter()
                .map(|m| resolve_decl(m, shapes, inner_ctx))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(UserDecl::Mutual { members: resolved })
        }
    }
}

// ─── Parser internals ────────────────────────────────────────────────────

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            pos: 0,
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn peek_char(&self) -> Option<char> {
        self.input.get(self.pos).copied().map(char::from)
    }

    fn skip_ws(&mut self) {
        while self.pos < self.input.len() {
            let c = self.input[self.pos];
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else if c == b'/' && self.input.get(self.pos + 1) == Some(&b'/') {
                // line comment to end of line — defensive; canonical form has none.
                while self.pos < self.input.len() && self.input[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn expect_char(&mut self, c: char) -> Result<(), ParseError> {
        self.skip_ws();
        if self.peek_char() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(ParseError::Expected {
                at: self.pos,
                what: format!("`{c}`"),
            })
        }
    }

    /// Match keyword `s` only when the following byte is not an identifier
    /// continuation. e.g. `peek_keyword("u8")` against `"u8"` ⇒ true,
    /// against `"u8x"` ⇒ false. Does NOT advance `pos`.
    fn peek_keyword(&self, s: &str) -> bool {
        let bytes = s.as_bytes();
        let mut p = self.pos;
        // Allow leading whitespace for `peek_keyword` so callers don't have to
        // explicitly `skip_ws` first; but we do not consume it.
        while p < self.input.len() && self.input[p].is_ascii_whitespace() {
            p += 1;
        }
        if self.input.get(p..p + bytes.len()) != Some(bytes) {
            return false;
        }
        match self.input.get(p + bytes.len()) {
            Some(b) if is_ident_continue(*b) => false,
            _ => true,
        }
    }

    fn expect_keyword(&mut self, s: &str) -> Result<(), ParseError> {
        if !self.peek_keyword(s) {
            return Err(ParseError::Expected {
                at: self.pos,
                what: format!("keyword `{s}`"),
            });
        }
        self.skip_ws();
        self.pos += s.len();
        Ok(())
    }

    fn parse_ident(&mut self) -> Result<String, ParseError> {
        self.skip_ws();
        let start = self.pos;
        let Some(&first) = self.input.get(self.pos) else {
            return Err(ParseError::UnexpectedEof { at: self.pos });
        };
        if !is_ident_start(first) {
            return Err(ParseError::InvalidIdent { at: self.pos });
        }
        self.pos += 1;
        while self.pos < self.input.len() && is_ident_continue(self.input[self.pos]) {
            self.pos += 1;
        }
        Ok(std::str::from_utf8(&self.input[start..self.pos])
            .unwrap()
            .to_string())
    }

    /// Like `parse_ident` but also accepts kebab segments (`leo4-sample`).
    /// Used for `package` names.
    fn parse_ident_or_kebab(&mut self) -> Result<String, ParseError> {
        self.skip_ws();
        let start = self.pos;
        let Some(&first) = self.input.get(self.pos) else {
            return Err(ParseError::UnexpectedEof { at: self.pos });
        };
        if !is_ident_start(first) {
            return Err(ParseError::InvalidIdent { at: self.pos });
        }
        self.pos += 1;
        while self.pos < self.input.len() {
            let c = self.input[self.pos];
            if is_ident_continue(c) || c == b'-' {
                self.pos += 1;
            } else {
                break;
            }
        }
        Ok(std::str::from_utf8(&self.input[start..self.pos])
            .unwrap()
            .to_string())
    }

    fn parse_fqn(&mut self) -> Result<String, ParseError> {
        let mut s = self.parse_ident()?;
        while self.pos < self.input.len() && self.input[self.pos] == b'.' {
            self.pos += 1;
            let next = self.parse_ident()?;
            s.push('.');
            s.push_str(&next);
        }
        Ok(s)
    }

    fn parse_type(&mut self) -> Result<RawType, ParseError> {
        self.skip_ws();
        // Primitives & builtin generics, in keyword form.
        for (kw, ty) in PRIMITIVE_KEYWORDS {
            if self.peek_keyword(kw) {
                self.expect_keyword(kw)?;
                return Ok(RawType::Builtin(ty.clone()));
            }
        }
        if self.peek_keyword("list") {
            self.expect_keyword("list")?;
            self.expect_char('<')?;
            let t = self.parse_type()?;
            self.expect_char('>')?;
            return Ok(RawType::Builtin(IDLType::List(Box::new(raw_to_builtin(
                t,
            )?))));
        }
        if self.peek_keyword("option") {
            self.expect_keyword("option")?;
            self.expect_char('<')?;
            let t = self.parse_type()?;
            self.expect_char('>')?;
            return Ok(RawType::Builtin(IDLType::Option(Box::new(raw_to_builtin(
                t,
            )?))));
        }
        if self.peek_keyword("result") {
            self.expect_keyword("result")?;
            self.expect_char('<')?;
            let ok = self.parse_type()?;
            self.skip_ws();
            let err = if self.peek_char() == Some(',') {
                self.pos += 1;
                Some(Box::new(raw_to_builtin(self.parse_type()?)?))
            } else {
                None
            };
            self.expect_char('>')?;
            return Ok(RawType::Builtin(IDLType::Result(
                Box::new(raw_to_builtin(ok)?),
                err,
            )));
        }
        if self.peek_keyword("tuple") {
            self.expect_keyword("tuple")?;
            self.expect_char('<')?;
            let mut ts: Vec<IDLType> = Vec::new();
            loop {
                ts.push(raw_to_builtin(self.parse_type()?)?);
                self.skip_ws();
                if self.peek_char() == Some(',') {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            self.expect_char('>')?;
            return Ok(RawType::Builtin(IDLType::Tuple(ts)));
        }
        if self.peek_keyword("io") {
            self.expect_keyword("io")?;
            self.expect_char('<')?;
            let t = self.parse_type()?;
            self.expect_char('>')?;
            return Ok(RawType::Builtin(IDLType::Io(Box::new(raw_to_builtin(t)?))));
        }
        // Phase 10-B1: `fn(T1, …, Tn) -> R` first-class function-arrow type.
        if self.peek_keyword("fn") {
            self.expect_keyword("fn")?;
            self.skip_ws();
            self.expect_char('(')?;
            let mut args: Vec<IDLType> = Vec::new();
            self.skip_ws();
            if self.peek_char() != Some(')') {
                loop {
                    args.push(raw_to_builtin(self.parse_type()?)?);
                    self.skip_ws();
                    if self.peek_char() == Some(',') {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
            }
            self.expect_char(')')?;
            self.skip_ws();
            self.expect_char('-')?;
            self.expect_char('>')?;
            let ret = raw_to_builtin(self.parse_type()?)?;
            return Ok(RawType::Builtin(IDLType::Fn {
                args,
                ret: Box::new(ret),
            }));
        }
        // D-i 2026-05-19: `future<T>` and `stream<T>` are effect
        // markers, valid only at a func's immediate return position
        // (handled in `parse_func_decl`). Anywhere else — inside a
        // payload type — is a parse error.
        if self.peek_keyword("future") {
            return Err(ParseError::Expected {
                at: self.pos,
                what: "type (`future<T>` is only valid at a func's return position, not inside a payload)".into(),
            });
        }
        if self.peek_keyword("stream") {
            return Err(ParseError::Expected {
                at: self.pos,
                what: "type (`stream<T>` is only valid at a func's return position, not inside a payload)".into(),
            });
        }
        if self.peek_keyword("Self") {
            self.expect_keyword("Self")?;
            self.skip_ws();
            if self.peek_char() == Some('<') {
                self.expect_char('<')?;
                let mut args: Vec<IDLType> = Vec::new();
                loop {
                    args.push(raw_to_builtin(self.parse_type()?)?);
                    self.skip_ws();
                    if self.peek_char() == Some(',') {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                self.expect_char('>')?;
                return Ok(RawType::Builtin(IDLType::SelfApp(args)));
            }
            return Ok(RawType::Builtin(IDLType::Self_));
        }
        // Phase 6 cycle-breaker `Cyc<n>` — n is an ASCII-decimal
        // unsigned integer (no leading zeros). Validity in scope
        // is checked at resolve time, not parse time.
        if self.peek_keyword("Cyc") {
            self.expect_keyword("Cyc")?;
            self.expect_char('<')?;
            let n = self.parse_unsigned_int()?;
            self.expect_char('>')?;
            return Ok(RawType::Cyc(n));
        }
        // Nominal reference (FQN, optional generic args).
        let fqn = self.parse_fqn()?;
        self.skip_ws();
        let args = if self.peek_char() == Some('<') {
            self.pos += 1;
            let mut args: Vec<RawType> = Vec::new();
            loop {
                args.push(self.parse_type()?);
                self.skip_ws();
                if self.peek_char() == Some(',') {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            self.expect_char('>')?;
            args
        } else {
            vec![]
        };
        Ok(RawType::Named { fqn, args })
    }

    fn parse_enum_decl(&mut self) -> Result<RawDecl, ParseError> {
        self.expect_keyword("enum")?;
        let fqn = self.parse_fqn()?;
        let _generics = self.parse_optional_generic_params()?;
        self.expect_char('{')?;
        let mut cases: Vec<String> = Vec::new();
        loop {
            self.skip_ws();
            if self.peek_char() == Some('}') {
                break;
            }
            cases.push(self.parse_ident()?);
            self.skip_ws();
            if self.peek_char() == Some(',') {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.expect_char('}')?;
        self.expect_char(';')?;
        Ok(RawDecl::Enum { fqn, cases })
    }

    fn parse_record_decl(&mut self) -> Result<RawDecl, ParseError> {
        self.expect_keyword("record")?;
        let fqn = self.parse_fqn()?;
        let generics = self.parse_generic_param_names()?;
        self.expect_char('{')?;
        let mut fields: Vec<(String, RawType)> = Vec::new();
        loop {
            self.skip_ws();
            if self.peek_char() == Some('}') {
                break;
            }
            let name = self.parse_ident()?;
            self.expect_char(':')?;
            let ty = self.parse_type()?;
            fields.push((name, ty));
            self.skip_ws();
            if self.peek_char() == Some(',') {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.expect_char('}')?;
        self.expect_char(';')?;
        Ok(RawDecl::Record { fqn, generics, fields })
    }

    fn parse_variant_decl(&mut self) -> Result<RawDecl, ParseError> {
        self.expect_keyword("variant")?;
        let fqn = self.parse_fqn()?;
        let generics = self.parse_generic_param_names()?;
        self.expect_char('{')?;
        let mut cases: Vec<(String, Vec<RawType>)> = Vec::new();
        loop {
            self.skip_ws();
            if self.peek_char() == Some('}') {
                break;
            }
            let case_name = self.parse_ident()?;
            self.skip_ws();
            let payload = if self.peek_char() == Some('(') {
                self.pos += 1;
                let mut ts: Vec<RawType> = Vec::new();
                loop {
                    ts.push(self.parse_type()?);
                    self.skip_ws();
                    if self.peek_char() == Some(',') {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                self.expect_char(')')?;
                ts
            } else {
                vec![]
            };
            cases.push((case_name, payload));
            self.skip_ws();
            if self.peek_char() == Some(',') {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.expect_char('}')?;
        self.expect_char(';')?;
        Ok(RawDecl::Variant { fqn, generics, cases })
    }

    fn parse_resource_decl(&mut self) -> Result<RawDecl, ParseError> {
        self.expect_keyword("resource")?;
        let fqn = self.parse_fqn()?;
        let generics = self.parse_generic_param_names()?;
        self.skip_ws();
        // Optional `{ … }` body of methods — accepted, skip-balanced.
        // v0 plugin doesn't emit a body; Phase 3+ may consume it for
        // method dispatch.
        if self.peek_char() == Some('{') {
            self.pos += 1;
            let mut depth: i32 = 1;
            while self.pos < self.input.len() && depth > 0 {
                let c = self.input[self.pos];
                if c == b'{' {
                    depth += 1;
                } else if c == b'}' {
                    depth -= 1;
                }
                self.pos += 1;
            }
        }
        self.expect_char(';')?;
        Ok(RawDecl::Resource { fqn, generics })
    }

    fn parse_use_decl(&mut self) -> Result<UseDeclRaw, ParseError> {
        self.expect_keyword("use")?;
        let path = self.parse_path()?;
        self.skip_ws();
        let alias = if self.peek_keyword("as") {
            self.expect_keyword("as")?;
            Some(self.parse_ident()?)
        } else {
            None
        };
        self.expect_char(';')?;
        Ok(UseDeclRaw { path, alias })
    }

    fn parse_path(&mut self) -> Result<String, ParseError> {
        self.skip_ws();
        let mut s = self.parse_ident()?;
        loop {
            if self.peek_char() == Some(':') {
                self.pos += 1;
                let next = self.parse_ident()?;
                s.push(':');
                s.push_str(&next);
            } else if self.peek_char() == Some('/') {
                self.pos += 1;
                let next = self.parse_ident()?;
                s.push('/');
                s.push_str(&next);
            } else {
                break;
            }
        }
        Ok(s)
    }

    fn parse_semver_segment(&mut self) -> Result<String, ParseError> {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.input.len() {
            let c = self.input[self.pos];
            if c.is_ascii_digit() || c == b'.' || c == b'-' || c.is_ascii_alphabetic() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(ParseError::Expected {
                at: self.pos,
                what: "semver".into(),
            });
        }
        Ok(std::str::from_utf8(&self.input[start..self.pos])
            .unwrap()
            .to_string())
    }

    fn parse_type_alias_decl(&mut self) -> Result<TypeAliasRaw, ParseError> {
        self.expect_keyword("type")?;
        let name = self.parse_ident()?;
        let generics = self.parse_optional_generic_params()?;
        self.expect_char('=')?;
        let body = self.parse_type()?;
        self.expect_char(';')?;
        Ok(TypeAliasRaw {
            name,
            generics,
            body,
        })
    }

    fn parse_constraint_decl(&mut self) -> Result<ConstraintDeclRaw, ParseError> {
        self.expect_keyword("constraint")?;
        let name = self.parse_ident()?;
        self.expect_char('=')?;
        // Capture body verbatim up to the terminating `;` (respecting `{}` /
        // `()` nesting). Whitespace will be collapsed by callers that care.
        let body = self.parse_balanced_to_semicolon()?;
        self.expect_char(';')?;
        Ok(ConstraintDeclRaw { name, body })
    }

    fn parse_balanced_to_semicolon(&mut self) -> Result<String, ParseError> {
        self.skip_ws();
        let start = self.pos;
        let mut depth_brace = 0i32;
        let mut depth_paren = 0i32;
        let mut depth_angle = 0i32;
        while self.pos < self.input.len() {
            let c = self.input[self.pos];
            match c {
                b'{' => depth_brace += 1,
                b'}' => depth_brace -= 1,
                b'(' => depth_paren += 1,
                b')' => depth_paren -= 1,
                b'<' => depth_angle += 1,
                b'>' => depth_angle -= 1,
                b';' if depth_brace == 0 && depth_paren == 0 && depth_angle == 0 => break,
                _ => {}
            }
            self.pos += 1;
        }
        let s = std::str::from_utf8(&self.input[start..self.pos])
            .unwrap()
            .trim()
            .to_string();
        Ok(s)
    }

    fn parse_interface_decl(&mut self) -> Result<InterfaceRaw, ParseError> {
        self.expect_keyword("interface")?;
        let name = self.parse_ident()?;
        let generics = self.parse_optional_generic_params()?;
        self.expect_char('{')?;

        let mut decls: Vec<RawDecl> = Vec::new();
        let mut funcs: Vec<RawFunc> = Vec::new();
        let mut use_decls: Vec<UseDeclRaw> = Vec::new();
        loop {
            self.skip_ws();
            if self.peek_char() == Some('}') {
                break;
            }
            if self.peek_keyword("use") {
                use_decls.push(self.parse_use_decl()?);
            } else if self.peek_keyword("enum")
                || self.peek_keyword("record")
                || self.peek_keyword("variant")
                || self.peek_keyword("resource")
                || self.peek_keyword("flags")
                || self.peek_keyword("mutual")
                || self.peek_keyword("external")
            {
                decls.push(self.parse_nominal_decl()?);
            } else if self.peek_keyword("type") {
                // Long-form `type Name = …;` — flatten into a record-style decl
                // by parsing into the type alias, then dropping it into the
                // RawDecl stream when its body is a nominal shape. For other
                // shapes (`type X = list<u32>`) the alias is kept as a
                // type-alias item — but `parse_raw`'s downstream `Schema`
                // doesn't surface aliases yet, so we just lose them here.
                // Phase 3 WIT lowering will need them.
                let _alias = self.parse_type_alias_decl()?;
            } else if self.peek_keyword("func") {
                funcs.push(self.parse_func_decl()?);
            } else {
                return Err(ParseError::Expected {
                    at: self.pos,
                    what: "interface body item".into(),
                });
            }
            // Each sub-decl parser consumes its own trailing `;` (or `}` for
            // bodies). No extra `;` to eat here.
        }
        self.expect_char('}')?;
        Ok(InterfaceRaw {
            name,
            generics,
            decls,
            funcs,
            use_decls,
        })
    }

    fn parse_world_decl(&mut self) -> Result<WorldRaw, ParseError> {
        self.expect_keyword("world")?;
        let name = self.parse_ident()?;
        self.expect_char('{')?;
        // For v0 we capture each world_item line as verbatim text.
        let mut items: Vec<String> = Vec::new();
        loop {
            self.skip_ws();
            if self.peek_char() == Some('}') {
                break;
            }
            let item = self.parse_balanced_to_semicolon()?;
            self.expect_char(';')?;
            if !item.is_empty() {
                items.push(item);
            }
        }
        self.expect_char('}')?;
        Ok(WorldRaw { name, items })
    }

    fn parse_nominal_decl(&mut self) -> Result<RawDecl, ParseError> {
        if self.peek_keyword("enum") {
            self.parse_enum_decl()
        } else if self.peek_keyword("record") {
            self.parse_record_decl()
        } else if self.peek_keyword("variant") {
            self.parse_variant_decl()
        } else if self.peek_keyword("resource") {
            self.parse_resource_decl()
        } else if self.peek_keyword("flags") {
            self.parse_flags_decl()
        } else if self.peek_keyword("mutual") {
            self.parse_mutual_decl()
        } else if self.peek_keyword("external") {
            self.parse_external_decl()
        } else {
            Err(ParseError::Expected {
                at: self.pos,
                what: "nominal decl keyword".into(),
            })
        }
    }

    /// `external <fqn>[<generic_params>];` — Phase 8 step 2.
    /// External-marshal nominal: the wire format lives in a custom
    /// `LeanMarshal` instance, not in the IDL.
    fn parse_external_decl(&mut self) -> Result<RawDecl, ParseError> {
        self.expect_keyword("external")?;
        let fqn = self.parse_fqn()?;
        let generics = self.parse_generic_param_names()?;
        self.expect_char(';')?;
        Ok(RawDecl::ExternalMarshal { fqn, generics })
    }

    /// `mutual { nominal_decl nominal_decl … }` — Phase 6 cluster.
    /// The parser accepts any number of inner decls (including 0 or 1);
    /// the resolver enforces "≥ 2 members" (SPEC/phase-6-mutual.md §1).
    /// Nested `mutual` blocks are a parse error.
    fn parse_mutual_decl(&mut self) -> Result<RawDecl, ParseError> {
        self.expect_keyword("mutual")?;
        self.expect_char('{')?;
        let mut members: Vec<RawDecl> = Vec::new();
        loop {
            self.skip_ws();
            if self.peek_char() == Some('}') {
                break;
            }
            if self.peek_keyword("mutual") {
                return Err(ParseError::Expected {
                    at: self.pos,
                    what: "nested `mutual` blocks are not allowed".into(),
                });
            }
            members.push(self.parse_nominal_decl()?);
        }
        self.expect_char('}')?;
        // Trailing `;` matches the SPEC `mutual_decl = … "}" ";"` rule —
        // canonical IDL has the closing `};` after the cluster, just
        // like a nominal_decl. Singleton-group rejection (`< 2 members`)
        // happens later in `resolve_decl`.
        self.expect_char(';')?;
        Ok(RawDecl::Mutual { members })
    }

    /// Parse an ASCII-decimal unsigned integer with no leading sign and
    /// no leading-zero stutter. Used by `Cyc<n>`.
    fn parse_unsigned_int(&mut self) -> Result<u32, ParseError> {
        self.skip_ws();
        let start = self.pos;
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(ParseError::Expected {
                at: start,
                what: "unsigned decimal integer".into(),
            });
        }
        let s = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| ParseError::Expected {
                at: start,
                what: "ASCII decimal integer".into(),
            })?;
        s.parse::<u32>().map_err(|_| ParseError::Expected {
            at: start,
            what: format!("u32 (got `{s}`)"),
        })
    }

    fn parse_flags_decl(&mut self) -> Result<RawDecl, ParseError> {
        self.expect_keyword("flags")?;
        let fqn = self.parse_fqn()?;
        let generics = self.parse_generic_param_names()?;
        self.expect_char('{')?;
        let mut members: Vec<String> = Vec::new();
        loop {
            self.skip_ws();
            if self.peek_char() == Some('}') {
                break;
            }
            members.push(self.parse_ident()?);
            self.skip_ws();
            if self.peek_char() == Some(',') {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.expect_char('}')?;
        self.expect_char(';')?;
        Ok(RawDecl::Flags { fqn, generics, members })
    }

    /// Thin wrapper around `parse_optional_generic_params` that
    /// projects each binder down to its `name` field. Used by the
    /// nominal-decl parsers (record / variant / resource), which only
    /// need the binder *names* for `Subst.substIDL`-style lookups; the
    /// kind / value-type annotations land in `parse_optional_generic_params`'s
    /// fuller `Vec<GenericParamRaw>` for callers that want them.
    fn parse_generic_param_names(&mut self) -> Result<Vec<String>, ParseError> {
        let raw = self.parse_optional_generic_params()?;
        Ok(raw
            .into_iter()
            .map(|p| match p {
                GenericParamRaw::Type { name, .. } | GenericParamRaw::Value { name, .. } => name,
            })
            .collect())
    }

    fn parse_optional_generic_params(&mut self) -> Result<Vec<GenericParamRaw>, ParseError> {
        self.skip_ws();
        if self.peek_char() != Some('<') {
            return Ok(Vec::new());
        }
        self.pos += 1;
        let mut params: Vec<GenericParamRaw> = Vec::new();
        loop {
            self.skip_ws();
            if self.peek_char() == Some('>') {
                break;
            }
            params.push(self.parse_generic_param()?);
            self.skip_ws();
            if self.peek_char() == Some(',') {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.expect_char('>')?;
        Ok(params)
    }

    fn parse_generic_param(&mut self) -> Result<GenericParamRaw, ParseError> {
        let name = self.parse_ident()?;
        self.skip_ws();
        if self.peek_char() != Some(':') {
            // Bare `T` → defaults to kind `Type`.
            return Ok(GenericParamRaw::Type {
                name,
                kind: Some(RawKind::Type),
                constraint: None,
            });
        }
        self.pos += 1; // eat `:`
        self.skip_ws();
        // kind starts with `Type` or `(`. constraint_expr starts with anything else.
        if self.peek_keyword("Type") || self.peek_char() == Some('(') {
            // Could still be `(constraint_expr)`. Disambiguate by trying kind
            // first; if that yields a non-`->`-extended `(` group whose body
            // isn't kind-shaped, fall back. For v0 we keep it simple: `Type`
            // / parenthesised kind is always a kind.
            let kind = self.parse_kind()?;
            Ok(GenericParamRaw::Type {
                name,
                kind: Some(kind),
                constraint: None,
            })
        } else {
            // Either a value_param (`n : Nat`) or a type_param with constraint
            // expression. We can't fully disambiguate without a typing
            // context. Heuristic: if the annotation parses as a `type` and
            // that type's *kind* is `Type`, treat it as `value_param`;
            // anything else (e.g. starting with `scalar`/`ord`/`¬`) is a
            // `constraint_expr`. For v0 we record both possibilities as a
            // verbatim string and let downstream sort it out at admit-set
            // time.
            let annotation = self.parse_balanced_to_terminator(b',', b'>')?;
            // Best-effort classification: starts with a known constraint
            // keyword → constraint; starts with `¬` → constraint; otherwise
            // treat as a value-param `type` text (rare in monomorphised
            // schemas; the plugin emits no value-params).
            let trimmed = annotation.trim();
            let looks_constraint = matches!(
                trimmed.split_whitespace().next().unwrap_or(""),
                "scalar" | "ord" | "eq" | "hash" | "pod" | "marshal" | "resource"
            ) || trimmed.starts_with('¬');
            if looks_constraint {
                Ok(GenericParamRaw::Type {
                    name,
                    kind: None,
                    constraint: Some(trimmed.to_string()),
                })
            } else {
                // Re-parse the captured text as a type expression so the
                // value-param body is structurally available downstream.
                let mut sub = Parser::new(trimmed);
                let ty = sub.parse_type()?;
                Ok(GenericParamRaw::Value { name, ty })
            }
        }
    }

    fn parse_kind(&mut self) -> Result<RawKind, ParseError> {
        let head = if self.peek_keyword("Type") {
            self.expect_keyword("Type")?;
            RawKind::Type
        } else if self.peek_char() == Some('(') {
            self.pos += 1;
            let inner = self.parse_kind()?;
            self.expect_char(')')?;
            inner
        } else {
            return Err(ParseError::Expected {
                at: self.pos,
                what: "kind (`Type` or `(kind)`)".into(),
            });
        };
        self.skip_ws();
        if self.peek_str("->") {
            self.pos += 2;
            let tail = self.parse_kind()?;
            Ok(RawKind::Arrow(Box::new(head), Box::new(tail)))
        } else {
            Ok(head)
        }
    }

    /// Read raw bytes up to (but not consuming) any of `term1`/`term2`,
    /// respecting `<>`/`()`/`{}` nesting. Used for generic-param
    /// annotations whose internal shape we don't fully parse yet.
    fn parse_balanced_to_terminator(&mut self, term1: u8, term2: u8) -> Result<String, ParseError> {
        self.skip_ws();
        let start = self.pos;
        let mut depth_angle = 0i32;
        let mut depth_paren = 0i32;
        let mut depth_brace = 0i32;
        while self.pos < self.input.len() {
            let c = self.input[self.pos];
            if depth_angle == 0 && depth_paren == 0 && depth_brace == 0 && (c == term1 || c == term2) {
                break;
            }
            match c {
                b'<' => depth_angle += 1,
                b'>' => depth_angle -= 1,
                b'(' => depth_paren += 1,
                b')' => depth_paren -= 1,
                b'{' => depth_brace += 1,
                b'}' => depth_brace -= 1,
                _ => {}
            }
            self.pos += 1;
        }
        let s = std::str::from_utf8(&self.input[start..self.pos])
            .unwrap()
            .trim()
            .to_string();
        Ok(s)
    }

    fn peek_str(&self, s: &str) -> bool {
        let bytes = s.as_bytes();
        let mut p = self.pos;
        while p < self.input.len() && self.input[p].is_ascii_whitespace() {
            p += 1;
        }
        self.input.get(p..p + bytes.len()) == Some(bytes)
    }

    fn parse_func_decl(&mut self) -> Result<RawFunc, ParseError> {
        self.expect_keyword("func")?;
        let name = self.parse_ident()?;
        let _generics = self.parse_optional_generic_params()?;
        self.expect_char('(')?;
        let mut params: Vec<(String, RawType)> = Vec::new();
        self.skip_ws();
        if self.peek_char() != Some(')') {
            loop {
                let pname = self.parse_ident()?;
                self.expect_char(':')?;
                let pty = self.parse_type()?;
                params.push((pname, pty));
                self.skip_ws();
                if self.peek_char() == Some(',') {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }
        self.expect_char(')')?;
        // `func name(…) -> ret;` — `-> ret` is grammar-optional. Absent
        // return value = zero-length on the wire (SPEC/canonical-abi.md §14).
        // D-i 2026-05-19: `future<T>` / `stream<T>` at the return
        // position are *effect markers*, not type wrappers. Desugar
        // them here into `FuncDecl.effect` + inner-`T` ret.
        self.skip_ws();
        let (ret, effect) = if self.peek_str("->") {
            self.pos += 2;
            self.skip_ws();
            if self.peek_keyword("future") {
                self.expect_keyword("future")?;
                self.expect_char('<')?;
                let inner = self.parse_type()?;
                self.expect_char('>')?;
                (inner, Effect::Async)
            } else if self.peek_keyword("stream") {
                self.expect_keyword("stream")?;
                self.expect_char('<')?;
                let inner = self.parse_type()?;
                self.expect_char('>')?;
                (inner, Effect::Stream)
            } else {
                (self.parse_type()?, Effect::Sync)
            }
        } else {
            (RawType::Builtin(IDLType::Tuple(vec![])), Effect::Sync)
        };
        self.expect_char(';')?;
        Ok(RawFunc { name, params, ret, effect })
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Coerce a `RawType` to a pure `IDLType` when we know it must be a builtin
/// (e.g. inside `list<…>`). For a nominal reference, fall back to embedding
/// it as a `Record`-shaped placeholder — `resolve` patches it up later.
/// This *does* mean a `list<MyRecord>` survives parsing pre-resolution.
fn raw_to_builtin(rt: RawType) -> Result<IDLType, ParseError> {
    match rt {
        RawType::Builtin(b) => Ok(b),
        RawType::Named { fqn, args } => {
            let args = args
                .into_iter()
                .map(raw_to_builtin)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(IDLType::Record { fqn, args })
        }
        // `Cyc<i>` here is a parser-time guess (e.g. inside `list<…>`
        // before resolution). Carry it through verbatim; `resolve_type`
        // re-checks scope and bounds at the full schema level.
        RawType::Cyc(i) => Ok(IDLType::Cyc(i)),
    }
}

const PRIMITIVE_KEYWORDS: &[(&str, IDLType)] = &[
    ("u8", IDLType::U8),
    ("u16", IDLType::U16),
    ("u32", IDLType::U32),
    ("u64", IDLType::U64),
    ("i8", IDLType::I8),
    ("i16", IDLType::I16),
    ("i32", IDLType::I32),
    ("i64", IDLType::I64),
    ("f32", IDLType::F32),
    ("f64", IDLType::F64),
    ("bool", IDLType::Bool),
    ("char", IDLType::Char),
    ("string", IDLType::String),
    ("bigint", IDLType::BigInt),
    ("bignat", IDLType::BigNat),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_interface() {
        let s = parse("package p; interface i { }").unwrap();
        assert_eq!(s.package, "p");
        assert_eq!(s.interface, "i");
        assert!(s.user_decls.is_empty());
        assert!(s.funcs.is_empty());
    }

    #[test]
    fn single_func() {
        let s = parse(
            "package p; interface i { func add(_0: u64, _1: u64) -> u64; }",
        )
        .unwrap();
        assert_eq!(s.funcs.len(), 1);
        let f = &s.funcs[0];
        assert_eq!(f.name, "add");
        assert_eq!(f.params.len(), 2);
        assert_eq!(f.params[0], ("_0".into(), IDLType::U64));
        assert_eq!(f.ret, IDLType::U64);
    }

    #[test]
    fn record_with_field_decl() {
        let s = parse(
            "package p; interface i { record p.Point { x: f64, y: f64 }; func midpoint(_0: p.Point) -> p.Point; }",
        )
        .unwrap();
        assert_eq!(s.user_decls.len(), 1);
        match &s.user_decls[0] {
            UserDecl::Record { fqn, fields, .. } => {
                assert_eq!(fqn, "p.Point");
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0], ("x".into(), IDLType::F64));
            }
            _ => panic!("expected Record"),
        }
        assert_eq!(s.funcs.len(), 1);
        let f = &s.funcs[0];
        assert_eq!(
            f.ret,
            IDLType::Record {
                fqn: "p.Point".into(),
                args: vec![]
            }
        );
    }

    #[test]
    fn variant_self_recursive() {
        let s = parse(
            "package p; interface i { variant p.Tree { leaf, node(Self, Self) }; func mk() -> p.Tree; }",
        )
        .unwrap();
        match &s.user_decls[0] {
            UserDecl::Variant { fqn, cases, .. } => {
                assert_eq!(fqn, "p.Tree");
                assert_eq!(cases.len(), 2);
                assert_eq!(cases[1].0, "node");
                assert_eq!(cases[1].1, vec![IDLType::Self_, IDLType::Self_]);
            }
            _ => panic!("expected Variant"),
        }
    }

    #[test]
    fn enum_decl() {
        let s = parse(
            "package p; interface i { enum p.Color { red, green, blue }; func name(_0: p.Color) -> string; }",
        )
        .unwrap();
        match &s.user_decls[0] {
            UserDecl::Enum { fqn, cases } => {
                assert_eq!(fqn, "p.Color");
                assert_eq!(cases, &vec!["red", "green", "blue"]);
            }
            _ => panic!("expected Enum"),
        }
        assert_eq!(s.funcs[0].params[0].1, IDLType::Enum("p.Color".into()));
    }

    #[test]
    fn resource_decl() {
        let s = parse(
            "package p; interface i { resource p.Handle; func id(_0: p.Handle) -> p.Handle; }",
        )
        .unwrap();
        assert!(matches!(s.user_decls[0], UserDecl::Resource { .. }));
        let ret = &s.funcs[0].ret;
        assert!(matches!(ret, IDLType::Resource { .. }));
    }

    #[test]
    fn list_option_result() {
        let s = parse(
            "package p; interface i { func f(_0: list<u32>, _1: option<string>) -> result<u64, string>; }",
        )
        .unwrap();
        let f = &s.funcs[0];
        assert_eq!(f.params[0].1, IDLType::List(Box::new(IDLType::U32)));
        assert_eq!(f.params[1].1, IDLType::Option(Box::new(IDLType::String)));
        assert_eq!(
            f.ret,
            IDLType::Result(
                Box::new(IDLType::U64),
                Some(Box::new(IDLType::String))
            )
        );
    }

    #[test]
    fn package_kebab() {
        let s = parse("package leo4-sample; interface Sample { }").unwrap();
        assert_eq!(s.package, "leo4-sample");
    }

    #[test]
    fn unknown_nominal_errors() {
        let err = parse("package p; interface i { func f(_0: p.Missing) -> u32; }").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("unknown nominal type `p.Missing`"), "got: {msg}");
    }

    #[test]
    fn package_with_subnamespace_and_version() {
        let s = parse("package my:analytics@1.2.3; interface i { }").unwrap();
        assert_eq!(s.package, "my:analytics");
    }

    #[test]
    fn use_decl_at_top_level() {
        let raw = parse_raw("package p; use a:b/c as x; interface i { }").unwrap();
        assert_eq!(raw.use_decls.len(), 1);
        assert_eq!(raw.use_decls[0].path, "a:b/c");
        assert_eq!(raw.use_decls[0].alias.as_deref(), Some("x"));
    }

    #[test]
    fn world_decl_skipped_into_items() {
        let raw = parse_raw(
            "package p; world w { import iface-a; export iface-b; use a:b; }",
        )
        .unwrap();
        assert_eq!(raw.worlds.len(), 1);
        assert_eq!(raw.worlds[0].name, "w");
        assert_eq!(raw.worlds[0].items.len(), 3);
    }

    #[test]
    fn top_level_nominal_decl() {
        // record/enum at document scope, no interface block.
        let raw = parse_raw(
            "package p; record p.Pair { a: u32, b: u32 }; enum p.Side { left, right };",
        )
        .unwrap();
        assert_eq!(raw.decls.len(), 2);
    }

    #[test]
    fn top_level_constraint_decl_captured_verbatim() {
        let raw = parse_raw(
            "package p; constraint marshalScalar = scalar ∧ marshal; interface i { }",
        )
        .unwrap();
        assert_eq!(raw.constraint_decls.len(), 1);
        assert_eq!(raw.constraint_decls[0].name, "marshalScalar");
        assert!(raw.constraint_decls[0].body.contains("scalar"));
    }

    #[test]
    fn func_optional_return_type() {
        let s = parse("package p; interface i { func ping(); }").unwrap();
        // Absent `->` is equivalent to a zero-length tuple in the AST.
        assert_eq!(s.funcs[0].ret, IDLType::Tuple(vec![]));
    }

    #[test]
    fn interface_body_accepts_use() {
        let raw = parse_raw("package p; interface i { use a:b; func f() -> u32; }").unwrap();
        assert_eq!(raw.interfaces[0].use_decls.len(), 1);
        assert_eq!(raw.interfaces[0].funcs.len(), 1);
    }

    #[test]
    fn type_param_with_kind_annotation() {
        // `F : Type -> Type` HK binder, on a func decl.
        let raw = parse_raw(
            "package p; interface i { func map<F : Type -> Type, A, B>(_0: u32) -> u32; }",
        )
        .unwrap();
        let f = &raw.interfaces[0].funcs[0];
        assert_eq!(f.name, "map");
        // generics are captured in the interface's func generics array via
        // parse_optional_generic_params; surface via funcs[0]... unused
        // downstream for v0, but the syntax must parse cleanly.
    }

    #[test]
    fn type_param_with_constraint_annotation() {
        let raw = parse_raw(
            "package p; interface i { func bucketize<T : scalar>(_0: u32) -> u32; }",
        )
        .unwrap();
        assert_eq!(raw.interfaces[0].funcs[0].name, "bucketize");
    }

    #[test]
    fn value_param_in_generic_params() {
        // `{n : Nat}` value-param binder. The annotation parses as a
        // type expression (`Nat` → IDL reference to user type or a
        // primitive, depending on resolution context). For our parser
        // it lives only as Raw form — resolution doesn't depend on it.
        let raw = parse_raw(
            "package p; interface i { func vlen<n : u32>(_0: u32) -> u32; }",
        )
        .unwrap();
        assert_eq!(raw.interfaces[0].funcs[0].name, "vlen");
    }

    #[test]
    fn generic_record_with_kinded_param() {
        let raw = parse_raw(
            "package p; interface i { record p.Pair<a : Type, b : Type> { fst: a, snd: b }; }",
        )
        .unwrap();
        assert_eq!(raw.decls.len(), 1);
        if let RawDecl::Record { fqn, fields, .. } = &raw.decls[0] {
            assert_eq!(fqn, "p.Pair");
            assert_eq!(fields[0].0, "fst");
        } else {
            panic!("expected Record");
        }
    }

    #[test]
    fn self_app_args_preserved() {
        let s = parse(
            "package p; interface i { variant p.Tree<a> { leaf, node(Self<a>, Self<a>) }; func f() -> p.Tree<u32>; }",
        )
        .unwrap();
        match &s.user_decls[0] {
            UserDecl::Variant { cases, .. } => {
                let node = &cases[1];
                assert_eq!(node.0, "node");
                assert_eq!(
                    node.1,
                    vec![
                        IDLType::SelfApp(vec![
                            IDLType::Record { fqn: "a".into(), args: vec![] }
                        ]),
                        IDLType::SelfApp(vec![
                            IDLType::Record { fqn: "a".into(), args: vec![] }
                        ]),
                    ]
                );
            }
            _ => panic!("expected Variant"),
        }
    }

    #[test]
    fn resource_with_methods_body() {
        let s = parse(
            "package p; interface i { resource p.Handle { /* method body ignored in v0 */ }; func id(_0: p.Handle) -> p.Handle; }",
        )
        .unwrap();
        assert!(matches!(s.user_decls[0], UserDecl::Resource { .. }));
    }

    // ─── Phase 6: mutual_decl + Cyc<i> ─────────────────────────────

    #[test]
    fn mutual_group_basic() {
        let s = parse(
            "package p; interface i { mutual { \
               variant p.Expr { lit(u64), neg(Cyc<0>), seq(Cyc<1>) }; \
               variant p.Stmt { nop, block(list<Cyc<1>>), call(Cyc<0>) }; \
             }; func dummy() -> u8; }",
        )
        .unwrap();
        let group = match &s.user_decls[0] {
            UserDecl::Mutual { members } => members,
            other => panic!("expected Mutual, got {other:?}"),
        };
        assert_eq!(group.len(), 2);
        match &group[0] {
            UserDecl::Variant { fqn, cases, .. } => {
                assert_eq!(fqn, "p.Expr");
                // neg's payload is Cyc<0>
                let neg_payload = &cases.iter().find(|(n, _)| n == "neg").unwrap().1;
                assert_eq!(neg_payload, &vec![IDLType::Cyc(0)]);
            }
            _ => panic!("expected Variant for member[0]"),
        }
    }

    #[test]
    fn mutual_group_singleton_rejected() {
        let err = parse(
            "package p; interface i { mutual { variant p.X { a, b(Cyc<0>) }; }; }",
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("at least 2"), "{msg}");
    }

    #[test]
    fn cyc_outside_mutual_rejected() {
        let err = parse(
            "package p; interface i { variant p.X { a, b(Cyc<0>) }; }",
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("outside any `mutual"), "{msg}");
    }

    // ─── Phase 7 step 1: future / stream effect desugar ────────────

    #[test]
    fn func_future_return_desugars_to_async_effect() {
        let s = parse(
            "package p; interface i { func tick(_0: u32) -> future<u64>; }",
        )
        .unwrap();
        assert_eq!(s.funcs.len(), 1);
        let f = &s.funcs[0];
        assert_eq!(f.effect, Effect::Async);
        assert_eq!(f.ret, IDLType::U64);
    }

    #[test]
    fn func_stream_return_desugars_to_stream_effect() {
        let s = parse(
            "package p; interface i { func ticks(_0: u32) -> stream<u8>; }",
        )
        .unwrap();
        let f = &s.funcs[0];
        assert_eq!(f.effect, Effect::Stream);
        assert_eq!(f.ret, IDLType::U8);
    }

    #[test]
    fn future_in_payload_position_rejected() {
        let err = parse(
            "package p; interface i { func f(_0: list<future<u32>>) -> u32; }",
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("future<T>"), "{msg}");
    }

    #[test]
    fn stream_in_record_field_rejected() {
        let err = parse(
            "package p; interface i { record p.X { y: stream<u32> }; }",
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("stream<T>"), "{msg}");
    }

    #[test]
    fn cyc_index_out_of_range_rejected() {
        let err = parse(
            "package p; interface i { mutual { \
               variant p.A { a, b(Cyc<5>) }; \
               variant p.B { x }; \
             }; }",
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("Cyc<5>"), "{msg}");
    }

    #[test]
    fn fn_arrow_in_param() {
        // Phase 10-B1: function-arrow as a function parameter.
        let s = parse(
            "package p; interface i { func solve(_0: u32, _1: fn(u32) -> bool) -> bool; }",
        )
        .unwrap();
        assert_eq!(s.funcs.len(), 1);
        let f = &s.funcs[0];
        assert_eq!(f.params.len(), 2);
        assert_eq!(
            f.params[1].1,
            IDLType::Fn {
                args: vec![IDLType::U32],
                ret: Box::new(IDLType::Bool)
            }
        );
    }

    #[test]
    fn fn_arrow_nullary_parses() {
        let s = parse(
            "package p; interface i { func run(_0: fn() -> string) -> string; }",
        )
        .unwrap();
        assert_eq!(
            s.funcs[0].params[0].1,
            IDLType::Fn {
                args: vec![],
                ret: Box::new(IDLType::String)
            }
        );
    }
}
