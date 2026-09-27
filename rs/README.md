# tabnas-alchemy (Rust)

The `tabnas-alchemy` crate, library `tabnas_alchemy` and binary `alchemy`:
the alchemy language as a tabnas grammar plugin, with its checker and
interpreter over `tabnas-transduce` and `tabnas-render`. See the repository
[README](../README.md), [AGENTS.md](../AGENTS.md) and
[`docs/language.md`](../docs/language.md).

The engine, the two crates the language lowers to, the fixture runner and
the test grammars are sibling checkouts named by path in `Cargo.toml`. From
this directory: `cargo test --all-targets`, `cargo test --doc`,
`cargo clippy --all-targets --all-features -- -D warnings`.
