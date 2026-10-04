# tabnas-alchemy (Rust)

The `tabnas-alchemy` crate, library `tabnas_alchemy`: the alchemy
language as a tabnas grammar plugin, with its checker, its planner and its
interpreter, and the types every tabnas transducer and renderer shares.

`tabnas_alchemy::shared` is those types: the event and table protocols,
sinks, `Fail` and its codes, limits, selectors, datums, the renderers'
options and text boundary, and the `Routers` and `Renderers` interfaces a
program is lowered through. It depends on nothing else in the fleet; with
`default-features = false` it is the whole crate, and that is how
`tabnas-transduce` (which implements `Routers`) and `tabnas-render` (which
implements `Renderers`) take it.

The language is the default `language` feature: the reader (`parse`,
`canonical`, `format`, `desugar`), the resolver, the checker (`check`,
`types`), the evaluator (`interp`, `value`) and its lowering to sinks
(`lower`), the effect summary (`effects`), the embedded standard library
(`stdlib`), and the API a host embeds (`compile`, `Program`). The host
passes the routers and the renderers in; the `alchemy` command, which
does, is [tabnas-alchemy-cli](https://github.com/tabnas/alchemy-cli). See
the repository [README](../README.md), [AGENTS.md](../AGENTS.md) and
[`docs/language.md`](../docs/language.md).

```rust
use std::sync::Arc;

use tabnas_alchemy::shared::{JsonEvent, Limits, Metrics, Sink};
use tabnas_alchemy::{compile, Output};

let program = compile(
    "def export [input] (json input)",
    "echo.alc",
    Arc::new(tabnas_transduce::routers()),
    Arc::new(tabnas_render::renderers()),
)?;
assert_eq!(program.output(), Output::Text);
let mut sink = program.sink(Box::new(Vec::new()), None, &Limits::default(), Metrics::new())?;
sink.event(JsonEvent::ArrayStart)?;
sink.event(JsonEvent::ArrayEnd)?;
sink.event(JsonEvent::End)?;
# Ok::<(), tabnas_alchemy::shared::Fail>(())
```

The host pushes the source's events into the sink and one `End`; the
output is written through a coalescing writer and flushed at `End`.
`Program::output` says what the program produces (its own text, table
rows the host renders as CSV or JSON, or JSON events), `row_selector`
under which selector the source may be read one row at a time, and
`explain` prints the plan report. `compile` builds the plan as well as
checking the program, under `MAX_PLAN_STEPS` and `MAX_EVAL_DEPTH`, on a
thread of `STACK_BYTES`; a host running programs it did not write gives
the sink a thread of that size, passes its abort flag to
`Program::with_abort`, and sets `max_output_bytes` (see Embedding in
[`docs/language.md`](../docs/language.md)).

The engine, the JSON grammar, the routers and renderers the tests pass
in, the fixture runner and the test grammars are sibling checkouts named
by path in `Cargo.toml`. From this directory:
`cargo test --all-targets`, `cargo test --doc`,
`cargo clippy --all-targets --all-features -- -D warnings`, and
`cargo build --no-default-features` for the shared types alone.
