# tabnas-alchemy

A small, typed, functional streaming language for the
[tabnas](https://github.com/tabnas/parser) engine, in which both
**transducers** (what a source contributes to an output model) and
**renderers** (how that model becomes text) are written. Programs are
layout S-expressions, parsed by a tabnas grammar plugin like every other
grammar in the fleet, and lowered onto the routers and renderers the host
passes in, which [tabnas-transduce](https://github.com/tabnas/transduce)
and [tabnas-render](https://github.com/tabnas/render) implement.

alchemy also holds the types those packages share: the event protocol
(events, sinks, tables, `Fail` and its codes, limits, selectors, datums)
and the `Routers` and `Renderers` interfaces a program is lowered through.
They sit in a unit of their own that depends on nothing, so transduce and
render build on them without taking the language.

```
def api-binding
  record
    entry :columns
      path "response" "metadata" "fields"
    entry :rows
      path "response" "payload" "deep" "records" each-index
    entry :column column-from-meta

def export [input]
  pipe input
    table-from-json api-binding
    csv csv-options
```

Run it over any format the tabnas parsers read with
[aless](https://github.com/rjrodger/aless):

```bash
aless --alchemy export.alc response.json        # CSV on standard output
aless --alchemy export.alc --explain response.json
```

Rust only for now. See [`docs/language.md`](docs/language.md) for the
language and [`AGENTS.md`](AGENTS.md) for how the repository is worked on.

## Layout

| Path | What it is |
|---|---|
| [`rs/`](rs/) | the `tabnas-alchemy` crate (library `tabnas_alchemy`); with `default-features = false` it is only the shared types, `tabnas_alchemy::shared` |
| [`go/`](go/), [`ts/`](ts/) | the Go module and the npm package; the shared types are `go/shared` and `@tabnas/alchemy/shared` |
| [`alchemy-grammar.jsonic`](alchemy-grammar.jsonic) | the grammar document, options and rules: the one source every runtime embeds (`make embed`) |
| [`test/spec/`](test/spec/) | shared fixtures, run by the fleet's fixture runner: `reader.tsv` (layout to canonical), `pipe.tsv` (desugared), `check.tsv` (the checker's codes and the plan reports) and `run.tsv` (a program over a document: the bytes it writes, or the failure); [`test/AGENTS.md`](test/AGENTS.md) describes them |
| [`docs/language.md`](docs/language.md) | the language reference; every example in it is a fixture row |
| [`stdlib/`](stdlib/) | the standard library's own definitions in alchemy (`table.alc`, `csv.alc`), embedded in the crate (as the copies in `rs/stdlib/`, written by `make embed`) and the reference the native paths are checked against |
| [`ci/rust/run.sh`](ci/rust/run.sh) | the gate CI runs |

The crate is the whole language: the reader (the grammar plugin, the
syntax tree with spans, the canonical and layout printers, desugaring),
the resolver and the checker (types, affine streams, protocols, strict
mode), the planner (`explain`), the interpreter, which lowers a plan
through the `Routers` and `Renderers` its host passes in and runs the
standard compositions natively, and the embedded standard library. The
differential test proves the native paths and the library's own text
produce the same bytes. The `alchemy` command, which composes alchemy,
transduce and render, is
[tabnas-alchemy-cli](https://github.com/tabnas/alchemy-cli).

## Build and test

Sibling checkouts (`../parser`, `../transduce`, `../render`, `../support`,
`../debug`, the grammars) are named by path in `rs/Cargo.toml`.

```bash
make build
make test
```

With the `alchemy` command from
[tabnas-alchemy-cli](https://github.com/tabnas/alchemy-cli):

```bash
alchemy canon export.alc            # fully parenthesized, one form per line
alchemy format export.alc           # layout form
alchemy check export.alc            # parses, desugars, resolves and checks; silent when it does
alchemy explain export.alc          # the plan report
alchemy run export.alc response.json   # the CSV, streamed to standard output
```

The program above names `column-from-meta` as the spec writes it
(`docs/language.md`, "Programs"); with that definition added it checks,
and `run` over the spec's document prints the spec's CSV bytes.

## License

MIT. Copyright (c) Richard Rodger.
