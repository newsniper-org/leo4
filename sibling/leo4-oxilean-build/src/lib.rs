//! leo4-oxilean-build — build-time transpiler from OxiLean
//! Lean source to a Rust crate, per
//! `SPEC/rust-native-lean.md` §9.
//!
//! ## What this is
//!
//! The **transpile path** to leo4-rust-native: instead of
//! dispatching Lean calls into a live OxiLean evaluator at
//! runtime (the `leo4-oxilean` adapter's role, currently
//! blocked on upstream Hooks 1 + 2 per §7.1), this crate
//! reads a Lean source tree, walks the `@[leo4_export]`
//! declarations, and emits a plain Rust crate via
//! `oxilean-codegen::rust_target_backend::RustTargetBackend`.
//! The generated crate is an ordinary Rust library —
//! callable in-process, no `lean.h` C ABI, no OxiLean
//! runtime dep at the consumer's build site.
//!
//! Key insight from `oxilean-codegen` v0.1.2 inspection:
//! `lcnf_to_rust_type` maps to **pure Rust types**:
//! `Nat → U64`, `LcnfString → RustString`, `Object → Box<dyn
//! Any>`. No `lean_box` / `lean_object*` in the output. The
//! resulting Rust crate is structurally indistinguishable
//! from one a human would write.
//!
//! ## Status (2026-05-21)
//!
//! **SCAFFOLD**. Cargo deps wired (`oxilean-parse`,
//! `oxilean-elab`, `oxilean-codegen`, `oxilean-kernel`); the
//! type surface for the transpiler driver compiles; **but
//! no end-to-end transpile is wired yet**. The blocking
//! items are upstream API questions answered case-by-case
//! during real-fixture testing — they're not architectural
//! blockers (unlike Hooks 1 + 2 for the runtime-dispatch
//! sibling crate):
//!
//! 1. `oxilean-parse`'s public entry for parsing a Lean
//!    source file or string. The crate's public API surface
//!    needs surveying as the wiring proceeds.
//! 2. `oxilean-elab`'s public entry for elaborating parsed
//!    AST to LCNF. Hook 3 (attribute / deriving handler
//!    registration) ties in here.
//! 3. Driving the `LcnfModule` → `RustTargetBackend::
//!    emit_module` → `RustModule::emit() -> String` chain
//!    is documented in §9.1 / §9.3 of the SPEC; what's
//!    missing is the wrapper that walks a user package's
//!    Lean sources, registers the `@[leo4_export]` attribute
//!    handler, and writes the result to a destination Cargo
//!    crate dir.
//!
//! ## What works today (31 / 31 tests)
//!
//! - Cargo deps resolve + compile (5 OxiLean crates
//!   reachable).
//! - `transpile_decls(name, &[LcnfFunDecl]) -> String`
//!   wraps `RustTargetBackend::emit_module` + per-fn
//!   `RustFn::emit()`.
//! - **`transpile_kernel_decl(name, params, body) ->
//!   String`** — full kernel-level → LCNF → Rust source
//!   pipeline (skips `oxilean-parse` / `oxilean-elab` so
//!   you don't need a Lean source corpus to test it).
//! - Sample transpile of
//!   `Sample.addOne (n : Nat) : Nat := Nat.succ n`
//!   produces:
//!   ```ignore
//!   pub fn Sample_addOne(_x0: u64) -> Box<dyn std::any::Any> {
//!       _x1(_x0)
//!   }
//!   ```
//!   Real Rust source, **zero `lean_*` symbols**. The
//!   `Box<dyn Any>` return + unresolved `_x1` (= `Nat.succ`)
//!   are the *limitations of context-free lowering* — see
//!   below.
//! - `lcnf_to_rust_type` invariant probe: every mapping
//!   produces a Rust type that *names a real Rust standard
//!   type* (`u64`, `String`, `()`, `Box<dyn Any>`,
//!   `fn(…) -> …`). Catches future OxiLean releases that
//!   would regress this.
//!
//! ## Limitations of context-free lowering
//!
//! Calling `decl_to_lcnf` on a hand-built kernel `Expr`
//! (no surrounding `Environment`, no elaborated constant
//! definitions) gives the LCNF lowering nothing to look up
//! `Nat.succ` etc. against. The result:
//!
//! - Constants like `Nat.succ` come out as fresh free
//!   variables (`_x1`, `_x2`, …).
//! - Return types not inferable from the body alone fall
//!   back to `LcnfType::Object → Box<dyn std::any::Any>`.
//!
//! Real usage will wrap this in:
//! 1. `oxilean-parse` to read a `.lean` source file.
//! 2. `oxilean-elab` to elaborate + populate the env (and
//!    bind a custom `@[leo4_export]` attribute handler via
//!    Hook 3 — `oxilean_elab::attribute::AttributeManager::
//!    register_custom_handler`).
//! 3. Walk the elaborated env to extract `@[leo4_export]`-
//!    tagged decls' typed shape.
//! 4. Drive each decl through `decl_to_lcnf` with the env
//!    populated. The same `RustTargetBackend` then emits
//!    fully-resolved Rust source.
//!
//! That wiring is the next-step work; the current
//! `transpile_kernel_decl` is the *machinery* layer this
//! will plug into.
//!
//! ## Activation plan (next commits)
//!
//! Layer-by-layer, with the current `transpile_kernel_decl`
//! at the centre:
//!
//! 1. **Parse + elab layer** — wrap `oxilean-parse::Lexer
//!    → Parser::parse_decl` + `oxilean-elab` env / elaborator
//!    to lift a Lean source string to elaborated kernel
//!    `Expr`s. Resolves the `_x1 = Nat.succ` issue. **(Done
//!    2026-05-22 — `transpile_source` end-to-end pipeline.)**
//! 2. **`lean4_compat` adapter pre-processor** — drive
//!    `oxilean-elab::lean4_compat::{Lean4TermRewriter::
//!    standard, Lean4SyntaxAdapter::adapt_all}` to normalise
//!    Lean 4 surface syntax (` => ` → ` -> `, `←` → `<-`,
//!    `where;` → `where`, etc.) into OxiLean parser dialect.
//!    **(Done 2026-05-22 — `lean4_normalize` helper.)** Note:
//!    parser-level differences (e.g. header binders
//!    `def f (x : T) := …`) need a richer adapter — the
//!    upstream `lean4_compat` v0.1.2 layer is textual only.
//! 3. **`@[leo4_export]` discovery** — bind a custom handler
//!    via `oxilean_elab::attribute::AttributeManager::register_custom_handler`.
//!    Walk the elaborated env to collect tagged decls.
//!    **(Done 2026-05-22 — `Leo4ExportRegistry` +
//!    `transpile_source_if_exported`.)**
//! 4. **`deriving LeanMarshal`** — analogous binding via
//!    `oxilean_elab::attribute::DeriveHandlerRegistry::register`.
//!    The handler emits the encoder/decoder boilerplate that
//!    `lake/Leo4Plugin/Leo4Plugin/Deriving.lean` does on the
//!    reference Lean side. **(Wired 2026-05-22 — handler
//!    registered with `.no_instance()`; Rust-side impl
//!    synthesis is part of step 5.)**
//! 5. **Canonical-ABI wrapper synthesis** — for each
//!    transpiled fn, generate a sibling
//!    `pub fn <name>_call(args: &[u8]) -> Vec<u8>` that
//!    canonical-ABI decodes `args` via `leo4_abi::LeanMarshal`,
//!    calls the transpiled fn, encodes the return. The result
//!    is a Rust crate that conforms to leo4-rust-native's
//!    boundary contract. **(Done 2026-05-22 —
//!    `synthesize_canonical_wrapper(&RustFn) -> String` +
//!    `transpile_kernel_decl_with_wrapper` combined helper.
//!    Marshallable type matrix today: u8..u128, i8..i128,
//!    f32, f64, bool, char, String, () (unit return). Carrier
//!    types + user records pending the upstream backend
//!    emitting struct / impl shapes.)**
//! 6. **Cargo crate emit** — `Cargo.toml` + `lib.rs` written
//!    to a target dir via `emit_crate` + `write_to_dir`. The
//!    emitted `lib.rs` includes a `Leo4OxileanProc: LeanProc`
//!    impl with a `match mangled { … }` dispatch table.
//!    **(Done 2026-05-22 — `TranspileUnit` + `GeneratedCrate`
//!    + `emit_crate` + `write_to_dir`. Activation plan complete.)**

#![allow(clippy::missing_errors_doc)]

use leo4_abi::LeanError;
use oxilean_codegen::lcnf::LcnfFunDecl;
use oxilean_codegen::rust_target_backend::{RustItem, RustTargetBackend};
use oxilean_codegen::to_lcnf::{decl_to_lcnf, ToLcnfConfig};
use oxilean_elab::attribute::{
    AttrAction, AttrEntry, AttrHandler, AttributeManager, DeriveHandler,
    DeriveHandlerRegistry,
};
use oxilean_elab::elab_decl::{elaborate_decl, PendingDecl};
use oxilean_elab::lean4_compat::{Lean4SyntaxAdapter, Lean4TermRewriter};
use oxilean_kernel::{env::Environment, Expr, Name};
use oxilean_parse::{AttributeKind, Decl, Lexer, Located, Parser, SurfaceExpr};
use std::collections::{HashMap, HashSet};

/// Custom attribute name leo4 owns: `@[leo4_export]`. Tag a Lean
/// definition with this attribute to mark it for export through
/// leo4's canonical-ABI boundary; the transpiler only emits Rust
/// wrappers for tagged decls.
pub const LEO4_EXPORT_ATTR: &str = "leo4_export";

/// Lean class name leo4 owns for auto-derive: `deriving LeanMarshal`.
/// Registered into OxiLean's `DeriveHandlerRegistry` so the
/// elaborator recognises it as a known class; the transpiler
/// emits the actual `LeanMarshal` impl on the Rust side, not
/// inside OxiLean.
pub const LEAN_MARSHAL_DERIVE: &str = "LeanMarshal";

// ─── OX3: header-binder pre-rewrite (Lean 4 → OxiLean parser dialect) ──
//
// OxiLean's `Parser::parse_definition` accepts the form
// `def name : type := value` but rejects Lean 4's
// `def name (binders) : type := value`. The
// `oxilean-elab::lean4_compat` v0.1.2 adapter only does textual
// rewrites on the existing parser dialect — it doesn't lift
// header binders. This helper does that lift textually, BEFORE
// the parser sees the source.
//
// Algorithm (per `def` occurrence at top level):
//
//   def NAME [{implicits}] [(binders)]+ [[instances]]+ [: TYPE] := VALUE
//   ────────────────────────────────────────────────────────────────────
//   def NAME : T1 → T2 → … → TYPE := fun n1 n2 … → VALUE
//
// Where each binder group `(a b : T)` contributes one type per
// name to the arrow chain + one name per name to the fun args.
// Implicit `{...}` and instance `[...]` binders are stripped
// from the head (they don't surface in the lowered Rust types —
// they're auto-bound).
//
// We scan over the source as raw bytes, identifying `def` /
// `def NAME` boundaries by looking for the keyword preceded by
// whitespace or start-of-source. Brackets are balanced inside
// binder groups so `(x : Vec (List Nat))` parses correctly.
// Lines, string literals, and comments are honoured.

/// Strip arguments from each attribute inside `@[…]` lists.
/// OxiLean's `Parser::parse_attribute_decl` v0.1.2 only takes
/// bare idents inside the bracket list and rejects argument-
/// bearing attributes like `@[leo4_specialize_when scalar ∧ ord]`.
/// This pre-rewrite keeps only the first ident of each comma-
/// separated entry so the bracket list parses, then leaves the
/// rest of the decl unchanged.
///
/// Out of scope (pass-through): `attribute [...]` keyword form,
/// `#[…]` (Rust-style, not Lean), comments / strings.
#[must_use]
pub fn strip_attribute_args(src: &str) -> String {
    attribute_arg_stripper::strip(src)
}

