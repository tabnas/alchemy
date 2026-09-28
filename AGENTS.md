# Agents Guide — alchemy

This repository is **tabnas-alchemy**: a small, typed, functional
streaming language in which both transducers and renderers are written,
parsed by a **tabnas grammar plugin** like every other grammar in the
fleet, and compiled onto [tabnas-transduce](https://github.com/tabnas/transduce)
and [tabnas-render](https://github.com/tabnas/render).
[aless](https://github.com/rjrodger/aless) runs alchemy programs against
any format its parsers read. `CLAUDE.md` is a symlink to this file.

## Core principle: dependencies change only on explicit instruction

**Dependencies may only be changed by explicit instruction from the
maintainer.** This covers every dependency `rs/Cargo.toml` and
`rs/Cargo.lock` declare. Adding, removing, re-pointing or re-versioning
any of them is a dependency change.

- **A dependency never arrives as a side effect.** If a change would alter
  a dependency, stop and ask before making it.
- **An explicit instruction names the change.** A goal is not an
  instruction for its means.
- **This repository's own version sites are not dependencies.**
- **Versions track the latest release.**

The sibling tabnas crates are taken by path from sibling checkouts
(admin ADR-21: committed manifests stay path-only).

## Core principle: transient tasks report progress

**Every transient task produces status output at least every 30 seconds,
with an estimate of how far through it is, as a percentage, where one can
be made.** A build, a test run, a fixture sweep, a wait on CI: each prints
a line per step or per interval. A quick command that finishes within 30
seconds needs nothing extra.

## What this project is

Read [`docs/language.md`](docs/language.md) (the language) and transduce's
[`docs/architecture.md`](https://github.com/tabnas/transduce/blob/main/docs/architecture.md)
section 4 (the design this crate implements). In one paragraph: the
**reader** is a tabnas grammar over layout S-expressions: two-space
indentation makes lists, a line with several forms or with children is a
list, a line with exactly one form and no children is that form, parens
are explicit lists, brackets are vectors, `;` starts a comment, strings
take JSON escapes, `:name` is a keyword. The grammar builds a tagged value
the **AST** module turns into `Expr` with source spans. **Desugaring**
rewrites `def name [params] body`, `pipe` (always data-last), and the
other conveniences into the core forms (`def`, `fn`, `let`, `if`,
`match`). The **checker** infers types, enforces affine streams (a stream
is consumed at most once) and protocol composition, and computes an
effect summary the **planner** reports (`explain`). The **interpreter**
evaluates values eagerly and streams lazily, lowering `route`,
`scan-emit`, `csv`, `json` and the text algebra to transduce and render
sinks; the standard compositions run natively, and a differential test
proves the interpreted standard-library definitions in `stdlib/*.alc`
produce the same bytes.

## The reader repeats by replacement, never by a push chain

**Every repetition is a replace loop: the loop is `r:`, the item may be
`p:`.** An alternate that hands control to another rule either pushes it
(`p:`), opening a frame that keeps the pusher on the stack for its close
phase and links parent to child, or replaces with it (`r:`), re-entering
in the same frame, handing the parent on and linking `prev`; an alternate that only matches its tokens, or pops the frame to end the rule, does neither. Push is for structure (a
list inside a list, a block under its line), replace for sequence (the
next line, the next form, the next item). The iterations of a repetition
then add no depth: rule depth follows a program's nesting, which
`MAX_NESTING` bounds, and never its length. A repetition spelled as a
push chain, each item pushing the rule that reads the rest, parses and is
still wrong: depth grows with the item count until a guard refuses a flat
input (aless refuses a parse past 3,000 open rules, its `MAX_RULE_DEPTH`;
tabnas-json refuses nesting past 128 levels), the rule stack and memory
grow with it (the rewind history does not: it records consumed tokens
either way and is capped), and the tree comes out nested where the source
is flat.

**The reader is in that shape.** In the grammar document in
`rs/src/grammar.rs`, a container pushes its first item and the item's
close replaces it with the next, in the same frame, so the container's
close runs once, after the last item. `program` and `block` push the
first `line`, and `line`'s close replaces it with the next line
(`r: line`) on `#NL`, or after its block on the next line's first token.
`line`, `paren` and `bracket` push their first `form`, and `form`'s close
replaces it with the next form (`r: form`) until the token that ends its
parent's sequence: the closer for `paren` and `bracket`, a layout token
or the end of the source for a line, left (`b: 1`) for the parent to
take. Each item appends its finished value to its container's node,
which a replace hands on as `parent_node` (`@alchemy-item` for a form,
`@alchemy-line-end` for a line); nothing reads an item back through the
push link (`child_value`), which names only the first item of a loop.
The pushes that remain are structure: `line`'s `#IN` into `block` and
`form`'s openers into `paren` and `bracket`.

`rs/tests/repeat_test.rs` pins it, reading `d` with a rule subscriber on
`make()`: over 10,000 items each repetition reaches the maximum depth one
item needs (top-level lines 2, child lines under one line 4, a line's
inline forms 2, the items of `( )` and of `[ ]` 4), while 100 nested
parens reach 201, as real recursion should; the installed grammar's only
close-phase push is the block and its only replaces are those two loops;
and ten times the lines parse in about ten times the time. Write any new
repetition the same way. The resolver, the checker and the
interpreter generate no grammar rules (they work on `Expr`); the one
other grammar this crate runs is tabnas-json, on a program's `json`
input, and its repetitions are that repository's to keep. Rule depth over
a repetition is constant; a test that repeats an item ten thousand times
and asserts the maximum `d` stays what a single item needs is the proof.

## Repository map

The pipeline reads left to right: `lex`/`grammar` → `ast` → `desugar` →
`resolve` → `check` → `interp` (builds the plan) → `lower` (the sinks);
`effects` reads the plan for `explain`; `program` is the API around it.

| Path | What it is |
|---|---|
| `rs/src/lex.rs` | the layout lex matcher: `#IN`, `#DE`, `#NL`, `#KW`, words and the delimiter-depth scan |
| `rs/src/grammar.rs` | the grammar plugin: the document, `alchemy()`, `make()`, `parse_value()`, `parse()`, `parse_file()` |
| `rs/src/ast.rs` | `Expr`, `SourceSpan`, `same_shape`, `canonical()` and layout `format()` |
| `rs/src/desugar.rs` | `def` with parameters, `pipe`, the shapes of `let`, `if` and `match` |
| `rs/src/resolve.rs` | scopes and linking: top-level `def`s in any order, locals, the library's names; `unknown_name`; recursion refused |
| `rs/src/types.rs`, `rs/src/check.rs` | the types and the checker: inference, affine streams (`STREAM_REUSED`), protocols, strict mode (`STREAMABILITY_UNKNOWN`), the `export` contract |
| `rs/src/effects.rs` | the effect summary and the `explain` report, as text and as JSON |
| `rs/src/value.rs` | runtime values, every one `Send`; streams and texts as plans |
| `rs/src/interp.rs` | the evaluator: definitions, closures, natives, partials, patterns, the two scopes, the native fast paths |
| `rs/src/lower.rs` | plans to sinks: `Router`, `ScanEmit`, `TableFromJson`, the renderers, the text algebra |
| `rs/src/program.rs` | the API a host embeds: `compile`, `Program::{output, row_selector, explain, explain_json, sink}` |
| `rs/src/stdlib/registry.rs` | the natives: arity, kind, implementation, signature and effect |
| `rs/src/stdlib/mod.rs`, `stdlib/*.alc` | the standard library's own definitions, embedded, resolved and checked on first use |
| `rs/src/bin/alchemy.rs` | `alchemy canon | format | check | explain | run` |
| `rs/tests/spec_test.rs` | the shared fixtures through `tabnas_support::Runner`, the layout round trip, the reference's examples |
| `rs/tests/debug_model_test.rs` | the grammar composed with `tabnas-debug`, as every grammar carries |
| `rs/tests/repeat_test.rs` | every repetition a replace loop: rule depth over 10,000 items of each, the grammar's pushes and replaces, linear parse time |
| `rs/tests/cli_test.rs` | the built binary, run as a script runs it |
| `rs/tests/run_test.rs` | the API end to end: the worked example both ways, the spec's streaming rows, the `json` echo, `records` |
| `rs/tests/stdlib_test.rs` | the differential test: the interpreted library against the native path on every fixture and generated document |
| `test/spec/reader.tsv` | shared fixtures: layout → canonical, and the reader's errors by code |
| `test/spec/pipe.tsv` | shared fixtures: layout → canonical of the desugared program, and the desugaring errors |
| `test/spec/check.tsv` | shared fixtures: program → `ERROR:<finer code>` for every resolver and checker code, and program → plan report |
| `docs/language.md` | the language reference; every example in it is a fixture row |
| `ci/rust/run.sh` | the gate `.github/workflows/rust.yml` runs |

## Verify your work

From `rs/`:

```bash
cargo fmt --check
cargo build --all-targets
cargo test --all-targets
cargo test --doc
cargo clippy --all-targets --all-features -- -D warnings
```

`ci/rust/run.sh` runs exactly that and needs the sibling checkouts its
header lists. Every `dsl` example in the design document and in
`docs/language.md` is a row in `test/spec/`, run by the shared fixture
runner, so a stale example fails the gate.

## Error codes

This crate raises codes from `tabnas_transduce::Code`, the one shared set.
Its own are `DSL_PARSE_ERROR` (the reader: a tab in indentation, a dedent
to no level, an unterminated string or list, an invalid pipeline step) and
`DSL_TYPE_ERROR` (an unknown name, a wrong argument type or count, a
protocol mismatch), plus `STREAM_REUSED` and `STREAMABILITY_UNKNOWN` from
the checker. Runtime failures carry the transduce and render codes
unchanged. A diagnostic names the source span (file, row and column) of
the user's declaration, not a generated node. The code is the contract;
the message is informative.

A `DSL_PARSE_ERROR` carries a finer code as the first word of its
message, before `: ` -- the grammar's own (`tab_indent`, `bad_indent`,
`bad_dedent`, `unbalanced`, `too_deep`, and for desugaring `empty_step`,
`bad_def`, `bad_let`, `bad_if`, `bad_match`), declared in the grammar document's
`options.error` and `options.hint`, or the engine's (`unterminated_string`,
`unprintable`, `unexpected`). That word is what a fixture pins as
`ERROR:<code>`, so it is part of the contract too: never rename or
repurpose one; add one when a new failure needs it, with its message and
hint.

The later stages follow the same convention. `DSL_TYPE_ERROR` carries,
from the resolver, `unknown_name`, `not_def`, `duplicate_def`,
`reserved`, `bad_fn`, `misplaced_def`, `bad_pattern`; from the checker,
`arity`, `type_mismatch`, `protocol_mismatch`, `no_export`,
`bad_output`; from the runtime, `duplicate_key` (a record with two
entries for one key), `no_match` (a `match` no case took) and
`render_of_text` (a renderer asked of a program that renders its own
text). `STREAM_REUSED` carries `reused` or `captured`;
`STREAMABILITY_UNKNOWN` carries `recursion` (from the resolver, a
definition that reaches itself; from the evaluator, nesting past
`MAX_EVAL_DEPTH`, which a function applied to itself reaches),
`dynamic` or `unknown_output`. `test/spec/check.tsv` pins each code
`alchemy check` can reach, with the position it names: `check` builds
the plan, so `duplicate_key`, `no_match` and the evaluator's
`recursion` have rows where the plan's own evaluation meets them.
`render_of_text` needs a renderer, which `check` never takes;
`rs/tests/run_test.rs` and `rs/src/lower.rs` pin it. Runtime failures
carry the transduce and render codes unchanged, and a `fail "message"`
in a program is `INPUT_INVALID` with the message and the form's
position, from `check` too when the plan's evaluation reaches it.

## Untrusted input

**A program is code; a document is data; the two never mix.** Data-supplied
paths are validated segment vectors (`as-path`) and are never read as
source. The interpreter resolves only registered operators and the
definitions in front of it: there is no `eval`, no host function access,
no I/O from a program. A function a program obtains from data (a record
field) can be applied to values, but strict mode refuses it where a
stream operator would run it per item.

What bounds a program's text is its nesting: at most `MAX_NESTING` (256)
levels, counting a layout line, each indentation level and each open
delimiter as one, refused by the reader as `too_deep` before anything is
built, and held after desugaring, where a `pipe` nests a level per step.
The bound is what lets the printers, the desugarer and `Drop` recurse per
level; without it a two-kilobyte program of nested parentheses aborted
the process with a stack overflow. The syntax tree is otherwise bounded
by the source's size. The checker types definitions in dependency order,
so a chain of definitions each naming the next does not nest it, and
follows a stream into the bodies it is passed to at most `MAX_APPLIED`
(32) definitions deep.

A program is code, and it runs twice: `compile` evaluates everything a
stream does not defer to build the plan, and the run evaluates the
per-item functions. Both are bounded: evaluation nests at most
`MAX_EVAL_DEPTH` (1,000) levels (`recursion` past it: a function applied
to itself, or definitions, calls and values chained that deep), and
building the plan takes at most `MAX_PLAN_STEPS` (1,000,000) evaluation
steps (`RESOURCE_LIMIT_EXCEEDED` naming `max_plan_steps`; a program of
forty nested doublings asks for 2^40). Those bounds are only reached
before the stack's end on a thread of `STACK_BYTES` (64 MiB): `compile`
makes one, the `alchemy` command runs on one, and a host that pushes
events into a sink must run it on one. The work of one item is bounded
by the host's abort flag (`Program::with_abort`), read every few
evaluation steps. At run time every transduce limit applies as the
stage that holds the data names it (`max_capture_bytes` for a program's
`capture` or the limit its third argument names, `max_record_bytes`,
`max_metadata_bytes` and `max_columns` for both tables,
`max_metadata_bytes` and `max_depth` for a `scan-emit` state,
`max_scalar_bytes` for a cell's JSON text, `max_depth`,
`max_scalar_bytes` and `max_key_bytes` at the source), and the writer
enforces `max_output_bytes`, which also bounds a finite text and one
item's text as they are built. Hosts that run programs they did not
write set `max_output_bytes` and a timeout.
