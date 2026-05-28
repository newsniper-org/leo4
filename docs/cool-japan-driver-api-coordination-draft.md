# cool-japan/oxilean — `oxilean_runtime::driver` API coordination draft

> Status: **DISCUSSION-ONLY DRAFT** (2026-05-28). NOT a PR
> request. Posted to the cool-japan/oxilean tracker as a
> design discussion before any body-level patch lands
> upstream.
>
> Companion to the OX7 / OX8 contribution series at
> `docs/cool-japan-upstream-pr-draft.md`. That doc covers
> the codegen folds + the `oxilean-parse-peg` donation —
> things that are functionally complete on the fork side
> and ready for a real PR. This doc covers a different
> ask: **API surface coordination** for a new module that
> didn't exist in v0.1.3 upstream. We want maintainer
> input on the shape *before* committing to a body
> implementation that has to live with whatever signature
> upstream eventually accepts.

## What we want to discuss (and what we don't)

Discussion in scope:

  - The proposed `oxilean_runtime::driver` module's
    **public API shape**: function signatures, error type,
    resolver injection model, args-passing convention.
  - The dependency direction (`oxilean-runtime` consumes
    `oxilean-kernel` already; this module also consumes
    the existing `extern_resolver::ExternResolver`).
  - Whether this module is the right home for the
    capability vs. a new sibling (`oxilean-driver`?
    `oxilean-cli/driver/`?).

Discussion **NOT** in scope (yet):

  - The body of the IO walker — `IO.bind` traversal,
    `EStateM Error IO.RealWorld α` lowering, builtin
    dispatch through `FunctionEntry::builtin`. Those land
    as a separate PR series once the API contract is
    settled.
  - Async / multi-threaded extensions. v0 driver is
    strictly synchronous, single-threaded (matches the
    Lean runtime invariant).

## Why this needs API coordination

leo4 has a downstream consumer (`sibling/leo4-oxilean-
runner`) that *already* calls the proposed API. The body
of `run_main_with_args` returns `DriverError::NotYetImplemented`
on every input today; leo4 surfaces that as a clean
"upstream-blocked, everything else wired" error so
end-users can distinguish it from real failures.

Two paths forward:

  - **Fork-only commitment** — leo4 keeps the
    `0.1.3-leo4-ox7` fork branch and ships its own body.
    Workable but means leo4's first-class
    `--impl rust-transpile` path stays divergent from
    cool-japan's upstream indefinitely.
  - **Upstream coordination** — agree on the API shape
    with cool-japan first, then the body PR lands on
    upstream and the fork drops the divergence. This is
    the preferred path; this doc starts it.

## Proposed API surface

### `pub fn run_main(env, resolver, name) -> Result<(), DriverError>`

Convenience wrapper around `run_main_with_args` with an
empty program-args slot:

```rust
pub fn run_main(
    env: &Environment,
    resolver: SharedExternResolver,
    main_name: &Name,
) -> Result<(), DriverError>;
```

Drives `def <name> : IO α := …` (or `def <name> : IO
Unit := …`) to its IO effects under the installed
`resolver`. Result discarded (α may be any type; the
runtime cares only that effects fire).

### `pub fn run_main_with_args(env, resolver, name, args) -> Result<(), DriverError>`

The longer form, for `def main : List String → IO α`:

```rust
pub fn run_main_with_args(
    env: &Environment,
    resolver: SharedExternResolver,
    main_name: &Name,
    args: &[&str],
) -> Result<(), DriverError>;
```

`args` is `&[&str]` (not `&[String]`) so callers can pass
`&["arg1", "arg2"]` without allocating; the driver
internally lifts them into Lean's `List String`
representation at evaluation time. Empty `args` defers to
the no-arg `main : IO α` shape — `run_main` is sugar for
`run_main_with_args(env, resolver, name, &[])`.

### `pub enum DriverError`

```rust
pub enum DriverError {
    NotFound { name: String },
    NotADefinition { name: String, kind: &'static str },
    NotYetImplemented { reason: String },
    ExternFailed(oxilean_kernel::ffi::ExternCallError),
}
```

