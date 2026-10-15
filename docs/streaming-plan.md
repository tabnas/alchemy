# Any format to any other, streamed: the plan

A plan, written on 2026-10-12 from the maintainer's ruling of the same
day, for making every conversion in the tabnas fleet stream through one
general path, with no format-specific streaming code anywhere. alchemy
and its test cases come first; the other repositories follow in the
order section 11 gives. Nothing here is implemented yet.

On 2026-10-15 the maintainer confirmed section 1 as the ruling and took
decisions 1 and 2 of section 10: a close pass carries its node's value
when that value is a scalar, and `LinesSource` stays until the engine
can read its input in chunks. The sections below are written to both.

## 1. The ruling

- **Custom streaming is not permitted.** A reader, a chunker or an event
  adapter written for one format is out, wherever it lives.
- **alchemy, transduce and render are the general solution.** An alchemy
  program defines a transducer that turns the input into a defined event
  stream, and a render turns that stream into the output.
- **The common event stream is the key.** It is the output of every
  transducer and the input of every render, so any format whose package
  ships both parts converts to and from any other.
- **The event stream is constructed by listening to parser events**, the
  way tabnas/debug's trace does: the engine's subscribers, nothing else.
- **Existing packages ship their parts as alchemy snippets**, a transducer
  beside the render they ship today. No custom code is needed.
- **A custom ABNF grammar defines its transducer deterministically**: the
  annotations that build its value also generate its alchemy snippet.

## 2. What exists, and what the ruling removes

Today a document reaches a render by one of three routes, chosen by the
host from the format's name (`transduce/rs/src/source/mod.rs`):

| Route | Code | Knows a format by |
|---|---|---|
| a record at a time from a reader | `LinesSource` in `source/lines.rs` | its syntax: JSON Lines' line breaks, CSV's quoting, header and separators |
| events as the parse proceeds | the rule-event adapter in `source/rule_events.rs`, gated by `capability::INCREMENTAL` | a list of ten verified names (`pgn` joined in transduce 0.2.8); the adapter itself infers events by watching the value cells the rules build |
| the parsed value walked | `ValueSource`, `SourceMode::Materialize` | nothing; retains the whole value |

The first is custom streaming and goes. The second is general in its
code but not in its contract: it infers events from the imperative
value, so it works only where a differential suite has shown the
inference sound, and four grammars (toml, ini, csv, feed) can never
pass it. The ruling replaces inference with declaration: each format's
package says, in alchemy, how its parser events become the common
stream. The third stays as what a format with no transducer part gets,
and as the oracle every transducer is verified against.

What the ruling removes, by repository:

- transduce: `capability::INCREMENTAL` as a gate; the inference in
  `rule_events.rs`; and, once the engine reads its input in chunks
  (decision 2), `LinesSource` and `LineFormat`. What stays general and
  is kept: `Guarded`, `Prune` (the engine's tree is still built while
  events leave, and pruning the streamed elements is format-neutral),
  the router, the table transducer, `scan-emit`, the limits and metrics.
- aless: `Plan::Incremental`, `Plan::Materialize` and every
  `capability::incremental` call in `src/export.rs`; with the line
  source, `Plan::Lines`, `line_format()` and the `--max-size`
  exemptions that exist only for it.
- alchemy-cli: the same reads in `rs/src/translate.rs`, on the same
  timing.

The line source is the one exception to the ruling, and it is kept
for a time. It holds JSON Lines at 11 MiB and CSV at 46 MiB whatever
the file's size (transduce's `docs/BENCH.md`, "Through aless"). The
general path parses a whole text, because the engine does
(`Tabnas::parse(&str)`), so on that path those two formats would cost
what every other format costs: the parse's working set, about 300 to
600 MiB for a 22 MB document with the rule history bound at 3. By
decision 2 the line source stays until the engine can take its input
from a reader (section 9; step 10 of section 11), and goes only then
(step 12), so JSON Lines and CSV never regress. Until then their
transducer parts are written and verified like every other format's,
and hosts keep reading those two formats through the line source.

## 3. The common event stream

