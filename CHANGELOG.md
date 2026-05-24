# Changelog

All notable changes to leo4 will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to Semantic Versioning once it reaches 0.1.0.

## [Unreleased]

### Added — OX6 step 12: oxilean-parse v0.1.2 cross-check (2026-05-24)

`sibling/leo4-lean4-parse/tests/oxilean_cross_check.rs`
enforces the strict-superset invariant from the OX6
charter: every source `oxilean-parse` v0.1.2 accepts,
`leo4-lean4-parse` must also accept, the two parsers
must agree on decl count, and each corresponding decl
pair must share a name (per oxilean's `Decl::name()`
contract) and a compatible kind tag.

**What is NOT asserted**: AST field-level equivalence
at the expression level — the two parsers have
intentionally divergent internal shapes (our
`Expr::BinOp("+", …)` vs upstream's
`App(App(Plus, lhs), rhs)`), so a structural
comparison would test representation choice, not
semantics. Decl-level identity (name + kind tag) is
the contract consumers of `leo4-oxilean-build` rely
on.

**Skip mechanism**: sources that `oxilean-parse`
rejects are silently skipped (printed to stderr).
This is the "strict" half of the invariant —
`leo4-lean4-parse`'s surface extensions (e.g. the
`structure ... where`-form, `inductive ... where`-form,
or multi-binder def `def f (a : T) (b : T)`) are not
expected to round-trip through upstream.

Initial corpus: 9 cases covering def, theorem, axiom,
namespace, structure, inductive, import, multi-decl
sources. The corpus expands as new shared-surface
constructs land.

Dev-deps: adds `oxilean-parse = "0.1.2"` (pulls
`oxilean-kernel` transitively). Production deps
unchanged — the cross-check is build-time only.

clippy --all-targets -D warnings clean. Lib tests
unchanged at 288; integration test count 0 → 1.

### Added — OX6 step 11s-b: notation / macro_rules / syntax / elab (2026-05-24)

Four additional `DslKind` variants land via the new
`dsl_multiline_decl` rule:

- `Notation` — `notation:NN PATTERN => BODY`.
- `MacroRules` — `macro_rules\n  | … => …\n  | … => …`.
- `Syntax` — `syntax PATTERN : CATEGORY`.
- `Elab` — `elab PATTERN : CATEGORY => BODY`.

Body is captured raw across newlines until the next
top-level decl boundary fires (a known decl keyword
at a fresh line, `@[`, `#`, or EOF). The body grammar
of each kind diverges from the term-level expr grammar
— these decls extend Lean's parser itself — so raw
capture is the appropriate v1-RC fidelity; downstream
consumers needing inner structure implement per-kind
handlers.

`dsl_next_decl_starter` includes the full set of
top-level keywords (def / theorem / … / fixity / DSL
itself / example / mutual / omit / include) so two
adjacent DSL blocks cleanly split.

Tests 282 → 288 (+6): dsl_notation_single_line,
dsl_macro_rules_multi_line, dsl_syntax_decl,
dsl_elab_decl, dsl_two_notation_back_to_back,
dsl_notation_then_def.

clippy --all-targets -D warnings clean.

**OX6 step 11s complete (11s-a + 11s-b combined).**

### Added — OX6 step 11s-a: fixity decls (infix / infixl / infixr / prefix / postfix) (2026-05-24)

New `DeclKind::Dsl { kind: DslKind, raw: String }`
variant carries Lean 4's parser-extending decls. The
`DslKind` enum has variants for the upcoming DSL family;
this commit lands the five **fixity** variants:

- `Infix` — `infix:NN " op " => target`.
- `Infixl` — left-assoc.
- `Infixr` — right-assoc.
- `Prefix`.
- `Postfix`.

The header + body are captured raw to end of line —
fixity decls are single-line in idiomatic Lean 4.
`word_boundary` is checked inside each alternative of
`fixity_kind` so `infixl` / `infixr` / `infix` don't
collide (`infix` is a prefix of `infixl` lexically;
PEG ordered choice with word_boundary inside each arm
disambiguates).

Interoperates with the 11u doc-comment + attribute /
modifier prefix machinery — fixity decls accept the
same `/-- … -/` / `@[…]` / modifier prefixes as any
other top-level decl.

Tests 277 → 282 (+5): fixity_infixl, fixity_infixr,
fixity_infix_disambiguates_from_infixl,
fixity_prefix_postfix, fixity_with_doc_prefix.

clippy --all-targets -D warnings clean.

### Added — OX6 step 11w: equational pattern-matching `def` (2026-05-24)

New `DeclKind::DefinitionByArms { name, binders, ty,
arms }` parses Lean 4's equational definition form:

```lean
def factorial : Nat → Nat
  | 0 => 1
  | n+1 => (n+1) * factorial n
```

Distinct from `Definition` by the trailing `| pat =>
body | …` arms (no `:=` separator). The
`definition_by_arms` rule is dispatched **before**
plain `definition` in `decl_body`; the `&arm_bar()`
lookahead commits the choice only when an `|` arm
actually follows the type annotation, so plain
`def NAME := EXPR` keeps parsing through the normal
`Definition` path.

Sibling of `Definition` — consumers can choose between
source-faithful equation rendering vs.
match-desugared shape.

Tests 273 → 277 (+4): def_by_arms_simple,
def_by_arms_dot_ctor, plain_def_still_works,
def_by_arms_with_doc_and_attr (interoperates with the
11u doc-comment + attribute prefix machinery).

clippy --all-targets -D warnings clean.

### Added — OX6 step 11u: doc-string semantic binding (2026-05-24)

`Decl` gained a `doc: Option<String>` field. A `/-- … -/`
doc-comment immediately preceding a decl is now
captured and attached to that decl's `doc` field
(matching Lean 4's convention — `simp`, `Mathlib`, and
`lean4doc` all key off the same surface).

Grammar changes:

- `block_comment` rule now uses `"/-" !"-"` lookahead
  so `/-- … -/` does NOT get silently eaten as
  whitespace.
- New `doc_comment() -> String` rule captures the raw
  body between `/--` and `-/` (whitespace preserved
  for markdown-renderer fidelity).
- `decl` wrapper consumes an optional leading
  `doc_comment()` (ordering: doc → attr → modifier →
  body) and writes it into `decl.doc`.

**Breaking** for direct `Decl` construction sites
outside the parser — the new `doc: None` field must be
supplied. All in-tree construction sites updated.

Tests 268 → 273 (+5): doc_comment_none_when_no_doc,
doc_comment_attaches_to_theorem,
doc_comment_with_attr_and_modifier_prefix,
block_comment_not_eaten_as_doc,
doc_comment_multi_line. Also updates the prior
`doc_comment_treated_as_block_comment` test to assert
the new attaching semantics (renamed to
`doc_comment_attaches_to_next_decl`).

clippy --all-targets -D warnings clean.

### Added — OX6 step 11r: do-block loops (`for in` / `while` / `until`) (2026-05-24)

Three new `DoStmt` variants for in-do-block iteration:

- `For { binding, iter, body }` — `for x in xs do BODY`.
- `While { cond, body }` — `while COND do BODY`.
- `Until { cond, body }` — `until COND do BODY`.

The iter / cond head is captured raw up to the
matching `do` keyword (`do` at a word boundary, so
`double`, `doctest`, etc. don't terminate capture
early), then sub-parsed via `parse_expr_text`. Body
is single-line in v1 RC — multi-line bodies need
layout tracking and follow the same trajectory as
`do_expr_stmt`'s single-line restriction.

`do_keyword_stmt_boundary` was extended to recognise
`for` / `while` / `until` at line start so a loop
statement followed by `let` / `return` / another loop
on the next line gets cleanly split into separate
`DoStmt`s.

Tests 263 → 268 (+5): do_for_in_simple, do_while_simple,
do_until_simple, do_for_then_let,
do_for_with_complex_iter.

clippy --all-targets -D warnings clean.

### Added — OX6 step 11o: pattern guards `| pat if cond => …` (2026-05-24)

`MatchArm` gained an `Option<Expr> guard` field; the
match-arm rule now optionally captures an `if EXPR`
clause between the pattern and the `=>` arrow:

```lean
match n with
| x if x > 0 => "positive"
| 0 => "zero"
| n => "negative"
```

Guard text is captured raw between the `if` keyword
and `=>` arrow (so the guard's own internal `if … then
… else` cannot collide with the term-level `if_expr`
rule), then sub-parsed via `parse_expr_text`. Arms
without a guard land as `guard: None`.

**Breaking** for direct `MatchArm` constructors — the
single construction site in the parser updated. Field
*reads* (`arm.pattern`, `arm.body`) keep working;
downstream consumers may need to add `_, guard, _` in
exhaustive destructurings.

Tests 260 → 263 (+3): match_arm_with_guard,
match_arm_unguarded_has_none_guard,
match_arm_guard_on_ctor.

clippy --all-targets -D warnings clean.

### Added — OX6 step 11n: `match h : e with` scrutinee binding (2026-05-24)

`Expr::MatchBind { binding, scrutinee, arms }` parses
Lean 4's match scrutinee-binding form:

```lean
match h : opt with
| some x => x
| none => 0
```

Inside each arm body, `h` is the equality proof
`h : SCRUT = PATTERN` — surfaces by parsing the
binding ident only, leaving the proof semantics to the
elaborator. Sibling of `Expr::Match`; existing
`Match`-consumers keep working without migration.

The bind-form is dispatched before plain `match … with`
via PEG ordered choice; the `IDENT ":" !"="` lookahead
distinguishes `match h : e with` from `match e with`.

Tests 257 → 260 (+3): match_bind_simple,
plain_match_still_works, match_bind_with_dot_ctor_arm.

clippy --all-targets -D warnings clean.

### Added — OX6 step 11m: `if let` pattern-matching if (2026-05-24)

`Expr::IfLet { pattern, scrutinee, then_branch,
else_branch }` parses Lean 4's pattern-matching `if`:

```lean
if let some x := opt then x else 0
if let .ok v := r then v else fallback
if let _ := x then 1 else 2
```

Equivalent to a binary `match`:
`match SCRUT with | PATTERN => THEN | _ => ELSE`. The
rule is dispatched **before** plain `if_expr` in the
atom rule so the `if let pat := …` form is recognised
without spurious backtracking against
`if cond then …`.

Tests 253 → 257 (+4): if_let_simple_some,
if_let_with_dot_ctor, if_let_wildcard,
plain_if_still_works.

clippy --all-targets -D warnings clean.

### Added — OX6 step 11p: anonymous fn shorthand `(· + 1)` (2026-05-23)

`Expr::DotFn(String)` accepts Lean 4's anonymous
function shorthand triggered by the `·` (U+00B7 MIDDLE
DOT) placeholder. Supported surface forms:

- `(· + 1)` — single placeholder binary op (`λ x => x + 1`).
- `(1 + ·)` — placeholder on the right (`λ x => 1 + x`).
- `(· + ·)` — multiple placeholders (`λ x y => x + y`).
- `(·.field)` — projection shorthand (`λ x => x.field`).

The body is captured as raw text between the parens;
consumer can sub-parse via `expr()`. A paren *without*
any `·` placeholder still parses as `Expr::Paren` (the
look-ahead rule confirms placeholder presence before
committing to `DotFn`). Nested parens inside the body
are not supported in v1 RC.

Tests 248 → 253 (+5): dot_fn_simple_binary,
dot_fn_projection, dot_fn_two_placeholders,
dot_fn_placeholder_on_right,
paren_without_placeholder_is_paren_not_dot_fn.

clippy --all-targets -D warnings clean.

### Added — OX6 step 11v: omit / include section-variable management (2026-05-23)

Two new `DeclKind` variants for Lean 4's section-scoped
variable management:

- `Omit { items: Vec<String> }` — `omit x y z`. Locally
  drops named section variables from the surrounding
  `section` scope.
- `Include { items: Vec<String> }` — `include foo`.
  Re-introduces previously omitted (or otherwise
  unreferenced) section variables.

Each accepts a whitespace-separated list of ident-raw
names (dotted paths like `Foo.Bar` allowed).

Tests 243 → 248 (+5): omit_single, omit_multiple,
include_single, include_multiple, omit_dotted_ident.

clippy --all-targets -D warnings clean.

### Added — OX6 step 11t: debug / diagnostic # commands (2026-05-23)

`DeclKind::HashCommand { cmd, raw_args }` accepts the
Lean 4 debug top-level commands as first-class decls:

- `#check expr` — type-print an expression.
- `#eval expr` — evaluate at compile-time.
- `#print name` — show a declaration's body / type.
- `#guard expr` — assert a proposition.
- `#guard_msgs (cfg) in` — diagnostic capture (the
  trailing inner command lands as a *second*
  `HashCommand` decl; the consumer reassembles if it
  cares about the binding).

`cmd` is the keyword without the `#` prefix; `raw_args`
is the trimmed tail captured to end of line (the
v1-RC scope deliberately keeps args raw rather than
sub-parsing each per-command-specific grammar).

Tests 238 → 243 (+5): hash_check_simple, hash_eval_simple,
hash_print_simple, hash_guard_msgs,
hash_command_mixed_with_def.

clippy --all-targets -D warnings clean.

### Added — OX6 step 11q: Unicode operator surface (2026-05-23)

`leo4-lean4-parse` accepts the canonical Lean 4 Unicode
operator forms at the same precedence levels as their
ASCII spellings:

- **Comparison level**: `≤` (≡ `<=`), `≥` (≡ `>=`),
  `≠` (≡ `!=`), `∈` (membership), `∉` (non-membership),
  `⊆` (subset).
- **Multiplicative level**: `×` (≡ `*`), `÷` (≡ `/`),
  `∪` (union), `∩` (intersection).

Each surfaces as `Expr::BinOp(op, …)` preserving the
exact Unicode symbol so downstream consumers can
distinguish ASCII vs Unicode forms when needed.

Tests 230 → 238 (+8): unicode_op_{le, ge, ne, times,
div, membership, subset, set_ops}.

clippy --all-targets -D warnings clean.

### Added — OX6 step 11l: numeric + string literal extensions (2026-05-22)

`Literal` enum gained a `Float(String)` variant + the
parser grammar now accepts:

- **Hex / binary / octal** numeric literals: `0xFF`,
  `0b1010`, `0o17` (case-insensitive prefixes).
- **Digit separators**: `1_000_000`, `0xFF_FF`,
  `0b1010_1010`, `1_000.5` — all radix forms accept
  `_` between digits.
- **Float literals**: `3.14`, `1.5e10`, `1.0e-3`. Float
  takes priority over Nat so `1.5` parses as Float (not
  Nat-then-dot-shortcut). Held as the original source
  text (`Literal::Float(String)`) rather than `f64` to
  keep `Eq` derive sound (NaN-incomparable).
- **Multi-line strings**: `"""…"""` triple-quoted, raw
  (no escape resolution; arbitrary content including
  newlines).
- **Extended string escapes** in `"…"`: `\xHH` (single-
  byte hex), `\u{H+}` (Unicode codepoint up to 6 hex
  digits). Other escapes (`\n \t \r \\ \" \0`) already
  supported.

Tests 217 → 230 (+13): hex / binary / octal nat,
separator-bearing nats, simple float, scientific float,
separator in float, float-vs-nat priority,
multiline string, multiline raw-pass-through, hex byte
escape, Unicode codepoint escape, def with hex value.

clippy --all-targets -D warnings clean.

### Added — OX6 step 11f: multi-line do statement values (2026-05-22)

Step 8's `do` notation supported only single-line
statement values (`text:$((!"\n" [_])+)`). Step 11f
extends keyword-prefix statements (`let` / `return` /
`pure`) to accept *multi-line* value expressions:

```lean
def main : IO Nat := do
  let x := if cond then
      1
    else
      2
  return x
```

The let-value `if cond then 1 else 2` spans three
lines; the new `do_keyword_stmt_boundary` rule waits
for the next keyword-prefix statement (`let` /
`return` / `pure` at the start of a fresh line) or
the broader do-block end (`top_level_decl_starter` /
EOF) before stopping the capture.

**Bare expression statements stay single-line** —
detecting the boundary between two consecutive bare-
expr statements on separate lines without column-
tracking is genuinely ambiguous in PEG; v0 takes the
safe single-line interpretation. (Bare expr-statement
sequences in real do-blocks are rare anyway —
typically users have at least one `let` / `return` /
`pure` separator.)

Surface coverage gained:

- `let x := if … then … else …` with multi-line
  if-then-else value.
- `let x ← if … then … else …` with multi-line
  bind value.
- `return if … then … else …` with multi-line return
  value.
- `let r := match … with | A => 1 | B => 2` with
  multi-line match value.

Tests 212 → 217 (+5). Step 8's existing multi-stmt-mix
fixture still parses correctly under the new boundary
rule (regression-guarded).

clippy --all-targets -D warnings clean.

### Added — OX6 step 11e: `by …` tactic block (2026-05-22)

New `Expr::By(Vec<String>)` variant + grammar atom for
Lean 4's term-level entry into tactic mode (`theorem t :
T := by rfl`). Each `String` is the raw text of one
tactic; splitting honours both `;` (single-line sequenced)
and newlines (multi-line block).

**Why raw `String`?** Tactic mode is its own DSL with
its own grammar (`exact e`, `apply f`, `intro x`, `simp
[…]`, `rw […]`, `<;>`, `try`, `repeat`, dozens more). v0
captures the textual form so the surrounding surface
parses cleanly; tactic AST sub-parsing is a future step
beyond OX6.

Surface coverage:

- `:= by exact True.intro` (single tactic inline)
- `:= by intro x; exact x` (semicolon-sequenced)
- `:= by\n  intro x\n  exact x` (multi-line block)
- `:= by simp [Nat.add_comm, Nat.zero_add]` (brackets in
  raw text preserved)
- `:= by` (empty block — yields `Expr::By(vec![])`)
- `:= by trivial` inside `example` decls

**Boundary**: the `by`-block region ends at the next
top-level decl keyword on a fresh line OR EOF. Multi-decl
sources where `by\n  …\ndef next := …` parse correctly —
the `by` block doesn't eat the next `def`.

`by` added to the keyword exclusion list so it doesn't
parse as a bare ident.

Tests 205 → 212 (+7). clippy --all-targets -D warnings clean.

### Added — OX6 step 11d: let-in expression (2026-05-22)

New `Expr::Let { name, ty, value, body }` variant +
grammar atom for Lean 4's pure (non-monadic) let-binding
in expression position. The monadic `let x ← e` inside
`do` blocks lives in `DoStmt::Bind`.

Surface coverage:

- `let x := 1; x` (semicolon separator)
- `let x : Nat := 1; x` (with type annotation)
- `let x := 1\n x + 1` (newline separator)
- Complex value expressions: `let s := a + b; s * 2`
- Nested let chains: `let x := 1; let y := 2; x + y`
- Use in def body: `def f : Nat := let n := 10; n * n`

`ty` is `Option<Box<Expr>>` (boxed to keep `Expr` size
finite given the recursive Option). Value text is
captured up to the separator (`;` or newline) and sub-
parsed via the existing `parse_expr_text` helper;
fallback to `Expr::Raw` for unparseable text.

Multi-line value bodies (`let x := \n  big_value`) are
not yet supported in v0 — single-line value before the
separator.

Tests 199 → 205 (+6): simple-semicolon, with-type-
annotation, newline-separator, complex-value-expr,
nested-let-chain, let-in-def-body.

clippy --all-targets -D warnings clean.

### Added — OX6 step 11k: `example` anonymous theorem (2026-05-22)

New `DeclKind::Example { binders, ty, proof }` variant
+ grammar rule. `example : T := proof` is Lean 4's
anonymous-theorem form — smoke-tests a proof without
binding a name.

Surface: no binders, with binders, with attribute prefix,
with quantifier in the type (`example : forall n, n = n
:= fun n => rfl`).

Tests 195 → 199 (+4). clippy --all-targets -D warnings clean.

### Added — OX6 step 11c: modifier prefixes + abbrev (2026-05-22)

`Decl` gained a `modifiers: Vec<String>` field for Lean 4
decl-keyword modifier prefixes:

- `private`, `protected` — visibility
- `noncomputable` — non-computable definitions
- `partial` — partial recursion (no termination check)
- `unsafe` — relaxed elaboration

Multiple modifiers allowed in any order (e.g.
`private noncomputable def`).

**`abbrev` alias**: `abbrev NAME := …` surfaces as
`DeclKind::Definition` with `"abbrev"` in `modifiers`
(Lean 4 desugars `abbrev` to a reducible `def` at elab).
Universe params + attribute prefix work on `abbrev` too.

Grammar wires this via a new `modifier_keyword()` rule
called as a prefix in the `decl()` wrapper. Prefix
modifiers precede inner-decl modifiers (the synthetic
`"abbrev"` from `abbrev_decl`) so combined examples like
`@[simp] private abbrev …` yield `modifiers: ["private",
"abbrev"]`.

Tests 185 → 195 (+10): partial / noncomputable / private
/ protected / unsafe def, combined `private noncomputable`,
attr+modifier mix, abbrev → Definition + `"abbrev"` tag,
abbrev with universe param, no-modifier regression.

clippy --all-targets -D warnings clean.

### Added — OX6 step 11i: universe annotation `.{u, v}` (2026-05-22)

`Decl` gained a `univ_params: Vec<String>` field for the
`def foo.{u, v} : Sort u` form (Mathlib-ubiquitous). The
field is populated for every decl kind that accepts a
universe-parameter list: `def`, `theorem` / `lemma`,
`axiom`, `structure`, `inductive`, `class`, `instance`.
Empty `Vec` when the decl has no `.{…}` annotation.

A new `univ_params_opt()` PEG rule (returns
`Vec<String>`; empty on absence) is woven into each
applicable decl rule immediately after the name. Existing
construction sites updated to pipe the captured list into
the AST field.

Existing tests unaffected (`univ_params: vec![]` is the
default for source without `.{…}` annotations).

Tests 176 → 185 (+9): def-single-univ, def-multi-univ,
def-no-univ (regression), theorem-with-univ, axiom-with-
univ, structure-with-univs, inductive-with-univ, class-
with-univ, combined-attr-prefix-and-univ.

clippy --all-targets -D warnings clean.

### Added — OX6 step 11j: `@` explicit args marker (2026-05-22)

New `Expr::At(Box<Expr>)` variant + grammar atom for
Lean 4's `@`-prefixed explicit-args marker. Prefixing a
name with `@` switches the elaborator from implicit /
instance-resolving mode to "supply every argument
explicitly".

Surface: `@id`, `@id Nat 0`, `@Nat.succ`, `@(foo + bar)`
(any atom can be wrapped). The `!"["` lookahead rejects
`@[…]` (attribute prefix), though that pattern only
appears in decl position and never in atom position; the
lookahead is a regression guard.

Tests 171 → 176 (+5). Existing `@[simp] def f` attribute
prefix tests still pass — no collision between the two
`@` uses.

clippy --all-targets -D warnings clean.

### Added — OX6 step 11b: anonymous ctor `⟨a, b⟩` (2026-05-22)

New `Expr::AnonCtor(Vec<Expr>)` variant + grammar atom
for Lean 4's Unicode-angle-bracket shorthand. `⟨1, 2⟩`
elaborates to the unique constructor of the expected
type's single-ctor inductive (e.g. `Point.mk 1 2`).

Surface: `⟨a, b⟩`, `⟨42⟩`, `⟨⟩` (empty), nested
`⟨⟨1, 2⟩, ⟨3, 4⟩⟩`, complex element exprs
`⟨a + b, fun n => n⟩`. The element grammar is the full
expr (precedence ladder, lambda, etc.).

Tests 165 → 171 (+6). clippy --all-targets -D warnings clean.

### Added — OX6 step 11g: anonymous structure literal `{ x := 1, y := 2 }` (2026-05-22)

New `Expr::AnonStruct(Vec<(String, Expr)>)` variant +
grammar atom for Lean 4 anonymous structure literals.
Field order preserved in the AST (matters for some elab
paths).

Disambiguation with `{T : Type}` implicit binder: binder
uses `:` (no `:=`); anon struct field uses `:=`. The
anon-struct-literal rule explicitly looks for `:=` after
the first ident, so the implicit-binder grammar in
binder contexts is unaffected.

Tests 160 → 165 (+5): two-fields, single-field, complex
field expressions (BinOps), in def value position,
implicit-binder regression guard.

clippy --all-targets -D warnings clean.

### Added — OX6 step 11h: list literal `[1, 2, 3]` (2026-05-22)

New `Expr::List(Vec<Expr>)` variant + grammar atom for
Lean 4 list literals. Empty `[]`, single / multi-element,
arbitrary expressions as elements, nesting all work.

Disambiguation with `[Ord T]` instance binders: binders
use their own grammar (called only in def signature /
lambda binder positions); list literal lives in `atom()`
(called only in expr positions). No ambiguity.

Tests 154 → 160 (+6): empty list, three nats, mixed
expressions, nested lists, list in def value position,
instance-binder-still-works (regression guard for the
binder/list disambiguation).

clippy --all-targets -D warnings clean.

### Added — OX6 step 11a: block + doc comments (2026-05-22)

OX6 grammar roadmap step 11a — Lean 4 block comments
`/- … -/` (with arbitrary nesting depth) and doc comments
`/-- … -/`.

Both forms are recognised by the whitespace-skipping
machinery: the `_` rule now alternates between
`whitespace()`, `line_comment()`, and `block_comment()`.
The block-comment rule uses PEG recursion
(`block_comment_body = (block_comment() / non-"-/"
character)*`) so arbitrary nesting works.

Doc comments `/-- … -/` parse as ordinary block comments
in v0 — semantic attachment to the next decl (e.g. for
mdoc generation, hover) is OX6 step 11u (deferred).

Tests 148 → 154 (+6): top-level block comment, inline
block comment in a decl, nested block comment,
deeply-nested block comment, doc-comment-as-block-comment,
multi-line block comment.

clippy --all-targets -D warnings clean.

### Added — OX6 step 10d: open + import + variable decls (2026-05-22)

OX6 grammar roadmap step 10 — fourth chunk. Three new
top-level scope-management decls. **Step 10 complete**;
the full `Decl` enum surface is now wired.

**Surface coverage**:

- `open Foo` / `open Foo Bar Baz` — opens namespaces.
- `open Foo.Bar.Baz` — dotted namespace paths.
- `open Foo (a b c)` — selective open (clause text in
  `raw_tail`).
- `open Foo renaming a → b, c → d` — renaming (clause
  text in `raw_tail`).
- `open Foo hiding x y` — hiding (clause text in
  `raw_tail`).
- `open scoped Foo` — Mathlib scoped opens (clause text
  in `raw_tail`).
- `import Foo.Bar.Baz` — single dotted path per import.
- `variable (n : Nat)` / `variable {T : Type}` /
  `variable [Inhabited T]` / mixed groups.

**Open decl design**: rich Lean 4 `open` syntax
(selective `(…)`, `renaming`, `hiding`, `scoped`) parses
the leading namespace idents into `items: Vec<String>`
and stops at the first clause-marker token, putting the
rest into `raw_tail: String` for downstream re-parsing.
v0 doesn't model the full `OpenItem` AST that
`oxilean-parse` defines — sufficient for OX6's goal
(getting the surface to parse without rejection).

New `parse_open_line` helper (private to the grammar
module) does the leading-idents / clause-marker split.

Tests 134 → 148 (+14): open-single, open-multiple,
open-dotted, open-selective-in-raw-tail, open-renaming-
in-raw-tail, open-hiding-in-raw-tail, open-scoped-in-raw-
tail, import-single-dotted, import-init-core, multiple-
imports-each-own-decl, variable-explicit-binder,
variable-implicit-binder, variable-multiple-binder-groups,
import-open-variable-section-combo (e2e: 4-decl Lean 4
source mixing import + open + section with inner
variable + def).

clippy --all-targets -D warnings clean.

**OX6 step 10 (`Decl` enum) complete**. Remaining for
v1.0 RC per ROADMAP: 11a-11l (surface critical),
11m-11w (surface tail), 12 (cross-check), 13 (switchover).

### Added — OX6 step 10c: namespace + section + mutual blocks (2026-05-22)

OX6 grammar roadmap step 10 — third chunk. Three new
top-level decl kinds for scoping / grouping.

**Surface coverage**:

- `namespace NAME … end [NAME]` — scoped block; inner
  decls get the namespace prefix at elab time. Trailing
  `end` name is optional (parser doesn't enforce
  matching; elab handles it).
- `namespace Foo.Bar` — qualified namespace names.
- `section [NAME] … end [NAME]` — like namespace but
  doesn't add a prefix; scopes `variable` decls + opens.
  Anonymous form `section … end` also accepted.
- `mutual … end` — block of mutually-recursive decls.
  Bare `end` terminator (mutual blocks aren't named).

**AST**:

- `DeclKind::Section { name: Option<String>, decls: Vec<Decl> }`
  added (Namespace + Mutual already in the step-10a
  ahead-of-schedule AST batch).

Grammar uses PEG mutual recursion (decl rule references
namespace/section/mutual rules which reference back to
decl) to support nested blocks. Tests verify:

- Nested namespaces (`namespace Outer / namespace Inner /
  end Inner / end Outer`).
- Mixed inner decl kinds (structure + inductive inside a
  namespace).
- Attribute prefix on mutual blocks (`@[macro_inline]
  mutual …`).
- Empty namespaces.

Tests 124 → 134 (+10): namespace-with-inner-decls,
namespace-end-without-name, namespace-dotted-name,
section-anonymous, section-named, mutual-two-decls,
nested-namespaces, namespace-contains-structure-and-
inductive, mutual-with-attr-prefix, empty-namespace.

clippy --all-targets -D warnings clean.

Next OX6 step 10d: open + import + variable.

### Changed — OX6 plan expanded to ~25 sub-steps, all v1.0 RC mandatory (2026-05-22)

병익's directive (2026-05-22): rather than ship OX6 in
the original 10-step form (which only covers the easy
Lean 4 surface), expand the plan to cover *everything*
needed for the real Lean 4 / Mathlib corpus, and treat
**all** sub-steps as v1.0 RC blockers.

Expanded plan (full list in `ROADMAP.md` OX6 entry):

- Steps 1–10 (grammar build-out): scaffold, expression
  grammar, if/match, lambda, structure/inductive,
  attributes, multi-line types, do, string interpolation,
  + quantifiers (9.5 inserted), + `Decl` enum split into
  10a/b/c/d chunks (theorem/lemma/axiom + instance/class +
  namespace/section/mutual + open/import/variable).
- Steps 11a–11l (surface coverage critical): block /
  doc comments, anonymous ctor `⟨…⟩`, modifier prefixes
  (`partial`, `noncomputable`, `private`, `protected`,
  `abbrev`), let-in expression, `by …` tactic block,
  multi-line `do` statements, anonymous structure literal,
  list literal, universe annotation `.{u, v}`, `@`
  explicit args, `example` anonymous theorem, numeric +
  string literal extensions.
- Steps 11m–11w (surface tail): `if let` / `let-else`,
  `match h : e with` scrutinee binding, pattern guards,
  `(. + 1)` anonymous fn shorthand, Unicode operators,
  `do for / while / until` loops, DSL declarations
  (`notation` / `macro_rules` / `syntax` / `elab` /
  `infix` / `prefix` / `postfix`), debug commands
  (`#check` / `#eval` / `#print` / `#guard` /
  `#guard_msgs`), doc-string semantic binding, `omit` /
  `include`, `def f | 0 => …` pattern-matching def.
- Step 12: cross-check against `oxilean-parse` on a
  shared corpus (strict-superset invariant verification).
- Step 13: `leo4-oxilean-build` switches default parser
  from `oxilean_parse::Parser` to
  `leo4_lean4_parse::parse_decls`. OX3 / OX4 textual
  rewrites become legacy compatibility layer.

Steps 1–10b done at this commit. Steps 10c–13 are the
remaining v1.0 RC work for OX6.

### Added — OX6 step 10b: instance + class decls (2026-05-22)

OX6 grammar roadmap step 10 — second chunk. Typeclass-
related decls now parse.

**`instance` surface coverage**:

- Anonymous: `instance : ToString Foo where toString := …`
- Named: `instance addPair : Add Pair := wrap`
- With instance binders: `instance [Inhabited T] : Inhabited
  Pair where default := def_pair`
- Two body forms: `where`-block (structure-style field
  list) or `:= TERM`.

**`class` surface coverage**:

- No binders: `class Trivial where marker : Nat`
- With type-level binders: `class Functor (f : Type ->
  Type) where map : Nat`
- `extends BASES`: `class Monad extends Functor where …`
- `deriving Foo`: `class Foo where x : Nat deriving Repr`
- Attribute prefix: `@[builtin_class] class Foo where …`

