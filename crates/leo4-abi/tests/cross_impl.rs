//! Cross-impl encoder conformance.
//!
//! Reads a fixture file (default `/tmp/leo4-conformance.txt`, generated
//! by `tests/conformance/run.sh`) where each line has the form
//!
//!   <kind>/<value-name>=<hex bytes>
//!
//! and asserts that re-encoding the same logical value on the Rust side
//! produces identical bytes. Fixture lines marked with a leading `#`
//! are comments.

use std::path::PathBuf;

use leo4_abi::{bigint::BigInt, bignat::BigNat, marshal::encode_to_vec};

fn fixture_path() -> PathBuf {
    let p = std::env::var("LEO4_CONFORMANCE_FILE")
        .unwrap_or_else(|_| "/tmp/leo4-conformance.txt".into());
    PathBuf::from(p)
}

fn hex_of(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn rust_bytes(name: &str) -> Result<Vec<u8>, String> {
    let (kind, val) = name
        .split_once('/')
        .ok_or_else(|| format!("malformed name {name}"))?;
    Ok(match kind {
        "u8" => encode_to_vec(&parse_uint::<u8>(val)?),
        "u16" => encode_to_vec(&parse_uint::<u16>(val)?),
        "u32" => encode_to_vec(&parse_uint::<u32>(val)?),
        "u64" => encode_to_vec(&parse_uint::<u64>(val)?),
        "i8" => encode_to_vec(&parse_int_i8(val)?),
        "i16" => encode_to_vec(&parse_int_i16(val)?),
        "i32" => encode_to_vec(&parse_int_i32(val)?),
        "i64" => encode_to_vec(&parse_int_i64(val)?),
        "f32" => encode_to_vec(&parse_f32(val)?),
        "f64" => encode_to_vec(&parse_f64(val)?),
        "bool" => encode_to_vec(&match val {
            "true" => true,
            "false" => false,
            _ => return Err(format!("unknown bool {val}")),
        }),
        "char" => encode_to_vec(&match val {
            "A" => 'A',
            "han" => '한',
            _ => return Err(format!("unknown char fixture {val}")),
        }),
        "string" => encode_to_vec(&match val {
            "empty" => "".to_string(),
            "hello" => "hello".to_string(),
            "han" => "안녕".to_string(),
            _ => return Err(format!("unknown string fixture {val}")),
        }),
        "nat" => {
            let v = match val {
                "0" => BigNat::default(),
                "1" => BigNat::from_u64(1),
                "u64max" => BigNat::from_u64(u64::MAX),
                _ => return Err(format!("unknown nat fixture {val}")),
            };
            encode_to_vec(&v)
        }
        "int" => {
            let v = match val {
                "0" => BigInt::from_i64(0),
                "-1" => BigInt::from_i64(-1),
                "+0xdeadbeefcafebabe" => BigInt {
                    negative: false,
                    magnitude: BigNat::from_u64(0xdead_beef_cafe_babe),
                },
                _ => return Err(format!("unknown int fixture {val}")),
            };
            encode_to_vec(&v)
        }
        other => return Err(format!("unknown kind {other}")),
    })
}

// ── helpers ──────────────────────────────────────────────────────────

fn parse_hex_u64(s: &str) -> Result<u64, String> {
    let trim = s.strip_prefix("0x").unwrap_or(s);
    u64::from_str_radix(trim, 16).map_err(|e| format!("bad hex: {e}"))
}

fn parse_uint<T>(s: &str) -> Result<T, String>
where
    T: TryFrom<u64>,
    <T as TryFrom<u64>>::Error: std::fmt::Debug,
{
    let n = parse_hex_u64(s)?;
    T::try_from(n).map_err(|e| format!("uint overflow: {e:?}"))
}

fn parse_int_i8(val: &str) -> Result<i8, String> {
    Ok(match val {
        "0" => 0,
        "-1" => -1,
        "min" => i8::MIN,
        "max" => i8::MAX,
        _ => return Err(format!("unknown i8 fixture {val}")),
    })
}
fn parse_int_i16(val: &str) -> Result<i16, String> {
    Ok(match val {
        "0" => 0,
        "-1" => -1,
        "min" => i16::MIN,
        "max" => i16::MAX,
        _ => return Err(format!("unknown i16 fixture {val}")),
    })
}
fn parse_int_i32(val: &str) -> Result<i32, String> {
    Ok(match val {
        "0" => 0,
        "-1" => -1,
        "min" => i32::MIN,
        "max" => i32::MAX,
        _ => return Err(format!("unknown i32 fixture {val}")),
    })
}
fn parse_int_i64(val: &str) -> Result<i64, String> {
    Ok(match val {
        "0" => 0,
        "-1" => -1,
        "min" => i64::MIN,
        "max" => i64::MAX,
        _ => return Err(format!("unknown i64 fixture {val}")),
    })
}
fn parse_f32(val: &str) -> Result<f32, String> {
    Ok(match val {
        "0.0" => 0.0,
        "-0.0" => -0.0,
        _ => return Err(format!("unknown f32 fixture {val}")),
    })
}
fn parse_f64(val: &str) -> Result<f64, String> {
    Ok(match val {
        "pi" => std::f64::consts::PI,
        "0.0" => 0.0,
        "-0.0" => -0.0,
        _ => return Err(format!("unknown f64 fixture {val}")),
    })
}

#[test]
fn rust_encoder_matches_lean_fixture() {
    let path = fixture_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("skipping: fixture {path:?} missing; run tests/conformance/run.sh");
        return;
    };
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut total = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, expected)) = line.split_once('=') else {
            panic!("malformed line: {line}");
        };
        total += 1;
        match rust_bytes(name) {
            Err(e) => failures.push((name.to_string(), e)),
            Ok(actual) => {
                let actual_hex = hex_of(&actual);
                if actual_hex != expected {
                    failures.push((
                        name.to_string(),
                        format!("expected {expected}, got {actual_hex}"),
                    ));
                }
            }
        }
    }
    if !failures.is_empty() {
        let msg = failures
            .iter()
            .map(|(n, e)| format!("  {n}: {e}"))
            .collect::<Vec<_>>()
            .join("\n");
        panic!(
            "{} of {total} fixtures diverged between Lean and Rust:\n{msg}",
            failures.len()
        );
    }
    assert!(total >= 20, "expected at least 20 fixture lines, got {total}");
}