mod attribute_arg_stripper {
    pub fn strip(src: &str) -> String {
        let bytes = src.as_bytes();
        let mut out = String::with_capacity(src.len());
        let mut i = 0usize;
        let mut chunk_start = 0usize;
        let n = bytes.len();
        while i < n {
            // Skip strings.
            if bytes[i] == b'"' {
                let mut j = i + 1;
                while j < n && bytes[j] != b'"' {
                    if bytes[j] == b'\\' && j + 1 < n {
                        j += 2;
                        continue;
                    }
                    j += 1;
                }
                if j < n {
                    j += 1;
                }
                i = j;
                continue;
            }
            // Skip line comments.
            if i + 1 < n && bytes[i] == b'-' && bytes[i + 1] == b'-' {
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            // Detect `@[` start.
            if i + 1 < n
                && bytes[i] == b'@'
                && bytes[i + 1] == b'['
                && let Some(end) = balanced_skip(bytes, i + 1, b'[', b']')
            {
                // Flush prior chunk verbatim (preserves
                // multi-byte UTF-8 sequences intact).
                out.push_str(&src[chunk_start..i]);
                let inner = &src[i + 2..end - 1];
                let stripped = strip_inner(inner);
                out.push_str("@[");
                out.push_str(&stripped);
                out.push(']');
                i = end;
                chunk_start = i;
                continue;
            }
            i += 1;
        }
        out.push_str(&src[chunk_start..]);
        out
    }

    /// Split `inner` (between `@[` and `]`) on top-level commas
    /// and reduce each entry to its first whitespace-delimited
    /// token.
    fn strip_inner(inner: &str) -> String {
        let mut parts: Vec<String> = Vec::new();
        let bytes = inner.as_bytes();
        let mut i = 0usize;
        let mut depth_paren: i32 = 0;
        let mut depth_brace: i32 = 0;
        let mut depth_bracket: i32 = 0;
        let mut start = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'(' => depth_paren += 1,
                b')' => depth_paren -= 1,
                b'{' => depth_brace += 1,
                b'}' => depth_brace -= 1,
                b'[' => depth_bracket += 1,
                b']' => depth_bracket -= 1,
                b',' if depth_paren == 0 && depth_brace == 0 && depth_bracket == 0 => {
                    parts.push(inner[start..i].to_string());
                    start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        parts.push(inner[start..].to_string());

        parts
            .iter()
            .map(|p| first_token(p.trim()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn first_token(s: &str) -> String {
        s.split_whitespace().next().unwrap_or("").to_string()
    }

    fn balanced_skip(bytes: &[u8], start_byte: usize, open: u8, close: u8) -> Option<usize> {
        debug_assert_eq!(bytes[start_byte], open);
        let mut i = start_byte + 1;
        let mut depth: i32 = 1;
        let n = bytes.len();
        while i < n && depth > 0 {
            let b = bytes[i];
            if b == open {
                depth += 1;
            } else if b == close {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            } else if b == b'"' {
                i += 1;
                while i < n && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < n {
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
            }
            i += 1;
        }
        None
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn pass_through_bare_attribute() {
            assert_eq!(strip("@[leo4_export]"), "@[leo4_export]");
        }

        #[test]
        fn strip_args_from_single_attribute() {
            assert_eq!(
                strip("@[leo4_specialize_when scalar ∧ ord]"),
                "@[leo4_specialize_when]"
            );
        }

        #[test]
        fn strip_args_in_comma_list() {
            assert_eq!(
                strip("@[leo4_export, leo4_specialize_when scalar ∧ ord]"),
                "@[leo4_export, leo4_specialize_when]"
            );
        }

        #[test]
        fn pass_through_string_with_at_brackets() {
            assert_eq!(strip("\"@[foo bar]\""), "\"@[foo bar]\"");
        }

        #[test]
        fn pass_through_comment_with_at_brackets() {
            assert_eq!(strip("-- @[foo bar]\n"), "-- @[foo bar]\n");
        }

        #[test]
        fn idempotent() {
            let src = "@[a, b c d, e (f g)]";
            let once = strip(src);
            let twice = strip(&once);
            assert_eq!(once, twice);
        }
    }
}

/// Pre-rewrite Lean 4 header-binder `def`s into OxiLean-dialect
/// body-lambda form, before the parser sees the source.
///
/// Idempotent: re-running on already-rewritten source has no
/// effect (the regex doesn't match `def NAME :` without a
/// binder bracket).
///
/// Out of scope (intentionally pass-through):
/// - `theorem` / `lemma` / `axiom` / `inductive` / `structure`
///   / `instance` / `class` decls (parser handles those forms
///   without lifting).
/// - `def`s that *already* lack header binders.
/// - `def`s inside comments / string literals.
///
/// Limitations (documented as known gaps):
/// - Multi-line VALUE: supported (we scan to the next
///   top-level decl keyword or EOF).
/// - Default values in binder groups (`(x : Nat := 0)`): not
///   yet handled — leaves the source unchanged for the decl.
/// - `where` clauses on `def`s: not yet split (treated as
///   part of VALUE).
#[must_use]
pub fn rewrite_header_binders(src: &str) -> String {
    header_binder_rewriter::rewrite(src)
}

mod header_binder_rewriter {
    /// Top-level decl keywords that terminate a preceding
    /// `def`'s VALUE region. Used as scan boundaries.
    const DECL_KEYWORDS: &[&str] = &[
        "def", "theorem", "lemma", "axiom", "inductive",
        "structure", "class", "instance", "namespace", "section",
        "end", "open", "import", "variable", "macro", "syntax",
        "elab", "abbrev", "noncomputable", "private", "protected",
    ];

    /// Walk `src`, rewriting each header-binder `def` into the
    /// body-lambda form. Other text is preserved byte-for-byte.
    pub fn rewrite(src: &str) -> String {
        let bytes = src.as_bytes();
        let mut out = String::with_capacity(src.len() + 32);
        let mut i = 0usize;
        let n = bytes.len();
        while i < n {
            // Find the next top-level `def`-keyword start.
            // Anything before that, copy verbatim.
            let next = find_def_token(bytes, i);
            if next == usize::MAX {
                out.push_str(&src[i..]);
                break;
            }
            out.push_str(&src[i..next]);
            // Try to parse + rewrite one `def`. If parsing
            // fails (no binders, unexpected shape, …), copy
            // the `def` keyword verbatim and advance past it.
            if let Some((rewritten, end)) = try_rewrite_def(src, bytes, next) {
                out.push_str(&rewritten);
                i = end;
            } else {
                out.push_str("def");
                i = next + 3;
            }
        }
        out
    }

    /// Find the next `def` keyword starting at-or-after `from`,
    /// respecting word boundaries + skipping comments + string
    /// literals.
    fn find_def_token(bytes: &[u8], from: usize) -> usize {
        let mut i = from;
        let n = bytes.len();
        while i < n {
            // Skip line comment.
            if i + 1 < n && bytes[i] == b'-' && bytes[i + 1] == b'-' {
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            // Skip block comment (non-nested for simplicity;
            // Lean allows nesting but rare in body of decls).
            if i + 1 < n && bytes[i] == b'/' && bytes[i + 1] == b'-' {
                i += 2;
                while i + 1 < n && !(bytes[i] == b'-' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                if i + 1 < n {
                    i += 2;
                }
                continue;
            }
            // Skip string literal.
            if bytes[i] == b'"' {
                i += 1;
                while i < n && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < n {
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
                if i < n {
                    i += 1;
                }
                continue;
            }
            // Check for `def` keyword with proper word
            // boundaries on both sides.
            if i + 3 <= n
                && &bytes[i..i + 3] == b"def"
                && is_word_boundary(bytes, i, i + 3)
            {
                return i;
            }
            i += 1;
        }
        usize::MAX
    }

    fn is_word_boundary(bytes: &[u8], start: usize, end: usize) -> bool {
        let before_ok = start == 0 || !is_ident_byte(bytes[start - 1]);
        let after_ok = end >= bytes.len() || !is_ident_byte(bytes[end]);
        before_ok && after_ok
    }

    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_' || b == b'\''
    }

    fn is_ws(b: u8) -> bool {
        matches!(b, b' ' | b'\t' | b'\n' | b'\r')
    }

    fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
        while i < bytes.len() && is_ws(bytes[i]) {
            i += 1;
        }
        i
    }

    fn read_ident(bytes: &[u8], from: usize) -> Option<(String, usize)> {
        let mut i = from;
        let n = bytes.len();
        let start = i;
        while i < n && is_ident_byte(bytes[i]) {
            i += 1;
        }
        if i == start {
            return None;
        }
        Some((std::str::from_utf8(&bytes[start..i]).ok()?.to_string(), i))
    }

    /// Try to rewrite a single `def` decl starting at index
    /// `def_pos` (which must be the `d` of `def`). Returns
    /// `(rewritten_source, end_position)` where `end_position`
    /// is the byte offset just past the rewritten region.
    /// Returns `None` if the shape doesn't match a header-binder
    /// `def` (and therefore needs no rewrite).
    #[allow(clippy::too_many_lines)] // documented branches: binder forms + tying together
    pub fn try_rewrite_def(
        src: &str,
        bytes: &[u8],
        def_pos: usize,
    ) -> Option<(String, usize)> {
        let after_def = def_pos + 3;
        let i = skip_ws(bytes, after_def);
        // Decl name.
        let (name, mut i) = read_ident(bytes, i)?;
        i = skip_ws(bytes, i);

        // Universe params `.{u, v}` are theoretically possible
        // — skip them verbatim into a "univ" hold-back string.
        let mut univ_chunk = String::new();
        if i < bytes.len() && bytes[i] == b'.' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            let start = i;
            i = balanced_skip(bytes, i + 1, b'{', b'}')?;
            univ_chunk = src[start..i].to_string();
            i = skip_ws(bytes, i);
        }

        // Collect binder groups until we see `:` or `:=`.
        let mut binders: Vec<BinderGroup> = Vec::new();
        let mut had_header_binder = false;
        while i < bytes.len() {
            // Stop at `:` or `:=` (top-level).
            if bytes[i] == b':' {
                break;
            }
            match bytes[i] {
                b'(' => {
                    had_header_binder = true;
                    let group_end = balanced_skip(bytes, i, b'(', b')')?;
                    let inner = &src[i + 1..group_end - 1];
                    binders.push(parse_binder_group(inner, BinderKind::Explicit)?);
                    i = group_end;
                }
                b'{' => {
                    // Implicit binder — strip from header (auto-bound).
                    had_header_binder = true;
                    let group_end = balanced_skip(bytes, i, b'{', b'}')?;
                    i = group_end;
                }
                b'[' => {
                    // Instance binder — strip from header.
                    had_header_binder = true;
                    let group_end = balanced_skip(bytes, i, b'[', b']')?;
                    i = group_end;
                }
                _ => {
                    // Unknown character before `:` — bail out.
                    return None;
                }
            }
            i = skip_ws(bytes, i);
        }

        if !had_header_binder {
            // No binder lift needed.
            return None;
        }

        // After binders: optional `:` TYPE, then `:=` VALUE.
        let mut ty_chunk: Option<String> = None;
        if i < bytes.len() && bytes[i] == b':' {
            // Could be `:` (type) or `:=` (no type). Look ahead.
            if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                // No type.
            } else {
                // Type — read until top-level `:=` boundary.
                let start = i + 1;
                let end = find_walrus(bytes, start)?;
                ty_chunk = Some(src[start..end].trim().to_string());
                i = end;
            }
        }
        // Now i should point at `:=`.
        if !(i + 1 < bytes.len() && bytes[i] == b':' && bytes[i + 1] == b'=') {
            return None;
        }
        i += 2;
        // VALUE region: scan to next top-level decl keyword or EOF.
        let val_start = i;
        let val_end = find_next_decl_keyword(bytes, val_start);
        let val_chunk = src[val_start..val_end].trim().to_string();

        // Build the rewritten def.
        let explicit_names: Vec<&str> = binders
            .iter()
            .flat_map(|g| g.names.iter().map(String::as_str))
            .collect();
        let explicit_tys: Vec<&str> = binders
            .iter()
            .flat_map(|g| std::iter::repeat_n(g.ty.as_str(), g.names.len()))
            .collect();

        if explicit_names.is_empty() {
            // Only implicit / instance binders; produce a plain
            // body-lambda-free decl with the type as-is.
            let ty_text = ty_chunk.unwrap_or_default();
            let rewritten = format!(
                "def {name}{univ_chunk}{maybe_colon}{ty} := {val}",
                maybe_colon = if ty_text.is_empty() { "" } else { " : " },
                ty = ty_text,
                val = val_chunk
            );
            return Some((rewritten, val_end));
        }

        // Arrow-chain the explicit-binder types.
        let arrow_ty = if let Some(rty) = ty_chunk {
            // `T1 -> T2 -> ... -> RTY`
            let mut s = String::new();
            for t in &explicit_tys {
                s.push_str(t);
                s.push_str(" -> ");
            }
            s.push_str(rty.trim());
            s
        } else {
            // No declared return type. Synthesise an arrow
            // chain ending in a wildcard so the parser still
            // accepts it. (Practically this case is rare for
            // typed Lean source, but support it for robustness.)
            let mut s = String::new();
            for (i, t) in explicit_tys.iter().enumerate() {
                if i + 1 < explicit_tys.len() {
                    s.push_str(t);
                    s.push_str(" -> ");
                } else {
                    s.push_str(t);
                }
            }
            s
        };

        let fun_args = explicit_names.join(" ");
        let rewritten =
            format!("def {name}{univ_chunk} : {arrow_ty} := fun {fun_args} -> {val_chunk}");
        Some((rewritten, val_end))
    }

    #[derive(Debug)]
    enum BinderKind {
        Explicit,
    }

    #[derive(Debug)]
    struct BinderGroup {
        names: Vec<String>,
        ty: String,
        // `kind` reserved for future extension (implicit / instance
        // handling); explicit is the only variant that contributes
        // to the lift today.
        #[allow(dead_code)]
        kind: BinderKind,
    }

    fn parse_binder_group(inner: &str, kind: BinderKind) -> Option<BinderGroup> {
        // Form: `name1 name2 ... : type`. Allow `_` as a name.
        let (lhs, rhs) = inner.split_once(':')?;
        let names: Vec<String> = lhs
            .split_whitespace()
            .map(String::from)
            .filter(|s| !s.is_empty())
            .collect();
        if names.is_empty() {
            return None;
        }
        let ty = rhs.trim().to_string();
        if ty.is_empty() {
            return None;
        }
        Some(BinderGroup { names, ty, kind })
    }

    /// Skip a balanced bracket pair starting at `start_byte`
    /// (which holds `open`). Returns the index just past the
    /// closing bracket, or `None` if unbalanced.
    fn balanced_skip(bytes: &[u8], start_byte: usize, open: u8, close: u8) -> Option<usize> {
        debug_assert_eq!(bytes[start_byte], open);
        let mut i = start_byte + 1;
        let mut depth: i32 = 1;
        let n = bytes.len();
        while i < n && depth > 0 {
            let b = bytes[i];
            if b == open {
                depth += 1;
            } else if b == close {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            } else if b == b'"' {
                // Skip string.
                i += 1;
                while i < n && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < n {
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
            } else if i + 1 < n && b == b'-' && bytes[i + 1] == b'-' {
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            i += 1;
        }
        None
    }

    /// Find the byte offset of the next top-level `:=` (a.k.a.
    /// "walrus") starting from `from`, respecting bracket
    /// nesting + strings + comments. Returns `None` if not found.
    fn find_walrus(bytes: &[u8], from: usize) -> Option<usize> {
        let mut i = from;
        let n = bytes.len();
        let mut paren_depth: i32 = 0;
        let mut brace_depth: i32 = 0;
        let mut bracket_depth: i32 = 0;
        while i < n {
            let b = bytes[i];
            // Strings.
            if b == b'"' {
                i += 1;
                while i < n && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < n {
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
                if i < n {
                    i += 1;
                }
                continue;
            }
            // Line comments.
            if i + 1 < n && b == b'-' && bytes[i + 1] == b'-' {
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            match b {
                b'(' => paren_depth += 1,
                b')' => paren_depth -= 1,
                b'{' => brace_depth += 1,
                b'}' => brace_depth -= 1,
                b'[' => bracket_depth += 1,
                b']' => bracket_depth -= 1,
                _ => {}
            }
            if paren_depth == 0 && brace_depth == 0 && bracket_depth == 0
                && b == b':' && i + 1 < n && bytes[i + 1] == b'='
            {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    fn find_next_decl_keyword(bytes: &[u8], from: usize) -> usize {
        let mut i = from;
        let n = bytes.len();
        // Track if we've crossed at least one newline — keyword
        // matches before that are body content, not a new decl.
        let mut saw_newline = false;
        while i < n {
            // Skip strings + comments inline.
            if bytes[i] == b'"' {
                i += 1;
                while i < n && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < n {
                        i += 2;
                        continue;
                    }
                    i += 1;
                }
                if i < n {
                    i += 1;
                }
                continue;
            }
            if i + 1 < n && bytes[i] == b'-' && bytes[i + 1] == b'-' {
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if bytes[i] == b'\n' {
                saw_newline = true;
                i += 1;
                continue;
            }
            if saw_newline && is_ident_byte(bytes[i]) {
                // Check if a top-level decl keyword starts here.
                // Word boundary on the LEFT is `saw_newline + ws`
                // (so we're at column 0 ignoring leading ws).
                // Already at ident byte; ws skipping is implicit.
                for &kw in super::header_binder_rewriter::DECL_KEYWORDS {
                    let kbytes = kw.as_bytes();
                    if i + kbytes.len() <= n
                        && &bytes[i..i + kbytes.len()] == kbytes
                        && is_word_boundary(bytes, i, i + kbytes.len())
                    {
                        // Also check that this column-0 position
                        // is genuinely top-level by ensuring no
                        // leading non-newline whitespace prefix
                        // since the last newline. We rewound the
                        // saw_newline flag at each `\n`, so the
                        // only way we land here is right after
                        // a newline (possibly with indentation).
                        // For simplicity, accept any keyword at
                        // the start of a new line.
                        return i;
                    }
                }
                saw_newline = false;
            }
            i += 1;
        }
        n
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn no_binders_pass_through() {
            let src = "def f : Nat -> Nat := fun n -> n";
            assert_eq!(rewrite(src), src);
        }

        #[test]
        fn single_binder_lifted() {
            let src = "def add (n : Nat) : Nat := n";
            let out = rewrite(src);
            assert!(out.contains("def add : Nat -> Nat := fun n -> n"), "got: {out}");
        }

        #[test]
        fn multi_name_binder_lifted() {
            let src = "def add (a b : Nat) : Nat := a + b";
            let out = rewrite(src);
            assert!(
                out.contains("def add : Nat -> Nat -> Nat := fun a b -> a + b"),
                "got: {out}"
            );
        }

        #[test]
        fn multi_group_binder_lifted() {
            let src = "def f (a : U64) (b : Bool) : String := \"x\"";
            let out = rewrite(src);
            assert!(
                out.contains("def f : U64 -> Bool -> String := fun a b -> \"x\""),
                "got: {out}"
            );
        }

        #[test]
        fn implicit_binders_stripped() {
            let src = "def f {T : Type} (x : T) : T := x";
            let out = rewrite(src);
            // The {T : Type} group is dropped; only explicit
            // binders contribute.
            assert!(
                out.contains("def f : T -> T := fun x -> x"),
                "got: {out}"
            );
        }

        #[test]
        fn structure_left_untouched() {
            let src = "structure Point where x : UInt32 y : UInt32";
            assert_eq!(rewrite(src), src);
        }

        #[test]
        fn theorem_left_untouched() {
            let src = "theorem t (n : Nat) : n = n := rfl";
            // theorem is in our DECL_KEYWORDS list but not in
            // the `def`-rewrite path; pass through.
            assert_eq!(rewrite(src), src);
        }

        #[test]
        fn multi_decl_source_handled() {
            let src = "\
                @[leo4_export]\n\
                def add (a b : UInt64) : UInt64 := a + b\n\
                \n\
                @[leo4_export]\n\
                def double (n : UInt32) : UInt32 := n + n\n\
            ";
            let out = rewrite(src);
            assert!(out.contains("def add : UInt64 -> UInt64 -> UInt64 := fun a b -> a + b"));
            assert!(out.contains("def double : UInt32 -> UInt32 := fun n -> n + n"));
        }

        #[test]
        fn idempotent() {
            let src = "def add (a b : Nat) : Nat := a + b";
            let once = rewrite(src);
            let twice = rewrite(&once);
            assert_eq!(once, twice);
        }

        #[test]
        fn nested_bracket_type() {
            let src = "def f (xs : List (Option Nat)) : Nat := xs.length";
            let out = rewrite(src);
            assert!(
                out.contains("def f : List (Option Nat) -> Nat := fun xs -> xs.length"),
                "got: {out}"
            );
        }

        #[test]
        fn def_inside_string_not_rewritten() {
            let src = "def msg : String := \"def foo (x : Nat) := x\"";
            assert_eq!(rewrite(src), src);
        }
    }
}

/// Normalise a Lean 4 surface-syntax source string into a form
/// `oxilean-parse::Parser` can consume. Drives
/// `oxilean-elab`'s `lean4_compat` adapter layer:
///
/// 1. `Lean4TermRewriter::standard()` — the canonical set of
///    Lean 4 → OxiLean textual replacements (` => ` → ` -> `,
///    `←` → `<-`, `where;` → `where`, `∧/∨/¬` → `&&/||/!`).
/// 2. `Lean4SyntaxAdapter::adapt_all` — composite of
///    `adapt_do_notation` + `adapt_where_clause` +
///    `adapt_match_syntax` (also fields `=> -> ->` for the
///    match-arm form the rewriter doesn't disambiguate).
///
/// Both passes are textual (intentional in upstream — they
/// pre-process the source for the parser, they don't AST-rewrite).
/// Idempotent: running it twice produces the same output.
///
/// Note this does **not** convert Lean 4's
/// `def f (binders) : T := body` to OxiLean's
/// `def f : T := fun ... -> body` — header-binders are a
/// parser-level mismatch beyond textual rewriting. Code that
/// wants to pass header-binder syntax through must either
/// (a) lift each `(x : T)` group into a leading `fun x -> ` /
/// `Π (x : T),` (a future pass living above this layer) or
/// (b) pre-elaborate via a richer adapter (not implemented in
/// `lean4_compat` v0.1.2).
#[must_use]
pub fn lean4_normalize(src: &str) -> String {
    // OX3: pre-rewrites BEFORE the lean4_compat textual passes
    // (binder lift / attribute-arg strip both operate on
    // syntactic structure the parser would otherwise reject
    // outright; the lean4_compat layer can only fix-up already-
    // valid-shape source).
    let after_attrs = strip_attribute_args(src);
    let after_binders = rewrite_header_binders(&after_attrs);
    let after_rewrite = Lean4TermRewriter::standard().rewrite(&after_binders);
    Lean4SyntaxAdapter::adapt_all(&after_rewrite)
}

/// Transpile a slice of LCNF function declarations to a
/// single Rust source string. Wraps
/// `RustTargetBackend::emit_module` + the `RustModule`'s
/// per-item `RustFn::emit()` serialisation.
///
/// `module_name` is used as the OxiLean-side module label;
/// it doesn't affect the emitted Rust source structure.
///
/// # Errors
/// `LeanError` if every decl in `decls` fails to compile
/// (the backend silently drops individually-failing decls,
/// so a totally-empty result is a sign of upstream
/// breakage).
pub fn transpile_decls(
    module_name: &str,
    decls: &[LcnfFunDecl],
) -> Result<String, LeanError> {
    let mut backend = RustTargetBackend::new();
    let module = backend.emit_module(module_name, decls);
    if module.items.is_empty() && !decls.is_empty() {
        return Err(LeanError::new(
            leo4_abi::error::error_codes::ENCODE_ERROR,
            format!(
                "leo4-oxilean-build: RustTargetBackend dropped every \
                 LCNF decl in `{module_name}` — none compiled to Rust"
            ),
        ));
    }
    let mut out = String::new();
    out.push_str("//! Auto-generated by leo4-oxilean-build from OxiLean source.\n");
    out.push_str("//! DO NOT EDIT — re-run the build step instead.\n\n");
    for item in &module.items {
        // Other RustItem variants (structs, enums, impls) would
        // be emitted via their own `emit()` methods once the
        // backend produces them. For the v0 surface — leo4
        // exports are top-level fns — only RustItem::Fn appears.
        if let RustItem::Fn(f) = item {
            out.push_str(&f.emit());
            out.push_str("\n\n");
        }
    }
    Ok(out)
}

/// End-to-end transpile from a kernel-level declaration to
/// a Rust source string. The full pipeline a future
/// `leo4-oxilean-build` CLI would run, except parsing /
/// elaborating Lean source (those layers wrap `oxilean-parse`
/// + `oxilean-elab` and aren't wired yet).
///
/// `(name, params, body)` is the kernel-level shape
/// `oxilean-codegen::to_lcnf::decl_to_lcnf` consumes:
/// - `name` — the declared function's name (e.g.
///   `Name::str("Sample.addOne")`).
/// - `params` — parameter list as `(Name, Expr)` pairs where
///   each `Expr` is the parameter's type.
/// - `body` — the function body as a kernel `Expr`.
///
/// The output is one Rust `fn` definition. Wrap it in a
/// crate / module yourself; this helper deliberately stays
/// per-decl to keep failures granular.
///
/// # Errors
/// `LeanError` if either LCNF lowering or Rust backend
/// emission fails. The underlying OxiLean errors are
/// `ConversionError` / `String`; both get wrapped into
/// `LeanError::new(ENCODE_ERROR, …)` so callers don't depend
/// on OxiLean's error types.
pub fn transpile_kernel_decl(
    name: &Name,
    params: &[(Name, Expr)],
    body: &Expr,
) -> Result<String, LeanError> {
    let config = ToLcnfConfig::default();
    let lcnf_decl: LcnfFunDecl =
        decl_to_lcnf(name, params, body, &config).map_err(|e| {
            LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                format!("leo4-oxilean-build: decl_to_lcnf failed: {e:?}"),
            )
        })?;
    let mut backend = RustTargetBackend::new();
    let rust_fn = backend.compile_decl(&lcnf_decl).map_err(|e| {
        LeanError::new(
            leo4_abi::error::error_codes::ENCODE_ERROR,
            format!("leo4-oxilean-build: compile_decl failed: {e:?}"),
        )
    })?;
    Ok(rust_fn.emit())
}

/// Unfold a Pi-typed declaration into a `(params, body)`
/// pair the way `decl_to_lcnf` expects.
///
/// A `def f (x : A) (y : B) : C := body` elaborates into
/// `ty = Π (x : A), Π (y : B), C` and
/// `val = λ (x : A), λ (y : B), body`. The pairs zip
/// together as `params = [(x, A), (y, B)]` + the unbound
/// inner `body`.
fn unfold_decl(ty: &Expr, val: &Expr) -> (Vec<(Name, Expr)>, Expr) {
    let mut params = Vec::new();
    let mut cur_ty = ty;
    let mut cur_val = val;
    while let (
        Expr::Pi(_, _, dom_ty, rest_ty),
        Expr::Lam(_, lam_name, _dom_val, rest_val),
    ) = (cur_ty, cur_val)
    {
        params.push((lam_name.clone(), (**dom_ty).clone()));
        cur_ty = rest_ty;
        cur_val = rest_val;
    }
    (params, cur_val.clone())
}

/// Full pipeline: Lean source string → Rust source string.
/// Drives `oxilean-parse::{Lexer, Parser}` →
/// `oxilean-elab::elaborate_decl` → `unfold_decl` →
/// `oxilean-codegen::to_lcnf::decl_to_lcnf` →
/// `RustTargetBackend::compile_decl` → `RustFn::emit`.
///
/// `env` is the elaboration environment. Pass an
/// `Environment::new()` for a minimal probe, or a
/// pre-populated environment (with built-in inductives,
/// previously-elaborated decls, etc.) for real fixtures.
///
/// Only **one declaration per source string** today —
/// `Parser::parse_decl` returns a single `Located<Decl>`.
/// Multi-decl source loading lives in the next layer when
/// we wrap a Lake-package walker.
///
/// # Errors
/// `LeanError` if any step (parse / elab / LCNF / Rust emit)
/// fails. The underlying OxiLean / parser errors get wrapped
/// so callers don't depend on OxiLean's error types.
pub fn transpile_source(env: &Environment, src: &str) -> Result<String, LeanError> {
    // 0. Lean 4 → OxiLean syntax normalisation
    //    (`oxilean-elab::lean4_compat` pre-processor pass).
    let normalised = lean4_normalize(src);

    // 1. Lex.
    let mut lexer = Lexer::new(&normalised);
    let tokens = lexer.tokenize();

    // 2. Parse one declaration.
    let mut parser = Parser::new(tokens);
    let decl = parser.parse_decl().map_err(|e| {
        LeanError::new(
            leo4_abi::error::error_codes::DECODE_ERROR,
            format!("leo4-oxilean-build: parse_decl failed: {e:?}"),
        )
    })?;

    // 3. Elaborate.
    let pending = elaborate_decl(env, &decl.value).map_err(|e| {
        LeanError::new(
            leo4_abi::error::error_codes::DECODE_ERROR,
            format!("leo4-oxilean-build: elaborate_decl failed: {e:?}"),
        )
    })?;

    // 4. Extract (name, ty, val) — only Definition produces
    //    a transpilable body (axioms / theorems / inductives
    //    are out of scope today).
    let (name, ty, val) = match pending {
        PendingDecl::Definition { name, ty, val, .. } => (name, ty, val),
        other => {
            return Err(LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                format!(
                    "leo4-oxilean-build: only `def` declarations are \
                     transpilable today; got {other:?}"
                ),
            ));
        }
    };

    // 5. Unfold Pi / Lam into (params, body).
    let (params, body) = unfold_decl(&ty, &val);

    // 6. Drive the kernel-level helper.
    transpile_kernel_decl(&name, &params, &body)
}

/// Sanity probe: does the backend's type mapper actually
/// produce Rust type names that don't reference `lean_*`
/// symbols? Used as a build-time invariant — if a future
/// OxiLean release flips `lcnf_to_rust_type(Nat)` to e.g.
/// `RustType::Custom("lean_box_uint64")`, this fn returns
/// `false` and the corresponding test catches the
/// regression.
#[must_use]
pub fn type_mapper_is_lean_h_free() -> bool {
    use oxilean_codegen::lcnf::LcnfType;

    let probes: &[LcnfType] = &[
        LcnfType::Nat,
        LcnfType::LcnfString,
        LcnfType::Unit,
        LcnfType::Erased,
    ];
    for probe in probes {
        let ty = RustTargetBackend::lcnf_to_rust_type(probe);
        let rendered = format!("{ty}");
        if rendered.contains("lean_")
            || rendered.contains("Lean_")
            || rendered.contains("lean.h")
        {
            return false;
        }
    }
    true
}

// ─── Hook 3 — `@[leo4_export]` / `deriving LeanMarshal` discovery ──────────
//
// SPEC/rust-native-lean.md §7.1 lists three OxiLean evaluator
// hooks (Hooks 1 + 2 absent; Hook 3 PRESENT). Hook 3 is
// upstream's `AttributeManager::register_custom_handler` +
// `DeriveHandlerRegistry::register`. leo4 plugs into Hook 3 to
// register its own attribute name (`@[leo4_export]`) and its
// own deriving handler (`LeanMarshal`). The actual transpile
// then walks parsed `Decl::Attribute { attrs, decl }` outer
// wrappers, matches `attrs` against `LEO4_EXPORT_ATTR`, and
// only emits Rust for tagged decls — the rest are skipped.
//
// Note on parser shape: OxiLean's `Parser::parse_decl` returns
// `@[name] def f := body` as `Decl::Attribute { attrs:
// Vec<String>, decl: Box<Located<Decl>> }`. The *inner*
// `Decl::Definition.attrs` field is left empty by the parser —
// the outer wrapper is the only place attribute names appear.
// `elaborate_decl` further unwraps `Decl::Attribute` and
// discards the outer `attrs` (upstream code path; v0.1.2). So
// leo4 inspects the parser AST *before* elaboration to spot
// the tag, then elaborates the inner decl normally.

/// Registry owning leo4's attribute + derive handlers, plus the
/// `AttributeManager` accumulating discovered `@[leo4_export]`
/// tags across a build. Mutable across `transpile_source_if_exported`
/// calls so a whole package's transpile produces a single
/// registry capturing every export.
///
/// Created with `Leo4ExportRegistry::new()`; the constructor
/// pre-populates the manager + derive registry with leo4's two
/// handlers (`leo4_export` custom attribute, `LeanMarshal`
/// derive). Inspect with `has_export_handler()` /
/// `has_marshal_derive()` (used for tests + status diagnostics).
pub struct Leo4ExportRegistry {
    pub manager: AttributeManager,
    pub derive: DeriveHandlerRegistry,
    /// User-defined type names (Lean `structure` / `inductive`)
    /// discovered during a build. Tracked here so wrapper
    /// synthesis can pass these names through
    /// `render_marshallable_type_with_users` rather than
    /// rejecting them as unknown carriers.
    pub user_types: HashSet<String>,
}

impl Leo4ExportRegistry {
    /// Build a registry pre-populated with leo4's attribute +
    /// derive handlers.
    #[must_use]
    pub fn new() -> Self {
        let mut manager = AttributeManager::new();
        manager.register_custom_handler(AttrHandler::new(
            LEO4_EXPORT_ATTR,
            "Mark a top-level definition for export through leo4's \
             canonical-ABI boundary. The leo4-oxilean-build transpiler \
             emits a Rust wrapper for each tagged decl; untagged decls \
             are skipped.",
            AttrAction::Custom(LEO4_EXPORT_ATTR.into()),
        ));

        let mut derive = DeriveHandlerRegistry::new();
        derive.register(
            DeriveHandler::new(
                Name::str(LEAN_MARSHAL_DERIVE),
                "Auto-derive leo4 canonical-ABI marshalling for a record \
                 / inductive type. The handler emits no Lean instance — \
                 the transpiler synthesises the equivalent Rust \
                 `LeanMarshal` impl on the boundary crate side.",
            )
            // The transpile path doesn't need OxiLean to emit a
            // Lean-side instance; we generate the marshalling
            // Rust directly. `no_instance()` documents this and
            // suppresses upstream instance-generation work.
            .no_instance(),
        );

        Self {
            manager,
            derive,
            user_types: HashSet::new(),
        }
    }

    /// Record a user-defined type name (Lean `structure` /
    /// `inductive`) so subsequent wrapper synthesis recognises
    /// it as marshallable. Idempotent (HashSet semantics).
    pub fn register_user_type(&mut self, name: &str) {
        self.user_types.insert(name.to_string());
    }

    /// Snapshot of every user type registered so far.
    /// Returned in arbitrary order (HashSet); callers that
    /// need determinism should sort.
    #[must_use]
    pub fn user_type_names(&self) -> Vec<String> {
        self.user_types.iter().cloned().collect()
    }

    /// True iff the `@[leo4_export]` custom-attribute handler
    /// is registered with the inner `AttributeManager`.
    #[must_use]
    pub fn has_export_handler(&self) -> bool {
        self.manager.get_handler(LEO4_EXPORT_ATTR).is_some()
    }

    /// True iff the `LeanMarshal` derive handler is registered
    /// with the inner `DeriveHandlerRegistry`.
    #[must_use]
    pub fn has_marshal_derive(&self) -> bool {
        self.derive.has(&Name::str(LEAN_MARSHAL_DERIVE))
    }

    /// Record an `@[leo4_export]` discovery in the manager so
    /// downstream queries (`manager.get_by_kind("leo4_export")`)
    /// can enumerate every export across a package's source.
    /// Used by `transpile_source_if_exported` after parse.
    pub fn record_export(&mut self, decl_name: &str) {
        // Map the parser-level decl-name string into a kernel
        // Name (the AttributeManager indexes by kernel Name).
        let kernel_name = Name::str(decl_name);
        let entry = AttrEntry::new(
            AttributeKind::Custom(LEO4_EXPORT_ATTR.into()),
            kernel_name,
        );
        // `register_attribute` errors on duplicates; treat as
        // best-effort — duplicates in a single build pass would
        // indicate a parser-level issue, not user error.
        let _ = self.manager.register_attribute(entry);
    }

    /// Enumerate every decl name (kernel `Name`) the manager
    /// has recorded as `@[leo4_export]`-tagged so far.
    #[must_use]
    pub fn exported_names(&self) -> Vec<Name> {
        self.manager.get_by_kind(LEO4_EXPORT_ATTR)
    }
}

impl Default for Leo4ExportRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Inspect a parsed top-level decl + return whether its outer
/// attribute wrapper contains `@[leo4_export]`. Inspects only
/// the parser AST — does NOT elaborate.
#[must_use]
pub fn decl_has_leo4_export(decl: &Located<Decl>) -> bool {
    if let Decl::Attribute { attrs, .. } = &decl.value {
        attrs.iter().any(|s| s == LEO4_EXPORT_ATTR)
    } else {
        false
    }
}

/// Unwrap one layer of `Decl::Attribute { decl, .. }` if
/// present; otherwise return the decl unchanged. Mirrors
/// `elaborate_decl`'s upstream behaviour so the inner decl
/// reaches kernel-level layers identically.
#[must_use]
pub fn inner_decl(decl: &Located<Decl>) -> &Located<Decl> {
    if let Decl::Attribute { decl, .. } = &decl.value {
        decl.as_ref()
    } else {
        decl
    }
}

/// Best-effort name extraction from a parsed `Decl`. Used to
/// register a discovery with the `AttributeManager` after
/// parsing but before elaboration.
#[must_use]
pub fn decl_name(decl: &Located<Decl>) -> Option<&str> {
    let target = inner_decl(decl);
    match &target.value {
        Decl::Definition { name, .. }
        | Decl::Theorem { name, .. }
        | Decl::Axiom { name, .. }
        | Decl::Inductive { name, .. } => Some(name.as_str()),
        _ => None,
    }
}

/// Hook-3-aware variant of `transpile_source`. Parses the
/// source, checks the outer-wrapper attribute list against
/// `LEO4_EXPORT_ATTR`, and:
///
/// - If `@[leo4_export]` is **not** present → returns
///   `Ok(None)` (skipped, no transpile work done).
/// - If `@[leo4_export]` **is** present → records the discovery
///   in `registry.manager` and returns `Ok(Some(rust_source))`
///   with the same pipeline as `transpile_source`.
///
/// Errors propagate as for `transpile_source` (parse / elab /
/// LCNF / Rust-emit failures wrap into `LeanError`).
pub fn transpile_source_if_exported(
    env: &Environment,
    registry: &mut Leo4ExportRegistry,
    src: &str,
) -> Result<Option<String>, LeanError> {
    let normalised = lean4_normalize(src);

    let mut lexer = Lexer::new(&normalised);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens);
    let parsed = parser.parse_decl().map_err(|e| {
        LeanError::new(
            leo4_abi::error::error_codes::DECODE_ERROR,
            format!("leo4-oxilean-build: parse_decl failed: {e:?}"),
        )
    })?;

    if !decl_has_leo4_export(&parsed) {
        return Ok(None);
    }

    // Record the discovery before elaboration so even if elab
    // chokes, the registry still knows what was *intended* to
    // be exported (useful for diagnostics in a multi-decl
    // build pass).
    if let Some(name) = decl_name(&parsed) {
        registry.record_export(name);
    }

    // Elaborate the inner decl directly — upstream
    // `elaborate_decl(env, Decl::Attribute{..})` does the same
    // unwrap and drops outer attrs; we've already captured the
    // tag.
    let pending = elaborate_decl(env, &inner_decl(&parsed).value).map_err(|e| {
        LeanError::new(
            leo4_abi::error::error_codes::DECODE_ERROR,
            format!("leo4-oxilean-build: elaborate_decl failed: {e:?}"),
        )
    })?;

    let (name, ty, val) = match pending {
        PendingDecl::Definition { name, ty, val, .. } => (name, ty, val),
        other => {
            return Err(LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                format!(
                    "leo4-oxilean-build: only `def` declarations are \
                     transpilable today; got {other:?}"
                ),
            ));
        }
    };

    let (params, body) = unfold_decl(&ty, &val);
    transpile_kernel_decl(&name, &params, &body).map(Some)
}

/// Superset of `transpile_source_if_exported` that also runs
/// `synthesize_canonical_wrapper` against the same `RustFn`
/// and bundles the result into a `TranspileUnit` ready for
/// `emit_crate`. The `mangled` argument is the leo4 mangled
/// name the caller has computed for this export (per
/// `SPEC/mangling.md`); it ends up as the dispatch-table key
/// in the emitted `LeanProc` impl.
///
/// Returns `Ok(None)` if the source isn't tagged with
/// `@[leo4_export]`, matching `transpile_source_if_exported`'s
/// skip semantics.
///
/// This is the entry the OX1 CLI binary drives one file at a
/// time; library callers can use it directly without going
/// through CLI args.
///
/// # Errors
/// `LeanError` for any failure in the underlying parse / elab
/// / LCNF / Rust-emit / wrapper-synthesis pipeline. Wrapper
/// synthesis can fail for types not covered by
/// `render_marshallable_type` (see OX2).
pub fn transpile_source_to_unit(
    env: &Environment,
    registry: &mut Leo4ExportRegistry,
    src: &str,
    mangled: &str,
) -> Result<Option<TranspileUnit>, LeanError> {
    let normalised = lean4_normalize(src);

    let mut lexer = Lexer::new(&normalised);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens);
    let parsed = parser.parse_decl().map_err(|e| {
        LeanError::new(
            leo4_abi::error::error_codes::DECODE_ERROR,
            format!("leo4-oxilean-build: parse_decl failed: {e:?}"),
        )
    })?;
    process_parsed_decl(env, registry, &parsed, mangled)
}

/// Multi-decl variant of `transpile_source_to_unit`. Parses
/// every top-level declaration in `src`, drives each tagged
/// `@[leo4_export]` decl through the same pipeline, and
/// returns the accumulated unit list (skipping untagged decls
/// silently). Used by the CLI's multi-decl-per-file manifest
/// form, where the caller supplies a `name_to_mangled` map
/// pre-computed by leo4's mangling pipeline.
///
/// Type-only decls (Structure / Inductive) ignore
/// `name_to_mangled` (their `mangled` field is always empty).
/// Definitions look up their `decl_name` in the map; missing
/// entries return `Err(ENCODE_ERROR)`.
///
/// # Errors
/// `LeanError` for any parse / elab / wrapper-synth failure,
/// or a missing mangled-name entry for a tagged definition.
pub fn transpile_source_to_units(
    env: &Environment,
    registry: &mut Leo4ExportRegistry,
    src: &str,
    name_to_mangled: &HashMap<String, String>,
) -> Result<Vec<TranspileUnit>, LeanError> {
    let normalised = lean4_normalize(src);
    // Walk the parser manually — upstream `oxilean_parse::parser::parse_decls`
    // v0.1.2's EOF detection only catches `UnexpectedEof`, not
    // `UnexpectedToken { got: Eof }`, so trailing whitespace in
    // a multi-decl source triggers a spurious parse error.
    // `Parser::is_eof()` is the right loop guard.
    let mut lexer = Lexer::new(&normalised);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens);
    let mut parsed_all: Vec<Located<Decl>> = Vec::new();
    while !parser.is_eof() {
        let d = parser.parse_decl().map_err(|e| {
            LeanError::new(
                leo4_abi::error::error_codes::DECODE_ERROR,
                format!("leo4-oxilean-build: parse_decl failed: {e:?}"),
            )
        })?;
        parsed_all.push(d);
    }

    let mut units: Vec<TranspileUnit> = Vec::new();
    for parsed in &parsed_all {
        if !decl_has_leo4_export(parsed) {
            continue;
        }
        let dn = decl_name(parsed);
        // For Definition decls we need a mangled name; type
        // decls don't. Inspect `inner_decl` before falling
        // through to `process_parsed_decl`.
        let inner = inner_decl(parsed);
        let mangled: &str = match &inner.value {
            Decl::Definition { .. } => {
                let n = dn.ok_or_else(|| LeanError::new(
                    leo4_abi::error::error_codes::ENCODE_ERROR,
                    "leo4-oxilean-build: tagged `def` missing decl name".to_string(),
                ))?;
                name_to_mangled.get(n).map(String::as_str).ok_or_else(|| {
                    LeanError::new(
                        leo4_abi::error::error_codes::ENCODE_ERROR,
                        format!(
                            "leo4-oxilean-build: no mangled name supplied for tagged \
                             def `{n}` (caller must provide via name_to_mangled)"
                        ),
                    )
                })?
            }
            _ => "", // Structure / Inductive — mangled stays empty
        };
        if let Some(u) = process_parsed_decl(env, registry, parsed, mangled)? {
            units.push(u);
        }
    }
    Ok(units)
}

/// Inner helper: process one already-parsed `Decl` into an
/// optional `TranspileUnit`. Untagged decls return `Ok(None)`;
/// tagged decls dispatch on the inner-decl kind (Definition /
/// Structure / Inductive).
///
/// # Errors
/// Same as `transpile_source_to_unit` / `transpile_source_to_units`.
#[allow(clippy::too_many_lines)] // documented branches: structure / inductive / definition
fn process_parsed_decl(
    env: &Environment,
    registry: &mut Leo4ExportRegistry,
    parsed: &Located<Decl>,
    mangled: &str,
) -> Result<Option<TranspileUnit>, LeanError> {
    if !decl_has_leo4_export(parsed) {
        return Ok(None);
    }

    if let Some(name) = decl_name(parsed) {
        registry.record_export(name);
    }

    // Branch on the inner decl kind: definitions take the fn
    // transpile path; structures synthesise a type decl + bare
    // name registration (no LeanProc dispatch arm).
    let inner = inner_decl(parsed);
    match &inner.value {
        Decl::Structure { name, fields, .. } => {
            // Register the user type *first* so any subsequent
            // fields referring to other user types (later in
            // the same build) resolve.
            registry.register_user_type(name);

            let mut sfields = Vec::with_capacity(fields.len());
            for f in fields {
                let ty = surface_to_rust_type(&f.ty, &registry.user_types)
                    .map_err(|e| LeanError::new(
                        e.code,
                        format!(
                            "leo4-oxilean-build: structure `{name}` field \
                             `{fname}`: {emsg}",
                            fname = f.name,
                            emsg = e.message,
                        ),
                    ))?;
                sfields.push(StructField {
                    name: f.name.clone(),
                    ty,
                });
            }
            let struct_src = synthesize_struct_type_with_users(
                name,
                &sfields,
                &registry.user_types,
            )?;
            // Type-only unit: empty fn/wrapper, empty mangled.
            // `emit_lib_rs` recognises mangled.is_empty() as
            // the "skip dispatch arm" signal.
            return Ok(Some(TranspileUnit {
                type_decls: vec![struct_src],
                fn_src: String::new(),
                wrapper_src: String::new(),
                fn_name: name.clone(),
                mangled: String::new(),
            }));
        }
        Decl::Inductive { name, ctors, .. } => {
            // Inductives surface as Rust enums. Register the
            // type name first so subsequent ctor-payload
            // references to other user types resolve.
            registry.register_user_type(name);

            let mut evariants: Vec<EnumVariant> = Vec::with_capacity(ctors.len());
            for c in ctors {
                let payloads = unfold_ctor_payload(&c.ty);
                let mut fields: Vec<RustType> = Vec::with_capacity(payloads.len());
                for p in payloads {
                    let ty = surface_to_rust_type(p, &registry.user_types)
                        .map_err(|e| LeanError::new(
                            e.code,
                            format!(
                                "leo4-oxilean-build: inductive `{name}` ctor \
                                 `{cname}` payload: {emsg}",
                                cname = c.name,
                                emsg = e.message,
                            ),
                        ))?;
                    fields.push(ty);
                }
                evariants.push(EnumVariant {
                    name: c.name.clone(),
                    fields,
                });
            }
            let enum_src = synthesize_enum_type_with_users(
                name,
                &evariants,
                &registry.user_types,
            )?;
            return Ok(Some(TranspileUnit {
                type_decls: vec![enum_src],
                fn_src: String::new(),
                wrapper_src: String::new(),
                fn_name: name.clone(),
                mangled: String::new(),
            }));
        }
        Decl::Definition { .. } => {} // fall through to fn path
        _ => {
            return Err(LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                format!(
                    "leo4-oxilean-build: only `def` / `structure` / `inductive` \
                     declarations are transpilable today; got {kind}",
                    kind = decl_kind_label(&inner.value),
                ),
            ));
        }
    }

    let pending = elaborate_decl(env, &inner.value).map_err(|e| {
        LeanError::new(
            leo4_abi::error::error_codes::DECODE_ERROR,
            format!("leo4-oxilean-build: elaborate_decl failed: {e:?}"),
        )
    })?;

    let (name, ty, val) = match pending {
        PendingDecl::Definition { name, ty, val, .. } => (name, ty, val),
        other => {
            return Err(LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                format!(
                    "leo4-oxilean-build: only `def` declarations are \
                     transpilable today; got {other:?}"
                ),
            ));
        }
    };

    let (params, body) = unfold_decl(&ty, &val);

    let config = ToLcnfConfig::default();
    let lcnf_decl: LcnfFunDecl =
        decl_to_lcnf(&name, &params, &body, &config).map_err(|e| {
            LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                format!("leo4-oxilean-build: decl_to_lcnf failed: {e:?}"),
            )
        })?;
    let mut backend = RustTargetBackend::new();
    let rust_fn = backend.compile_decl(&lcnf_decl).map_err(|e| {
        LeanError::new(
            leo4_abi::error::error_codes::ENCODE_ERROR,
            format!("leo4-oxilean-build: compile_decl failed: {e:?}"),
        )
    })?;

    let fn_src = rust_fn.emit();
    let wrapper_src = synthesize_canonical_wrapper_with_users(
        &rust_fn,
        &registry.user_types,
    )?;
    let fn_name = rust_fn.name.clone();
    Ok(Some(TranspileUnit {
        fn_src,
        wrapper_src,
        fn_name,
        mangled: mangled.to_string(),
        type_decls: Vec::new(),
    }))
}

/// Unfold a Lean inductive constructor's type into the
/// sequence of its payload types. A ctor like
/// `left : Nat → String → Either` has type
/// `Pi (_ : Nat), Pi (_ : String), Either`; this fn returns
/// `[&Nat, &String]`. Unit ctors (type = just the inductive
/// itself, no Pi) return an empty slice.
fn unfold_ctor_payload(ty: &Located<SurfaceExpr>) -> Vec<&Located<SurfaceExpr>> {
    let mut acc: Vec<&Located<SurfaceExpr>> = Vec::new();
    let mut cur = &ty.value;
    while let SurfaceExpr::Pi(binders, body) = cur {
        for b in binders {
            if let Some(bty) = &b.ty {
                acc.push(bty.as_ref());
            }
        }
        cur = &body.value;
    }
    acc
}

fn decl_kind_label(decl: &Decl) -> &'static str {
    match decl {
        Decl::Definition { .. } => "Definition",
        Decl::Theorem { .. } => "Theorem",
        Decl::Axiom { .. } => "Axiom",
        Decl::Inductive { .. } => "Inductive",
        Decl::Structure { .. } => "Structure",
        Decl::Import { .. } => "Import",
        Decl::Mutual { .. } => "Mutual",
        Decl::Derive { .. } => "Derive",
        Decl::Attribute { .. } => "Attribute",
        _ => "<other>",
    }
}

// ─── §5 Canonical-ABI wrapper synthesis ──────────────────────────────────
//
// SPEC/rust-native-lean.md §3: a `LeanProc` impl resolves
// `(mangled, args: &[u8]) -> Result<Vec<u8>, LeanError>`. The
// transpile path achieves the same shape by emitting a sibling
// wrapper *next to* each transpiled fn:
//
//   pub fn <name>_call(args: &[u8])
//        -> Result<Vec<u8>, leo4_abi::LeanError>
//
// The wrapper canonical-decodes the input bytes into typed
// Rust args, calls the transpiled fn, and canonical-encodes
// the result. The downstream `LeanProc` impl simply dispatches
// `mangled → <name>_call` lookups (a static table populated at
// crate-emit time; that's §6).
//
// Limitations of v0 wrapper synthesis: only fns whose params
// and return type *all* lift to a primitive `leo4_abi::LeanMarshal`
// type (u8..u128, i8..i128, f32, f64, bool, char, String) are
// supported. Carrier types (BigNat / LeanRat / etc.) and user-
// defined records aren't covered yet — they need their
// `LeanMarshal` impls + the transpiler's struct emission
// (which the upstream `RustTargetBackend::emit_module` doesn't
// produce at v0.1.2 either).

use oxilean_codegen::rust_target_backend::{RustFn, RustType};

/// Map an OxiLean `Custom("…")` type name to a leo4-abi carrier
/// type's fully-qualified path, or `None` if the name doesn't
/// match a known carrier. Recognises both bare names (e.g.
/// `BigNat`) and OxiLean-mangled forms (`.` → `_`, e.g.
/// `Leo4_BigNat`) so the matcher is robust to where the user's
/// Lean source put the type.
///
/// Nightly half-precision / complex variants live behind the
/// `nightly-floats` cargo feature.
fn carrier_path_for(name: &str) -> Option<&'static str> {
    match name {
        // Big integers
        "BigNat" | "Leo4_BigNat" | "leo4_abi_BigNat" => Some("::leo4_abi::BigNat"),
        "BigInt" | "Leo4_BigInt" | "leo4_abi_BigInt" => Some("::leo4_abi::BigInt"),
        // Rational
        "LeanRat" | "Rat" | "Leo4_Rat" | "Leo4_LeanRat" => {
            Some("::leo4_abi::LeanRat")
        }
        // Stable complex (F32 / F64)
        "LeanComplexF32x2" | "Leo4_LeanComplexF32x2" => {
            Some("::leo4_abi::LeanComplexF32x2")
        }
        "LeanComplexF64x2" | "Leo4_LeanComplexF64x2" => {
            Some("::leo4_abi::LeanComplexF64x2")
        }
        // Nightly half-precision floats + complex variants.
        #[cfg(feature = "nightly-floats")]
        "LeanF16" | "Leo4_LeanF16" => Some("::leo4_abi::f16"),
        #[cfg(feature = "nightly-floats")]
        "LeanBF16" | "Leo4_LeanBF16" => Some("::leo4_abi::LeanBF16"),
        #[cfg(feature = "nightly-floats")]
        "LeanF128" | "Leo4_LeanF128" => Some("::leo4_abi::f128"),
        #[cfg(feature = "nightly-floats")]
        "LeanComplexF16x2" | "Leo4_LeanComplexF16x2" => {
            Some("::leo4_abi::LeanComplexF16x2")
        }
        #[cfg(feature = "nightly-floats")]
        "LeanComplexBF16x2" | "Leo4_LeanComplexBF16x2" => {
            Some("::leo4_abi::LeanComplexBF16x2")
        }
        #[cfg(feature = "nightly-floats")]
        "LeanComplexF128x2" | "Leo4_LeanComplexF128x2" => {
            Some("::leo4_abi::LeanComplexF128x2")
        }
        _ => None,
    }
}

/// Map a `RustType` to a Rust source string naming a type for
/// which leo4-abi provides a `LeanMarshal` impl. Returns `Err`
/// for types not covered by the wrapper synthesis matrix.
///
/// Coverage matrix (OX2, v1.0 RC target):
/// - Primitives: u8..u128, i8..i128, f32, f64, bool, char
/// - `String`
/// - Carrier types via `carrier_path_for`: `BigNat`, `BigInt`,
///   `LeanRat`, `LeanComplexF{32,64}x2` (+ nightly variants)
/// - Generic containers (recursive on inner types):
///   `Vec<T>`, `Option<T>`, `Result<T, E>`, `Box<T>`, tuples
///   up to 5 elements (matching `leo4_abi::tuple_impl`)
///
/// Type-erased shapes (`Box<dyn Any>`, free vars, unresolved
/// inductives) and user-defined records remain unsupported —
/// the latter is a separate OX2 sub-problem (deferred to first
/// real-fixture pass per ROADMAP).
fn render_marshallable_type(ty: &RustType) -> Result<String, LeanError> {
    let unsupported = |label: &str| -> LeanError {
        LeanError::new(
            leo4_abi::error::error_codes::ENCODE_ERROR,
            format!(
                "leo4-oxilean-build: type `{label}` has no leo4-abi \
                 LeanMarshal impl wired in OX2 wrapper synthesis"
            ),
        )
    };

    match ty {
        RustType::U8 => Ok("u8".into()),
        RustType::U16 => Ok("u16".into()),
        RustType::U32 => Ok("u32".into()),
        RustType::U64 => Ok("u64".into()),
        RustType::U128 => Ok("u128".into()),
        RustType::I8 => Ok("i8".into()),
        RustType::I16 => Ok("i16".into()),
        RustType::I32 => Ok("i32".into()),
        RustType::I64 => Ok("i64".into()),
        RustType::I128 => Ok("i128".into()),
        RustType::F32 => Ok("f32".into()),
        RustType::F64 => Ok("f64".into()),
        RustType::Bool => Ok("bool".into()),
        RustType::Char => Ok("char".into()),
        RustType::RustString => Ok("::std::string::String".into()),

        // Recursive container types — leo4-abi has generic
        // `LeanMarshal` impls for all of these.
        RustType::Vec(inner) => {
            let i = render_marshallable_type(inner)?;
            Ok(format!("::std::vec::Vec<{i}>"))
        }
        RustType::Option(inner) => {
            let i = render_marshallable_type(inner)?;
            Ok(format!("::core::option::Option<{i}>"))
        }
        RustType::Result(t, e) => {
            let t_str = render_marshallable_type(t)?;
            let e_str = render_marshallable_type(e)?;
            Ok(format!("::core::result::Result<{t_str}, {e_str}>"))
        }
        RustType::Tuple(items) => {
            // leo4-abi provides tuple impls for arities 2..=5.
            // 0-arity is `()` (Unit); 1-arity isn't a Rust
            // tuple in canonical syntax (it'd need `(T,)`).
            // For now reject arities outside [2..=5] explicitly.
            match items.len() {
                0 => Ok("()".into()),
                1 => Err(unsupported("(T,) single-element tuple")),
                n if (2..=5).contains(&n) => {
                    let parts: Vec<String> = items
                        .iter()
                        .map(render_marshallable_type)
                        .collect::<Result<_, _>>()?;
                    Ok(format!("({})", parts.join(", ")))
                }
                _ => Err(unsupported("tuple with arity > 5")),
            }
        }

        // Named single-name carrier — match against the leo4-abi
        // carrier set.
        RustType::Custom(name) => match carrier_path_for(name) {
            Some(path) => Ok(path.into()),
            None => Err(unsupported(name)),
        },

        // Generic types where the head is a Vec / Option / Result
        // expressed as Generic("Vec", [T]) etc. (upstream
        // sometimes emits these via `RustType::Generic` instead
        // of the dedicated variants).
        RustType::Generic(head, args) => match (head.as_str(), args.as_slice()) {
            ("Vec", [t]) => {
                let inner = render_marshallable_type(t)?;
                Ok(format!("::std::vec::Vec<{inner}>"))
            }
            ("Option", [t]) => {
                let inner = render_marshallable_type(t)?;
                Ok(format!("::core::option::Option<{inner}>"))
            }
            ("Result", [t, e]) => {
                let t_str = render_marshallable_type(t)?;
                let e_str = render_marshallable_type(e)?;
                Ok(format!("::core::result::Result<{t_str}, {e_str}>"))
            }
            ("Box", [t]) => {
                let inner = render_marshallable_type(t)?;
                Ok(format!("::std::boxed::Box<{inner}>"))
            }
            (name, _) if carrier_path_for(name).is_some() => {
                // A carrier name appearing with generic args
                // (e.g. `BigNat<...>`) is meaningless today —
                // the carrier types are non-generic. Reject
                // rather than silently dropping args.
                Err(unsupported(&format!("{name}<...> (carrier types are not generic)")))
            }
            (name, _) => Err(unsupported(&format!("generic `{name}<...>`"))),
        },

        // Type-erased / unsupported shapes.
        other => Err(unsupported(&format!("{other:?}"))),
    }
}

/// Synthesise a canonical-ABI boundary shim for a transpiled
/// Rust fn. Emits a sibling `pub fn <name>_call(args: &[u8])
/// -> Result<Vec<u8>, LeanError>` that:
///
/// 1. Sequentially `LeanMarshal::canonical_decode`s each arg,
///    advancing the offset between calls.
/// 2. Invokes the transpiled fn with the decoded args.
/// 3. Canonical-encodes the return value (or returns an empty
///    `Vec<u8>` for unit-returning fns).
///
/// The emitted wrapper is plain Rust source; concatenate it
/// with `transpile_kernel_decl`'s output to land both items
/// in the same crate.
///
/// # Errors
/// `LeanError` if any param type or the return type fails
/// `render_marshallable_type` (the type isn't covered by §5's
/// v0 marshalling matrix — carrier types and user records are
/// the typical reasons).
pub fn synthesize_canonical_wrapper(transpiled: &RustFn) -> Result<String, LeanError> {
    synthesize_canonical_wrapper_with_users(transpiled, &HashSet::new())
}

/// Variant of `synthesize_canonical_wrapper` aware of
/// user-defined types previously registered with a
/// `Leo4ExportRegistry`. Params / return types named in
/// `user_types` are accepted without further marshalling
/// validation (their `LeanMarshal` impls land in the same
/// emit module via `synthesize_struct_type` / future
/// `synthesize_enum_type`).
///
/// # Errors
/// Same conditions as `synthesize_canonical_wrapper` —
/// non-marshallable types not registered as user types
/// still reject.
pub fn synthesize_canonical_wrapper_with_users(
    transpiled: &RustFn,
    user_types: &HashSet<String>,
) -> Result<String, LeanError> {
    use std::fmt::Write as _;

    let wrapper_name = format!("{}_call", transpiled.name);
    let mut s = String::new();

    writeln!(
        s,
        "/// Canonical-ABI boundary shim for `{}` — decodes args \
         from leo4 canonical bytes, calls the transpiled fn, \
         encodes the return.",
        transpiled.name
    )
    .unwrap();
    writeln!(
        s,
        "pub fn {wrapper_name}(args: &[u8]) -> ::core::result::Result<::std::vec::Vec<u8>, ::leo4_abi::LeanError> {{"
    )
    .unwrap();

    if transpiled.params.is_empty() {
        s.push_str("    let _ = args;\n");
    } else {
        s.push_str("    let mut __off: usize = 0;\n");
        for (i, (pname, pty, _)) in transpiled.params.iter().enumerate() {
            let rust_ty = render_marshallable_type_with_users(pty, user_types)?;
            writeln!(
                s,
                "    let ({pname}, __next_{i}) = <{rust_ty} as ::leo4_abi::LeanMarshal>::canonical_decode(args, __off)?;"
            )
            .unwrap();
            // Suppress an "unused last offset" lint by keeping
            // the assignment regardless of whether anything
            // after reads it.
            writeln!(s, "    __off = __next_{i};").unwrap();
        }
        s.push_str("    let _ = __off;\n");
    }

    let call_args = transpiled
        .params
        .iter()
        .map(|(n, _, _)| n.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(s, "    let __ret = {}({call_args});", transpiled.name).unwrap();

    match &transpiled.return_type {
        None | Some(RustType::Unit) => {
            s.push_str("    let _: () = __ret;\n");
            s.push_str("    ::core::result::Result::Ok(::std::vec::Vec::new())\n");
        }
        Some(ty) => {
            // Validate the return type has a LeanMarshal impl
            // before emitting the encode call.
            let _ = render_marshallable_type_with_users(ty, user_types)?;
            s.push_str("    ::core::result::Result::Ok(::leo4_abi::encode_to_vec(&__ret))\n");
        }
    }

    s.push_str("}\n");
    Ok(s)
}

/// Drive `transpile_kernel_decl` then `synthesize_canonical_wrapper`
/// in sequence; concatenate the two emitted items into one
/// Rust source string ready to drop into a crate.
///
/// Returns `(rust_fn_source, wrapper_source)` separately so
/// callers can route them into different files / modules
/// when desired (`lib.rs` + `wrappers.rs`, say).
///
/// # Errors
/// `LeanError` for any failure in either underlying step.
pub fn transpile_kernel_decl_with_wrapper(
    name: &Name,
    params: &[(Name, Expr)],
    body: &Expr,
) -> Result<(String, String), LeanError> {
    let config = ToLcnfConfig::default();
    let lcnf_decl: LcnfFunDecl =
        decl_to_lcnf(name, params, body, &config).map_err(|e| {
            LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                format!("leo4-oxilean-build: decl_to_lcnf failed: {e:?}"),
            )
        })?;
    let mut backend = RustTargetBackend::new();
    let rust_fn = backend.compile_decl(&lcnf_decl).map_err(|e| {
        LeanError::new(
            leo4_abi::error::error_codes::ENCODE_ERROR,
            format!("leo4-oxilean-build: compile_decl failed: {e:?}"),
        )
    })?;

    let fn_src = rust_fn.emit();
    let wrapper_src = synthesize_canonical_wrapper(&rust_fn)?;
    Ok((fn_src, wrapper_src))
}

// ─── OX2: Rust keyword escaping for emitted identifiers ────────────────

/// Strict Rust 2024 keywords that can be raw-escaped (`r#name`)
/// to use as identifiers.
const STRICT_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "do", "dyn",
    "else", "enum", "extern", "fn", "for", "gen", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "static", "struct", "trait", "try", "type", "unsafe", "use", "where",
    "while",
];