Four arms, all `Debug` + `Display` + `std::error::Error`:

  - **`NotFound`** — `env.get(main_name)` returned `None`.
    Either the wrong name was passed or the elaboration
    + check_declaration loop hasn't reached the `def`
    yet.
  - **`NotADefinition`** — the named decl is an `Axiom`,
    `Theorem`, or `Opaque`. `main` must be a regular
    `Definition`. `kind` is the surface label of the
    rejected variant.
  - **`NotYetImplemented`** — v0 walker doesn't recognise
    the reduced expression shape yet. The body PR
    reduces this arm's surface as more reductions land.
    A clean signal that "the gap is *this shape*, not
    *everything*".
  - **`ExternFailed`** — an `@[extern]` callback the
    walker dispatched returned an error. Carries the
    upstream `ExternCallError` verbatim.

### Resolver injection

`SharedExternResolver` is the existing
`Arc<dyn ExternResolver>` from
`oxilean_runtime::extern_resolver`. No new trait, no new
trait-object box; the driver just takes ownership of
the `Arc` (via `Arc::clone` internally) so the resolver
outlives any recursive IO-walker frame it spawns.

`#[allow(clippy::needless_pass_by_value)]` on the
function signature acknowledges that we accept the
`Arc` by value rather than by reference — this is
intentional: it makes the embedder side
(`leo4-oxilean-runner`) reasonable in the presence of
move-only resolver wrappers, and the internal `Arc::clone`
is cheap.

## What's already in the fork (for reference)

Fork commit `f9bfd45` (2026-05-28) lands the module +
the public surface above with a stub body. Fork commit
`8b2af9f` (2026-05-28) lands the v0 walker — `IO.pure`
shape only — which is enough to make the API contract
exercise itself in a real test:

```rust
let mut env = Environment::new();
env.add(Declaration::Definition { /* main : IO Unit := IO.pure () */ })?;
let resolver: SharedExternResolver = Arc::new(MyResolver::new());
oxilean_runtime::driver::run_main(&env, resolver, &Name::str("main"))?;
```

That call walks to completion on the fork today. Every
other IO shape returns `DriverError::NotYetImplemented`
with a debug repr of the unrecognised expression.

## Questions for cool-japan

1. **Module home.** Is `oxilean_runtime::driver` the right
   place, or would `oxilean-cli/src/driver.rs` (alongside
   the existing `commands::check_source`) be a better fit?
   We picked `runtime` because it owns the `ExternResolver`
   trait and `dispatch_extern_const` helper, which the
   walker will call into.
2. **Naming.** `run_main` reads naturally to us; would
   cool-japan prefer `execute` / `eval_main` / `drive` /
   something else for consistency with existing
   `oxilean_runtime` surface?
3. **`NotYetImplemented` vs panic.** v0 surfaces shapes
   the walker can't yet reduce as a `DriverError` arm
   rather than `unimplemented!()` — letting downstream
   handle the gap gracefully. Is this acceptable, or
   would cool-japan prefer the walker be panic-on-
   incomplete with a separate "complete?" predicate?
4. **`SharedExternResolver` by value vs reference.** We
   take `Arc<…>` by value (then `Arc::clone` internally).
   Embedders generally don't care; consistency with the
   rest of `oxilean-runtime`'s API conventions would tell
   us which to prefer.
5. **`args: &[&str]` vs `&[String]`.** No allocation
   pressure at the call site, but the driver has to lift
   into a Lean `List String` anyway. Open to either.
6. **Error visibility on `ExternFailed`.** We thread the
   `ExternCallError` through verbatim. Should the driver
   also tag it with the offending decl name for
   downstream debugging, or stay purely transparent?
7. **Body PR sequencing.** Once the API is settled, would
   cool-japan prefer one large body PR or a sub-phased
   series (`IO.pure` → `IO.bind` → builtin dispatch →
   `EStateM` lowering)?

Happy to revise based on maintainer feedback; the fork's
`0.1.3-leo4-ox7` branch is the canonical reference for the
proposed API shape today.

---

*Companion docs in the leo4 repo (linkable if useful):*

  - `docs/cool-japan-upstream-pr-draft.md` — the OX7 / OX8
    contribution PR draft (codegen folds +
    `oxilean-parse-peg` donation + extern-resolver hooks).
    This driver coordination is a sibling discussion, not
    a subset.
  - `docs/p0b-byte-packing-design.md` — leo4's
    function-arrow callback ABI design. The driver's IO
    walker will need to fire callback dispatch through
    the resolver at the points described there; the API
    shape proposed in this doc is already shaped to
    accommodate that.
  - `docs/ox8-1-leo4-oxilean-audit.md` — original audit
    that identified the driver gap as the OX8 closure
    blocker.
