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
| [`check.tsv`](spec/check.tsv) | `input`, `expected` | a program → its `explain` report, as a JSON string; or the resolver's, checker's or plan evaluation's error |
| [`run.tsv`](spec/run.tsv) | `input`, `expected`, `doc`, `render` | a program run over a JSON document → the exact bytes it writes, as a JSON string; or the failure |

| Column | Meaning |
|---|---|
| `input` | The program's text. Escapes `\n` `\r` `\t` `\\` are decoded. |
| `expected` | A JSON string (the canonical text, the report, or the output bytes), or `ERROR:<code>` / `ERROR:<code>@<row>:<col>` for a program or run that must fail. Not escape-decoded: JSON's own escapes apply, so `"a\r\n"` ends in CRLF. |
| `doc` | `run.tsv` only. The JSON document the program runs over, escape-decoded like `input`; an empty cell is `null`. |
| `render` | `run.tsv` only. How a table or JSON-events result is rendered: empty (the host's default: CSV for a table, JSON for events), `csv` or `json`. A program that renders its own text takes none (`render_of_text`). |

A run is what `alchemy run` does: compile the program (which builds the
plan), then read `doc` with the tabnas JSON grammar incrementally, pruned
under the program's row selector when it has one, and push its events
through the program's sink, with the default `Limits`. A run needs
transduce's routers and render's renderers, which alchemy does not
depend on, so `run.tsv` is run by
[tabnas/alchemy-cli](https://github.com/tabnas/alchemy-cli), the
composition root, in every runtime (see "Who runs what"). Its Rust runner
runs every row twice, with the standard compositions native and through
the library's text (`with_native(false)`), and fails a row whose two
paths differ in a byte, a code or a position. A port that runs only the
library's text is held to the same bytes.

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
without a runner, and names `run.tsv` as run by alchemy-cli.

In [tabnas/alchemy-cli](https://github.com/tabnas/alchemy-cli), against
this repository's `test/spec/` in the sibling checkout: `run.tsv`, natively
and interpreted, and the full catalogue test over all four files.

- Rust: `rs/tests/spec_test.rs`.
- TypeScript: `ts/test/spec.test.ts`.
- Go: `go/e2e`'s `TestSpecRun`. Rows that read a document need
  `-tags tabnas_nodecell`, which alchemy-cli's gate passes.

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
  `run.tsv` row, alchemy-cli's gates (`../alchemy-cli/ci/rust/run.sh` and
  `../alchemy-cli/ci/polyglot/run.sh`, which downstream.yml runs).
