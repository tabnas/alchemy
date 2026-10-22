# Any format to any other, streamed: the plan

A plan, written on 2026-10-12 from the maintainer's ruling of the same
day, for making every conversion in the tabnas fleet stream through one
general path, with no format-specific streaming code anywhere. alchemy
and its test cases come first; the other repositories follow in the
order section 11 gives. Nothing here is implemented yet.

On 2026-10-15 the maintainer confirmed section 1 as the ruling and took
all six decisions of section 10: a close pass carries its node's value
when that value is a scalar; `LinesSource` stays until the engine can
read its input in chunks; a native `transduce-tree` may stand in for the
snippet where the differential test holds the two equal; the generator
of a custom grammar's part lives in tabnas-bnf; `ParserEvents/1` carries
rule passes only in its first version; and the engine flags each pass
made while a probe is open, which the source drops. The maintainer also
ruled on the rollout: each format keeps the route it has today until
its own part lands, and a format may still join transduce's incremental
list in the meantime (section 7). The sections below are written to all of it.

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

- transduce: `capability::INCREMENTAL` as a gate and the inference in
  `rule_events.rs`, once every format on the list has its part or
  declares `whole`, as markdown does (section 7); and, once the engine
  reads its input in chunks (decision 2), `LinesSource` and
  `LineFormat`. What stays general and is kept: `Guarded`, `Prune` (the
  engine's tree is still built while events leave, and pruning the
  streamed elements is format-neutral), the router, the table
  transducer, `scan-emit`, the limits and metrics.
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
| `:tokens` | `rule.o` or `rule.c` | the tokens the pass consumed, each a record of `:name`, `:val`, `:src`, `:row`, `:col`: those it matched, less the `b` its alternate gave back from their end (the next rule's to consume), and none when `b` is more than it recorded, since a close alternate can back up a token it did not record; a number token's `:val` carries its `:src` as the lexeme, and a token with no value (the end of the source) has `null` |
| `:forced` | `done.forced` | a close synthesized by recovery |
| `:n` | `rule.n` | the named counters |
| `:value` | `rule.node` | the node's value when it is a scalar at a close pass; `missing` otherwise (decision 1, taken 2026-10-15) |

The end of the stream is not an item: as `events` does, the source
delivers everything the parse produced and the program's sink receives
`End` only after the engine returned `Ok`, so a document is complete
only when it validated. Lexer events (`subscribe_lex`: every token,
trivia included) are not in the first version; a part that needs them
(comments, blank lines) is the case that adds a `(token t)` item later
(decision 5, taken 2026-10-15: rule passes only).

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
events its value contradicts. Telling them apart needs the engine,
and by decision 6 the engine flags each pass made while a probe is
open, so the source drops it before any part sees it. A probe always
rewinds, so no pass in one is ever the document's, and nothing needs
holding back. Step 2 depends on that engine change, in all three
runtimes, and so does every custom grammar that probes (section 8).

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
`["open", {"name": "map", "d": 1, "i": 3, "alt": {"p": "pair", "r": "", "b": 0, "g": ["map", "json", "pair"]}, "tokens": [["#OB", "{", "{", 1, 1]], "forced": false, "n": {"pk": 0}}]`.
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
  report reads `ParserEvents/1 → Stream<ParserEvent> → Stream<Value> →
  JsonEvents/1 → Text`.
- **The input's protocol is the host's to declare.** Today `export
  [input]` takes `JsonEvents`. A host that runs a program over a parser
  compiles it with `compile_with(src, file, Input::ParserEvents,
  routers, renderers)` (and `compile_sources_with`; TypeScript's
  `{ input: 'ParserEvents/1' }` option, Go's `CompileWith`), an
  addition beside `compile`, which keeps `JsonEvents`, so every existing
  host, program and fixture compiles unchanged. The checker then refuses
  `events input` on a parser-events input and `parser-events input` on a
  JSON-events input, both `protocol_mismatch`, with the message naming
  which the host declared. (Landed in tabnas/alchemy#66, with
  `Program::parser_sink`, the sink a host pushes the rule passes into.)

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
  for a grammar whose rules map onto a tree. `spec` is a record keyed by
  rule name (taken 2026-10-15: key the spec by rule), each entry a
  record of what that rule's passes do: `:container` (`:object` or
  `:array`), opened by its open pass and closed by its close pass;
  `:key`, the member name its open pass announces (a token's index, whose
  `:val` names it, or a constant string); `:scalar`, the value the rule
  yields (a token's index, that token of its open pass when the pass
  consumed one, its `:val` with the lexeme a number token carries; or
  `:value`, its close pass's own). The step looks each
  pass's rule up with `get` by its `:name`, one lookup per pass whatever
  the spec's size, and a rule the spec does not name does nothing. The
  json grammar is the first user:

  ```text
  def json-transduce [input]
    transduce-tree
      record
        entry :map (record (entry :container :object))
        entry :list (record (entry :container :array))
        entry :pair (record (entry :key 0))
        entry :val (record (entry :scalar 0))
      input
  ```

  The token indexes and which rule carries the key are the grammar's:
  for tabnas-json, `pair`'s open pass consumes the key string and the
  colon, and a scalar `val`'s open pass consumes its token, while a
  container `val` gives its opening token back to `map` or `list`, so
  `:scalar 0` yields nothing for it. The step keeps one marker per open
  container, so its retention is the document's nesting and never its
  width or length.
- `transduce-step`, `transduce-finish` and the small helpers the step
  uses, so a format whose grammar does not fit the declarative form
  (YAML, XML, CSV) writes its own `scan-emit` over the same pieces, as
  the render parts write theirs over `events`. Markdown, whose passes
  never carry the finished document, has no part and declares `whole`
  (section 7).

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
  `arity` for an `(open …)` pattern of more than one field. Each code with its
  position, `ERROR:<code>@row:col`, where it has one. The `check.tsv`
  runner needs the input protocol as a column, so the file gains an
  optional `protocol` column (empty is `JsonEvents`, `parser` the rule
  passes; `input` is already the program's column), and each runtime's
  runner reads it, alchemy-cli's catalogue test included (Rust's through
  `tabnas_support::Runner`). Landed in tabnas/alchemy#66 and
  tabnas/alchemy-cli#33.
- **`test/spec/transduce.tsv`**, a new file, run by alchemy-cli as
  `run.tsv` is: columns `input` (the program), `expected` (the bytes, or
  `ERROR:<code>`), `grammar` (the parser whose events the program reads:
  `json` first, then the others alchemy-cli depends on), `doc` (the
  document, in that grammar), `render`, `duplicates` (the policy for a
  member an object repeats within a captured value: empty, `first` or
  `last`). Its first rows: `json-transduce`
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
2. **jsonl, jsonc, json5**: the same shape; jsonl's part emits the
   root array's start before the first line and its end at the end.
3. **csv**: the first imperative grammar, and one of the two the line
   source serves until step 12 (decision 2). Its part keeps the
   header's labels in its state (under `max_metadata_bytes`) and emits
   one object per record, which the inference could never do: a `val`'s
   close yields the cell under the header's name at its position, or
   `field~` and the position past the header, an `elem` that opens
   without pushing a `val` or closes giving its token back on the comma
   group is an empty cell, and a record's close fills the header cells
   it lacks with the empty string, as the grammar's `field.empty` does.
   Its events are compared with the walk as every part's are.
4. **yaml, xml, jsonic, zon**: hand-written `scan-emit`s where
   the declarative form does not fit; each refusal the adapter documents
   today (a stream of several documents, a merge key, an implicit list)
   becomes an explicit `fail` in the part, or a shape the part follows
   where it can: jsonic's part follows an implicit list whose first
   element is a scalar (the val's close replaces it with `list`, so the
   array starts before the scalar) and leaves one whose first element is
   a container, streamed before the list is known, to the tree contract
   (a second root), as it leaves a repeated member, which `map.extend`
   merges (`DUPLICATE_MEMBER`); yaml's part refuses a merge key and an
   alias to a mapping or a sequence in its own words (`fail
   :unstreamable`, alchemy 0.2.10) and leaves a stream of several
   documents to the tree contract. zon's rules are json's, but an integer
   no double holds is the `$big` object in Rust and a bigint or `big.Int`
   in TypeScript and Go, which the declarative form cannot take from a
   token, and an enum literal under `enumTag` is wrapped only after the
   close; its part steps the library's `transduce-tree` over json's spec
   for the containers and the keys, and yields a scalar token's value
   itself: a big integer, the one-member record a host's parser presents
   it as (its Rust reader always, alchemy-cli's TypeScript and Go parsers
   by a lex hook), as that object; an enum literal is its name, since the
   wrapping reaches no pass. markdown has no part: its passes carry each
   line's text, the grammar builds the blocks and the inlines in its
   actions and sets the finished document as the root's node, which no
   pass carries (decision 1), so its manifest declares `whole`, as item 6
   allows where a package builds the tree after the parse.
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

A format switches to its part the day the part lands, and keeps the
route it has today until then (decided 2026-10-15): a format on
transduce's incremental list streams through the inference as it does
now, and any other is read whole, so the order is also the rollout and
nothing streams less in between. The list and its inference go when
every format on it has its part, or declares `whole` as markdown does
(step 13). A format may still join the
list before its part lands, as PGN did in transduce 0.2.8, and then
switches like the others.

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
- a grammar with no annotations is read whole: its value is the
  compiler's `{rule, src, kids}` tree, whose `src` holds the text of
  every node beneath it and comes before its `kids`, so no stream can
  write the root before the document has ended.

The part is the library's `transduce-tree` over a spec keyed by rule
name, which step 8's record in section 11 describes.
`tabnas_bnf::transduce_alc(&spec, name)` writes it as
`<name>-transduce`, or says why a host reads the grammar's documents
whole, as it does for a grammar with no annotations and for the
annotated shapes the spec cannot say (TypeScript's `transduceAlc` and
Go's `TransduceAlc` alike); `tabnas_abnf::abnf_transduce(src, name,
opts)` converts an ABNF grammar and writes it; and the test is a
differential suite over abnf's fixtures, in alchemy-cli, and aless's
grammar library, in aless: the generated part's events equal the walk of
the value the compiled grammar builds. aless runs the generated part for
every `--grammar` and `--grammar-expr` format the generator writes one
for, through the one path a format's part takes, and reads the others
whole, as it reads any format with no part. A render part for a custom
format (so that a custom format can be a `--render` target) is not in
this plan; it would be the same generator writing the reverse.

## 9. aless, and the engine

**aless** (after sections 5 to 8): one plan, `parser → ParserEventSource
→ composed program`, for every format whose package ships a part, but
the two the line source serves. A format with no part yet keeps the
route it has today: the incremental source where transduce lists it,
the walk of its value otherwise. `export::plan` shrinks as parts land,
and goes, with `Plan` and `line_format`, once the line source and the
incremental list have both gone (steps 12 and 13);
`translate::crates()` reads the `transduce` part; the whole-value route
stays for a format with no part and no place on the list. The fallback
when a part refuses a document part way stays as it is: once to the
walk when nothing was written, else `output: "partial"`. The agent
tests that pin streaming behaviour
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

   Measured on 2026-10-15, before decision 3, with aless 0.4.4 on the
   crates it pins (built with its `matrix` profile, release optimisation
   without fat LTO), on an idle four-core container, the median of
   three runs: 40,000 JSON records of eight members each, a nested
   object and a short array among them, 6.5 MB and 1,020,002 JSON
   events. The engine makes 2,240,004 rule passes over it, 5.6 a
   member and 2.2 an event. The bare parse (`rule.history` 3) takes
   4.57 s; an empty `rule_done` subscriber adds 29% (0.58 µs a pass,
   the engine's copy of the rule for the notification), and building
   an owned record of each pass as section 4 defines it adds 63% (1.29
   µs a pass). Through aless, `--render json` takes 5.80 s; a program
   that writes its input as JSON with no step of its own takes 6.86 s;
   a minimal interpreted `scan-emit` (one marker per open container,
   each event passed on) takes 9.56 s, 2.6 µs an event; and the YAML
   render part, a full hand-written `scan-emit`, takes 22.58 s, about
   15 µs an event. So an interpreted part over parser events costs at
   least 1.3 + 2.6 µs for each of 2.2 passes an event: about twice
   today's streaming time for the simplest step, and three to six
   times for a step as heavy as a render's. A native `transduce-tree`
   reads each pass in place, with no record built and no step
   interpreted, at about what today's inference costs. The interpreter
   was measured on today's events, which are smaller items than a
   parser event's record, so the interpreted figures are a floor.
4. Memory: recorded in `docs/BENCH.md` before and after, JSON Lines and
   CSV included, so that step 12 shows those two formats holding their
   flat memory on the general path before the line source goes.

Decisions for the maintainer, in the order the work meets them. All
six were taken on 2026-10-15.

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
   **Decided 2026-10-15: permitted.** The measurement under acceptance
   3 puts an interpreted part at two to six times today's streaming
   time, and a native one at about today's. The snippet stays the
   specification, and alchemy-cli's differential test holds the native
   `transduce-tree` to it on every fixture.
4. **Where the generator lives**: tabnas-bnf, notation-neutral, with
   abnf (and ebnf, gbnf) exposing it; or each front end. **Decided
   2026-10-15: tabnas-bnf.** One generator where the annotations are
   attached, which the three front ends expose.
5. **Lexer events in `ParserEvents/1`**: rule passes only in the first
   version, or `(token t)` items from `subscribe_lex` as well, for parts
   that keep comments. **Decided 2026-10-15: rule passes only.** A
   `(token t)` item is added when a part needs comments or blank lines.
6. **Rewound passes** (section 4): how the source keeps a probe's
   passes from a part. The engine reports where a probe starts and
   where it rewinds, and the source holds the passes in between until
   the probe decides, then drops them; or the engine marks each pass
   made while a probe is open, and the source withholds those. Either
   is an engine change in all three runtimes, and it blocks step 2.
   **Decided 2026-10-15: the engine flags each pass made while a probe
   is open, and the source drops it.** A probe always rewinds, so this
   needs no buffering and no new subscription.

## 11. Order of work

| Step | Repository | Change | Depends on |
|---|---|---|---|
| 1 | alchemy | shared `parser_event.rs`; `Type::ParserEvent`; `parser-events`, `open`, `close`; input protocol; `explain`; `stdlib/transduce.alc`; `Part.transduce`; `check.tsv` rows; `docs/language.md`; in Rust, TypeScript and Go | decision 1 (taken) |
| 2 | transduce | `ParserEventSource`; `test/spec/parser-events.tsv`; a probe's flagged passes dropped (decision 6); a native `transduce-tree`, held to the snippet (decision 3); `capability` and the inference kept until step 13; `LinesSource` kept (decision 2) | 1, the engine's probe flag (step 10) |
| 3 | alchemy-cli | `transduce.tsv` runner with the differential rule; parts test; `translate` and `run --grammar` on the new source | 1, 2 |
| 4 | json | the first `alchemy/transduce.alc`, manifest, embed, descriptor | 1, 3 |
| 5 | jsonl, jsonc, json5, csv | parts, in that order | 4 |
| 6 | yaml, xml, markdown, jsonic, zon | hand-written parts; markdown's `whole` | 4; for zon, a big integer token presented as the `$big` object, as alchemy-cli's parsers present it |
| 7 | toml, ini, then the schema formats | buffering parts, or `whole` | 4, a path-insert operator if measured necessary |
| 8 | bnf, abnf | the generated part and its differential test | 1, 3 |
| 9 | aless | one plan over `ParserEventSource` for every format with a part but JSON Lines and CSV, the current routes for the rest; the generated part for `--grammar` | 2, 4, 8 |
| 10 | parser | first the probe flag (decision 6), which step 2 needs; then the chunked lexer source, the lighter notification, the per-rule cost | the flag first; the rest measured first, in the maintainer's order |
| 11 | admin | the ADR: translation between formats is composed from per-format parts, and every format's events come from its own alchemy transducer over the engine's parser events; no host or transducer knows a format by name, the line source excepted until step 12 and the incremental list until step 13 | 1 |
| 12 | transduce, alchemy-cli, aless | `LinesSource` and `LineFormat` retired; JSON Lines and CSV read through their parts; aless's `Plan::Lines` and `line_format` go | 5, 10 (decision 2) |
| 13 | transduce, alchemy-cli, aless | `capability::INCREMENTAL` and the inference in `rule_events.rs` retired; aless's `export::plan` and `Plan` go once step 12 has landed too | every listed format's part (steps 4 to 7), or its `whole` |

Under way: step 10's probe flag is tabnas/parser#309. Step 1 is
tabnas/alchemy#66, in all three runtimes, with tabnas/alchemy-cli#33 for
its catalogue test: the language (the shared types, the types,
`parser-events`, `open` and `close`, the declared input, `explain`, the
`check.tsv` rows and `docs/language.md`); `stdlib/transduce.alc`, whose
`transduce-tree` each runtime's test holds, with the json spec of
section 5.3, to the events of tabnas-json's own rule passes; and the
translation over a transducer. Under the additive API decided for it,
section 5.5's `Descriptor.transduce` and `Part.transduce` became
`Part::transducer(descriptor, text)` beside `from_descriptor`, and
`compose_parsed` and `compose_program_parsed` beside `compose` and
`compose_program`. The tree contract is the native `tree-contract`, and
its refusal leads with `not_a_tree`.

Step 2 is tabnas/transduce#51, with tabnas/alchemy#67 and
tabnas/alchemy-cli#34 beside it. `ParserEventSource` relays the passes
in all three runtimes, and `test/spec/parser-events.tsv` pins them. The
native `transduce-tree` is transduce's `TransduceTree`, pinned by
`test/spec/tree.tsv`. alchemy's lowering takes it through an optional
`Routers::transduce_tree`, for the library's scan with a spec of the
standard shape (the shared `TreeSpec`), and runs the library's text
everywhere else. alchemy-cli's differential test holds the two to the
same bytes in each runtime. The source sorts an alternate's groups,
which the engines order differently. It relays two more differences
between the engines as they are, which transduce's `DIVERGENCE.md` 6 and
7 record: the first rule's instance number, and a fixed token's value.
Step 11 is tabnas/admin#152, ADR-28.

Step 3 is tabnas/alchemy#69 and tabnas/alchemy-cli#35: `transduce.tsv`
with the json transducer's rows and a `duplicates` column (empty, `first`
or `last`, which the runners hand to `with_duplicates`) beside `grammar`,
`doc` and `render`; alchemy-cli's runner of it in all three runtimes, held
to the walk of the grammar's value as section 5.6's rule says; `translate`
reading a format whose package ships a part through it, over the parser's
rule passes, with the walk standing in where the part's events are not
the value's (a repeated member) and for a path; and `run --grammar
FORMAT`, a program over a format's rule passes. Step 4 is tabnas/json#111,
released as tabnas-json 0.5.18, which alchemy-cli#35 reads. Step 5's
first group is tabnas/jsonl#53, tabnas/jsonc#91 and tabnas/json5#102,
released as jsonl 0.1.18, jsonc 0.5.15 and json5 0.5.17, which
tabnas/alchemy-cli#36 reads: json's spec for each, jsonl's with the
`jsonl` rule's passes opening and closing the root array, each row of
`transduce.tsv` for them held to the walk as json's are. Reading json5's
part found where a part's events and the walk part ways: json5's grammar
merges a repeated member's containers, and the composition's inferred
table kept the last, so `{a:{x:1},a:{y:2}}` into CSV lost `x`.
tabnas/alchemy#71, released as alchemy 0.2.9, makes a composition over a
part refuse a repeated member (`DUPLICATE_MEMBER`), as the tree contract
does, so a host reads the document whole; `LastWins` stays for a
document's events, where the incremental source's own net catches a
merge. ZON's part is the first of step 6, hand-written
(tabnas/zon#104, released as zon 0.5.18, which tabnas/alchemy-cli#37
reads; its rows are tabnas/alchemy#73): a big integer token's value is
the `$big` object in Rust and a bigint or `big.Int` in TypeScript and
Go, which json's spec fails (`DSL_TYPE_ERROR`, which no host reads the
document whole on) where the walk writes `{"$big": …}`, and no stream
form in transduce's sources was needed, since alchemy-cli's TypeScript
and Go parsers already present the token as the object by a lex hook,
as Rust's reader builds it; so the part steps the library's
`transduce-tree` over json's spec for the containers and the keys, and
yields a scalar token's value itself, the one-member record as that
object. An enum literal under `enumTag` is its name: the wrapping is
made on the rule's value after the close and reaches no pass. CSV's
part, step 5's last (tabnas/csv#104, released as csv 0.6.7, which
tabnas/alchemy-cli#38 reads; its rows are tabnas/alchemy#75), is
hand-written too, since the grammar builds its records in its actions,
which the passes do not show: the `csv` rule's passes open and close
the array, a `record`'s its object after the header, a `val`'s close
yields the cell under the header's name at its position, or `field~`
and the position past the header, an `elem` that opens without pushing
a `val` or closes giving its token back on the comma group is an empty
cell, and a record's close fills the header cells it lacks with the
empty string; an empty document, which makes no pass, is the empty
array at `finish`. A step's outputs are one vector and a record's
missing cells are as many pairs as the header has left, so the part is
two scans, the first yielding each cell as a pair and each container as
a keyword, the second writing each as its events. Every event is the
walk of the grammar's value under its default options, a header name
repeated aside, which the part streams twice and the tree contract
refuses, so a host reads the document whole; alchemy-cli, which read
CSV whole before, now streams it through the part, and the line source
keeps serving aless until step 12. Step 5 is complete.

jsonic's part, step 6's second (tabnas/jsonic#116, released as jsonic
0.7.10, which tabnas/alchemy-cli#39 reads; its rows are
tabnas/alchemy#77), is hand-written over json's rule shape, since the
relaxed forms are built in the grammar's actions, which the passes do
not show: a `map`'s passes open and close an object, a `list`'s an
array, a `pair`'s open on its key alternate announces its key (a
string's value, any other token's source) and one opened on a separator
with no key announces nothing, and a `val` yields its scalar at its
close, from the value the close carries, so a close whose alternate
replaces the val with `list`, the first element of an implicit list,
starts the array before it and the list's open starts no second one; an
undefined value is null in an object and nothing in a list, a separator
with no value a null element, a pair inside a list dropped, its passes
passed by until it closes, and the empty document null. Over the 996
inputs of jsonic's fixtures the part writes what the walk writes for
974, and the tree contract refuses the other 22: an implicit list
whose first element is a container, whose events stream before the
list is known (a second root), and a member name repeated, which the
grammar's `map.extend` merges. alchemy-cli's `translate` reads those
whole, since it commits an attempt's bytes only once the attempt
succeeds (`{a:1},{b:2}` comes out as `[{"a":1},{"b":2}]` through the
part); a host that has written already, as aless's export streams,
reports partial output instead, as the fallback rule above says, since
no pass tells the part the list is coming before its first element
has streamed. A part reads a document
under the grammar's default options, which every host that reads a
part builds the grammar with: the passes carry no options, and an
option that reshapes the value (jsonic's `list.pair`, whose pair
element fires the same alternate the default drops, `list.child`,
`info.map`, `lex.emptyResult`) is not in them, so a host that sets one
reads the document whole; a part's header says so, as csv's does for
its options.

YAML's part, step 6's third (tabnas/yaml#130, released as yaml 0.5.25,
which tabnas/alchemy-cli#40 reads; its rows are tabnas/alchemy#81), is
hand-written, since the grammar builds its block structure and the
relaxed forms it shares with jsonic in its actions, which the passes do
not show: a `map`'s passes open and close an object, a flow `list`'s an
array, the `val` or `indent` whose open pushes the block sequence's rule
an array its own close ends, and a sequence's mapping (`yamlElemMap`,
replaced by its pairs in one frame) an object its last pair's close
ends; a `pair`'s or `yamlElemPair`'s key comes from its key token (the
one after `?` on an explicit key), held until the value begins, a key
with no value in a flow mapping is a null member at once, and a `val`,
or an `indent` opened on a plain scalar, yields its scalar at its close,
so that adjacent scalars start the array before the first; an undefined
value is null in a mapping, at the root and as a block sequence's item,
and nothing in a flow sequence, a separator with no value a null
element, and directives and document markers alone null. What the
passes cannot show the part refuses in its own words, the first use of
`fail :unstreamable` (tabnas/alchemy#78, released as alchemy 0.2.10:
`STREAMABILITY_UNKNOWN` with the finer code `unstreamable`, on which a
host that reads a part reads the document whole, as it does on
`DUPLICATE_MEMBER`; the same change puts the tree contract after the
part in every composition, so a second root reaches the host as
`not_a_tree` rather than as the renderer's `PROTOCOL_ORDER_ERROR`): an
alias to a mapping or a sequence, whose value the token carries whole,
and a plain `<<` merge key, whose members the grammar folds in after
the passes; a stream of several documents is a second root to the tree
contract. Over the 568 inputs of yaml's fixtures the part writes what
the walk writes for 535 and refuses the other 33 (18 streams of several
documents, 13 merge keys, 2 aliases). Where the part met the ports, the
review and alchemy-cli's gates found them apart, and the same change
repairs each: a sequence entry's pair with no value (`- a:`) walked as
`{"a":null}` in TypeScript and Go and as `{}` in Rust, and is a null
member in all three now; with `map.ordered` on, the TypeScript grammar
recorded none of the members it stores itself in the engine's key-order
side channel, so a walk in source order put them last; and Go's lexer
handed an alias to a mapping or a sequence as the marker it hands an
alias to nothing, which the passes cannot tell apart, where TypeScript
hands the value's copy.

XML's part, step 6's fourth (tabnas/xml#100, released as xml 0.7.19,
which tabnas/alchemy-cli#41 reads; its rows are tabnas/alchemy#83),
is hand-written, since the grammar builds an element's members past its
start tag, and resolves its namespaces, in its actions, which the passes
do not show: an `element`'s open carries its start tag's token, the
element's name and its attributes in source order, and starts the
element's object; `head`'s open on it yields the attributes and starts
the children; a `child`'s open on a text yields the text; and the
element's close ends the children and yields the members after them,
resolved against the open elements' attributes as the grammar resolves
them: the prefix before the name's first colon, the namespace the
nearest declaration binds it to (for a name with no prefix the default
namespace, which `xmlns` declares, and `xmlns:` too, as the grammar
reads it, and which `xmlns=""` undeclares; `xml` is bound to the XML
namespace), the innermost `xml:space` that is not `default`, and the
innermost `xml:lang` that is not empty. A self-closing element, which
has no `head`, leaves its closing items to the close of the `child` or
`xml` rule around it. A step's outputs are one vector, so the part is
two scans, as csv's is. One thing the passes did not show the lexer now
gives them: a DOCTYPE's ATTLIST defaults were added in the element's
action, and the lexer now applies them to the start tag's token, in all
three ports. Over the 3,277 inputs of xml's fixtures with the default
options and of the W3C conformance suite, the part writes what the walk
writes for the 2,614 the grammar reads, and fails as the walk fails for
the other 663, whose refusal the engine reports after the passes before
it. Where the part met the ports, alchemy-cli's Go runner found them
apart, and the same change repairs Go: under `xmlns=""` Go wrote an
element's `namespace` as the empty string, where TypeScript and Rust
write none.

Markdown has no part, step 6's fifth (tabnas/markdown#96, released as
markdown 0.7.13, which tabnas/alchemy-cli#42 reports): its rule
passes are one `line` per source line, a replace loop whose token
carries the line's text, and the grammar builds the blocks in its
actions and the inlines once the last line is in, since a link or an
image may name a link reference definition that comes later in the
document. The finished document is the root rule's node, which the
passes carry only when it is a scalar (decision 1), and a part could
rebuild it only by running CommonMark itself, which alchemy has no
operators for (no substring and no arithmetic, and no definition
reaches itself). So markdown's manifest says why it is read whole, in a
`whole` sentence, as the package-built trees of section 7's item 6 do,
and alchemy-cli reports it. Nothing streams less: transduce's inference,
through which aless reads markdown until step 13, sends the document's
events only once its last line is in, since the grammar sets it whole
as the root's node in its last action. Markdown leaves the incremental
list with the list itself.

The generated part, step 8 (tabnas/bnf#117, released as bnf 0.1.31;
tabnas/abnf#126, released as abnf 0.4.25; its tests
tabnas/alchemy-cli#46), is alchemy's `transduce-tree` with a spec read
off the value actions the compiler attaches to an annotated rule, keyed
by rule name: a rule whose open alternate begins an object or an array
is a container, a member named by a constant is a key or, on the head of
an `@object` chain, a member, and a member that holds the text its
pushed rule matched is a text. Two fields the spec lacked come with it
(tabnas/alchemy#86, released as alchemy 0.2.11, with the natives in
tabnas/transduce#53, released as transduce 0.2.10): `:member`, a member
an open pass names after the container its rule begins, since the head
of an `@object` chain begins the object and names the member its first
link fills; and `:text`, a rule whose close pass yields first the source
text of the tokens the passes between its open and its close consumed,
the text `@setval$` and `@push$` read with `src`. The routers say
whether their native reads the two fields, and the lowering hands a spec
that names either only to routers that do; a native holds a text to
`max_scalar_bytes` at the joins the library's `string-join` makes, in
its order. `transduceAlc(spec, name)` writes `<name>-transduce`, the
same text in TypeScript, Go (`TransduceAlc`) and Rust (`transduce_alc`),
or `{whole}` with the reason a host reads the grammar's documents whole:
a name alchemy cannot give a definition, or a rule's; alternates of one
rule whose value actions differ, or a value action the spec cannot say;
a rule two containers would carry on; a container or text rule an open
alternate replaces; a value rule beneath a text member; and a start rule
with no annotation. The tree of one object per rule pass that the plan
first proposed for that last case is not what such a grammar builds: its
value is the `{rule, src, kids}` tree, whose `src` comes before its
`kids` and holds the text of every node beneath it, so no stream writes
the root before the document has ended, and the generator declares it
whole. abnf's `abnfTransduce(src, name, opts)` converts and writes it.
Over abnf's `alignment-abnf-ast.tsv`, the parts of the 23 annotated
grammars write the value each compiled grammar builds, natively and
interpreted, in all three runtimes, and the generator declares the other
49 read whole; a simulation over aless's grammar library before the
change agreed on the ten grammars there.

Step 9 began with the grammars from the command line
(rjrodger/aless#76): a `--grammar` or `--grammar-expr` grammar that step
8's generator writes a part for, as it does for every annotated grammar
of abnf's fixtures and of aless's library, is read through it, compiled
once with the grammar, and any other is read whole, for the reason the
generator gives. The text's rule passes go from `ParserEventSource` into
the part by `Program::parser_sink_into` (tabnas/alchemy#88, released as
alchemy 0.2.12, in all three runtimes: a program of JSON events over
parser events into a sink of the host's own, through the lowering's own
events, so the source's limits hold on the part's events and the native
`transduce-tree` runs where the routers offer it), then through a tree's
contract into the path's scope and the export's chain, as before.
aless's guard, its depth cap and its timeout, stays on the parser, and
the program's abort flag reaches the part's steps. A document the part
refuses is read whole once more when nothing was written, as an
incremental grammar's is, and so is one that nests past the transducer's
`max_depth` first, since the whole read holds a custom grammar's value
to the measure it always has. Over a 100,000-line hosts file the first 4
KB leave in 0.15 s instead of 6.37 s, and the whole run takes 1.5 to
1.65 times the whole read (10.46 s against 6.84 s into JSON), the cost
section 10 measured for recording each rule pass; the memory is the
same, since the engine builds its tree either way. The built-in formats
with parts keep their routes for now. Moving them is the rest of step 9,
and it waits on the maintainer's call on its cost, as PGN's part does:
over 10,000 records in alchemy-cli's release build, 1.2 times the whole
read for JSON, whose part is the native `transduce-tree`, and 4.8 times
for YAML and 3.9 times for XML, whose parts are interpreted; and
`ParserEventSource` drops nothing behind the stream, where the
incremental source prunes the exported array's elements, so the engine's
tree holds the whole document.

The work the maintainer put before both calls came next. On PGN's part
the call was to speed up the interpreter first, and on the built-ins to
prune first, then measure again. tabnas/transduce#56, released as
transduce 0.2.11, made `ParserEventSource` prune: once the sink has seen
a close pass, the source empties the array the closing rule's node is,
for the grammars `capability::prunable` lists, and a differential suite
keeps that list honest both ways. The interpreter work is three pull
requests, released as alchemy 0.2.13, none of which changes an output, a
failure or a limit:
- tabnas/alchemy#91 looks a free name up once per scope, builds a rule
  pass's record from a shape made once, hashes the evaluator's tables
  with FxHash and shares an atom's text: 25% fewer instructions on
  YAML's part.
- tabnas/alchemy#92 makes vectors persistent, so `push` and `pop` cost
  the same at any length, and measures a scan state from the sums its
  vector's leaves keep: a state that keeps every item of 40,000 takes
  0.47 s, against 42.94 s before.
- tabnas/alchemy#93 keeps a long string as a rope, so the library's
  `transduce-write` costs each pass its own tokens rather than the text
  so far (tabnas/alchemy#86's thread on it): a `:text` rule over 160,000
  items takes 8.01 s, against 40.40 s before.

Measured again with both, a part costs more time than aless's own read,
and no more memory. Over generated records in alchemy-cli's release
build, a part read pruned by `run --grammar`, with the engine's rule
history bounded to three predecessors (`options.rule.history`, as aless
sets it on every parser it makes), against aless 0.4.5's own read with
`--render json`:

| | aless 0.4.5 | the part, pruned |
|---|---|---|
| JSON, 10,000 records | 1.15 s, 32 MiB | 1.40 s, 32 MiB |
| JSON, 40,000 records | 4.60 s, 98 MiB | 5.86 s, 99 MiB |
| YAML, 10,000 records | 1.94 s, 53 MiB | 7.43 s, 33 MiB |
| YAML, 40,000 records | 7.83 s, 177 MiB | 29.14 s, 97 MiB |
| XML, 10,000 records | 1.14 s, 115 MiB | 5.26 s, 32 MiB |
| XML, 40,000 records | 4.51 s, 394 MiB | 20.36 s, 96 MiB |

PGN's part reads a 605 KB database in 5.75 s at 21 MiB, against aless
0.4.5's 0.52 s at 19 MiB. Every part's time grows in proportion to its
input: four times the records take about four times as long. Pruning
empties each array a grammar builds behind the stream, a JSON or a YAML
document's root sequence among them: the tree a pruned run leaves holds
as many elements after 2,000 records as after 200. A part's memory still
grows by about 2 KB a record, and so does aless's own read: the Rust
engine's lexer holds the whole text as a vector of its characters and
another of their byte offsets, twelve bytes a character, up to
twenty-four as the vectors double, 25 MiB of the 29.5 MiB a pruned run
over JSON's 10,000 records peaks at. A
first measurement the same day left the rule history unbounded, as
alchemy-cli's parsers leave it. The engine then keeps a snapshot of each
rule a repetition replaces, 126 MiB for JSON's 10,000 records and 474
MiB for 40,000, and that measurement read the memory as the root
sequence's. The maintainer's calls, on the measurement above:
- aless reads JSON, YAML and XML through their parts, at that cost.
- The engine bounds its rule history by default, at three predecessors:
  the default tabnas/parser's `doc/rule-history-bound.md` proposes once
  every runtime carries the option, as all three now do.
- What still grows by about 2 KB a record is traced before anything is
  built for it: it is the lexer's copy of the text, which step 10's
  chunked lexer source is for. Emptying a parent's array behind each
  element's close, proposed on the first measurement, would free
  nothing, and is not built.
- For PGN, alchemy gains natives that join two vectors, compare two
  values and count, in all three runtimes, to replace what its part
  spells today as six scans an event and `split` tricks; the part is
  then rewritten on them and measured again.

The engine's default came first. tabnas/parser#313, released as
tabnas-parser 0.12.14, bounds Rust's rule history at three predecessors
unless a grammar or a host sets the option, and admin's ADR-29 records
the decision. TypeScript and Go keep theirs unbounded for now, a
difference from TypeScript that tabnas/parser's `DIVERGENCE.md`
registers, for two reasons that running the fleet's suites under a
bound of three measured:
- In TypeScript an unbounded link is the live rule, which costs nothing
  to make, and a bounded one is a copy built on every step. Through
  `@tabnas/json`, 20,000 records took 12.0 s bounded against 3.6 s
  unbounded, and one record more than four times as long. TypeScript
  takes the default once a bounded link costs about what a live one
  does.
- In Go a bounded link is a copy too, and a Go rule's node is a plain
  field the copy cannot share, so a grammar's write through a linked
  rule's node lands on the copy and is lost: 25 of the engine's own Go
  tests failed, and with the parent link kept live the Go suites of
  expr, csv, jsonic, toml, yaml and c still failed. Go takes the default
  once its bounded links carry those writes.

The same release fixes three things the bound and the trace turned up.
Under a bound, a TypeScript pusher's after action can write the child it
pushed, as the path plugin does, and an edit it makes to its own links
stands; and a Rust pusher links its finished child as a bounded copy, as
TypeScript does, so the child keeps nothing past itself alive. And Rust's lexer
keeps a character's byte offset only for a text that holds a character
outside ASCII, where the two differ, and allocates its vectors once, at
their length: an ASCII text costs four bytes a character beside itself,
where it cost twelve, and up to twenty-four as the vectors doubled.
Measured as above, a part read pruned by `run --grammar` with the rule
history bounded at three, before 0.12.14 and with it:

| | before 0.12.14 | with 0.12.14 |
|---|---|---|
| JSON, 10,000 records | 1.54 s, 32 MiB | 1.43 s, 19 MiB |
| JSON, 40,000 records | 5.94 s, 98 MiB | 5.99 s, 48 MiB |
| YAML, 10,000 records | 7.46 s, 33 MiB | 7.75 s, 21 MiB |
| YAML, 40,000 records | 30.08 s, 97 MiB | 28.51 s, 51 MiB |
| XML, 10,000 records | 5.33 s, 32 MiB | 5.18 s, 22 MiB |
| XML, 40,000 records | 20.88 s, 96 MiB | 20.09 s, 56 MiB |

Every part writes the same bytes as before, in about the same time.
What still grows, about 1 KB a record, is the text. A heap profile of
JSON's 10,000 records puts 9.8 MB of the 11.4 MB peak in three copies of
it: the lexer's vector of its characters at four bytes each (6.5 MB),
the text as alchemy-cli reads it (1.6 MB), and the engine context's own
copy (1.6 MB). That is six bytes a byte of the document, and step 10's
chunked lexer source is for it. aless's own read, which sets the bound
itself, gains the lexer's share with its pin on 0.12.14
(rjrodger/aless#81), with `--render json` in the same build profile
before and after, the same bytes in about the same time:

| | aless on 0.12.13 | aless on 0.12.14 |
|---|---|---|
| JSON, 10,000 records | 32 MiB | 20 MiB |
| JSON, 40,000 records | 99 MiB | 49 MiB |
| YAML, 10,000 records | 53 MiB | 42 MiB |
| YAML, 40,000 records | 178 MiB | 133 MiB |
| XML, 10,000 records | 116 MiB | 106 MiB |
| XML, 40,000 records | 394 MiB | 355 MiB |

So at 40,000 records a part read pruned holds about what aless's own
read holds for JSON, and less for YAML and XML. alchemy-cli's parsers, which set no bound, take the
engine's default once its lock names 0.12.14 (tabnas/alchemy-cli#48).

For PGN's part, the natives came first (tabnas/alchemy#97, released as
alchemy 0.2.14): `push-all` joins two vectors, `equal` compares two
values as a `match` pattern compares a constant, and `inc` counts. The
part, rewritten on
them (tabnas/chess#69, released as chess 0.1.18), takes two scans an
event rather than six: the second writes only a comment's commands, a
list of lists that no native joins whole. `equal` and `inc` replace its
`split` tricks, and with `inc` a count past 10,000 streams rather than
being read whole. Over 500 generated games, in alchemy-cli's release
build on tabnas-parser 0.12.13, before its default bound, against the
streaming read alchemy-cli had before:

| 500 games | the read before | the part, first | the part, rewritten |
|---|---|---|---|
| annotated, 603 KB | 0.94 s | 7.98 s | 4.97 s |
| bare moves, 212 KB | 0.48 s | 4.27 s | 2.46 s |

The rewritten part takes 5.3 and 5.1 times the read, within the two to
six times the plan expected, and the same memory. The passes alone,
through a step that does nothing, take 1.44 s and 0.83 s; the rest is
the interpreter's reference counts, name lookups and allocation, not
anything the part spells. alchemy-cli reads PGN through the part
(tabnas/alchemy-cli#43).

The maintainer's calls on both came on 2026-10-21: every format whose
package ships a part is read through it, and step 13 moves its hosts now
and transduce later.

Step 9 is complete. rjrodger/aless#82 reads JSON, YAML and XML through
their parts, and rjrodger/aless#83 JSONC, JSON5, jsonic, ZON and PGN,
released with rjrodger/aless#85 below as aless 0.4.7. The records are
the same: JSONC and JSON5 take about a tenth to a fifth longer than
aless's own read, jsonic and ZON three and a half times as long, and PGN
six times, and jsonic holds 65 MiB at 40,000 records where it held 167.
The move found one thing a part leaves to its host: jsonic, JSON5 and
YAML merge a repeated member, which their parts stream one after the
other, and aless's table routes kept the last, as JSON's value does.
rjrodger/aless#85 holds a part to a tree's contract unless its format's
value keeps the last of a repeated member (JSON's, JSONC's and JSON
Lines'), so such a document is read whole, as tabnas/alchemy#71 made a
composition over a part refuse one.

Step 13's hosts no longer choose the incremental route by a format's
name. tabnas/alchemy-cli#50, in all three runtimes: `run` reads its JSON
through JSON's part, pruned, where it read through the incremental
source; `translate` reads a format with a part through it and any other
whole; and `Format::read` reads through the part, so a number keeps the
lexeme it was spelled with. The Go command no longer needs the
`tabnas_nodecell` build tag, which the incremental source did.
rjrodger/aless#86 does the same for aless: Markdown, the one format the
list named that ships no part, is read whole, and JSON Lines handed over
as one text goes through its part. transduce keeps
`capability::INCREMENTAL` and the inference until step 12, since
`LinesSource` still reads JSON Lines through it.

Step 10's chunked source is designed and prototyped
(tabnas/parser#315, `doc/chunked-source.md`). A Rust prototype reading a
pruned JSON, JSON Lines or CSV document from a reader held 5.8 MB to
7.4 MB from 10,000 records to 160,000, where the whole text grows to
92 MB to 430 MB, for 0.9% more instructions on a whole text (1.1% on the
wall clock) and 2.2% to 3.5% more from a reader (3% to 4%). The
maintainer adopted it on 2026-10-22, Rust first (admin's ADR-30,
tabnas/admin#155): a reader of bytes, a grammar that declares
`lex.window`, a long line's error excerpt cut to the window and
registered, the margin and the block fixed at 64 KiB, and the engine,
json, jsonc, jsonl, csv and transduce shipped in one wave before the
hosts read a file through a reader.

The Rust engine reads a source through a window (tabnas/parser#316):
`Tabnas::parse_reader`, `Lexer::window` and `Lexer::fill`, and the
Rust-only error code `unreadable` for a reader that fails or bytes that
are not UTF-8, for 1.0% more instructions on a whole text, with a
differential test that holds 15,096 parses from a reader to the same
text parsed whole. tabnas-json, tabnas-jsonc and tabnas-jsonl declare
the option (tabnas/json#113, tabnas/jsonc#93, tabnas/jsonl#55), csv's
quoted field scans the window and asks it to fill (tabnas/csv#106), and
`ParserEventSource::from_reader` streams the passes of a source read so
(tabnas/transduce#59). The wave shipped them as tabnas-parser 0.12.15,
tabnas-json 0.5.19, tabnas-jsonc 0.5.16, tabnas-jsonl 0.1.19,
tabnas-csv 0.6.8 and tabnas-transduce 0.2.12.

The hosts read a file through a reader. aless exports a JSON or JSONC
file through its part a window at a time (rjrodger/aless#89). It skips a
byte-order mark and replaces bytes that are not UTF-8 as its whole read
does, and reads the file again whole where the part refuses the stream.
alchemy-cli's Rust command reads `run`'s input, a file or standard input,
through a reader, and so does `translate` for a file on the route
through its part (tabnas/alchemy-cli#53). A read that fails, or bytes
that are not UTF-8, met part way, fail as a file that cannot be read,
with status 2, as before. `translate` and `run --grammar` do not prune,
so on those routes the engine's tree still holds every record.

transduce's `docs/BENCH.md` measures the change and the general path
step 12 leaves (tabnas/transduce#61). A JSON file aless exports through
its part from a reader holds 12 MiB; read whole, it held 136 MiB for
21.6 MB and 261 MiB for 43.6 MB, in the same time. On the general path,
through their parts from a reader, JSON Lines and CSV hold memory flat
as well, but take longer than `LinesSource`. JSON Lines takes a fifth
more: its part is a rule table for the native `transduce-tree`. CSV
takes four times as long: its part is hand-written, and alchemy's
evaluator runs its `scan-emit` steps on every pass. So step 12 waits on
the maintainer's ruling for CSV: a native route for its part, a faster
evaluator, or a line source kept for CSV.

A repeated key in a YAML sequence entry (`- a: ...`) was the one mapping
whose later value tabnas-yaml kept rather than merging, so a host's
whole read and its stream through YAML's part could disagree on it.
tabnas/yaml#132 merges it as every mapping does, and tabnas/yaml#133
fixes the bug found beside it: a key with an empty value in an entry
took its sibling keys as its value. Released as tabnas-yaml 0.5.26, and
the hosts moved to it (tabnas/alchemy-cli#51, rjrodger/aless#87),
released as alchemy-cli 0.1.15 and aless 0.4.8.

Each step is a pull request opened ready for review, with its fixture
rows and its measurements in the description; a step whose test fires
downstream by design lands with the downstream change that accepts both
states first, as the fleet's convention is.