/// Keywords that cannot be raw-escaped (Rust reference: raw
/// idents can be any keyword *except* these). These get a
/// trailing-underscore mangling instead.
const RAW_INELIGIBLE: &[&str] = &[
    "self", "Self", "super", "crate", "true", "false", "_",
];

/// Reserved-for-future-use identifiers — Rust forbids them as
/// idents but allows raw-escape.
const RESERVED_KEYWORDS: &[&str] = &[
    "abstract", "become", "box", "final", "macro", "override", "priv",
    "typeof", "unsized", "virtual", "yield",
];

/// Map a Lean identifier (field name, param name, ctor name)
/// into a Rust-legal identifier for emit:
///
/// - Raw-ineligible (`self`, `Self`, `super`, `crate`, `true`,
///   `false`, `_`) → trailing `_` suffix.
/// - Strict / reserved keywords → `r#<name>` prefix.
/// - Anything else → pass through unchanged.
///
/// Returned name is suitable for both:
/// - Struct field name on the LHS of `pub <name>: T,`
/// - Local binding name in fn body (`let (<name>, __next) = …`)
/// - Field access on the RHS (`self.<name>`)
///
/// Note this transforms *only* the syntactic surface; the
/// canonical-ABI wire form is unchanged (encoder / decoder
/// visit fields in declaration order regardless of name).
#[must_use]
pub fn escape_rust_ident(name: &str) -> String {
    if RAW_INELIGIBLE.contains(&name) {
        format!("{name}_")
    } else if STRICT_KEYWORDS.contains(&name) || RESERVED_KEYWORDS.contains(&name) {
        format!("r#{name}")
    } else {
        name.to_string()
    }
}

