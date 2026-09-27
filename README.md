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
| [`stdlib/`](stdlib/) | the standard library's own definitions in alchemy |
| [`test/spec/`](test/spec/) | shared fixtures, run by the fleet's fixture runner |
| [`docs/language.md`](docs/language.md) | the language reference |
| [`ci/rust/run.sh`](ci/rust/run.sh) | the gate CI runs |

## Build and test

Sibling checkouts (`../parser`, `../transduce`, `../render`, `../support`,
`../debug`, the grammars) are named by path in `rs/Cargo.toml`.

```bash
make build
make test
```

## License

MIT. Copyright (c) Richard Rodger.
