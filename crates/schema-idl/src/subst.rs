//! Type-parameter substitution.
//!
//! Walks an `IDLType` tree and replaces references to a binder name
//! with the substituted concrete type. The grammar (`SPEC/idl-grammar.ebnf`)
//! represents a bare type-variable reference as `IDLType::Record { fqn, args }`
//! where `fqn` happens to match a binder string and `args` is empty — there
//! is no dedicated `IDLType::TypeVar` variant. `substitute` performs that
//! disambiguation against a caller-supplied environment.
//!
//! This is the missing piece called out in `schema-idl-shortcomings.md` #4:
//! the leo4 plugin's nominal shim handler currently bails when
//! `generics.is_empty()` is false because it has no generic way to walk a
//! `UserDecl` and produce its instantiated field / case types. With these
//! helpers, callers can resolve a generic record / variant against concrete
//! type arguments before handing the result to their wire-format emitter.

use crate::idl::{IDLType, UserDecl};

/// Substitute every occurrence of a binder name in `env` with its
/// associated `IDLType`. The substitution is shallow — once a binder is
/// replaced, the substituent is **not** walked further (substituents are
/// already concrete in the call sites we care about).
///
/// Recognition rule: an `IDLType::Record { fqn, args }` with `args.is_empty()`
/// **and** `fqn` matching one of the binder names in `env` is treated as a
/// type-variable and replaced. Any other `Record` / `Variant` / `Resource`
/// reference is left alone (its arguments are recursively substituted).
///
/// Primitives, `Enum`, `Flags`, `Self_`, and `SelfApp` pass through
/// verbatim — `Enum` and `Flags` can't be type-parametrised, and `Self`
/// reference resolves at a different layer (variant helper emission).
#[must_use]
pub fn substitute(ty: &IDLType, env: &[(String, IDLType)]) -> IDLType {
    use IDLType::*;
    let look = |name: &str| env.iter().find(|(n, _)| n == name).map(|(_, t)| t.clone());
    match ty {
        Record { fqn, args } if args.is_empty() => {
            look(fqn).unwrap_or_else(|| Record { fqn: fqn.clone(), args: vec![] })
        }
        Record { fqn, args } => Record {
            fqn: fqn.clone(),
            args: args.iter().map(|a| substitute(a, env)).collect(),
        },
        Variant { fqn, args } => Variant {
            fqn: fqn.clone(),
            args: args.iter().map(|a| substitute(a, env)).collect(),
        },
        Resource { fqn, args } => Resource {
            fqn: fqn.clone(),
            args: args.iter().map(|a| substitute(a, env)).collect(),
        },
        List(t) => List(Box::new(substitute(t, env))),
        Option(t) => Option(Box::new(substitute(t, env))),
        Result(t, e) => Result(
            Box::new(substitute(t, env)),
            e.as_ref().map(|x| Box::new(substitute(x, env))),
        ),
        Tuple(ts) => Tuple(ts.iter().map(|x| substitute(x, env)).collect()),
        Io(t) => Io(Box::new(substitute(t, env))),
        SelfApp(ts) => SelfApp(ts.iter().map(|x| substitute(x, env)).collect()),
        Fn { args, ret } => Fn {
            args: args.iter().map(|x| substitute(x, env)).collect(),
            ret: Box::new(substitute(ret, env)),
        },
        // Primitives + Enum + Flags + bare Self: identity.
        t => t.clone(),
    }
}

/// Zip a declaration's `generics` binders with a concrete `args` list. Returns
/// `None` if the arities don't match (a caller-side bug; the kind discipline
/// in the plugin should have rejected the IDL earlier).
fn make_env(generics: &[String], args: &[IDLType]) -> Option<Vec<(String, IDLType)>> {
    if generics.len() != args.len() {
        return None;
    }
    Some(
        generics
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect(),
    )
}

/// Instantiate a `UserDecl::Record` at concrete type arguments. Returns the
/// post-substitution field list (name → instantiated type) on success,
/// `None` if `decl` is not a record or if the arities don't match.
#[must_use]
pub fn instantiate_record(decl: &UserDecl, args: &[IDLType]) -> Option<Vec<(String, IDLType)>> {
    let UserDecl::Record { generics, fields, .. } = decl else {
        return None;
    };
    let env = make_env(generics, args)?;
    Some(
        fields
            .iter()
            .map(|(n, t)| (n.clone(), substitute(t, &env)))
            .collect(),
    )
}

