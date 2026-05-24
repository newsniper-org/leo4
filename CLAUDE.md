# Working Agreement for Claude Code on leo4

> Read this file at the start of every Claude Code session before touching any
> source. Read `LEO4-DESIGN.md` next, then the relevant `SPEC/*.md`.
>
> **Companion doc**: `AGENTS.md` is the *cookbook* — concrete command
> patterns, "what files usually change together?", common pitfalls,
> subagent guidance. This file (`CLAUDE.md`) is the *working
> agreement* — how to behave. Read both.

## Identity of This Project

leo4 is a Lean 4 ↔ Rust interop library. It is **not** a fork of `leo3`. It is
designed from scratch by 병익 with assistance from Claude (web). All major
design decisions are in `LEO4-DESIGN.md` §1; do not reopen them without
written justification in a separate document.

## Stance

- 병익 is a senior engineer who works across hardware, ML, and systems. Match
  that level. Do not over-explain Rust or Lean basics. Do explain decisions
  that involve a tradeoff or a non-obvious choice.
- The default language for human-readable text in commits, comments, and
  documentation is **English**. The default language for chat with 병익 in
  Claude Code is **Korean** (대화는 한국어, 코드/주석/문서/커밋은 영어).
- If a request from 병익 contradicts `LEO4-DESIGN.md`, raise the conflict
  explicitly and ask which takes precedence. Do not silently override either.

## What "Done" Means

For each work item, "done" means all of:

1. Code compiles cleanly: `cargo check`, `cargo clippy -- -D warnings`,
   `lake build` (whichever applies).
2. Tests pass: `cargo test`, `lake test` (whichever applies).
3. Spec is updated if the change affects ABI, IDL, mangling, or handshake.
4. The change has at least one example in `examples/` or test in `tests/`
   exercising it end-to-end if it crosses the boundary.

## Code Conventions

### Rust

- Edition `2024`. MSRV pinned in `rust-toolchain.toml`.
- `#![warn(clippy::pedantic)]` at workspace level, with explicit
  `#![allow(clippy::xxx)]` per crate as needed.
- No `unsafe` outside `leo4-abi`, `leo4-mslean4`, and `leo4-wasm`. Each
  `unsafe` block must have a `// SAFETY:` comment.
- Public API uses `Result<T, LeanError>` for fallible operations.
  `LeanError` is in `leo4::err`.
- Lifetimes on `LeanRef<'a, T>` are NEVER elided. Always written out.
- The public API stays sync on both native and wasm. Lean `IO α`
  exports lift to `future<α>` at the IDL boundary, but the
  generated Rust wrapper is sync — wasm hides WASIp3 async imports
  via `futures::executor::block_on` inside a sync wasm export
  (D4 lift, 2026-05-20). Do not surface `async fn` in user-facing
  Rust API.

### Lean

- Lean 4, version pinned in `lean-toolchain`.
- All `@[leo4_export]` attributes go on top-level definitions only.
- Boundary functions return `IO α` or pure `α` for `α : Type 0` only.
- Constraint quotations use the `leo4_constraint` syntax category from
  `Leo4/Syntax.lean`. Do not use raw strings.

### Cross-cutting

- Mangled names are derived, not written. Never hand-type a mangled symbol.
  If you find yourself wanting to, fix `mangle()` instead.
- The schema hash is FNV-1a-64 over the normalized IDL bytes; the 8 hash
  bytes are emitted big-endian into the mangled name via lowercase base32
  (no padding). Rationale and exact construction: `SPEC/mangling.md` §3.
- Endianness for canonical ABI is little-endian per WIT spec.
- **OS-specific code must live behind an identified abstraction
  layer** — see `OS-PORTABILITY.md` for the policy and the audit
  ledger. New `#[cfg(target_os=…)]` / `cfg(unix)` / `cfg(windows)`
  branches outside an identified layer either get lifted into a
  layer in the same commit or rejected. The Phase 9 spawn / IPC
  layer (`SPEC/reverse-direction.md` §4.4) is the model.

## How to Work With Spec Files

`SPEC/*.md` are normative. Code conforms to them; they do not document code.
If you discover the spec is wrong or ambiguous, change the spec first in a
separate commit, then change the code.

`SPEC/*.md` files in scope:

- `SPEC/idl-grammar.ebnf` — leo4 IDL grammar
- `SPEC/canonical-abi.md` — wire format per type
- `SPEC/mangling.md` — symbol name derivation
- `SPEC/handshake.md` — schema hash and admit-set summary file format
- `SPEC/phase-6-mutual.md` — mutual recursion (Phase 6 design + rules)

## How to Work With the Lake Plugin

The Lake plugin (`lake/Leo4Plugin/`) hooks into Lake's build process to:

1. Find all `@[leo4_export]` definitions in the user's Lean package.
2. Extract constraints from each.
3. Compute admit-sets via `Lean.Meta.SynthInstance.getInstances` (closed-world
   over the user package + its transitive deps).
4. Emit IDL, WIT (optional), shim source, and handshake files.
5. Drive `cc`/`leanc` to compile the shim and link Lean.

The plugin is itself a Lake target; it builds once per `lean-toolchain` change.