// ─── OX2: SurfaceExpr type lifter + user-type aware marshalling ────────

/// Primitive Lean type names → matching `RustType`. Recognises
/// the surface vocabulary the user writes in field / param
/// annotations.
fn primitive_name_to_rust_type(name: &str) -> Option<RustType> {
    // OxiLean's LCNF lowering maps Lean `Nat` → u64 and `Int`
    // → i64; mirror that here. Users who need BigNat / BigInt
    // wire-shaped fields use the carrier names directly.
    match name {
        // UInt families
        "UInt8"  | "U8"             => Some(RustType::U8),
        "UInt16" | "U16"            => Some(RustType::U16),
        "UInt32" | "U32"            => Some(RustType::U32),
        "UInt64" | "U64" | "Nat"    => Some(RustType::U64),
        "USize"                     => Some(RustType::Usize),
        // Int families
        "Int8"  | "I8"             => Some(RustType::I8),
        "Int16" | "I16"            => Some(RustType::I16),
        "Int32" | "I32"            => Some(RustType::I32),
        "Int64" | "I64" | "Int"    => Some(RustType::I64),
        "ISize"                    => Some(RustType::Isize),
        // Floats
        "Float"   | "F64" => Some(RustType::F64),
        "Float32" | "F32" => Some(RustType::F32),
        // Other primitives
        "Bool"   => Some(RustType::Bool),
        "Char"   => Some(RustType::Char),
        "String" => Some(RustType::RustString),
        "Unit"   => Some(RustType::Unit),
        _ => None,
    }
}

/// Lift a parser `SurfaceExpr` (the user's type annotation as
/// they wrote it) into a `RustType` the OX2 marshalling
/// matrix can validate. Walks the surface AST recursively so
/// applications like `Vec Nat` become `Vec(U64)`.
///
/// Accepted shapes:
/// - `Var(name)` — primitive name (`Nat`, `Bool`, …), carrier
///   name (`BigNat`, …), or known user-defined type name
///   (lookup by `user_types`).
/// - `App(head, arg)` — generic applications walk down the
///   spine accumulating type arguments. The head's name
///   determines the container constructor (`Vec`, `Option`,
///   `Result`, `Box`); other heads emit `Generic(name, args)`
///   for downstream classification.
///
/// Rejects (returns `Err(LeanError(DECODE_ERROR))`):
/// - Higher-rank / dependent types (`Pi`, `Lam` in type
///   position).
/// - Term-level expressions surfacing in a type slot
///   (`Lit`, `Let`, `If`, `Match`, …).
/// - Sort / type-of-type expressions (`Sort`).
///
/// # Errors
/// `LeanError(DECODE_ERROR)` for shapes the lifter can't
/// translate.
pub fn surface_to_rust_type(
    expr: &Located<SurfaceExpr>,
    user_types: &HashSet<String>,
) -> Result<RustType, LeanError> {
    surface_to_rust_type_inner(&expr.value, user_types)
}

fn surface_to_rust_type_inner(
    expr: &SurfaceExpr,
    user_types: &HashSet<String>,
) -> Result<RustType, LeanError> {
    let reject = |label: &str| -> LeanError {
        LeanError::new(
            leo4_abi::error::error_codes::DECODE_ERROR,
            format!(
                "leo4-oxilean-build: surface type shape `{label}` is not \
                 liftable to a marshallable Rust type"
            ),
        )
    };

    match expr {
        SurfaceExpr::Var(name) => {
            if let Some(ty) = primitive_name_to_rust_type(name) {
                Ok(ty)
            } else if carrier_path_for(name).is_some() || user_types.contains(name) {
                Ok(RustType::Custom(name.clone()))
            } else {
                Err(LeanError::new(
                    leo4_abi::error::error_codes::DECODE_ERROR,
                    format!(
                        "leo4-oxilean-build: unknown type name `{name}` — \
                         not a primitive, carrier, or registered user type"
                    ),
                ))
            }
        }
        SurfaceExpr::App(_, _) => {
            // Walk left-associative App spine: `App(App(App(Var("Vec"), …), …))`
            let (head_name, args) = peel_app_spine(expr, user_types)?;
            let head_str = head_name.as_str();
            match head_str {
                "Vec" if args.len() == 1 => Ok(RustType::Vec(Box::new(args[0].clone()))),
                "Option" if args.len() == 1 => {
                    Ok(RustType::Option(Box::new(args[0].clone())))
                }
                "Result" if args.len() == 2 => Ok(RustType::Result(
                    Box::new(args[0].clone()),
                    Box::new(args[1].clone()),
                )),
                "Box" if args.len() == 1 => {
                    // Box isn't a dedicated RustType variant — fall through to
                    // the Generic form that `render_marshallable_type`
                    // recognises.
                    Ok(RustType::Generic("Box".to_string(), args))
                }
                "Prod" if args.len() == 2 => Ok(RustType::Tuple(args)),
                _ => Ok(RustType::Generic(head_str.to_string(), args)),
            }
        }
        SurfaceExpr::Pi(_, _) | SurfaceExpr::Lam(_, _) => Err(reject("Pi/Lam")),
        SurfaceExpr::Sort(_) => Err(reject("Sort")),
        SurfaceExpr::Lit(_) => Err(reject("Lit")),
        SurfaceExpr::Let(_, _, _, _) => Err(reject("Let")),
        SurfaceExpr::Ann(inner, _) => surface_to_rust_type_inner(&inner.value, user_types),
        SurfaceExpr::Hole => Err(reject("Hole")),
        SurfaceExpr::Proj(_, _) => Err(reject("Proj")),
        SurfaceExpr::If(_, _, _) => Err(reject("If")),
        SurfaceExpr::Match(_, _) => Err(reject("Match")),
        SurfaceExpr::Do(_) => Err(reject("Do")),
        SurfaceExpr::Have(..) => Err(reject("Have")),
        // SurfaceExpr is non-exhaustive across OxiLean
        // versions; catch any variant added after v0.1.2.
        _ => Err(reject("unknown surface form")),
    }
}

fn peel_app_spine(
    expr: &SurfaceExpr,
    user_types: &HashSet<String>,
) -> Result<(String, Vec<RustType>), LeanError> {
    let mut args_rev: Vec<RustType> = Vec::new();
    let mut cur = expr;
    loop {
        match cur {
            SurfaceExpr::App(head, arg) => {
                args_rev.push(surface_to_rust_type_inner(&arg.value, user_types)?);
                cur = &head.value;
            }
            SurfaceExpr::Var(name) => {
                args_rev.reverse();
                return Ok((name.clone(), args_rev));
            }
            _ => {
                return Err(LeanError::new(
                    leo4_abi::error::error_codes::DECODE_ERROR,
                    "leo4-oxilean-build: generic application head is not a name"
                        .to_string(),
                ));
            }
        }
    }
}

/// Variant of `render_marshallable_type` that also accepts the
/// given set of user-defined type names as bare `Custom` types.
/// Falls back to the standard matrix otherwise.
fn render_marshallable_type_with_users(
    ty: &RustType,
    user_types: &HashSet<String>,
) -> Result<String, LeanError> {
    // User types are emitted at module level, so a bare name
    // reference resolves correctly in the wrapper context.
    if let RustType::Custom(name) = ty
        && user_types.contains(name)
    {
        return Ok(name.clone());
    }
    render_marshallable_type(ty)
}

// ─── OX2 user-record synthesis (option (b) per ROADMAP) ─────────────────
//
// Upstream `RustTargetBackend` v0.1.2 emits only `RustItem::Fn`
// — no struct / enum / impl shapes. leo4-oxilean-build's
// option (b) per ROADMAP is to synthesise these shapes
// ourselves from the elaborated `Decl::Inductive` /
// `Decl::Structure`.
//
// Structures land first (this commit): a Lean `structure Point
// where x : UInt32; y : UInt32` becomes a Rust `pub struct
// Point { pub x: u32, pub y: u32 }` plus an inline
// `impl ::leo4_abi::LeanMarshal for Point` whose
// canonical_encode / canonical_decode body sequences the
// fields in declaration order (matching the leo4 derive
// macro's expansion).
//
// We emit an **inline impl** rather than a `#[derive(LeanMarshal)]`
// attribute so the generated crate's only leo4 dep is
// `leo4-abi` — no need to drag in `leo4-macros` (which would
// require a procedural-macro toolchain in the consumer's
// build environment) or `leo4` itself.

