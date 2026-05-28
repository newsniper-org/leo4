//! Option-A transpile pipeline — emit a **pure native
//! Rust crate** from a Lean source set.
//!
//! Contrast with the canonical path
//! (`synthesize_canonical_wrapper` + `emit_crate` /
//! `emit_lib_rs` + `LeanProc` impl): that path is
//! cross-impl conformant (the rust-transpile output
//! receives the same canonical-ABI `leo4_call` shape
//! the mslean4 path produces), at the cost of
//! mangling, schema_hash, and boundary
//! encode/decode on every call. Option A drops all
//! of that. The transpiled crate is just plain
//! Rust:
//!
//! ```ignore
//! // transpiled/src/lib.rs
//! pub fn add(a: u64, b: u64) -> u64 { ... }
//! ```
//!
//! User consumers call `transpiled::add(1, 2)`
//! directly — no C ABI hop, no mangling, no
//! `leo4-abi` runtime dependency on the emitted
//! crate's Cargo.toml.
//!
//! When this is the right choice:
//! - You target the OxiLean transpile path only and
//!   don't need wire-compatibility with the mslean4
//!   path on the same schema.
//! - Per the OX5-oxi promise: OxiLean-only users
//!   install nothing beyond `leo4-oxilean-build`.
//!   Option A keeps that promise *also* zero-cost
//!   at boundary call sites.
//!
//! When to stay on the canonical path instead:
//! - You need a single Rust crate that can be loaded
//!   by *either* the rust-transpile or the mslean4
//!   dispatcher (the canonical path's `LeanProc`
//!   impl supports this).
//! - You want cross-impl conformance fixtures to
//!   verify byte-identical wire payloads.

use leo4_abi::LeanError;
use oxilean_kernel::env::Environment;
use oxilean_parse::Decl as OxDecl;
use std::path::Path;

use crate::{parse_decls_for_transpile, unfold_decl};
use oxilean_elab::elab_decl::{elaborate_decl, PendingDecl};

/// One transpiled native function from a Lean source.
/// `name` is the declared Lean name (no mangling); the
/// `source` is the `pub fn <name>(...) -> ...`
/// emit from `oxilean-codegen::rust_target_backend`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PureFn {
    pub name: String,
    pub source: String,
}

/// The transpiled output of one or more Lean source
/// files: every `def` becomes a `pub fn` in the
/// emitted crate's `src/lib.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PureCrate {
    /// Cargo crate name. Maps to `[package].name` in
    /// the emitted Cargo.toml.
    pub crate_name: String,
    /// All native function sources, in declaration
    /// order across all input files.
    pub fns: Vec<PureFn>,
}

impl PureCrate {
    /// Render `Cargo.toml`. Intentionally minimal —
    /// edition, package name + version, no
    /// dependencies (option-A's whole point).
    #[must_use]
    pub fn render_cargo_toml(&self) -> String {
        format!(
            "[package]\nname = \"{}\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[lib]\npath = \"src/lib.rs\"\n",
            self.crate_name
        )
    }

    /// Render `src/lib.rs`. Concatenates every
    /// transpiled fn separated by a blank line.
    /// Adds a header doc-comment naming the crate as
    /// transpiled output (for diagnostic clarity in
    /// case a user ends up reading it).
    #[must_use]
    pub fn render_lib_rs(&self) -> String {
        let mut out = String::new();
        out.push_str("//! Transpiled by `leo4-oxilean-build` (option A: pure native Rust,\n");
        out.push_str("//! no canonical-ABI wrapper). Do not edit — re-run the transpile\n");
        out.push_str("//! to regenerate.\n");
        out.push_str("#![allow(clippy::all, dead_code, unused_variables, unused_imports)]\n\n");
        for (i, f) in self.fns.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&f.source);
            out.push('\n');
        }
        out
    }

    /// Write `Cargo.toml` + `src/lib.rs` to `out_dir`.
    /// Creates `out_dir/src/` if it doesn't exist.
    ///
    /// # Errors
    /// `LeanError` for I/O failures.
    pub fn write_to_dir(&self, out_dir: &Path) -> Result<(), LeanError> {
        let src_dir = out_dir.join("src");
        std::fs::create_dir_all(&src_dir).map_err(|e| {
            LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                format!("pure_emit: create {}: {e}", src_dir.display()),
            )
        })?;
        std::fs::write(out_dir.join("Cargo.toml"), self.render_cargo_toml()).map_err(|e| {
            LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                format!("pure_emit: write Cargo.toml: {e}"),
            )
        })?;
        std::fs::write(src_dir.join("lib.rs"), self.render_lib_rs()).map_err(|e| {
            LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                format!("pure_emit: write src/lib.rs: {e}"),
            )
        })?;
        Ok(())
    }
}

