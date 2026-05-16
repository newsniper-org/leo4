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

use crate::idl::{FuncDecl, IDLType, Schema, UserDecl};

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
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolveError::UnknownNominal { fqn } => {
                write!(f, "unknown nominal type `{fqn}` (no record/enum/variant/resource decl)")
            }
            ResolveError::EnumWithArgs { fqn } => {
                write!(f, "enum `{fqn}` cannot take generic arguments")
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawDecl {
    Record { fqn: String, fields: Vec<(String, RawType)> },
    Enum { fqn: String, cases: Vec<String> },
    Variant { fqn: String, cases: Vec<(String, Vec<RawType>)> },
    Resource { fqn: String },
}

impl RawDecl {
    fn fqn(&self) -> &str {
        match self {
            RawDecl::Record { fqn, .. }
            | RawDecl::Enum { fqn, .. }
            | RawDecl::Variant { fqn, .. }
            | RawDecl::Resource { fqn, .. } => fqn,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawFunc {
    pub name: String,
    pub params: Vec<(String, RawType)>,
    pub ret: RawType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawSchema {
    pub package: String,
    pub interface: String,
    pub decls: Vec<RawDecl>,
    pub funcs: Vec<RawFunc>,
}

/// Top-level entry: parse and resolve in one shot.
pub fn parse(input: &str) -> Result<Schema, Box<dyn std::error::Error>> {
    let raw = parse_raw(input)?;
    let schema = resolve(&raw)?;
    Ok(schema)
}

/// Parse to `RawSchema` without resolving named types.
pub fn parse_raw(input: &str) -> Result<RawSchema, ParseError> {
    let mut p = Parser::new(input);
    p.skip_ws();
    p.expect_keyword("package")?;
    let package = p.parse_ident_or_kebab()?;
    p.expect_char(';')?;
    p.expect_keyword("interface")?;
    let interface = p.parse_ident()?;
    p.expect_char('{')?;

    let mut decls: Vec<RawDecl> = Vec::new();
    let mut funcs: Vec<RawFunc> = Vec::new();

    loop {
        p.skip_ws();
        if p.peek_char() == Some('}') {
            break;
        }
        if p.peek_keyword("enum") {
            decls.push(p.parse_enum_decl()?);
        } else if p.peek_keyword("record") {
            decls.push(p.parse_record_decl()?);
        } else if p.peek_keyword("variant") {
            decls.push(p.parse_variant_decl()?);
        } else if p.peek_keyword("resource") {
            decls.push(p.parse_resource_decl()?);
        } else if p.peek_keyword("func") {
            funcs.push(p.parse_func_decl()?);
        } else {
            return Err(ParseError::Expected {
                at: p.pos,
                what: "`enum`/`record`/`variant`/`resource`/`func` decl or `}`".into(),
            });
        }
        p.skip_ws();
        p.expect_char(';')?;
    }

    p.expect_char('}')?;
    p.skip_ws();
    if !p.at_end() {
        return Err(ParseError::TrailingInput { at: p.pos });
    }

    Ok(RawSchema { package, interface, decls, funcs })
}

/// Resolve all `Named { fqn, args }` references against the user
/// declarations the schema itself provides.
pub fn resolve(raw: &RawSchema) -> Result<Schema, ResolveError> {
    let shapes = build_shape_map(&raw.decls);
    let user_decls = raw
        .decls
        .iter()
        .map(|d| resolve_decl(d, &shapes))
        .collect::<Result<Vec<_>, _>>()?;
    let funcs = raw
        .funcs
        .iter()
        .map(|f| {
            let params = f
                .params
                .iter()
                .map(|(n, t)| Ok::<_, ResolveError>((n.clone(), resolve_type(t, &shapes)?)))
                .collect::<Result<Vec<_>, _>>()?;
            let ret = resolve_type(&f.ret, &shapes)?;
            Ok::<_, ResolveError>(FuncDecl {
                name: f.name.clone(),
                params,
                ret,
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
}

fn build_shape_map(decls: &[RawDecl]) -> HashMap<String, Shape> {
    let mut map = HashMap::with_capacity(decls.len());
    for d in decls {
        let s = match d {
            RawDecl::Record { .. } => Shape::Record,
            RawDecl::Enum { .. } => Shape::Enum,
            RawDecl::Variant { .. } => Shape::Variant,
            RawDecl::Resource { .. } => Shape::Resource,
        };
        map.insert(d.fqn().to_string(), s);
    }
    map
}

fn resolve_type(t: &RawType, shapes: &HashMap<String, Shape>) -> Result<IDLType, ResolveError> {
    match t {
        RawType::Builtin(b) => Ok(b.clone()),
        RawType::Named { fqn, args } => {
            let args_idl = args
                .iter()
                .map(|a| resolve_type(a, shapes))
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
                None => Err(ResolveError::UnknownNominal { fqn: fqn.clone() }),
            }
        }
    }
}

fn resolve_decl(d: &RawDecl, shapes: &HashMap<String, Shape>) -> Result<UserDecl, ResolveError> {
    match d {
        RawDecl::Record { fqn, fields } => {
            let fields = fields
                .iter()
                .map(|(n, t)| Ok::<_, ResolveError>((n.clone(), resolve_type(t, shapes)?)))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(UserDecl::Record {
                fqn: fqn.clone(),
                generics: vec![],
                fields,
            })
        }
        RawDecl::Enum { fqn, cases } => Ok(UserDecl::Enum {
            fqn: fqn.clone(),
            cases: cases.clone(),
        }),
        RawDecl::Variant { fqn, cases } => {
            let cases = cases
                .iter()
                .map(|(n, payload)| {
                    let payload = payload
                        .iter()
                        .map(|t| resolve_type(t, shapes))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok::<_, ResolveError>((n.clone(), payload))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(UserDecl::Variant {
                fqn: fqn.clone(),
                generics: vec![],
                cases,
            })
        }
        RawDecl::Resource { fqn } => Ok(UserDecl::Resource {
            fqn: fqn.clone(),
            generics: vec![],
        }),
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

    fn expect_str(&mut self, s: &str) -> Result<(), ParseError> {
        self.skip_ws();
        let bytes = s.as_bytes();
        if self.input.get(self.pos..self.pos + bytes.len()) == Some(bytes) {
            self.pos += bytes.len();
            Ok(())
        } else {
            Err(ParseError::Expected {
                at: self.pos,
                what: format!("`{s}`"),
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
        if self.peek_keyword("Self") {
            self.expect_keyword("Self")?;
            // optional <args> — SPEC syntax allows Self<…>; for v0 we
            // mangle to bare `self`, so swallow any args without
            // distinguishing.
            if self.peek_char() == Some('<') {
                self.expect_char('<')?;
                loop {
                    let _ = self.parse_type()?;
                    self.skip_ws();
                    if self.peek_char() == Some(',') {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                self.expect_char('>')?;
            }
            return Ok(RawType::Builtin(IDLType::Self_));
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
        Ok(RawDecl::Enum { fqn, cases })
    }

    fn parse_record_decl(&mut self) -> Result<RawDecl, ParseError> {
        self.expect_keyword("record")?;
        let fqn = self.parse_fqn()?;
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
        Ok(RawDecl::Record { fqn, fields })
    }

    fn parse_variant_decl(&mut self) -> Result<RawDecl, ParseError> {
        self.expect_keyword("variant")?;
        let fqn = self.parse_fqn()?;
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
        Ok(RawDecl::Variant { fqn, cases })
    }

    fn parse_resource_decl(&mut self) -> Result<RawDecl, ParseError> {
        self.expect_keyword("resource")?;
        let fqn = self.parse_fqn()?;
        Ok(RawDecl::Resource { fqn })
    }

    fn parse_func_decl(&mut self) -> Result<RawFunc, ParseError> {
        self.expect_keyword("func")?;
        let name = self.parse_ident()?;
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
        self.expect_str("->")?;
        let ret = self.parse_type()?;
        Ok(RawFunc { name, params, ret })
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
}