`JsonEvents/1` stays exactly as it is, in alchemy's shared types
(`rs/src/shared/event.rs`): `ObjectStart`, `ObjectEnd`, `ArrayStart`,
`ArrayEnd`, `Key`, `Null`, `Bool`, `Number` with its lexeme, `String`,
and one `End` after the root value, once the source has validated. A
render that writes from a tree takes each key once per object, held by
`TreeContract` in front of it. A format whose documents are not a plain
tree declares a `schema` and an `embed`, as XML, feed, expr, semver, CSS,
proto and PGN do today. None of that changes, and no render changes: a
render part reads the common stream wherever it came from.

## 4. Parser events: the new source protocol

The engine offers four subscriptions (`parser/rs/src/lib.rs`):
`subscribe_tokens`, `subscribe_lex`, `subscribe_rules` and
`subscribe_rule_done`. debug's trace (`debug/rs/src/trace.rs`) builds
its `lex`, `rule`, `stack`, `parse` and `node` lines from the last three
and nothing else, and that is the model: a transducer sees what the
trace sees.

**`ParserEvents/1`** is one item per rule pass, from `subscribe_rule_done`,
which the engine fires after a pass's actions have run, for an open pass
and a close pass alike. Each item is a tagged value a `match` takes:

```text
(open rule)    the rule's open pass completed
(close rule)   the rule's close pass completed
```

where `rule` is a record:

| Field | From | What it is |
|---|---|---|
| `:name` | `rule.name` | the rule's name (`map`, `pair`, `val`) |
| `:d`, `:i` | `rule.d`, `rule.i` | the rule's depth and instance |
| `:alt` | `done.alt` | the alternate that matched: `:p` (the rule pushed), `:r` (the rule replaced with), `:b` (tokens given back), `:g` (its groups); `missing` when the state declared none |
| `:tokens` | `rule.o` or `rule.c` | the tokens the pass consumed, each a record of `:name`, `:val`, `:src`, `:row`, `:col` |
| `:forced` | `done.forced` | a close synthesized by recovery |
| `:n` | `rule.n` | the named counters |
| `:value` | `rule.node` | the node's value when it is a scalar at a close pass; `missing` otherwise (decision 1, taken 2026-10-15) |

The end of the stream is not an item: as `events` does, the source
delivers everything the parse produced and the program's sink receives
`End` only after the engine returned `Ok`, so a document is complete
only when it validated. Lexer events (`subscribe_lex`: every token,
trivia included) are not in the first version; a part that needs them
(comments, blank lines) is the case that adds a `(token t)` item later
(decision 5).

Containers never travel in parser events. A rule's node is handed on
only as a scalar, so a transducer that wants a container has to emit its
start and end from the rule passes that open and close it. That is what
makes the stream general and streamed: nothing in it grows with the
document except what the part chooses to retain, which `explain`
reports.

Not every pass is final. A grammar can rewind the tokens it has
consumed (`Context::rewind`): tabnas-bnf compiles a choice its
lookahead cannot settle into a probe, which marks the position as the
rule opens (`ProbeInit`), parses ahead, and rewinds to the mark to take
the branch it found (`ProbeDecide`), and the engine has the same probe
built in for a serialized grammar's `pd_` keys. The passes made between
a mark and its rewind are the probe's, not the document's, and
`subscribe_rule_done` reports them as it reports any other. A part must
never take them as committed, or a parse that validates could stream
events its value contradicts. Telling them apart needs the engine: it
reports where a probe starts and where it rewinds, so the source holds
back the passes in between and drops them, or it marks each pass made
while a probe is open, so the source withholds those. That is decision
6; it blocks step 2, and every custom grammar that probes (section 8)
depends on it.