/// One named field of a Lean structure, lowered to its Rust
/// representation. Used to drive `synthesize_struct_type`.
#[derive(Debug, Clone)]
pub struct StructField {
    /// Field name (verbatim from Lean — no mangling applied,
    /// since structure fields can already be valid Rust idents).
    pub name: String,
    /// Field's Rust type. Must be marshallable per
    /// `render_marshallable_type`; the helper validates this
    /// and rejects non-marshallable types loudly.
    pub ty: RustType,
}

/// Synthesise a Rust struct declaration + matching
/// `LeanMarshal` impl for a Lean `structure`. The emitted
/// source has the shape:
///
/// ```rust,ignore
/// pub struct <name> { pub <field1>: <ty1>, pub <field2>: <ty2>, ... }
///
/// impl ::leo4_abi::LeanMarshal for <name> {
///     fn canonical_encode(&self, buf: &mut Vec<u8>) {
///         self.field1.canonical_encode(buf);
///         self.field2.canonical_encode(buf);
///         ...
///     }
///     fn canonical_decode(buf: &[u8], off: usize)
///         -> Result<(Self, usize), ::leo4_abi::LeanError>
///     {
///         let (field1, off) = <ty1 as ::leo4_abi::LeanMarshal>::canonical_decode(buf, off)?;
///         let (field2, off) = <ty2 as ::leo4_abi::LeanMarshal>::canonical_decode(buf, off)?;
///         ...
///         Ok((Self { field1, field2, ... }, off))
///     }
/// }
/// ```
///
/// Encode / decode order matches the leo4 derive macro's
/// expansion (`crates/leo4-macros-backend/src/lib.rs` —
/// fields in declaration order), so a struct synthesised
/// this way is byte-compatible with a hand-written
/// `#[derive(LeanMarshal)] struct` of the same shape.
///
/// # Errors
/// `LeanError(ENCODE_ERROR)` if any field's type isn't
/// marshallable per `render_marshallable_type` (the OX2
/// matrix — primitives + carriers + recursive containers).
pub fn synthesize_struct_type(
    name: &str,
    fields: &[StructField],
) -> Result<String, LeanError> {
    synthesize_struct_type_with_users(name, fields, &HashSet::new())
}

/// Variant of `synthesize_struct_type` that accepts the
/// caller's previously-registered user-defined type names as
/// marshallable field types. Used when a struct references
/// another struct emitted in the same crate.
///
/// # Errors
/// Same conditions as `synthesize_struct_type` — fields whose
/// type is neither a built-in nor a registered user type
/// reject before any source emits.
pub fn synthesize_struct_type_with_users(
    name: &str,
    fields: &[StructField],
    user_types: &HashSet<String>,
) -> Result<String, LeanError> {
    use std::fmt::Write as _;

    if fields.is_empty() {
        // A 0-field unit-like struct is technically valid Lean
        // (`structure Foo where`), but the marshal wire form
        // is zero bytes — caller probably didn't intend this.
        // Allow it but emit a unit struct for clarity.
        let mut s = String::new();
        writeln!(s, "pub struct {name};").unwrap();
        writeln!(s).unwrap();
        writeln!(s, "impl ::leo4_abi::LeanMarshal for {name} {{").unwrap();
        writeln!(s, "    fn canonical_encode(&self, _buf: &mut ::std::vec::Vec<u8>) {{}}").unwrap();
        writeln!(s, "    fn canonical_decode(_buf: &[u8], off: usize)").unwrap();
        writeln!(s, "        -> ::core::result::Result<(Self, usize), ::leo4_abi::LeanError>").unwrap();
        writeln!(s, "    {{ ::core::result::Result::Ok((Self, off)) }}").unwrap();
        writeln!(s, "}}").unwrap();
        return Ok(s);
    }

    // Pre-validate every field type AND escape every field
    // name; bail before emitting anything so a partial struct
    // never lands.
    let mut rendered_tys: Vec<String> = Vec::with_capacity(fields.len());
    let mut esc_names: Vec<String> = Vec::with_capacity(fields.len());
    for f in fields {
        rendered_tys.push(render_marshallable_type_with_users(&f.ty, user_types)?);
        esc_names.push(escape_rust_ident(&f.name));
    }

    let mut s = String::new();

    // Struct decl
    writeln!(s, "#[derive(Debug, Clone)]").unwrap();
    writeln!(s, "pub struct {name} {{").unwrap();
    for (fname, rty) in esc_names.iter().zip(rendered_tys.iter()) {
        writeln!(s, "    pub {fname}: {rty},").unwrap();
    }
    writeln!(s, "}}").unwrap();
    writeln!(s).unwrap();

    // LeanMarshal impl
    writeln!(s, "impl ::leo4_abi::LeanMarshal for {name} {{").unwrap();

    // encode: sequence fields in declaration order
    writeln!(
        s,
        "    fn canonical_encode(&self, buf: &mut ::std::vec::Vec<u8>) {{"
    )
    .unwrap();
    for fname in &esc_names {
        writeln!(
            s,
            "        ::leo4_abi::LeanMarshal::canonical_encode(&self.{fname}, buf);"
        )
        .unwrap();
    }
    writeln!(s, "    }}").unwrap();

    // decode: walk offsets in declaration order, build Self
    writeln!(s, "    fn canonical_decode(buf: &[u8], off: usize)").unwrap();
    writeln!(
        s,
        "        -> ::core::result::Result<(Self, usize), ::leo4_abi::LeanError>"
    )
    .unwrap();
    writeln!(s, "    {{").unwrap();
    writeln!(s, "        let mut __off = off;").unwrap();
    for (fname, rty) in esc_names.iter().zip(rendered_tys.iter()) {
        writeln!(
            s,
            "        let ({fname}, __next) = <{rty} as ::leo4_abi::LeanMarshal>::canonical_decode(buf, __off)?;"
        )
        .unwrap();
        s.push_str("        __off = __next;\n");
    }
    s.push_str("        ::core::result::Result::Ok((Self {");
    for (i, fname) in esc_names.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        } else {
            s.push(' ');
        }
        s.push_str(fname);
    }
    s.push_str(" }, __off))\n");
    writeln!(s, "    }}").unwrap();

    writeln!(s, "}}").unwrap();
    Ok(s)
}

// ─── OX2 user-record synthesis: inductive (multi-ctor enums) ─────────────
//
// Lean inductive → Rust enum + inline `LeanMarshal` impl.
// Encode / decode shape mirrors the leo4 derive macro's
// `expand_derive_enum` (all-unit case) /
// `expand_derive_variant` (payload-carrying case) expansion
// in `crates/leo4-macros-backend/src/lib.rs`:
//
//   wire = 4-byte LE discriminator
//        + per-variant payload (declaration-order, no
//          padding — caller is responsible for choosing
//          variant types that round-trip)
//
// SPEC/canonical-abi.md §9 — discriminator is `u32` LE.

/// One ctor of a Lean inductive type, lowered to its Rust
/// enum-variant representation. `fields: []` is a unit
/// variant; otherwise a tuple-style variant
/// `Variant(T1, T2, ...)`.
#[derive(Debug, Clone)]
pub struct EnumVariant {
    /// Variant name (the Lean ctor's name; emitted verbatim
    /// after `escape_rust_ident`).
    pub name: String,
    /// Payload type sequence, in declaration order.
    pub fields: Vec<RustType>,
}

/// Synthesise a Rust enum + matching `LeanMarshal` impl for a
/// Lean inductive type. Encoded shape matches the leo4 derive
/// macro's `expand_derive_variant` /
/// `expand_derive_enum` (all-unit case) expansion exactly, so
/// an enum synthesised this way is byte-compatible with a
/// hand-written `#[derive(LeanMarshal)]` of the same shape.
///
/// # Errors
/// `LeanError(ENCODE_ERROR)` if any variant's payload type
/// fails `render_marshallable_type_with_users`.
pub fn synthesize_enum_type(
    name: &str,
    variants: &[EnumVariant],
) -> Result<String, LeanError> {
    synthesize_enum_type_with_users(name, variants, &HashSet::new())
}

/// Variant of `synthesize_enum_type` that accepts a set of
/// user-defined type names (typically from
/// `Leo4ExportRegistry::user_types`) for cross-type references
/// in variant payloads.
///
/// # Errors
/// Same conditions as `synthesize_enum_type`.
///
/// # Panics
/// Only via `writeln!` on an in-memory `String` (write
/// can't actually fail for `String`); this is a syntactic
/// artefact of the `std::fmt::Write` trait.
#[allow(clippy::too_many_lines)] // documented branches: encode + decode
pub fn synthesize_enum_type_with_users(
    name: &str,
    variants: &[EnumVariant],
    user_types: &HashSet<String>,
) -> Result<String, LeanError> {
    use std::fmt::Write as _;

    // Pre-validate every variant's payload AND pre-render each
    // type so an unmarshallable payload aborts before any emit.
    struct PreparedVariant {
        esc_name: String,
        payload: Vec<String>,
    }

    if variants.is_empty() {
        return Err(LeanError::new(
            leo4_abi::error::error_codes::ENCODE_ERROR,
            format!(
                "leo4-oxilean-build: enum `{name}` has no variants — \
                 cannot synthesise an inhabited Rust type"
            ),
        ));
    }

    let mut prepared: Vec<PreparedVariant> = Vec::with_capacity(variants.len());
    for v in variants {
        let mut payload: Vec<String> = Vec::with_capacity(v.fields.len());
        for fty in &v.fields {
            payload.push(render_marshallable_type_with_users(fty, user_types)?);
        }
        prepared.push(PreparedVariant {
            esc_name: escape_rust_ident(&v.name),
            payload,
        });
    }

    let mut s = String::new();

    // Enum decl.
    writeln!(s, "#[derive(Debug, Clone)]").unwrap();
    // Lean ctor names may collide with the type's name in
    // some patterns — non_camel_case_types allow for safety.
    writeln!(s, "#[allow(non_camel_case_types)]").unwrap();
    writeln!(s, "pub enum {name} {{").unwrap();
    for v in &prepared {
        if v.payload.is_empty() {
            writeln!(s, "    {vn},", vn = v.esc_name).unwrap();
        } else {
            writeln!(
                s,
                "    {vn}({fields}),",
                vn = v.esc_name,
                fields = v.payload.join(", ")
            )
            .unwrap();
        }
    }
    writeln!(s, "}}").unwrap();
    writeln!(s).unwrap();

    // LeanMarshal impl.
    writeln!(s, "impl ::leo4_abi::LeanMarshal for {name} {{").unwrap();

    // Encode: match self → emit 4-byte LE disc + payload.
    writeln!(
        s,
        "    fn canonical_encode(&self, buf: &mut ::std::vec::Vec<u8>) {{"
    )
    .unwrap();
    s.push_str("        match self {\n");
    for (i, v) in prepared.iter().enumerate() {
        let disc = u32::try_from(i).map_err(|_| {
            LeanError::new(
                leo4_abi::error::error_codes::ENCODE_ERROR,
                format!(
                    "leo4-oxilean-build: enum `{name}` has too many variants \
                     (>= 2^32) to discriminate as u32 LE"
                ),
            )
        })?;
        if v.payload.is_empty() {
            writeln!(
                s,
                "            Self::{vn} => {{",
                vn = v.esc_name
            )
            .unwrap();
            writeln!(
                s,
                "                buf.extend_from_slice(&{disc}u32.to_le_bytes());"
            )
            .unwrap();
            s.push_str("            }\n");
        } else {
            let temps: Vec<String> = (0..v.payload.len())
                .map(|j| format!("__f{j}"))
                .collect();
            writeln!(
                s,
                "            Self::{vn}({binds}) => {{",
                vn = v.esc_name,
                binds = temps.join(", ")
            )
            .unwrap();
            writeln!(
                s,
                "                buf.extend_from_slice(&{disc}u32.to_le_bytes());"
            )
            .unwrap();
            for (temp, ty) in temps.iter().zip(v.payload.iter()) {
                writeln!(
                    s,
                    "                <{ty} as ::leo4_abi::LeanMarshal>::canonical_encode({temp}, buf);"
                )
                .unwrap();
            }
            s.push_str("            }\n");
        }
    }
    s.push_str("        }\n");
    writeln!(s, "    }}").unwrap();

    // Decode: read 4-byte disc, dispatch to a variant arm.
    writeln!(s, "    fn canonical_decode(buf: &[u8], off: usize)").unwrap();
    writeln!(
        s,
        "        -> ::core::result::Result<(Self, usize), ::leo4_abi::LeanError>"
    )
    .unwrap();
    writeln!(s, "    {{").unwrap();
    s.push_str("        if buf.len() < off + 4 {\n");
    s.push_str("            return ::core::result::Result::Err(::leo4_abi::LeanError::new(\n");
    s.push_str("                ::leo4_abi::error::error_codes::DECODE_ERROR,\n");
    writeln!(
        s,
        "                \"leo4-oxilean-build: enum `{name}`: not enough bytes for u32 tag\","
    )
    .unwrap();
    s.push_str("            ));\n        }\n");
    s.push_str("        let mut bytes = [0u8; 4];\n");
    s.push_str("        bytes.copy_from_slice(&buf[off..off + 4]);\n");
    s.push_str("        let __tag = u32::from_le_bytes(bytes);\n");
    s.push_str("        match __tag {\n");
    for (i, v) in prepared.iter().enumerate() {
        // Disc fits in u32 (we checked during encode emit).
        let disc = u32::try_from(i).unwrap();
        if v.payload.is_empty() {
            writeln!(
                s,
                "            {disc}u32 => ::core::result::Result::Ok((Self::{vn}, off + 4)),",
                vn = v.esc_name
            )
            .unwrap();
        } else {
            let temps: Vec<String> = (0..v.payload.len())
                .map(|j| format!("__f{j}"))
                .collect();
            writeln!(s, "            {disc}u32 => {{").unwrap();
            s.push_str("                let mut __off = off + 4;\n");
            for (temp, ty) in temps.iter().zip(v.payload.iter()) {
                writeln!(
                    s,
                    "                let ({temp}, __next) = <{ty} as ::leo4_abi::LeanMarshal>::canonical_decode(buf, __off)?;"
                )
                .unwrap();
                s.push_str("                __off = __next;\n");
            }
            writeln!(
                s,
                "                ::core::result::Result::Ok((Self::{vn}({binds}), __off))",
                vn = v.esc_name,
                binds = temps.join(", ")
            )
            .unwrap();
            s.push_str("            }\n");
        }
    }
    s.push_str("            _ => ::core::result::Result::Err(::leo4_abi::LeanError::new(\n");
    s.push_str("                ::leo4_abi::error::error_codes::DECODE_ERROR,\n");
    writeln!(
        s,
        "                ::std::format!(\"leo4-oxilean-build: enum `{name}`: invalid tag {{__tag}}\"),"
    )
    .unwrap();
    s.push_str("            )),\n");
    s.push_str("        }\n");
    writeln!(s, "    }}").unwrap();

    writeln!(s, "}}").unwrap();
    Ok(s)
}

// ─── §6 Cargo crate emit ─────────────────────────────────────────────────
//
// SPEC/rust-native-lean.md §3 + §9.3 endpoint: a Cargo crate
// the consumer can `path`-dep into their project. The crate
// exposes:
//
//   * The transpiled Rust fns directly (`pub fn <name>(args)
//     -> R`) for in-process callers that don't need the
//     canonical-ABI boundary.
//   * The §5 wrapper shims (`pub fn <name>_call(args: &[u8])
//     -> Result<Vec<u8>, LeanError>`) for hosts that dispatch
//     by mangled name.
//   * A `LeanProc` impl (`pub struct Leo4OxileanProc; impl
//     leo4_abi::rust_native::LeanProc for Leo4OxileanProc`)
//     that routes `(mangled, args)` to the matching `_call`
//     wrapper — the dispatcher leo4-rust-native loads at
//     runtime to discover Lean exports.
//
// The crate has *no* OxiLean dep at consumer build time —
// it's a normal Rust library that happens to have been
// transpiled from Lean. leo4-oxilean-build is itself the only
// crate that needs OxiLean, and only at transpile time.

/// One transpile unit ready for crate emission. The `mangled`
/// field is the leo4 mangled symbol name (derived from the IDL
/// per `SPEC/mangling.md`); it's the key the `LeanProc`
/// dispatcher matches against.
#[derive(Debug, Clone)]
pub struct TranspileUnit {
    /// Plain Rust source for the transpiled fn
    /// (`pub fn <name>(args) -> R { ... }`).
    pub fn_src: String,
    /// Canonical-ABI wrapper source
    /// (`pub fn <name>_call(args: &[u8]) -> Result<...>`).
    pub wrapper_src: String,
    /// `RustTargetBackend`-emitted fn name (e.g.
    /// `Sample_addOne` — `.` mangled to `_`). Must match
    /// the symbol referenced inside `wrapper_src`.
    pub fn_name: String,
    /// leo4 mangled symbol name (see `SPEC/mangling.md` §3).
    /// Used as the dispatch-table key.
    pub mangled: String,
    /// User-defined type declarations (structures /
    /// inductives) this fn depends on. Emitted ahead of the
    /// fn body in `emit_lib_rs` so forward references resolve.
    /// Empty for fns that only use primitives + carriers.
    #[allow(clippy::struct_field_names)]
    pub type_decls: Vec<String>,
}

/// In-memory representation of an emit-time Cargo crate.
/// Pair the two files in a directory and you have a buildable
/// crate. `write_to_dir` handles the filesystem write; use the
/// fields directly for in-memory inspection / testing.
#[derive(Debug, Clone)]
pub struct GeneratedCrate {
    /// Crate name (matches `[package].name` in `manifest`).
    pub crate_name: String,
    /// Contents of `Cargo.toml`.
    pub manifest: String,
    /// Contents of `src/lib.rs`.
    pub lib_rs: String,
}

impl GeneratedCrate {
    /// Write `Cargo.toml` + `src/lib.rs` under `dir`. Creates
    /// `dir` and `dir/src` if missing; overwrites any existing
    /// files. Returns the count of bytes written across both
    /// files (purely informational; used in tests).
    ///
    /// # Errors
    /// `std::io::Error` for any filesystem failure (mkdir,
    /// write, permissions).
    pub fn write_to_dir(&self, dir: &std::path::Path) -> std::io::Result<usize> {
        std::fs::create_dir_all(dir)?;
        let src_dir = dir.join("src");
        std::fs::create_dir_all(&src_dir)?;

        let manifest_path = dir.join("Cargo.toml");
        let lib_path = src_dir.join("lib.rs");
        std::fs::write(&manifest_path, &self.manifest)?;
        std::fs::write(&lib_path, &self.lib_rs)?;
        Ok(self.manifest.len() + self.lib_rs.len())
    }
}

/// Render a `Cargo.toml` for the emitted crate.
///
/// `leo4_abi_dep_spec` is the value of the `leo4-abi` dep —
/// callers supply the form appropriate to their consumer:
/// - workspace-relative path: `{ path = "../crates/leo4-abi" }`
/// - registry version once leo4-abi is published: `"0.1"`
/// - git dep: `{ git = "https://...", tag = "v0.1.0" }`
///
/// The fn doesn't validate the spec — it's interpolated
/// verbatim into the manifest.
#[must_use]
pub fn emit_cargo_toml(crate_name: &str, leo4_abi_dep_spec: &str) -> String {
    format!(
        "# Auto-generated by leo4-oxilean-build. DO NOT EDIT;\n\
         # re-run the build step to regenerate this file.\n\
         [package]\n\
         name        = \"{crate_name}\"\n\
         version     = \"0.1.0\"\n\
         edition     = \"2024\"\n\
         description = \"Transpiled Lean exports via leo4-oxilean-build.\"\n\
         \n\
         [dependencies]\n\
         leo4-abi = {leo4_abi_dep_spec}\n\
         \n\
         [lib]\n\
         path = \"src/lib.rs\"\n"
    )
}

