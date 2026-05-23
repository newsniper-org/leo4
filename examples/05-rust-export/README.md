# examples/05-rust-export — Rust → Lean end-to-end demo

Phase 9-7. Exercises every code path of the reverse-direction
pipeline (Phase 9-1 through 9-6) against a tiny Rust
"mini-solver" cdylib. Lean drives, Rust computes.

## Surface

The Rust crate (this directory, top-level `Cargo.toml`)
exposes four `#[leo4::export]` functions:

| Rust signature                              | Wire shape           |
|---------------------------------------------|----------------------|
| `pub fn is_prime(n: u64) -> bool`           | u64 → bool           |
| `pub fn next_prime(n: u64) -> u64`          | u64 → u64            |
| `pub fn count_primes_below(n: u64) -> u64`  | u64 → u64            |
| `pub fn factor_smallest(n: u64) -> Option<u64>` | u64 → option<u64> |

These together cover the v9-5 Lean-wrapper mapping table
(scalar in/out, `Option<T>` return).

## Build + run (4-step manual workflow)

Until Lake-plugin auto-discovery of the glue shim lands (9-6
follow-up), the user runs each step explicitly. Each command
is invocable from the repo root unless noted otherwise.

```sh
# 1. Build the user cdylib + the leo4-rust-bridge static archive.
cargo build --release -p leo4-example-05-rust-export
cargo build --release -p leo4-rust-bridge
cargo build --release -p leo4-rust-worker

CDYLIB=$(realpath target/release/libleo4_example_05_rust_export.so)
BRIDGE=$(realpath target/release/libleo4_rust_bridge.a)
WORKER=$(realpath target/release/leo4-rust-worker)

# 2. Emit the IDL / handshake / Lean wrapper for the cdylib.
mkdir -p examples/05-rust-export/lean/Generated
cargo run --release --quiet -p leo4-rust-emit -- \
  --cdylib       "$CDYLIB" \
  --out-dir      examples/05-rust-export/lean/Generated \
  --emit-lean \
  --lean-module  Leo4ExampleMiniSolverRust.Rust
# Produces inside Generated/:
#   leo4_example_05_rust_export.leo4-rust-exports.idl
#   leo4_example_05_rust_export.leo4-rust-handshake
#   leo4_example_05_rust_export.leo4-rust-imports.lean
# Rename the .lean to match Lake's expected layout:
mv examples/05-rust-export/lean/Generated/leo4_example_05_rust_export.leo4-rust-imports.lean \
   examples/05-rust-export/lean/Leo4ExampleMiniSolverRust/Rust.lean

# 3. Build the Lean-side glue shim (the one place lean.h is allowed).
leanc -c -std=c2x shim/leo4_rust_bridge_lean.c \
   -o examples/05-rust-export/lean/leo4_rust_bridge_lean.o

# 4. Lake-build the Lean side, then link the executable manually.
#    (Lake-plugin auto-link integration is a 9-6 follow-up; until
#    then the manual leanc invocation below works.)
cd examples/05-rust-export/lean
lake build Leo4Example05
leanc \
  .lake/build/lib/Leo4Example05.olean.o \
  .lake/build/lib/Main.olean.o \
  ../../../lake/Leo4/.lake/build/lib/Leo4.olean.o \
  leo4_rust_bridge_lean.o \
  "$BRIDGE" \
  -o leo4Example05
cd ../../..

# Run with the cdylib path + worker binary path pinned.
LEO4_RUST_CDYLIB="$CDYLIB" \
LEO4_RUST_WORKER_BIN="$WORKER" \
LEO4_RUST_HANDSHAKE_PKG=leo4_example_05_rust_export \
LEO4_RUST_HANDSHAKE_IFACE=Leo4ExampleMiniSolverRust \
  ./examples/05-rust-export/lean/leo4Example05
```

The `LEO4_RUST_HANDSHAKE_PKG` / `_IFACE` env vars tell the
worker which package / interface name to feed into its
`schema_hash` recomputation — they must match the values
`leo4-rust-emit` baked into the handshake JSON. (The
default-derived names here are `leo4_example_05_rust_export`
and `Leo4ExampleMiniSolverRust`, computed from the
cdylib stem.)

## Expected output

```
leo4 reverse-direction demo — schema_hash = …

is_prime:
  is_prime(0) = false
  is_prime(1) = false
  is_prime(2) = true
  is_prime(3) = true
  is_prime(4) = false
  is_prime(5) = true
  is_prime(6) = false
  is_prime(7) = true
  is_prime(8) = false
  is_prime(9) = false
  is_prime(97) = true
  is_prime(100) = false
  is_prime(7919) = true
  is_prime(7920) = false

next_prime:
  next_prime(1) = 2
  next_prime(2) = 3
  next_prime(10) = 11
  next_prime(13) = 17
  next_prime(100) = 101
  next_prime(1000) = 1009

count_primes_below:
  count_primes_below(10) = 4
  count_primes_below(100) = 25
  count_primes_below(1000) = 168

factor_smallest:
  factor_smallest(0) = none
  factor_smallest(1) = none
  factor_smallest(2) = some 2
  factor_smallest(15) = some 3
  factor_smallest(49) = some 7
  factor_smallest(97) = some 97
  factor_smallest(999983) = some 999983
```

(Exact format of `Option` / `Bool` strings depends on Lean's
default `Repr`/`ToString`; the numeric values are the
load-bearing checks.)

## Why this demo matters

It is the first commit on the Phase 9 ladder where the
entire pipeline executes end-to-end:

- `#[leo4::export]` proc-macro (9-1) emits per-fn wrapper
  symbols + `linkme` entries.
- `leo4-rust-emit` (9-2) walks the cdylib's `EXPORTS` and
  produces the handshake JSON.
- `leo4-rust-emit --emit-lean` (9-5) generates the typed
  Lean wrapper module.
- `libleo4_rust_bridge.a` (9-4a/b) is the dispatcher; on
  POSIX it spawns `leo4-rust-worker` (9-3) via `posix_spawn`
  and talks to it over `socketpair`.
- `shim/leo4_rust_bridge_lean.c` (9-6) bridges Lean's
  `lean_object*` ABI to the dispatcher's byte-pointer entry.

Every wire (mangled name, IDL form, schema_hash, error
codes) is shared with the forward direction — the cross-impl
mangling harness (`tests/mangling/`) keeps them in lockstep.

## Cargo workspace integration

The Rust side is registered as a workspace member
(`leo4-example-05-rust-export` in the root `Cargo.toml`) so
`cargo build --workspace` builds the cdylib alongside the
other examples. The Lean side is **NOT** a Lake workspace
member of `lake/Leo4/`; it lives next to its `lakefile.lean`
inside this directory and references `lake/Leo4/` via a
relative `require` path.