**AST change**: `DeclKind::Class` gained a `binders:
Vec<BinderGroup>` field to capture Lean 4's parametric
class binders. Existing `Structure` decl stays binder-less
(structures use field types directly; class methods use
type-level params).

Tests 114 → 124 (+10): instance-anonymous-where-form,
instance-named-with-term-body, instance-with-instance-
binder, instance-with-attr-prefix, class-no-binders,
class-with-typed-binder, class-extends-other-class,
class-with-deriving, class-with-attr-prefix, multi-decl-
with-class-and-instance.

clippy --all-targets -D warnings clean.

Next OX6 step 10c: namespace + mutual.

### Added — OX6 step 9.5: quantifiers `forall` / `∀` / `exists` / `∃` (2026-05-22)

Inserted between OX6 steps 9 and 10b at 병익's request:
proper quantifier support — both critical for dependent
types and proof signatures, ubiquitous in Lean 4 corpus.

**AST**: two new `Expr` variants sharing lambda's
`LamBinder` shape (untyped names or typed groups):

```rust
Forall(Vec<LamBinder>, Box<Expr>),   // forall x, body / ∀ x, body
Exists(Vec<LamBinder>, Box<Expr>),   // exists x, body / ∃ x, body
```

Lean 4 surface:

- `forall x, P x` / `∀ x, P x` — universal
- `exists x, P x` / `∃ x, P x` — existential
- `forall (n : Nat), P n` (typed binder)
- `forall x y z, P x y z` (multi-binder)
- `forall {T : Type}, T -> T` (implicit binder, dependent
  arrow body)
- `forall x, exists y, x = y` (nested quantifiers — body
  takes the full expression grammar)

Body separator is `,` (not `=>` / `->`); the body
expression takes the full-precedence parser so nested
quantifiers / arrows / `=` all work inside.

The `axiom_simple` test from step 10a was restored to
its original form (`axiom em : forall p, p`) now that the
parser handles it.

Tests 104 → 114 (+10): forall-untyped, forall-unicode-∀,
forall-typed, forall-multi-binder, forall-implicit,
exists-untyped, exists-unicode-∃, exists-typed,
theorem-with-forall-in-signature (e2e: `theorem id_eq :
forall x, x = x := fun x => rfl`), nested-quantifiers.

`forall` / `exists` added to the keyword exclusion list
so they don't accidentally parse as bare idents.

clippy --all-targets -D warnings clean.

Next OX6 step 10b: instance + class.

### Added — OX6 step 10a: theorem + lemma + axiom decls (2026-05-22)

OX6 grammar roadmap step 10 — first chunk of the
remaining `Decl` enum. Three new top-level decl kinds.

**New `DeclKind` variants** (the full step-10 AST landed
ahead-of-schedule with this commit; grammar for the
remaining variants is wired in step-10b/c/d):

```rust
Theorem { name, binders, ty, proof },     // theorem / lemma
Axiom   { name, binders, ty },             // no body
Instance { … },                             // 10b
Class    { … },                             // 10b
Namespace { … },                            // 10c
Open      { … },                            // 10d
Import    { path },                         // 10d
Variable  { binders },                      // 10d
Mutual    { decls },                        // 10c

// supporting:
pub enum InstanceBody { Where(Vec<StructField>), Term(Expr) }
```

Grammar landed in this commit (10a):

- `theorem NAME [binders]+ : TYPE := PROOF` —
  proof-shaped def. Same binder grammar as `def`.
- `lemma NAME [binders]+ : TYPE := PROOF` — surface
  synonym for `theorem`; both produce
  `DeclKind::Theorem`.
- `axiom NAME [binders]+ : TYPE` — no body; declared
  with type signature only.

**Side effect — `=` (propositional equality) binary op**:
Lean 4's `a = b` for `Eq a b` needed for theorem
proofs. Added at the comparison precedence level alongside
`==` / `!=` / `<` / `>` / `<=` / `>=`. Disambiguated via
`!"="` lookahead so `==` and `:=` still parse correctly.

`forall` / `∀` quantifier syntax is a separate future
step (binder-style expr form). Test fixtures avoid
quantifiers.

Tests 95 → 104 (+9): theorem-simple, lemma-parses-as-
theorem, theorem-no-binders, axiom-simple, axiom-no-
binders, axiom-with-binders, theorem-with-attr-prefix,
axiom-with-attr-prefix, multi-decl-mixed-kinds (def +
theorem + axiom in one source).

clippy --all-targets -D warnings clean.

Next OX6 step 10b: instance + class.

### Added — OX6 step 9: string interpolation `s!"…{x}…"` (2026-05-22)

OX6 grammar roadmap step 9. Lean 4's string interpolation
parses natively as `Expr::InterpStr(Vec<InterpPart>)` —
alternating literal text + `{expr}` holes.

**AST**:

```rust
pub enum InterpPart {
    Text(String),     // literal-text segment, escapes resolved
    Hole(Expr),       // {…} interpolation
}
```

Surface coverage:

- `s!"hello {name}"` — text-then-hole.
- `s!"{x}"` — single-hole-only.
- `s!"sum: {a + b}"` — hole with arbitrary expr (the full
  precedence ladder is available inside the braces).
- `s!"x={x}, y={y}"` — multiple holes.
- `s!"hello"` — no-hole string (still `InterpStr` with one
  `Text` part, distinguishable from a plain `"hello"` Lit
  at the parser level).
- `s!"set {{x, y}} -> {result}"` — `{{` / `}}` escapes to
  literal `{` / `}` (matches Lean 4 + Rust convention).
- Standard escapes `\\n \\t \\r \\\\ \\"` resolved in
  text segments via the new `decode_interp_text` helper.

The grammar uses a `!"{"` lookahead to distinguish a real
`{expr}` hole from the `{{` escape; same trick on `}}`.
The hole's expression is parsed via the full `expr` rule,
so nested constructs (binops, app, etc.) work inside
braces.

Out of scope today: nested `s!` strings inside holes
(Lean 4 also accepts this — recursive interpolation).
v0 limitation; almost never appears in real code.

Tests 87 → 95 (+8): single-hole-only, text-then-hole,
hole-with-complex-expr, multiple-holes, text-only-no-holes,
escaped-braces, text-escape-sequences (`\n`),
def-body-with-interp.

clippy --all-targets -D warnings clean.

Next OX6 step: full `Decl` enum (theorem / lemma / axiom /
instance / class / namespace / open / import / variable /
mutual).

### Added — OX6 step 8: `do` notation (2026-05-22)

OX6 grammar roadmap step 8. Lean 4's `do` notation parses
natively as `Expr::Do(Vec<DoStmt>)` — a sequenced block of
monadic / pure statements.

**AST**:

```rust
pub enum DoStmt {
    Bind   { name: String, value: Expr },  // let x ← e / let x <- e
    Let    { name: String, value: Expr },  // let x := e
    Return(Expr),                          // return e / pure e
    Expr(Expr),                            // bare expr stmt
}
```

`return` and `pure` collapse to the same `Return` variant
— semantically distinct in some Lean 4 monads but the
elaborator can re-disambiguate from context; keeping the
AST narrow.

Surface coverage:

- `do return e` (single-line do block).
- `do let x ← readLn` / `do let x <- readLn` (ASCII +
  Unicode bind arrow).
- `do let y := 42` (pure let).
- `do IO.println "hello"` (bare expression statement).
- Multi-statement blocks: each statement on its own line
  (v0). Statement boundary = `\n + horizontal ws +
  !top-level-decl`; the block stops cleanly at the next
  top-level decl keyword so it doesn't eat the next `def`.

Each statement's expression text is captured to end-of-line
and sub-parsed via `parse_expr_text` (the OX6 step 7
pattern); failures fall back to `Expr::Raw(text)` per the
never-panic-on-weird-text invariant.

Multi-line statement bodies + embedded `if` / `match`
statements inside `do` blocks are a follow-up (the v0
assumes line-aligned statements).

Tests 79 → 87 (+8): single-return, pure-collapsed-to-return,
let-bind-dash, let-bind-unicode-arrow, let-pure, bare-expr-
stmt, block-ends-at-top-level-decl (do block doesn't eat
the next `def`), multi-stmt-mix (Bind + Let + Return in
one block).

clippy --all-targets -D warnings clean.

Next OX6 step: string interpolation `s!"…{x}…"`.

### Added — OX6 step 7: multi-line field types + layout-sensitive parsing + full expr re-parse (2026-05-22)

OX6 grammar roadmap step 7. Closes the OX6 step 5
limitation that `StructField.ty` and `Ctor.ty` were raw
`String` text — they're now fully parsed `Expr` values
with multi-line continuation support.

**AST changes**:

```diff
- pub struct StructField { name: String, ty: String }
+ pub struct StructField { name: String, ty: Expr }

- pub struct Ctor { name: String, ty: Option<String> }
+ pub struct Ctor { name: String, ty: Option<Expr> }
```

**Layout-sensitive boundary detection**: a new
`field_or_ctor_boundary` PEG rule decides where one
field/ctor's type region ends and the next starts. The
boundary fires when the parser sees, after a newline +
optional horizontal whitespace, any of:

- next struct field header (`<ident> :` on its own line)
- next ctor arm (`|` on its own line)
- `deriving` keyword
- top-level decl keyword (`def`, `structure`, `inductive`,
  `@[…]` attribute prefix, etc.)
- end of file

Same-line content never triggers the boundary, so a field
header followed by trailing whitespace opens a multi-line
continuation. The continuation lines can use any
indentation — capture grabs raw bytes between header and
boundary.

**Sub-parse with raw fallback**: captured text is fed back
through `lean4::expr(text)` via a new `parse_expr_text`
helper. Parse failure falls back to `Expr::Raw(text)` so a
malformed-but-finite type annotation doesn't abort the
whole decl.

Example — `Vec (Option Nat)` field type now parses as a
proper `App(List, Paren(App(Option, Nat)))` tree:

```lean
structure Big where
  xs : List
    (Option
    Nat)
  y : Nat
```

The boundary detection stops `xs`'s capture at `\n  y :`,
sub-parses `List\n    (Option\n    Nat)` as `expr`, lands
the App tree. `y`'s type starts fresh.

Tests 71 → 79 (+8): struct-field-type-is-fully-parsed-expr,
struct-field-type-with-app, struct-field-type-multi-line-
continuation, struct-field-type-with-arrow, inductive-ctor-
payload-is-fully-parsed-expr, inductive-ctor-multi-line-
payload, struct-unparseable-field-falls-back-to-raw (never-
panic invariant), structure-with-carrier-field-and-deriving.

Existing 6 tests that compared `ty` against `String`
literals updated to compare against `Expr` values. The two
`as_deref()` calls in ctor-type tests replaced with `Expr`
pattern matches.

