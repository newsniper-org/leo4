//! Post-OX6 CLI refactor — per-(sub)crate `leo4.toml`
//! config file.
//!
//! Replaces the `--impl <kind>` CLI flag on `leo4 create`
//! / `leo4 init` (the flag is gone after the refactor).
//! Runtime-impl selection becomes a project property:
//!
//! ```toml
//! [[impl]]
//! kind = "mslean4"
//! out  = "target/leo4-mslean4"
//!
//! [[impl]]
//! kind = "rust-transpile"
//! out  = "target/leo4-rust-transpile"
//! ```
//!
//! Multiple `[[impl]]` entries are allowed; each impl's
//! `out` path **must** be disjoint from every other's
//! (overlap is rejected at config-parse time). `out` is
//! optional for a single-impl config and defaults to
//! `target/leo4-<kind>`.

use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// The per-(sub)crate `leo4.toml` deserialised form.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Leo4Config {
    /// One entry per runtime impl this (sub)crate
    /// targets. Order is preserved (matters for `leo4
    /// run` when no explicit `--impl` is passed — the
    /// first entry wins).
    #[serde(rename = "impl", default)]
    pub impls: Vec<ImplEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ImplEntry {
    /// Runtime impl identifier. Accepted values match
    /// the historical `--impl` flag: `mslean4` /
    /// `rust-native` (alias `rust`) / `rust-transpile`.
    pub kind: String,
    /// Optional output path for this impl's generated
    /// artefacts. Defaults to `target/leo4-<kind>` when
    /// absent. With multiple impls present, every entry
    /// MUST have a distinct (canonicalised relative)
    /// `out` path — overlap is rejected at
    /// `Leo4Config::validate` time.
    #[serde(default)]
    pub out: Option<String>,
}