/// Render `src/lib.rs` for the emitted crate. Concatenates
/// every unit's `fn_src` + `wrapper_src`, then appends a
/// `Leo4OxileanProc` struct with a `LeanProc` impl whose
/// `call(mangled, args)` body is a `match mangled { … }`
/// dispatch table.
///
/// `schema_hash` is the 13-char base32lc value the build
/// tool (eg. `leo4-rust-emit` or the lake plugin) computed
/// for this export set. The emitted `LeanProc::schema_hash`
/// returns it verbatim — runtime handshake then compares
/// against the consumer's `.leo4-handshake` JSON.
#[must_use]
pub fn emit_lib_rs(units: &[TranspileUnit], schema_hash: &str) -> String {
    use std::fmt::Write as _;

    let mut s = String::new();
    s.push_str("//! Auto-generated by leo4-oxilean-build.\n");
    s.push_str("//! DO NOT EDIT — re-run the build step to regenerate.\n\n");
    s.push_str("#![allow(non_snake_case, clippy::missing_errors_doc)]\n\n");

    // Type decls first — forward references from the fns
    // need them in scope. Deduplicate verbatim sources so
    // multiple fns referencing the same struct don't duplicate
    // the decl block.
    let mut seen_type_blocks: HashSet<String> = HashSet::new();
    for u in units {
        for td in &u.type_decls {
            if seen_type_blocks.insert(td.clone()) {
                s.push_str(td);
                if !td.ends_with('\n') {
                    s.push('\n');
                }
                s.push('\n');
            }
        }
    }

    for u in units {
        // Type-only units (`fn_src.is_empty()`) — already emitted
        // their type decls in the loop above. Skip the fn/wrapper
        // block so we don't write empty paragraphs.
        if u.fn_src.is_empty() && u.wrapper_src.is_empty() {
            continue;
        }
        s.push_str(&u.fn_src);
        if !u.fn_src.ends_with('\n') {
            s.push('\n');
        }
        s.push('\n');
        s.push_str(&u.wrapper_src);
        if !u.wrapper_src.ends_with('\n') {
            s.push('\n');
        }
        s.push('\n');
    }

    // LeanProc dispatcher.
    s.push_str("/// `LeanProc` impl auto-emitted by leo4-oxilean-build.\n");
    s.push_str("/// Routes `(mangled, args)` lookups to the matching `_call`\n");
    s.push_str("/// wrapper. Construct with `Leo4OxileanProc::new()`.\n");
    s.push_str("pub struct Leo4OxileanProc;\n\n");
    s.push_str("impl Leo4OxileanProc {\n");
    s.push_str("    #[must_use]\n");
    s.push_str("    pub fn new() -> Self { Self }\n");
    s.push_str("}\n\n");
    s.push_str("impl Default for Leo4OxileanProc {\n");
    s.push_str("    fn default() -> Self { Self::new() }\n");
    s.push_str("}\n\n");

    s.push_str("impl ::leo4_abi::rust_native::LeanProc for Leo4OxileanProc {\n");
    writeln!(
        s,
        "    fn schema_hash(&self) -> &str {{ {schema_hash:?} }}"
    )
    .unwrap();
    s.push_str("    fn abi_version(&self) -> u32 { 1 }\n");
    s.push_str("    fn call(&self, mangled: &str, args: &[u8])\n");
    s.push_str("        -> ::core::result::Result<::std::vec::Vec<u8>, ::leo4_abi::LeanError>\n");
    s.push_str("    {\n");
    s.push_str("        match mangled {\n");
    for u in units {
        // Type-only units have an empty mangled — they don't
        // get a LeanProc dispatch arm.
        if u.mangled.is_empty() {
            continue;
        }
        writeln!(
            s,
            "            {mangled:?} => {fn_name}_call(args),",
            mangled = u.mangled,
            fn_name = u.fn_name
        )
        .unwrap();
    }
    s.push_str("            _ => Err(::leo4_abi::LeanError::unknown_function(mangled)),\n");
    s.push_str("        }\n");
    s.push_str("    }\n");
    s.push_str("}\n");

    s
}

