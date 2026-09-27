# tabnas-alchemy (Rust)

The `tabnas-alchemy` crate, library `tabnas_alchemy` and binary `alchemy`:
the alchemy language as a tabnas grammar plugin, with its checker, its
planner and its interpreter over `tabnas-transduce` and `tabnas-render`.
The reader (`parse`, `canonical`, `format`, `desugar`), the resolver, the
checker (`check`, `types`), the evaluator (`interp`, `value`) and its
lowering to sinks (`lower`), the effect summary (`effects`), the embedded
standard library (`stdlib`), and the API a host embeds (`compile`,
`Program`). See the repository [README](../README.md),
[AGENTS.md](../AGENTS.md) and [`docs/language.md`](../docs/language.md).

```rust
use tabnas_alchemy::{compile, Output};
use tabnas_transduce::{JsonEvent, Limits, Metrics, Sink};

let program = compile("def export [input] (json input)", "echo.alc")?;
assert_eq!(program.output(), Output::Text);
let mut sink = program.sink(Box::new(Vec::new()), None, &Limits::default(), Metrics::new())?;
sink.event(JsonEvent::ArrayStart)?;
sink.event(JsonEvent::ArrayEnd)?;
sink.event(JsonEvent::End)?;
# Ok::<(), tabnas_transduce::Fail>(())
```

The host pushes the source's events into the sink and one `End`; the
output is written through a coalescing writer and flushed at `End`.
`Program::output` says what the program produces (its own text, table
rows the host renders as CSV or JSON, or JSON events), `row_selector`
under which selector the source may be read one row at a time, and
`explain` prints the plan report.

The engine, the two crates the language lowers to, the fixture runner and
the test grammars are sibling checkouts named by path in `Cargo.toml`. From
this directory: `cargo test --all-targets`, `cargo test --doc`,
`cargo clippy --all-targets --all-features -- -D warnings`.
