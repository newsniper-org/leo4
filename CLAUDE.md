# Working Agreement for Claude Code on leo4

> Read this file at the start of every Claude Code session before touching any
> source. Read `LEO4-DESIGN.md` next, then the relevant `SPEC/*.md`.

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
- No `unsafe` outside `leo4-abi`, `leo4-native`, and `leo4-wasm`. Each
  `unsafe` block must have a `// SAFETY:` comment.
- Public API uses `Result<T, LeanError>` for fallible operations.
  `LeanError` is in `leo4::err`.
- Lifetimes on `LeanRef<'a, T>` are NEVER elided. Always written out.
- No `async` in public API until WASIp3 stabilizes (D4).

### Lean

- Lean 4, version pinned in `lean-toolchain`.
- All `@[leo4_export]` attributes go on top-level definitions only.
- Boundary functions return `IO α` or pure `α` for `α : Type 0` only.
- Constraint quotations use the `leo4_constraint` syntax category from
  `Leo4/Syntax.lean`. Do not use raw strings.

### Cross-cutting

- Mangled names are derived, not written. Never hand-type a mangled symbol.
  If you find yourself wanting to, fix `mangle()` instead.
- BLAKE3 hash truncations are `first_8_bytes` then `base32lc`. No exceptions.
- Endianness for canonical ABI is little-endian per WIT spec.

## How to Work With Spec Files

`SPEC/*.md` are normative. Code conforms to them; they do not document code.
If you discover the spec is wrong or ambiguous, change the spec first in a
separate commit, then change the code.

`SPEC/*.md` files in scope:

- `SPEC/idl-grammar.ebnf` — leo4 IDL grammar
- `SPEC/canonical-abi.md` — wire format per type
- `SPEC/mangling.md` — symbol name derivation
- `SPEC/handshake.md` — schema hash and admit-set summary file format

## How to Work With the Lake Plugin

The Lake plugin (`lake/Leo4Plugin/`) hooks into Lake's build process to:

1. Find all `@[leo4_export]` definitions in the user's Lean package.
2. Extract constraints from each.
3. Compute admit-sets via `Lean.Meta.SynthInstance.getInstances` (closed-world
   over the user package + its transitive deps).
4. Emit IDL, WIT (optional), shim source, and handshake files.
5. Drive `cc`/`leanc` to compile the shim and link Lean.

The plugin is itself a Lake target; it builds once per `lean-toolchain` change.

**The hook stability of `Lake.Module.recBuildLean` is the subject of
spike 0** (`spike/SPIKE-0-lake-hook.md`). Do not assume any particular
hook is stable until spike 0 reports its findings.

## How to Work With the macro-on-extern-C Layer

`leo4_macros::import` expands `#[leo4::import(module = "…")]` blocks into:

1. An `extern "C"` block with one entry per admit-set instantiation,
   each with an explicit `#[link_name = "<mangled>"]`.
2. A generic Rust wrapper function that does:
   - `match T::SCALAR_TAG` (or equivalent for non-scalar generics) to
     dispatch to the right `extern "C"` symbol.
   - Canonical-encode arguments via `leo4-abi`.
   - Call the `extern "C"` function.
   - Canonical-decode the return value.
   - Wrap errors in `LeanError`.

The macro reads `target/leo4/<pkg>.leo4-mangling` to know which symbols to
declare. If the file does not exist, the macro emits a build error that
instructs the user to run `lake build` first.

## How to Add a New Boundary Type

1. Update `SPEC/canonical-abi.md` with the wire format.
2. Update `SPEC/mangling.md` with the type's `mangle_type` rule.
3. Add the type to the IDL grammar in `SPEC/idl-grammar.ebnf` if it is a
   built-in.
4. Implement `LeanType` / `LeanMarshal` in `leo4-abi`.
5. Implement encode/decode in `leo4-abi`.
6. Implement the corresponding Lean side in `lake/Leo4/Marshal.lean`.
7. Add a conformance test in `tests/conformance/` that round-trips the type
   through both backends.

## Build Commands Quick Reference

```bash
# From repo root
just build               # runs lake build, then cargo build, in order
just test                # all tests, both sides
just spec-lint           # validates SPEC/*.md consistency
just regen-mangling      # regenerates the mangled-name table
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
- Don't introduce `tokio` or any async runtime before WASIp3 stabilizes (D4).
- Don't merge a PR that changes a mangled name without changing the schema
  hash, or vice versa.
- Don't speculatively add features. leo4 is small on purpose.