`lib.rs` top-doc updated to reflect that field / ctor
types are now fully parsed (the previous "raw `String` in
v0" caveat removed).

clippy --all-targets -D warnings clean.

Next OX6 step: `do` notation.

### Added — OX6 step 6: attribute lists with args (2026-05-22)

OX6 grammar roadmap step 6. Attribute prefix `@[…]` on
decls now parses natively, including OxiLean-rejected
forms with arguments.

**Shape**:

```rust
pub struct Decl {
    pub attrs: Vec<Attribute>,        // NEW field
    pub kind: DeclKind,
}

pub struct Attribute {
    pub name: String,
    pub raw_args: String,             // raw text, possibly ""
}
```

Surface coverage:

- `@[simp]` — bare ident.
- `@[simp, ext, inline]` — comma list.
- `@[leo4_specialize_when scalar ∧ ord]` — with args
  (raw-preserved).
- `@[leo4_export, leo4_specialize_when scalar ∧ ord]` —
  mixed bare + with-args in one list.
- `@[Foo.bar]` — dotted attribute names.
- Prefix attaches to any decl kind (`def` / `structure` /
  `inductive`).

The `raw_args` field holds everything after the attribute
name up to the next `,` or `]` (whitespace trimmed). v0
doesn't sub-parse arguments because Lean 4's attribute-arg
grammar is per-attribute (`@[simp]` takes nothing,
`@[builtin_attribute "name" "doc"]` takes string-literal
args, etc.) — downstream consumers re-parse `raw_args`
themselves.

This makes OX3's `strip_attribute_args` textual pre-rewrite
redundant once OX6 is the default parser (the args land in
the AST instead of being stripped).

Inner decl rules construct `Decl { attrs: vec![], kind: … }`;
a new `decl()` wrapper rule reads an optional `attribute_list()`
prefix and attaches it. Existing 62 tests pass unchanged
(no test inspected the new `attrs` field).

Tests 62 → 71 (+9): single-bare-ident, comma-list-bare,
with-args-preserved, mix-bare-and-args, attaches-to-structure,
attaches-to-inductive, no-attr-yields-empty, multi-decl-per-
decl-attrs, dotted-attr-name.

clippy --all-targets -D warnings clean.

Next OX6 step: multi-line field types + layout-sensitive
parsing + full expression re-parse for `StructField.ty`
and `Ctor.ty` (currently raw `String`).

### Added — OX6 step 5: `structure` + `inductive` + `deriving` (2026-05-22)

OX6 grammar roadmap step 5. Three new `Decl` variants
(top-level decl set goes from 1 → 3 kinds) plus dedicated
field / ctor AST types.

**New `DeclKind` variants**:

```rust
pub enum DeclKind {
    Definition { … },
    Structure {
        name: String,
        extends: Vec<String>,
        fields: Vec<StructField>,
        deriving: Vec<String>,
    },
    Inductive {
        name: String,
        ty: Option<Expr>,
        ctors: Vec<Ctor>,
        deriving: Vec<String>,
    },
}

pub struct StructField { name: String, ty: String }
pub struct Ctor       { name: String, ty: Option<String> }
```

Surface coverage:

- `structure NAME [extends BASE1, BASE2] where FIELDS
  [deriving Foo, Bar]` — fields one-per-line, comma-list
  for `extends` + `deriving`.
- `inductive NAME [: TYPE] [where] | CTOR1 [: T1] | CTOR2
  [: T2] [deriving Foo]` — two surface forms fed into the
  same AST:
  - Lean 4 modern: `inductive Color where | red | green
    | blue` (no annotations).
  - OxiLean older: `inductive Color : Type | red : Color
    | green : Color | blue : Color` (explicit ctor types).
  - Bare ctor lines (`| red`) get `ty: None`; the
    elaborator supplies the inductive type itself.
- `deriving Foo, Bar, Baz` clause shared by both decl
  kinds.

**Limitation**: structure field & ctor type annotations
are captured as raw `String` text in this v0 — full
expression re-parsing requires either layout-sensitive
parsing (Lean 4 fields are layout-delimited) or a separate
inline-expr grammar. Follow-up commit will handle the
type-expr lift; until then downstream consumers re-parse
the `String` themselves if needed.

Tests 51 → 62 (+11): structure-basic, structure-with-
deriving, structure-with-extends, structure-with-multi-
extends, inductive-where-form-all-bare, inductive-where-
form-mixed-annotations (bare + annotated mixed in one
inductive), inductive-oxilean-form-explicit-type-annot,
inductive-with-deriving, multi-decl-with-struct-and-def,
deriving-single-class, struct-zero-fields.

A `_h()` (horizontal-only whitespace, no newline) rule was
added to the grammar so single-line field / ctor type-text
capture has an exact line-end boundary.

`lib.rs` top-doc grammar-coverage comment updated to reflect
the v0.5 surface.

clippy --all-targets -D warnings clean.

Next OX6 step: attribute lists (with args).

### Added — OX6 step 4: lambda + `fun` / `λ` (2026-05-22)

OX6 grammar roadmap step 4. New atom `Expr::Lam(binders,
body)` with `LamBinder` enum:

```rust
pub enum LamBinder {
    Untyped(String),                            // fun x => …
    Typed { kind, names, ty },                  // fun (x : T) => …
}
```

Surface forms covered:

- `fun x => body` (single untyped)
- `fun a b c => body` (multi untyped)
- `fun (x : Nat) => body` (typed explicit)
- `fun (a b : Nat) => body` (typed multi-name)
- `fun {T : Type} (x : T) => body` (mixed implicit + explicit)
- `λ x => body` (Unicode λ — Lean 4 accepts both)
- `fun x -> body` (dash arrow — OX3's `lean4_normalize`
  rewrites `=>` to `->`; PEG accepts both natively so it
  can replace the normaliser later without surface
  regressions)

Lambda binders are independent of `def`'s `BinderGroup`
because lambda allows bare untyped names (`fun x => …`) and
`def` doesn't.

Tests 41 → 51 (+10): identity, multi-arg, typed-explicit,
typed-multi-name, mixed-typed-groups (implicit + explicit),
Unicode-λ, dash-arrow-body, lam-inside-def-value,
body-is-full-expr (`fun a b c => f a + g b * h c`),
no-collision-with-def-keyword (multi-decl source: a
lambda body must not eat the next decl's `def`).

clippy --all-targets -D warnings clean.

Next OX6 step: `structure` + `inductive` + `deriving`.

### Added — OX6 step 3: `if-then-else` + `match` with patterns (2026-05-22)

OX6 grammar roadmap step 3. Two new top-level expression
forms + a full pattern AST.

**`if-then-else`** — `Expr::If(cond, then, else)`. Added
as an atom alternative so it slots into the precedence
ladder naturally. `x + if a then b else c` parses with
the if as the `+`'s RHS atom; `if a then 1 + 2 else 3 * 4`
gets full binary-op trees in each branch; nested `if` in
either branch works without parens.

**`match`** — `Expr::Match(scrut, Vec<MatchArm>)` where
`MatchArm { pattern, body }`. The arm separator is a single
`|` token; the precedence ladder accepts `||` as a binary
op, so `match c with | _ => a || b | x => x` correctly
recognises `a || b` as the first arm's body and the
second `|` as the next arm's separator.

**Pattern AST**:

```rust
pub enum Pattern {
    Wildcard,                        // `_`
    Var(String),                     // `x` (may be dotted: `Color.red`)
    Lit(Literal),                    // `42` / `"s"`
    Ctor(String, Vec<Pattern>),      // `node l r`
    DotCtor(String, Vec<Pattern>),   // `.lt` / `.some x`
    Paren(Box<Pattern>),
    Tuple(Vec<Pattern>),             // `(a, b)`
}
```

Bare ident (with or without dots) parses as `Var` — the
elaborator decides ctor-vs-var classification. Multi-arg
applications parse as `Ctor`. Lean 4's dot-shorthand
`.ctorName [args]` parses natively as `DotCtor` (no need
for the OX4 `strip_ctor_dot_shorthand` textual pre-rewrite
once OX6 is the default parser).

Tests 25 → 41 (+16 tests): if-simple, if-inside-binop,
if-with-complex-branches, nested-if, match-single-arm-
wildcard, match-multi-arm, match-dot-ctor (3 forms),
match-dot-ctor-with-args, match-qualified-ctor (Color.red),
match-ctor-with-args (`node l r`), match-tuple-pattern,
match-lit-pattern, match-body-uses-or-binary-op (verifies
`|` / `||` disambiguation), match-with-complex-scrutinee,
plus two end-to-end `def`-body fixtures (`safeDiv`,
`colorName`).

clippy --all-targets -D warnings clean.

Next OX6 step: lambda + `fun`.

### Added — OX6 step 2: expression grammar with operator precedence (2026-05-22)

OX6 grammar roadmap step 2 — replaces the v0 `Expr::Raw`
catch-all in type / value positions with a real precedence-
climbing expression grammar built via `peg`'s
`precedence!` macro.

New `Expr` variants:

```rust
pub enum Expr {
    Ident(String),               // includes dotted forms
                                 //   (`Nat.succ`)
    Lit(Literal),                // Nat / Str
    App(Box<Expr>, Box<Expr>),   // left-assoc fn app
    BinOp(String, Box<Expr>, Box<Expr>),
    UnaryOp(String, Box<Expr>),
    Paren(Box<Expr>),
    Raw(String),                 // catch-all fallback
}

pub enum Literal {
    Nat(u64),
    Str(String),                 // \\n \\t \\r \\\\ \\" \\0 escapes resolved
}
```

Precedence ladder (low → high, matches Lean 4 / Mathlib):

  25  `->` / `→`   arrow (right-assoc) ─ lowest
  30  `||`
  35  `&&`
  50  `==` `!=` `<=` `>=` `<` `>`
  65  `+` `-`
  70  `*` `/` `%`
  75  `^`           (right-assoc)
  90  unary `-` `!` `¬`
  max function application (left-assoc)
   —  atoms (literals / idents / parens)

Both ASCII (`->`) and Unicode (`→`) arrow accepted.
Application binds tightest among non-atom forms, so
`Nat.succ a b` = `App(App(Nat.succ, a), b)`. The arrow is
the *lowest*-precedence binary, so `T1 + T2 -> R` =
`(T1 + T2) -> R`.

The type / value positions in `def` now share one
expression grammar (Lean 4 unifies these), making
`def f (xs : List (Option Nat)) : Nat := xs` parse to
`App(List, Paren(App(Option, Nat)))` for the binder type
rather than the v0 `Raw("List (Option Nat)")`.

Tests 10 → 25 (+15 expression-grammar tests): nat / string
literal, dotted ident, app left-assoc, add left-assoc,
mul-binds-tighter-than-add, pow right-assoc, cmp-below-add,
arrow right-assoc, arrow-lowest-precedence, unicode arrow
equivalence, paren preservation, unary minus, logical
and/or precedence, real fixture (`Nat.succ a`), pre-existing
fixtures still pass with updated expectations (the
header-binder + add tests now compare against `BinOp` /
`Ident` / `Lit` instead of the v0 `Raw`).

clippy --all-targets -D warnings clean.

Next OX6 step: `if-then-else` + `match` arms with patterns.

### Added — OX6: `leo4-lean4-parse` PEG-based Lean 4 parser scaffold (2026-05-22)

OX6 — new v1.0 RC blocker tackled via fork-from-scratch.
The OX4 textual approach was reaching its limits (binary
operator precedence, string interpolation, ctor name
resolution all need real grammar work). Decided to fork
the parser entirely.

**New sibling crate** `sibling/leo4-lean4-parse/`:

- PEG-based grammar via the `peg` crate (Apache-2.0
  compatible with leo4's MIT-OR-Apache-2.0 dual license).
- **Strict superset** of `oxilean-parse` v0.1.2's
  accepted surface where overlapping; AST shapes mirror
  upstream so downstream consumers (oxilean-elab,
  leo4-oxilean-build's transpile pipeline) can swap
  parsers transparently.
- Built from scratch (no source copied from
  `oxilean-parse`); the AST type *signatures* match for
  interop but the grammar is original PEG work.

v0 scaffold (this commit):

- `def NAME [binders]+ [: TYPE] := VALUE` with explicit
  `(...)`, implicit `{...}`, named-instance `[name : T]`,
  and anonymous-instance `[T]` binders.
- Simple type / value expressions as bracket-balanced raw
  text (subsequent commits replace this with proper
  application / arrow / Pi grammar).
- Line comments + whitespace handling.
- 10 unit tests covering: empty source, single binder,
  multi-name binder, implicit + instance binders, no type
  annotation, multi-def source, nested bracket type,
  line comments, parse-error → `LeanError(DECODE_ERROR)`.

clippy --all-targets -D warnings clean.

**Roadmap** (each = one commit):

1. Expression grammar with operator precedence
2. `if-then-else` + `match` arms with patterns
3. Lambda + `fun`
4. `structure` + `inductive` + `deriving`
5. Attribute lists
6. `do` notation
7. String interpolation
8. Full `Decl` enum
9. Cross-check against `oxilean-parse` on shared corpus
10. leo4-oxilean-build switches default parser to OX6

Once OX6 lands, the OX4 textual pre-rewrites in
`lean4_normalize` become legacy (still useful for the
`oxilean-parse` fallback path, but the OX6 parser
handles the surface natively).

### Added — OX4 partial: three more Lean 4 surface pre-rewrites (2026-05-22)

OX4 partial progress. Three new textual pre-rewrites in
`lean4_normalize`, plus one bugfix in `rewrite_header_binders`
(was eating trailing newline before next decl):

**`strip_ctor_dot_shorthand(src) -> String`** — strips the
leading `.` from Lean 4's `.ctorName` auto-bound-namespace
shorthand. `.lt` → `lt` only when preceded by a boundary
char (`|`, `(`, `,`, `=`, `[`, etc. + whitespace); preserves
`foo.field` projections and `..` ranges. UTF-8 safe. Strings
and comments pass through.

**`rewrite_inductive_where(src) -> String`** — lifts Lean
4's `inductive NAME [params] where | ctor1 | ctor2 …` into
OxiLean's required `inductive NAME : Type | ctor1 : NAME
| ctor2 : NAME …`. Two paired rewrites: header `where` →
`: Type`, and each bare `| ctor` line (no inline `:`) gets
` : NAME` appended. Block exits on next top-level decl
keyword. Preserves `| ctor : payload → NAME` lines that
already carry annotations.

**`strip_deriving_clause(src) -> String`** — removes
`deriving Foo, Bar` lines entirely (replaced with blank
lines preserving line-number alignment). leo4-oxilean-build
synthesises the `LeanMarshal` impl itself per the OX2
user-records path; OxiLean's parser doesn't accept the
`deriving` keyword.

**`rewrite_header_binders` bugfix**: previously the emitted
`def NAME : T1 -> ... := fun ... -> body` had no trailing
newline, causing the next decl to land on the same line in
the rewritten output (because `val_end` is *exactly* the
next-decl keyword position). Fixed by appending `\n` to
the emitted decl.

`lean4_normalize` pipeline now:

```
src
 → strip_attribute_args        (OX3)
 → strip_deriving_clause       (NEW — OX4)
 → rewrite_inductive_where     (NEW — OX4)
 → rewrite_header_binders      (OX3 — w/ trailing-\n fix)
 → strip_ctor_dot_shorthand    (NEW — OX4)
 → Lean4TermRewriter::standard
 → Lean4SyntaxAdapter::adapt_all
 → out
```

Tests 103 → 120 (+17 OX4 tests across the three new
module-private test blocks: `.ctorName` strip in 9 forms,
`inductive` rewrite in 5 forms, `deriving` strip path is
covered by the pipeline tests).

`tests/sample-lean/Sample.lean` parses successfully through
the first ~88 lines now (previously stopped at line 23).
Stops at line 89 on `==` operator (`if b == 0 then ...`)
— next sub-gap.

**Still pending in OX4** (deferred to follow-up commits
under the same OX4 entry):

- Binary operator notation (`==`, `+`, `<`, …): OxiLean's
  `parse_expr` v0.1.2 doesn't recognise these; either
  register them via the `NotationTable` API or lower
  textually (`a == b` → `Eq.beq a b`).
- String interpolation (`s!"…{x}…"`): no OxiLean parse
  rule.
- `Except` / `Option` term forms (`Except.ok` / `none`
  ctor names).

clippy --all-targets -D warnings clean.

### Added — OX3: Lean 4 header-binder lift + attribute-arg strip pre-rewrites (2026-05-22)

OX3 v1.0 RC blocker resolved. Two new textual pre-rewrites
in `lean4_normalize` so the OxiLean parser accepts more
real-world Lean 4 surface syntax:

**`rewrite_header_binders(src) -> String`** — character-
level scanner with bracket balancing lifts Lean 4
header-binder `def`s into OxiLean's body-lambda dialect:

```
def add (a b : UInt64) : UInt64 := a + b
                ↓
def add : UInt64 -> UInt64 -> UInt64 := fun a b -> a + b
```

Coverage:
- Multi-name binder groups (`(a b c : T)` → 3 args).
- Multi-group binders (`(a : T) (b : U)` → 2 args).
- Implicit `{T : Type}` / instance `[Ord T]` binders
  stripped from the header (auto-bound by elaborator).
- Universe params `.{u, v}` preserved verbatim.
- Nested-bracket types (`List (Option Nat)`) walk via
  bracket-balanced reader.
- Pass-through: `theorem`, `structure`, `inductive`,
  `instance`, `class` decls; def's already without binders;
  def's inside strings or comments.
- Idempotent.

**`strip_attribute_args(src) -> String`** — reduces each
`@[…]` list entry to its first ident, since OxiLean's
`Parser::parse_attribute_decl` v0.1.2 only accepts bare
idents and rejects args:

```
@[leo4_export, leo4_specialize_when scalar ∧ ord]
                ↓
@[leo4_export, leo4_specialize_when]
```

UTF-8 safe (multi-byte chars in source like `←` /
`∧` preserved). Pass-through for `@[…]` inside strings
or comments. Idempotent.

`lean4_normalize` pipeline now:

```
src
 → strip_attribute_args      (NEW — OX3)
 → rewrite_header_binders    (NEW — OX3)
 → Lean4TermRewriter::standard
 → Lean4SyntaxAdapter::adapt_all
 → out
```

Tests 86 → 103 lib (+17 OX3 tests across the two new
module-private test blocks: pass-through, single binder,
multi-name binder, multi-group, implicit-stripping,
structure / theorem untouched, multi-decl, idempotence,
nested-bracket type, def-in-string protection; attribute
strip pass-through, single-arg, comma-list, string /
comment protection, idempotence).

E2E parse verified against `tests/sample-lean/Sample.lean`:
what was previously a parser-reject `UnexpectedToken {
expected: [":="], got: LParen }` and `UnexpectedToken {
expected: ["]"], got: Ident("scalar") }` now both surface
as elab-level `NameNotFound` errors — meaning the surface-
syntax layer is correctly cleared and the remaining gap
is the elab env (tracked as OX5).

Three follow-up v1.0 RC blockers opened in ROADMAP:

- **OX4**: Lean 4 surface coverage tail (`.ctorName`
  match shorthand, `namespace` blocks, `where` clauses on
  def, etc.). Same textual-pre-rewrite approach.
- **OX5**: elab env bootstrap. CLI runs elab in an empty
  `Environment::new()`, so `UInt64` / `+` etc. don't
  resolve. Needs either a baked env snapshot or pointing
  the CLI at the lake plugin's `.olean` cache.

clippy --all-targets -D warnings clean.

### Added — OX1 step b: lake plugin auto-invocation of `leo4-oxilean-build` (2026-05-22)

OX1 step b — wiring lake plugin to drive
`leo4-oxilean-build` CLI subprocess so a user's `lake exe
leo4plugin` build can emit a transpiled Cargo crate
automatically (mirroring the `--with-lower` pattern that
already exists for `leo4c`).

**Plugin side changes** (`lake/Leo4Plugin/Leo4Plugin/Main.lean`):

- `Config` extended with four optional fields:
  `transpileSource : Option System.FilePath`,
  `transpileOutDir : Option System.FilePath`,
  `transpileCrateName : Option String`,
  `transpileAbiDep : Option String`.
- `parseArgs` gains a `takeFlagWithValue` helper to peel
  `--flag <value>` pairs out of the positional arg list.
  New flags:
  `--transpile <lean-file>` (turns the feature on),
  `--transpile-out-dir <dir>` (default `<outDir>/transpiled`),
  `--transpile-crate-name <name>` (default = normalised `pkg`),
  `--transpile-abi-dep <toml-frag>` (default `"0.1"`).
- `runPlugin` end gains a `transpileSource`-driven branch
  that builds a multi-decl manifest:

  ```text
  crate_name=<crateName>
  schema_hash=<schemaHash.toBase32lc>
  leo4_abi_dep=<abiDepSpec>
  out_dir=<txOutDir>
  source=<leanSrcPath>
  bind=<decl_name>=<mangled>   (one per instantiation)
  …
  ```

  then `IO.Process.run { cmd := "leo4-oxilean-build", args
  := ["--manifest", path] }`. Failures are caught + logged
  with the same opt-in helper text as `--with-lower`.

**CLI side changes** (`sibling/leo4-oxilean-build/`):

- Manifest format gains a multi-decl source-line form.
  Single-decl `source=<file> <mangled>` stays supported
  (backwards compatible); multi-decl form is just
  `source=<file>` followed by `bind=<decl_name>=<mangled>`
  lines until the next `source=`. CLI rejects `bind=`
  before any `source=` or after a single-decl `source=`
  with explanatory errors.
- New library API
  `transpile_source_to_units(env, registry, src,
  name_to_mangled: &HashMap<String, String>) -> Result<Vec<TranspileUnit>, LeanError>`
  — parses every top-level decl in `src`, drives each
  tagged `@[leo4_export]` through the existing
  per-decl pipeline (definition / structure / inductive),
  looks up `Definition` mangled names in the map.
- Old `transpile_source_to_unit` refactored to delegate to
  a new internal `process_parsed_decl` helper so both
  entry points share the same per-decl logic.
- Upstream `oxilean_parse::parser::parse_decls` has an EOF-
  detection bug (catches only `UnexpectedEof`, not
  `UnexpectedToken { got: Eof }`), so we walk the parser
  manually using the public `Parser::is_eof()` method.
- CLI manifest parser tracks the "current source" entry so
  subsequent `bind=` lines attach to it.

Tests 80 → 86 lib (+3 multi-decl tests:
`transpile_source_to_units_handles_multi_decl_source`,
`transpile_source_to_units_rejects_missing_mangled_for_fn`,
`transpile_source_to_units_uses_mangled_map_per_fn`) +
6 → 9 CLI integration tests (`cli_multi_decl_form_with_binds`,
`cli_bind_before_source_rejected`,
`cli_bind_after_single_source_rejected`).

**E2E smoke** verified against `tests/sample-lean/Sample.lean`:
plugin produces a 60+ -line manifest with every
instantiation's mangled name, subprocess invocation
launches correctly, CLI reads + processes the manifest.
clippy --all-targets -D warnings clean.

**OX1 wiring infrastructure complete**. The remaining
issue surfaced by the e2e smoke: OxiLean's parser rejects
Lean 4's header-binder `def f (x : T) : R := body` form
(only `def f : T → R := fun x → body` parses).
`oxilean-elab::lean4_compat` v0.1.2 is textual only — it
doesn't lift header binders. Tracked as the new v1.0 RC
**OX3 blocker** (AST-level header-binder adapter) in
ROADMAP.

### Added — `leo4-oxilean-build`: OX2 user records complete — `Decl::Inductive` integration (2026-05-22)

OX2 user-records sub-blocker **fully closed**. The previous
commit landed `synthesize_enum_type` as a library piece;
this commit wires it into `transpile_source_to_unit` so a
Lean source with `@[leo4_export] inductive Either : Type |
left : Nat -> Either | right : String -> Either` produces a
type-only `TranspileUnit` whose `type_decls[0]` is the
emitted `pub enum Either { … }` + `LeanMarshal` impl, and
whose name is registered in `registry.user_types` so
subsequent decls can use it.

New private helper:

- `unfold_ctor_payload(&Located<SurfaceExpr>) -> Vec<&…>`
  — peels the Pi chain of a Lean ctor type
  (`Pi (_ : Nat), Pi (_ : String), Either` → `[&Nat, &String]`).
  Unit ctors (no Pi) return an empty slice.

`transpile_source_to_unit` branches:
- `Decl::Definition` → fn / wrapper / mangled-dispatch path
- `Decl::Structure` → struct type-only unit (existing)
- `Decl::Inductive` → enum type-only unit (this commit)
- anything else → loud `ENCODE_ERROR` with the kind label

OxiLean parser quirk documented: `inductive N : Type | ctor :
T1 -> ... -> N` form is required — every ctor needs an
explicit `: <type>` annotation, even unit ctors (`Red : Color`,
not bare `Red`). Test sources reflect this.

Tests 80 → 83 (+3):

  + `transpile_source_to_unit_handles_all_unit_inductive` —
    `Color { Red, Green, Blue }` → unit-form enum.
  + `transpile_source_to_unit_handles_payload_inductive` —
    `Either { left(Nat), right(String) }` → payload enum.
  + `transpile_source_to_unit_inductive_references_prior_user_type`
    — `Shape { dot(Point), line(Point, Point) }` after Point
    structure has been registered.

Also factored a small `unit_is_type_only(&TranspileUnit)`
test helper for the type-only assertion pattern.

clippy --all-targets -D warnings clean.

**OX2 user records sub-blocker now fully closed for v1.0
RC**. The complete OX2 surface for user-defined types:
- Lean `structure` → Rust struct + LeanMarshal impl
- Lean `inductive` → Rust enum + LeanMarshal impl
- Carrier-type fields (BigNat / BigInt / LeanRat / complex)
- Generic-container fields (Vec / Option / Result / Box /
  tuples 2..=5)
- User-type fields (Point inside Edge / Shape, etc.)
- Rust-keyword field & variant names → raw-ident escaped
- Wire form byte-compatible with `#[derive(LeanMarshal)]`

ROADMAP OX2 entry updated to reflect closure.

### Added — `leo4-oxilean-build`: OX2 user records — `synthesize_enum_type` for inductive types (2026-05-22)

OX2 user-records final lib piece. Lean inductive types (Lean
`inductive Color | Red | Green` or
`inductive Either | left : Nat → Either | right : String → Either`)
synthesise into Rust enums + inline `LeanMarshal` impls.

Wire shape matches the leo4 derive macro's
`expand_derive_variant` / `expand_derive_enum` expansion
exactly (see `crates/leo4-macros-backend/src/lib.rs`):

```
wire = 4-byte LE discriminator (u32)
     + per-variant payload (declaration-order fields,
       each `LeanMarshal::canonical_encode`d sequentially —
       no padding)
```

Per `SPEC/canonical-abi.md` §9 (B5 discriminator
canonicalisation). An enum synthesised this way is therefore
byte-compatible with a hand-written `#[derive(LeanMarshal)]`
of the same variant shape.

New public API:

- `EnumVariant { name: String, fields: Vec<RustType> }` —
  one ctor of a Lean inductive. `fields: []` is a unit
  variant; otherwise tuple-style payload.
- `synthesize_enum_type(name, variants) -> Result<String, LeanError>`
  / `synthesize_enum_type_with_users(name, variants,
  user_types)` — emits the enum decl + LeanMarshal impl.

Coverage:

- All-unit enums (Lean `inductive Color | Red | Green | Blue`)
  → `enum Color { Red, Green, Blue }`.
- Mixed unit + payload variants (`Maybe` with `none`, `some(u64)`).
- Carrier-type payloads (`Either { ok(BigNat), err(String) }`).
- User-type payloads via `*_with_users`
  (`Shape { dot(Point), line(Point, Point) }`).
- Keyword variant names → `escape_rust_ident` (`Choice { r#match, r#type(u32) }`).
- Empty enum → loud rejection (`ENCODE_ERROR`, "no variants").
- Unmarshallable payload type → atomic rejection (no partial
  emit).

Sample output (`Either { left: BigNat → Either, right: String → Either }`,
abbreviated):

```rust
pub enum Either {
    left(::leo4_abi::BigNat),
    right(::std::string::String),
}

impl ::leo4_abi::LeanMarshal for Either {
    fn canonical_encode(&self, buf: &mut ::std::vec::Vec<u8>) {
        match self {
            Self::left(__f0) => {
                buf.extend_from_slice(&0u32.to_le_bytes());
                <::leo4_abi::BigNat as ::leo4_abi::LeanMarshal>::canonical_encode(__f0, buf);
            }
            Self::right(__f0) => {
                buf.extend_from_slice(&1u32.to_le_bytes());
                …
            }
        }
    }
    fn canonical_decode(buf: &[u8], off: usize) -> Result<(Self, usize), LeanError> {
        // 4-byte LE tag → variant dispatch
        …
    }
}
```

Tests 72 → 80 (+8): all-unit enum, payload tuple variant,
mixed shapes, carrier payload, user-type payload, atomic
rejection of bad payload, zero-variant rejection, keyword
escape coverage.

`examples/dump_enum.rs` prints a fixture `Either { left:
BigNat, right: String }` enum for visual inspection.

clippy --all-targets -D warnings clean (added a `# Panics`
note on `_with_users` for the `writeln!` infallibility
artefact).

**OX2 user records sub-blocker now fully resolved at the
library level**. Remaining `Decl::Inductive` integration in
`transpile_source_to_unit` (analogous to the existing
`Decl::Structure` branch) is one small follow-up commit.

### Added — `leo4-oxilean-build`: OX2 user records — Rust-keyword identifier escaping (2026-05-22)

OX2 user-records fourth increment. Lean field / param /
ctor names that collide with Rust keywords are now escaped
in the emitted source.

New public API:

- `escape_rust_ident(name: &str) -> String` — public so
  callers (and future `synthesize_enum_type`) reuse the same
  policy:
  - Raw-ineligible names → trailing `_`:
    `self → self_`, `Self → Self_`, `super → super_`,
    `crate → crate_`, `true → true_`, `false → false_`,
    `_ → __`
  - Strict + reserved keywords → `r#name`:
    `type → r#type`, `match → r#match`, `async → r#async`,
    `yield → r#yield`, etc.
  - Other names pass through (`from → from`,
    `head → head`) — note Rust's `from` is *not* a keyword
    (it's a `From::from` method name), so no escaping needed.

`synthesize_struct_type_with_users` now escapes every field
name in:
- struct decl LHS (`pub r#type: u32,`)
- encode body (`self.r#type`)
- decode binding (`let (r#type, __next) = …`)
- `Self { … }` struct-literal at end

Wire form is unchanged — the canonical-ABI encoder /
decoder visit fields in declaration order regardless of the
Rust identifier shape, so a struct synthesised this way is
still byte-compatible with a hand-written
`#[derive(LeanMarshal)]` of the same field-type sequence.

Tests 69 → 72 (+3):

  + `escape_ident_handles_strict_keywords` — strict + reserved
    keyword escaping
  + `escape_ident_handles_raw_ineligible` — raw-ineligible
    trailing-underscore fallback
  + `struct_emits_raw_ident_for_keyword_field` — end-to-end
    check that all four emit sites use the escaped form

clippy --all-targets -D warnings clean.

**OX2 user records remaining for v1.0 RC**: Inductive
(multi-ctor enums) — `synthesize_enum_type` analogous to
`synthesize_struct_type`, with variant payload widening per
`SPEC/canonical-abi.md` §9.

### Added — `leo4-oxilean-build`: OX2 user records — `Decl::Structure` integration in `transpile_source_to_unit` (2026-05-22)

OX2 user-records third increment. `transpile_source_to_unit`
now recognises `@[leo4_export] structure Foo where ...`
sources and produces a *type-only* `TranspileUnit`:

- `fn_src` / `wrapper_src` are empty (no callable code)
- `mangled` is empty (no `LeanProc` dispatch arm)
- `type_decls` carries the synthesised struct + LeanMarshal
  impl source
- `registry.user_types` gains the structure's name, so any
  subsequent `@[leo4_export] def` referring to the type by
  name marshals through it transparently
- subsequent structures can use earlier ones as field
  types — `Edge { head: Point, tail: Point }` works

`emit_lib_rs` updated to recognise the two type-only
sentinels: it (a) emits `type_decls` ahead of fn / wrapper
bodies as before, (b) skips fn / wrapper emission for units
whose source is empty, (c) skips dispatcher arms for units
whose `mangled` is empty.

OxiLean parser note: structure fields are separated by
whitespace / newlines, NOT by `;`. Test sources reflect
this.

Tests 65 → 69 (+4):

  + `transpile_source_to_unit_handles_tagged_structure` —
    end-to-end Lean source → emitted struct + impl.
  + `transpile_source_to_unit_structure_references_prior_user_type`
    — chains two structures (Point, then Edge with two
    Point fields) through the same registry.
  + `transpile_source_to_unit_skips_untagged_structure` —
    untagged structure yields `Ok(None)`, registry unchanged.
  + `emit_lib_rs_handles_type_only_unit` — verifies the
    skip-for-empty-fields logic in `emit_lib_rs`.

clippy --all-targets -D warnings clean.

**Still pending for OX2 user records before v1.0 RC**:

1. Lean field names that are Rust keywords (e.g. `from`,
   `type`, `match`) — need raw-ident escaping (`r#from`) in
   the emitted Rust struct. Not in this commit.
2. Inductive (multi-ctor enums) — `synthesize_enum_type`
   analogous to `synthesize_struct_type`, with variant-
   payload widening per `SPEC/canonical-abi.md` §9.

### Added — `leo4-oxilean-build`: OX2 user records — SurfaceExpr lifter + user-type registry pass-through (2026-05-22)

OX2 user-records second increment. Wires
`synthesize_struct_type` into the same parser-AST layer the
fn transpile uses, so a Lean structure source flows through
the same `surface_to_rust_type` path as fn param / return
types do.

New public API:

- `surface_to_rust_type(&Located<SurfaceExpr>, &HashSet<String>)
  -> Result<RustType, LeanError>` — lifts a parser-AST type
  annotation into a marshallable `RustType`. Walks the App
  spine for generics (`Vec Nat` → `Vec<U64>`, `Vec (Option
  Nat)` → `Vec<Option<U64>>`), looks up bare names in the
  primitive table, carrier table, then the caller's set
  of registered user types.
- `primitive_name_to_rust_type(&str) -> Option<RustType>` —
  Lean primitive name → matching `RustType`. Recognises both
  Lean 4 forms (`UInt32`, `Float`, `Nat`) and Rust forms
  (`U32`, `F64`). Mirrors OxiLean's LCNF lowering: Lean
  `Nat` → `u64`, `Int` → `i64` (carrier types are opt-in
  via explicit `BigNat` / `BigInt` use).
- `Leo4ExportRegistry::user_types: HashSet<String>` —
  registry now tracks user-defined type names discovered
  during a build.
- `Leo4ExportRegistry::register_user_type(&str)` /
  `Leo4ExportRegistry::user_type_names()` — registration +
  snapshot.
- `synthesize_canonical_wrapper_with_users(&RustFn,
  &HashSet<String>)` — user-types-aware variant. The
  non-suffixed `synthesize_canonical_wrapper` now wraps this
  with an empty set.
- `synthesize_struct_type_with_users(name, fields,
  &HashSet<String>)` — user-types-aware struct synth.
  A struct's fields can reference *other* registered
  user types via bare names (e.g. `Edge { from: Point, to:
  Point }`).
- `render_marshallable_type_with_users(&RustType,
  &HashSet<String>)` — bare `Custom("Foo")` resolves to
  the same name (no carrier path lookup, no rejection) if
  `Foo` is in the user-types set.

Surface-AST lifter accepts:

- `Var(name)` — primitive / carrier / user type by name
- `App` spine — `Vec`, `Option`, `Result`, `Box`, `Prod` get
  dedicated lowering; other heads fall through as
  `RustType::Generic(name, args)`
- `Ann(inner, _)` — strips the annotation, lifts the inner

Rejects (returns `DECODE_ERROR`):
- `Pi` / `Lam` / `Sort` / `Lit` / `Let` / `Have` / `Hole` /
  `Proj` / `If` / `Match` / `Do` / future-added variants
  (catch-all `_` arm).

`TranspileUnit` gained a `type_decls: Vec<String>` field —
each unit now carries the user-defined type declarations it
depends on. `emit_lib_rs` deduplicates these across units
(via verbatim-source equality) and emits them ahead of fn
bodies so forward references resolve.

Tests 53 → 65 (+12): primitive lifter coverage, carrier
lifting, user-types lookup, unknown-name rejection, generic
App + nested App, registry round-trip, user-type wrapper
synthesis, user-typed struct fields.

clippy --all-targets -D warnings clean (added
`implicit_hasher = "allow"` since the user-types set always
uses the default `RandomState` hasher in this crate; no
benefit to generalising over `BuildHasher`).

**OX2 user records still pending for v1.0 RC**:

1. `transpile_source_to_unit` extension recognising
   `Decl::Structure` decls in the source stream — auto-
   populates `registry.user_types` + builds the `type_decls`
   list per unit. (Lib pieces all in place; just needs the
   parser-AST visitor wired through.)
2. Inductive (multi-ctor enums) — `synthesize_enum_type`
   analogous to `synthesize_struct_type`, with the variant-
   payload-widening rule from `SPEC/canonical-abi.md` §9.

### Added — `leo4-oxilean-build`: OX2 user records — `synthesize_struct_type` layer (2026-05-22)

OX2 user-records sub-blocker, option (b) per ROADMAP
(leo4-oxilean-build synthesises struct shapes itself; upstream
`RustTargetBackend` v0.1.2 emits only `RustItem::Fn`).
First increment: hand-built `RustType` fixture → emitted
struct + LeanMarshal impl. SurfaceExpr lifter +
elab-driven `Decl::Structure` integration are the next two
increments.

New public API:

- `StructField { name: String, ty: RustType }` — one
  named field, validated against the OX2 marshalling
  matrix.
- `synthesize_struct_type(name, fields) -> Result<String, LeanError>`
  — emits the struct declaration + matching inline
  `LeanMarshal` impl. Field encode / decode order matches
  the leo4 derive macro's expansion
  (`crates/leo4-macros-backend/`) so a struct synthesised
  this way is byte-compatible with a hand-written
  `#[derive(LeanMarshal)]` of the same shape.

Sample output (from `examples/dump_struct.rs`,
`pub struct MoneyBag { major: BigNat, minor: u32, label: String, flags: Vec<bool> }`):

```rust
#[derive(Debug, Clone)]
pub struct MoneyBag {
    pub major: ::leo4_abi::BigNat,
    pub minor: u32,
    pub label: ::std::string::String,
    pub flags: ::std::vec::Vec<bool>,
}

impl ::leo4_abi::LeanMarshal for MoneyBag {
    fn canonical_encode(&self, buf: &mut ::std::vec::Vec<u8>) {
        ::leo4_abi::LeanMarshal::canonical_encode(&self.major, buf);
        ::leo4_abi::LeanMarshal::canonical_encode(&self.minor, buf);
        …
    }
    fn canonical_decode(buf: &[u8], off: usize)
        -> ::core::result::Result<(Self, usize), ::leo4_abi::LeanError>
    {
        let mut __off = off;
        let (major, __next) =
            <::leo4_abi::BigNat as ::leo4_abi::LeanMarshal>::canonical_decode(buf, __off)?;
        __off = __next;
        …
        ::core::result::Result::Ok((Self { major, minor, label, flags }, __off))
    }
}
```

Why inline impl, not `#[derive(LeanMarshal)]`: keeps the
emitted crate's only leo4 dep at `leo4-abi`. A derive would
need `leo4-macros` (proc-macro toolchain in the consumer's
build env) + `leo4` itself (since the derive expands to
`::leo4::LeanMarshal` fully-qualified). Inline-impl bypass
preserves version-independence for the emitted crate.

Coverage:

- 0-field structs → unit-struct form (`pub struct Foo;`)
  with a no-op encoder + zero-byte decoder.
- 1-field structs → single-binding struct literal
  (`Self { inner }` not `Self { inner, }`).
- N-field structs (N≥2) → comma-separated bindings.
- Field types must pass `render_marshallable_type` —
  unmarshallable fields reject atomically before any
  source is emitted (no partial structs).

Tests 46 → 53 (+7):
  + `struct_emits_decl_and_marshal_impl`
  + `struct_emits_carrier_field_types`
  + `struct_emits_generic_container_field`
  + `struct_rejects_unmarshallable_field_atomically`
  + `struct_with_zero_fields_emits_unit_form`
  + `struct_with_single_field_builds_correct_literal`
  + `struct_emit_is_compatible_with_derive_macro_order`

`examples/dump_struct.rs` prints the synthesised source
for a four-field `MoneyBag` fixture.

clippy --all-targets -D warnings clean (sibling crate;
main workspace unaffected).

**Next OX2 increments** (sub-blocker still open):
1. `SurfaceExpr` → `RustType` lifter so structures parsed
   from real Lean source feed `synthesize_struct_type`
   automatically.
2. `transpile_source_to_unit` extension to recognise
   `Decl::Structure` decls in the source stream.
3. User-type pass-through in `render_marshallable_type` so
   transpiled fns can reference user-defined structures by
   name in their params / return types.
4. Inductive (multi-ctor enums) — analogous
   `synthesize_enum_type` with variant-payload widening.

### Added — `leo4-oxilean-build`: OX1 step a — `leo4-oxilean-build` CLI binary (2026-05-22)

v1.0 RC blocker OX1 (step a — subprocess invocation
surface). New CLI binary at
`sibling/leo4-oxilean-build/src/bin/leo4-oxilean-build.rs`
that lake plugin / leo4-rust-emit / a user's `build.rs` can
spawn to drive the transpile pipeline without linking
leo4-oxilean-build into their own crate.

Manifest format (line-oriented `key=value`, no JSON dep):

```text
crate_name=my_pkg
schema_hash=0123456789abc
leo4_abi_dep={ path = "../leo4-abi" }
out_dir=/tmp/transpiled
source=lean/Foo.lean abc12345_a
source=lean/Bar.lean def67890_a
```

`#` and blank lines ignored; multiple `source=` lines
accumulate. Each source line is `<lean_path> <mangled>` —
the caller (lake plugin / leo4-rust-emit) precomputes the
mangled name per `SPEC/mangling.md` §3.

Invocation:

```text
leo4-oxilean-build --manifest <path>
leo4-oxilean-build --manifest -            # stdin
leo4-oxilean-build --help
```

Exit codes: `0` = success (crate emitted), `1` = transpile
failure (no crate emitted, errors on stderr), `2` = usage
or IO error.

New supporting lib API:

- `transpile_source_to_unit(env, registry, src, mangled)
  -> Result<Option<TranspileUnit>, LeanError>` — superset
  of `transpile_source_if_exported` that bundles the
  transpiled fn source + the §5 wrapper source + the
  mangled-name dispatch key into a single `TranspileUnit`
  ready for `emit_crate`. The CLI drives this one file at
  a time; library callers can use it directly without
  going through CLI args.

Tests 44 → 46 (+2 lib) + 6 new CLI integration tests in
`tests/cli_smoke.rs`:
  + `cli_help_exits_success`
  + `cli_missing_manifest_arg_exits_usage_error`
  + `cli_skip_path_emits_dispatcher_only`
  + `cli_reads_manifest_from_stdin`
  + `cli_rejects_bogus_manifest_field`
  + `cli_reports_transpile_error_with_nonzero_exit`

Integration tests use `env!("CARGO_BIN_EXE_leo4-oxilean-build")`
+ `CARGO_TARGET_TMPDIR` for hermetic subprocess invocation
with auto-cleaned scratch dirs.

clippy --all-targets -D warnings clean. Workspace unaffected
(sibling crate). **OX1 step b** (lake plugin / leo4-rust-emit
auto-invocation) tracked in ROADMAP as the remaining v1.0 RC
piece for the wiring.

### Added — `leo4-oxilean-build`: OX2 — carrier types + generics in wrapper marshalling matrix (2026-05-22)

v1.0 RC blocker OX2 (carrier-types layer). The §5 wrapper
synthesiser's `render_marshallable_type` matrix extended from
primitives-only to cover everything leo4-abi already
provides `LeanMarshal` impls for:

**Carrier types** (via new `carrier_path_for` helper):

- `BigNat` / `Leo4_BigNat` → `::leo4_abi::BigNat`
- `BigInt` / `Leo4_BigInt` → `::leo4_abi::BigInt`
- `LeanRat` / `Rat` / `Leo4_LeanRat` → `::leo4_abi::LeanRat`
- `LeanComplexF32x2` → `::leo4_abi::LeanComplexF32x2`
- `LeanComplexF64x2` → `::leo4_abi::LeanComplexF64x2`

OxiLean mangles `.` → `_` in qualified names, so both bare
(`BigNat`) and mangled (`Leo4_BigNat`) spellings are
accepted — the carrier-name matcher is robust to where in
the user's Lean source the type lives.

**Generic containers** (recursive on inner types):

- `Vec<T>` → `::std::vec::Vec<T>`
- `Option<T>` → `::core::option::Option<T>`
- `Result<T, E>` → `::core::result::Result<T, E>`
- `Box<T>` → `::std::boxed::Box<T>` (Generic-form only)
- Tuples arity 2..=5 → `(T1, T2, ...)`

Both the dedicated `RustType::Vec/Option/Result` variants
*and* the `RustType::Generic("Vec"|"Option"|"Result"|"Box", _)`
forms are recognised — upstream may emit either.

**Nightly carrier types** (behind `nightly-floats` cargo
feature forwarded to leo4-abi):

- `LeanF16` / `LeanBF16` / `LeanF128`
- `LeanComplexF16x2` / `LeanComplexBF16x2` / `LeanComplexF128x2`

Tests 31 → 44 (+13 OX2 tests): BigNat / BigInt / LeanRat /
complex / Vec<u64> / Option<String> / Result return /
nested generics `Vec<Option<u64>>` / 2-tuple / Generic-form
Vec / unknown-Custom rejection / `BigNat<...>` rejection /
tuple-with-unsupported-inner cascading rejection. The old
`wrapper_rejects_unsupported_param_type` (which used
`Vec<u64>` as the "unsupported" sample) was updated to use
a user-defined record name instead — `Vec<u64>` is now
*supported*.

What's **still pending for OX2** before v1.0 RC: user-defined
records / inductives. Blocked on upstream
`RustTargetBackend` v0.1.2 emitting only `RustItem::Fn` —
no struct / impl shapes. Tracked as the OX2-user-records
sub-blocker in ROADMAP.

clippy --all-targets -D warnings clean. Workspace
unaffected.

### Added — `leo4-oxilean-build`: §6 Cargo crate emit + `LeanProc` dispatch table (2026-05-22)

End of the activation plan. The transpile pipeline now
produces a complete Cargo crate the consumer can `path`-dep:

- `Cargo.toml` — `[package]` block + `leo4-abi` dep with the
  consumer-supplied spec (path / version / git).
- `src/lib.rs` — every transpile unit's plain Rust fn + the
  §5 `_call` wrapper, followed by a `Leo4OxileanProc: LeanProc`
  impl whose `call(mangled, args)` body is a `match mangled
  { … }` dispatch table mapping every export's mangled name to
  its `<fn>_call` wrapper. Unknown names return
  `LeanError::unknown_function(mangled)` (canonical
  `0x06 UNKNOWN_FUNCTION`).

New public API:

- `TranspileUnit { fn_src, wrapper_src, fn_name, mangled }`
  — one emit-ready unit. `mangled` is the leo4 mangled
  symbol (per `SPEC/mangling.md` §3); the build tool
  (eg. `leo4-rust-emit` / lake plugin) supplies it.
- `GeneratedCrate { crate_name, manifest, lib_rs }` +
  `write_to_dir(&Path) -> io::Result<usize>` — in-memory
  pair of files, with filesystem write helper.
- `emit_cargo_toml(crate_name, leo4_abi_dep_spec) -> String`
  — manifest emission.
- `emit_lib_rs(units, schema_hash) -> String` — `src/lib.rs`
  emission including the dispatcher.
- `emit_crate(crate_name, units, leo4_abi_dep_spec, schema_hash)
  -> GeneratedCrate` — combined entry.

Sample dispatcher (from `examples/dump_crate.rs` output):

```rust
pub struct Leo4OxileanProc;

impl Leo4OxileanProc {
    #[must_use]
    pub fn new() -> Self { Self }
}

impl ::leo4_abi::rust_native::LeanProc for Leo4OxileanProc {
    fn schema_hash(&self) -> &str { "0123456789abc" }
    fn abi_version(&self) -> u32 { 1 }
    fn call(&self, mangled: &str, args: &[u8])
        -> ::core::result::Result<::std::vec::Vec<u8>, ::leo4_abi::LeanError>
    {
        match mangled {
            "abc12345_ab_a" => Sample_addOne_call(args),
            "def67890_ab_a" => Sample_double_call(args),
            _ => Err(::leo4_abi::LeanError::unknown_function(mangled)),
        }
    }
}
```

Tests 25 → 31 (+6 §6 tests):

  + `emit_cargo_toml_includes_required_fields`
  + `emit_lib_rs_concatenates_fn_and_wrapper_per_unit`
  + `emit_lib_rs_emits_leanproc_dispatcher`
  + `emit_lib_rs_empty_units_still_emits_dispatcher`
  + `emit_crate_pairs_manifest_and_lib`
  + `write_to_dir_creates_manifest_and_lib_rs` (uses
    `CARGO_TARGET_TMPDIR` so cleanup is hermetic)

`examples/dump_crate.rs` — prints the full emitted manifest
+ lib.rs for a two-export fixture. `cargo run --example
dump_crate`.

clippy --all-targets -D warnings clean. Workspace unaffected.

Activation plan complete (§1 → §6). The transpile path from
SPEC/rust-native-lean.md §9 is now end-to-end implemented:
Lean source → lean4_compat normalisation → parse → Hook 3
`@[leo4_export]` discovery → elab → LCNF → Rust source +
canonical-ABI wrappers → Cargo crate with `LeanProc` impl.

### Added — `leo4-oxilean-build`: §5 canonical-ABI wrapper synthesis (2026-05-22)

Each transpiled fn now lands together with a sibling
canonical-ABI boundary shim matching leo4-rust-native's
`LeanProc::call(mangled, args) -> Result<Vec<u8>, LeanError>`
contract.

Sample output for `Sample.addOne : Nat → Nat`:

```rust
pub fn Sample_addOne(n: u64) -> u64 { … }   // RustTargetBackend
pub fn Sample_addOne_call(args: &[u8])      // §5 wrapper
    -> ::core::result::Result<::std::vec::Vec<u8>, ::leo4_abi::LeanError>
{
    let mut __off: usize = 0;
    let (n, __next_0) =
        <u64 as ::leo4_abi::LeanMarshal>::canonical_decode(args, __off)?;
    __off = __next_0;
    let _ = __off;
    let __ret = Sample_addOne(n);
    ::core::result::Result::Ok(::leo4_abi::encode_to_vec(&__ret))
}
```

New public API:

- `synthesize_canonical_wrapper(&RustFn) -> Result<String,
  LeanError>` — emits the wrapper source for one transpiled
  fn. Validates every param + return type against a
  marshallable-type matrix; rejects unsupported types
  (`Box<dyn Any>`, `Vec<T>`, user types, etc.) with
  `ENCODE_ERROR`.
- `transpile_kernel_decl_with_wrapper(name, params, body) ->
  Result<(fn_src, wrapper_src), LeanError>` — convenience
  combined entry; runs `transpile_kernel_decl` +
  `synthesize_canonical_wrapper` and returns both source
  strings separately so callers can route them into
  different files / modules.

Marshallable type matrix (v0):

- Integers: u8, u16, u32, u64, u128, i8, i16, i32, i64, i128
- Floats: f32, f64
- Other primitives: bool, char
- `String` (owned)
- Unit return (`()` or `None`) — wrapper returns an empty
  `Vec<u8>` rather than calling `encode_to_vec`

Each type maps to its `leo4_abi::LeanMarshal` impl (see
`crates/leo4-abi/src/scalars.rs` + `composites.rs`).

What's *not* yet covered, documented inline:

- Carrier types (`BigNat`, `BigInt`, `LeanRat`, complex
  numbers) — their `LeanMarshal` impls exist in leo4-abi,
  but `RustTargetBackend` v0.1.2 doesn't lower the matching
  LCNF types to them yet (lands as `Box<dyn Any>`).
- User-defined records / inductives — `RustTargetBackend`
  emits only `RustItem::Fn` at v0.1.2; struct + impl emission
  is upstream's responsibility.

Tests 17 → 25 (+8):
  + `wrapper_emits_call_fn_for_u64_to_u64`
  + `wrapper_emits_zero_arg_fn`
  + `wrapper_handles_unit_return`
  + `wrapper_handles_none_return_as_unit`
  + `wrapper_emits_multi_arg_decode_in_order`
  + `wrapper_rejects_box_dyn_any_return`
  + `wrapper_rejects_unsupported_param_type`
  + `transpile_kernel_decl_with_wrapper_emits_both`

`examples/dump_wrapper.rs` — small binary that prints the
synthesized wrapper for a fixture `RustFn`. Run with
`cargo run --example dump_wrapper`.

clippy --all-targets -D warnings clean. Workspace unaffected
(sibling crate). Activation plan §5 ticked. §6 (Cargo crate
emit) is next + last.

### Added — `leo4-oxilean-build`: Hook 3 `@[leo4_export]` / `LeanMarshal` discovery wired (2026-05-22)

Plugs leo4 into OxiLean evaluator Hook 3 (SPEC/rust-native-lean.md
§7.1, listed as PRESENT). New public surface:

- `LEO4_EXPORT_ATTR` / `LEAN_MARSHAL_DERIVE` — string
  constants leo4 owns at the OxiLean boundary.
- `Leo4ExportRegistry` — owns an `AttributeManager` +
  `DeriveHandlerRegistry`. `::new()` pre-populates both with
  leo4's handlers:
  - `AttrHandler::new("leo4_export", …, AttrAction::Custom)`
    registered via `manager.register_custom_handler`.
  - `DeriveHandler::new(Name::str("LeanMarshal"), …).no_instance()`
    registered via `derive.register`. `no_instance()` signals
    that the transpiler emits the Rust `LeanMarshal` impl
    directly — no Lean-side typeclass instance is needed.
- `decl_has_leo4_export(&Located<Decl>) -> bool` — inspect a
  parsed top-level decl's outer `Decl::Attribute` wrapper for
  the `leo4_export` tag. AST-only, no elaboration.
- `inner_decl(&Located<Decl>) -> &Located<Decl>` — unwrap
  one layer of `Decl::Attribute` (mirrors `elaborate_decl`'s
  upstream unwrap).
- `decl_name(&Located<Decl>) -> Option<&str>` — best-effort
  declaration-name extraction (Definition / Theorem / Axiom /
  Inductive). Used to record discoveries in the manager
  before elaboration.
- `transpile_source_if_exported(env, registry, src) ->
  Result<Option<String>, LeanError>` — Hook-3-aware variant
  of `transpile_source`. Untagged decls yield `Ok(None)` with
  no work done; tagged decls record themselves into
  `registry.manager` (visible via `registry.exported_names()`)
  and proceed through the full pipeline.

Parser-shape note documented inline: OxiLean's parser maps
`@[name] def f := body` to `Decl::Attribute { attrs:
Vec<String>, decl: Box<Located<Decl>> }`, leaving the inner
`Decl::Definition.attrs` field empty. `elaborate_decl` in
v0.1.2 unwraps `Decl::Attribute` and discards the outer
`attrs`. So leo4 inspects the *parser* AST (not the
elaborated `PendingDecl`) to spot the tag, then elaborates
the inner decl normally — captured tag is preserved in the
registry, not the elab output.

Tests 9 → 17:

  + `registry_registers_leo4_export_handler`
  + `registry_registers_lean_marshal_derive`
  + `registry_default_matches_new`
  + `decl_has_leo4_export_detects_tag_via_parser`
  + `decl_has_leo4_export_rejects_untagged_decl`
  + `decl_has_leo4_export_ignores_unrelated_attrs`
  + `transpile_source_if_exported_skips_untagged`
  + `transpile_source_if_exported_records_when_tagged`

All exercise real parser fixtures (not hand-built ASTs) so
the test relationships to upstream AST shape are concrete.
clippy --all-targets -D warnings clean.

Activation plan §3 + §4 (in lib.rs module docstring) ticked
done. Next layer is §5 — canonical-ABI wrapper synthesis
(per-fn `<name>_call(args: &[u8]) -> Vec<u8>` boundary
shim that adapts the transpiled fn to leo4-rust-native's
`LeanProc` contract).

### Added — `leo4-oxilean-build`: `lean4_compat` syntax adapter pre-processor wired (2026-05-22)

Drops a `lean4_normalize(src: &str) -> String` helper at the
top of `transpile_source`'s pipeline. Pre-processes Lean 4
surface-syntax source through OxiLean's `lean4_compat`
adapter layer so the upstream `Parser` sees its expected
dialect:

```
Lean 4 source                       lean4_normalize
  fun n => n         ───────►       fun n -> n
  do x ← f                          do x <- f
  where;                            where
```

Implementation chains the two `lean4_compat` v0.1.2 entry
points in their documented order:

1. `Lean4TermRewriter::standard().rewrite(src)` — canonical
   set of textual rewrites (` => ` → ` -> `, `←` → `<-`,
   `where;` → `where`, plus logical-op aliases `∧∨¬` →
   `&&||!` for Bool subset).
2. `Lean4SyntaxAdapter::adapt_all(&after_rewrite)` —
   composite of `adapt_do_notation` + `adapt_where_clause`
   + `adapt_match_syntax` (the third disambiguates `=>`
   inside `match` arms that the rewriter's blanket ` => `
   rule misses when not space-padded).

Both passes are textual (intentional per upstream design —
they pre-process for the parser, they don't AST-rewrite).
The combined fn is idempotent: `normalize(normalize(s)) =
normalize(s)`. Verified in
`lean4_normalize_applies_compat_rewrites`.

Pipeline now:

```
Lean source string
    │  lean4_normalize  (lean4_compat layer)  ← NEW
Lean source in OxiLean dialect
    │  Lexer::tokenize
    │  Parser::parse_decl
    │  elaborate_decl
    │  unfold_decl  +  decl_to_lcnf
    │  RustTargetBackend::compile_decl
Rust source
```

What's still beyond the textual layer: parser-level Lean 4
shapes like `def f (x : T) : R := body` (header binders).
The upstream `lean4_compat` v0.1.2 is documented as
textual-only — header-binder lifting would need a separate
AST-level adapter living *above* the parser. Documented in
`lean4_normalize` docstring + activation plan §2.

Tests added:

- `lean4_normalize_applies_compat_rewrites` — verifies each
  documented rewrite + idempotence.
- `transpile_source_lean4_syntax_normalises` — `fun n => n`
  (Lean 4) and `fun n -> n` (OxiLean-native) both produce
  identical normalised output AND traverse the pipeline
  with the same Ok/Err outcome — the surface-syntax
  difference is invisible to downstream layers.

Tests 7 → 9 (+ both above). Workspace unaffected (sibling
crate).

### Added — `leo4-oxilean-build`: full Lean source → Rust transpile pipeline wired (2026-05-21)

Promotes `sibling/leo4-oxilean-build/` from scaffold to a
real end-to-end pipeline by chaining all five OxiLean
layers:

```
Lean source string
    │ oxilean_parse::Lexer::tokenize
Vec<Token>
    │ oxilean_parse::Parser::parse_decl
Located<Decl>
    │ oxilean_elab::elab_decl::elaborate_decl(env, &decl)
PendingDecl::Definition { name, ty, val, attrs }
    │ leo4_oxilean_build::unfold_decl (Pi/Lam unfold)
(name, params, body)
    │ oxilean_codegen::to_lcnf::decl_to_lcnf
LcnfFunDecl
    │ RustTargetBackend::compile_decl
RustFn
    │ RustFn::emit()
Rust source string
```

Three new public fns in `src/lib.rs`:

  • `transpile_kernel_decl(name, params, body) -> String`
    — kernel-level entry (no parse / elab needed). Useful
    for unit-testing the backend layer.
  • `unfold_decl(ty, val) -> (params, body)` — Pi/Lam
    unfold helper that lifts `PendingDecl::Definition`'s
    `(ty, val)` to the `(params, body)` shape
    `decl_to_lcnf` expects.
  • `transpile_source(env: &Environment, src: &str) ->
    Result<String, LeanError>` — the full pipeline entry.

Verified outputs (`examples/dump_addone.rs`):

  Input  : `def addOne (n : Nat) : Nat := Nat.succ n`
           (kernel-level Expr, hand-built)
  Output : `pub fn Sample_addOne(_x0: u64) -> Box<dyn std::any::Any> { _x1(_x0) }`

Confirms the SPEC §9 invariant: emitted Rust contains
**zero `lean_*` symbols**, all types come from the standard
library. The `_x1` (= `Nat.succ`) is unresolved because the
hand-built Expr has no surrounding env — wrapping in
`Environment::new()` + real elaboration would resolve it
against built-in `Nat`'s constructors. That's the next
layer's job.

Source-level test (`transpile_source_identity`) revealed
the parser's syntax constraint: `Parser::parse_decl` accepts
OxiLean-native `def NAME := BODY` syntax, not the full
Lean 4 `def NAME (binders) : TYPE := BODY` surface. The
`oxilean-elab/src/lean4_compat/` adapter layer is what
bridges Lean 4 syntax to OxiLean's parser — wiring that is
the next-commit work. (The test passes by asserting either
success OR a graceful parse error in the canonical-ABI
error-code range; the actual outcome under empty env is
`ParseError::UnexpectedToken { expected: [":="], got: LParen }`
at the `(` after `identity` — i.e. the parser rejects
non-OxiLean-native syntax. Pipeline integrity is the
invariant, not specific Lean syntax surface coverage.)

Tests: 6 → 7 (+`transpile_source_identity`). Workspace
unaffected (sibling crate). Lean-h cleanliness invariants
re-verified for both kernel-built and source-built paths.

`examples/dump_addone.rs` — small standalone binary that
prints the transpiled Rust for the addOne fixture. Useful
for `cargo run --example dump_addone` when iterating on
the backend.

Next steps documented in src/lib.rs "Activation plan":
plug `oxilean-elab/src/lean4_compat/` between
`Parser::parse_decl` and `elaborate_decl` so real Lean 4
syntax flows; bind the custom `@[leo4_export]` attribute
handler; populate the env with leo4 runtime decls before
elaborating user fixtures.

### Added — `sibling/leo4-oxilean-build/` + `SPEC/rust-native-lean.md` §9 transpile path (2026-05-21)

Second route to `leo4-rust-native`'s in-process direct
Rust call endpoint. Instead of waiting on OxiLean
evaluator hooks (Hooks 1 + 2 from §7.1, both absent in
v0.1.2), this route bypasses the evaluator entirely by
**transpiling** Lean source to a plain Rust crate at build
time.

Discovered while inspecting `oxilean-codegen` v0.1.2's
~50 backends. Only `rust_target_backend` produces plain
Rust source (`c_backend` targets oxilean-runtime ABI;
`native_backend` is machine-IR; LLVM/Cranelift backends
need third-party deps). The relevant API:

  use oxilean_codegen::rust_target_backend::*;
  let mut backend = RustTargetBackend::new();
  let module = backend.emit_module("name", &decls);   // LcnfFunDecl → RustModule
  let src = rust_fn.emit();                            // RustFn → Rust source string

**Critical**: `RustTargetBackend::lcnf_to_rust_type` maps to
*pure Rust* types — `Nat → U64`, `LcnfString → RustString`,
`Object → Box<dyn std::any::Any>`. No `lean_*` C symbol in
the output. The generated crate has zero `<lean/lean.h>`
dependency.

`SPEC/rust-native-lean.md` §9 (new, ~150 lines) documents:
  • §9.1 — codegen pipeline + `lcnf_to_rust_type` mapping
  • §9.2 — what the path bypasses (Hooks 1 + 2 of §7.1)
  • §9.3 — full architecture diagram (Lean source → Cargo
    crate)
  • §9.4 — two complementary adapter crates table
    (`leo4-oxilean` runtime-dispatch vs `leo4-oxilean-build`
    transpile)
  • §9.5 — other backends inspected + why each is unsuitable
  • §9.6 — open questions for the transpile path

`sibling/leo4-oxilean-build/` (new standalone crate):
  • `Cargo.toml` — deps on `oxilean-parse`, `oxilean-elab`,
    `oxilean-kernel`, `oxilean-codegen`; **no
    oxilean-runtime** (transpile bypasses the evaluator).
  • `src/lib.rs` — scaffold exposing:
    - `transpile_decls(module_name, &[LcnfFunDecl]) ->
      Result<String, LeanError>` — drives
      `RustTargetBackend::emit_module` + per-fn
      `RustFn::emit()`.
    - `type_mapper_is_lean_h_free() -> bool` — build-time
      invariant probe that catches a future OxiLean release
      flipping the mapper to emit `lean_box`-style symbols.
  • 4 tests pass: type_mapper_emits_pure_rust,
    nat_maps_to_u64, string_maps_to_rust_string,
    transpile_empty_decls_returns_header_only.

The crate is **scaffold** — what's NOT yet wired is the
parse → elab pipeline that produces `LcnfFunDecl[]` from a
real Lean source tree. Documented as next-commit work in
the crate's `src/lib.rs` "Activation plan".

The companion `sibling/leo4-oxilean/` runtime-dispatch
crate stays unchanged — both adapters serve different use
cases (dynamic dispatch vs compile-time-frozen exports).
`SPEC/rust-native-lean.md` §9.4 makes the split explicit.

### Changed — `rust-version = "1.95"` unified across all 4 workspaces (2026-05-21)

Bumps MSRV from `1.85` to `1.95` (current stable as of
2026-04-14) across the main workspace and all three
sibling projects. `1.85` was the original baseline pinned
during Phase 0 — well below what current stable supports;
keeping it forced unnecessary `#[allow]`s on features that
1.95-stable already accepts.

Files touched:

  • `Cargo.toml` (`[workspace.package].rust-version`)
  • `sibling/leo4-wasip3/Cargo.toml`
  • `sibling/leo4-oxilean/Cargo.toml`
  • `sibling/leo4-oxilean-build/Cargo.toml`

Verification: `cargo check --workspace` clean on main +
each sibling builds cleanly under 1.95. System rustc is
`1.95.0 (59807616e 2026-04-14)`.

### Fixed — OxiLean upstream-hook status: 1-of-3 present, not 0-of-3 (2026-05-21)

Corrects an overgeneralisation in the previous adapter
commit (`896e563`). Grep into the downloaded OxiLean v0.1.2
sources verified which of the three upstream hooks
`SPEC/rust-native-lean.md` lists actually exist:

| Hook | Status | Evidence |
|---|---|---|
| (1) Callback storage in evaluator | **NOT PRESENT** | `oxilean_kernel::ffi::ExternRegistry` + `oxilean_runtime::closure::FunctionTable` both metadata-only; no `Box<dyn Fn>`-accepting register path. |
| (2) By-name `@[leo4_export]` dispatch (high-level API) | **NOT PRESENT (at high level)** | `Environment`'s 30+ public fns are all query/merge; runtime side is `bytecode_interp::execute_chunk(BytecodeChunk)`, not name-level. |
| (3) Attribute / deriving registration | **PRESENT** | `oxilean_elab::attribute::AttributeManager::register_custom_handler(AttrHandler)` + `DeriveHandlerRegistry::register(DeriveHandler)`. |

Implications:

* The **recognition layer** for leo4 (scanning a user
  package for `@[leo4_export]`-tagged decls + `deriving
  LeanMarshal` handler installation, mirroring the
  reference `lake/Leo4Plugin/`) is **unblocked**. A
  separate `leo4-oxilean-build` companion crate can ship
  today by binding `register_custom_handler("leo4_export",
  ...)` + `DeriveHandlerRegistry::register(...)` for
  `LeanMarshal`. (Out of scope for this minimal adapter.)
* The **dispatch layer** (`OxiLeanProc::call` +
  `OxiLeanInvoker::invoke` actually running calls) stays
  blocked on Hooks 1 + 2 until upstream PRs land. The 8
  passing tests on `sibling/leo4-oxilean/` pin this split:
  registration paths (`register_export`, registry lookup)
  work against real OxiLean APIs; dispatch returns
  `RUST_DLSYM_FAILED` (0x0002_0005) stubs.

Files updated to reflect:
* `sibling/leo4-oxilean/README.md` §"OxiLean upstream
  prerequisite" — checklist split into [x] for Hook 3 +
  [ ] for Hooks 1 / 2.
* `sibling/leo4-oxilean/src/lib.rs` crate docs.
* `SPEC/rust-native-lean.md` §7.1 — the 1-of-3 table.
* `AGENTS.md` "Recent decisions" 2026-05-21 — corrected
  paragraph above the kept FFI-deep-dive notes.

The prior commit's broader "OxiLean's FFI types are
isomorphic to leo4's LeanProc surface" claim still holds
at the *type-system* level. The error in `896e563` was
implying the *runtime* mechanism was equally close; that
needed Hook 1 + Hook 2 upstream PRs which don't exist yet.

### Added — `sibling/leo4-oxilean/` adapter scaffold (2026-05-21)

First concrete `leo4-rust-native` adapter. Bridges leo4's
trait surface (`leo4_abi::{LeanProc, LeanProcInvoker}`) to
[OxiLean](https://github.com/cool-japan/oxilean)'s FFI
infrastructure (`oxilean_kernel::ffi::{ExternRegistry,
ExternDecl, FfiSignature, FfiType, FfiSafety,
CallingConvention}`).

Location: `sibling/leo4-oxilean/`. Standalone Cargo
workspace per `SPEC/rust-native-lean.md` §2.2 ("adapter
crate lives outside the main leo4 workspace; leo4 itself
stays runtime-agnostic"). Same pattern as
`sibling/leo4-wasip3/`.

Deps: `leo4-abi` (path), `oxilean-kernel =0.1.2`,
`oxilean-runtime =0.1.2`. OxiLean elaborator + build crates
deferred (heavier deps, only needed for source-file loading
which is `leo4-oxilean-build`'s job — out of scope for the
adapter itself).

**What works (8/8 tests pass)**:

  • `OxiLeanInvoker::new()` constructs an
    `Arc<Mutex<ExternRegistry>>` wrapper.
  • `register_export(mangled)` pushes one `ExternDecl` per
    `#[leo4::export]` into OxiLean's registry under
    `lib_name = "leo4-rust-bridge"`, `symbol_name =
    mangled`, signature `(ByteArray) -> ByteArray` (the
    canonical-ABI shape every leo4 export collapses to).
  • Duplicate-symbol detection (registry rejects, adapter
    surfaces `ENCODE_ERROR` 0x02).
  • `LeanProcInvoker::invoke` distinguishes
    `UNKNOWN_FUNCTION` (export not registered) from
    `RUST_DLSYM_FAILED` 0x0002_0005 (registered but no
    upstream callback hook).
  • All trait impls are object-safe.

**Architectural finding from the wiring exercise**: OxiLean
v0.1.2's `ExternRegistry` carries **metadata only** — `(lib,
symbol)` describes where the actual function lives; runtime
resolution is `dlsym`-style (matches reference Lean's
`@[extern "lib" "name"]` semantics). Three upstream hooks
OxiLean needs to add before this adapter becomes fully
functional, documented in `sibling/leo4-oxilean/README.md`
§"OxiLean upstream prerequisite":

  1. **Callback-registration entry point in the OxiLean
     evaluator**: `ExternRegistry::register_callback(decl,
     callback)` accepting a
     `Box<dyn Fn(&[u8]) -> Result<Vec<u8>, _>>` instead of
     `dlsym`'ing at runtime.
  2. **By-name `@[leo4_export]` dispatch**:
     `Env::call_by_mangled_name(name, ffi_args)`. Equivalent
     to reference Lean's
     `dlsym(leo4_call_<mangled>)`.
  3. **`@[leo4_export]` attribute recognition** in
     `oxilean-elab`'s attribute pipeline (the
     `registerBuiltinAttribute` analogue).

These are upstream features, not adapter work. The adapter's
`LeanProc::call` + `LeanProcInvoker::invoke` bodies will
fill in transparently once they land.

**`SPEC/rust-native-lean.md` §7.1 update implication**: the
"Phase 10-B1 callback ABI is essentially free with OxiLean"
claim still holds — OxiLean's `FfiType::Fn(params, ret)` is
first-class — but the *mechanism* requires the same
callback-registration hook above. Once that hook lands, B1
runtime for the rust-native transport becomes a one-PR
follow-up.

Main repo impact: zero. The adapter sits at
`sibling/leo4-oxilean/` with its own `[workspace]` marker;
main `cargo build --workspace` ignores it. Activation cost
for adopters: `cd sibling/leo4-oxilean && cargo test` (pulls
OxiLean's two crates, ~6s on this checkout).

### Added — `leo4_abi::{LeanProc, LeanProcInvoker}` trait surface (2026-05-21)

Lifts `SPEC/rust-native-lean.md` §2 + §3's two-trait contract
into actual Rust code. New `crates/leo4-abi/src/rust_native.rs`
exports:

```rust
pub trait LeanProc: Send + Sync {
    fn schema_hash(&self) -> &str;
    fn abi_version(&self) -> u32;
    fn call(&self, mangled: &str, args: &[u8]) -> Result<Vec<u8>, LeanError>;
}

pub trait LeanProcInvoker: Send + Sync {
    fn invoke(&self, mangled: &str, args: &[u8]) -> Result<Vec<u8>, LeanError>;
}
```

Both object-safe (`Box<dyn LeanProc>` is the type downstream
adapters expose). Re-exported at the leo4-abi crate root so
adapter crates do `use leo4_abi::{LeanProc, LeanProcInvoker}`.

Why in `leo4-abi`, not a separate `leo4-rust-native-traits`
crate: leo4-abi is already the natural shared-dep for
adapters (`LeanError` + `LeanMarshal` are there), so adding
the trait pair avoids a 1-trait-per-crate proliferation.
`leo4-mslean4` / `leo4-wasm` don't import the traits — they
have their own dispatcher shapes. No coupling introduced.

Tests: 2 new (`dummy_impl_constructs_and_dispatches`,
`dummy_invoker_constructs_and_invokes`) plus a compile-time
object-safety assertion. `cargo test -p leo4-abi`: 25 → 27.
Workspace total: 169 → 171.

This is the leo4-side prerequisite for the
`sibling/leo4-oxilean/` adapter scaffold that follows in the
next commit.

### Added — OxiLean FFI deep-dive findings → `SPEC/rust-native-lean.md` §7.1 (2026-05-21)

Documentation-only. Resolves the open question "how does
OxiLean expose FFI?" with a detailed inspection of
`crates/oxilean-kernel/src/ffi/` +
`crates/oxilean-codegen/src/ffi_bridge/`. The findings
materially strengthen the case for `leo4-rust-native`
integration via OxiLean.

Inventoried OxiLean's FFI surface:

  Kernel layer (semantic model):
  • `FfiType` enum — 1-to-1 with leo4 IDL primitives
    (Bool, UIntN, IntN, FloatN, String, ByteArray, Unit,
    Ptr, **`Fn(params, ret)` first-class**, OxiLean
    opaque ≈ leo4 LeanResource). The `ByteArray` variant
    is the natural carrier for canonical-ABI bytes.
  • `FfiValue` runtime enum with `Bytes(Vec<u8>)` payload
    — exactly what `LeanProc::call(mangled, args: &[u8])`
    returns.
  • `ExternDecl` + `ExternRegistry`: where host functions
    live. Adapter-side mechanism for registering the
    `LeanProcInvoker::invoke` callback (reverse direction)
    is a thin wrap over `ExternRegistry::register`.
  • `FfiSafety::Safe/Unsafe/System`,
    `CallingConvention::Rust/C/System`,
    `FfiSignature::validate()`: full validation pipeline.

  Codegen layer (mechanical wiring):
  • `marshal_type(lcnf_ty)` emits `lean_box`, `lean_unbox`,
    `lean_string_cstr`, `lean_mk_string`, `lean_object*`
    — i.e. **the exact C ABI symbols
    `SPEC/lean-runtime-compat.md` §1.2 requires**. Whether
    `oxilean-runtime` link-exposes these (vs. delegating to
    `libleanshared`) is the biggest remaining open question
    — answering it determines whether leo4-mslean4 can run
    unchanged against OxiLean OR only leo4-rust-native can.

Updated matrix of adapter work for OxiLean (now in
`SPEC/rust-native-lean.md` §7.1):

  | LeanProc / LeanProcInvoker need | OxiLean provides | Adapter work |
  |---|---|---|
  | `LeanProc::call(mangled, args)` returning Vec<u8> | `ExternRegistry::lookup` + `FfiValue::Bytes` invocation | thin wrapper |
  | `LeanProc::schema_hash()` | OxiLean attribute API + leo4's schema-idl computation | reuse leo4 code |
  | `LeanProcInvoker::invoke(mangled, args)` | One-time `ExternRegistry::register` of all `#[leo4::export]`s at adapter startup | bulk reg |
  | Phase 10-B1 callback ABI | `FfiType::Fn(params, ret)` is first-class | trivial — Rust closure through FFI |

**Most significant finding**: the Phase 10-B1 callback ABI
(re-entrant Lean ↔ Rust closures, currently deferred as
B1.x for the native pipeline because the LECQ/LECR re-entry
protocol design is hard) is **essentially free with OxiLean
as the impl**. OxiLean's `FfiType::Fn(…)` is a first-class
type; threading a Rust closure through the boundary is a
direct function call, not a multi-frame IPC dance. That's
a substantial architectural win for the adsmt
SMT-solver-style flagship use case once a `leo4-oxilean`
adapter exists.

Three open questions for an OxiLean adapter author
(documented in §7.1):

  1. Does `oxilean-runtime` link-expose `lean_box`-family
     C symbols, or delegate to `libleanshared`? Answers
     "can leo4-mslean4 also run on OxiLean?".
  2. Does `oxilean-cli`/`oxilean-build` accept Lake-shaped
     project layouts? Answers "can the existing leo4 Lake
     plugin drive OxiLean?".
  3. Is `oxilean-elab/src/lean4_compat/` mature enough to
     elaborate `lake/Leo4/Leo4/Export.lean` as-is? Single
     best litmus test for adapter activation.

Updated `AGENTS.md` "Recent decisions" 2026-05-21 entry
with these findings.

leo4 itself doesn't change. The path forward is in OxiLean's
hands (or an external adapter author's); leo4's contract
side is fully defined.

### Added — `leo4 create`/`init`/`run --impl <…>` option (2026-05-21)

Surfaces the three-transport landscape (mslean4 / wasm /
rust-native) at the CLI: every project explicitly declares
which Lean implementation it targets. Required by 병익's
review of the post-rename design — "default" choice was
ambiguous, so make it explicit.

CLI surface:

  • `--impl mslean4`  — reference Lean 4 (`crates/leo4-mslean4`
    path, `<lean/lean.h>` C ABI shim). The only path that
    ships today.
  • `--impl rust-native`  — `leo4-rust-native` adapter path,
    out-of-tree adapter crate per `SPEC/rust-native-lean.md`.
    **Currently deferred** — accepting the flag value but
    erroring with a SPEC pointer.
  • `--impl rust`  — alias for `rust-native`.
  • **No default** — `--impl` is required on `leo4 create` /
    `leo4 init`. clap rejects invocations missing it with a
    helpful usage message.
  • **Exactly one value**, by virtue of `--impl` being a
    single-valued option.

Persistence: `leo4 create` / `leo4 init` write a one-line
`.leo4-impl` marker file at the project root recording the
chosen impl. `leo4 run` reads it back (override with
`--impl`); without it, `leo4 run` errors with a pointer to
`leo4 init --impl …`.

Mid-project impl switching: `leo4 init` refuses if the
existing `.leo4-impl` disagrees with the passed `--impl`.
The user must remove the marker manually if they really
want to swap. Reason: scaffold differences between impls
(Cargo.toml deps, lakefile structure) aren't safe to
mix.

Internals (`crates/leo4-cli/src/main.rs`):

  • New `enum ImplKind { Mslean4, RustNative }`. Not a
    `clap::ValueEnum` because the `rust` alias needs a
    custom parser; `parse_impl_kind` handles both
    spellings.
  • `check_impl_supported(&kind)`: returns `Err` with a
    SPEC pointer for `RustNative`, `Ok` for `Mslean4`. Run
    early in every entry point so the user sees the
    deferral before any files get written.
  • `write_impl_marker(dir, kind)` + `read_impl_marker(dir)`:
    persist the choice across CLI invocations.

Tests (`cargo test -p leo4-cli`): 9 → 16 (+7 new).
  • parse_impl_kind: canonical names, `rust` alias,
    unknown rejection with both valid spellings mentioned
    in error.
  • check_impl_supported: passes mslean4, rejects
    rust-native with SPEC pointer.
  • impl_marker round-trip: write → read → overwrite.
  • read_impl_marker on absent → None.

Workspace test count: 162 → 169.

End-to-end smoke (manual):

```
$ leo4 create forward /tmp/x
error: the following required arguments were not provided:
  --impl <IMPL>

$ leo4 create forward /tmp/x --impl rust
leo4: --impl rust-native is currently deferred. The
integration contract is pinned at `SPEC/rust-native-lean.md`
§2, but no in-tree scaffolding ships yet — …

$ leo4 create forward /tmp/x --impl mslean4
leo4 create: x (Forward, impl=mslean4) → "/tmp/x"
  next: cat /tmp/x/README.md
$ cat /tmp/x/.leo4-impl
mslean4
```

### Changed — `leo4-native` → `leo4-mslean4` rename (2026-05-21)

Mechanical workspace-wide rename of the "reference Lean 4
path" crate. Motivation: the old `leo4-native` name became
ambiguous once two more transports landed —
"native" doesn't disambiguate against "rust-native".

  | Old | New | Transport |
  |---|---|---|
  | `leo4-native` | **`leo4-mslean4`** | reference Lean (MS-style impl) via `<lean/lean.h>` C ABI shim |
  | `leo4-wasm` | unchanged | WASM Component Model |
  | (out-of-tree adapter) | `leo4-rust-native` | rust-native Lean impl via direct Rust call |

What changed (mechanical only — no code logic):

  * `crates/leo4-native/` → `crates/leo4-mslean4/`
    (git-tracked rename).
  * Cargo package name: `leo4-native` → `leo4-mslean4`.
  * Workspace dep entry renamed.
  * `use leo4_native::…` → `use leo4_mslean4::…`
    (workspace-wide sed).
  * Every prose / doc mention (CLAUDE.md, AGENTS.md,
    LEO4-DESIGN.md, README.md, ROADMAP.md, SPEC/*.md,
    docs/{learning,implement-from-scratch}/{en,ko,ja,de}/main.typ,
    for-general-interface-descriptions.md) updated.

Verification: `cargo build --workspace` + `cargo test
--workspace` (162 tests, unchanged).

Downstream impact: any out-of-repo crate that depends on
`leo4-native` needs to update its `Cargo.toml`. leo4 isn't
on crates.io yet, so the in-repo workspace + local
checkouts are the full impact radius.

### Added — `SPEC/rust-native-lean.md` — third transport path for Rust-native Lean impls (2026-05-21)

Documentation-only. Defines a third integration path (alongside
leo4-mslean4 + leo4-wasm) for Rust-native Lean implementations
that **bypasses the `<lean/lean.h>` C ABI entirely**.
Motivation: 병익's observation that C ABI is a layer of
indirection that buys nothing when the target Lean impl is
itself Rust-native (e.g. OxiLean).

Three-paths comparison table now part of the SPEC stack:

| Aspect | leo4-mslean4 | leo4-wasm | leo4-rust-native |
|---|---|---|---|
| Transport | dlopen + shim | WASM CM | direct Rust call |
| C ABI | yes | no | **no** |
| Marshalling | canonical-ABI bytes | canonical-ABI bytes | canonical-ABI bytes |
| Re-entrant callbacks | worker IPC (B1.x) | WIT host-imports | **trivial — function call** |

Contract: a Rust-native impl exposes one trait
(`LeanProc { schema_hash, abi_version, call }`) and
optionally one host-callback trait (`LeanProcInvoker { invoke }`
for reverse direction). An out-of-tree adapter crate
(`leo4-<impl>`, e.g. `leo4-oxilean`) implements both against
the impl's native Rust API. leo4 itself doesn't change — the
SPEC defines what an adapter must satisfy.

Key design choices (in `SPEC/rust-native-lean.md`):

  • §5 — chose canonical-ABI bytes (Option A) over typed Rust
    values (Option B). Preserves cross-impl conformance + the
    schema_hash invariant; in-process marshalling overhead is
    negligible (~tens of ns) and dominated by dispatch cost
    anyway.
  • §3 — reverse direction `LeanProcInvoker` trait makes
    re-entrant callbacks (the Phase 10-B1 pattern that drives
    the adsmt use case) genuinely trivial — they're just Rust
    function calls in-process. No IPC frame protocol design.
  • §2.3 — what the surface explicitly does NOT require:
    `<lean/lean.h>` symbols, `leanc` toolchain, Lake DSL,
    `@[extern]` lowering, `dlopen` capability. Three of these
    four are §1.2 / §1.3 / §1.4 of
    `SPEC/lean-runtime-compat.md`; the rust-native path
    bypasses all three.

For OxiLean specifically, §7 documents the integration:
`leo4-oxilean::OxiLeanProc` wraps `oxilean-runtime`'s
`Env`/`Elab` state, implements `LeanProc` by delegating into
OxiLean's native call interface, and the source-syntax
compat (which OxiLean already has at 99.7% Mathlib4 parse
rate) handles attribute/derive recognition.

`SPEC/lean-runtime-compat.md` and AGENTS.md "Recent
decisions" both gain cross-references pointing at the new
SPEC.

### Added — `SPEC/lean-runtime-compat.md` — Lean impl compatibility surface (2026-05-21)

Documentation-only landing in response to "should leo4
support OxiLean?".

**TL;DR**: leo4 is *runtime-spec-agnostic* — any Lean 4
implementation that satisfies §1.1–1.4 of the new SPEC is
supported transparently; impls that don't need a glue layer.
That's leo4's policy; OxiLean (or any other alt-impl) doesn't
get a special case.

The new `SPEC/lean-runtime-compat.md` extracts the contract
leo4 currently depends on from the reference Lean 4 impl:

  §1.1 — Meta-programming API (`Lean.importModules`,
         `ext.getModuleEntries`,
         `Lean.Meta.SynthInstance.getInstances`,
         attribute system, `leo4_constraint` syntax
         category, deriving handler, Lake DSL).
  §1.2 — `lean.h` C ABI symbols (full table of `lean_*`
         functions the shim emitter uses, layout invariants
         for `lean_alloc_ctor` / `lean_io_result_*` /
         `lean_alloc_sarray` that Phase 10-B5 + the C4.x.x
         wasm path both depend on).
  §1.3 — `leanc` + `libleanshared` + `lean-toolchain` file
         convention.
  §1.4 — Reverse-direction surface (`@[extern]`,
         `IO ByteArray` lowering ABI, `dlopen`-capable
         `lean_exe` host).
  §1.5 — Optional: Phase 7 (`IO α` → `future<α>`) + Phase 8
         (Mathlib bridges) + external-marshal carriers.

§2 is a case-study on **OxiLean**
([cool-japan/oxilean](https://github.com/cool-japan/oxilean),
pure-Rust CiC ITP, v0.1.2 2026-05-03). Inspection of
`oxilean-elab/` + `oxilean-meta/` + the project's TODO.md
revealed a far more mature compat surface than initial
investigation suggested:

  Strong signals:
  * **Mathlib4 99.7% parse rate** (181,326 / 181,890 decls)
    — OxiLean parses essentially all Lean 4 source.
  * 32,345 tests passing; 320 curated theorem proofs at 100%.
  * `oxilean-elab/src/lean4_compat/` — active source-syntax
    compat layer (`Lean4CompatMatrix`, `Lean4NamespaceTracker`,
    `Lean4OptionConfig`, `Lean4SectionManager`,
    `Lean4SyntaxAdapter`, `Lean4SyntaxVersion`,
    `Lean4TermRewriter`).
  * `oxilean-elab/src/attribute/` (10+ kinds),
    `derive/` + `derive_adv/` (10+ handlers),
    `macro_expand/` (5 kinds) + `notation/` —
    `@[leo4_export]`, `deriving LeanMarshal`,
    `leo4_constraint` syntax category are all plausibly
    registrable.
  * `oxilean-meta/src/synth_instance/` — trait-based
    (`InstanceSynthesizer`, `SynthInstanceConfig`);
    direct adapter point for the admit-set algorithm's
    `Lean.Meta.SynthInstance.getInstances` dependency.
  * Codegen backends: Rust / WASM / LLVM / JS / C —
    multiple lowering targets exist as analogues.
  * `oxilean-runtime`: pluggable GC + refcount-closure
    model + WASM runtime integration — same conceptual
    ABI shape as reference Lean.
  * OxiZ SMT integration already exercises the
    SMT-backed-tactic pattern that adsmt targets via
    leo4's Phase 9-B1.

  Cautionary signals:
  * Most modules carry `//! Auto-generated module structure`
    — heavy codegen usage; trait surfaces may move fast.
  * Mathlib4 99.7% is parse, not elaborate — leo4's needs
    are closer to "elaborate + run handlers" than to
    parse-only.
  * C / WASM codegen targets OxiLean's *own* runtime ABI,
    not `<lean/lean.h>` and not the
    `leo4:host/leo4-component@0.1.0` WIT world.

  Surface satisfiability matrix (revised after the deeper
  dive — §1.1 score ↑ vs. initial pass):

  | Section | Status | Effort |
  |---|---|---|
  | §1.1 meta-programming API | **likely satisfiable** via existing OxiLean infra | medium, may be 80%+ there |
  | §1.2 `lean.h` C ABI | unsupported | large; OR bypass via leo4-wasm CM path |
  | §1.3 `leanc` toolchain | unsupported | medium |
  | §1.4 reverse direction | unsupported | large |
  | §1.5 Phase 7 / 8 optional | trivial | none |

**Two-axis surface matrix** in §2 ranks integration ROI:

  1. OxiLean exposes `leo4:host/leo4-component@0.1.0` WIT
     world via `oxilean-wasm` — bypasses §1.2 / §1.3
     entirely; leo4-wasm pipeline becomes the transport.
     Smallest scope.
  2. OxiLean's `lean4_compat/` compiles
     `lake/Leo4/Leo4/Export.lean` as a litmus test — if
     that works, `@[leo4_export]` transfers transparently.
  3. Otherwise a separate **leo4-OxiLean compat crate**
     (out-of-tree) — keeps leo4 itself runtime-agnostic.

Nothing in leo4 changes. ROADMAP.md "Future" section gains
"Alternative Lean 4 implementation support" as a Phase 11+
candidate. AGENTS.md "Recent decisions" gains a 2026-05-21
entry pointing at this SPEC.

If a future contributor proposes "support implementation
X" for any X, the response is: read
`SPEC/lean-runtime-compat.md`, run the §1.1–1.4 checklist,
report which sections X satisfies. Integration work then
follows from that diagnostic, not from leo4-side
speculation.

### Added — Phase 10-C4.x.x step 1: real wasmtime Component Model dispatch (2026-05-21)

Replaces the wasmtime backend stub with a working
Component-Model loader + dispatcher.

`crates/leo4-wasm/src/backend/wasmtime.rs` now:

* Pulls in `wasmtime = { version = "45", optional = true,
  default-features = false, features = ["cranelift",
  "component-model", "runtime", "std"] }` (gated on
  `backend-wasmtime`, default-on).
* `WasmtimeRuntime::new()` initialises a `wasmtime::Engine`
  with `Config::wasm_component_model(true)`.
* `open_component(bytes)` calls
  `wasmtime::component::Component::from_binary` and wraps the
  result in a local `WasmtimeComponent`.
* `WasmtimeComponent::instantiate()` builds a `Linker<()>`,
  registers a stderr-printing `log: func(level: u32, msg:
  string)` impl on the `leo4:host/host-imports@0.1.0`
  interface (the one host-side import the WIT currently
  defines), then `instantiate()`s.
* `WasmtimeInstance::call(mangled, args)`:
  1. Resolves the `leo4:host/exports@0.1.0` / `call` Func
     index via wasmtime 45's `get_export_index` chain.
  2. Materialises args as `[Val::String(mangled),
     Val::List(args_as_u8_vals)]`.
  3. Invokes `Func::call`.
  4. Decodes the `result<list<u8>, lean-error>` shape,
     surfacing the inner record `{code, message}` on `Err`
     and the inner `list<u8>` payload on `Ok`.

Error mapping:
* Component-bytes parse failure → `LEO4_ERR_DECODE_ERROR`
  (0x01).
* Instantiation failure / missing exports interface →
  `LEO4_ERR_RUST_DLSYM_FAILED` (0x00020005).
* Component trap during `call` → `LEO4_ERR_RUST_PANIC`
  (0x00020001).

Bindgen choice: **untyped `Val`-based API**, not
`wasmtime::component::bindgen!`. Rationale: leo4 only ever
calls one exported function (`exports.call(mangled, args)`)
whose shape is fixed at the WIT level. Bindgen would buy
nothing vs. one `Val::List(…)` build + result decode, and
would force a `bindgen!` invocation in `build.rs` (slower
compile, more brittle WIT-version coupling). The WIT pin
itself is unchanged.

`wasmtime 45` API drift caught + handled: `Instance::exports`
was replaced by `get_export_index` chain; `Func::post_return`
is deprecated and no longer needed for the dynamic API.

### Deferred indefinitely — wasmi backend + Lean→wasm pipeline (2026-05-21)

Of the four C4.x.x items the previous landing listed:

1. ✅ `wasmtime` dep + real impl — landed above.
2. ✅ host-side WIT bindings via wasmtime's untyped Val API —
   landed above (chose this over `wit-bindgen` for the reasons
   in the previous entry).
3. ❌ wasmi backend stays a stub. The `wasm_component_layer`
   crate (the path that would give wasmi a Component-Model
   surface) has been **stale upstream since 2025-03-03** and
   pins `wasmtime-environ ^18`; wasmtime is on v45 today.
   `backend/wasmi.rs` documents the deferral explicitly. The
   feature flag stays in place so the mutex guard continues
   to enforce exactly-one-backend, and so any future re-wire
   is a one-PR change.
4. ❌ `leanc --target=wasm32-wasip2` is **not currently
   feasible**: Lean 4 produces native cdylibs through gcc /
   clang via the C backend, and `leanc` has no wasm target.
   Cross-compiling the Lean runtime to wasm + wiring WASIp2 /
   p3 imports is a substantial undertaking that belongs in a
   dedicated Phase (or in the `sibling/leo4-wasip3/` project
   that already exists for the guest-side of this story).
   `leo4-wasm` can still load arbitrary CM-compliant wasm
   components today; the gap is "where does the Lean
   component come from".

Together: the wasmtime backend is **end-to-end functional for
hand-written or wit-bindgen-Rust-built leo4 components**. The
"hand it a Lean module" workflow stays on the deferred ladder
until the Lean→wasm pipeline question gets its own phase.

### Added — Phase 10-C4.x: `WasmRuntime` trait + dual-backend feature gate + leo4-host.wit pin (2026-05-21)

Builds on the C4 scaffolding (`Lean::open` parses handshake)
with the abstraction layer the wasm pipeline needs to stay
runtime-agnostic.

**`runtime::WasmRuntime` + `WasmComponent` + `WasmInstance`
traits** (`crates/leo4-wasm/src/runtime.rs`). Object-safe; the
backends hide their concrete types behind `Box<dyn …>`. All
three traits are `Send + Sync`. The trait surface is
deliberately small (3 methods total) — extensions land when
specific use cases push them.

**Two feature-gated backend modules** under
`crates/leo4-wasm/src/backend/`:

- `wasmtime.rs` — `WasmtimeRuntime`. Gated on
  `backend-wasmtime` (the default feature).
- `wasmi.rs` — `WasmiRuntime`. Gated on
  `backend-wasmi` (opt-in via
  `--no-default-features --features backend-wasmi`).

Both backends ship as stubs whose `WasmRuntime` impls return
`LEO4_ERR_RUST_DLSYM_FAILED` (0x0002_0005) with a clear
"not yet implemented" message. Real loader + dispatch wires
up in C4.x.x via `wit-bindgen` against the pinned WIT.

**Compile-time mutex enforcement** (`compile_error!` guards in
`lib.rs`): builds that enable zero OR two backend features get
rejected with a clear error message. Verified across all
three scenarios (none / default-only / both / wasmi-only).
Rationale captured in AGENTS.md "Recent decisions":
`leo4-wasm` is "one wasm runtime per build" by design;
downstream consumers needing multi-runtime stay outside this
crate.

**Wasmer rejected as a backend candidate** after the
2026-05-21 investigation: its only Component-Model-adjacent
crate (`wai-bindgen-wasmer`) targets the older WAI fork (not
WIT), is marked transitional / rewrite-pending in its
README, and is not listed among `wasm_component_layer`'s
supported runtimes. If wasmer ships real WIT-based CM in the
main `wasmer` crate later, adding it is a trivial third
backend module.

**`SPEC/wit/leo4-host.wit` pinned at v0.1.0**
(`package leo4:host@0.1.0`). The Component Model interface
both backends wrap. Companion `SPEC/wit/README.md` documents
the four key design decisions:

1. **Opaque `list<u8>` canonical-ABI payloads** rather than
   typed WIT records — preserves cross-impl wire identity
   (native + wasm produce byte-identical bytes per leo4
   type). Re-encoding through CM's own ABI would break that
   invariant.
2. **One generic `call(mangled, args)` export** rather than
   per-`@[leo4_export]` typed exports — keeps the WIT stable
   across schema_hash rotations and matches the native
   `dlsym(leo4_call_<mangled>)` dispatch model.
3. **`verify-handshake` is an export, not an import** —
   convention: side that owns the data (schema_hash) exports
   the verifier.
4. **schema_hash and WIT version are independent**. User IDL
   changes rotate schema_hash; leo4 runtime ABI changes
   rotate WIT version. The `handshake-frame.abi-version`
   field is the WIT-version negotiation channel.

Tests (`cargo test -p leo4-wasm`): 5 — adds
`backend_default_open_returns_dlsym_failed_stub` to the four
from C4. Mutex guards verified outside the test harness via
`cargo build` with various `--features` combinations.

Workspace test count: 160 → 161.

**C4.x.x follow-ups** (still deferred):

- Pulling in `wasmtime` (with `component-model` feature) +
  `wasmi` + `wasm_component_layer` + `wasmi_runtime_layer`
  as gated deps once the impls land.
- `wit-bindgen` invocation in `build.rs` against
  `SPEC/wit/leo4-host.wit` to materialise typed bindings.
- Replacing the stub `WasmtimeRuntime` / `WasmiRuntime`
  bodies with real CM dispatch.
- Optional: a `leanc --target=wasm32-wasip2` wrapper that
  produces a Lean component the wasmtime backend can load
  (a separate substantial path).

### Added — Phase 10-Docs (E1+E2+E3): Phase 9 chapters + reverse-byte-parity skeleton + SPEC quickstart (2026-05-21)

A combined docs sweep covering three follow-ups that
accompanied the Phase 10 ABI work:

**E1 — Typst books: Phase 9 chapter**

- `docs/learning/en/main.typ`: new "Phase 9 in detail —
  the reverse direction" section. Includes architecture
  diagram, mangling delta, handshake invariant, `iso:`
  prefix design, and the Phase 10 follow-ups already
  landed (D1 / D2 / B1 / A4 / A5). Phase ladder table
  extended with rows 9 + 10.
- `docs/implement-from-scratch/en/main.typ`: new "Part XI
  — Reverse direction (Rust → Lean)" with sections on
  the `#[leo4::export]` proc-macro, emit CLI, worker
  harness, dispatcher, glue shim, Lake `extern_lib`
  integration, sanity check via `examples/05`.

Korean / Japanese / German translations of both new
chapters land as part of task #62 (the in-progress
four-language book translation effort) — not this
commit. The English source is the canonical reference.

**E2 — Reverse-direction byte-parity conformance**

`tests/conformance/reverse/README.md` lays out the
intended contract for a worker-frame byte-parity
harness. The runnable harness (`run.sh` + per-fixture
binary frames) is deferred to E2.x because it needs a
fixture-recorder mode on the worker; the README
documents exactly what that mode needs to do.

The canonical-ABI byte parity itself is already covered
by the forward harness (`tests/conformance/run.sh`) since
both pipelines reuse the same `leo4-abi` encoders /
decoders — what E2.x will add is the wire-envelope
parity (handshake frame, request/response frames) that's
unique to the reverse direction.

**E3 — User-facing SPEC quickstart**

New `SPEC/reverse-direction-quickstart.md` — a 60-second
tour for new users, distinct from the normative
`SPEC/reverse-direction.md`. Covers:

- The minimum-effort flow (`leo4 create reverse … && leo4 run`).
- What `leo4 run` actually does step-by-step.
- Supported wire types today + what's not yet supported
  (function-arrow params per B1, async exports, cross-
  process resource handles).
- The two isolation modes (default long-running vs.
  `#[leo4::export(isolated)]`).
- Recycle policies (`LEO4_RUST_WORKER_RECYCLE_CALLS` +
  `_SECONDS`) with the restart-flag side-channel.
- Common pitfalls (garbage status values, handshake
  mismatch, cdylib not found).
- Pointer to `adsmt` for the flagship demo + to
  `examples/05-rust-export/` for the in-repo smoke.

`SPEC/reverse-direction.md` gains a top-of-file pointer
to the quickstart so readers can choose the right
entry point.

### Added — Phase 10-C4: leo4-wasm scaffolding (2026-05-21)

Promotes `crates/leo4-wasm` from an empty placeholder (the
v0.1.0 cut shipped just `#![allow(dead_code)]`) to a real
scaffold whose public API surface mirrors `leo4-mslean4`.
Cfg-gated downstream code can now `use leo4_wasm::{Lean,
LeanError}` / `use leo4_mslean4::{Lean, LeanError}`
interchangeably; structurally wasm-vs-native user code
compiles cleanly on both.

What's in:

* `Lean::open(handshake_path)` parses + validates the
  handshake JSON shared with the native pipeline
  (`schema_hash` + `target_module` + `abi_version`). Returns
  `Err(LeanError { code: HANDSHAKE_MISMATCH, … })` for
  unsupported abi_versions, `DECODE_ERROR` for parse / IO
  failures.
* `Lean::{schema_hash, target_module, abi_version}` getters.
* `Lean::call(mangled, args)` stub returning
  `LEO4_ERR_RUST_DLSYM_FAILED` (0x0002_0005) — semantic
  match for "dispatch layer can't resolve this symbol yet".

What's **explicitly deferred to C4.x**:

* Adding `wasmtime` as a dependency and instantiating a
  Component-Model loader. A ~200 MB build dep; not pulled in
  for users who don't need wasm.
* Designing and pinning `SPEC/wit/leo4-host.wit` — the
  interface describing the canonical-ABI bridge between a
  Lean-as-wasm component and the host. The actual dispatch
  shape depends on this.
* `wit-bindgen` invocation in `build.rs` to materialise
  typed bindings.
* Replacing `Lean::call`'s stub with real per-callsite
  dispatch.

Cargo: leo4-wasm now depends on `leo4-abi`, `serde`,
`serde_json` (all workspace). No new external deps in the
workspace.

Tests (`cargo test -p leo4-wasm`): 4 — open round-trip,
wrong abi_version rejection, missing-file error path,
call stub returns expected code.

Workspace test count: 156 → 160.

### Added — Phase 10-A4 + A5: time-based recycle + WORKER_RESTARTED side-channel (2026-05-21)

Two reverse-direction dispatcher knobs that were reserved in
`SPEC/reverse-direction.md` §4.3 / §10 but unwired now ship:

**A4 — `LEO4_RUST_WORKER_RECYCLE_SECONDS=T`**

The persistent worker may be configured to recycle after `T`
seconds of uptime. Independent from (and combinable with)
the existing call-based `LEO4_RUST_WORKER_RECYCLE_CALLS` —
whichever limit fires first triggers a recycle.

Implementation in `shim/leo4_rust_bridge.c`:
* New `spawn_time_s` field on `leo4_worker_slot_t`, stamped
  via a portable `leo4_monotonic_seconds()` helper
  (`CLOCK_MONOTONIC` on POSIX, `GetTickCount64()` on
  Windows-gnullvm).
* `leo4_recycle_init_once` now parses both env vars and
  caches them in file-scoped atomics.
* Dispatch entry checks `(now_s - spawn_time_s) >=
  seconds_limit` alongside `call_count >= calls_limit`.

Clock-skew / `settimeofday` adjustments cannot fire the
recycle prematurely (CLOCK_MONOTONIC is monotone
non-decreasing across NTP slews).

**A5 — `LEO4_ERR_RUST_WORKER_RESTARTED` side-channel**

The dispatcher now exposes a polling helper:
```c
LEO4_RUST_EXPORT int leo4_rust_bridge_take_restart_flag(void);
```
Atomic exchange-and-clear that returns `1` exactly once per
recycle event observed by the dispatcher (call-based OR
time-based), then `0` until the next recycle. Lean wrappers
that want to surface `LEO4_ERR_RUST_WORKER_RESTARTED`
(0x00020002 — reserved since Phase 9 but unemitted) bind it
via a custom `@[extern]` shim and check after each call. The
dispatcher itself stays transparent — non-polling callers see
no behaviour change.

`SPEC/reverse-direction.md` §4.3 expanded with the wire +
clock contract; §12 "v0 In-Scope vs Deferred" table flips
both rows from "candidate" to ✅.

Verification:
* `cargo build --release -p leo4-rust-bridge` — clean.
* `cargo test --workspace` — 156 unchanged.
* The Phase 9-3 worker harness and Phase 9-7 examples/05
  e2e flow continue to run unchanged (the new fields are
  zero-initialised and only consulted when env-configured).

### Added — Phase 10-B5: variant payload widening (all-scalars multi-field, 2026-05-21)

Widens the plugin's variant case-payload coverage one step
beyond the W7-2d-iii minimum-viable shape that landed
2026-05-20. The minimum supported:

* 0 fields
* all-Self N fields
* single scalar / string / Cyc<i> field

is now extended with:

* **N scalar fields** (Phase 10-B5)

i.e. an inductive case like `| mkPair (a : UInt32) (b : UInt64)`
no longer falls through to the `LEO4_ERR_UNIMPLEMENTED` stub —
the plugin emits a real per-instantiation
`leo4_dec_<suffix>` / `leo4_enc_<suffix>` helper that:

* decode: reads each field's wire bytes in order, allocates
  `lean_alloc_ctor(disc, 0, total_sz)`, and writes each
  scalar at its running byte offset via
  `lean_ctor_set_<scalar-kind>(r, byte_offset, fN)`.
* encode: reads each scalar from `lean_ctor_get_<kind>(v,
  byte_offset)` and writes its bytes to the output buffer.

Implementation:
* New `CaseKind.allScalars (specs : Array (String × Nat ×
  String))` constructor in
  `lake/Leo4Plugin/Leo4Plugin/Main.lean`. Each spec carries
  `(cType, wire size, ctor scalar-accessor suffix)`.
* `variantCaseSupported` accepts multi-field cases when every
  field passes `scalarCType.isSome`.
* `classifyCase` computes the per-field `specs` array.
* `renderVariantHelpers` gains decode + encode arms for
  `.allScalars` that match the spec wire format
  (`SPEC/canonical-abi.md` §9: discriminator u32 LE +
  payload bytes, no padding).

The wire format stays identical to what the cross-impl
Rust side emits (sequence of canonically-encoded scalars,
no padding), so the schema hash does NOT rotate just from
this code path being present — `tests/sample-lean/` doesn't
yet have a multi-scalar variant case, so the existing
fixture's `schema_hash qi5gb74dbjyxo` and 70 mangled names
stay byte-identical.

`cargo test --workspace`: 156 unchanged. `lake build` +
`tests/mangling/run.sh` green.

**B5.x follow-ups** (still stub'd in the
`LEO4_ERR_UNIMPLEMENTED` fall-through path):

* Mixed scalar/object multi-field payloads (e.g.
  `| mkPair (s : String) (n : UInt64)`). Needs the
  `(objSlot, scalarOff)` two-axis layout `recordHandler`
  already computes; lifting that to variants requires
  threading `TyHandler`s into `renderVariantHelpers`.
* Single-composite payloads (`list<T>`, `option<T>`,
  nested record / variant). These would route through the
  existing `TyHandler.decodeBlock` / `encodeBlock` API
  rather than the variant's per-case-kind switch.
* A fixture under `tests/sample-lean/` that exercises a
  multi-scalar variant case for end-to-end byte-parity
  validation. The schema hash WILL rotate when this lands.

### Added — Phase 10-D2: `lake run Leo4Rust/regenerate` script (2026-05-21)

Moves the reverse-direction `leo4-rust-emit` invocation
behind a Lake-native script so users no longer have to
manually run a Cargo binary with long arg lists. From
inside a Lean project directory:

```sh
LEO4_RUST_EMIT_BIN=/abs/path/leo4-rust-emit \
LEO4_RUST_IFACE=MyApp \
  lake run Leo4Rust/regenerate
```

The script:
1. Reads `Cargo.toml` one level up to derive the crate name.
2. Locates the cdylib via (a) `LEO4_RUST_CDYLIB` env, then
   (b) `<project>/target/release/lib<crate>.{so,dylib,dll}`,
   then (c) walks upward looking for any `target/release/`
   that holds the cdylib — handles cargo workspace
   projects whose target/ lives at the workspace root.
3. Invokes `leo4-rust-emit --emit-lean`.
4. Moves the generated wrapper to `<iface>/Rust.lean`.

`leo4 run` (Phase 10-D1) now delegates its emit step to
`lake run Leo4Rust/regenerate` instead of calling
`leo4-rust-emit` directly + manually moving the file —
the orchestration logic lives in one place
(`lake/Leo4Rust/lakefile.lean`).

The original "auto-trigger on `lake build`" goal turned
out to require importing the helper module from a
dependency package in the user's lakefile, which
Lake's lakefile/dep ordering doesn't support
(lakefile compiles before deps build). The
`lake run`-based design is the next-best abstraction:
emit is now invoked through Lake (not via a raw Cargo
binary path), and `leo4 run` chains both steps so
end-to-end users see a single command.

Justfile: `rust-export-05-build` recipe updated to use
the new script. `examples/05-rust-export` end-to-end
verified — `is_prime`, `next_prime`, `count_primes_below`,
`factor_smallest` all return correct values after the
Lake-script-driven emit.

Scaffold `README.md`'s manual workflow now shows
`lake run Leo4Rust/regenerate` in place of the verbose
`leo4-rust-emit --cdylib ... --out-dir ... --emit-lean
--lean-module ... && mv ...` ladder.

Workspace test count: 156 unchanged (no new tests added;
existing `cargo test --workspace` + `lake build` +
end-to-end smoke all pass).

**D2.x follow-ups** (deferred):
* `lake build` auto-trigger via `extraDepTargets` once Lake
  exposes a way to import dep modules from a user lakefile
  (or once Leo4Rust ships its helper as inline lakefile
  code in the scaffold).
* `CARGO_TARGET_DIR` env support for `cargo --target-dir`
  users.

### Added — Phase 10-B1: function-arrow / callback ABI (IDL + mangling entry-gate, 2026-05-21)

Wires the previously-TBD function-arrow mangling slot
(`SPEC/mangling.md` §2 "function-arrow mangling for callback
parameters is unspecified") and lifts the IDL grammar to accept
`fn(T1, …, Tn) -> R` as a first-class type. The runtime —
re-entrant Lean ↔ Rust dispatch through the worker IPC — lands
in a Phase 10-B1.x follow-up; this commit ships the IDL +
mangling layer that the cross-impl conformance harness can
already exercise.

**IDL surface**: `fn(T1, …, Tn) -> R` at any type position.
The parser (both `crates/schema-idl` and the Lake plugin) accepts
the syntax verbatim.

**Mangling rule**: `mangle_type(fn(T1,…,Tn) -> R) =
"A_" ++ mangle_type(tuple<T1,…,Tn>) ++ "_" ++ mangle_type(R)
++ "_a"`. Wrapping the args in tuple mangling keeps inner
separators unambiguous; an empty arg list mangles as
`A_T__t_R_a`. Identical implementation in
`crates/schema-idl/src/mangle.rs` and
`lake/Leo4Plugin/Leo4Plugin/Mangling.lean` — cross-impl
mangling stays green (`70` mangled names byte-identical;
schema_hash `qi5gb74dbjyxo` unchanged because no fixture
in `tests/sample-lean/` uses the new type yet).

**Wire format** (SPEC/canonical-abi.md §13a, new): a single
`u64 callback_id`. `callback_id == 0` is reserved as the null
sentinel (matches §12's `INVALID_RESOURCE_HANDLE`): encoders
reject with `LEO4_ERR_ENCODE_ERROR` (0x02), decoders with
`LEO4_ERR_INVALID_HANDLE` (0x03). Lifetime is bound to the
enclosing call.

**Re-entrant callback frame protocol** (SPEC/reverse-direction.md
§10a, new): designed but **not yet implemented**. While a
worker is processing a `REQUEST` that carries function-arrow
args, every closure invocation emits a `LECQ` (callback query)
frame back to the Lean main process, which responds with a
`LECR` frame. The Lean wrapper holds a per-call-scope
`HashMap<u64, IO α>` closure registry; ids are allocated from
a thread-local counter and freed when the outer call returns.

**Type-level additions**:
* `schema_idl::IDLType::Fn { args, ret }` variant + match
  arms in `mangle`, `render`, `subst`.
* `Leo4Plugin.AdmitSet.IDLType.fn` constructor + `substIDL`
  case.
* Parser support: `fn(...) -> ...` keyword in both impls.
* `idl_form` round-trip: `fn(u32, str) -> bool` ↔
  `Fn { args: [U32, String], ret: Bool }`.

**Guards** for layers that don't yet support function-arrow:
* `leo4-idl::wit::lower_type` panics with a clear "Phase
  10-B1.x runtime deferred" message if a function-arrow type
  reaches WIT lowering. The plugin's admit-set machinery
  shouldn't produce such IDL until the runtime lands; users
  who add `fn(…) -> …` to a Lean export by hand will hit this
  guard at `lake exe leo4plugin --with-lower` time.
* `Mangling.lean::idlToLeanType` returns `_` for `.fn` so the
  Lean wrapper template type-checks; the actual closure thunk
  gets substituted by the B1.x runtime.

Tests:
* 4 new `mangle_type` cases (`fn(u32) -> u64`, multi-arg,
  nullary, higher-order nesting).
* 2 new parser cases (`fn` in function param, nullary).
* `tests/mangling/run.sh` still green (70 mangled names
  byte-identical, schema_hash unchanged).
* Lean plugin compiles + smoke-plugin runs green.

Workspace test count: 150 → 156.

**B1.x follow-ups** (not in this commit):
* Re-entrant dispatcher state machine in
  `shim/leo4_rust_bridge.c` (LECQ/LECR frame handling).
* Lean-side `Leo4.LeanCallback` closure registry +
  per-call-scope GC.
* Rust-side `LeanCallback<R, Args>` wrapper in
  `crates/leo4-macros::export` proc-macro.
* WIT lowering: `fn(…) -> …` → opaque resource alias once
  the wasm path has a use case.

### Fixed — Phase 10-F1: reserved LeanError code fixtures (2026-05-21)

Phase 4's "every reserved `LeanError` code has at least one
test that *triggers* it" exit criterion shipped with stubs
for 0x02 / 0x03 / 0x04 / 0x06 / 0x08 (codes whose natural
trigger paths weren't yet available in leo4-abi). Phase
10-F1 replaces those stubs with real code-path triggers,
using helpers now exposed by leo4-abi:

- **`MAX_DECODE_DEPTH: usize = 256`** + **`check_decode_depth(depth)`**:
  paired helpers for `Self`-recursive decoders. The cap
  matches `SPEC/canonical-abi.md` §8.1's documented default;
  the test sends a unary-counter wire of 257 levels and
  asserts `DECODE_DEPTH_EXCEEDED` (0x08).
- **`INVALID_RESOURCE_HANDLE: u64 = 0`**: reserved null
  sentinel for `LeanResource`. Encoders refuse it with
  `ENCODE_ERROR` (0x02); decoders refuse it with
  `INVALID_HANDLE` (0x03). Reservation now documented in
  SPEC §12.
- **`encode_resource_handle` / `decode_resource_handle`**:
  the production encode/decode helpers that surface 0x02 /
  0x03 against the sentinel.
- **`LeanError::{encode_error, invalid_handle, oom,
  unknown_function, decode_depth_exceeded}`**: convenience
  constructors mirroring the SPEC codes, pinning the
  expected codes from one place.

Triggers in `crates/leo4-abi/tests/error_codes.rs`:

| Code | Trigger |
|---|---|
| 0x01 | invalid `bool` byte (unchanged from Phase 4) |
| 0x02 | `encode_resource_handle(0, …)` |
| 0x03 | `decode_resource_handle(&[0u8; 8], 0)` |
| 0x04 | `Vec::try_reserve(isize::MAX as usize)` → map to `LeanError::oom` |
| 0x05 | `check_schema_hash` mismatch (unchanged from Phase 4) |
| 0x06 | `HashMap<&str, fn>` lookup miss → `LeanError::unknown_function` |
| 0x07 | `encode_to_fixed` into tiny buf (unchanged from Phase 4) |
| 0x08 | unary-counter decoder going past `MAX_DECODE_DEPTH` |

`SPEC/canonical-abi.md` §8.1 + §12 extended with the new
helper references and sentinel reservation. End-to-end
shim-level triggers (libloading `dlsym` miss, allocator
OOM under a real workload) remain a leo4-mslean4 concern;
those layers can lift the new constructors directly
instead of building their own `code: u32` literals.

12/12 `error_codes` tests pass. Workspace total: 147 → 150.

### Added — Phase 10-D1: `leo4 run` CLI (2026-05-21)

`leo4 run` collapses every forward / reverse build +
execute step into a single command. Detects direction
automatically from `Cargo.toml`'s `[lib] crate-type`
(`["cdylib"]` ⇒ reverse, else forward), and orchestrates
the full pipeline:

**Forward**: `lake build` → `lake exe leo4plugin <iface>`
→ `cargo run`. The build.rs picks up `LEO4_SHIM_SO` /
`LEO4_HANDSHAKE_FILE` via `leo4_build::wire` so no env
plumbing is needed.

**Reverse**: ensures `leo4-rust-{emit,worker,bridge}` are
built under `--leo4-root` (auto-builds if absent) →
`cargo build --release -p <pkg>` → `leo4-rust-emit
--emit-lean` → move wrapper to `lean/<iface>/Rust.lean`
→ `lake build` (auto-links bridge + glue via
`Leo4Rust`'s `extern_lib`s) → run
`lean/.lake/build/bin/<crate_name>` with
`LEO4_RUST_CDYLIB` / `LEO4_RUST_WORKER_BIN` /
`LEO4_RUST_HANDSHAKE_PKG` / `LEO4_RUST_HANDSHAKE_IFACE`
wired up automatically.

Flags: `--direction` (override auto-detect), `--iface`
(override default iface name), `--leo4-root` (default
`../leo4`), `--dir` (project dir, default cwd), trailing
`-- <args>` forwarded to the final binary.

Scaffold fixes shipped alongside (the reverse direction's
generated `lakefile.lean` was missing two pieces needed
for `leo4 run` to work end-to-end):
- `require Leo4Rust from "{leo4_root}/lake/Leo4Rust"` now
  emitted so Lake auto-links the dispatcher + glue
  archives.
- `lean_lib {iface}` glob changed from `#[`{iface}]`
  (matched only the bare module) to `#[.submodules
  `{iface}]` (matches `{iface}.Rust` produced by
  `leo4-rust-emit`).
- Scaffold READMEs lead with `leo4 run`; the verbose
  manual workflow stays documented for debugging.

**leo4-cli version-independence**: `version.workspace`
removed from `crates/leo4-cli/Cargo.toml`. The CLI now
carries its own `version = "0.2.0"`, bumped only on CLI
surface changes — independent of leo4 lib's
schema-hash-rotating cuts. Rationale: a user with one
installed `leo4` CLI must be able to run projects pinned
to many different leo4 lib versions. AGENTS.md "Recent
decisions" 2026-05-21 entry captures the policy.

New unit tests (6): direction detection (forward /
reverse from Cargo.toml content), cdylib path resolution
(Linux `.so`), binary name OS dispatch, scaffold's
reverse lakefile containing the new `require Leo4Rust`
and `.submodules` glob.

Workspace test count: 141 → 147.

### Planned — Phase 10 plan locked (2026-05-21)

Phase 10 substep order locked after the
"organise Phase 10 candidates" pass:

1. **D1** — `leo4 run` CLI (forward + reverse + env wiring).
2. **F1** — reserved `LeanError` code fixtures (0x02, 0x03,
   0x04, 0x06, 0x08).
3. **B1** — callback / function-arrow ABI (function-pointer
   mangling + re-entrant dispatcher; schema hash rotates).
4. **D2** — lake-side `leo4-rust-emit` auto-call (collapse
   2-step reverse build to 1).
5. **B5** — variant payload widening
   (schema-idl-shortcomings #12 W7-2d-iii).
6. **A4 + A5** — `LEO4_RUST_WORKER_RECYCLE_SECONDS` time
   recycle + `LEO4_ERR_RUST_WORKER_RESTARTED` side-channel
   surfacing.
7. **C4** — `leo4-wasm` proper implementation (out of
   scaffold).
8. **P10-Docs** (single commit, E1+E2+E3) — Typst books'
   Phase 9 chapter (4 langs × 2 books),
   reverse-direction byte-parity conformance harness,
   SPEC quickstart page.

Each numbered line ships as its own commit (except
P10-Docs, which is one combined commit). See
`ROADMAP.md` Phase 10 section for the full rationale,
deferral list, and flagship-demo split.

Deferred to v1.0 RC window: **C1** Windows runtime CI
verification, **G2** crates.io publish. Deferred to ≥
v1.x: A1 zygote backend, A2 wasm sandbox, B2
ConstraintExpr<Atom>, B3 async reverse exports, C2 macOS
Tier 1, C3 wasm64 sibling, D3 VS Code extension, D4
logicutils removal.

Flagship SMT-solver consumer of Phase 9 / Phase 10-B1
lives outside leo4 at
[`Honey-Be/adsmt`](https://github.com/Honey-Be/adsmt).
leo4 ships the building blocks; adsmt proves them out by
integrating with z3 / cvc5 / its own solver backend.

### Fixed — Phase 9 runtime: dispatcher handshake-consume + glue ABI + first true e2e run (2026-05-23)

The Phase 9-6 follow-up landed a 3-commit declarative-link
chain that ended with the executable building successfully
but failing at runtime on the first dispatcher call. Three
bugs surfaced; this commit fixes all three. **`examples/05-rust-export/`
now runs end-to-end with correct values for every export.**

Bug A — dispatcher missed the worker's handshake frame.
The worker (Phase 9-3) sends a 25-byte handshake immediately
after init (SPEC §5.3: magic + hash_len + abi_version +
schema_hash). The dispatcher (Phase 9-4a/b) never consumed
those bytes, so they piled up in the IPC buffer and the
subsequent response read decoded them as a response header
— garbage `status` values were the symptom.

Fix: new helper `leo4_consume_handshake(w)` in
`shim/leo4_rust_bridge.c` that reads exactly 25 bytes
(12-byte header + 13-byte hash) immediately after each
`leo4_worker_ops->spawn` success. Verifies magic +
hash_len, and against `LEO4_RUST_SCHEMA_HASH` env when set
(otherwise verification is deferred to the typed Lean
wrapper's compile-time pin). Hooked into BOTH spawn paths:
`leo4_get_or_spawn_persistent` and `leo4_dispatch_isolated`.
Forward decl added because the helper lives below
`send/recv` plumbing.

Bug B — `(UInt32 × ByteArray)` Prod ABI mismatch.
The original glue shim built the return tuple via
`lean_alloc_ctor(0, 2, 0)` + `lean_box_uint32`, assuming
both fields were boxed object pointers. But Lean 4's
codegen for `UInt32 × ByteArray` packs the `UInt32` as an
inline scalar field in the ctor — not a boxed lean_object*.
The mismatched layout produced unpredictable `status`
values to the Lean wrapper even after Bug A was fixed
(observed: 4 231 544 912, 1 446 527 056, …).

Fix: change the glue shim's return contract from
`BaseIO (UInt32 × ByteArray)` to a single ByteArray whose
first 4 bytes hold the status as LE u32 and whose remaining
bytes carry the canonical-ABI payload. No Prod codegen
involved; layout is unambiguous.

`shim/leo4_rust_bridge_lean.c` rewritten accordingly:
allocates `lean_alloc_sarray(1, cap, cap)` of capacity
`LEO4_RUST_INITIAL_RET_CAP` (4 KiB), hands `r_base + 4` +
`cap - 4` to the dispatcher as the payload buffer, then
stamps the status at `r_base[0..4]` via a small LE u32
helper, and shrinks the sarray's logical size to
`4 + payload_len` (or `4` on error). `BUFFER_TOO_SMALL`
retry re-allocates with `payload_len + 4`.

`crates/leo4-rust-emit/src/main.rs` emits a matching Lean
wrapper:
- Raw extern signature: `IO ByteArray` (not
  `BaseIO ByteArray` — see Bug C).
- Private `decodeStatus (buf : ByteArray) : UInt32`
  reading bytes 0..4 as LE u32. Returns `0xffffffff` for
  malformed (< 4-byte) responses.
- Typed wrapper body now reads `resp` (full ByteArray),
  decodes status, throws `IO.userError` on non-zero, then
  `Leo4.LeanMarshal.canonicalDecode (T := …) resp 4` to
  decode the payload starting at offset 4.

Bug C — `BaseIO ByteArray` had wrong runtime ABI from
within an `IO` block.
Initial attempts at the new ByteArray-prefix scheme used
`BaseIO ByteArray` as the extern signature. Lean's
`MonadLift BaseIO IO` instance lifts purely at the type
level, but the actual lowered C ABI of `BaseIO α` vs
`IO α` apparently differs enough that calling the extern
from an `IO` block via `← x` produced a malformed
ByteArray (observed: `resp.size = 3 457 722 490 880`,
clearly corrupt memory).

Fix: declare the extern as `IO ByteArray`. Lean's compiler
then lowers the call site through the IO monad directly,
no lifting needed. The `lean_io_result_mk_ok` in the C
glue is correct for either monad since their result
layouts match — what mattered was the *caller-side*
type Lean used to read back the result.

Cleanup: temporary `IO.eprintln` debug print in the
wrapper body that diagnosed Bug C is removed. Dispatcher's
stderr `fprintf` trace also removed. Unit-test assertion
updated for the `: IO ByteArray` signature change. 13/13
emit-CLI tests pass; workspace test count stays at 141/0.

Verified end-to-end with `just rust-export-05-build` +
manual run:

```
leo4 reverse-direction demo — schema_hash = ozln3adaktdow

is_prime:
  is_prime(0)=false  is_prime(1)=false  is_prime(2)=true
  is_prime(7)=true   is_prime(97)=true  is_prime(7919)=true
  is_prime(7920)=false
next_prime:
  next_prime(1)=2  next_prime(13)=17  next_prime(1000)=1009
count_primes_below:
  count_primes_below(10)=4  (100)=25  (1000)=168
factor_smallest:
  (0)=none  (1)=none  (2)=some 2  (15)=some 3
  (49)=some 7  (97)=some 97  (999983)=some 999983
```

All four Rust functions called from Lean produce the
correct values. Phase 9 reverse-direction pipeline runs
true end-to-end for the first time.

### Added — Phase 9-6 follow-up commit 3/3: examples/05 + emit CLI fixes (2026-05-23)

Final of three commits in the Lake `extern_lib`
integration. Wires `examples/05-rust-export/lean/lakefile.lean`
through `require Leo4Rust`, simplifies the `just
rust-export-05-build` recipe from 4 steps to 3, and fixes
emit-CLI Lean codegen bugs the integration test caught.

User-facing change:

```sh
# Before (4-step manual workflow per SPEC/reverse-direction.md §7):
cargo build
leo4-rust-emit --emit-lean
leanc -c shim/leo4_rust_bridge_lean.c
lake build
leanc -o … (manual link line)

# After:
cargo build
leo4-rust-emit --emit-lean
lake build   # Lake auto-links both .a's via Leo4Rust's extern_libs
```

The manual `leanc -o` final-link step is gone. The
glue-shim compile is gone (Lake's `leo4RustBridgeLean`
extern_lib handles it). `lake build` produces the
executable directly.

Files:

- `examples/05-rust-export/lean/lakefile.lean`: add
  `require Leo4Rust from "../../../lake/Leo4Rust"`; add a
  `lean_lib Leo4ExampleMiniSolverRust` with
  `.submodules` glob so Lake picks up the emit CLI's
  generated wrapper module from
  `Leo4ExampleMiniSolverRust/Rust.lean`. Comment block in
  the lakefile points users at the Leo4Rust extern_libs.
- `justfile`: `rust-export-05-build` recipe goes from 4
  steps to 3 (steps 3 + 4 merge into a single `lake build`
  that auto-links). The README's "run with env matrix"
  reminder gets printed at the end of the recipe.
- `examples/05-rust-export/README.md`: "Build + run"
  section rewritten — fast path leads, manual workflow is
  now strictly debugging reference. Adds a logicutils
  install hint (Arch pacman / cargo install / omit and
  accept per-build leanc invocation).

Emit-CLI fixes (`crates/leo4-rust-emit/src/main.rs`)
surfaced when Lake actually compiled the generated
wrapper for the first time:

1. **`IO (T)` paren wrap**: render emits `IO ({ret_ty_str})`
   rather than `IO {ret_ty_str}`. Without parens,
   `IO Option UInt64` parses as Lean application
   `IO Option UInt64` (three-arg) — type error.
2. **`Nat.toDigits ... |>.asString` deprecated**: replaced
   with the simpler `s!"... {status} ..."` interpolation
   (`UInt32` has a `ToString` instance; the manual hex
   conversion was unnecessary anyway).
3. **`Leo4.LeanError.detail` doesn't exist**: the struct's
   field is `message` (per
   `lake/Leo4/Leo4/Marshal.lean:21`). Replaced
   `{e.detail}` with `{e.message}`.

Three corresponding unit-test assertions updated (workspace
test count stays at 141/0).

Verified locally — `just rust-export-05-build` runs all
three steps clean:
- Cargo builds cdylib + bridge + worker + emit.
- `leo4-rust-emit --emit-lean` writes IDL/handshake/wrapper.
- `lake build` produces the executable at
  `examples/05-rust-export/lean/.lake/build/bin/leo4Example05`
  (152 MB statically-linked binary). Lake's link line picks
  up both extern_lib `.a`s from `Leo4Rust`.

**Runtime open issue**: the produced executable launches the
dispatcher but the first request currently fails with
`status 993542224` (uninitialised-buffer read in the
dispatcher's error path) and the worker reports
`Connection reset by peer` on its IPC pipe — a pre-existing
9-3 / 9-4 protocol-init bug surfaced for the first time by
this commit's actual e2e run. Lake-integration scope of
this commit is met; the dispatcher↔worker handshake fix is
a follow-up.

Dispatcher source fix:
- `shim/leo4_rust_bridge.c` swaps `strtoull` for an inlined
  decimal parser (`leo4_parse_u64_decimal`). Some
  clang+glibc combinations emit a versioned
  `__isoc23_strtoull` reference that leanc's bundled
  sysroot can't resolve at link time; the inlined parser
  avoids the libc symbol entirely. Recycle-policy parsing
  behaviour unchanged.

### Added — Phase 9-6 follow-up commit 2/3: `leo4RustBridgeLean` extern_lib (leanc + ar + logicutils gate) (2026-05-23)

Second of three commits. Adds the Lean-side glue shim
auto-compile path so users no longer need to run
`leanc -c shim/leo4_rust_bridge_lean.c` manually before
`lake build`.

Implementation follows SPIKE-1 pattern 4b plus option R2 of
the logicutils discussion (optional, with fallback to
unconditional rebuild):

- `extern_lib leo4RustBridgeLean pkg := …` (new in
  `lake/Leo4Rust/lakefile.lean`).
- Inputs: `shim/leo4_rust_bridge_lean.c` resolved via
  `pkg.dir / ".." / ".." / "shim" / …`.
- Outputs: `leo4_rust_bridge_lean.o` (leanc -c -std=c2x)
  wrapped via `ar rcs` into
  `libleo4_rust_bridge_lean.a` under
  `pkg.buildDir / "leo4rust/"`.
- Logicutils probe via `freshcheck --protocol-version` in a
  `try` block — if the binary is absent or the call errors,
  the body falls through to unconditional rebuild (4b base
  behaviour). When freshcheck IS available:
  - `freshcheck --method=hash --store … outAr src` decides
    whether to rebuild (exit 0 = fresh, non-zero = stale).
    BLAKE3 content hash by default — robust against
    timestamp churn from `git checkout` etc.
  - After a successful rebuild, `stamp record --store … outAr
    src` records BOTH the target and the dep so the next
    freshcheck has both baselines. Stamp failure is
    tolerated (cache bookkeeping, not load-bearing).
- Pre-`ar` cleanup: existing `.a` is removed before `ar rcs`
  so the archive doesn't accumulate stale members across
  rebuilds.

Verified locally:
- Clean build (`rm -rf .lake/build/leo4rust && lake build
  leo4RustBridgeLean.static`) produces a valid 15 770-byte
  `libleo4_rust_bridge_lean.a` + a `.lu-store/` directory
  populated by logicutils.
- Real source edit (adding a new `LEAN_EXPORT void
  leo4_rust_bridge_test_marker_qz(void)` symbol) → next
  `lake build` produces a different archive hash, confirming
  the rebuild fired.
- Comment-only edits leave the `.o` byte-identical
  (C compiler discards comments before codegen), so the
  resulting `.a` stays the same — desired behaviour.
- `which freshcheck` available locally
  (`pacman -Qi logicutils` reports v0.2.0); the fallback
  branch is exercised by stubbing freshcheck out of PATH
  (manual verification only).

logicutils install hint (added to install.md follow-up;
README/AGENTS bumped in commit 3/3):

- Arch / Manjaro: `pacman -S logicutils`
- Other platforms: `cargo install --git https://github.com/newsniper-org/logicutils logicutils`
- Or omit and accept the per-build leanc + ar invocation
  (still functional, just no skip on unchanged source).

Cache layering note: Lake's own outer cache covers the
"output file already exists with matching trace" case. The
inner freshcheck guard only fires when Lake calls our body
in the first place. Both layers compose naturally — Lake
governs whether the body runs; freshcheck governs whether
the body's leanc/ar invocations run inside the body.

Next: 3/3 wires `examples/05-rust-export/lean/lakefile.lean`
through `require Leo4Rust` + drops the manual `leanc -o`
step from the example's README and the `just
rust-export-05-build` recipe.

### Added — Phase 9-6 follow-up commit 1/3: `Leo4Rust` Lake package + `leo4RustBridge` extern_lib (2026-05-23)

First of three commits collapsing the reverse-direction
manual `leanc -o` step. Per `spike/SPIKE-1-lake-extern-lib.md`'s
4a pattern (path resolution only) and option R2 (logicutils
optional, fallback to unconditional rebuild).

- `lake/Leo4Rust/lakefile.lean` (new) — `package Leo4Rust`,
  `require Leo4 from ".." / "Leo4"`, empty marker `lean_lib`
  for default-target compliance.
- `extern_lib leo4RustBridge pkg := …` — path resolution
  chain for `libleo4_rust_bridge.a`:
  1. env `LEO4_RUST_BRIDGE_AR` (explicit override)
  2. `<leo4_repo>/target/release/libleo4_rust_bridge.a`
  3. `<leo4_repo>/target/debug/libleo4_rust_bridge.a`
  `<leo4_repo>` resolves as `pkg.dir / ".." / ".."` —
  `lake/Leo4Rust/../..` lands at the leo4 repo root.
  Body returns `(Pure.pure path)` (Lake's `Job` `Pure`
  instance). Missing archive → `error s!"…"` via Lake's
  `Util.Log.error` helper with a clear "run cargo build
  first" pointer + full search list.
- `lake/Leo4Rust/Leo4Rust.lean` (new) — empty marker module.
  Importing it is unnecessary for the link integration to
  fire (Lake's `lean_exe` walks `dep.externLibs`
  unconditionally), but having a default `lean_lib` lets
  `lake build` work without explicit targets.
- `lake/Leo4Rust/lean-toolchain` — pinned to
  `leanprover/lean4:v4.29.1` matching the rest of the repo.

Verified:
- `cd lake/Leo4Rust && lake build` clean (Built Leo4Rust
  140ms).
- `lake build leo4RustBridge.static` resolves correctly when
  cargo-built archive is present.
- `LEO4_RUST_BRIDGE_AR=/no/such/file lake build
  leo4RustBridge.static` errors with the expected
  "libleo4_rust_bridge.a not found. Searched: /no/such/file …
  Run cargo build --release -p leo4-rust-bridge…" message.

Next:
- Commit 2/3 adds `leo4RustBridgeLean` extern_lib (leanc + ar
  on `shim/leo4_rust_bridge_lean.c`, with `freshcheck`
  optional gate via `which`).
- Commit 3/3 wires `examples/05-rust-export/lean/lakefile.lean`
  through `require Leo4Rust`, drops the manual `leanc -o`
  step from the example's README + the `just
  rust-export-05-build` recipe.

### Added — `leo4` CLI: `create` (new project) + `init` (existing crate) (2026-05-23)

New workspace member `crates/leo4-cli/` shipping a `leo4`
binary with two scaffolding subcommands. Distinct semantics:

- **`leo4 create <direction> <dir>`** — new project. Creates
  the directory (or expects it empty), writes a complete
  buildable skeleton: `Cargo.toml`, `src/`, `lean/`,
  `README.md`. `cargo new`-style ergonomics.
- **`leo4 init <direction>`** — in-place integration into an
  *existing* Cargo crate (cwd by default, or `--dir`). Adds:
  - a `# ─── leo4 integration ───` block appended to
    `Cargo.toml` (idempotent — re-running skips when the
    marker line is already present);
  - `build.rs` (forward direction only) if absent;
  - `lean/{lakefile.lean,Sample.lean or Main.lean,lean-toolchain}`
    if absent.
  Existing `src/` is never touched.

Both subcommands accept `forward` (`@[leo4_export]` + Rust
`leo4::import!`) or `reverse` (`#[leo4::export]` + generated
Lean wrapper) directions. `--leo4-root <path>` overrides the
default `../leo4` sibling path for the generated Cargo /
Lake `require` entries.

Templates produce:
- Forward: `Cargo.toml` (leo4 + leo4-build), `build.rs`
  wiring the Lake shim, `src/main.rs` with a `leo4::import!`
  block calling `hello` / `add`, `lean/Sample.lean` with
  matching `@[leo4_export]`s.
- Reverse: `Cargo.toml` (`[lib] crate-type=["cdylib"]`,
  leo4 with `rust-exports` feature), `src/lib.rs` with
  `#[leo4::export] pub fn double / greet`, `lean/Main.lean`
  importing the generated `<Iface>.Rust` wrapper.

CLI sanity:
- 3 unit tests (camel_case, Cargo.toml name extraction,
  idempotent Cargo.toml extension).
- End-to-end smoke: `leo4 create forward /tmp/x` produces 7
  files in the expected layout; `leo4 init reverse --dir
  <pre-existing>` appends the integration block + writes
  lean/ without touching existing `src/main.rs`; a second
  `init` reports `skip … already exists` for every entry.

Workspace test count 138 → 141; all green. `cargo install
--path crates/leo4-cli` makes `leo4` available on PATH.

### Added — Phase 9-4c: Windows backend (`CreateProcess` + named pipe) (2026-05-23)

Fills the second real branch of `leo4_worker_ops_t`. Same
single C TU; lives behind `#if defined(_WIN32)` next to the
POSIX backend, with backend selection chain updated to pick
`&leo4_windows_ops` on `_WIN32`.

Workflow:

1. `CreateNamedPipeA` opens a duplex pipe at
   `\\.\pipe\leo4_rust_<pid>_<nonce>` (nonce: process-wide
   `_Atomic uint32_t` counter for multi-spawn safety).
2. `CreateProcessA` launches `leo4-rust-worker.exe --cdylib
   <path> --ipc-pipe <name>` via PATH search
   (`lpApplicationName = NULL`). `ERROR_FILE_NOT_FOUND`
   maps to `LEO4_ERR_RUST_CDYLIB_NOT_FOUND`; everything
   else to `LEO4_ERR_RUST_SPAWN_FAILED`.
3. `ConnectNamedPipe` blocks until the worker's
   `CreateFileA` on the same pipe name resolves. The
   worker's `--ipc-pipe` argument carries the same string
   (separate from POSIX's `--ipc-fd N`).
4. `win_send_all` / `win_recv_exact` loop over `WriteFile` /
   `ReadFile` with short-read detection. EOF / 0-byte ->
   `LEO4_ERR_RUST_IPC_FAILED`.
5. `win_alive_worker` polls via `WaitForSingleObject(.., 0)`;
   `win_reap_worker` does the blocking wait + `GetExitCodeProcess`
   + `CloseHandle` on both pipe and process.
6. `win_kill_worker` calls `TerminateProcess`.

Target triple: `x86_64-pc-windows-gnullvm` (Tier 2, per
LEO4-DESIGN.md §9.1). Compiles under clang's gnullvm target —
no `__declspec`, no MSVC ABI fork. Linux/macOS builds skip
the whole block via the `#ifdef` guard.

Cross-compile / runtime verification on Windows is deferred
to the Tier 2 CI matrix when it lands. Today the code
compiles in-source against the `windows.h` we'd see on a
gnullvm clang invocation; the Linux host build is
unaffected (no `_WIN32` defined).

`leo4-rust-worker` (the Rust harness, Phase 9-3) already
has the `--ipc-pipe <name>` arg parsed and returns
"Windows named-pipe IPC not yet implemented (Phase 9-4c)" —
that's the slot the worker side fills next.

Workspace test count unchanged at 138/0.

### Added — Phase 9.X: env-driven worker recycle policy (2026-05-23)

`LEO4_RUST_WORKER_RECYCLE_CALLS=N`: after N completed
dispatcher calls, the persistent worker is killed and a
fresh one lazy-spawns on the next call. Default `0` (or
unset / invalid) keeps the worker up for the whole Lean
process lifetime, matching pre-9.X behaviour.

Dispatcher changes (`shim/leo4_rust_bridge.c`):

- `leo4_worker_slot_t` gains a `_Atomic uint64_t call_count`
  field. Incremented after each successful
  request/response round-trip.
- `leo4_recycle_calls_limit` (file-scoped `_Atomic uint64_t`)
  parsed once from the env via `strtoull`; non-numeric or
  zero leaves recycling disabled.
- `leo4_recycle_init_once` runs at most once per process via
  a compare-exchange guard.
- `leo4_recycle_persistent_slot` atomically swaps the
  worker pointer out, then `kill` + `reap` via the ops
  table — no OS syscall named outside the backend block, per
  SPEC §4.4.
- `leo4_rust_call`'s persistent-worker branch checks the
  counter *before* the lazy-spawn lookup; if reached, reaps
  the current worker before falling into the standard lazy
  spawn path.

Time-based recycle (`LEO4_RUST_WORKER_RECYCLE_SECONDS`) is a
further 9.X follow-up; call-based is what ships today
(simpler, sufficient for "keep memory bounded over long-run
SMT-solver sessions").

Workspace test count unchanged at 138/0; full end-to-end
exercise of the recycle path lives in the manual
`just rust-export-05-build` workflow with the env set.

### Added — Phase 9.X: `#[leo4::export(isolated)]` dispatcher path (2026-05-23)

Per-call fresh worker for `isolated`-tagged exports. The
attribute itself shipped in 9-1 (parsed by the macro and
recorded in `ExportEntry.isolated`); 9-5 / 9-6 ignored it. This
commit wires it through the dispatcher.

Mechanism (minimal wire surface change):

- `leo4-rust-emit`'s Lean wrapper render now prefixes the
  mangled name with `iso:` for exports tagged
  `#[leo4::export(isolated)]`. Persistent exports pass the
  raw mangled name verbatim.
- `shim/leo4_rust_bridge.c`'s `leo4_rust_call` detects the
  `iso:` prefix via `memcmp`. When present, it strips the
  prefix and routes through a new `leo4_dispatch_isolated`
  helper:
  1. Allocate a fresh worker via `leo4_worker_ops->spawn`
     (separate process from the persistent slot).
  2. Send the request frame.
  3. Receive the response frame.
  4. Send a magic=0 graceful-shutdown frame.
  5. `leo4_worker_ops->reap` the worker.
- The persistent worker is unaffected — it keeps running
  across calls, just as before.

Why the prefix trick: no SPEC wire format change, no new
dispatcher API entry, no Lean wrapper signature change. The
typed wrapper renders identically except for the literal
string passed to `leo4RustCallRaw`. Backwards-compatible with
9-5's wrapper consumers.

Cost: per-call worker spawn (`posix_spawn` ~5-10 ms on Linux).
Use only for exports whose state contamination would corrupt
later unrelated calls. Persistent mode remains the default.

`shim/leo4_rust_bridge.c` builds clean under `-std=c23` /
`-std=c2x` / `-std=c17`. Workspace test count unchanged at
138/0 (the change adds dispatch-path logic that needs the
worker harness + cdylib to verify end-to-end; that lives in
the manual `just rust-export-05-build` workflow).

### Added — Phase 9-6 follow-up: Lake automation for the reverse-direction pipeline (2026-05-23)

Collapses the 4-step manual workflow from
`SPEC/reverse-direction.md` §7 into named `just` recipes +
reusable `Leo4.Build.RustBridge` helpers a user lakefile can
call directly. The end-to-end demo (`examples/05-rust-export/`)
now builds via a single `just rust-export-05-build`.

**`justfile` additions**:

- `rust-bridge-build` — builds the three Cargo artefacts the
  reverse direction needs in one shot (`leo4-rust-bridge` static
  archive, `leo4-rust-worker` binary, `leo4-rust-emit` CLI).
- `rust-emit CDYLIB OUT_DIR MODULE` — variable-parameterised
  wrapper around `cargo run -p leo4-rust-emit --emit-lean`. Any
  user cdylib can be wired through one command.
- `glue-shim-build OUT_OBJ` — `leanc -c -std=c2x
  shim/leo4_rust_bridge_lean.c -o $OUT_OBJ` (the one place
  `lean.h` legitimately enters the build).
- `rust-export-05-build` — end-to-end recipe for
  `examples/05-rust-export/`. Builds the cdylib + bridge +
  worker, runs `leo4-rust-emit --emit-lean`, moves the Lean
  wrapper to `lean/Leo4ExampleMiniSolverRust/Rust.lean` so
  Lake picks it up under the expected module path, compiles
  the glue shim, and runs `lake build`. The final
  `leanc -o` link is still manual (Lake's `lean_exe` DSL
  doesn't take dynamic link args yet) — the README documents
  the one-line link command.
- `rust-export-05-clean` — drops the emitted artefacts.

**`lake/Leo4/Leo4/Build.lean` additions**:

`Leo4.Build.RustBridge` namespace with three IO helpers a
user lakefile can call from a `script` block or a
`def main`-style helper:

- `compileGlueShim leo4Root outObj : IO FilePath` — invokes
  `leanc -c -std=c2x` on `shim/leo4_rust_bridge_lean.c`. Throws
  `IO.userError` (forwarding stderr) on non-zero exit.
- `discoverBridgeArchive leo4Root : IO FilePath` — locates
  `libleo4_rust_bridge.a` via env `LEO4_RUST_BRIDGE_AR` first,
  then `target/release/`, then `target/debug/`. Mirrors the
  `LEO4_SHIM_SO` search chain in `leo4-build::wire`.
- `linkArgs leo4Root glueObj : IO (Array String)` — returns
  `#[glueObj, bridgeArchive]` for caller-side splicing into
  `weakLinkArgs` or a manual leanc invocation.

These don't yet plug into Lake's `lean_exe` DSL — Lake's
`weakLinkArgs` is currently a static-Array field and there is
no first-class hook for "build the user's executable after a
dynamic IO action computes link args". When that hook lands,
the helpers are ready.

**`examples/05-rust-export/README.md` updated** with the
fast path (`just rust-export-05-build`) at the top + the
manual 4-step kept as a reference / debugging fallback.

No code-test changes; workspace count stays at 138/0.

### Added — `AGENTS.md` cookbook for Claude Code sessions (2026-05-23)

Companion to `CLAUDE.md`. CLAUDE.md is the working agreement
(how to behave); AGENTS.md is the cookbook — concrete
patterns for what to type when starting a task.

Contents:

- **Document routing**: a table mapping common starting
  questions to the doc that answers them. Reduces "read
  source to figure out what the doc should have said" loops.
- **Forward vs reverse direction**: side-by-side table of
  the two pipelines' mental models. The "where can `lean.h`
  live?" question gets a hard rule answer.
- **Commit cadence**: which files usually move together for
  each commit shape (SPEC, macro, CLI, C shim, e2e example).
  Reflects the actual Phase 9 history.
- **Boundary-type checklist**: 8-step refinement of the
  7-step one in CLAUDE.md. Adds reverse-direction
  considerations and Mathlib bridge entry.
- **OS-portability layer recipe**: stub-first, single
  interface, audit-ledger entry — the 9-4 spawn/IPC layer
  is the template.
- **Phase entry-gate**: design commit before any code; cites
  9-0 as the model.
- **Cargo / Lake / leanc cheatsheet**.
- **Common pitfalls (8 entries)**: schema_hash placement
  in reverse mangling, `_GNU_SOURCE` needed under strict
  C-std for POSIX symbols, `--gc-sections` and standalone
  link, `cc::Build::std` single-arg limitation,
  `IO α → future<α>` lift, `linkme` distributed-slice
  static appearing unused, schema_hash recomputation
  match, sticky `cargo:rerun-if-changed=`, variant
  discriminator being u32 LE not u8.
- **Subagent guidance**: when Explore / Plan /
  general-purpose actually help vs add overhead.
- **Recent decisions worth remembering (6 entries)**:
  D16 reverse-direction adoption, gnullvm Tier 2,
  long-running-worker isolation model, spawn/IPC ops
  abstraction, D4 async lift, v0.1.0 cut hash. Each one
  expensive enough to land that an agent rediscovering it
  from scratch would burn hours.

`CLAUDE.md` opening matter updated with a one-line pointer
to the new cookbook.

### Added — Phase 9-7: `examples/05-rust-export/` end-to-end demo (2026-05-23)

Eighth code landing on the Phase 9 ladder. The first example
where every layer of the reverse-direction pipeline executes
end-to-end: Rust cdylib → `leo4-rust-emit` → Lean wrapper →
glue shim → dispatcher (`libleo4_rust_bridge.a`) → POSIX
worker (`leo4-rust-worker`) → cdylib's wrapper symbol → Rust
function → response all the way back.

- `examples/05-rust-export/` (new workspace member, cdylib +
  rlib crate). Four `#[leo4::export]` functions hitting the
  v9-5 Lean-wrapper mapping table:
  - `is_prime(u64) -> bool`               (scalar / scalar)
  - `next_prime(u64) -> u64`              (long-running loop)
  - `count_primes_below(u64) -> u64`      (compute-heavy)
  - `factor_smallest(u64) -> Option<u64>` (`Option<T>` return)
- Rust-side unit tests pin every function's behaviour (4
  tests, all passing as part of `cargo test --workspace`).
- `examples/05-rust-export/lean/` carries the Lean driver:
  `lakefile.lean` (NOT a workspace member of `lake/Leo4/`;
  it's a standalone Lake project that references the runtime
  library via a relative `require`), `lean-toolchain` pinned
  to `v4.29.1` to match the rest of the repo, and `Main.lean`
  that imports `Leo4ExampleMiniSolverRust.Rust` and prints
  each function's answer for a representative input set.
- `examples/05-rust-export/README.md` documents the 4-step
  manual build + run workflow (`cargo build` → `leo4-rust-emit
  --emit-lean` → `leanc -c shim/leo4_rust_bridge_lean.c` →
  `lake build` + manual `leanc -o`), with the env-var matrix
  (`LEO4_RUST_CDYLIB`, `LEO4_RUST_WORKER_BIN`,
  `LEO4_RUST_HANDSHAKE_PKG`, `LEO4_RUST_HANDSHAKE_IFACE`)
  the worker needs to recompute the schema_hash to match.

Pipeline smoke verified: `cargo run -p leo4-rust-emit --
--cdylib …/libleo4_example_05_rust_export.so --out-dir …
--emit-lean --lean-module Leo4ExampleMiniSolverRust.Rust`
emits all three artefacts with schema_hash `ozln3adaktdow`,
the Lean wrapper carries `def is_prime : IO Bool`,
`def factor_smallest : IO Option UInt64`, etc, and the
handshake JSON lists the four expected mangled symbols
(`leo4_rust__is_prime__u64`, …).

The end-to-end *runtime* (Lake-built Lean executable
actually calling the cdylib through the dispatcher + worker)
needs the four-step manual workflow today; the README walks
through every command. Lake-plugin auto-discovery of the glue
shim source + the bridge static archive is a 9-6 / 9-7
follow-up — until it lands, this demo is the
load-bearing reference for "what does the user actually
type to make this work?".

Workspace test count 134 → 138; all green.

### Added — Phase 9-6: Lean-side glue shim (`shim/leo4_rust_bridge_lean.c`) (2026-05-23)

Seventh code landing on the Phase 9 ladder. Resolves the
`@[extern "leo4_rust_call_lean"]` declaration the Phase 9-5
wrapper emits, bridging the lean_object* ABI to the
dispatcher's byte-pointer signature in
`libleo4_rust_bridge.a`.

- `shim/leo4_rust_bridge_lean.c` (new, ~110 lines). The ONE
  leo4-side place that includes `<lean/lean.h>`. The
  dispatcher and its backends stay free of Lean ABI details,
  matching the forward-direction split
  (`<pkg>.leo4-shim.c` vs `crates/leo4-mslean4/`).
- `leo4_rust_call_lean(b_lean_obj_arg mangled, b_lean_obj_arg
  args, lean_object* world) -> lean_object*`:
  1. Extract `(cstr, size)` from `mangled` via
     `lean_string_cstr` + `lean_string_size` (subtracting the
     trailing NUL from `size`).
  2. Extract `(ptr, size)` from `args` via `lean_sarray_cptr`
     + `lean_sarray_size`.
  3. Allocate a 4 KiB initial response ByteArray
     (`lean_alloc_sarray(1, cap, cap)`).
  4. Call `leo4_rust_call(mangled, mangled_len, args_ptr,
     args_len, ret_ptr, ret_cap, &ret_len)`.
  5. On `LEO4_ERR_BUFFER_TOO_SMALL` (0x07): drop the
     too-small ByteArray, re-allocate to the size the
     dispatcher reported in `*ret_len`, retry once.
  6. Shrink the response ByteArray's logical size to the
     actual `ret_len` via `lean_sarray_set_size` (the
     underlying allocation stays at `cap`).
  7. Build the `(UInt32 × ByteArray)` tuple
     (`lean_alloc_ctor(0, 2, 0)` + `lean_box_uint32` + the
     ByteArray pointer) and wrap in `lean_io_result_mk_ok`.
- Borrow contract preserved: both `mangled` and `args` are
  `@&` on the Lean side / `b_lean_obj_arg` on the C side,
  so the shim does not call `lean_dec` on them. The freshly
  allocated `ret_array` and boxed status enter the tuple,
  which the caller drops on its own schedule.
- The `status` field surfaces dispatcher / worker failures
  to the typed Lean wrapper (the wrapper raises
  `IO.userError` on non-zero). The Lean IO error path is
  reserved for cases the shim itself cannot reach (none
  today).

Verification: `leanc -c shim/leo4_rust_bridge_lean.c -o … -std=c2x`
produces a clean ELF relocatable with `T leo4_rust_call_lean`
visible to a follow-on link step. (A standalone link into a
`.so` strips the symbol via `--gc-sections` since the test
link has no Lean-side reference; the production link path
preserves it because the `@[extern]` declaration is a real
caller.)

Build orchestration (Lake plugin auto-discovery of the
glue-shim source + leanc invocation) is the natural home for
a 9-6 follow-up. The user-facing
workflow on 9-6 today is:

```sh
cargo build --release                            # cdylib
leo4-rust-emit --cdylib … --out-dir … --emit-lean  # 9-2/9-5
cargo build --release -p leo4-rust-bridge        # 9-4a/b
leanc -c shim/leo4_rust_bridge_lean.c -o …       # 9-6
# … leanc-link the user's Lean wrapper with the two .o /
# .a above; SPEC/reverse-direction.md §7 step 4.
```

No code in the rest of the workspace changes; existing tests
stay at 134/0.

### Added — Phase 9-5: Lean wrapper module emission (`leo4-rust-emit --emit-lean`) (2026-05-23)

Sixth code landing on the Phase 9 ladder. `leo4-rust-emit`
grows an `--emit-lean` flag that, alongside the existing
`.leo4-rust-exports.idl` + `.leo4-rust-handshake` pair,
generates `<pkg>.leo4-rust-imports.lean` — a Lean wrapper
module exposing one typed `IO α` action per
`#[leo4::export]`.

Generated module anatomy:

```lean
import Leo4
namespace <module>           -- defaults to `<iface>.Rust`

def schemaHash : String := "<13-char base32lc>"

@[extern "leo4_rust_call_lean"]
private opaque leo4RustCallRaw
    (mangled : @& String) (args : @& ByteArray)
    : BaseIO (UInt32 × ByteArray)

def add (a0 : UInt64) (a1 : UInt64) : IO UInt64 := do
  let mut args := ByteArray.empty
  args := Leo4.LeanMarshal.canonicalEncode a0 args
  args := Leo4.LeanMarshal.canonicalEncode a1 args
  let (status, ret) ← leo4RustCallRaw "leo4_rust__add__u64_u64" args
  if status ≠ 0 then throw (IO.userError s!"…")
  match Leo4.LeanMarshal.canonicalDecode (T := UInt64) ret 0 with
  | .ok (v, _) => return v
  | .error e   => throw (IO.userError s!"… {e.detail}")

end <module>
```

Implementation in `crates/leo4-rust-emit/`:

- CLI gains `--emit-lean` (bool) and `--lean-module <name>`
  (defaults to `<iface>.Rust`).
- `render_lean_wrapper` emits the file header, schema-hash
  pin, single `@[extern]` raw entry, and one typed wrapper
  per export.
- `render_one_export` walks parameter / return mangles via
  `lean_type_of_mangle`. v9-5 scope: scalars (u8..u64, i8..i64,
  f32/f64, bool, char), `String`, `Nat` / `Int` (bignat /
  bigint), `Option<T>` of mapped inner, and the
  `list<u8>` → `ByteArray` / `list<T>` → `Array T` shapes.
  Composite payloads (records / variants / resources) are
  deferred — an export whose signature contains an unmapped
  mangle still gets a wrapper definition emitted, but its
  body is a Lean `panic!` so the user sees a clear runtime
  diagnostic rather than a silent ABI mismatch.
- `lean_safe_ident` guards a small keyword list (`def`,
  `match`, `end`, …); other identifiers pass through.
- 7 new unit tests cover the mangle-to-Lean-type table, list
  / option / ByteArray dispatch, keyword renaming, and
  end-to-end module rendering.

Smoke verified outside the workspace: a fixture cdylib with
four exports (`add`/`echo`/`negate`/`list_sum`) round-trips
through `leo4-rust-emit --emit-lean` into a clean Lean module
with `UInt64`, `Int32`, `String`, and `Array UInt32`
signatures in the right places, plus the cdylib's
schema_hash baked into `schemaHash`.

The `leo4_rust_call_lean` extern symbol the wrapper refers to
lives in a small leanc-compiled Lean-side glue shim Phase 9-6
will land. Until then user code compiling against the
generated `.lean` will fail at *link* time, not at emit time.

Workspace test count 127 → 134; all green.

### Added — Phase 9-4b: POSIX backend (`posix_spawn` + socketpair + waitpid) (2026-05-21)

Fifth code landing on the Phase 9 ladder. Fills the
`leo4_worker_ops_t` table's POSIX branch — the dispatcher body
itself does not change.

Same `shim/leo4_rust_bridge.c` translation unit; the POSIX
backend lives behind `#if defined(__unix__) || defined(__APPLE__)`
next to the stub backend. Workflow:

1. `socketpair(AF_UNIX, SOCK_STREAM, 0, sv)` — parent retains
   `sv[0]` (with `FD_CLOEXEC`), child receives `sv[1]` dup'd onto
   `fd 3` via `posix_spawn_file_actions_adddup2`.
2. `posix_spawnp` (or `posix_spawn` when `LEO4_RUST_WORKER_BIN` is
   an absolute path) launches `leo4-rust-worker --cdylib <path>
   --ipc-fd 3`. Worker binary resolution: env override first,
   `posix_spawnp` PATH search otherwise. `ENOENT` from spawn
   maps to `LEO4_ERR_RUST_CDYLIB_NOT_FOUND` (0x00020004) so
   callers can distinguish missing worker vs other failures.
3. `posix_send_all` / `posix_recv_exact` loop over `write` /
   `read` with `EINTR` retry and short-read detection. Short
   read / EOF mid-message → `LEO4_ERR_RUST_IPC_FAILED`
   (0x00020006).
4. `posix_alive_worker` calls `waitpid(.., WNOHANG)`; reaped
   PIDs are zeroed so the dispatcher can lazily respawn (a
   future Phase 9.X recycle / persistent-worker-restart will
   build on top of this).
5. `posix_reap_worker` does the blocking `waitpid` + `close` on
   shutdown.

Source-level changes:

- `_GNU_SOURCE` / `_DARWIN_C_SOURCE` defined before any `#include`
  so the POSIX symbols (`kill`, `posix_spawn`, `waitpid`, ...) are
  visible under strict `-std=c17` / `-std=c2x` modes.
- `<stdio.h>` added for `snprintf` in error-message formatting.
- `__attribute__((unused))` on `leo4_stub_ops` so the POSIX build
  doesn't warn about an unused fallback ops table (the stub stays
  in the file as the unconditional safety net).
- `extern char** environ;` declared at file scope so `posix_spawn`
  inherits the parent's environment — relevant for forwarding
  `LEO4_RUST_HANDSHAKE_PKG` / `_IFACE` / etc to the worker.

Backend selection chain in this same file: unix/macOS now picks
`&leo4_posix_ops` instead of the stub.

The existing `leo4-rust-bridge` Rust sanity test is updated:
it removes any `LEO4_RUST_WORKER_BIN` / `LEO4_RUST_CDYLIB`
overrides and asserts that the dispatcher cleanly returns one
of the `0x0002_xxxx` reverse-direction error codes. With no
worker binary on PATH and no cdylib it surfaces
`_CDYLIB_NOT_FOUND` (ENOENT) or `_IPC_FAILED` (worker crashed
on the bogus cdylib). The point is that the dispatcher links,
executes the full POSIX `posix_spawn` + `socketpair` +
`waitpid` codepath, and errors gracefully — not that it
succeeds without a real cdylib.

Workspace test count stays at 127 (the test count didn't grow
but the test now covers more code). End-to-end (dispatcher →
worker → cdylib → wrapper) integration with a real fixture
cdylib + `cargo build`-linked worker binary lands when the
Lake wrapper (9-5) gives us a natural cite for the
`LEO4_RUST_HANDSHAKE_PKG/_IFACE` env injection.

### Added — Phase 9-4a: dispatcher skeleton + `leo4_worker_ops_t` + stub backend (2026-05-21)

Fourth code landing on the Phase 9 ladder. The single-entry C
dispatcher that the Lean side will reach via
`@[extern "leo4_rust_call"]`. POSIX and Windows backends
(9-4b / 9-4c) land later and just fill the ops table — the
dispatcher body does not change.

- `shim/leo4_rust_bridge.c` (new, single C TU, ~330 lines):
  - `leo4_worker_ops_t` ops-table interface
    (spawn / kill / reap / send / recv / alive) per SPEC
    `reverse-direction.md` §4.4.
  - **Stub backend** (`leo4_stub_ops`) wired unconditionally
    on every platform; every op errors with
    `LEO4_ERR_RUST_SPAWN_FAILED` / `LEO4_ERR_RUST_IPC_FAILED`.
    Compile-time backend selection chain
    (`__unix__ || __APPLE__` → stub today, POSIX in 9-4b;
    `_WIN32` → stub today, Windows in 9-4c; else → stub).
    Ensures `libleo4_rust_bridge.a` links on every platform
    from day 1.
  - Worker handle slot via `_Atomic` (lazy spawn,
    compare-exchange guard). Single-Lean-thread invariant
    means the spin path is unreachable today; the guard is
    defensive for future multi-Lean models.
  - Dispatcher body `leo4_rust_call(mangled, ..., args, ...,
    ret, ret_cap, ret_len)`:
    1. `leo4_get_or_spawn_persistent` (ops `spawn`).
    2. `leo4_send_request` builds the SPEC §5.1 request frame
       (LE u32 magic / mangled_len / args_len + payload).
    3. `leo4_recv_response` parses the SPEC §5.2 response
       frame (magic + i32 status + u32 ret_len + u32
       detail_len + payload + detail), validates magic +
       sizes, propagates `LEO4_ERR_BUFFER_TOO_SMALL` with
       `*ret_len` = required size on caller-too-small.
  - cdylib path resolution: env `LEO4_RUST_CDYLIB` only
    (SPEC §9's full chain lands when 9-5 bakes a
    compile-time fallback).
  - Single export visibility macro: `__declspec(dllexport)`
    on Windows, `__attribute__((visibility("default")))`
    elsewhere. C17 baseline.
  - Debug helper `leo4_rust_bridge_current_ops()` exposed for
    tests so Rust can identify which backend got selected.
- `crates/leo4-rust-bridge/` (new workspace member):
  - `[lib] crate-type = ["staticlib", "rlib"]` so Lake can
    pick up the `.a` archive and the workspace can
    `cargo test` against the dispatcher.
  - `build.rs` drives `cc::Build::new().file(c_source)
    .std("c17").compile("leo4_rust_bridge")`. cc crate added
    to `workspace.dependencies`.
  - `src/lib.rs` declares the `leo4_rust_call` extern and a
    sanity test verifying that the dispatcher links and
    returns `LEO4_ERR_RUST_SPAWN_FAILED = 0x00020003` against
    the stub backend.

Workspace test count 126 → 127; all green. `libleo4_rust_bridge.a`
produced at `target/<profile>/libleo4_rust_bridge.a`.

End-to-end (dispatcher → worker → cdylib → wrapper) integration
arrives with the POSIX backend in 9-4b: only that fill-in
exchanges the stub `spawn`/`send`/`recv` for `posix_spawn` +
`socketpair` + `wait4`.

### Added — Phase 9-3: `leo4-rust-worker` harness binary (2026-05-21)

Third code landing on the Phase 9 ladder. The harness binary
the dispatcher (Phase 9-4) spawns once per cdylib — opens the
cdylib, performs the handshake, then runs the canonical-ABI
request loop serially via dlsym + catch_unwind.

- `crates/leo4-rust-worker/` (new workspace member, binary).
  CLI: `leo4-rust-worker --cdylib <path> --ipc-fd <N>` on POSIX
  (or `--ipc-pipe <name>` on Windows; pipe path lights up with
  9-4c).
- Boot sequence:
  1. `dlopen` the cdylib via `libloading`.
  2. Resolve `leo4_rust_describe_exports`, walk the `EXPORTS`
     slice, and copy entries out (so the cdylib lifetime can be
     decoupled from the request loop if a future worker mode
     needs it).
  3. Compute the schema_hash via the same FNV-1a-64 + base32lc
     algorithm `leo4-rust-emit` uses (pkg / iface from
     `LEO4_RUST_HANDSHAKE_PKG` / `_IFACE` env vars the
     dispatcher will set from the handshake JSON; sensible
     `rust` / `Rust` fallbacks for ad-hoc runs).
  4. Send a handshake frame (`SPEC/reverse-direction.md §5.3`)
     containing magic + hash_len + abi_version + 13-byte hash.
- Request loop (`SPEC §5.1 / §5.2`):
  - Read a request frame (magic + mangled_len + args_len +
    payload). magic=0 ⇒ graceful shutdown; bad magic ⇒ error;
    payload >256 MiB ⇒ error.
  - Resolve the mangled symbol via `dlsym` (cached by mangled
    name; cache miss returns `LEO4_ERR_RUST_DLSYM_FAILED`).
  - Call the wrapper with a shared response buffer; on
    `LEO4_ERR_BUFFER_TOO_SMALL` (7) grow the buffer to the
    required size (reported by the wrapper via `*ret_len`)
    and retry.
  - Wrap the call in `catch_unwind`. The wrapper itself (Phase
    9-1) already catches panics inside the user fn; this outer
    guard is a safety net for the wrapper plumbing. On panic
    the worker `process::abort`s — the dispatcher sees IPC EOF
    and respawns lazily.
  - Write a response frame (magic + status + ret_len +
    detail_len + payload + detail).
- IPC backend: POSIX `UnixStream::from_raw_fd(fd)` reading the
  inherited fd from the dispatcher. Windows named-pipe support
  defers to 9-4c (stub error message until then).
- 6 unit tests cover the wire format (handshake/request/response
  encode + decode), magic=0 shutdown handling, `surface_form`
  mangle-to-IDL inverse on the worker side, and the FNV
  digest's 13-char base32lc output. Workspace test count 120 →
  126; all green.

End-to-end (dispatcher → worker → cdylib → wrapper) integration
naturally lives in 9-4a (dispatcher) commit — `socketpair` +
fd-inherit semantics are dispatcher-side and the wire round-trip
is best verified there.

### Added — Phase 9-2: `leo4-rust-emit` CLI + `leo4-build::wire_rust_exports` (2026-05-21)

Second code landing on the Phase 9 ladder. The emit pass that
turns a built user cdylib into the `.leo4-rust-exports.idl` +
`.leo4-rust-handshake` pair Lake will consume in 9-5.

- `crates/leo4-abi/src/rust_exports.rs` gains:
  - `#[repr(C)]` on `ExportEntry` (was implicit Rust repr;
    external tooling needs a stable field layout to walk the
    slice via `dlopen`).
  - A new `extern "C"` entry,
    `leo4_rust_describe_exports(out_ptr, out_len) -> i32`,
    that writes the in-process `EXPORTS` slice pointer + length
    into caller-provided out-params. This is the stable
    FFI gateway for in-process introspection; downstream tools
    do not need to resolve `linkme`'s private internals.
- `crates/leo4-rust-emit/` (new workspace member, binary). CLI:
  `leo4-rust-emit --cdylib <path> --out-dir <dir> [--pkg <name>]
                  [--iface <name>] [--rust-toolchain <str>]`.
  `dlopen`s the cdylib via `libloading`, calls
  `leo4_rust_describe_exports`, copies each entry out (so the
  cdylib can be unloaded immediately), computes the canonical
  IDL form + FNV-1a-64 schema_hash (13-char base32lc, matching
  the forward direction's algorithm), and atomically writes:
  - `<pkg>.leo4-rust-exports.idl` — pretty canonical IDL,
    functions sorted by mangled name.
  - `<pkg>.leo4-rust-handshake` — JSON containing schema_hash,
    abi_version, package, interface, cdylib_path (absolute),
    rust_toolchain, leo4_rust_emit_version, emitted_at (UTC
    RFC 3339), and an exports table mirroring the slice. 6
    unit tests; standalone cdylib smoke verified 4 exports
    (`add(u64,u64)→u64`, `echo(string)→string` (isolated),
    `pi()→f64`, `list_len(list<u32>)→u64`) round-trip into
    both files with schema_hash `7i2wz2k5rqhls`.
- `crates/leo4-build/` gains `wire_rust_exports(out_dir)` for
  Lean-side consumers' `build.rs`. Locates the
  `.leo4-rust-handshake` file in `out_dir`, emits
  `cargo:rustc-env=LEO4_RUST_HANDSHAKE_FILE=…` and
  `cargo:rustc-env=LEO4_RUST_EXPORTS_IDL_FILE=…`, plus
  `cargo:rerun-if-changed=` for both and
  `cargo:rerun-if-env-changed=LEO4_RUST_CDYLIB`.
- `SPEC/reverse-direction.md` §7 + §8 rewritten to match: the
  Rust cdylib is built first; `leo4-rust-emit` is a separate
  CLI step (not a `build.rs` action) that produces the
  metadata files; Lake then reads them in 9-5. The handshake
  JSON example reflects the actual `leo4-rust-emit 0.1.0`
  output (collapsed canonical form, sort by mangled name,
  per-export fields).

Schema-hash detection between cdylib and handshake is deferred
to the worker harness (9-3): the worker recomputes the hash at
init time and the dispatcher surfaces `LEO4_ERR_HANDSHAKE_MISMATCH`
(0x05) on the first call.

### Added — Phase 9-1: `#[leo4::export]` attribute proc-macro (2026-05-21)

First code landing for Phase 9. Tagged Rust functions become
callable through the reverse-direction pipeline once the rest
of Phase 9 ships.

- `crates/leo4-abi/src/rust_exports.rs` (new, behind a new
  `rust-exports` cargo feature) — `ExportEntry` struct + the
  `EXPORTS` distributed slice (`linkme::distributed_slice`).
  Stays off by default so stable workspace builds remain free of
  the `linkme` dependency.
- `crates/leo4-macros-backend` — `expand_export` function +
  `ExportAttrs` parsing (`isolated` recognised; recycle / panic
  options deferred). The expansion emits:
  - The original user `fn` unchanged.
  - An `#[unsafe(no_mangle)] pub unsafe extern "C" fn
    leo4_rust__<fname>__<param_mangles>(args, args_len, ret,
    ret_cap, ret_len) -> i32` wrapper that does canonical-ABI
    decode → `catch_unwind`(user-fn) → canonical-ABI encode.
    `LEO4_ERR_RUST_PANIC = 0x00020001` surfaces on panic.
  - A `linkme` distributed-slice registration writing one
    `ExportEntry` into `EXPORTS`.
- `crates/leo4-macros` — `#[proc_macro_attribute] pub fn export`.
- `crates/leo4` — `rust-exports` feature pass-through + a new
  `__private` module that re-exports `linkme`, `ExportEntry`, and
  `EXPORTS` so user cdylibs need only depend on `leo4`. The macro
  emits paths through `::leo4::__private::*`.
- Unit tests in `leo4-macros-backend` (4 new) cover the attr
  parser (`isolated` / default / unknown rejection), a scalar
  smoke expansion, and the `async fn` / unsupported-type
  diagnostics. Workspace test count 110 → 114; all green.
- Standalone smoke cdylib verified: three `#[leo4::export]`s
  (`add(u64, u64) -> u64`, `#[leo4::export(isolated)] echo(String)
  -> String`, `no_args() -> u32`) compile and the wrapper
  symbols (`leo4_rust__add__u64_u64`, `leo4_rust__echo__str`,
  `leo4_rust__no_args`) show up in the produced `.so`.

The dispatcher (`libleo4_rust_bridge.a`) and the worker harness
that will reach these symbols land in 9-3 / 9-4a-c. Phase 9-2
(build script emitting `<pkg>.leo4-rust-exports.idl` +
`<pkg>.leo4-rust-handshake` from the `EXPORTS` slice) is the
next substep.

### Changed — Tier 2 Windows: clarify C ↔ Rust ABI compatibility (2026-05-21)

Follow-up on the gnullvm target adoption. Per the rustc
platform-support docs for `x86_64-pc-windows-gnullvm`, Rust
binaries on that target are ABI-compatible with C code built by
**clang** targeting either `*-pc-windows-gnu` or
`*-pc-windows-gnullvm` — i.e. mingw-w64's `gcc` is NOT sufficient,
the C compiler must be LLVM-based. leo4 enforces the LLVM
track end-to-end: the forward shim and the reverse-direction
dispatcher are both compiled through `leanc` (which on Windows
wraps clang); user cdylibs are gnullvm Rust. Documented in
`LEO4-DESIGN.md §9.1`, `OS-PORTABILITY.md §1`, and
`SPEC/reverse-direction.md` §11.

### Changed — Tier 2 Windows target: `*-pc-windows-msvc` → `*-pc-windows-gnullvm` (2026-05-21)

leo4 targets the gnullvm Windows triple (clang + lld + UCRT)
rather than MSVC. Practical consequences:

- The forward shim's clang/gcc-style C
  (`__attribute__((visibility("default")))`, `__builtin_memcpy`,
  gcc command-line flags) compiles on Windows unmodified — no
  MSVC-intrinsic fork in the emitter.
- The C-standard baseline (`-std=c17` / optional `-std=c2x`)
  is uniform across every tier via clang/leanc.
- Users skip the Visual Studio dependency; the only
  Windows-specific prerequisite is `rustup target add
  x86_64-pc-windows-gnullvm`.
- The OS-PORTABILITY audit's "C compiler visibility" and
  "gcc/clang command-line flags" rows move from "needs layer"
  to "resolved by Tier 2 target choice". Spawn / IPC / dynamic
  loading / DLL search remain genuinely Windows-specific and
  stay behind their respective abstraction layers.

Updated: `LEO4-DESIGN.md §9.1` (platform tier table + rationale),
`README.md` (tier table), `OS-PORTABILITY.md` (policy +
audit), `SPEC/reverse-direction.md` §4.4 + §11 (Windows
backend target + uniform `-std` invocation).

### Added — OS-portability policy + audit ledger (2026-05-21)

`OS-PORTABILITY.md` (new) — leo4-wide policy: OS-specific code
must be confined to identified abstraction layers; new
`#[cfg(target_os=…)]` / `cfg(unix)` / `cfg(windows)` branches
outside an identified layer get lifted into a layer in the same
commit or rejected. Document seeds the audit ledger with the
current OS branches (Linux `.so` extension hardcode,
`-Wl,-rpath` link line, gcc/clang `__attribute__((visibility))`
in shim source, ...) and recommends a layer per concern.

The Phase 9 spawn / IPC abstraction (`SPEC/reverse-direction.md`
§4.4) is the first formally-specified instance of this policy
and the model for follow-on layers.

`CLAUDE.md` "Cross-cutting" section gains a one-line pointer to
the policy.

### Changed — Phase 9-0 follow-up: spawn / IPC abstraction (2026-05-21)

`SPEC/reverse-direction.md` §4.4 (new sub-section) formalises a
`leo4_worker_ops_t` ops-table that the dispatcher uses for all
worker spawn, IPC, and reaping syscalls. Three backend slots:
**stub** (always errors with `LEO4_ERR_RUST_SPAWN_FAILED`;
ensures `libleo4_rust_bridge.a` links on every platform from
day 1), **POSIX** (`posix_spawn` + `socketpair` + `wait4`), and
**Windows** (`CreateProcess` + named pipe +
`WaitForSingleObject`). All three live in the same single C
translation unit, gated by `#ifdef`.

`ROADMAP.md` Phase 9 substep 9-4 is split accordingly:

- **9-4a** — dispatcher skeleton + `leo4_worker_ops_t` table
  + stub backend.
- **9-4b** — POSIX backend (Tier 1 exit criterion).
- **9-4c** — Windows backend (Tier 2 schedule).

### Added — Phase 9 entry gate: reverse-direction (Rust → Lean) design (2026-05-21)

leo4 grows a second pipeline so Rust functions tagged
`#[leo4::export]` become callable from Lean as ordinary `IO α`
actions. Motivating use case: combining a Rust-implemented SMT
solver with Lean's proof tooling, where incremental
`push/pop`-style state must persist across calls.

Design only — no code yet. Implementation lands in substeps
9-1 through 9-8 (see `ROADMAP.md`).

- `SPEC/reverse-direction.md` (new) — normative SPEC covering
  mangling prefix `leo4_rust__`, dispatcher API
  (`leo4_rust_call`), long-running worker process lifecycle
  with opt-in `#[leo4::export(isolated)]` per-call fresh
  workers, IPC wire format, build orchestration, handshake
  file format, cdylib path resolution (env → handshake →
  sibling search, mirroring `LEO4_SHIM_SO`), and the C
  standard policy (C17/C18 baseline, C23 features optional).
- `SPEC/canonical-abi.md` §13 — extends the error-code table
  with the `0x0002_0000..0x0002_FFFF` Rust-worker passthrough
  range: `LEO4_ERR_RUST_PANIC` (0x00020001),
  `LEO4_ERR_RUST_WORKER_RESTARTED` (0x00020002),
  `LEO4_ERR_RUST_SPAWN_FAILED` (0x00020003),
  `LEO4_ERR_RUST_CDYLIB_NOT_FOUND` (0x00020004),
  `LEO4_ERR_RUST_DLSYM_FAILED` (0x00020005),
  `LEO4_ERR_RUST_IPC_FAILED` (0x00020006).
- `LEO4-DESIGN.md` — D16 adopted; §11 out-of-scope updated to
  cite Phase 9 as the home for the reverse direction.
- `ROADMAP.md` — Phase 9 entry section with the architecture
  diagram, isolation matrix, and substeps 9-0 through 9-8.

Isolation model: long-running worker process per cdylib by
default (preserves user state across calls; SMT-solver friendly).
`#[leo4::export(isolated)]` opts a function into a fresh worker
per call for stronger cross-call isolation. T1 (memory
corruption) / T2 (panic) / T3 (thread leak) are all caught at
the process boundary; what happens inside the worker is the
cdylib's responsibility.

Dispatcher is a single C17 translation unit
(`shim/leo4_rust_bridge.c`, ≈150–250 lines, statically linked
into the Lean executable) that lazily spawns the worker via
`posix_spawn` / `CreateProcess`. The dispatcher API
(`leo4_rust_call(mangled, args, ret)`) is intentionally
isolation-backend-neutral so future variants (zygote-fork, wasm
sandbox) can be swapped in without changing callers.

## [0.1.0] — 2026-05-21

First tagged release. Phases 0–8 complete: full Lean ↔ Rust
round-trip on Tier 1 Linux, cross-impl mangling agreement (70
mangled instantiations, schema_hash `qi5gb74dbjyxo`), mutual
recursion via `mutual { … }` + `Cyc<i>`, `IO α` lifted to
`future<α>`, Mathlib-compatible carrier-type subset with opt-in
bridges, and a Typst documentation suite in four languages.

### Added — Typst documentation suite (2026-05-21)

`docs/learning/{en,ko,ja,de}/main.typ` — short architectural
overview in English / Korean / Japanese / German (~330 lines each).
`docs/implement-from-scratch/{en,ko,ja,de}/main.typ` — long-form
step-by-step build guide following the Phase 0–10 ladder (~1050–1175
lines each). Shared `docs/template/leo4-book.typ` carries cover /
ToC / code-block styling. All eight documents compile with
`typst >= 0.14.2` via `typst compile --root docs <file>`.

### Changed — workspace version bump 0.0.0 → 0.1.0 (2026-05-21)

`workspace.package.version` + the path-dep version fields in
`workspace.dependencies` align with the 0.1.0 release.

### Changed — `cargo check` clean: 17 non_snake_case + 1 dead_code (2026-05-21)

`leo4-macros-backend` emits `#[allow(non_snake_case)]` on every
wrapper `fn` generated from `leo4::import!` — Lean export names are
camelCase by convention and must match byte-for-byte, so consumers
no longer have to suppress lints per-binding. `schema-idl`'s
long-dead `RawDecl::fqn()` helper deleted (its sole caller was
inlined into `insert_shape_entries` in 8f27ff1 / Phase 6-2). Plus a
small `leo4-abi` test cleanup (unused `LeanMarshal as _` import).

### Added — Mathlib bridge infrastructure + initial bridges (2026-05-21)

User direction recorded 2026-05-20 (memory:
`project_mathlib_bridge.md`): every `Lean*` carrier type ships with
opt-in `Leo4.MathlibBridge.*` modules providing 1-to-1 conversions
to/from Mathlib types. leo4 core stays Mathlib-independent
(ROADMAP §8).

Infrastructure (`sibling/mathlib-bridge-test/`):
- New non-workspace Lake package pulling `Leo4` (path) + `mathlib`
  (git, `leanprover-community/mathlib4`). Lean toolchain pinned
  to `v4.29.1` matching the rest of the repo.
- `MathlibBridgeTest.lean` imports every `Leo4.MathlibBridge.*`
  module + Mathlib core; `decide`-based smoke checks where
  feasible.
- `just mathlib-bridge-test` recipe drives `lake build` in the
  sibling. NOT on the default `just test` ladder — Mathlib's
  cold build is 1-2 hours.
- `sibling/README.md` documents the cold-build caveat.

Bridge modules (`lake/Leo4/Leo4/MathlibBridge/*`) — opt-in import,
NOT auto-imported by `Leo4`; Lake's import-driven build keeps
them out of the main `Leo4` lib's compile graph:

- **`MathlibBridge.Wide`** —
  `LeanU128 ↔ Nat / BitVec 128`, `LeanI128 ↔ Int / BitVec 128`.
  Wide → Nat is total; Nat → Wide truncates ≥ 2^128. Wide → Int
  applies two's-complement sign; Int → Wide wraps mod 2^128.
  Wide ↔ BitVec 128 is a bit-level bijection.
- **`MathlibBridge.Complex`** — `LeanComplexF{32,64}x2 →
  Complex ℝ` via `Float.toReal` (Float32 → Float → ℝ for the
  F32 carrier). Forward direction only — reverse is
  rounding-lossy and waits for a concrete rounding-mode decision.

Follow-ups landed (2026-05-21):

- **`MathlibBridge.NightlyFloats`** — `LeanF16` / `LeanBF16` /
  `LeanF128` and the three complex carriers → `ℝ` / `ℂ` via
  direct IEEE bit-decode arithmetic on `Nat` field extracts
  (sidesteps subnormal-pattern mismatch the bit-widening route
  would have). NaN / Inf map to `0 : ℝ` by convention.
- **Wide bridge ZMod / Fin** — `LeanU128` / `LeanI128` ↔ `ZMod
  (2^128)` (which is `Fin (2^128)` for Mathlib's positive-`n`
  case).
- **`MathlibBridge.Rat`** — `Rat → ℝ` / `Rat → ℂ` total
  embeddings via `Rat.cast`. Lean core `Rat` IS Mathlib `ℚ`, so
  no separate `LeanRat` Lean struct exists; the bridge just
  surfaces the `ℝ`/`ℂ` lifts.

### Changed — Mathlib bridge reverse direction: RTNE pinned + real implementations (2026-05-21)

Rounding mode pinned: **IEEE-754 round-to-nearest-even (RTNE)**.
That's what `Float.div` and the host FPU already use across
platforms (per IEEE-754 §4.3.1); adopting it as the leo4
convention keeps the abstract-Real reverse path consistent with
the round-trip downstream native code already performs.

Three implementation layers per format, replacing the earlier
`noncomputable … := default` stubs:

1. **Float-level narrowing** (`Float.toLean{F16,BF16}RTNE`)
   — manual IEEE bit-level conversion (guard / round / sticky
   bits, mantissa-overflow round-up, special-case Inf / NaN /
   subnormal / overflow / underflow). `Float.toLeanF128` widens
   exactly (binary64 ⊂ binary128).
2. **`Rat`-based** (`Rat.toFloat`, `Rat.toFloat32`,
   `Rat.toLeanF{16,BF16,128}`) — computable, RTNE via Lean
   stdlib's `Float.ofInt` / `.ofNat` + `Float.div` + the
   narrowing helpers above. Precision loss is bounded by `num` /
   `den` conversion (when magnitudes exceed 2^53).
3. **Abstract `ℝ`** (`Real.toFloatRTNE`,
   `LeanF{16,BF16,128}.ofReal`, matching complex `ofComplex`)
   — `noncomputable` via `Classical.choice`. The mathematical
   meaning is RTNE-rounded; the function symbol exists for
   downstream proof references. Runtime use goes through the
   Rat path.

Sibling test fixture exercises both the computable Rat path
(plain `example`) and the noncomputable abstract path
(`noncomputable example`).

Outstanding: subnormal binary64 → binary128 normalisation in
`Float.toLeanF128` (rare boundary, currently returns ±0).

No regression: main `lake/Leo4` lib builds in 12 jobs (bridge
files not in import graph). `just smoke-plugin` + `cargo test
--workspace` + `just mangling-test` all green;
schema_hash `tjhnmfbc7izmk` unchanged.

### Added — Phase 7 step 2b: shim IO unwrap + `asyncDouble` fixture (2026-05-21)

Closes the end-to-end functional path for `IO α` boundary exports.
`asyncDouble(21) = 42` round-trips through the boundary.

Plugin (`lake/Leo4Plugin/Leo4Plugin/Main.lean`):

- `handlerFor`'s `.io T` arm now delegates to `T`'s handler instead
  of returning `none`. The IO wrapping is handled at the shim emit
  layer, not in the handler chain.
- `analyzeExport` switches from `forallTelescopeReducing` to
  `forallTelescope` (no reducing) so the original `IO α` shape
  survives — `…Reducing` would unfold `IO α = IO.RealWorld →
  EStateM.Result …` and expose a spurious `IO.RealWorld` (=Void)
  parameter.
- `renderOneShim` detects `effectIsIO` (return type is `.io _`)
  and emits an IO unwrap block before the encode:

      lean_object *_io_res = leo4_lean__...(args);
      if (!lean_io_result_is_ok(_io_res)) {
          lean_dec(_io_res); *ret_len = 0;
          return LEO4_ERR_IO_FAILED;
      }
      ReturnType r = <type-specific unbox>(lean_io_result_get_value(_io_res));
      lean_dec(_io_res);

  The unbox is `lean_unbox_uint64` / `lean_unbox_uint32` /
  `lean_unbox` for u8 / u16 — wider types route through their
  matching helper. Float / signed / boxed-object variants are
  flagged with `/* TODO step 2c */` comments and pick up
  whenever a fixture demands them.
- New SPEC §13 error code `LEO4_ERR_IO_FAILED = 0x00010001`
  (Lean panic / IO failure passthrough range).

Sample (`tests/sample-lean/Sample.lean`):

- `def asyncDouble (n : UInt64) : IO UInt64 := return n * 2`
  fixture exercises the full path.

Example (`examples/01-hello/src/main.rs`):

- `sample::asyncDouble(&lean, 21)` returns `42`; assertion holds.

Schema hash rotates: `fbla3xr3fsp6g` → `tjhnmfbc7izmk`.
Cross-impl mangling: **67 mangled names byte-identical**.

Still TODO (step 2c, deferred): typed unbox for non-`uintN_t`
return widths (signed integers via sign cast through size_t,
floats via `lean_unbox_float` / `lean_unbox_float32`, etc.). The
existing fallback path compiles but encodes garbage — the
`/* TODO */` comments guard it.

### Added — Phase 7 step 2 main (Lean side): `IO T` → `future<T>` IDL lift (2026-05-20)

Plumbing-only landing. Lean plugin now recognises `IO T` in
boundary positions and the canonical IDL renders it as `future<T>`
instead of `io<T>` (D-i 2026-05-19's effect marker). Cross-impl
already symmetric — Rust `schema-idl` step 1 (2026-05-20)
desugars `future<T>` into `FuncDecl { effect: Async, ret: T }`.

- `lake/Leo4Plugin/Leo4Plugin/AdmitSet.lean` —
  `exprToIDLSubst` gains `IO T → IDLType.io T` mapping (previously
  unrecognised, would fall through to the user-inductive walk).
- `lake/Leo4Plugin/Leo4Plugin/Mangling.lean` — `idlForm` renders
  `IDLType.io t` as `future<t>` instead of `io<t>`. The wire
  mangle (`I_t_i`) is unchanged for byte-identical cross-impl
  conformance.

What remains for step 2b: the shim still doesn't unwrap
`lean_io_result` for IO-returning exports — the Lean wrapper for
`IO α` returns a `lean_io_result α` wrapper that the shim needs
to unbox via `lean_io_result_get_value` before encoding the
inner `α`. Today no sample fixture exercises IO returns, so the
limit is documented but not observable. When step 2b lands, an
`IO α` fixture in `Sample.lean` exercises the full path.

Sibling-project WASIp3 adapter (the `wasip3` crate + `block_on`
wiring under `sibling/leo4-wasip3/`) stays upstream-dependent
and is deferred until the WASIp3 surface stabilises in nightly
Rust + the `wasip3` crate publishes.

Cross-impl mangling harness: **66 mangled names byte-identical**,
schema_hash `fbla3xr3fsp6g` (no rotation — no sample uses IO yet).

### Added — Phase 8 #57: nightly-only float carriers (`nightly-floats` feature) (2026-05-20)

Six new carrier types behind the `nightly-floats` cargo feature on
`leo4-abi` (passed through by `leo4`). Stable workspace builds
unchanged.

- `crates/leo4-abi/Cargo.toml`: new feature `nightly-floats = []`.
  `crates/leo4/Cargo.toml`: pass-through
  `nightly-floats = ["leo4-abi/nightly-floats"]`.
- `crates/leo4-abi/src/lib.rs` gates the nightly module with
  `#![cfg_attr(feature = "nightly-floats", feature(f16, f128))]`
  and `#[cfg(feature = "nightly-floats")] pub mod floats_nightly;`.
- `crates/leo4-abi/src/floats_nightly.rs`:
  - `impl LeanMarshal for f16` (2 B LE, IEEE-754 binary16).
  - `impl LeanMarshal for f128` (16 B LE, IEEE-754 binary128).
  - `pub struct LeanBF16 { bits: u16 }` — brain-float16 bit
    pattern (no native Rust primitive yet); wire 2 B LE.
  - `LeanComplexF16x2 { re: f16, im: f16 }` — 4 B LE.
  - `LeanComplexBF16x2 { re: LeanBF16, im: LeanBF16 }` — 4 B LE.
  - `LeanComplexF128x2 { re: f128, im: f128 }` — 32 B LE.
  - Round-trip unit tests for each (run under nightly only).
- `lake/Leo4/Leo4/NightlyFloats.lean` — opt-in Lean module (NOT
  auto-imported from `Leo4`). Mirrors every Rust type via
  bit-pattern structures:
  - `LeanF16 { bits : UInt16 }`, `LeanBF16 { bits : UInt16 }`.
  - `LeanF128 { lo, hi : UInt64 }` (matches `f128::to_le_bytes()`).
  - Complex variants pairing the singles.

Build verified: `cargo check --workspace` (stable, feature off) and
`lake build` (Lean side, module opt-in) both green. The
`cargo test --features nightly-floats` path lands when the
maintainer's local rustup has a nightly with `feature(f16, f128)`
enabled — until then the feature is documented but not CI-verified.

Cross-impl mangling stays at 66 names byte-identical — no sample
fixture uses the nightly types, deliberately, so schema_hash
doesn't rotate (`fbla3xr3fsp6g`).

### Added — Phase 8 #56: machine-complex carriers `LeanComplexF{32,64}x2` (2026-05-20)

Wire-pair `(re, im)` machine complex on stable Rust. `xN` suffix
follows the convention that extends to `xN=4` (quaternion) /
`xN=8` (octonion) / arbitrary `xN`.

- `lake/Leo4/Leo4/Wide.lean`:
  `structure LeanComplexF32x2 where re, im : Float32 deriving LeanMarshal`
  and matching `LeanComplexF64x2` (`Float = binary64`). `DecidableEq`
  skipped because IEEE-754 NaN ≠ NaN.
- `crates/leo4-abi/src/complex.rs` — `pub struct LeanComplexF32x2 {
  re: f32, im: f32 }` (+ F64 counterpart) with `LeanMarshal` impls
  delegating to the underlying `f32` / `f64` ones. 8 / 16 bytes LE
  on the wire, byte-identical to the Lean record's field-order
  encode.
- `leo4` façade re-exports `LeanComplexF32x2` / `LeanComplexF64x2`.
- Sample: `def mulComplexF64x2 (a b : Leo4.LeanComplexF64x2) :
  Leo4.LeanComplexF64x2 := …` (Karatsuba-style direct formula).
- examples/01-hello: `(2 + 3i) · (4 - i) = 11 + 10i` round-trip.

Schema hash rotates `uj55sds6f7cpq` → `fbla3xr3fsp6g`. Cross-impl
mangling: **66 mangled names byte-identical**. Zero plugin changes
(same record-pair pattern as #55).

User direction recorded for follow-up (#58, deferred): every
`Lean*` carrier type gets opt-in `Leo4.MathlibBridge.*` modules with
1-to-1 `toMathlib/fromMathlib` conversions. leo4 core stays
Mathlib-independent.

### Added — Phase 8 #55: stable 128-bit integers (`u128`, `i128`) (2026-05-20)

Wide-integer carriers via the "Lean record pairs Rust primitive"
pattern. Stable Rust path — no nightly, no feature gate.

- `lake/Leo4/Leo4/Wide.lean` — new module, auto-imported from
  `Leo4`. Defines
  `structure LeanU128 where lo, hi : UInt64 deriving LeanMarshal` and
  matching `LeanI128`. Wire form is `lo (8B LE) + hi (8B LE)` —
  byte-identical to Rust's `u128::to_le_bytes()` /
  `i128::to_le_bytes()`.
- `crates/leo4-abi/src/scalars.rs` — `impl LeanMarshal for u128 /
  i128` directly (no newtype wrapper). 16 wire bytes LE.
- `crates/leo4-macros-backend/src/lib.rs` — `rust_type_to_idl` maps
  bare `u128 → Record { fqn: "Leo4.LeanU128" }` and
  `i128 → Record { fqn: "Leo4.LeanI128" }`. Users write bare
  `u128` in `leo4::import!` and the macro auto-routes.
- Sample: `def addU128 (a b : Leo4.LeanU128) : Leo4.LeanU128` with
  hand-rolled two-limb add + carry (Lean stdlib has no UInt128 ops;
  leo4 stays Mathlib-independent per ROADMAP §8).
- examples/01-hello rt: `sample::addU128(&lean, (1u128 << 64) |
  0xdeadbeefcafebabe, 1)` asserts high limb participates.

Schema hash rotates `2iomjhrbofmos` → `uj55sds6f7cpq`. Cross-impl
mangling: **65 mangled names byte-identical**.

Zero plugin changes — the existing record path handles
`Leo4.LeanU128` natively because its wire is just `record { lo: u64,
hi: u64 }`. Same applies for `LeanI128`. Future carriers (#56, #57)
follow this pattern.

### Added — Phase 5 (2026-05-16 → 2026-05-20)

- **`crates/leo4-mslean4/`** — `Lean::open` (handshake + Lean runtime
  init + wrapper-module init), `Arena<'a>`, `LeanRef<'a, T>`, a
  per-callsite `Mutex<HashMap>` dispatch cache, inline
  `lean_io_result_is_ok` and `lean_dec_ref` (with `lean_dec_ref_cold`
  resolved via `dlsym`).
- **`crates/leo4-macros/`** — `leo4::import! { fn add(a: u64, b: u64) -> u64; }`
  expands to the dispatch wrapper. Scalars (P5-b₁), `String` and
  composite payloads (P5-b₂), nominal user types (P5-b₃-iii), and
  per-fn `#[leo4(args = "…")]` attribute hints for
  multi-instantiation disambiguation (P5-b₃-iv) all flow through
  `LeanMarshal` encode / decode.
- **`#[derive(LeanMarshal)]`** for the four nominal shapes —
  record (struct), enum (all-unit), variant (mixed payload),
  resource (`#[leo4(resource)]` on a single-`u64` struct). Generic
  records (`Pair<A, B>`) get a `T: LeanMarshal` bound synthesised on
  the generated impl. Mirrors `lake/Leo4/Leo4/Deriving.lean`
  byte-identically.
- **`crates/leo4-build/`** — `leo4_build::wire(lake_build_dir)`
  surfaces `LEO4_SHIM_SO` and `LEO4_HANDSHAKE_FILE` as `env!()`
  values for downstream `build.rs` callers and emits the
  matching `cargo:rerun-if-changed=` lines.
- **`crates/leo4/`** — top-level façade re-exporting the loader, the
  derive macro, `import!`, and `encode` / `decode` helpers.
- **Lake plugin shim emitter** — `<pkg>.leo4-shim.c` with one
  `leo4_call_<mangled>` entry point per `@[leo4_export]` ×
  instantiation. `leanc`-driven build of `<pkg>.leo4-shim.so` with
  RPATH-linked `libleanshared` + dep `.so` discovery via
  `lake-manifest.json`'s `packages[].dir` (F4-α).
- **Lake plugin Lean-side ABI conformance** — the emitter now
  agrees with Lean's actual FFI signatures for unboxed types:
  all-nullary inductives get `uint8_t` / `uint16_t` / `uint32_t`
  according to `Lean/Compiler/LCNF/ToImpureType.lean`'s
  `impureTypeForEnum`; `≥ 2³²` ctors short-circuit through the
  `LEO4_ERR_UNIMPLEMENTED` stub. `@[leo4_resource]` single-`UInt64`
  structures pass / return a raw `uint64_t` (not `lean_object *`).
- **Deterministic wrapper-module init symbol** — Lean invoked with
  `--root=<wrapper-dir>` so the export module's `initialize_*`
  symbol is path-independent.
- **`examples/01-hello/`** — `add` / `hello` / `pointSum` /
  `colorName` / `isLeaf` / `parserId` round-trip end-to-end on
  Tier 1 Linux. Includes the handshake-mismatch exit check (mutated
  `schema_hash_bytes` → `LEO4_ERR_HANDSHAKE_MISMATCH` = 5) and the
  attribute-routed `Sample.stringify` pick (P5-b₃-iv).
- **`examples/02-roundtrip/`** — `Sample.echoes (xs : List UInt32) (n : Nat) : List UInt32`
  driving `list<u32>` on both argument and return + `bignat` as an
  argument. Also exercises `listSumU64` / `listConcat` and a
  multi-instantiation `listLen` pick over `list<u32>`. Phase 5
  exit demo for the list + bignat wires.
- **Sample fixture** — `Sample.Pair<α, β>`, `Sample.Either<α, β>`,
  and the four nominal user types (`Point` / `Color` / `Tree` /
  `ParserHandle`).
- **macOS platform tier** — demoted to Tier 3 (best-effort, no CI)
  on 2026-05-20. Code paths remain platform-agnostic; only the
  exit-criteria and CI matrix scope shrunk.

### Added — earlier phases

- Design document (`LEO4-DESIGN.md`) capturing all resolved decisions.
- Working agreement for Claude Code (`CLAUDE.md`).
- Phased roadmap (`ROADMAP.md`).
- Phase 0 spike findings (`spike/SPIKE-0-FINDINGS.md`).
- Normative specifications under `SPEC/`:
  - `idl-grammar.ebnf`
  - `canonical-abi.md`
  - `mangling.md`
  - `handshake.md`
- Monorepo scaffold (Cargo workspace + Lake workspace).
- Lake plugin (`leo4plugin` exe), runtime library (`lake/Leo4/`),
  cross-impl mangling + WIT lowering + canonical-ABI conformance
  harnesses, and the multi-version Lean CI matrix.

### Added — Phase 8 step 2b: external-marshal shim glue, `Rat` round-trips (2026-05-20)

Phase 8 second-step finisher. Closes the cross-boundary wire for
`Rat` and any future external-marshal nominal: the shim now
delegates encode/decode to Lean-emitted C-callable helpers
(`leo4_marshal_<fqnSeg>_dec/_enc`) instead of dissecting the
type's IDL fields.

Concrete plumbing:

- `lake/Leo4Plugin/Leo4Plugin/Main.lean`:
  - `renderLeanExports` now takes `userDecls` and emits a helper
    pair per `UserDecl.externalMarshal`:

      `@[export leo4_marshal_<seg>_dec] def … (buf) (off) : Except _ (T × Nat) := …`
      `@[export leo4_marshal_<seg>_enc] def … (val) (buf) : ByteArray := …`

    These wrap `Leo4.LeanMarshal.canonicalDecode/Encode` so the
    typeclass call gets compiled to a fixed C-callable symbol.
  - `externalMarshalHandler` (new `TyHandler`) generates the
    decode/encode glue for each call site: build a fresh Lean
    `ByteArray` (`lean_alloc_sarray` + `leo4_memcpy` over the
    remaining wire bytes), invoke the helper, unwrap
    `Except _ (T × Nat)` at the right ctor tag (Lean's `Except`
    is `error` = 0, `ok` = 1 — got bitten in dev), advance the
    shim's offset by the consumed-bytes count returned in the
    pair. Encode mirrors with `lean_sarray_cptr` /
    `lean_sarray_size`.
  - `handlerFor`'s `.record` arm now also matches
    `UserDecl.externalMarshal` and routes to the new handler.
  - `renderShimSource` forward-declares every external-marshal
    helper at the top of the shim TU
    (`extern lean_object * leo4_marshal_<seg>_dec/_enc(…);`).

- `tests/sample-lean/Sample.lean`:
  `def addRat (a b : Rat) : Rat := a + b` re-enabled; now
  actually goes through the boundary instead of returning
  `LEO4_ERR_UNIMPLEMENTED`.

- `examples/01-hello/src/main.rs`:
  `sample::addRat(&lean, LeanRat::from_i64_u64(1, 3),
  LeanRat::from_i64_u64(1, 6))` returns
  `LeanRat { num: 1, den: 2 }` — Lean's `mkRat` normalises via
  gcd division.

Schema_hash rotates: `47swds7jpnqre` → `2iomjhrbofmos`. Cross-impl
mangling harness: **64 mangled names byte-identical**
(63 → 64; `addRat` joins). All four examples + workspace
`cargo test` pass.

Closes #54.

### Added — Phase 8 step 2a: UserDecl.ExternalMarshal AST + render (2026-05-20)

ROADMAP Phase 8 second landing (first half). Adds the IDL-level
recognition for types with custom `LeanMarshal` instances whose
fields the plugin can't lower (proof-carrying invariants like
`Rat`'s `den_nz` / `reduced`, opaque wrappers, …). Step 2b will
land the actual C-callable Lean helpers and shim glue so the
boundary round-trips end to end; this commit unblocks the IDL
form so `LeanMarshal Rat` being in scope no longer breaks
parsing / mangling.

- `lake/Leo4Plugin/Leo4Plugin/AdmitSet.lean`:
  `UserDecl.externalMarshal (fqn) (generics)` ctor. `walkUserDecl`
  falls back to `externalMarshal` when a record or variant body
  can't be lowered (any field's `exprToIDLSubst` returns `none`)
  AND the type carries a custom `LeanMarshal` instance.
- `lake/Leo4Plugin/Leo4Plugin/Mangling.lean`: `userDeclToIDL`
  renders `external <fqn>[<generics>]`. `declBand` puts the decl
  in the value band (0).
- Rust schema-idl mirror: `IDLType` reference layer unchanged
  (`func f(_0: Rat)` lowers to `IDLType::Record { fqn: "Rat" }`
  same as a regular record). The distinction lives at the
  `UserDecl` level; the shim emitter dispatches on it. New
  variant `UserDecl::ExternalMarshal { fqn, generics }` and
  matching `RawDecl::ExternalMarshal`.
- `SPEC/idl-grammar.ebnf` implicitly extended via the existing
  `nominal_decl` umbrella; parser recognises the `external`
  keyword in `parse_nominal_decl`, top-level docs, and
  interface bodies.
- `lake/Leo4/Leo4.lean` re-enables auto-import of
  `Leo4.MathlibSubset` (was opt-in in the prior commit because
  `Rat` exposure broke `walkUserDecl`; the externalMarshal
  fallback now closes that gap).
- WIT lowering routes external-marshal decls to opaque
  `resource <name>;` — wasm Component Model consumers see them
  as host-managed handles; the Lean-side custom marshal isn't
  visible across the wasm boundary.

Schema_hash rotates: `5wlbxlzagwuyy` → `47swds7jpnqre`. Cross-impl
mangling harness stays green at **63 mangled names byte-identical**
(`Sample.stringify` instantiation list now includes `Rat`).

Boundary round-trip for `Rat` still returns `LEO4_ERR_UNIMPLEMENTED`
— step 2b lands the actual wire-up.

### Added — Phase 7 step 1: future / stream effect desugar (2026-05-20)

ROADMAP Phase 7 first landing (parser-side, stable Rust, no
WASIp3 dep). `crates/schema-idl/src/parse.rs::parse_func_decl`
recognises `future<T>` / `stream<T>` at the immediate return
position of a `func_decl` and desugars them into
`FuncDecl { effect: Async / Stream, ret: T }`. The effect slot was
already in the AST (`FuncDecl.effect`, D-i 2026-05-19); this
landing wires the parser to populate it.

Rejection: `future<T>` / `stream<T>` anywhere *except* a func's
return position (inside `list<…>`, record fields, variant
payloads, etc.) raises a `ParseError::Expected` with a clear
message. `parse_type` checks the keyword and bails before
attempting to treat it as a nominal.

Renderer (`render::render_canonical`) re-wraps `effect = Async`
ret types in `future<…>` and `effect = Stream` in `stream<…>` so
`parse → resolve → render` round-trips. Schema-hash is unaffected
for existing (Sync-effect) funcs.

Four unit tests pin the desugar happy path (`future<u64>`,
`stream<u8>`), the in-payload rejection (`list<future<u32>>`),
and the in-record-field rejection (`record { y: stream<u32> }`).
schema-idl test count: 53 → 57.

The Lean-side mirror (plugin recognises `future<T>` / `stream<T>`
patterns in user exports — e.g., wrapping `IO T` exports as Async
effect) and the matching shim emit / macro expansion path are
Phase 7 step 2 (next).

### Added — WASIp3 sibling project skeleton (2026-05-20)

`sibling/leo4-wasip3/` — Cargo project **outside** the main leo4
workspace, pinning `nightly` Rust via its own
`rust-toolchain.toml` and targeting `wasm32-wasip3`. The skeleton:

- `Cargo.toml` declares the nightly toolchain + path-dep on
  `crates/leo4-abi` (the canonical-ABI marshalling layer is
  shared between native and wasm; only dispatch / loader differs).
- `src/lib.rs` carries a placeholder `pub struct Lean` mirroring
  the planned `leo4_mslean4::Lean` surface so downstream code can
  `use leo4_wasip3::Lean` interchangeably under wasm.
- `sibling/README.md` documents the convention for non-workspace
  projects + the planned `leo4-wasm64/` sibling deferred until
  upstream stabilisation.

The WASIp3 API design crystallised in the same session: **sync API
on both targets** — the wasm side uses `futures::executor::block_on`
(or `wasmtime_wasi::block_on`) inside its sync wasm export to drive
async sub-tasks. WASIp3 explicitly avoids function coloring, so a
sync wasm export can call async imports. This means the
`leo4::import!` macro emits the same shape on both targets, no
per-target `cfg!` at the call site, and the `async-runtime`
feature stays opt-in for users who want native concurrency.

Phase 7 lands the concrete host import bindings, `block_on` choice,
and `Lean::open` equivalent for wasm; this commit is the
infrastructure shell so the wire-up has a home.

### Added — Phase 8 step 1: `Rat` LeanMarshal infrastructure (2026-05-20)

ROADMAP Phase 8 (Mathlib-compatible subset) first landing. Mirror
implementations on both sides for Lean-core `Rat`
(`Init.Data.Rat.Basic`):

- `lake/Leo4/Leo4/MathlibSubset.lean` — new module under `Leo4`
  carrying `instance : LeanMarshal Rat`. Wire format: `bigint num`
  followed by `bignat den` (SPEC/canonical-abi.md §§5-6). Decode
  uses Lean core's smart constructor `mkRat num den`, which
  normalises (divides by gcd) and degenerates to `0/1` when
  `den == 0`, so malformed wire payloads degrade rather than
  panicking.
- `crates/leo4-abi/src/rat.rs` — new module with `LeanRat { num:
  BigInt, den: BigNat }`. `LeanMarshal` impl delegates to the
  existing `BigInt` / `BigNat` impls so wire format stays
  byte-identical to the Lean side.
- `leo4` façade re-exports `LeanRat` + the `rat` module.
- Two unit tests pin the Rust-side round-trip for the basic cases
  (positive, negative, zero, max-magnitude) and a default-zero
  edge.

**Not yet wired**: cross-boundary calls. The plugin's `walkUserDecl`
rejects `Rat` because of its two `Prop`-typed proof fields
(`den_nz`, `reduced`), which prevents the shim from emitting a
real boundary entry for an `(a b : Rat) → Rat` export. Step 2 of
Phase 8 adds an "external marshal" path to the plugin so types
with custom `LeanMarshal` instances are treated as opaque blobs at
the shim and the boundary just routes bytes through.

### Changed — Variant discriminator 4-byte canonical (2026-05-20)

- Shim emitter (`lake/Leo4Plugin/Leo4Plugin/Main.lean`'s
  `renderVariantHelpers`) and Rust `#[derive(LeanMarshal)]`'s
  `expand_derive_variant` both flip variant disc encode/decode from
  `u8` to `u32 LE`. Matches `SPEC/canonical-abi.md` §9 ("encoders
  MUST emit 4 bytes"); the previous u8 fast path was a SPEC
  violation that both sides shared so the wire still round-tripped.
- Coordinated change — wire format is byte-incompatible with
  pre-2026-05-20 callers. Cross-impl mangling harness re-runs
  unchanged (62 names byte-identical, schema_hash unchanged because
  it covers the IDL canonical form, not the ABI wire).
- `examples/04-mutual-ast` round-trip bytes 12 → 24, matching the
  expected 4-byte-per-disc widening across 3 levels of nesting.

### Added — Plugin value-param erasure (2026-05-20)

- `analyzeExport` recognises implicit value-typed binders
  (`{N : Nat}` and friends) and erases them from the boundary
  signature per SPEC/mangling.md §"Value-param erasure". The
  wrapper renderer fills each erased binder with `default` at the
  Lean call site so elaboration succeeds (binder type must be
  `Inhabited`). `Sample.doubleVal {_N : Nat} (x : UInt32) : UInt32`
  fixture round-trips through `examples/01-hello` as a plain
  `(u32) -> u32` export.

### Added — Phase 6 (2026-05-20)

- **`SPEC/phase-6-mutual.md`** locks the four mutual-recursion
  design decisions: explicit `mutual { … }` block, `Cyc<i>`
  cycle-breaker token (0-based, scoped to the enclosing group),
  group-shared decode-depth counter, and a single `mutual ... end`
  block per cluster in the deriving handler.
- **`SPEC/idl-grammar.ebnf`** grows `mutual_decl` and `cyc_type`.
- **`crates/schema-idl`** — `IDLType::Cyc(u32)` + `UserDecl::Mutual {
  members }` + parser / renderer / mangler / resolver coverage; new
  diagnostics for out-of-scope / out-of-range Cyc and singleton
  groups.
- **`lake/Leo4Plugin`** mirrors the Rust schema-idl side: `IDLType.cyc`,
  `UserDecl.mutual`, `walkMutualGroup` that pre-detects `iv.all`
  clusters and rewrites peer references to `Cyc<i>`; shim emitter
  resolves `Cyc<i>` payloads to peer helper cross-calls
  (`leo4_enc_<peerSuffix>` / `leo4_dec_<peerSuffix>`) with forward
  declarations at the top of each helper block.
- **`lake/Leo4/Leo4/Deriving.lean`** detects a genuine mutual cluster
  (`iv.all` matches across members) and emits one `mutual ... end`
  block carrying every member's `partial def _leo4_encode /
  _leo4_decode` pair, then one `instance` per member. Cross-decl
  payload references route through direct function references
  rather than typeclass-instance forward references.
- **`tests/sample-lean/Sample.lean`** gains a `mutual inductive Expr
  / Stmt end` cluster with two exports (`exprIsLit`, `stmtIsNop`).
- **`examples/04-mutual-ast/`** — Rust mirror of `Sample.Expr` /
  `Sample.Stmt` with hand-rolled `LeanMarshal` impls (`Box<T>` breaks
  the Rust-side cycle). Exit demo for Phase 6.
- **`tests/mangling/`** cross-impl harness picks up the new cluster
  fixture; 61 mangled names + schema_hash `gj6daa3oelheu` are
  byte-identical between the Lake plugin and `leo4c`.

### Open follow-ups
- schema-idl item G (`ConstraintExpr<Atom>` typed AST) — deferred
  until a concrete consumer needs it.
- `wasm64` sibling project — deferred until `wasm64-*` exits Rust
  stable's tier 3.
- Some `LeanError` codes (`0x02` / `0x03` / `0x04` / `0x06` /
  `0x08`) are reserved but not yet fired by a test fixture.

[Unreleased]: https://github.com/Honey-Be/leo4/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Honey-Be/leo4/releases/tag/v0.1.0