/// Transpile one Lean source string into a list of
/// `PureFn`s, one per top-level `def`.
///
/// **No `@[leo4_export]` filter** — option A emits
/// every `def` as a `pub fn`. The canonical path's
/// tag-discovery convention exists to let the
/// dispatcher know which functions are reachable via
/// `leo4_call`; the pure path doesn't have a
/// dispatcher, so the tag is moot.
///
/// # Errors
/// `LeanError` for any failure in the underlying
/// parse / elab / LCNF / Rust-emit pipeline.
pub fn transpile_source_to_pure_fns(
    env: &Environment,
    src: &str,
) -> Result<Vec<PureFn>, LeanError> {
    // Same `lean4_normalize` pre-pass the canonical
    // path uses — keeps both paths in step on the
    // surface accepted by oxilean-parse fallback.
    let normalised = crate::lean4_normalize(src);
    let parsed_all = parse_decls_for_transpile(&normalised)?;

    // OX7 multi-decl env threading (2026-05-28). Mirrors the
    // change in `transpile_source_to_units` — clone the
    // caller's env so each elaborated decl re-enters a
    // *local* environment, making cross-fn references within
    // the same source visible to later decls' elaboration.
    let mut local_env = env.clone();
    let mut out = Vec::new();
    for decl in &parsed_all {
        let inner = crate::inner_decl(decl);
        if let OxDecl::Definition { name, .. } = &inner.value {
            let pending = elaborate_decl(&local_env, &inner.value).map_err(|e| {
                LeanError::new(
                    leo4_abi::error::error_codes::DECODE_ERROR,
                    format!("pure_emit: elaborate_decl({name}): {e:?}"),
                )
            })?;
            let (pname, ty, val) = match pending {
                PendingDecl::Definition { name, ty, val, .. } => (name, ty, val),
                other => {
                    // Skip non-Definition outputs
                    // (axioms, theorems, …) silently —
                    // option A is fn-only.
                    let _ = other;
                    continue;
                }
            };
            // Register the just-elaborated def into the local
            // env BEFORE codegen so any later decl in this
            // source can resolve a cross-fn call to it.
            // DuplicateDeclaration is treated as no-op (some
            // bootstrap envs ship the same name as a stub).
            {
                use oxilean_kernel::{
                    env::Declaration,
                    reduce::ReducibilityHint,
                };
                let decl_for_env = Declaration::Definition {
                    name: pname.clone(),
                    univ_params: Vec::new(),
                    ty: ty.clone(),
                    val: val.clone(),
                    hint: ReducibilityHint::Regular(1),
                };
                let _ = local_env.add(decl_for_env);
            }
            // OX7 (#1, 2026-05-26): forward the declared
            // return type from `unfold_decl` so the
            // emitted Rust fn signature honours the
            // declaration's signature instead of falling
            // back to `Box<dyn Any>` for
            // TailCall-terminated bodies.
            let (params, body, ret_type) = unfold_decl(&ty, &val);
            let source = crate::transpile_kernel_decl_with_ret_type(
                &pname,
                &params,
                Some(&ret_type),
                &body,
            )?;
            out.push(PureFn {
                name: pname.to_string(),
                source,
            });
        }
    }
    Ok(out)
}

/// Transpile multiple Lean source files into one
/// `PureCrate`. `crate_name` is the resulting Cargo
/// crate's `[package].name`. `sources` is an
/// in-declaration-order list of source strings (one
/// per Lean file).
///
/// # Errors
/// `LeanError` for any per-file transpile failure.
pub fn transpile_sources_to_pure_crate(
    env: &Environment,
    crate_name: &str,
    sources: &[&str],
) -> Result<PureCrate, LeanError> {
    // OX7 multi-decl env threading (2026-05-28) extended
    // across files: clone once, thread the env through every
    // source's transpile so a `def` in file N can reference a
    // `def` from file 0..N-1. `transpile_source_to_pure_fns`
    // would otherwise clone the (immutable) `env` per call
    // and lose cross-file decls.
    let mut local_env = env.clone();
    let mut fns = Vec::new();
    for src in sources {
        let mut per = transpile_source_to_pure_fns_inner(&mut local_env, src)?;
        fns.append(&mut per);
    }
    Ok(PureCrate {
        crate_name: crate_name.to_string(),
        fns,
    })
}

