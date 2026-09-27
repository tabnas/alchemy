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
| [`test/spec/`](test/spec/) | shared fixtures, run by the fleet's fixture runner: `reader.tsv` (layout to canonical) and `pipe.tsv` (desugared) |
| [`docs/language.md`](docs/language.md) | the language reference; every example in it is a fixture row |
| `stdlib/` | *later*: the standard library's own definitions in alchemy, which arrive with the interpreter; nothing is tracked there yet |
| [`ci/rust/run.sh`](ci/rust/run.sh) | the gate CI runs |

What exists today is the reader: the grammar plugin, the syntax tree with
spans, the canonical and layout printers, desugaring, and the `alchemy`
command's `canon`, `format` and `check`. The checker, the planner
(`explain`) and the interpreter (`run`) follow.

## Build and test

Sibling checkouts (`../parser`, `../transduce`, `../render`, `../support`,
`../debug`, the grammars) are named by path in `rs/Cargo.toml`.

```bash
make build
make test
```

```bash
rs/target/debug/alchemy canon export.alc    # fully parenthesized, one form per line
rs/target/debug/alchemy format export.alc   # layout form
rs/target/debug/alchemy check export.alc    # parses and desugars; silent when it does
```

## License

MIT. Copyright (c) Richard Rodger.
