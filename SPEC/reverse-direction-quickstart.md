# Reverse direction — Quickstart

> Phase 10-E3 (2026-05-21). Companion to the normative
> `SPEC/reverse-direction.md`. This page is **user-facing** — it
> shows you how to *use* the reverse direction, not how it's
> implemented. For the wire-level contracts, mangling rules, and
> error-code reservations, jump to `SPEC/reverse-direction.md`.

## In one sentence

leo4's reverse direction lets Lean call into Rust: tag a Rust
function with `#[leo4::export]`, run `leo4 run`, and Lean
gets a typed `IO α` wrapper that invokes your Rust code over
an IPC dispatcher.

## The 60-second tour

```bash
# 1. Scaffold a new reverse-direction project.
leo4 create reverse my-solver
cd my-solver

# 2. Add a Rust function.
cat > src/lib.rs <<'EOF'
#[leo4::export]
pub fn add(a: u64, b: u64) -> u64 { a + b }
EOF

# 3. Run end-to-end. `leo4 run` builds the cdylib, emits
#    the Lean wrapper, builds the executable, and invokes
#    it with the right env matrix.
leo4 run
```

That's it. The Lean side's `Main.lean` (also scaffolded)
imports the generated wrapper and calls `← add 2 3`.

## What `leo4 run` actually does

The flow is documented in detail in `SPEC/reverse-direction.md`
§7 (Build Orchestration). The short version:

1. `cargo build --release -p <crate>` produces a cdylib.
2. `lake run Leo4Rust/regenerate` (Phase 10-D2) invokes
   `leo4-rust-emit` against the cdylib, emitting:
   - `<crate>.leo4-rust-exports.idl`
   - `<crate>.leo4-rust-handshake` (JSON; carries the
     `schema_hash` + `abi_version`)
   - `<iface>/Rust.lean` (typed wrappers)
3. `cd lean && lake build` links the `lean_exe`, picking up
   the dispatcher archive + glue shim automatically through
   the `Leo4Rust` Lake package's `extern_lib`s.
4. The resulting binary is invoked with the env matrix
   (`LEO4_RUST_CDYLIB`, `LEO4_RUST_WORKER_BIN`,
   `LEO4_RUST_HANDSHAKE_PKG`, `LEO4_RUST_HANDSHAKE_IFACE`).

## Supported wire types (today)

Anything that has a `LeanMarshal` impl on the Rust side
works as a `#[leo4::export]` parameter or return type:

- scalars: `u8` `u16` `u32` `u64` `i8` `i16` `i32` `i64` `f32` `f64` `bool` `char`
- strings: `String`
- bignums: `BigInt`, `BigNat`
- generics: `Vec<T>`, `Option<T>`, `Result<T>`,
  `(T1, T2, …)` for any supported `T`s
- nominal: any record/variant/enum/resource that
  `#[derive(LeanMarshal)]` accepts — **no Lean mirror
  module needed.** RC.2 (2026-05-31) closed end-to-end
  user-type mirror emission: `#[derive(LeanMarshal)]`
  publishes a `USER_TYPES` entry, `leo4-rust-emit`
  reads it, and the generated `<iface>/Rust.lean`
  carries a real `structure` / `inductive` decl with a
  `deriving Leo4.LeanMarshal` clause. So
  `#[leo4::export] pub fn solve(v: AdsmtVerdict) -> u64`
  works without any hand-written `lean/Adsmt/Verdict.lean`.
- `Option<T>` / `Result<T, E>` return types

What's not yet supported:

- **Function-arrow parameters** (`fn(T) -> R`): IDL +
  mangling shipped in Phase 10-B1; inbound + outbound
  runtime substrate landed Phase 10-B1.x (2026-05-28),
  with the OxiLean transpile-path IO walker closure
  on 2026-05-31 (#76 P0c) covering `IO.bind` /
  `@[extern]` / monad transformer family /
  canonical-ABI arg encoding / stdlib `IO.println` +
  `IO.FS.*` direct dispatch on the fork branch
  `0.1.3-leo4-ox7`. mslean4-path callback dispatch
  via worker IPC `LECQ` / `LECR` frames remains
  v1.x.
- **`async fn` exports**: deferred to ≥ v1.x.
- **Cross-process resource handles** (passing a Rust
  `LeanRef<'a, T>` across the worker boundary): deferred.

## Two isolation modes

Default is one long-running worker per cdylib --- fast,
keeps state across calls (the SMT-solver use case wants
this).

For security-sensitive workloads, opt in to per-call
isolation:

```rust
#[leo4::export(isolated)]
pub fn evaluate_untrusted_code(s: String) -> String { … }
```

The dispatcher routes each call through a fresh worker
process; the worker `_exit`s after the call returns. No
wire-format or API change --- just an internal `iso:`
prefix on the mangled name. See
`SPEC/reverse-direction.md` §4.2.

## Recycle policies (long-running mode)

If the long-running worker accumulates state you'd rather
bound, configure recycle policies via env. Independent
limits; whichever fires first wins.

```bash
# Restart the worker after every 1000 calls.
export LEO4_RUST_WORKER_RECYCLE_CALLS=1000

# Restart the worker after 60 seconds of uptime.
export LEO4_RUST_WORKER_RECYCLE_SECONDS=60
```

When a recycle fires, the next request transparently uses
the fresh worker. Callers that need to detect recycle
events (because they care about state loss) can poll
`leo4_rust_bridge_take_restart_flag` --- see
`SPEC/reverse-direction.md` §4.3 (Phase 10-A4 / A5).

## Common pitfalls

- **"Garbage status values" on first call.** The
  dispatcher missed the worker's 25-byte handshake. Fix:
  upgrade to the post-2026-05-23 `libleo4_rust_bridge.a`
  (the consume-handshake call lives there).
- **`LEO4_ERR_HANDSHAKE_MISMATCH` (0x05).** The
  `LEO4_RUST_HANDSHAKE_PKG` / `_IFACE` env values don't
  match what `leo4-rust-emit` used. Either re-run
  `leo4 run` (it sets them automatically) or check that
  your scaffolded scaffold's `LEO4_RUST_HANDSHAKE_IFACE`
  matches the iface name in your Lean wrapper module.
- **"cdylib not found"** from `lake run
  Leo4Rust/regenerate`. The script searches
  `<project>/target/release/lib<crate>.{so,dylib,dll}`
  first, then walks upward to handle cargo-workspace
  projects whose `target/` lives at the workspace root.
  If neither finds it, set `LEO4_RUST_CDYLIB` explicitly.
- **Lake exe name mismatch.** Your scaffold's
  `lean_exe <crate_name>` produces a binary at
  `lean/.lake/build/bin/<crate_name>`. If you renamed
  `lean_exe`, update `leo4 run`'s exe lookup via
  `--bin <name>`.

## Where to go next

- **Production demo**: a full SMT-solver integration using
  the reverse direction lives at
  [`Honey-Be/adsmt`](https://github.com/Honey-Be/adsmt).
- **Mini example in-repo**: `examples/05-rust-export/`.
- **Normative SPEC**: `SPEC/reverse-direction.md`.
- **Architectural rationale**: `LEO4-DESIGN.md` D16.
- **Phase ladder**: `ROADMAP.md` Phase 9.
