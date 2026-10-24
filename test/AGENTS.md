# Agents Guide — shared spec fixtures

`spec/*.tsv` holds the cross-runtime conformance fixtures. Every runtime
runs **every** file in this directory, so a change here is a change to
the contract each port is held to — edit with that in mind. Today the
Rust crate is the only runtime; the TypeScript and Go ports run the same
files, unchanged, when they arrive.

## Format

Tab-separated, one case per line, with a header row naming the columns.
Blank lines are skipped, and so are comment lines — a line starting with
`#` that contains no tab.

| File | Columns | A row is |
|---|---|---|
| [`reader.tsv`](spec/reader.tsv) | `input`, `expected` | a program → its canonical form, as a JSON string; or the reader's error |
| [`pipe.tsv`](spec/pipe.tsv) | `input`, `expected` | a program → the canonical form of the desugared program; or the desugarer's error |
| [`check.tsv`](spec/check.tsv) | `input`, `expected`, `protocol` | a program → its `explain` report, as a JSON string; or the resolver's, checker's or plan evaluation's error |
| [`run.tsv`](spec/run.tsv) | `input`, `expected`, `doc`, `render` | a program run over a JSON document → the exact bytes it writes, as a JSON string; or the failure |
| [`transduce.tsv`](spec/transduce.tsv) | `input`, `expected`, `grammar`, `doc`, `render`, `duplicates` | a program over a grammar's rule passes (a transducer), run over a document in that grammar → the exact bytes it writes, as a JSON string; or the failure |