/// Parse failure modes for `leo4.toml`. Distinct codes
/// per failure shape so callers (CLI / tests) can
/// produce targeted diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// `leo4.toml` not present at the expected path.
    NotFound,
    /// I/O error reading the file (e.g. permissions).
    Io(String),
    /// Malformed TOML or schema mismatch.
    Malformed(String),
    /// Zero `[[impl]]` entries — a config with no impl
    /// is meaningless.
    NoImpls,
    /// Unknown `kind` value in an `[[impl]]` entry.
    UnknownKind(String),
    /// Two `[[impl]]` entries share the same `out`
    /// path. `out` of the offending pair is in the
    /// payload.
    OverlappingOut(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "leo4.toml not found"),
            Self::Io(s) => write!(f, "leo4.toml read failed: {s}"),
            Self::Malformed(s) => write!(f, "leo4.toml malformed: {s}"),
            Self::NoImpls => write!(f, "leo4.toml has no [[impl]] entries"),
            Self::UnknownKind(k) => {
                write!(
                    f,
                    "leo4.toml: unknown impl kind `{k}`. Accepted: \
                     `mslean4`, `rust-native` (alias `rust`), \
                     `rust-transpile`."
                )
            }
            Self::OverlappingOut(p) => {
                write!(
                    f,
                    "leo4.toml: two [[impl]] entries share `out = {p:?}`. \
                     Each impl's output path MUST be disjoint."
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl Leo4Config {
    /// Load + validate a `leo4.toml` at `dir/leo4.toml`.
    /// Returns `ConfigError::NotFound` if absent.
    pub fn load_from_dir(dir: &Path) -> Result<Self, ConfigError> {
        let p = dir.join("leo4.toml");
        if !p.exists() {
            return Err(ConfigError::NotFound);
        }
        let raw = fs::read_to_string(&p).map_err(|e| ConfigError::Io(e.to_string()))?;
        Self::parse_str(&raw)
    }

    /// Parse + validate from a TOML source string.
    /// Exposed for testing.
    ///
    /// Named `parse_str` (not `from_str`) so it doesn't
    /// shadow the `std::str::FromStr::from_str` trait
    /// method — implementing `FromStr` here would force
    /// `String` parameter on the trait, but we want the
    /// dedicated `ConfigError` shape.
    pub fn parse_str(raw: &str) -> Result<Self, ConfigError> {
        let cfg: Self = toml::from_str(raw).map_err(|e| ConfigError::Malformed(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Enforce schema invariants:
    ///
    /// - At least one `[[impl]]` entry.
    /// - Each `kind` is a recognised impl identifier.
    /// - When more than one entry: every effective `out`
    ///   path is distinct (overlap rejected).
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.impls.is_empty() {
            return Err(ConfigError::NoImpls);
        }
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for e in &self.impls {
            if !is_known_kind(&e.kind) {
                return Err(ConfigError::UnknownKind(e.kind.clone()));
            }
            let out = effective_out(e);
            if !seen.insert(out.clone()) {
                return Err(ConfigError::OverlappingOut(out));
            }
        }
        Ok(())
    }

    /// Render this config to a TOML string suitable for
    /// `leo4 create` / `leo4 init` to write into the
    /// scaffold. The output is human-editable and
    /// includes inline comments documenting each field.
    #[must_use]
    pub fn render(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        out.push_str("# leo4 per-(sub)crate config — runtime impl selection.\n");
        out.push_str("# One [[impl]] entry per runtime; multiple entries are\n");
        out.push_str("# allowed but each impl's `out` path must be disjoint.\n");
        out.push('\n');
        for e in &self.impls {
            out.push_str("[[impl]]\n");
            let _ = writeln!(out, "kind = \"{}\"", e.kind);
            if let Some(o) = &e.out {
                let _ = writeln!(out, "out  = \"{o}\"");
            } else {
                let _ = writeln!(
                    out,
                    "out  = \"{}\"  # default; uncomment to override",
                    default_out_for(&e.kind)
                );
            }
            out.push('\n');
        }
        out
    }
}

impl ImplEntry {
    /// Convenience constructor for a kind with default
    /// `out` path.
    #[must_use]
    pub fn new(kind: &str) -> Self {
        Self {
            kind: kind.to_string(),
            out: None,
        }
    }
}

/// True iff `kind` is a recognised impl identifier
/// (mirrors `parse_impl_kind` in `main.rs`'s legacy
/// flag path so users see the same accepted-values
/// diagnostic regardless of which surface they hit).
fn is_known_kind(kind: &str) -> bool {
    matches!(kind, "mslean4" | "rust-native" | "rust" | "rust-transpile")
}

/// Default `out` path for a given impl kind.
fn default_out_for(kind: &str) -> String {
    // `rust` (the alias for `rust-native`) uses the
    // canonical path under the canonical name.
    let canonical = if kind == "rust" { "rust-native" } else { kind };
    format!("target/leo4-{canonical}")
}

/// Effective `out` path for an impl entry — either the
/// explicit `out` field or the default derived from kind.
fn effective_out(e: &ImplEntry) -> String {
    e.out.clone().unwrap_or_else(|| default_out_for(&e.kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_impl_default_out() {
        let cfg = Leo4Config::parse_str(
            r#"
[[impl]]
kind = "mslean4"
"#,
        )
        .expect("must parse");
        assert_eq!(cfg.impls.len(), 1);
        assert_eq!(cfg.impls[0].kind, "mslean4");
        assert!(cfg.impls[0].out.is_none());
    }

    #[test]
    fn parse_single_impl_explicit_out() {
        let cfg = Leo4Config::parse_str(
            r#"
[[impl]]
kind = "mslean4"
out = "build/m4"
"#,
        )
        .expect("must parse");
        assert_eq!(cfg.impls[0].out.as_deref(), Some("build/m4"));
    }

    #[test]
    fn parse_multi_impl_disjoint_out_succeeds() {
        let cfg = Leo4Config::parse_str(
            r#"
[[impl]]
kind = "mslean4"
out  = "target/leo4-mslean4"

[[impl]]
kind = "rust-transpile"
out  = "target/leo4-rust-transpile"
"#,
        )
        .expect("must parse");
        assert_eq!(cfg.impls.len(), 2);
    }

    #[test]
    fn parse_multi_impl_overlapping_out_rejects() {
        let err = Leo4Config::parse_str(
            r#"
[[impl]]
kind = "mslean4"
out = "build/x"

[[impl]]
kind = "rust-transpile"
out = "build/x"
"#,
        )
        .expect_err("overlapping out must reject");
        assert!(matches!(err, ConfigError::OverlappingOut(ref p) if p == "build/x"));
    }

    #[test]
    fn parse_multi_impl_default_paths_disjoint() {
        // Two different kinds with NO explicit `out` →
        // their default paths differ (`target/leo4-X`,
        // `target/leo4-Y`), so validation passes.
        let cfg = Leo4Config::parse_str(
            r#"
[[impl]]
kind = "mslean4"

[[impl]]
kind = "rust-transpile"
"#,
        )
        .expect("default paths are disjoint");
        assert_eq!(cfg.impls.len(), 2);
    }

    #[test]
    fn parse_multi_impl_same_kind_collides_by_default() {
        // Two `[[impl]]` with the SAME kind and no
        // explicit `out` → both default to the same
        // path → rejected.
        let err = Leo4Config::parse_str(
            r#"
[[impl]]
kind = "mslean4"

[[impl]]
kind = "mslean4"
"#,
        )
        .expect_err("same kind + default paths must collide");
        assert!(matches!(err, ConfigError::OverlappingOut(_)));
    }

    #[test]
    fn empty_config_rejects() {
        let err = Leo4Config::parse_str("").expect_err("empty must reject");
        assert!(matches!(err, ConfigError::NoImpls));
    }

    #[test]
    fn unknown_kind_rejects() {
        let err = Leo4Config::parse_str(
            r#"
[[impl]]
kind = "not-a-real-impl"
"#,
        )
        .expect_err("unknown kind must reject");
        assert!(matches!(err, ConfigError::UnknownKind(ref k) if k == "not-a-real-impl"));
    }

    #[test]
    fn rust_alias_accepted() {
        Leo4Config::parse_str(
            r#"
[[impl]]
kind = "rust"
"#,
        )
        .expect("`rust` is an accepted alias for `rust-native`");
    }

    #[test]
    fn malformed_toml_rejects() {
        let err = Leo4Config::parse_str("[[impl]\nkind = \"mslean4\"\n")
            .expect_err("malformed TOML must reject");
        assert!(matches!(err, ConfigError::Malformed(_)));
    }

    #[test]
    fn render_round_trips_through_parser() {
        let cfg = Leo4Config {
            impls: vec![
                ImplEntry::new("mslean4"),
                ImplEntry {
                    kind: "rust-transpile".to_string(),
                    out: Some("custom/path".to_string()),
                },
            ],
        };
        let rendered = cfg.render();
        let reparsed = Leo4Config::parse_str(&rendered).expect("rendered output must reparse");
        assert_eq!(reparsed.impls.len(), 2);
        assert_eq!(reparsed.impls[0].kind, "mslean4");
        assert_eq!(reparsed.impls[1].out.as_deref(), Some("custom/path"));
    }

    #[test]
    fn render_includes_default_out_as_commented_hint() {
        let cfg = Leo4Config {
            impls: vec![ImplEntry::new("mslean4")],
        };
        let rendered = cfg.render();
        assert!(rendered.contains("target/leo4-mslean4"));
    }
}