/// Instantiate a `UserDecl::Variant` at concrete type arguments. Returns
/// the post-substitution case list (name → instantiated payload types).
#[must_use]
pub fn instantiate_variant(
    decl: &UserDecl,
    args: &[IDLType],
) -> Option<Vec<(String, Vec<IDLType>)>> {
    let UserDecl::Variant { generics, cases, .. } = decl else {
        return None;
    };
    let env = make_env(generics, args)?;
    Some(
        cases
            .iter()
            .map(|(n, payload)| {
                (
                    n.clone(),
                    payload.iter().map(|t| substitute(t, &env)).collect(),
                )
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::idl::{IDLType::*, UserDecl};

    fn rec(fqn: &str) -> IDLType {
        Record { fqn: fqn.into(), args: vec![] }
    }

    #[test]
    fn binder_at_leaf_position_replaced() {
        let env = vec![("T".to_string(), U32)];
        assert_eq!(substitute(&rec("T"), &env), U32);
    }

    #[test]
    fn unmatched_fqn_unchanged() {
        let env = vec![("T".to_string(), U32)];
        assert_eq!(substitute(&rec("U"), &env), rec("U"));
    }

    #[test]
    fn primitives_pass_through() {
        let env = vec![("T".to_string(), U32)];
        for p in [U8, U16, U32, U64, I8, I16, I32, I64, F32, F64, Bool, Char, String, BigInt, BigNat] {
            assert_eq!(substitute(&p, &env), p);
        }
    }

    #[test]
    fn list_of_binder() {
        let env = vec![("T".to_string(), String)];
        let input = List(Box::new(rec("T")));
        let expected = List(Box::new(String));
        assert_eq!(substitute(&input, &env), expected);
    }

    #[test]
    fn nested_composite_walked() {
        // option<result<T, string>>  with T = u64
        let env = vec![("T".to_string(), U64)];
        let input = Option(Box::new(Result(
            Box::new(rec("T")),
            Some(Box::new(String)),
        )));
        let expected = Option(Box::new(Result(Box::new(U64), Some(Box::new(String)))));
        assert_eq!(substitute(&input, &env), expected);
    }

    #[test]
    fn record_with_args_walks_args_only() {
        // `Pair<T, u32>` should become `Pair<u64, u32>` for T = u64; the
        // head `Pair` itself is not a type-variable because args is
        // non-empty.
        let env = vec![("T".to_string(), U64)];
        let input = Record { fqn: "Pair".into(), args: vec![rec("T"), U32] };
        let expected = Record { fqn: "Pair".into(), args: vec![U64, U32] };
        assert_eq!(substitute(&input, &env), expected);
    }

    #[test]
    fn self_pass_through() {
        let env = vec![("T".to_string(), U32)];
        assert_eq!(substitute(&Self_, &env), Self_);
        assert_eq!(
            substitute(&SelfApp(vec![rec("T")]), &env),
            SelfApp(vec![U32])
        );
    }

    #[test]
    fn instantiate_record_fields() {
        // record Pair<a, b> { fst: a, snd: b }
        let decl = UserDecl::Record {
            fqn: "Pair".into(),
            generics: vec!["a".into(), "b".into()],
            fields: vec![("fst".into(), rec("a")), ("snd".into(), rec("b"))],
        };
        let inst = instantiate_record(&decl, &[U32, String]).unwrap();
        assert_eq!(inst, vec![("fst".into(), U32), ("snd".into(), String)]);
    }

    #[test]
    fn instantiate_record_arity_mismatch() {
        let decl = UserDecl::Record {
            fqn: "Pair".into(),
            generics: vec!["a".into(), "b".into()],
            fields: vec![("fst".into(), rec("a"))],
        };
        assert!(instantiate_record(&decl, &[U32]).is_none());
    }

    #[test]
    fn instantiate_variant_cases() {
        // variant Either<a, b> { left(a), right(b) }
        let decl = UserDecl::Variant {
            fqn: "Either".into(),
            generics: vec!["a".into(), "b".into()],
            cases: vec![
                ("left".into(), vec![rec("a")]),
                ("right".into(), vec![rec("b")]),
            ],
        };
        let inst = instantiate_variant(&decl, &[U64, Bool]).unwrap();
        assert_eq!(
            inst,
            vec![
                ("left".into(), vec![U64]),
                ("right".into(), vec![Bool]),
            ]
        );
    }

    #[test]
    fn instantiate_record_on_variant_returns_none() {
        let decl = UserDecl::Variant {
            fqn: "Either".into(),
            generics: vec!["a".into()],
            cases: vec![("only".into(), vec![rec("a")])],
        };
        assert!(instantiate_record(&decl, &[U32]).is_none());
    }
}