The source that produces this stream is transduce's: a
`ParserEventSource` over one text and a `Tabnas`, installing one
`rule_done` subscriber that relays each pass as an item, on the thread
that parses, through `Guarded` for the abort flag and the metrics,
handing the sink back when the parse returns, as `ParserSource::run_owned`
does today. It knows no grammar. It holds the source's limits as it
builds each item, before the program sees it: a token's `:val` or
`:src`, or a `:value`, longer than `max_scalar_bytes` is refused with
that limit's name, as a JSON-events source refuses a scalar, so one
huge token in a document reaches no program's step. `max_depth` and
`max_key_bytes` hold where the part's outputs become `JsonEvents`, as
on the events any program hands to a taker of them. The fixture
encoding of a parser event, for transduce's `test/spec/`, follows the
events' encoding in `docs/reference.md`: an array whose first element
names it,
`["open", {"rule": "map", "d": 1, "i": 3, "alt": {"p": "pair", "r": "", "b": 0, "g": ["map", "json", "pair"]}, "tokens": [["#OB", "{", "{", 1, 1]], "forced": false, "n": {"pk": 0}}]`.
A field that is `missing` is left out of the item, so `"value": null`
is always a null scalar; the open pass above has no value.

## 5. alchemy: the language changes

This is the first phase and the one the maintainer asked to focus on.
Everything below is alchemy's own, compiles with the inert stages its
tests use, and is pinned by fixture rows; running it is alchemy-cli's,
as `run.tsv` is (section 6). It lands in Rust, TypeScript and Go at
once, since each runtime runs every row of `check.tsv`; the paths below
are Rust's, and the ports follow their own layout.

### 5.1 Types and protocols

- `Type::ParserEvent`, and `Type::parser_events()` as
  `Stream<ParserEvent>`, beside `Type::Event` and `Type::events()` in
  `rs/src/types.rs`.
- A protocol `ParserEvents/1` at the head of a chain, so a transducer's
  report reads `ParserEvents/1 → JsonEvents/1 → Text`.
- **The input's protocol is the host's to declare.** Today `export
  [input]` takes `JsonEvents`. A host that runs a program over a parser
  compiles it with `Program::with_input(Protocol::ParserEvents)` (and
  `compile_sources` takes the same option); the default stays
  `JsonEvents`, so every existing program and fixture compiles
  unchanged. The checker then refuses `events input` on a parser-events
  input and `parser-events input` on a JSON-events input, both
  `protocol_mismatch`, with the message naming which the host declared.

### 5.2 Natives

- `parser-events input -> Stream<ParserEvent>`: every rule pass as one
  item, as it arrives; nothing retained between items. Its registry
  entry, checker arm, effect and lowering are `events`'s, item type
  aside (`rs/src/stdlib/registry.rs`, `rs/src/check.rs`,
  `rs/src/effects.rs`, `rs/src/lower.rs`).
- `open` and `close`: constructors of one field, matched as `(open r)`
  and `(close r)`, as `key` and `scalar` are.