**Spike 0 result (2026-05-16, RESOLVED):** the plugin is a
`lean_exe` (`lake exe leo4plugin <user-module>`) that calls
`Lean.importModules (loadExts := true)` on the user package's
already-built `.olean` files. We do **not** hook
`Lake.Module.recBuildLean` (it stayed `private` across v4.27.0 →
v4.30.0-rc2). Full investigation: `spike/SPIKE-0-FINDINGS.md`.

## How to Work With the rust-transpile Path (OxiLean)

`sibling/leo4-oxilean-build` is the no-lake-no-lean
rust-transpile path. Pipeline:

1. Parse the user's Lean source with
   `leo4_lean4_parse::parse_decls` (OX6 PEG-based parser,
   strict superset of oxilean-parse v0.1.2's accepted
   surface).
2. Translate `leo4_lean4_parse::Decl` →
   `oxilean_parse::Decl` via the `leo4_translate` module.
   On `TranslateError::Unsupported`, fall back to the
   legacy oxilean-parse-direct walker.
3. Elaborate against an env bootstrapped by
   `leo4_env_bootstrap::bootstrap_env()` —
   `oxilean_kernel::init_builtin_env` (Bool / Unit /
   Empty / Nat / String / Eq / Prod / List + axioms)
   plus leo4 boundary primitives (UInt8..128, Int8..128,
   Float32/64, Char) as `Declaration::Axiom`. **Zero
   lake/lean overhead.**
4. Lower via `oxilean_codegen::to_lcnf` and emit a Rust
   crate.

The `leo4-parser` cargo feature (default ON since
2026-05-24) selects the leo4-lean4-parse → leo4_translate
path; `--no-default-features` falls back to
oxilean-parse-direct.

## How to Work With the `leo4::import!` Layer

`leo4::import! { fn add(a: u64, b: u64) -> u64; … }` is a
function-procedural macro (`#[proc_macro]` in
`crates/leo4-macros/`, expansion in
`crates/leo4-macros-backend/`). It parses an extern-block-like
input and emits one sync Rust wrapper `fn` per declaration. Each
wrapper:

1. Canonical-encodes arguments via `<T as ::leo4::LeanMarshal>::canonical_encode`.
2. Calls `lean.call_shim(MANGLED_BODY, &args, &mut ret)` — dynamic
   dispatch through `libloading`, not `extern "C"` link-name.
3. Canonical-decodes the return value, wraps errors in `LeanError`.

The macro reads `LEO4_MANGLING_FILE` (set at build time by
`leo4_build::wire(...)` in the user's `build.rs`) to resolve each
fn name + arg-type tuple into a mangled body. For
multi-instantiation generic exports the user can disambiguate with
`#[leo4(args = "u64,str")]` on the `fn` declaration.

## How to Add a New Boundary Type

1. Update `SPEC/canonical-abi.md` with the wire format.
2. Update `SPEC/mangling.md` with the type's `mangle_type` rule.
3. Add the type to the IDL grammar in `SPEC/idl-grammar.ebnf` if it
   is a built-in.
4. Implement `LeanMarshal` (and `LeanResource` if applicable) in
   `crates/leo4-abi/src/…` for the Rust side.
5. Implement the corresponding Lean side in
   `lake/Leo4/Leo4/Builtins.lean` (primitives) or a dedicated
   module under `lake/Leo4/Leo4/` (carrier types).
6. Mirror the type discovery in the plugin
   (`lake/Leo4Plugin/Leo4Plugin/AdmitSet.lean` and `Main.lean`) if
   it surfaces as anything other than a regular record.
7. Add a conformance test in `tests/conformance/` round-tripping
   through both encoders.
8. Add a sample fixture exercising the type end-to-end — usually
   `tests/sample-lean/Sample.lean` + one of the examples.

## Build Commands Quick Reference

```bash
# From repo root
just build               # runs lake build, then cargo build, in order
just test                # all tests, both sides
just spec-lint           # validates SPEC/*.md consistency
just smoke-plugin        # rebuilds shim + emits .leo4-{schema,mangling,handshake}
just mathlib-bridge-test # type-checks Mathlib bridges (off the default ladder)
just clean               # nukes target/ and .lake/

# Lake-only
cd lake && lake build leo4plugin
cd lake && lake build Leo4

# Cargo-only
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

## When Stuck

- If the issue is about Lean's internals (Lake API, Elab, Meta), check the
  `lean-toolchain` version's source first: `~/.elan/toolchains/<version>/src/lean/`.
- If the issue is about Rust macro hygiene, prefer `proc_macro2` + `syn` +
  `quote` patterns established in `leo4-macros-backend/`.
- If the issue is about the canonical ABI, the authoritative reference is
  the WIT spec at https://github.com/WebAssembly/component-model/.
- If you genuinely don't know whether something belongs in spec vs. code,
  ask 병익. Don't guess.

## What Not to Do

- Don't import `lean.h` from Rust. Ever. That defeats the entire point of leo4.
- Don't add a dependency on `bindgen` to any Rust crate. The shim absorbs all
  Lean ABI details.
- Don't introduce `tokio` or any other async runtime in the main
  workspace. WASIp3 async lives inside the `sibling/leo4-wasip3/`
  project behind `futures::executor::block_on`; the public Rust API
  stays sync on every target (D4 lift, 2026-05-20).
- Don't merge a PR that changes a mangled name without changing the schema
  hash, or vice versa.
- Don't speculatively add features. leo4 is small on purpose.
