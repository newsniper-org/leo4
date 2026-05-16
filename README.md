# leo4

Lean 4 ↔ Rust interop that does not bind the Rust side to a Lean toolchain
version.

## Status

**Pre-implementation.** All major design decisions are resolved.
Phase 0 (Lake hook spike) is the next concrete task.

## Documents to read, in order

1. [`LEO4-DESIGN.md`](LEO4-DESIGN.md) — every design decision and its rationale.
2. [`CLAUDE.md`](CLAUDE.md) — working agreement for Claude Code sessions.
3. [`ROADMAP.md`](ROADMAP.md) — phased work plan.
4. [`spike/SPIKE-0-lake-hook.md`](spike/SPIKE-0-lake-hook.md) — the immediate next step.
5. `SPEC/*.md` — normative specifications:
   - [`SPEC/idl-grammar.ebnf`](SPEC/idl-grammar.ebnf)
   - [`SPEC/canonical-abi.md`](SPEC/canonical-abi.md)
   - [`SPEC/mangling.md`](SPEC/mangling.md)
   - [`SPEC/handshake.md`](SPEC/handshake.md)

## Why leo4 and not leo3

`leo3` is a fine effort, but it compiles against `lean.h` directly. That
makes the Rust crate version-locked to a specific Lean toolchain, and the
lock breaks whenever Lean's internal layout shifts. leo4 puts all Lean ABI
knowledge in a build-time-generated C shim, and exposes only a stable
canonical ABI to the Rust crate. The Rust crate therefore tracks the IDL,
not the Lean toolchain.

See `LEO4-DESIGN.md` §0 for the longer version.

## Layout

```
.
├── LEO4-DESIGN.md          # single source of truth
├── CLAUDE.md               # Claude Code working agreement
├── ROADMAP.md              # phased plan
├── SPEC/                   # normative specs
├── crates/                 # Cargo workspace
├── lake/                   # Lake workspace (Lean side)
├── shim/                   # C shim for the native backend
├── examples/               # end-to-end demos
├── tests/                  # integration + conformance tests
├── spike/                  # disposable experiments + findings
├── Cargo.toml
├── lakefile.lean
├── rust-toolchain.toml
├── lean-toolchain
└── justfile
```

## Build

Once Phase 1 has landed:

```bash
just build       # Lake first, Cargo second
just test
```

Until then, the workspace is a scaffold and `just build` will likely fail
in expected ways. See `ROADMAP.md`.

## License

MIT OR Apache-2.0.
