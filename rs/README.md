# tabnas-alchemy (Rust)

The `tabnas-alchemy` crate, library `tabnas_alchemy` and binary `alchemy`:
the alchemy language as a tabnas grammar plugin. What exists today is the
reader: the grammar plugin, the syntax tree with spans, the canonical and
layout printers, desugaring, and the `alchemy` command's `canon`, `format`
and `check`. The checker and the interpreter over `tabnas-transduce` and
`tabnas-render` follow. See the repository [README](../README.md),
[AGENTS.md](../AGENTS.md) and [`docs/language.md`](../docs/language.md).

The engine, the two crates the language lowers to, the fixture runner and
the test grammars are sibling checkouts named by path in `Cargo.toml`. From
this directory: `cargo test --all-targets`, `cargo test --doc`,
`cargo clippy --all-targets --all-features -- -D warnings`.