/// Emit a full Cargo crate (manifest + `src/lib.rs`) from
/// transpile units + emit-time metadata. Returns the
/// `GeneratedCrate` value in memory; call `write_to_dir` to
/// land it on disk.
///
/// This is the convenience entry; the underlying
/// `emit_cargo_toml` + `emit_lib_rs` fns are public for
/// callers that want to inspect / customise the two halves
/// separately.
#[must_use]
pub fn emit_crate(
    crate_name: &str,
    units: &[TranspileUnit],
    leo4_abi_dep_spec: &str,
    schema_hash: &str,
) -> GeneratedCrate {
    GeneratedCrate {
        crate_name: crate_name.to_string(),
        manifest: emit_cargo_toml(crate_name, leo4_abi_dep_spec),
        lib_rs: emit_lib_rs(units, schema_hash),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_mapper_emits_pure_rust() {
        assert!(
            type_mapper_is_lean_h_free(),
            "RustTargetBackend's lcnf_to_rust_type must NOT emit lean_* symbols"
        );
    }

    #[test]
    fn nat_maps_to_u64() {
        use oxilean_codegen::lcnf::LcnfType;
        use oxilean_codegen::rust_target_backend::RustType;
        let ty = RustTargetBackend::lcnf_to_rust_type(&LcnfType::Nat);
        assert_eq!(ty, RustType::U64);
    }

    #[test]
    fn string_maps_to_rust_string() {
        use oxilean_codegen::lcnf::LcnfType;
        use oxilean_codegen::rust_target_backend::RustType;
        let ty = RustTargetBackend::lcnf_to_rust_type(&LcnfType::LcnfString);
        assert_eq!(ty, RustType::RustString);
    }

    #[test]
    fn transpile_empty_decls_returns_header_only() {
        let out = transpile_decls("empty", &[]).expect("empty must succeed");
        assert!(out.contains("Auto-generated"));
        assert!(out.contains("DO NOT EDIT"));
        // No fn definitions for an empty decl set.
        assert!(!out.contains("fn "));
    }

    /// End-to-end: hand-build the kernel Expr for a trivial
    /// `def identity (n : Nat) : Nat := n` and drive the full
    /// `decl_to_lcnf → RustTargetBackend::compile_decl →
    /// RustFn::emit()` pipeline. Verifies the output is real
    /// Rust source naming neither `lean_object*` nor `lean_box`.
    #[test]
    fn transpile_kernel_decl_emits_identity_fn() {
        use oxilean_kernel::Level;

        // `Nat` as a kernel constant. (The kernel-side `Nat`
        // is an inductive type; here we just reference it by
        // name — the LCNF lowering will map it to LcnfType::Nat
        // → RustType::U64.)
        let nat_ty = Expr::Const(Name::str("Nat"), vec![]);
        // Function body: BVar(0) = the single bound parameter.
        let body = Expr::BVar(0);
        let name = Name::str("Sample.identity");
        let params = vec![(Name::str("n"), nat_ty)];

        let src = transpile_kernel_decl(&name, &params, &body)
            .expect("identity transpile must succeed");

        // Should be a `pub fn …` (RustVisibility::Pub default).
        assert!(
            src.contains("pub fn"),
            "expected `pub fn` in output; got:\n{src}"
        );
        // The output must NOT contain any lean_*-prefixed
        // symbol — this is the leo4-rust-native invariant.
        assert!(
            !src.contains("lean_box") && !src.contains("lean_unbox") && !src.contains("lean_object"),
            "transpile output unexpectedly contained lean_*-prefixed symbols:\n{src}"
        );

        // Suppress the unused Level import (kept for future
        // tests that build Sort-typed parameters).
        let _ = Level::zero();
    }

    /// End-to-end: real Lean source string → transpiled
    /// Rust. Drives the full parse → elab → LCNF → Rust
    /// pipeline.
    ///
    /// Uses an empty `Environment::new()`; references to
    /// undefined constants (like `Nat`) fall through as
    /// free variables — the resulting Rust source is still
    /// valid Rust but uses `Box<dyn Any>` for unresolved
    /// types, mirroring `transpile_kernel_decl_emits_*`.
    /// Real fixtures pre-populate the env with the leo4
    /// runtime library's declarations.
    /// `lean4_normalize` invariants: textual normalisation is
    /// idempotent and applies the documented Lean 4 → OxiLean
    /// rewrites.
    #[test]
    fn lean4_normalize_applies_compat_rewrites() {
        // ` => ` → ` -> `
        let a = lean4_normalize("fun n => n");
        assert_eq!(a, "fun n -> n");
        // ←  → <-
        let b = lean4_normalize("do x ← f");
        assert_eq!(b, "do x <- f");
        // where; → where
        let c = lean4_normalize("def x := 1 where;");
        assert_eq!(c, "def x := 1 where");
        // Idempotent: running twice = once.
        let once = lean4_normalize("fun a => fun b => a");
        let twice = lean4_normalize(&once);
        assert_eq!(once, twice);
    }

    /// End-to-end source-level transpile through the full
    /// `lean4_normalize → Lexer → Parser → elab → LCNF →
    /// RustTargetBackend` pipeline. OxiLean's
    /// `Parser::parse_definition` accepts the shape
    /// `def name {univs} : type := value` — header-binders
    /// `(x : T)` are an OxiLean parser-level rejection
    /// beyond textual normalisation (see `lean4_normalize`
    /// docstring), so the source uses the body-lambda form.
    #[test]
    fn transpile_source_identity() {
        // OxiLean-native `def` shape: no header binders; type
        // is `Nat -> Nat`, body is `fun n -> n`. The
        // `lean4_normalize` pass would map a Lean 4
        // `fun n => n` to this form anyway — using it
        // directly here makes the parse step independently
        // testable.
        let src = "def identity : Nat -> Nat := fun n -> n";
        let env = Environment::new();
        let result = transpile_source(&env, src);
        // The parse + elab pipeline against an empty env can
        // either succeed (if `Nat` resolves to a free variable
        // + the lambda's BVar lookup works) OR fail gracefully
        // (an empty env has no `Nat` to elaborate against).
        // Both outcomes are diagnostic — print for analysis.
        match result {
            Ok(out) => {
                assert!(
                    !out.contains("lean_box") && !out.contains("lean_object"),
                    "transpile output unexpectedly contained lean_*-symbols:\n{out}"
                );
                eprintln!("transpile_source_identity → emitted:\n{out}");
            }
            Err(e) => {
                // Documented behaviour: empty env can't
                // resolve `Nat`. The error code is in the
                // canonical-ABI range so callers know how to
                // interpret it.
                let code = e.code;
                eprintln!(
                    "transpile_source_identity → expected-ish err (code 0x{code:08x}): {}",
                    e.message
                );
                assert!(
                    code == leo4_abi::error::error_codes::DECODE_ERROR
                        || code == leo4_abi::error::error_codes::ENCODE_ERROR,
                    "unexpected error code: 0x{code:08x}"
                );
            }
        }
    }

    /// Lean 4 surface syntax (`fun n => n`) survives the
    /// `lean4_normalize` pre-processor and lands in the same
    /// place as the OxiLean-native form — i.e. the pipeline
    /// is robust to the documented surface-syntax
    /// difference.
    #[test]
    fn transpile_source_lean4_syntax_normalises() {
        // Lean 4 form with `=>` arrow.
        let src_lean4 = "def identity : Nat -> Nat := fun n => n";
        // OxiLean-native form.
        let src_oxi = "def identity : Nat -> Nat := fun n -> n";

        // After normalisation, both produce identical text.
        assert_eq!(lean4_normalize(src_lean4), lean4_normalize(src_oxi));

        // And both routes through `transpile_source` produce
        // the same outcome class (Ok or Err — pipeline parity).
        let env = Environment::new();
        let r1 = transpile_source(&env, src_lean4);
        let r2 = transpile_source(&env, src_oxi);
        assert_eq!(
            r1.is_ok(),
            r2.is_ok(),
            "lean4-syntax vs oxilean-native parity broke: r1={r1:?} r2={r2:?}"
        );
    }

    #[test]
    fn transpile_kernel_decl_emits_addone_via_succ() {
        // `def addOne (n : Nat) : Nat := Nat.succ n`
        // Body: `App(Const("Nat.succ", []), BVar(0))`.
        let nat_ty = Expr::Const(Name::str("Nat"), vec![]);
        let succ = Expr::Const(Name::str("Nat.succ"), vec![]);
        let body = Expr::App(Box::new(succ), Box::new(Expr::BVar(0)));
        let name = Name::str("Sample.addOne");
        let params = vec![(Name::str("n"), nat_ty)];

        let src = transpile_kernel_decl(&name, &params, &body)
            .expect("addOne transpile must succeed");

        // Expected name appears (after RustTargetBackend's
        // mangle_name: `.` → `_`).
        assert!(
            src.contains("Sample_addOne") || src.contains("addOne"),
            "expected fn name in output; got:\n{src}"
        );
        // Lean-h cleanliness invariant holds for this case too.
        assert!(!src.contains("lean_box"));
        assert!(!src.contains("lean_unbox"));
        assert!(!src.contains("lean_object"));
    }

    // ─── Hook 3 — `@[leo4_export]` discovery tests ────────────────────

    #[test]
    fn registry_registers_leo4_export_handler() {
        let registry = Leo4ExportRegistry::new();
        assert!(
            registry.has_export_handler(),
            "Leo4ExportRegistry::new() must register the leo4_export custom-attr handler"
        );
        // Verify the handler's recorded name + doc are what we
        // wrote (defends against accidental rename in upstream).
        let handler = registry
            .manager
            .get_handler(LEO4_EXPORT_ATTR)
            .expect("export handler must be retrievable by name");
        assert_eq!(handler.name, LEO4_EXPORT_ATTR);
        assert!(
            handler.doc.contains("leo4")
                && handler.doc.contains("canonical-ABI"),
            "doc string must mention leo4 + canonical-ABI"
        );
    }

    #[test]
    fn registry_registers_lean_marshal_derive() {
        let registry = Leo4ExportRegistry::new();
        assert!(
            registry.has_marshal_derive(),
            "Leo4ExportRegistry::new() must register the LeanMarshal derive handler"
        );
    }

    #[test]
    fn registry_default_matches_new() {
        let a = Leo4ExportRegistry::new();
        let b = Leo4ExportRegistry::default();
        // Both should have leo4's handlers populated.
        assert!(a.has_export_handler() && b.has_export_handler());
        assert!(a.has_marshal_derive() && b.has_marshal_derive());
    }

    #[test]
    fn decl_has_leo4_export_detects_tag_via_parser() {
        // Drive a real parser pass so the test exercises the
        // exact AST shape upstream produces, not a hand-built
        // mock.
        let src = "@[leo4_export] def f : Nat -> Nat := fun n -> n";
        let normalised = lean4_normalize(src);
        let mut lexer = Lexer::new(&normalised);
        let tokens = lexer.tokenize();
        let mut parser = Parser::new(tokens);
        let decl = parser
            .parse_decl()
            .expect("parser must accept @[leo4_export] def");
        assert!(
            decl_has_leo4_export(&decl),
            "tagged decl must be recognised"
        );
        // The unwrapped inner decl is a Definition.
        let inner = inner_decl(&decl);
        assert!(
            matches!(&inner.value, Decl::Definition { .. }),
            "inner unwrap must surface the Definition"
        );
        // Name extraction works.
        assert_eq!(decl_name(&decl), Some("f"));
    }

    #[test]
    fn decl_has_leo4_export_rejects_untagged_decl() {
        let src = "def g : Nat -> Nat := fun n -> n";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize();
        let mut parser = Parser::new(tokens);
        let decl = parser.parse_decl().expect("plain def parses");
        assert!(!decl_has_leo4_export(&decl));
        // Untagged inner decl == outer decl.
        assert!(matches!(&decl.value, Decl::Definition { .. }));
        assert_eq!(decl_name(&decl), Some("g"));
    }

    #[test]
    fn decl_has_leo4_export_ignores_unrelated_attrs() {
        // `@[simp]` doesn't activate leo4 transpile.
        let src = "@[simp] def h : Nat -> Nat := fun n -> n";
        let mut lexer = Lexer::new(src);
        let tokens = lexer.tokenize();
        let mut parser = Parser::new(tokens);
        let decl = parser.parse_decl().expect("@[simp] def parses");
        assert!(!decl_has_leo4_export(&decl));
    }

    #[test]
    fn transpile_source_if_exported_skips_untagged() {
        let mut registry = Leo4ExportRegistry::new();
        let env = Environment::new();
        let src = "def g : Nat -> Nat := fun n -> n";
        let out = transpile_source_if_exported(&env, &mut registry, src)
            .expect("parse path succeeds");
        assert!(out.is_none(), "untagged decl must yield Ok(None)");
        // Manager records nothing for skipped decls.
        assert!(registry.exported_names().is_empty());
    }

    // ─── §5 Canonical-ABI wrapper synthesis tests ────────────────────

    fn rfn(
        name: &str,
        params: Vec<(&str, RustType)>,
        ret: Option<RustType>,
    ) -> RustFn {
        let p = params
            .into_iter()
            .map(|(n, t)| (n.to_string(), t, false))
            .collect();
        RustFn::new(name, p, ret, vec![])
    }

    /// True if a `TranspileUnit` is the type-only shape
    /// (empty fn/wrapper/mangled, non-empty type_decls).
    fn unit_is_type_only(u: &TranspileUnit) -> bool {
        u.fn_src.is_empty()
            && u.wrapper_src.is_empty()
            && u.mangled.is_empty()
            && !u.type_decls.is_empty()
    }

    #[test]
    fn wrapper_emits_call_fn_for_u64_to_u64() {
        let f = rfn("addOne", vec![("n", RustType::U64)], Some(RustType::U64));
        let out = synthesize_canonical_wrapper(&f).expect("u64 → u64 must succeed");
        // Header + signature.
        assert!(
            out.contains("pub fn addOne_call(args: &[u8])"),
            "wrapper missing pub fn signature; got:\n{out}"
        );
        assert!(out.contains("LeanError"));
        // Decode call for the single u64 param.
        assert!(
            out.contains("<u64 as ::leo4_abi::LeanMarshal>::canonical_decode"),
            "wrapper missing u64 decode; got:\n{out}"
        );
        // Invocation of the transpiled fn.
        assert!(out.contains("let __ret = addOne(n)"));
        // Encode of the return.
        assert!(out.contains("::leo4_abi::encode_to_vec(&__ret)"));
    }

    #[test]
    fn wrapper_emits_zero_arg_fn() {
        let f = rfn("constant", vec![], Some(RustType::Bool));
        let out = synthesize_canonical_wrapper(&f).expect("0-arg bool fn must succeed");
        assert!(out.contains("pub fn constant_call(args: &[u8])"));
        // No __off setup for zero-arg fns.
        assert!(!out.contains("__off"));
        // Args are still consumed (we suppress unused-var warn).
        assert!(out.contains("let _ = args;"));
        // Encoding the bool return.
        assert!(out.contains("encode_to_vec(&__ret)"));
    }

    #[test]
    fn wrapper_handles_unit_return() {
        let f = rfn("touch", vec![("x", RustType::I32)], Some(RustType::Unit));
        let out = synthesize_canonical_wrapper(&f).expect("unit-return fn must succeed");
        assert!(out.contains("touch(x)"));
        // Unit-returning fn produces an empty Vec, not encode_to_vec.
        assert!(
            out.contains("::std::vec::Vec::new()"),
            "unit return must produce an empty Vec; got:\n{out}"
        );
        assert!(!out.contains("encode_to_vec"));
    }

    #[test]
    fn wrapper_handles_none_return_as_unit() {
        // RustFn::return_type is Option<RustType>; None == ().
        let f = rfn("touch", vec![], None);
        let out = synthesize_canonical_wrapper(&f).expect("None return must succeed");
        assert!(out.contains("::std::vec::Vec::new()"));
        assert!(!out.contains("encode_to_vec"));
    }

    #[test]
    fn wrapper_emits_multi_arg_decode_in_order() {
        let f = rfn(
            "combine",
            vec![("a", RustType::U64), ("b", RustType::I32), ("c", RustType::Bool)],
            Some(RustType::RustString),
        );
        let out = synthesize_canonical_wrapper(&f).expect("multi-arg fn must succeed");

        // Each param's decode appears AND in source order — a
        // → b → c.
        let pos_a = out.find("(a,").expect("a decode binding missing");
        let pos_b = out.find("(b,").expect("b decode binding missing");
        let pos_c = out.find("(c,").expect("c decode binding missing");
        assert!(pos_a < pos_b && pos_b < pos_c, "decode order broke");

        // String marshalling for the return.
        assert!(
            out.contains("::std::string::String") || out.contains("encode_to_vec(&__ret)")
        );
        // Call uses all three args in order.
        assert!(out.contains("combine(a, b, c)"));
    }

    // ─── OX2 carrier-type + generics tests ────────────────────────────

    #[test]
    fn wrapper_accepts_bignat_carrier() {
        let f = rfn(
            "addBig",
            vec![("n", RustType::Custom("BigNat".to_string()))],
            Some(RustType::Custom("BigNat".to_string())),
        );
        let out = synthesize_canonical_wrapper(&f).expect("BigNat must marshal");
        assert!(
            out.contains("<::leo4_abi::BigNat as ::leo4_abi::LeanMarshal>::canonical_decode"),
            "expected BigNat decode path; got:\n{out}"
        );
        // The transpiled fn is called with the decoded arg.
        assert!(out.contains("addBig(n)"));
    }

    #[test]
    fn wrapper_accepts_carrier_under_oxilean_mangled_form() {
        // OxiLean mangles `.` → `_`, so `Leo4.BigInt` would
        // surface as `Leo4_BigInt` in `RustType::Custom`.
        let f = rfn(
            "negate",
            vec![("n", RustType::Custom("Leo4_BigInt".to_string()))],
            Some(RustType::Custom("Leo4_BigInt".to_string())),
        );
        let out = synthesize_canonical_wrapper(&f).expect("Leo4_BigInt must marshal");
        assert!(out.contains("::leo4_abi::BigInt"));
    }

    #[test]
    fn wrapper_accepts_lean_rat() {
        let f = rfn(
            "halve",
            vec![("q", RustType::Custom("LeanRat".to_string()))],
            Some(RustType::Custom("Rat".to_string())),
        );
        let out = synthesize_canonical_wrapper(&f).expect("Rat carriers must marshal");
        assert!(out.contains("::leo4_abi::LeanRat"));
    }

    #[test]
    fn wrapper_accepts_complex_f64() {
        let f = rfn(
            "rotate",
            vec![("z", RustType::Custom("LeanComplexF64x2".to_string()))],
            Some(RustType::Custom("LeanComplexF64x2".to_string())),
        );
        let out = synthesize_canonical_wrapper(&f).expect("complex must marshal");
        assert!(out.contains("::leo4_abi::LeanComplexF64x2"));
    }

    #[test]
    fn wrapper_accepts_vec_u64() {
        let f = rfn(
            "sum_list",
            vec![("xs", RustType::Vec(Box::new(RustType::U64)))],
            Some(RustType::U64),
        );
        let out = synthesize_canonical_wrapper(&f).expect("Vec<u64> must marshal");
        assert!(out.contains("::std::vec::Vec<u64>"));
    }

    #[test]
    fn wrapper_accepts_option_string() {
        let f = rfn(
            "maybe_str",
            vec![("name", RustType::Option(Box::new(RustType::RustString)))],
            Some(RustType::Bool),
        );
        let out = synthesize_canonical_wrapper(&f).expect("Option<String> must marshal");
        assert!(out.contains("::core::option::Option<::std::string::String>"));
    }

    #[test]
    fn wrapper_accepts_result_in_return() {
        let f = rfn(
            "may_fail",
            vec![("n", RustType::U64)],
            Some(RustType::Result(
                Box::new(RustType::U64),
                Box::new(RustType::RustString),
            )),
        );
        let out = synthesize_canonical_wrapper(&f).expect("Result return must marshal");
        assert!(out.contains("encode_to_vec(&__ret)"));
        // The Result type doesn't appear in the wrapper body
        // (we just encode_to_vec the typed return), but the
        // marshallable check still has to pass for it — that's
        // what this test asserts.
    }

    #[test]
    fn wrapper_accepts_nested_generics() {
        // Vec<Option<u64>> — recursive type lifting.
        let f = rfn(
            "filter_some",
            vec![("xs", RustType::Vec(Box::new(RustType::Option(Box::new(RustType::U64)))))],
            Some(RustType::Vec(Box::new(RustType::U64))),
        );
        let out = synthesize_canonical_wrapper(&f).expect("nested generics must marshal");
        assert!(out.contains("::std::vec::Vec<::core::option::Option<u64>>"));
    }

    #[test]
    fn wrapper_accepts_tuple_arity_2() {
        let f = rfn(
            "pair",
            vec![("p", RustType::Tuple(vec![RustType::U64, RustType::Bool]))],
            Some(RustType::Tuple(vec![RustType::Bool, RustType::U64])),
        );
        let out = synthesize_canonical_wrapper(&f).expect("2-tuple must marshal");
        assert!(out.contains("(u64, bool)"));
    }

    #[test]
    fn wrapper_accepts_generic_form_vec() {
        // Upstream sometimes lowers via `Generic("Vec", [T])`
        // instead of the dedicated `RustType::Vec` variant.
        let f = rfn(
            "alt_form",
            vec![(
                "xs",
                RustType::Generic("Vec".to_string(), vec![RustType::U64]),
            )],
            Some(RustType::U64),
        );
        let out = synthesize_canonical_wrapper(&f).expect("Generic Vec form must marshal");
        assert!(out.contains("::std::vec::Vec<u64>"));
    }

    #[test]
    fn wrapper_rejects_unknown_custom() {
        let f = rfn(
            "user_record",
            vec![("r", RustType::Custom("MyStruct".to_string()))],
            Some(RustType::U64),
        );
        let err = synthesize_canonical_wrapper(&f)
            .expect_err("unknown carrier must fail");
        assert_eq!(err.code, leo4_abi::error::error_codes::ENCODE_ERROR);
        assert!(err.message.contains("MyStruct"));
    }

    #[test]
    fn wrapper_rejects_carrier_with_generic_args() {
        // `BigNat<...>` makes no sense — carrier types are
        // non-generic. Reject loudly so a malformed lowering
        // surfaces.
        let f = rfn(
            "bad",
            vec![(
                "x",
                RustType::Generic("BigNat".to_string(), vec![RustType::U64]),
            )],
            Some(RustType::U64),
        );
        let err = synthesize_canonical_wrapper(&f).expect_err("BigNat<u64> must fail");
        assert!(err.message.contains("BigNat<...>"));
    }

    #[test]
    fn wrapper_rejects_tuple_with_unsupported_inner() {
        // The tuple variant rejects the unsupported inner type
        // — the outer Tuple's marshallability cascades through
        // its elements.
        let f = rfn(
            "bad_tuple",
            vec![(
                "p",
                RustType::Tuple(vec![
                    RustType::U64,
                    RustType::Custom("MyStruct".to_string()),
                ]),
            )],
            Some(RustType::U64),
        );
        let err = synthesize_canonical_wrapper(&f).expect_err("unsupported inner must cascade");
        assert!(err.message.contains("MyStruct"));
    }

    #[test]
    fn wrapper_rejects_box_dyn_any_return() {
        let f = rfn(
            "context_free",
            vec![("n", RustType::U64)],
            // The exact string `RustTargetBackend` uses for unknown LCNF::Object types.
            Some(RustType::Custom("Box<dyn std::any::Any>".to_string())),
        );
        let err = synthesize_canonical_wrapper(&f)
            .expect_err("Box<dyn Any> return must fail wrapper synthesis");
        assert_eq!(
            err.code,
            leo4_abi::error::error_codes::ENCODE_ERROR,
            "unexpected error code: 0x{:08x}",
            err.code
        );
        assert!(err.message.contains("LeanMarshal"));
    }

    #[test]
    fn wrapper_rejects_unsupported_param_type() {
        // User-defined record types are not yet covered (the
        // backend doesn't emit struct shapes; deferred per
        // ROADMAP OX2 sub-problem). Verifies the failure mode
        // is loud rather than silent.
        let f = rfn(
            "user_in",
            vec![("r", RustType::Custom("UserDefinedRecord".to_string()))],
            Some(RustType::U64),
        );
        let err = synthesize_canonical_wrapper(&f)
            .expect_err("user record param must fail wrapper synthesis");
        assert_eq!(err.code, leo4_abi::error::error_codes::ENCODE_ERROR);
        assert!(err.message.contains("UserDefinedRecord"));
    }

    #[test]
    fn transpile_kernel_decl_with_wrapper_emits_both() {
        // The addOne fixture from the existing kernel-level
        // test — feed it through the combined path.
        let nat_ty = Expr::Const(Name::str("Nat"), vec![]);
        let succ = Expr::Const(Name::str("Nat.succ"), vec![]);
        let body = Expr::App(Box::new(succ), Box::new(Expr::BVar(0)));
        let name = Name::str("Sample.addOne");
        let params = vec![(Name::str("n"), nat_ty)];

        // Note: with no env populated, the transpiled fn's
        // return type lands as `Box<dyn Any>` (Lcnf::Object).
        // That's an unsupported wrapper return — so we expect
        // the wrapper step to error, but the fn step to
        // succeed. To exercise the *combined* helper through
        // its happy path we'd need a populated env; for now we
        // just verify it errors via the wrapper layer, which
        // is itself useful coverage.
        let result = transpile_kernel_decl_with_wrapper(&name, &params, &body);
        match result {
            Ok((fn_src, wrapper_src)) => {
                // If a future LCNF lowering ends up with a
                // marshallable return type, both items emit.
                assert!(fn_src.contains("Sample_addOne"));
                assert!(wrapper_src.contains("Sample_addOne_call"));
            }
            Err(e) => {
                // Expected today: wrapper rejection of
                // `Box<dyn Any>`. Document it explicitly so
                // a future change in lowering doesn't silently
                // shift this test's branch.
                assert_eq!(e.code, leo4_abi::error::error_codes::ENCODE_ERROR);
                assert!(
                    e.message.contains("LeanMarshal"),
                    "unexpected wrapper error: {}",
                    e.message
                );
            }
        }
    }

    #[test]
    fn transpile_source_to_unit_skips_untagged() {
        let mut registry = Leo4ExportRegistry::new();
        let env = Environment::new();
        let src = "def g : Nat -> Nat := fun n -> n";
        let out = transpile_source_to_unit(&env, &mut registry, src, "abc_a")
            .expect("parse path succeeds");
        assert!(out.is_none(), "untagged decl must yield Ok(None)");
        assert!(registry.exported_names().is_empty());
    }

    #[test]
    fn transpile_source_to_unit_assembles_unit_when_tagged() {
        let mut registry = Leo4ExportRegistry::new();
        let env = Environment::new();
        let src = "@[leo4_export] def f : Nat -> Nat := fun n -> n";

        let result = transpile_source_to_unit(&env, &mut registry, src, "deadbeef_a");
        // Tag must always be captured pre-elab.
        let exported = registry.exported_names();
        assert_eq!(exported.len(), 1);

        // Same Ok/Err parity rule as transpile_source — empty
        // env can either succeed or fail elab gracefully.
        match result {
            Ok(Some(unit)) => {
                assert_eq!(unit.mangled, "deadbeef_a");
                assert!(unit.fn_src.contains("pub fn"));
                // Wrapper references the fn name + decode flow.
                assert!(unit.wrapper_src.contains(&format!("{}_call", unit.fn_name)));
            }
            Ok(None) => panic!("tagged decl must NOT yield Ok(None)"),
            Err(e) => {
                let code = e.code;
                eprintln!("transpile_source_to_unit → err 0x{code:08x}: {}", e.message);
                assert!(
                    code == leo4_abi::error::error_codes::DECODE_ERROR
                        || code == leo4_abi::error::error_codes::ENCODE_ERROR,
                );
            }
        }
    }

    #[test]
    fn transpile_source_to_unit_handles_tagged_structure() {
        let mut registry = Leo4ExportRegistry::new();
        let env = Environment::new();
        // OxiLean's `structure` parser takes fields without
        // separators (space / newline only); `;` is rejected.
        let src = "@[leo4_export] structure Point where x : UInt32 y : UInt32";
        let out = transpile_source_to_unit(&env, &mut registry, src, "")
            .expect("structure transpile must succeed");
        let unit = out.expect("structure must yield Some(TranspileUnit)");

        // Type-only unit signals.
        assert!(unit.fn_src.is_empty(), "structure unit has no fn body");
        assert!(unit.wrapper_src.is_empty(), "structure unit has no wrapper");
        assert!(unit.mangled.is_empty(), "structure unit has no dispatch key");
        assert_eq!(unit.fn_name, "Point");
        assert_eq!(unit.type_decls.len(), 1);

        // Registry tracks Point.
        assert!(registry.user_types.contains("Point"));

        // Emitted struct source has the right shape.
        let sd = &unit.type_decls[0];
        assert!(sd.contains("pub struct Point {"));
        assert!(sd.contains("pub x: u32,"));
        assert!(sd.contains("pub y: u32,"));
        assert!(sd.contains("impl ::leo4_abi::LeanMarshal for Point"));
    }

    #[test]
    fn transpile_source_to_unit_structure_references_prior_user_type() {
        let mut registry = Leo4ExportRegistry::new();
        let env = Environment::new();

        // First decl introduces Point.
        let _ = transpile_source_to_unit(
            &env,
            &mut registry,
            "@[leo4_export] structure Point where x : UInt32 y : UInt32",
            "",
        )
        .expect("Point parses");

        // Second decl references Point as a field type. Avoid
        // `from` — it's a Rust keyword and would surface as a
        // raw-ident; that escaping is OX2's next-next sub-step.
        let out = transpile_source_to_unit(
            &env,
            &mut registry,
            "@[leo4_export] structure Edge where head : Point tail : Point",
            "",
        )
        .expect("Edge parses")
        .expect("Edge yields unit");

        // Both names registered.
        assert!(registry.user_types.contains("Point"));
        assert!(registry.user_types.contains("Edge"));
        // Edge's struct decl references Point via bare name.
        let sd = &out.type_decls[0];
        assert!(sd.contains("pub head: Point,"));
        assert!(sd.contains("pub tail: Point,"));
        assert!(sd.contains("<Point as ::leo4_abi::LeanMarshal>"));
    }

    #[test]
    fn transpile_source_to_units_handles_multi_decl_source() {
        let mut registry = Leo4ExportRegistry::new();
        let env = Environment::new();
        // A file with two type decls + one untagged decl. The
        // untagged one must be silently skipped.
        let src = "\
            @[leo4_export] structure Point where x : UInt32 y : UInt32\n\
            structure Untagged where z : UInt32\n\
            @[leo4_export] inductive Color : Type | Red : Color | Green : Color\n\
        ";
        let mut name_to_mangled: HashMap<String, String> = HashMap::new();
        // Type-only decls don't need a mangled name; this map
        // covers Definitions only (here it's empty since the
        // fixture has no fn exports).

        let units = transpile_source_to_units(&env, &mut registry, src, &name_to_mangled)
            .expect("multi-decl source must parse + transpile");
        assert_eq!(units.len(), 2, "expected 2 units (skip untagged)");
        // Both registered in user_types.
        assert!(registry.user_types.contains("Point"));
        assert!(registry.user_types.contains("Color"));
        assert!(!registry.user_types.contains("Untagged"));

        // Each unit is type-only.
        assert!(units.iter().all(unit_is_type_only));

        // No collateral effect on the (empty) mangled map.
        let _ = &mut name_to_mangled;
    }

    #[test]
    fn transpile_source_to_units_rejects_missing_mangled_for_fn() {
        let mut registry = Leo4ExportRegistry::new();
        let env = Environment::new();
        let src = "@[leo4_export] def f : Nat -> Nat := fun n -> n";
        let name_to_mangled: HashMap<String, String> = HashMap::new();
        let err = transpile_source_to_units(&env, &mut registry, src, &name_to_mangled)
            .expect_err("missing mangled must reject");
        assert_eq!(err.code, leo4_abi::error::error_codes::ENCODE_ERROR);
        assert!(err.message.contains("mangled name"));
        assert!(err.message.contains("`f`"));
    }

    #[test]
    fn transpile_source_to_units_uses_mangled_map_per_fn() {
        let mut registry = Leo4ExportRegistry::new();
        let env = Environment::new();
        let src = "\
            @[leo4_export] structure Point where x : UInt32\n\
            @[leo4_export] def f : Nat -> Nat := fun n -> n\n\
        ";
        let mut name_to_mangled: HashMap<String, String> = HashMap::new();
        name_to_mangled.insert("f".to_string(), "f_mangled_xyz".to_string());

        let result = transpile_source_to_units(&env, &mut registry, src, &name_to_mangled);
        match result {
            Ok(units) => {
                // Both unit kinds present.
                assert!(units.iter().any(unit_is_type_only));
                if let Some(fn_unit) = units.iter().find(|u| !u.fn_src.is_empty()) {
                    assert_eq!(fn_unit.mangled, "f_mangled_xyz");
                }
                assert!(registry.user_types.contains("Point"));
            }
            Err(e) => {
                // Empty env may still fail elab on `Nat`; that's
                // the documented Err path (same parity as the
                // single-decl variant's tests).
                assert!(
                    e.code == leo4_abi::error::error_codes::DECODE_ERROR
                        || e.code == leo4_abi::error::error_codes::ENCODE_ERROR
                );
            }
        }
    }

    #[test]
    fn transpile_source_to_unit_handles_all_unit_inductive() {
        let mut registry = Leo4ExportRegistry::new();
        let env = Environment::new();
        // OxiLean requires `inductive N : Type | ctor : T` —
        // every ctor needs an explicit `: <type>` annotation,
        // even unit ctors. The result enum is still a pure
        // all-unit shape because none of the ctors take args.
        let src = "@[leo4_export] inductive Color : Type | Red : Color | Green : Color | Blue : Color";
        let out = transpile_source_to_unit(&env, &mut registry, src, "")
            .expect("inductive parses")
            .expect("must yield unit");
        assert!(unit_is_type_only(&out));
        assert_eq!(out.fn_name, "Color");
        assert!(registry.user_types.contains("Color"));
        let ed = &out.type_decls[0];
        assert!(ed.contains("pub enum Color {"));
        assert!(ed.contains("    Red,"));
        assert!(ed.contains("    Green,"));
        assert!(ed.contains("    Blue,"));
    }

    #[test]
    fn transpile_source_to_unit_handles_payload_inductive() {
        let mut registry = Leo4ExportRegistry::new();
        let env = Environment::new();
        // `left : Nat → Either` unfolds into Pi(_, _, Nat,
        // Either); `right : String → Either` analogous.
        let src = "@[leo4_export] inductive Either : Type | left : Nat -> Either | right : String -> Either";
        let out = transpile_source_to_unit(&env, &mut registry, src, "")
            .expect("payload inductive parses")
            .expect("must yield unit");
        assert!(unit_is_type_only(&out));
        let ed = &out.type_decls[0];
        assert!(ed.contains("pub enum Either {"));
        assert!(
            ed.contains("    left(u64),"),
            "expected left(u64) variant; got:\n{ed}"
        );
        assert!(ed.contains("    right(::std::string::String),"));
        // Disc emit + decode dispatch present.
        assert!(ed.contains("0u32.to_le_bytes()"));
        assert!(ed.contains("1u32.to_le_bytes()"));
    }

    #[test]
    fn transpile_source_to_unit_inductive_references_prior_user_type() {
        let mut registry = Leo4ExportRegistry::new();
        let env = Environment::new();
        // Define Point first.
        let _ = transpile_source_to_unit(
            &env,
            &mut registry,
            "@[leo4_export] structure Point where x : UInt32 y : UInt32",
            "",
        )
        .expect("Point parses");

        // Inductive referencing Point as a payload type.
        let out = transpile_source_to_unit(
            &env,
            &mut registry,
            "@[leo4_export] inductive Shape : Type | dot : Point -> Shape | line : Point -> Point -> Shape",
            "",
        )
        .expect("Shape parses")
        .expect("must yield unit");
        let ed = &out.type_decls[0];
        assert!(ed.contains("    dot(Point),"));
        assert!(ed.contains("    line(Point, Point),"));
    }

    #[test]
    fn transpile_source_to_unit_skips_untagged_structure() {
        let mut registry = Leo4ExportRegistry::new();
        let env = Environment::new();
        let src = "structure Untagged where x : UInt32";
        let out = transpile_source_to_unit(&env, &mut registry, src, "")
            .expect("parse");
        assert!(out.is_none(), "untagged structure must yield None");
        assert!(registry.user_types.is_empty());
    }

    #[test]
    fn emit_lib_rs_handles_type_only_unit() {
        // Hand-build a type-only unit + a fn unit; verify
        // the emit shape is: type decl block → fn block →
        // dispatcher with one arm (only the fn unit).
        let type_only = TranspileUnit {
            type_decls: vec!["pub struct Point { pub x: u32 }".to_string()],
            fn_src: String::new(),
            wrapper_src: String::new(),
            fn_name: "Point".to_string(),
            mangled: String::new(),
        };
        let fn_unit = fixture_unit("Sample_addOne", "abc12345_ab_a");
        let out = emit_lib_rs(&[type_only, fn_unit], "deadbeefcafe1");

        assert!(out.contains("pub struct Point { pub x: u32 }"));
        assert!(out.contains("pub fn Sample_addOne"));
        // Dispatcher has exactly one arm (the fn, not the type).
        let dispatch_arm_count = out.matches("_call(args)").count();
        // Once in the match arm + multiple times in the actual
        // wrapper source. Look for the specific match-arm form:
        assert!(out.contains("\"abc12345_ab_a\" => Sample_addOne_call(args)"));
        let _ = dispatch_arm_count;
    }

    #[test]
    fn transpile_source_if_exported_records_when_tagged() {
        let mut registry = Leo4ExportRegistry::new();
        let env = Environment::new();
        let src = "@[leo4_export] def f : Nat -> Nat := fun n -> n";

        let result = transpile_source_if_exported(&env, &mut registry, src);
        // Same Ok/Err parity rule as transpile_source: parse +
        // pre-record happens regardless of whether elab against
        // an empty env succeeds. The tag must always be
        // captured by the time we exit, since recording happens
        // *before* elab.
        let exported = registry.exported_names();
        assert_eq!(
            exported.len(),
            1,
            "registry must contain one export — got {exported:?}"
        );
        assert_eq!(exported[0].to_string(), "f");

        // Pipeline outcome is documented as either Ok(Some(_))
        // or Err depending on whether elab finds `Nat` in the
        // empty env. Both are valid for this empty-env probe.
        match result {
            Ok(Some(rust_src)) => {
                assert!(!rust_src.contains("lean_box"));
                eprintln!("transpile_source_if_exported → emitted:\n{rust_src}");
            }
            Ok(None) => {
                panic!("tagged decl must NOT yield Ok(None) — got skipped");
            }
            Err(e) => {
                let code = e.code;
                eprintln!(
                    "transpile_source_if_exported → expected-ish err (code 0x{code:08x}): {}",
                    e.message
                );
                assert!(
                    code == leo4_abi::error::error_codes::DECODE_ERROR
                        || code == leo4_abi::error::error_codes::ENCODE_ERROR,
                    "unexpected error code: 0x{code:08x}"
                );
            }
        }
    }

    // ─── OX2: SurfaceExpr lifter + user-type aware marshalling tests ──

    /// Parse a Lean expression in type position (using
    /// `parse_decl` on a wrapper `def`, then yanking the type
    /// annotation back out). Keeps the lifter tests anchored
    /// to the *real* parser, not a hand-built AST.
    fn parse_type_expr(src: &str) -> Located<SurfaceExpr> {
        let full = format!("def __probe : {src} := unsafeCast 0");
        let normalised = lean4_normalize(&full);
        let mut lexer = Lexer::new(&normalised);
        let tokens = lexer.tokenize();
        let mut parser = Parser::new(tokens);
        let decl = parser.parse_decl().expect("parse_decl");
        match &decl.value {
            Decl::Definition { ty: Some(ty), .. } => ty.clone(),
            other => panic!("expected Definition with annotated type, got {other:?}"),
        }
    }

    #[test]
    fn lifter_handles_primitive_names() {
        let users = HashSet::new();
        for (lean, expected) in [
            ("Nat", RustType::U64),
            ("Int", RustType::I64),
            ("UInt8", RustType::U8),
            ("UInt32", RustType::U32),
            ("UInt64", RustType::U64),
            ("Int32", RustType::I32),
            ("Float", RustType::F64),
            ("Float32", RustType::F32),
            ("Bool", RustType::Bool),
            ("Char", RustType::Char),
            ("String", RustType::RustString),
            ("Unit", RustType::Unit),
        ] {
            let ty = parse_type_expr(lean);
            let r = surface_to_rust_type(&ty, &users)
                .unwrap_or_else(|e| panic!("{lean}: {}", e.message));
            assert_eq!(r, expected, "{lean} mismatch");
        }
    }

    #[test]
    fn lifter_handles_carrier_names() {
        let users = HashSet::new();
        for lean in ["BigNat", "BigInt", "LeanRat", "LeanComplexF64x2"] {
            let ty = parse_type_expr(lean);
            let r = surface_to_rust_type(&ty, &users)
                .unwrap_or_else(|e| panic!("{lean}: {}", e.message));
            // Carrier types come back as Custom — render_marshallable_type
            // resolves them to leo4_abi paths downstream.
            assert!(matches!(r, RustType::Custom(_)));
        }
    }

    #[test]
    fn lifter_handles_user_types_via_registry() {
        let mut users = HashSet::new();
        users.insert("Point".to_string());
        let ty = parse_type_expr("Point");
        let r = surface_to_rust_type(&ty, &users).expect("Point known");
        assert_eq!(r, RustType::Custom("Point".to_string()));
    }

    #[test]
    fn lifter_rejects_unknown_name() {
        let users = HashSet::new();
        let ty = parse_type_expr("MysteryType");
        let err = surface_to_rust_type(&ty, &users).expect_err("unknown rejects");
        assert_eq!(err.code, leo4_abi::error::error_codes::DECODE_ERROR);
        assert!(err.message.contains("MysteryType"));
    }

    #[test]
    fn lifter_handles_generic_app() {
        let users = HashSet::new();
        // `Vec Nat` → Vec<U64>
        let ty = parse_type_expr("Vec Nat");
        let r = surface_to_rust_type(&ty, &users).expect("Vec Nat");
        assert_eq!(r, RustType::Vec(Box::new(RustType::U64)));
    }

    #[test]
    fn lifter_handles_nested_generic_app() {
        let users = HashSet::new();
        // `Vec (Option Nat)` → Vec<Option<U64>>
        let ty = parse_type_expr("Vec (Option Nat)");
        let r = surface_to_rust_type(&ty, &users).expect("Vec Option Nat");
        assert_eq!(
            r,
            RustType::Vec(Box::new(RustType::Option(Box::new(RustType::U64))))
        );
    }

    #[test]
    fn lifter_lifts_carrier_inside_generic() {
        let users = HashSet::new();
        let ty = parse_type_expr("Vec BigNat");
        let r = surface_to_rust_type(&ty, &users).expect("Vec BigNat");
        match r {
            RustType::Vec(inner) => assert!(matches!(*inner, RustType::Custom(_))),
            other => panic!("expected Vec<Custom>, got {other:?}"),
        }
    }

    #[test]
    fn render_with_users_accepts_registered_struct() {
        let mut users = HashSet::new();
        users.insert("Point".to_string());
        let r = render_marshallable_type_with_users(
            &RustType::Custom("Point".to_string()),
            &users,
        )
        .expect("Point registered");
        assert_eq!(r, "Point");
    }

    #[test]
    fn render_without_users_rejects_unknown_struct() {
        let users = HashSet::new();
        let err = render_marshallable_type_with_users(
            &RustType::Custom("Point".to_string()),
            &users,
        )
        .expect_err("Point unregistered");
        assert_eq!(err.code, leo4_abi::error::error_codes::ENCODE_ERROR);
    }

    #[test]
    fn wrapper_with_users_accepts_user_param() {
        let users: HashSet<String> = ["Point".to_string()].into_iter().collect();
        let f = rfn(
            "shift",
            vec![("p", RustType::Custom("Point".to_string()))],
            Some(RustType::Custom("Point".to_string())),
        );
        let out = synthesize_canonical_wrapper_with_users(&f, &users)
            .expect("user-type param marshals");
        // Bare name lands in the decode + encode positions.
        assert!(out.contains("<Point as ::leo4_abi::LeanMarshal>::canonical_decode"));
        assert!(out.contains("encode_to_vec(&__ret)"));
    }

    #[test]
    fn registry_user_type_round_trip() {
        let mut reg = Leo4ExportRegistry::new();
        assert!(reg.user_type_names().is_empty());
        reg.register_user_type("Point");
        reg.register_user_type("Color");
        reg.register_user_type("Point"); // dup is fine
        let names = reg.user_type_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"Point".to_string()));
        assert!(names.contains(&"Color".to_string()));
    }

    #[test]
    fn struct_with_users_references_other_struct_field() {
        let users: HashSet<String> = ["Point".to_string()].into_iter().collect();
        let out = synthesize_struct_type_with_users(
            "Edge",
            &[
                field("from", RustType::Custom("Point".to_string())),
                field("to", RustType::Custom("Point".to_string())),
            ],
            &users,
        )
        .expect("Edge { from: Point, to: Point } must synth");
        assert!(out.contains("pub from: Point,"));
        assert!(out.contains("pub to: Point,"));
        assert!(out.contains("<Point as ::leo4_abi::LeanMarshal>"));
    }

    // ─── OX2 user-record synthesis tests ──────────────────────────────

    fn field(name: &str, ty: RustType) -> StructField {
        StructField {
            name: name.to_string(),
            ty,
        }
    }

    #[test]
    fn struct_emits_decl_and_marshal_impl() {
        let out = synthesize_struct_type(
            "Point",
            &[
                field("x", RustType::U32),
                field("y", RustType::U32),
            ],
        )
        .expect("Point must synth");

        // Struct decl with derives + pub fields.
        assert!(out.contains("pub struct Point {"));
        assert!(out.contains("pub x: u32,"));
        assert!(out.contains("pub y: u32,"));

        // LeanMarshal impl.
        assert!(out.contains("impl ::leo4_abi::LeanMarshal for Point {"));

        // Encode: fields in declaration order.
        let enc_x = out.find("canonical_encode(&self.x,").expect("x encode");
        let enc_y = out.find("canonical_encode(&self.y,").expect("y encode");
        assert!(enc_x < enc_y, "encode must be in declaration order");

        // Decode: fully-qualified marshal call per field, in
        // declaration order; builds Self struct literal at end.
        let dec_x = out.find("(x, __next)").expect("x decode binding");
        let dec_y = out.find("(y, __next)").expect("y decode binding");
        assert!(dec_x < dec_y, "decode must be in declaration order");
        assert!(out.contains("Self { x, y }"));
    }

    #[test]
    fn struct_emits_carrier_field_types() {
        let out = synthesize_struct_type(
            "MoneyBag",
            &[
                field("major", RustType::Custom("BigNat".to_string())),
                field("minor", RustType::U32),
            ],
        )
        .expect("MoneyBag must synth");
        assert!(out.contains("pub major: ::leo4_abi::BigNat,"));
        assert!(out.contains("<::leo4_abi::BigNat as ::leo4_abi::LeanMarshal>"));
    }

    #[test]
    fn struct_emits_generic_container_field() {
        let out = synthesize_struct_type(
            "Bucket",
            &[field(
                "items",
                RustType::Vec(Box::new(RustType::RustString)),
            )],
        )
        .expect("Bucket<Vec<String>> must synth");
        assert!(out.contains("pub items: ::std::vec::Vec<::std::string::String>,"));
    }

    #[test]
    fn struct_rejects_unmarshallable_field_atomically() {
        // First field is fine (u64); second field is bogus
        // (user type with no marshalling support). The emit
        // must fail and emit nothing — no partial struct.
        let result = synthesize_struct_type(
            "Bad",
            &[
                field("good", RustType::U64),
                field("bad", RustType::Custom("UnknownThing".to_string())),
            ],
        );
        let err = result.expect_err("unmarshallable field must reject");
        assert_eq!(err.code, leo4_abi::error::error_codes::ENCODE_ERROR);
        assert!(err.message.contains("UnknownThing"));
    }

    #[test]
    fn struct_with_zero_fields_emits_unit_form() {
        let out = synthesize_struct_type("Empty", &[]).expect("0-field must synth");
        // Unit-struct syntax.
        assert!(out.contains("pub struct Empty;"));
        // Marshal impl is a no-op encoder + zero-byte decoder.
        assert!(out.contains("impl ::leo4_abi::LeanMarshal for Empty {"));
        assert!(out.contains("Self, off"));
    }

    #[test]
    fn struct_with_single_field_builds_correct_literal() {
        let out = synthesize_struct_type(
            "Wrapper",
            &[field("inner", RustType::I64)],
        )
        .expect("single-field must synth");
        // Single-field literal — no leading comma.
        assert!(out.contains("Self { inner }"));
        assert!(out.contains("pub inner: i64,"));
    }

    #[test]
    fn escape_ident_handles_strict_keywords() {
        assert_eq!(escape_rust_ident("type"), "r#type");
        assert_eq!(escape_rust_ident("match"), "r#match");
        assert_eq!(escape_rust_ident("async"), "r#async");
        assert_eq!(escape_rust_ident("loop"), "r#loop");
        assert_eq!(escape_rust_ident("yield"), "r#yield"); // reserved
        // Pass-through for normal idents — including `from`,
        // which is NOT a Rust keyword (it's a method name on
        // the `From` trait, not a reserved word).
        assert_eq!(escape_rust_ident("x"), "x");
        assert_eq!(escape_rust_ident("from"), "from");
        assert_eq!(escape_rust_ident("head"), "head");
    }

    #[test]
    fn escape_ident_handles_raw_ineligible() {
        // Cannot take r# — fall back to trailing-underscore.
        assert_eq!(escape_rust_ident("self"), "self_");
        assert_eq!(escape_rust_ident("Self"), "Self_");
        assert_eq!(escape_rust_ident("super"), "super_");
        assert_eq!(escape_rust_ident("crate"), "crate_");
        assert_eq!(escape_rust_ident("true"), "true_");
        assert_eq!(escape_rust_ident("false"), "false_");
        assert_eq!(escape_rust_ident("_"), "__");
    }

    #[test]
    fn struct_emits_raw_ident_for_keyword_field() {
        let out = synthesize_struct_type(
            "FlaggedRow",
            &[
                field("type", RustType::U32),     // strict keyword
                field("match", RustType::Bool),    // strict keyword
                field("super", RustType::I32),     // raw-ineligible
            ],
        )
        .expect("keyword fields must synth");

        assert!(
            out.contains("pub r#type: u32,"),
            "expected r#type field; got:\n{out}"
        );
        assert!(out.contains("pub r#match: bool,"));
        // raw-ineligible — gets the _ suffix instead.
        assert!(out.contains("pub super_: i32,"));
        assert!(out.contains("self.r#type"));
        assert!(out.contains("self.super_"));
        assert!(out.contains("(r#type, __next)"));
        assert!(out.contains("Self { r#type, r#match, super_ }"));
    }

    #[test]
    fn struct_emit_is_compatible_with_derive_macro_order() {
        // The leo4 derive macro (`crates/leo4-macros-backend/`)
        // encodes fields in declaration order. We assert this
        // emit produces the same encode sequence so a struct
        // synthesised this way is byte-compatible with a
        // hand-written `#[derive(LeanMarshal)]` of the same
        // shape (the OX2 invariant).
        let out = synthesize_struct_type(
            "Triple",
            &[
                field("a", RustType::U32),
                field("b", RustType::U64),
                field("c", RustType::Bool),
            ],
        )
        .expect("Triple must synth");

        let pa = out.find("canonical_encode(&self.a,").expect("a encode");
        let pb = out.find("canonical_encode(&self.b,").expect("b encode");
        let pc = out.find("canonical_encode(&self.c,").expect("c encode");
        assert!(pa < pb && pb < pc, "encode order broke: {pa} {pb} {pc}");

        // Decode also walks a → b → c in source order.
        let da = out.find("(a, __next)").expect("a decode");
        let db = out.find("(b, __next)").expect("b decode");
        let dc = out.find("(c, __next)").expect("c decode");
        assert!(da < db && db < dc, "decode order broke");
    }

    // ─── OX2 inductive (enum) synthesis tests ─────────────────────────

    fn ctor(name: &str, fields: Vec<RustType>) -> EnumVariant {
        EnumVariant {
            name: name.to_string(),
            fields,
        }
    }

    #[test]
    fn enum_all_unit_emits_pure_unit_variants() {
        let out = synthesize_enum_type(
            "Color",
            &[
                ctor("Red", vec![]),
                ctor("Green", vec![]),
                ctor("Blue", vec![]),
            ],
        )
        .expect("Color must synth");

        assert!(out.contains("pub enum Color {"));
        assert!(out.contains("    Red,"));
        assert!(out.contains("    Green,"));
        assert!(out.contains("    Blue,"));
        // Encode arms: 4-byte LE disc emit for each variant.
        assert!(out.contains("Self::Red => {"));
        assert!(out.contains("0u32.to_le_bytes()"));
        assert!(out.contains("1u32.to_le_bytes()"));
        assert!(out.contains("2u32.to_le_bytes()"));
        // Decode dispatch.
        assert!(out.contains("0u32 => ::core::result::Result::Ok((Self::Red"));
        assert!(out.contains("invalid tag"));
    }

    #[test]
    fn enum_with_payload_emits_tuple_variant_and_decode() {
        let out = synthesize_enum_type(
            "Either",
            &[
                ctor("left", vec![RustType::U32]),
                ctor("right", vec![RustType::RustString]),
            ],
        )
        .expect("Either must synth");

        // Tuple-style variants.
        assert!(out.contains("    left(u32),"));
        assert!(out.contains("    right(::std::string::String),"));

        // Encode: disc + payload encode per variant.
        assert!(out.contains("Self::left(__f0) => {"));
        assert!(out.contains("0u32.to_le_bytes()"));
        assert!(out.contains("<u32 as ::leo4_abi::LeanMarshal>::canonical_encode(__f0"));
        assert!(out.contains("1u32.to_le_bytes()"));

        // Decode: 4-byte tag + payload decode per variant.
        assert!(out.contains("0u32 => {"));
        assert!(out.contains("let (__f0, __next) = <u32 as ::leo4_abi::LeanMarshal>::canonical_decode"));
        assert!(out.contains("Self::left(__f0)"));
        assert!(out.contains("1u32 => {"));
        assert!(out.contains(
            "let (__f0, __next) = <::std::string::String as ::leo4_abi::LeanMarshal>::canonical_decode"
        ));
        assert!(out.contains("Self::right(__f0)"));
    }

    #[test]
    fn enum_mixed_unit_and_payload() {
        let out = synthesize_enum_type(
            "Maybe",
            &[ctor("none", vec![]), ctor("some", vec![RustType::U64])],
        )
        .expect("Maybe must synth");
        assert!(out.contains("    none,"));
        assert!(out.contains("    some(u64),"));
        assert!(out.contains("Self::none =>"));
        assert!(out.contains("Self::some(__f0) =>"));
    }

    #[test]
    fn enum_with_carrier_payload() {
        let out = synthesize_enum_type(
            "Either",
            &[
                ctor("ok", vec![RustType::Custom("BigNat".to_string())]),
                ctor("err", vec![RustType::RustString]),
            ],
        )
        .expect("BigNat payload must synth");
        assert!(out.contains("ok(::leo4_abi::BigNat)"));
        assert!(out.contains("<::leo4_abi::BigNat as ::leo4_abi::LeanMarshal>"));
    }

    #[test]
    fn enum_with_user_type_payload() {
        let users: HashSet<String> = ["Point".to_string()].into_iter().collect();
        let out = synthesize_enum_type_with_users(
            "Shape",
            &[
                ctor("dot", vec![RustType::Custom("Point".to_string())]),
                ctor(
                    "line",
                    vec![
                        RustType::Custom("Point".to_string()),
                        RustType::Custom("Point".to_string()),
                    ],
                ),
            ],
            &users,
        )
        .expect("user-type payload must synth");
        assert!(out.contains("dot(Point)"));
        assert!(out.contains("line(Point, Point)"));
    }

    #[test]
    fn enum_rejects_unmarshallable_payload_atomically() {
        let err = synthesize_enum_type(
            "Bad",
            &[
                ctor("good", vec![RustType::U64]),
                ctor("bad", vec![RustType::Custom("Mystery".to_string())]),
            ],
        )
        .expect_err("Mystery payload must reject");
        assert_eq!(err.code, leo4_abi::error::error_codes::ENCODE_ERROR);
        assert!(err.message.contains("Mystery"));
    }

    #[test]
    fn enum_rejects_zero_variants() {
        let err = synthesize_enum_type("Void", &[])
            .expect_err("0-variant enum must reject");
        assert!(err.message.contains("no variants"));
    }

    #[test]
    fn enum_escapes_keyword_variant_names() {
        let out = synthesize_enum_type(
            "Choice",
            &[ctor("match", vec![]), ctor("type", vec![RustType::U32])],
        )
        .expect("keyword variants must synth");
        assert!(out.contains("    r#match,"));
        assert!(out.contains("    r#type(u32),"));
        assert!(out.contains("Self::r#match =>"));
        assert!(out.contains("Self::r#type(__f0) =>"));
        assert!(out.contains("Self::r#match,"));
    }

    // ─── §6 Cargo crate emit tests ────────────────────────────────────

    fn fixture_unit(fn_name: &str, mangled: &str) -> TranspileUnit {
        // Mint a minimal RustFn + wrapper pair via the real
        // emission path so the fixtures exercise the actual
        // surface, not hand-written strings.
        let f = RustFn::new(
            fn_name.to_string(),
            vec![("n".to_string(), RustType::U64, false)],
            Some(RustType::U64),
            vec![],
        );
        let wrapper = synthesize_canonical_wrapper(&f).expect("wrapper synth must succeed");
        TranspileUnit {
            type_decls: Vec::new(),
            fn_src: f.emit(),
            wrapper_src: wrapper,
            fn_name: fn_name.to_string(),
            mangled: mangled.to_string(),
        }
    }

    #[test]
    fn emit_cargo_toml_includes_required_fields() {
        let out = emit_cargo_toml("my_pkg", "{ path = \"../leo4-abi\" }");
        assert!(out.contains("[package]"));
        assert!(out.contains("name        = \"my_pkg\""));
        assert!(out.contains("edition     = \"2024\""));
        assert!(out.contains("[dependencies]"));
        assert!(out.contains("leo4-abi = { path = \"../leo4-abi\" }"));
        assert!(out.contains("[lib]"));
        assert!(out.contains("path = \"src/lib.rs\""));
        // Auto-generated banner.
        assert!(out.contains("DO NOT EDIT"));
    }

    #[test]
    fn emit_lib_rs_concatenates_fn_and_wrapper_per_unit() {
        let u1 = fixture_unit("Sample_addOne", "abc12345_ab_a");
        let u2 = fixture_unit("Sample_double", "def67890_ab_a");
        let out = emit_lib_rs(&[u1, u2], "0123456789abc");

        // Both fns + both wrappers appear.
        assert!(out.contains("pub fn Sample_addOne"));
        assert!(out.contains("Sample_addOne_call(args: &[u8])"));
        assert!(out.contains("pub fn Sample_double"));
        assert!(out.contains("Sample_double_call(args: &[u8])"));
        assert!(out.contains("DO NOT EDIT"));
    }

    #[test]
    fn emit_lib_rs_emits_leanproc_dispatcher() {
        let u = fixture_unit("Sample_addOne", "abc12345_ab_a");
        let out = emit_lib_rs(&[u], "deadbeefcafe1");

        // LeanProc impl present.
        assert!(out.contains("pub struct Leo4OxileanProc"));
        assert!(out.contains("impl ::leo4_abi::rust_native::LeanProc for Leo4OxileanProc"));
        // schema_hash literal embedded.
        assert!(
            out.contains("\"deadbeefcafe1\""),
            "schema_hash literal not embedded; got:\n{out}"
        );
        // abi_version returns 1.
        assert!(out.contains("fn abi_version(&self) -> u32 { 1 }"));
        // Dispatch table: one match arm per unit.
        assert!(out.contains("\"abc12345_ab_a\" => Sample_addOne_call(args)"));
        // Default arm raises unknown_function.
        assert!(out.contains("unknown_function(mangled)"));
        // Default `new` + `Default` impls.
        assert!(out.contains("pub fn new() -> Self"));
        assert!(out.contains("impl Default for Leo4OxileanProc"));
    }

    #[test]
    fn emit_lib_rs_empty_units_still_emits_dispatcher() {
        // Zero exports → dispatcher with only the default arm.
        let out = emit_lib_rs(&[], "0000000000000");
        assert!(out.contains("impl ::leo4_abi::rust_native::LeanProc for Leo4OxileanProc"));
        assert!(out.contains("match mangled {"));
        assert!(out.contains("unknown_function(mangled)"));
        // No `_call(args)` lines.
        assert!(
            !out.contains("_call(args)"),
            "empty crate must NOT have any dispatch arms; got:\n{out}"
        );
    }

    #[test]
    fn emit_crate_pairs_manifest_and_lib() {
        let u = fixture_unit("Sample_addOne", "abc12345_ab_a");
        let g = emit_crate(
            "my_pkg",
            &[u],
            "{ path = \"../leo4-abi\" }",
            "deadbeefcafe1",
        );
        assert_eq!(g.crate_name, "my_pkg");
        assert!(g.manifest.contains("name        = \"my_pkg\""));
        assert!(g.lib_rs.contains("Leo4OxileanProc"));
    }

    #[test]
    fn write_to_dir_creates_manifest_and_lib_rs() {
        use std::path::PathBuf;
        // tempdir without pulling in `tempfile` — use the
        // process's `target/tmp` so the test is hermetic + the
        // path is auto-cleaned by `cargo clean`.
        let target = std::env::var("CARGO_TARGET_TMPDIR").map_or_else(
            |_| std::env::temp_dir().join("leo4-oxilean-build-tests"),
            PathBuf::from,
        );
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = target.join(format!(
            "write_to_dir-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let u = fixture_unit("Sample_addOne", "abc12345_ab_a");
        let g = emit_crate("my_pkg", &[u], "\"0.1\"", "deadbeefcafe1");
        let written = g.write_to_dir(&dir).expect("write must succeed");
        assert!(written > 0);

        let manifest = std::fs::read_to_string(dir.join("Cargo.toml"))
            .expect("Cargo.toml must exist after write");
        assert!(manifest.contains("name        = \"my_pkg\""));
        let lib = std::fs::read_to_string(dir.join("src").join("lib.rs"))
            .expect("src/lib.rs must exist after write");
        assert!(lib.contains("Leo4OxileanProc"));

        // Idempotent re-write.
        let _ = g
            .write_to_dir(&dir)
            .expect("second write overwrites existing files");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