- Nothing else is new in the language. A transducer is a `scan-emit`
  over `parser-events`, its state the stack of open containers (one
  marker each, as YAML's render keeps), its outputs the existing event
  constructors (`object-start`, `(key name)`, `(scalar value)`, …), and
  `as-events` makes those outputs `JsonEvents` for whatever follows,
  through the `TaggedToJson` adapter alchemy already lowers.

### 5.3 The standard library: `stdlib/transduce.alc`

A library of definitions, embedded under `rs/stdlib/` and held to the
root file by `shared_sources_test.rs`, so that most formats write no
state machine at all:

- `transduce-tree spec input -> JsonEvents`: the declarative transducer
  for a grammar whose rules map onto a tree. `spec` is a record naming,
  by rule name, which passes open and close an object or an array, which
  pass announces a key (and from which token or constant), and which
  pass yields a scalar (from `:value`, or from a token's `:val` with its
  `:src` as the lexeme). The json grammar is the first user:

  ```text
  def json-transduce [input]
    transduce-tree
      record
        entry :object ["map"]
        entry :array ["list"]
        entry :key (record (entry :rule "pair") (entry :token 0))
        entry :scalar ["val"]
      input
  ```

  Its step keeps one marker per open container, so its retention is the
  document's nesting and never its width or length.
- `transduce-step`, `transduce-finish` and the small helpers the step
  uses, so a format whose grammar does not fit the declarative form
  (YAML, XML, Markdown, CSV) writes its own `scan-emit` over the same
  pieces, as the render parts write theirs over `events`.

The differential test in alchemy-cli (section 6) holds every part,
declarative or hand-written, to the walk of the grammar's own value.

### 5.4 The checker and `explain`

- Strict mode: `scan-emit` over `parser-events` is streaming with a
  bounded state, as over `events`; the parser-events stream is affine
  (`reused` when consumed twice); `as-events` of the step's outputs is
  `JsonEvents`.
- `explain`: `Protocol:` begins with `ParserEvents/1`; `Selection:`
  reads `none; every rule pass is delivered as an item`; `Retained
  state:` is the step's, capped at `max_metadata_bytes` and no deeper
  than `max_depth`; the guarantee is conditional, as for every
  `scan-emit`, and the report says the transducer is the program's own.

### 5.5 Translation parts: `transduce` beside `render`

In `rs/src/translate.rs`:

- `Descriptor` gains `transduce: Option<PartText>` and `Part` gains
  `transduce: Option<Alc>`, read from the manifest's
  `translate.transduce` (`"alchemy/transduce.alc"`, entry
  `<format>-transduce`), under the same rules as `lift` and `embed`:
  the path ends in `.alc`, the package hands the text over, the
  entry is explicit.
- `compose` and `compose_program` prepend the source's transducer when
  the host declares a parser-events input: `expr` starts as
  `(json-transduce input)`, then the lift, the adapters, the render, as
  today. A source part with no transducer is a format read whole: the
  host walks the grammar's value and declares a JSON-events input, and
  the loss note's `whole` sentence says why, as TOML's and INI's do
  today. A program's output (`compose_program`) is unchanged: the user's
  program takes the transduced events as it takes a source's.
- The tree contract holds a tree's render to its events whatever
  produced them, but with a parser-events input it cannot stand in
  front, where `Front::Tree` puts it today: the host's events are then
  parser events, and `TreeContract` takes `JsonEvents`. The composition
  puts it after the source's transducer instead, on the `JsonEvents`
  its `as-events` makes and before the lift (the lowering wraps
  `TaggedToJson`'s output in it), and `Front` is `None` for that input.
  `Front::Tree` stays for a JSON-events input.

### 5.6 The test cases

The fixtures are the contract every runtime is held to
(`test/AGENTS.md`), so the plan is written as rows first.

- **`test/spec/check.tsv`** (alchemy runs it, inert stages): a transducer
  program's `explain` report, protocol head and all; `protocol_mismatch`
  for `events input` under a parser-events input and for `parser-events
  input` under the default; `reused` for a parser-events stream consumed
  twice; `type_mismatch` for a step not answering a `transition`;
  `bad_pattern` for a malformed `(open …)` pattern. Each code with its
  position, `ERROR:<code>@row:col`, where it has one. The `check.tsv`
  runner needs the input protocol as a column, so the file gains an
  optional `input` column (empty is `JsonEvents`), and each runtime's
  runner reads it (Rust's through `tabnas_support::Runner`).
- **`test/spec/transduce.tsv`**, a new file, run by alchemy-cli as
  `run.tsv` is: columns `input` (the program), `expected` (the bytes, or
  `ERROR:<code>`), `grammar` (the parser whose events the program reads:
  `json` first, then the others alchemy-cli depends on), `doc` (the
  document, in that grammar), `render`. Its first rows: `json-transduce`
  into `json` over `{"a":[1,2.50,"x",{"b":null}],"c":{},"d":[],"e":1e21,"f":true}`,
  the bytes the walk gives with the lexemes kept (`2.50`, `1e21`); an
  empty object, an empty array, a root scalar, a document the grammar
  refuses (`INPUT_INVALID` at its position, after a protocol-valid
  prefix); a repeated member streamed twice, so that `TreeContract`
  refuses it (`DUPLICATE_MEMBER`) and a `LastWins` router keeps the
  last; and a program that writes a transducer's events as CSV through
  the inferred table.
- **`test/spec/parser-events.tsv`**, in transduce, not here: a text, a
  grammar and the parser-event stream in the encoding of section 4,
  so the ports agree on what the engine's passes look like before any
  program reads them. alchemy's fixtures assume it.
- **The differential rows are a runner's rule, not rows**: alchemy-cli's
  runner of `transduce.tsv` also compares, for every row whose document
  the grammar accepts and repeats no member, the transducer's
  `JsonEvents` with `ValueSource` over the grammar's parsed value,
  lexemes aside, and fails on any difference. The other rows have no
  such oracle and are held by their `expected` column alone: a document
  the grammar refuses has no value to walk, and its part must fail with
  the declared code before `End`; a repeated member is streamed twice
  where the value keeps one, and its row pins what `TreeContract` and
  the `LastWins` router make of it. That is the
  verification `capability::INCREMENTAL` used to stand for, moved from
  a list of names to every part on every fixture.
- **`docs/language.md`**: a section "Parser events and transducer
  parts", with `parser-events`, `open`, `close`, `transduce-tree` and
  the input protocol; every example in it a row in `test/spec/`, as the
  repository requires, and the catalogue test
  (`the_raised_messages_match_the_document`) extended with the new
  sentences in `alchemy-grammar.jsonic`.
- **alchemy's own tests**: `rs/tests/events_test.rs` gets the typing and
  affine tests for `parser-events`; `rs/tests/translation_parts_test.rs`
  composes a transducer part (compile only); `rs/tests/stdlib_test.rs`
  loads `transduce.alc`; `rs/tests/sources_test.rs` links a transducer
  part beside a render part and a program; `rs/tests/shared_sources_test.rs`
  holds the embedded copy.

### 5.7 Shared types

`rs/src/shared/` gains `parser_event.rs`: `ParserEvent`, its `Rule`,
`Alt` and `Token` records, the `ParserSink` trait a source pushes into,
and the encoding helpers the fixtures use. transduce implements the
source over them and alchemy's lowering consumes them, as `JsonEvent`
and `Sink` are shared today; neither crate depends on the other.

## 6. alchemy-cli: running the parts

- `rs/tests/spec_test.rs` runs `transduce.tsv`, natively and
  interpreted, as it runs `run.tsv`, and the differential comparison
  above for every row; `go/e2e` and `ts/test` follow when those
  runtimes run alchemy.
- `rs/tests/translation_parts_test.rs` asserts the `transduce` part of
  every package that declares one: present when declared, absent when
  not, its entry named, compiling in a composition.
- `translate` reads every format through `ParserEventSource` and the
  format's transducer part, and through the walk where a format ships
  none; `Reader::Parser`, `Reader::Checked` and `Reader::Tree` stay as
  the ways a parser is made and a document checked, and lose their mode
  logic.
- `alchemy run` keeps reading a JSON document and gains `--grammar NAME`
  to read the input with another package's parser, so a transducer part
  can be run from the command line against its own fixtures.

## 7. The formats: each package ships `alchemy/transduce.alc`

Each package adds a `transduce` entry to its manifest's `translate`
object, the file beside its render, the embedded copy under
`rs/translate/` with the drift test, and the structural descriptor's
new field in all three runtimes. admin's descriptor task keeps and
checks the key, as it does `lift` and `embed`. The order, by what each
one proves:

1. **json**: the proof that the declarative form reproduces the walk
   and the lexemes; the first rows of `transduce.tsv`.
2. **jsonl, jsonc, json5, zon**: the same shape; jsonl's part emits the
   root array's start before the first line and its end at the end.
3. **csv**: the first imperative grammar, and one of the two the line
   source serves until step 12 (decision 2). Its part keeps the
   header's labels in its state (under `max_metadata_bytes`) and emits
   one object per record, which the inference could never do. Its
   events are compared with the walk as every part's are.
4. **yaml, xml, markdown, jsonic**: hand-written `scan-emit`s where the
   declarative form does not fit; each refusal the adapter documents
   today (a stream of several documents, a merge key, an implicit list)
   becomes an explicit `fail :protocol-order` in the part, or a shape
   the part follows where it can.
5. **toml, ini**: parts that build the tree in their state and emit it
   at `finish`, since a later header can add to an earlier table; the
   manifest's `whole` sentence stays, and `explain` reports the
   retention. These may need a value operator that inserts under a
   path (`put` copies a record, and a state that grows per item is
   quadratic: a render that kept each mapping's keys took 10.9 s for
   16,000 keys, as the session handover of 2026-09-29, since removed
   in #62, recorded); measured first.
6. **feed, css, proto, pgn, expr, semver**: schema trees; a part each, or
   `whole` declared where the package's API builds the tree after the
   parse (expr's simplified form, proto's descriptor). pgn's part
   keeps the game-at-a-time stream it has had through the inference
   since transduce 0.2.8 and tabnas-chess 0.1.17.

A format is streamed the day its part lands, and read whole until then,
so the order is also the rollout and nothing breaks in between.

## 8. Custom grammars: the part is generated

tabnas-bnf is notation-neutral and already attaches the value actions
to an annotated rule (`bnf/src/emit.rs`: `@object$` or `@array$` on the
head link's open pass, `@key$` with the member's literal name on each
member link's open, `@setval$` or `@push$` with `src: true` on the close
of a member that does not build its own value). The same walk over the
same annotations writes the alchemy part:

- the head link's open pass emits `object-start` or `array-start`;
- a member link's open pass emits `(key "name")`;
- a member's close pass emits `(scalar src)` when the member is not
  nested, and nothing when it is, since the nested rule emits its own
  container;
- the rule's final close emits the container's end;
- a grammar with no annotations emits the compiler's `{rule, src, kids}`
  tree, one object per rule pass, so that the viewer's value and the
  stream agree.

`tabnas_bnf::transduce_alc(&spec) -> String` writes it;
`tabnas_abnf::abnf_transduce(src, opts)` exposes it; and the test is the
same differential suite over aless's grammar library and bnf's
fixtures: the generated part's events equal the walk of the value the
compiled grammar builds. aless registers the generated part for every
`--grammar` and `--grammar-expr` format, and a custom format streams
through the one path like any other. A render part for a custom format
(so that a custom format can be a `--render` target) is not in this
plan; it would be the same generator writing the reverse.

## 9. aless, and the engine

**aless** (after sections 5 to 8): one plan, `parser → ParserEventSource
→ composed program`, for every format but the two the line source
serves. `export::plan` shrinks to that one choice, and goes, with
`Plan` and `line_format`, when the line source does (step 12);
`translate::crates()` reads the `transduce` part; the whole-value route
stays for a format without a part, chosen by the part's absence and not
by a name. The fallback when a part refuses a document part way stays
as it is: once to the walk when nothing was written, else
`output: "partial"`. The agent tests that pin streaming behaviour
(`whole_records_reach_standard_output_before_the_error`,
`a_large_xml_document_streams_as_the_json_familys_do`) are the acceptance
tests of the switch, and `tests/translate_matrix.rs` runs unchanged in
intent over the new source.

**The engine** (in parallel, the maintainer's order across the three
runtimes): the general replacement for the line source, which stays
until it lands (decision 2).
transduce's `docs/architecture.md`, section 6, names the candidates: a
chunked source for the lexer, "the only route to bounded memory on one
huge JSON document"; a lighter `rule_done` notification without the
per-pass `Rule` clone; and the per-rule cost. The first is what lets
the line source go without a regression; the second pays for a stream
that now carries every pass to a program; the third is throughput.
Each is
measured before it is designed, as `doc/per-rule-cost.md` and
`doc/rule-history-bound.md` were.

## 10. Measurements, acceptance, and the decisions waiting

Acceptance, per part and for the whole:

1. Every transducer part's events equal the walk's on every fixture of
   its format, lexemes aside, or fail with a declared code before `End`;
   never a completed stream the walk contradicts (the suite's rule
   today, kept).
2. Retention is flat across ten times the rows for every streaming
   part (`transduce/rs/tests/retention_test.rs`), and a part that
   buffers says so in `explain` and in `whole`.
3. Throughput is measured before the switch: a parser pass is more
   frequent than a JSON event (five passes for one JSON member is
   ordinary), the interpreter costs about 8 µs an event (the YAML render,
   transduce's `docs/translation.md`, "What the pilot found"), and the
   engine parses at about 1 MB/s. A declarative `transduce-tree` run
   natively in transduce, as `csv` runs natively when its options match,
   held equal by the differential test, is the fast path if the
   interpreted part costs more than the maintainer accepts; the snippet
   stays the specification.
4. Memory: recorded in `docs/BENCH.md` before and after, JSON Lines and
   CSV included, so that step 12 shows those two formats holding their
   flat memory on the general path before the line source goes.

Decisions for the maintainer, in the order the work meets them. The
first two were taken on 2026-10-15; the others wait.

1. **`:value` on a close pass** (section 4): include the node's scalar
   value, so a part takes the grammar's own scalar resolution (YAML's
   plain scalars, TOML's dates, CSV's typed cells) rather than redo it
   from the token; or tokens only. Recommended: include it, scalars
   only. **Decided 2026-10-15: scalars only.** A close pass carries its
   node's value when that value is a scalar; containers never travel.
2. **When `LinesSource` goes**: with section 5's phase 2, as the ruling
   reads, accepting the memory regression for JSON Lines and CSV until
   the engine's chunked source lands; or at that landing. Recommended:
   with phase 2, recorded in BENCH.md, since the ruling is about where
   format knowledge lives and the regression is temporary. **Decided
   2026-10-15: when the chunked source lands.** The engine work that
   would end the regression is not designed yet, so the regression
   would have no end date; the line source stays until step 10 lands
   and goes in step 12.
3. **The native fast path** for `transduce-tree`: permitted under the
   existing rule (a native stands in for a snippet only when a
   differential test holds the two equal), or interpreted only.
4. **Where the generator lives**: tabnas-bnf, notation-neutral, with
   abnf (and ebnf, gbnf) exposing it; or each front end.
5. **Lexer events in `ParserEvents/1`**: rule passes only in the first
   version, or `(token t)` items from `subscribe_lex` as well, for parts
   that keep comments.
6. **Rewound passes** (section 4): how the source keeps a probe's
   passes from a part. The engine reports where a probe starts and
   where it rewinds, and the source holds the passes in between until
   the probe decides, then drops them; or the engine marks each pass
   made while a probe is open, and the source withholds those. Either
   is an engine change in all three runtimes, and it blocks step 2.

## 11. Order of work

| Step | Repository | Change | Depends on |
|---|---|---|---|
| 1 | alchemy | shared `parser_event.rs`; `Type::ParserEvent`; `parser-events`, `open`, `close`; input protocol; `explain`; `stdlib/transduce.alc`; `Part.transduce`; `check.tsv` rows; `docs/language.md`; in Rust, TypeScript and Go | decision 1 (taken) |
| 2 | transduce | `ParserEventSource`; `test/spec/parser-events.tsv`; a probe's passes held back (decision 6); `capability` and the inference retired; `LinesSource` kept (decision 2) | 1, decision 6 |
| 3 | alchemy-cli | `transduce.tsv` runner with the differential rule; parts test; `translate` and `run --grammar` on the new source | 1, 2 |
| 4 | json | the first `alchemy/transduce.alc`, manifest, embed, descriptor | 1, 3 |
| 5 | jsonl, jsonc, json5, zon, csv | parts, in that order | 4 |
| 6 | yaml, xml, markdown, jsonic | hand-written parts | 4 |
| 7 | toml, ini, then the schema formats | buffering parts, or `whole` | 4, a path-insert operator if measured necessary |
| 8 | bnf, abnf | the generated part and its differential test | 1, 3 |
| 9 | aless | one plan over `ParserEventSource` for every format but JSON Lines and CSV; the generated part for `--grammar` | 2, 4, 8 |
| 10 | parser | the chunked lexer source, the lighter notification, the per-rule cost | measured first; the maintainer's order |
| 11 | admin | the ADR: translation between formats is composed from per-format parts, and every format's events come from its own alchemy transducer over the engine's parser events; no host or transducer knows a format by name, the line source excepted until step 12 | 1 |
| 12 | transduce, alchemy-cli, aless | `LinesSource` and `LineFormat` retired; JSON Lines and CSV read through their parts; aless's `Plan::Lines` and `line_format` go, and with them `export::plan` | 5, 10 (decision 2) |

Each step is a pull request opened ready for review, with its fixture
rows and its measurements in the description; a step whose test fires
downstream by design lands with the downstream change that accepts both
states first, as the fleet's convention is.