/// Inner of `transpile_source_to_pure_fns` parameterised on
/// `&mut Environment` so the caller can thread env-decls
/// across calls (multi-file path). The pub fn above keeps
/// the historical `&Environment` shape for single-file
/// embedders.
fn transpile_source_to_pure_fns_inner(
    local_env: &mut Environment,
    src: &str,
) -> Result<Vec<PureFn>, LeanError> {
    let normalised = crate::lean4_normalize(src);
    let parsed_all = parse_decls_for_transpile(&normalised)?;
    let mut out = Vec::new();
    for decl in &parsed_all {
        let inner = crate::inner_decl(decl);
        if let OxDecl::Definition { name, .. } = &inner.value {
            let pending = elaborate_decl(local_env, &inner.value).map_err(|e| {
                LeanError::new(
                    leo4_abi::error::error_codes::DECODE_ERROR,
                    format!("pure_emit: elaborate_decl({name}): {e:?}"),
                )
            })?;
            let (pname, ty, val) = match pending {
                PendingDecl::Definition { name, ty, val, .. } => (name, ty, val),
                other => {
                    let _ = other;
                    continue;
                }
            };
            {
                use oxilean_kernel::{
                    env::Declaration,
                    reduce::ReducibilityHint,
                };
                let decl_for_env = Declaration::Definition {
                    name: pname.clone(),
                    univ_params: Vec::new(),
                    ty: ty.clone(),
                    val: val.clone(),
                    hint: ReducibilityHint::Regular(1),
                };
                let _ = local_env.add(decl_for_env);
            }
            let (params, body, ret_type) = unfold_decl(&ty, &val);
            let source = crate::transpile_kernel_decl_with_ret_type(
                &pname,
                &params,
                Some(&ret_type),
                &body,
            )?;
            out.push(PureFn {
                name: pname.to_string(),
                source,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leo4_env_bootstrap::bootstrap_env;

    #[test]
    fn pure_crate_render_cargo_toml_has_no_deps() {
        let c = PureCrate {
            crate_name: "transpiled".to_string(),
            fns: vec![],
        };
        let toml = c.render_cargo_toml();
        assert!(toml.contains("name = \"transpiled\""));
        assert!(toml.contains("edition = \"2024\""));
        // Option-A invariant: no leo4-abi, no
        // leo4-mslean4, no dispatcher dep.
        assert!(!toml.contains("leo4-abi"));
        assert!(!toml.contains("leo4-mslean4"));
        assert!(!toml.contains("[dependencies]"));
    }

    #[test]
    fn pure_crate_render_lib_rs_concatenates_fns() {
        let c = PureCrate {
            crate_name: "x".into(),
            fns: vec![
                PureFn {
                    name: "a".into(),
                    source: "pub fn a() -> u64 { 1 }".into(),
                },
                PureFn {
                    name: "b".into(),
                    source: "pub fn b() -> u64 { 2 }".into(),
                },
            ],
        };
        let rs = c.render_lib_rs();
        assert!(rs.contains("pub fn a() -> u64 { 1 }"));
        assert!(rs.contains("pub fn b() -> u64 { 2 }"));
        assert!(rs.contains("Transpiled by `leo4-oxilean-build`"));
    }

    #[test]
    fn pure_crate_write_to_dir_creates_src_layout() {
        // Use std::env::temp_dir + a unique nanosecond
        // suffix to avoid pulling in `tempfile`.
        let dir = std::env::temp_dir().join(format!(
            "leo4-oxilean-build-pure-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let c = PureCrate {
            crate_name: "p".into(),
            fns: vec![PureFn {
                name: "noop".into(),
                source: "pub fn noop() {}".into(),
            }],
        };
        c.write_to_dir(&dir).expect("write must succeed");
        assert!(dir.join("Cargo.toml").exists());
        assert!(dir.join("src/lib.rs").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_input_returns_empty_crate() {
        let env = bootstrap_env().expect("env bootstrap");
        let crate_ = transpile_sources_to_pure_crate(&env, "transpiled", &[])
            .expect("empty input must succeed");
        assert_eq!(crate_.fns.len(), 0);
        assert_eq!(crate_.crate_name, "transpiled");
    }
}