| Column | Meaning |
|---|---|
| `input` | The program's text. Escapes `\n` `\r` `\t` `\\` are decoded. |
| `expected` | A JSON string (the canonical text, the report, or the output bytes), or `ERROR:<code>` / `ERROR:<code>@<row>:<col>` for a program or run that must fail. Not escape-decoded: JSON's own escapes apply, so `"a\r\n"` ends in CRLF. |
| `doc` | `run.tsv` and `transduce.tsv`. The document the program runs over, escape-decoded like `input`: in `run.tsv` a JSON document, and an empty cell is `null`; in `transduce.tsv` a document in the row's `grammar`, and an empty cell is the empty text. |
| `render` | `run.tsv` and `transduce.tsv`. How a table or JSON-events result is rendered: empty (the host's default: CSV for a table, JSON for events), `csv` or `json`. A program that renders its own text takes none (`render_of_text`). |
| `grammar` | `transduce.tsv` only. The grammar whose rule passes the program reads: the `languageId` of a format alchemy-cli carries (`json` first, then the others it depends on). The document is parsed with that package's parser, as `alchemy run --grammar` parses it. |
| `duplicates` | `transduce.tsv` only. The policy for a member name an object repeats within a captured value (spec section 18.2), which the runner sets with `with_duplicates`: empty, the default, refuses the capture (`DUPLICATE_MEMBER`); `first` keeps the earlier value; `last` the later. |
| `protocol` | `check.tsv` only. The input the host declares when it compiles the program: empty for JSON events (what `compile` declares), `parser` for the parser's rule passes (`compile_with` over `Input::ParserEvents`). Every runner of `check.tsv` reads it, alchemy-cli's catalogue test included. |

A run is what `alchemy run` does: compile the program (which builds the
plan), then read `doc` with the tabnas JSON grammar incrementally, pruned
under the program's row selector when it has one, and push its events
through the program's sink, with the default `Limits`. A run needs
transduce's routers and render's renderers, which alchemy does not
depend on, so `run.tsv` is run by
[tabnas/alchemy-cli](https://github.com/tabnas/alchemy-cli), the
composition root, in every runtime (see "Who runs what"). Every runtime
runs the library's text, and is held to the same bytes, codes and
positions.

A `transduce.tsv` run is what `alchemy run --grammar` does: compile the
program over the parser's rule passes (`compile_with` over
`Input::ParserEvents`), read `doc` with the row's grammar through
transduce's `ParserEventSource`, and push each pass through the program's
parser sink (`Program::parser_sink`), with the default `Limits`. A
row's program is a transducer, its events the document's, and the runner
holds it to that: for every row whose program answers JSON events, whose
document the grammar accepts and which repeats no member, it also
compares the events the program writes with the walk of the grammar's
own value (transduce's `ValueSource`), a number's lexeme aside, and fails
on any difference. The other rows have no such oracle and are held by
`expected` alone: a document the grammar refuses has no value to walk,
and its program must fail with the declared code before `End`; a repeated
member is streamed twice where the value keeps one, and its rows pin what
the plain transducer, `tree-contract` and a capture make of it, under the
default policy and under `first` and `last` (the `duplicates` column,
which the runner hands to `with_duplicates`).

## Error expectations

The **code** is the contract, never the message. Which code a row pins:

- A failure with this crate's own codes (`DSL_PARSE_ERROR`,
  `DSL_TYPE_ERROR`, `STREAM_REUSED`, `STREAMABILITY_UNKNOWN`) pins its
  **finer code**, the first word of the message before `: `
  (`unbalanced`, `empty_step`, `type_mismatch`, `reused`, `recursion`,
  `duplicate_key`, `no_match`, `render_of_text`, ...). The full list is
  in `../AGENTS.md`, "Error codes", and `docs/language.md`, "Spans and
  errors".
- Any other failure pins the transduce or render code itself:
  `INPUT_INVALID` (a program's `fail`, a document that is not JSON, the
  library's own refusals), `INPUT_ORDER_VIOLATION`, `MISSING_VALUE`,
  `TARGET_VALUE_UNREPRESENTABLE`, `PROTOCOL_ORDER_ERROR`,
  `RESOURCE_LIMIT_EXCEEDED`. Their messages are free text: `fail "a: b"`
  is `INPUT_INVALID`, never `a`. (`reader.tsv`, `pipe.tsv` and
  `check.tsv` hold no such failure today, and their runner reads the
  first word whatever the code; `run.tsv`'s runner makes the
  distinction, and a port's runners should make it for every file.)

A row pins the code; the grammar document holds the message. Every
finer code a row meets, the engine's aside, is declared in
`../alchemy-grammar.jsonic`, `options.error` and `options.hint`, with
the text its raising sites write (`{name}` for what a site fills in, a
line per sentence), and each runtime's
`the_raised_messages_match_the_document` runs every error row here and
wants each failure's code declared there, its text a line of that code's
entry, each finer code raised with one code (`bad_let` from the
desugarer in `pipe.tsv` and from the resolver in `check.tsv` alike), and
every line met by some row. This repository's copy runs the rows of
`reader.tsv`, `pipe.tsv` and `check.tsv` and holds them to all but the
last clause; alchemy-cli's runs every row of all four files and holds the
last clause too. A row that meets a new failure,
or a new sentence of a known one, comes with its line in the catalogue.

A trailing `@<row>:<col>` also pins the 1-based position the failure
names: the program's form, for the reader, the checker and a program's
own runtime failures; the document's, for a document that is not JSON.
It is a stronger contract than the code alone, not a message: keep a
position where the failure has one to name. `@tabnas/support` reads
`ERROR:<code>@<row>:<col>` as a code and a position, in every runtime,
and so do the fleet's audits: admin's `ax-codes` counts a located cell
as exercising its code, and `ax-audit` counts it as a code, not a
message. A code pinned only with a position needs no bare row as well.

## Who runs what

Here, in each runtime: `reader.tsv`, `pipe.tsv` and `check.tsv`, which
compile with inert stages.

- Rust: `rs/tests/spec_test.rs`, one `tabnas_support::Runner` per file.
  `format_round_trips_every_fixture_row` reads every row's program back
  from its layout form.
- TypeScript: `ts/test/spec.test.ts`.
- Go: `go/spec_test.go`.

Each runtime's every-fixture-has-a-runner test fails when a file is added
without a runner, and names `run.tsv` and `transduce.tsv` as run by
alchemy-cli.

In [tabnas/alchemy-cli](https://github.com/tabnas/alchemy-cli), against
this repository's `test/spec/` in the sibling checkout: `run.tsv` and
`transduce.tsv`, and the full catalogue test
over all five files.

- Rust: `rs/tests/spec_test.rs`.
- TypeScript: `ts/test/spec.test.ts`.
- Go: `go/e2e`'s `TestSpecRun` and `TestSpecTransduce`. Rows that read a
  document need `-tags tabnas_nodecell`, which alchemy-cli's gate passes.

This repository's `.github/workflows/downstream.yml` runs alchemy-cli's
gates against each change here.

Finding `test/spec`, reading a file, decoding escapes, the `ERROR:`
contract, the comparison and the `<file>:<line>` in a failure message
come from [`@tabnas/support`](https://github.com/tabnas/support) and its
Go and Rust halves, so the loaders cannot drift from each other either.
An empty fixture fails, and so does a spec directory with no fixtures in
it.

## Rules

- Every `alchemy` example in `docs/language.md`, with the `canonical`,
  `core` or `check` block after it, is a row of `reader.tsv`, `pipe.tsv`
  or `check.tsv`, and
  `every_language_reference_example_is_a_fixture_row` fails when one is
  not. Change the page and the row together.
- Prefer a fixture here over an in-language assertion whenever a case is
  expressible as program (and document) → text or code. That is what
  keeps the runtimes honest against each other.
- The Rust crate is the reference implementation: the ports match it.
  Where a port exposes a genuine Rust defect, fix Rust first and pin the
  corrected behaviour here; record any difference that stays in a
  `DIVERGENCE.md`.
- A new row must pass in every runtime that exists: `cargo test
  --all-targets` from `rs/` (or `make test` from the root), and, for a
  `run.tsv` or `transduce.tsv` row, alchemy-cli's gates
  (`../alchemy-cli/ci/rust/run.sh` and `../alchemy-cli/ci/polyglot/run.sh`,
  which downstream.yml runs).
