//! Symbol mangling.
//!
//! Mirrors `lake/Leo4Plugin/Leo4Plugin/Mangling.lean` (`fqnSeg`,
//! `mangleType`, `mangle`). Both implementations MUST produce
//! byte-identical output for the same IDL input — pinned by
//! `tests/mangling/`.

use crate::hash::Hash;
use crate::idl::IDLType;

/// FQN → mangling-safe segment: dotted Lean name joined by `_`.
/// SPEC/mangling.md §2 "Fully-qualified names".
#[must_use]
pub fn fqn_seg(fqn: &str) -> String {
    fqn.replace('.', "_")
}

/// Type encoding per SPEC/mangling.md §2.
#[must_use]
pub fn mangle_type(t: &IDLType) -> String {
    use IDLType::*;
    match t {
        U8 => "u8".into(),
        U16 => "u16".into(),
        U32 => "u32".into(),
        U64 => "u64".into(),
        I8 => "i8".into(),
        I16 => "i16".into(),
        I32 => "i32".into(),
        I64 => "i64".into(),
        F32 => "f32".into(),
        F64 => "f64".into(),
        Bool => "b".into(),
        Char => "c".into(),
        String => "str".into(),
        BigInt => "bI".into(),
        BigNat => "bN".into(),
        List(t) => format!("L_{}_l", mangle_type(t)),
        Option(t) => format!("O_{}_o", mangle_type(t)),
        Result(t, None) => format!("Rz_{}__z", mangle_type(t)),
        Result(t, Some(e)) => format!("Rz_{}_{}_z", mangle_type(t), mangle_type(e)),
        Tuple(ts) => format!("T_{}_t", join_underscore(ts)),
        Record { fqn, args } if args.is_empty() => format!("S_{}_s", fqn_seg(fqn)),
        Record { fqn, args } => {
            format!("S_{}_{}_s", fqn_seg(fqn), join_underscore(args))
        }
        Variant { fqn, args } if args.is_empty() => format!("V_{}_v", fqn_seg(fqn)),
        Variant { fqn, args } => {
            format!("V_{}_{}_v", fqn_seg(fqn), join_underscore(args))
        }
        Enum(fqn) => format!("E_{}_e", fqn_seg(fqn)),
        Flags(fqn) => format!("F_{}_f", fqn_seg(fqn)),
        Resource { fqn, args } if args.is_empty() => format!("X_{}_x", fqn_seg(fqn)),
        Resource { fqn, args } => {
            format!("X_{}_{}_x", fqn_seg(fqn), join_underscore(args))
        }
        Io(t) => format!("I_{}_i", mangle_type(t)),
        Self_ => "self".into(),
        SelfApp(args) => format!("self_{}_x", join_underscore(args)),
    }
}

fn join_underscore(ts: &[IDLType]) -> String {
    ts.iter().map(mangle_type).collect::<Vec<_>>().join("_")
}

/// Full mangled name. SPEC/mangling.md §1.
///
/// `args` is the function's parameter-type list after generic substitution,
/// in declaration order. For non-generic functions it is simply the
/// parameter types verbatim.
#[must_use]
pub fn mangle(pkg: &str, iface: &str, fname: &str, args: &[IDLType], schema_hash: Hash) -> String {
    let pkg_seg = pkg.replace(':', "_");
    let args_seg = args.iter().map(mangle_type).collect::<Vec<_>>().join("_");
    format!(
        "leo4__{pkg_seg}__{iface}__{fname}__{args_seg}__h{}",
        schema_hash.to_base32lc()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t<T: Into<Box<IDLType>>>(x: T) -> Box<IDLType> {
        x.into()
    }

    #[test]
    fn primitives() {
        assert_eq!(mangle_type(&IDLType::U8), "u8");
        assert_eq!(mangle_type(&IDLType::F64), "f64");
        assert_eq!(mangle_type(&IDLType::Bool), "b");
        assert_eq!(mangle_type(&IDLType::Char), "c");
        assert_eq!(mangle_type(&IDLType::String), "str");
        assert_eq!(mangle_type(&IDLType::BigInt), "bI");
        assert_eq!(mangle_type(&IDLType::BigNat), "bN");
    }

    #[test]
    fn composites() {
        let lst_u32 = IDLType::List(t(IDLType::U32));
        assert_eq!(mangle_type(&lst_u32), "L_u32_l");

        let opt_str = IDLType::Option(t(IDLType::String));
        assert_eq!(mangle_type(&opt_str), "O_str_o");

        let res_ok_only = IDLType::Result(t(IDLType::U64), None);
        assert_eq!(mangle_type(&res_ok_only), "Rz_u64__z");

        let res_ok_err = IDLType::Result(t(IDLType::U64), Some(t(IDLType::String)));
        assert_eq!(mangle_type(&res_ok_err), "Rz_u64_str_z");

        let tup = IDLType::Tuple(vec![IDLType::F32, IDLType::F32]);
        assert_eq!(mangle_type(&tup), "T_f32_f32_t");
    }

    #[test]
    fn nominal_with_fqn() {
        let point = IDLType::Record {
            fqn: "Sample.Point".into(),
            args: vec![],
        };
        assert_eq!(mangle_type(&point), "S_Sample_Point_s");

        let tree = IDLType::Variant {
            fqn: "Sample.Tree".into(),
            args: vec![],
        };
        assert_eq!(mangle_type(&tree), "V_Sample_Tree_v");

        let color = IDLType::Enum("Sample.Color".into());
        assert_eq!(mangle_type(&color), "E_Sample_Color_e");

        let handle = IDLType::Resource {
            fqn: "Sample.ParserHandle".into(),
            args: vec![],
        };
        assert_eq!(mangle_type(&handle), "X_Sample_ParserHandle_x");

        let self_ = IDLType::Self_;
        assert_eq!(mangle_type(&self_), "self");

        // SPEC §"Self and Self<…>": Self<T1,…,Tn> mangles as
        // `self_<args>_x`; the head stays the constant `self` token
        // (no recursive expansion).
        let self_app =
            IDLType::SelfApp(vec![IDLType::U32, IDLType::String]);
        assert_eq!(mangle_type(&self_app), "self_u32_str_x");
    }

    #[test]
    fn generic_record() {
        let pair_u32_str = IDLType::Record {
            fqn: "My.Kv.Pair".into(),
            args: vec![IDLType::U32, IDLType::String],
        };
        assert_eq!(mangle_type(&pair_u32_str), "S_My_Kv_Pair_u32_str_s");
    }

    #[test]
    fn full_mangle_matches_spec_example2() {
        // SPEC/mangling.md §4 Example 2: `func sum(xs: list<u64>) -> u64;`
        // package my:data, interface util.
        let h = Hash { value: 0 }; // not the spec's hash; only checking shape
        let m = mangle(
            "my:data",
            "util",
            "sum",
            &[IDLType::List(t(IDLType::U64))],
            h,
        );
        assert!(m.starts_with("leo4__my_data__util__sum__L_u64_l__h"));
    }

    #[test]
    fn full_mangle_zero_args() {
        // `func hello() -> string;` — empty arg list ⇒ consecutive `__`.
        let h = Hash { value: 0 };
        let m = mangle("p", "i", "hello", &[], h);
        assert!(m.starts_with("leo4__p__i__hello____h"));
    }
}
