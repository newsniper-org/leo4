# leo4

Lean 4 ↔ Rust interop that does not bind the Rust side to a Lean
toolchain version.

## Status

**Phase 1 (Lean side, end-to-end) complete.** Phase 2 (Rust-side
`leo4-idl` + cross-impl mangling conformance) is the next concrete
task. See [`ROADMAP.md`](ROADMAP.md) for the full phase ladder.

What works today:

- `lake/Leo4/` runtime library exposing `@[leo4_export]`,
  `@[leo4_specialize_when …]`, the `leo4_constraint` syntax category,
  `class LeanMarshal` (`canonicalEncode`/`canonicalDecode` over a
  `ByteArray`), `class LeanResource` + `@[leo4_resource]`, primitive
  blanket `LeanMarshal` instances, and a `deriving LeanMarshal`
  handler for `structure` / `inductive` (including self-recursive
  variants).
- `lake/Leo4Plugin/` Lake plugin built as a `lean_exe`. Walks the user
  package via `Lean.importModules (loadExts := true)`, finds every
  `@[leo4_export]`, computes admit-sets (phantom / unbounded / class /
  value-erased — see LEO4-DESIGN.md §5), mangles names per
  [`SPEC/mangling.md`](SPEC/mangling.md), and atomically writes
  `<pkg>.leo4-schema`, `<pkg>.leo4-mangling`, `<pkg>.leo4-handshake`.
- `tests/sample-lean/` smoke fixture covering primitives, scalar
  generics, class-constraint generics, phantom generic,
  value-erased generic, user record / enum / variant / resource,
  self-recursive variant.

What is **not** built yet:

- Rust side `leo4-idl` parser + mangling, the cross-impl conformance
  harness (`tests/mangling/`), and the `leo4c` CLI — Phase 2.
- `<pkg>.wit` lowering — Phase 3.
- Wire-format round-trip on the Rust side and `LeanError.*` runtime
  paths — Phase 4.
- The C shim, `cc`/`leanc` invocation, `crates/leo4-native/`,
  `#[leo4::import]` macro, end-to-end examples — Phase 5.

The current state ships in `lake/`. The Rust workspace in `crates/` is
still a scaffold.

## Documents to read, in order

1. [`LEO4-DESIGN.md`](LEO4-DESIGN.md) — every design decision and its
   rationale (D1–D15, type-system layer, admit-set algorithm,
   forbidden constructs).
2. [`CLAUDE.md`](CLAUDE.md) — working agreement for Claude Code
   sessions in this repo.
3. [`ROADMAP.md`](ROADMAP.md) — phased work plan, exit criteria per
   phase, the deferred IDL-output-grouping decision.
4. [`spike/SPIKE-0-FINDINGS.md`](spike/SPIKE-0-FINDINGS.md) — why the
   plugin re-imports `.olean` rather than hooking
   `Lake.Module.recBuildLean`.
5. `SPEC/*.md` — normative specifications:
   - [`SPEC/idl-grammar.ebnf`](SPEC/idl-grammar.ebnf) — IDL grammar
     (WIT-superset, `kind`, `Self`/`Self<…>`, value-param)
   - [`SPEC/canonical-abi.md`](SPEC/canonical-abi.md) — wire format
   - [`SPEC/mangling.md`](SPEC/mangling.md) — name mangling, schema
     hash (FNV-1a-64 → base32lc), kind discipline
   - [`SPEC/handshake.md`](SPEC/handshake.md) — JSON file formats and
     atomic-emission contract

## Why leo4 and not leo3

`leo3` is a fine effort, but it compiles against `lean.h` directly.
That makes the Rust crate version-locked to a specific Lean toolchain,
and the lock breaks whenever Lean's internal layout shifts. leo4 puts
all Lean ABI knowledge in a build-time-generated C shim, and exposes
only a stable canonical ABI to the Rust crate. The Rust crate
therefore tracks the IDL, not the Lean toolchain.

See `LEO4-DESIGN.md` §0 for the longer version.

## Layout

```
.
├── LEO4-DESIGN.md          # single source of truth
├── CLAUDE.md               # Claude Code working agreement
├── ROADMAP.md              # phased plan
├── SPEC/                   # normative specs
├── crates/                 # Cargo workspace (scaffold; Phase 2+)
├── lake/                   # Lake workspace (Lean side, Phase 1 complete)
│   ├── Leo4/               # runtime library
│   └── Leo4Plugin/         # Lake plugin exe
├── shim/                   # C shim for the native backend (Phase 5)
├── examples/               # end-to-end demos (Phase 5)
├── tests/                  # integration + conformance tests
│   ├── sample-lean/        # Phase 1 smoke fixture
│   └── mangling/           # cross-impl conformance harness (Phase 2)
├── spike/                  # disposable experiments + findings
├── Cargo.toml
├── lakefile.lean
├── rust-toolchain.toml
├── lean-toolchain          # pinned: leanprover/lean4:v4.29.1
└── justfile
```

## Build and smoke-test

Lean toolchain pinned to **`leanprover/lean4:v4.29.1`**. The repo does
not require `elan`; the system-installed Lean of that version works.

Common tasks (run from repo root):

```bash
just                    # list available recipes
just plugin-build       # build the Lake plugin (and Leo4)
just sample-build       # build tests/sample-lean
just smoke-plugin       # run the plugin against the sample, emit
                        # tests/sample-lean/.lake/build/leo4/leo4-sample.{leo4-schema,leo4-mangling,leo4-handshake}
just schema-hash        # print the sample's resolved schema hash
just clean              # nuke build outputs

just build              # Lake first, Cargo second (CLAUDE.md D8)
just test               # full test ladder (Phase 4+ will populate it)
```

The `just build` and `just test` targets run Cargo too, which is
currently a near-empty workspace; they succeed but do little. Phase 2
fills `crates/leo4-idl/` and `crates/leo4c/`.

After `just smoke-plugin` the emitted files describe `tests/sample-lean`'s
IDL in canonical form. Examples of what the plugin produces (cropped):

```
$ cat tests/sample-lean/.lake/build/leo4/leo4-sample.leo4-schema
package leo4-sample;
interface Sample {
  enum Sample.Color { red, green, blue };
  record Sample.Point { x: f64, y: f64 };
  variant Sample.Tree { leaf, node(Self, Self) };
  resource Sample.ParserHandle;
  func add(_0: u64, _1: u64) -> u64;
  func parserId(_0: Sample.ParserHandle) -> Sample.ParserHandle;
  func pointSum(_0: Sample.Point) -> f64;
  ...
}
```

```
$ just schema-hash
7vi56qcxzb3xw
```

(Schema hashes rotate every time the canonical IDL form changes — by
design, so a stale Rust binary linking against a fresh shim fails at
link time. The exact value above corresponds to the current sample
fixture; yours will rotate as soon as you edit `tests/sample-lean`.)

## License

MIT OR Apache-2.0.
