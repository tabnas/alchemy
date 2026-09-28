# tabnas-alchemy

A small, typed, functional streaming language for the
[tabnas](https://github.com/tabnas/parser) engine, in which both
**transducers** (what a source contributes to an output model) and
**renderers** (how that model becomes text) are written. Programs are
layout S-expressions, parsed by a tabnas grammar plugin like every other
grammar in the fleet, and compiled onto
[tabnas-transduce](https://github.com/tabnas/transduce) and
[tabnas-render](https://github.com/tabnas/render).

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
| [`rs/`](rs/) | the `tabnas-alchemy` crate (library `tabnas_alchemy`, binary `alchemy`) |
| [`test/spec/`](test/spec/) | shared fixtures, run by the fleet's fixture runner: `reader.tsv` (layout to canonical), `pipe.tsv` (desugared) and `check.tsv` (the checker's codes and the plan reports) |
| [`docs/language.md`](docs/language.md) | the language reference; every example in it is a fixture row |
| [`stdlib/`](stdlib/) | the standard library's own definitions in alchemy (`table.alc`, `csv.alc`), embedded in the crate and the reference the native paths are checked against |
| [`ci/rust/run.sh`](ci/rust/run.sh) | the gate CI runs |

The crate is the whole language: the reader (the grammar plugin, the
syntax tree with spans, the canonical and layout printers, desugaring),
the resolver and the checker (types, affine streams, protocols, strict
mode), the planner (`explain`), the interpreter over `tabnas-transduce`
and `tabnas-render` with the standard compositions run natively, the
embedded standard library, and the `alchemy` command's `canon`,
`format`, `check`, `explain` and `run`. The differential test proves the
native paths and the library's own text produce the same bytes.

## Build and test

Sibling checkouts (`../parser`, `../transduce`, `../render`, `../support`,
`../debug`, the grammars) are named by path in `rs/Cargo.toml`.

```bash
make build
make test
```

```bash
rs/target/debug/alchemy canon export.alc            # fully parenthesized, one form per line
rs/target/debug/alchemy format export.alc           # layout form
rs/target/debug/alchemy check export.alc            # parses, desugars, resolves and checks; silent when it does
rs/target/debug/alchemy explain export.alc          # the plan report
rs/target/debug/alchemy run export.alc response.json   # the CSV, streamed to standard output
```

The program above names `column-from-meta` as the spec writes it
(`docs/language.md`, "Programs"); with that definition added it checks,
and `run` over the spec's document prints the spec's CSV bytes.

## License

MIT. Copyright (c) Richard Rodger.
