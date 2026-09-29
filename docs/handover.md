# Handover: streaming transducers, alchemy, and the work around them

Status as of 2026-09-29 12:40 UTC. This records the state of one working
session across the tabnas fleet and rjrodger/aless, so that whoever picks
it up can continue without the session's scratch files, which do not
survive it. Everything a later step needs is written here or linked from
here.

Read sections 1 to 3 for orientation, section 4 for what merged since
the last refresh and what it left, and section 5 for the next work in
order. Section 6
lists the conventions and traps that cost time.

## 1. What was asked

The maintainer's design document, "Declarative Streaming Transducers and
Renderers" (26 September 2026), proposes a small typed, functional
streaming language with an indentation-based S-expression reader, in which
both transducers and renderers are written. They compile to a shared,
backpressured plan over explicitly ordered semantic protocols. Single-use
streams, retained metadata, buffering limits and input-order assumptions
are part of the language contract.

The request was to implement it in Rust only, in three new repositories,
and expose it through aless's headless mode. The key criterion is to
**stream large volumes of data at high speed**. The maintainer's answers
to the planning questions were:

1. The source is any tabnas parser. Source events come from the engine's
   debugging hooks (rule events, `subscribe_rule_done`), extended if
   needed.
2. The DSL is read by a tabnas grammar plugin, laid out like every other
   grammar repository, in `tabnas/alchemy`.
3. The crate split is `transduce`, `render` and `alchemy`, as proposed.
4. Ordinary crates may be added as dependencies in the new repositories.
5. aless exposes it headlessly.
6. The throughput and memory targets proposed are fine.
7. The scope is the design's milestones 1 to 4 plus a JsonEvents renderer.
8. "Merge baby merge": the session merges its own green pull requests.

The parsers' JSON data structures are never altered. The transducer
consumes them, incrementally where that is verified and whole otherwise.

Requests that came later, in order:

- **Dynamic grammars in aless:** read any text format through a custom
  ABNF grammar compiled by tabnas/abnf (`--grammar`, `--grammar-expr`),
  with Unix formats as validation fixtures. Done (aless#11).
- **Panes:** a side-by-side view of the before and after data structures
  of a transduce-render.
  - Each pane has two modes, original source or parsed structure, and
    that two-mode pane is the normal case.
  - A third pane can show the alchemy program.
  - Panes can be arranged vertically or horizontally.
  - Consider ratatui, and highlight with tabnas/lsp where possible.
  - Done; see 5.9. ratatui draws the viewer (aless#22), the panes are
    merged (aless#23), and a pane's text is coloured through
    tabnas/lsp (aless#24).
- **A full Rust port of tabnas/lsp,** used directly as a Rust dependency
  of aless. Done, lsp#21; see 4.2 for what its reviews found and 5.9
  for what aless meets when it takes the crate.
- **The repetition rule:** "Star repetitions must always be alt.r not
  alt.p. This is fundamental. Make sure you have noted it in the agent
  guides." Noted in every affected guide. The compiler fix is bnf#80,
  merged; see 5.1.

Standing instructions:

- Any tabnas repository may be cloned, and pull requests opened for minor
  bug fixes.
- Push often.
- Open pull requests ready for review, never as drafts.
- Merge green pull requests.
- Continue to completion.

Requests that came in the second session (2026-09-28, from 12:50 UTC):

- **"Focus on the Rust implementation; TypeScript and Go are
  deferred."** Every port and release that only TypeScript and Go
  consumers need waits for a new instruction: the bnf npm and Go release
  (5.1), the TypeScript and Go ports of the debug renderer (5.6), and
  the Go engine release that bnf's `go.mod` would take. The Rust chain
  needs none of them: aless, gbnf and debug's Rust crate take bnf by git
  revision or sibling path.
- **A design question, then a pilot:** how aless will
  translate any format to any other without an N-squared set of
  translators. The answer given: every tabnas grammar already lifts to
  the one JSON-shaped event stream, so reading is done; each format
  should declare its shape (text, records or tree), ship an alchemy
  render snippet typed on the shape's protocol (and a lift snippet only
  where its meaning exceeds its syntax, such as CSV's header row), and a
  loss declaration; the runtime composes lift, a shape adapter chosen by
  the two shapes (`records`, `table-from-json` with a named default
  policy, `join` to text) and render, refusing typed rather than
  guessing; the snippets ship in each grammar repository, named in
  `tabnas.plugin.json` and embedded with `include_str!`, so the code is
  compiled in and only chosen by name at run time. A pilot with CSV
  (lift) and YAML (render) and an ADR section in transduce's
  architecture document were proposed. The maintainer's word (18:20):
  "Let's do this: The any-to-any translation design discussion,
  unchanged." The design became transduce's `docs/translation.md`
  (transduce#4), and the pilot has landed, CSV to YAML with no lift,
  since CSV's events carry its records already; see 5.12.

## 2. What was built

| Repository | What it is | Read first |
|---|---|---|
| tabnas/transduce | `tabnas-transduce`: protocols, the event source (whole walk, incremental from rule events, line-chunked JSON Lines and CSV), router and matcher, captures, `ScanEmit`, `TableFromJson`, limits, stable error codes | `docs/architecture.md` (the canonical design), `docs/reference.md`, `docs/BENCH.md`, `docs/translation.md` (any format to any other) |
| tabnas/render | `tabnas-render`: text algebra (`TextOut`, `WriteOut`, `StringOut`, `Join`, `ReplaceText`, `Concat`), the always-quoted CSV renderer, the JSON renderer, `RecordsToJson`, protocol validators | `docs/reference.md` |
| tabnas/alchemy | `tabnas-alchemy`: the language reader as a tabnas grammar plugin; desugar, resolve, check, effects and `explain`; the evaluator that builds plans; lowering onto transduce and render; the standard library; `events` and `compile_sources`, which a format's own render needs; the `alchemy` binary (`canon`, `format`, `check`, `explain`, `run`) | `docs/language.md`, `AGENTS.md`, `rs/README.md` |
| rjrodger/aless | `--render csv\|json` streams a document's records through the transducer (aless#10); `--grammar NAME=FILE` and `--grammar-expr` read any text format through a custom ABNF grammar (aless#11); `--alchemy FILE`, `--alchemy-expr TEXT` and `--explain` run a program over the input (aless#13); a custom grammar runs under the shared depth cap (aless#12); `--render yaml` writes any format as YAML through tabnas-yaml's own render (aless#21); the viewer draws through ratatui, measuring text by grapheme cluster (aless#22); panes put the document beside its output and the program (aless#23); a pane's text is coloured through tabnas-lsp, off the viewer's thread (aless#24); the explorer sees a change its directory's clock does not show (aless#25) | README "Scripts and agents", `skills/aless/SKILL.md` |
| tabnas/yaml | `alchemy/render.alc`: YAML's render as an alchemy part, named by the manifest's `translate` object and handed over as `render_text()` and `manifest_text()` (yaml#87) | `AGENTS.md`, "The translation parts" |
| tabnas/debug | `rs/src/abnf.rs`: the ABNF emitter reads the bnf#80 repeat loop by its whole scaffold and renders it as `*A` / `*( a b )` / `1*A` (debug#63) | `docs/reference.md`, "The repeat loop: the Rust port leads" |
| tabnas/parser | `options.rule.history` in the Rust port: a bound, 1 to 16, on the rule history a replace or push links, so the rule history a flat document keeps follows its nesting and not its length; the value it builds still grows with it (parser#252) | `doc/rule-history-bound.md`, `DIVERGENCE.md` |
| tabnas/lsp | `rs/`: the full Rust port (semantic tokens, documents, instances, analyze, outline, hover, completion, registry, loaders, the JSON-RPC server and the `tabnas-lsp` binary, the generator's Rust target) | `rs/README.md` |

Design decisions that are not obvious from the code:

- **The pipeline is push-based and synchronous.** The parser drives events
  from inside its own callback, and the stages are `Sink`s. The writer at
  the end blocks the parser, so backpressure is inherent. Cancellation is
  a shared `AbortFlag`, polled by a parse guard and by long loops.
- **Incremental streaming is a verified capability per grammar, never
  assumed.** A differential check (incremental events against a full-tree
  walk) passed for json, jsonl, jsonic, jsonc, json5, zon and markdown,
  and for yaml with refusals. It failed for toml, ini, csv, xml and feed,
  which build their values imperatively. Every grammar still works through
  the whole-document walk. JSON Lines and CSV also get a line-chunked
  source that bounds memory regardless.
- **alchemy has two paths:** the two standard compositions run natively,
  and everything else is interpreted. A differential test holds them to
  the same bytes. The known remaining differences are pinned by a test
  and listed in 5.11.
- **Host API for aless:** `compile`, then `Program::{output, row_selector,
  explain, explain_json, sink, sink_out, with_native, with_duplicates,
  with_abort}`, with the constants `STACK_BYTES` (64 MiB),
  `MAX_EVAL_DEPTH`, `MAX_PLAN_STEPS` and `MAX_APPLIED`. A host pushes
  `JsonEvent`s and one `End` on a thread of `STACK_BYTES`, and passes the
  same `AbortFlag` to `ParserSource::abort` and `Program::with_abort`.

## 3. Where everything stands

The first session used the branch `claude/streaming-transducer-render-kzyqua`
in every repository; the second used `claude/alchemy-handover-continuation-bil4x7`
in aless, alchemy, parser, gbnf, debug, lsp, transduce, yaml and admin,
and finished bnf#80 on its original branch. Merges use merge commits titled
`Merge pull request #N: <title>`.

| Repository | Merged | Open |
|---|---|---|
| tabnas/transduce | #1 the crate, #2 and #3 repetition-rule guide notes; #4 the translation design, `docs/translation.md` (09-28 19:10, `d5da8bc`); #5 `Fail` carries its file (21:00, `35ffda6`); #6 `Schema::Infer` binds the first row's names under `max_metadata_bytes` (21:41, `17da362`); #8 what the translation pilot found (23:53, `b81a495`) | none |
| tabnas/render | #1 the crate | none |
| tabnas/alchemy | #1 the reader, #2 phase 3 (`4c9c2a2`), #3 the reader's repetitions as `r:` loops (`f67942d`), #5 a lone carriage return at a line start is whitespace (09-28 12:08, `e9e4daf`), #6 the handover refreshed (15:04, `ae93555`), #7 the debug test accepts both renderings of the optional block (15:09, `c5f966d`), #8 the handover refreshed (18:08, `1b91383`), #9 `events` (20:47, `60850cb`), #10 `compile_sources` (21:02, `32427c4`), #11 `table-from-json` infers its columns under `:columns :infer` (22:03, `5909c43`), #12 `length`, `compare` and `number-class` (23:04, `5b31951`), #13 the handover refreshed (09-29 00:08, `ad0d0b4`), #14 the handover refreshed (01:31, `b04a255`), #15 the handover refreshed (02:26, `a6b7119`), #16 the handover refreshed (04:10, `536f0bc`) | this handover (#17) |
| rjrodger/aless | #10 `--render`, #11 custom ABNF grammars (`0cc8a94`), #12 a grammar from the command line runs under the shared depth cap (09-28 12:53, `5e2b7bf`), #13 `--alchemy`, `--alchemy-expr` and `--explain` (09-28 14:45, `2eec30f`), #21 `--render yaml` (23:39, `36897d7`); #22 the viewer draws through ratatui (09-29 01:01, `7232e41`); #23 panes: the document beside its output and the program (02:19, `1a73b87`); #24 colour for a pane's text through tabnas-lsp (03:35, `2bce042`); #25 the explorer sees a change its directory's clock does not show (04:07, `e98e598`) | none |
| tabnas/parser | #245, #246 guide notes; #248 Go builtins append `src` in amortized constant time (`8e60c46`); #249 TypeScript `@capture$` and `@fold$` append a child's kids one at a time, over the count the child had (09-28 12:36, `d8d00c3`); **#252** a bound on the rule history, the design and its Rust prototype (09-29 08:08, `3391d35`, 4.3); #253 the Rust package renamed `tabnas-parser` 0.12.6 for crates.io (11:10, `c2dd3a6`, 4.4) and #254 a Rust release tag of its own (12:25, `e58c4bf`), both by another session | none |
| tabnas/bnf | **#80** every repetition compiles to a same-depth replace loop (09-28 12:40, `d0a9227`) | none; the release is deferred (5.1) |
| tabnas/abnf | #97 guide note | none |
| tabnas/ebnf | #45, #46 guide notes | none |
| tabnas/gbnf | #47, #48 guide notes; #49 the DIVERGENCE.md 3 pin follows the compiler (`5ee4ff3`); #50 close DIVERGENCE.md 3 and hold the linear side as a standing check (09-28 12:51, `5c24c4d`) | none |
| tabnas/debug | **#63** the Rust emitter renders the repeat loop (09-29 12:11, `be188f1`, 4.1); `main`'s three `ci / ts` jobs are red until the TypeScript port (5.6), and its `Rust` job until the rename reaches its manifest (4.4) | none |
| tabnas/lsp | #20 the semantic-token crate (`b6339f9`); **#21** the full Rust port, eighteen commits (09-28 17:56, `2562bfa`); #23 the generator's patch tables name the engine only (18:34, `ab2646d`) | none |
| tabnas/yaml | #87 YAML's render as an alchemy part, the manifest's `translate` object, `render_text()` and `manifest_text()` (09-28 23:25, `b7c28c4`) | none |
| tabnas/admin | #93 the descriptor task keeps a descriptor's `translate` object and checks its shape (09-28 21:56, `2dc8003`) | none |

Another session, not these, drove the parser 0.12.5 release and the
yaml, ini and csv releases after it, and prepared the parser's 0.12.6
release under its crates.io name (parser#253, 4.4, and parser#254, the
Rust release tag). Leave those alone.

## 4. Merged since the last refresh, and what they left

### 4.1 tabnas/debug#63: the Rust emitter renders the repeat loop, merged

Merged on the maintainer's word ("Merge") at 09-29 12:11 as `be188f1`,
over red jobs of two kinds. The `Rust` job had been red on every run
since the parser's rename an hour before (4.4); the one-line manifest
change that fixes it is a dependency change, left for the maintainer.
The three `ci / ts` jobs had been red since bnf#80: debug's shared CI
builds the sibling abnf against bnf `main`, whose compiler emits the loop, and
the TypeScript emitter still lists the loop's entry as one of the
rule's alternatives, so four tests in `ts/test/abnf.test.js` fail on
`main` until the TypeScript port (5.6) lands: `round-trips repetition
(star and plus)`, `round-trips repetition inside a group`, `keeps
repetition as a production (does not fold *)`, and `round-trips a
control-character literal as %x, not a quoted char-val`, whose grammar
`csv = row *( CR row )` renders the loop's entry as an alternative.

Before the merge, eleven more Codex reviews found hand-built shapes the
emitter read as a loop, or rendered short of what the rule takes, and
once a wording error in the reference; a twelfth, of the final head
`b889ec6` a minute before the merge, found nothing. The first five were
answered by refusing each shape; the sixth changed the approach, and
those after it tightened it, and the loop reading now
accepts only what tabnas-bnf writes, as a dump of its output shows it:
the entry `{ c: [n.rep == 0], n: {rep: 1}, r: H }`, matching no token,
setting no other counter, first and once; continues in the compiler's
two shapes only, a terminal one consuming its token and replacing with
the loop, or a rule one peeking and replacing with the exact iteration
helper (`H$alt0` pushing the item, a rule that reaches the loop by no
synthetic way of its own, then `H$alt0$step1` replacing with `H`),
matching no empty token slot, guarded at most by the suffix-debt
counter `n.debt_… == 0` and setting no counter, at least one of them
live (a continue a FOLLOW peek before it covers never runs: the
compiler writes such dead ones where a token can both start the item
and follow the loop, and they still render, so a dead one's item must
be a live one's); the unconditional empty exit, with FOLLOW peeks that
give back what they peek, and no continue after it; nothing a function
routes; and on a synthetic loop, closes that do nothing. A user loop's
close that replaces with the rule renders as its name (`H = *A [ B H
]`). A synthetic rule the iteration pushes that repeats by a cycle of
its own, or a `$alt` rule with an empty way through, stays a
production. A `_plus` or `_rep` helper is written back as `1*X` /
`n*X`, and a `_plus` folds, only when its chain is the compiler's
construction, consuming or pushing exactly the item its loop repeats,
the loop's whole iteration being that item. The start rule is always a
production. In `rs/tests/abnf_test.rs` the scaffold test
(`abnf_does_not_read_an_entry_without_its_scaffold_as_a_loop`) pins
twenty-one hand-built non-loops, each rendering as `main` did; other
tests pin the helper, close, plus and start shapes and more entry-like
transitions.

Verification, rebuilt in the session's scratch (gone with it; rebuild
the same way): a harness over `/home/user/parser/rs`, `/home/user/bnf/rs`,
`/home/user/abnf/rs` and a copy of debug with two scratch helpers
exposing the loop set and an entry-only rendering. Over 2,000 random
ABNF grammars (seed 4242) and 53 fixed ones, compiled through the Rust
abnf crate and installed three ways (closure, builtins, recognition):
29,514 loops, the same set as the entry-only check of `cdfd35b`, none
rejected, no text difference; over 20 IR-level grammars under six
option sets: 150 loops, none rejected. The final head's emitted text
was compared with that of `52e30a0`, the first head of the whole-scaffold
check, over the same corpus: byte-identical on all 1,980 grammars that
compile within 20 s (73 slower ones skipped), and the IR-level set
still reads all 150 loops, the 30 debt-guarded ones included. That
comparison ran over the whole corpus ten times in the course of the
rounds, the last on the final head, and caught one regression, pushed and fixed
within the round (`ee11ad6`, then `59f534d`): the construction check
first read a terminal item only without lookahead, and `2*"d"` came out
as `D D *D`.
Every Rust crate that takes debug by path passes its debug test against
the final head, built with parser `3391d35`, before the rename (4.4):
alchemy (after alchemy#7), feed, zon, expr, semver and multisource.

### 4.2 tabnas/lsp: the full Rust port, merged

lsp#21 (`2562bfa`, 09-28 17:56): the first session had opened it from
the branch at `6688f76` minutes after writing this document (so the
"open one PR" step was already done, which the second session found
only when a second create was refused), and the second session ran the
two reviews it called for, fixed what they found, answered Codex, and
merged. The PR body is the full record. In short:

- **The reviews** ran as a workflow: two reviewers (real LSP sessions
  against both binaries and the TypeScript server; hostile specs and
  grammars against the loaders and the firewall) and one refuter per
  finding, reproducing it alone from its description. Seven findings,
  six confirmed, one refuted (the binary's unset size cap and parse
  deadline mirror the TypeScript server). The fixes are `af6da3a`, each
  with its test: request ids that are not a string or a number refused
  as `InvalidRequest` under a null id; counted repetitions in L3 text
  capped before the compile (`MAX_REPETITION_BOUND` 512,
  `MAX_REPETITION_TOTAL` 2048; the dialect compilers unroll a count at a
  cost that grows with its square, measured: 256 costs 0.4 s, 512 costs
  1.4 to 3.2 s in a debug build) and the compile on its own thread under
  `Loader::with_compile_budget` (10 s); the firewall's builtin probe
  memoized for the process and the alt scan stopped at a hundred
  findings; `load` read in the canonical order (spec, grammar, module);
  a spec file past serde_json's 128 levels given the firewall's
  refusal; a schema version above 2^53 printed as JavaScript's double.
- **The verification of the fixes** was meant to be a second workflow;
  it failed at once on the account's session limit (subagents refused
  until 17:10 UTC), so the fixes were verified by hand with the
  reviewers' own drivers, which the workflow had left in the session's
  scratch: the ids driver, the loaders' workspace driver on the two-key,
  deep and counted-repetition workspaces, the 35 malformed-input cases
  diffed against their record (one change, the fixed ids), and the
  parity sweep at 300 documents. All held.
- **Codex's four threads** on the opening head, fixed in `f54384c`: the
  generator's Rust target now writes a `[patch]` table per repository
  for the engine, every optional sibling and what each names by path
  (the map is in `ts/src/generate.js`, held to `rs/Cargo.toml` and to
  the sibling checkouts by a test). That went too far, as the review of
  this handover (alchemy#8) showed and a two-crate experiment with cargo
  1.85 confirmed: a crate built from its own checkout needs every
  optional path dependency present, feature on or off, but a git
  consumer with the feature off resolves without them, and only a
  consumer that turns `dialects` or `fleet` on needs those tables; the
  generated crate turns neither on. The follow-up, lsp#23 (18:34,
  `ab2646d`), has the generator's patch tables name the engine only,
  since the generated crate turns no feature on, and corrects
  `rs/README.md` "Install" the same way; binary
  names cargo refuses (`.`, `+`)
  refused at generation, verified with a scratch crate; `serve` returns
  whether the client asked for shutdown and a generated server exits 1
  otherwise; a manifest change that takes a language from an open
  document withdraws its diagnostics once. These touched
  `ts/src/generate.js`, the generator's Rust target: TypeScript code
  that emits Rust, taken as this PR's scope rather than deferred.
- **Filed from the reviews:** tabnas/parser#251 (the Rust engine's
  `parse_recover` is quadratic in document length on the fixture
  grammar: 25 KB in 5.7 s, 50 KB in 20 s, the time in
  `best_partial_value` and `Value::unwrap_undefined`) and lsp#22 (a
  lone-CR document gets an end-before-start symbol range in every
  runtime). The other engine strictness differences the loaders review
  met are in the PR body, unfiled.
- **Left for later, in the PR body:** the optional dialect and fleet
  crates are path dependencies of the library's manifest, so a git
  consumer that turns a feature on needs a `[patch]` table per
  repository for what the feature links (a consumer that turns none on
  needs only the engine's); moving those lanes and the binary into a
  crate of their own would spare even that. The engine's
  `is_builtin_action` is crate-private; admin's `lsp__ci.yml` needs a
  sync; the TypeScript defects the port reported (`.ebnf`/`.gbnf`
  refused by the firewall; `load.module` ignored).

### 4.3 tabnas/parser#252: a bound on the rule history, merged

Merged on the maintainer's word at 09-29 08:08 as `3391d35`, after three
more Codex rounds and two adversarial passes. `doc/rule-history-bound.md`
is the design; the Rust port carries `options.rule.history`, the number
of predecessor snapshots a rule can reach through `prev`:

- **1 to 16** (`MAX_RULE_HISTORY`): every link copies up to the bound's
  worth of snapshots, and past 16 the copies cost more time than the
  bound saves. A grammar asking for more is refused; a value set on the
  options directly is clamped, `Some(0)` read as 1, and a merge compares
  bounds the same way (`effective_rule_history`).
- **What a copy keeps.** A replaced rule's copy, and every predecessor
  copy, drops `child`, `next` and the next's name: a finished child's
  `next` leads back to its pusher's record, whose `prev` is the copy
  before it, a ladder through the whole sequence (keeping them on the
  nearest copy alone grew reach thirteen snapshots per item). The
  `parent` a pushed child keeps holds the pusher's `child` and `next`,
  so `parent.child` and `parent.next` read as unbounded; the pusher's
  copy made before it links the child drops them (kept, a rule pushing
  again from its close phase chained every child it pushed).
- **Measured**, release build, strict-JSON fixture, same value under
  every setting: 300,000 array items 2.8 s and 553 MB unbounded, 1.2 s
  and 57 MB at 3, 2.2 s and 51 to 56 MB at 16; the 25 MB records file
  2,724 MB and 223 s unbounded, 795 MB and 16 s at 3, both runs on a
  loaded machine (unbounded took 29 s on a quiet one, so read the times
  as a ratio at best). A probe over 20 grammar shapes found
  what a rule can reach constant in length at 1, 3 and 16.
- **Registered** in `DIVERGENCE.md` and `test/spec/divergent.tsv`, each
  row against a control: `prev.prev`, `prev.child` and `prev.next`
  past the bound, and the install refusal past 16, under "A bounded
  rule history in Rust"; and a split found while registering it that
  predates the option, "A pusher read back through its child's snapshot
  in Rust": a child pushed from its pusher's close phase reads
  `parent.child.parent.child` as the child pushed before (Rust) or as
  itself (TypeScript and Go), because Rust links the child's own
  snapshot to the pusher as it stood before the push. Repair direction
  Rust (5.13).

What remains is the maintainer's: the TypeScript and Go ports of the
option, with the same cap, TypeScript first, after which the register
rows move to `test/spec`; the document's last section is the proposed
entry for admin's `DECISIONS.md`.

### 4.4 tabnas/parser#253: the crate renamed, which stops every build by path

Merged by the maintainer at 09-29 11:10 as `c2dd3a6`, from another
session, and followed by parser#254 (12:25, `e58c4bf`), which gives the
Rust release a tag of its own: the Rust package is now `tabnas-parser`,
version 0.12.6, ready for crates.io, and its library keeps the name
`tabnas`, so `use tabnas::…` is unchanged. A manifest that takes the
engine by path as `tabnas = { path = "../../parser/rs" }` no longer
resolves against it: cargo looks for a package named `tabnas` there and
stops with `no matching package named tabnas found`. Each such manifest
needs `tabnas = { package = "tabnas-parser", path = … }` and its
lockfile refreshed. A profile keyed by the engine, such as aless's
`[profile.dev.package.tabnas]`, which keeps the engine's debug
assertions off (its `AGENTS.md` says why), must become
`[profile.dev.package.tabnas-parser]`: a profile names the package, not
the dependency's alias, and cargo only warns about a key that matches
nothing, so the build passes with the assertions back on. Both were
checked in this session with scratch crates.

- **What is red.** debug's `Rust` job, on every run since 11:10: the
  last runs on debug#63 and, since its merge, `main` at `be188f1`. It
  clones the parser's `main` and no other sibling, so it passes once
  debug's own manifest names the package.
- **What is red on its next run.** alchemy's `Rust` gate runs only on
  changes under `rs/`, `test/spec/`, `stdlib/`, `ci/rust/`,
  `docs/language.md` and the workflow, so a handover change does not run
  it. It takes each sibling from the branch of the same name when there
  is one, else from `main`. Against the parser's `main`, it passes once
  alchemy's own manifest names the package, and so do those of the
  siblings it builds that take the engine: json, jsonl, jsonic, yaml,
  csv, debug, transduce and render (support takes none). The parser's
  `claude/alchemy-handover-continuation-bil4x7` is still at `3391d35`,
  before the rename, so an alchemy change on that branch name builds
  against the old engine, and passes against it, until that parser
  branch is restarted from `main`.
- **What is not.** aless takes the tabnas crates from GitHub at the
  revisions its `Cargo.lock` pins (the parser at `d8d00c3`, 0.12.5), so
  it builds as before; a parser pin at or after `c2dd3a6` meets the
  rename, and with it the manifest line and the profile key above.
- **Whose it is.** Renaming a dependency is a dependency change in each
  repository, which only the maintainer instructs (each repository's
  dependency rule), so this session did not make it. It is one line per
  manifest, aless's profile key, and a lockfile refresh. Each
  repository's CI builds against its siblings' `main`, so the order in
  which each can go green is the sibling graph's: json and debug, which
  take the engine alone; then jsonl and jsonic, which take json; then
  yaml and csv, which take jsonic; then transduce, render and alchemy.

## 5. Next work, in order

Items 5.1 to 5.4, 5.7 and 5.9 are done, and 4.2 (the lsp port) with
them; so is the translation pilot (5.12), which came between; what
remains of each is listed.
The Rust-only instruction (section 1) orders the rest: 5.5's design and
Rust prototype are merged (parser#252, 4.3) and its TypeScript and Go
ports wait on the maintainer, as does 5.6's TypeScript port (the Rust
emitter is merged, debug#63, 4.1, and debug's `main` is red on
TypeScript until the port lands); 5.8's audit is done and its fixes
are TypeScript-first changes that wait on the maintainer. So does the
parser's rename (4.4): debug's Rust gate passes once its own manifest
names `tabnas-parser`, and alchemy's once its own and its siblings' do
(on this session's branch name it still builds against the engine
before the rename). So does 5.13, a Rust repair parser#252 deferred.
Where a subsection below keeps the fleet's TypeScript-first rule for
landing a change, that landing waits on the maintainer's word; the step
before it does not.

### 5.1 bnf#80: merged; the release deferred

Merged as `d0a9227` after the six reviews the first session had planned
(TypeScript against Go, TypeScript against Rust, engine semantics, a
differential over about 605,000 parses, untrusted input, and the fleet's
downstream). What they found and where it went:

- The TypeScript closures spread a loop's kids into the parent
  (`push(...kids)`) and overflowed the call stack past about 130,000
  items: fixed in `344e0d6`, then `c81586e` takes the count first so two
  nodes sharing one `kids` array cannot grow it under the loop; the
  engine's `@capture$` and `@fold$` had the same spread, fixed in
  parser#249 (`d8d00c3`). The bnf builtins-mode 200,000-item test joins
  the closure-mode one once the package depends on an engine release
  carrying #249.
- The Rust emitter kept the value actions on keyword guards inside an
  `; @array` terminal loop where the canonical strips them: fixed in
  `344e0d6`, in the canonical's order of operations too, pinned by two
  oracle fixtures.
- Go builtins mode is quadratic in the item count under the engine
  version `go.mod` pins (v0.12.4 grows a node's `src` by copying);
  parser#248 fixed the engine, and a Go engine release plus a `go.mod`
  bump remain. Deferred with Go.
- A loop whose item can match nothing after its dispatch token
  (`*("a" "b" / ε)` on `ac`) spins until the engine's rule budget ends
  the parse; pre-existing as an unbounded recursion on the push chain. A
  grammar-side refusal of a nullable loop item is a follow-up.
- Rust `Value::to_json` recurses per level and overflows an 8 MiB stack
  on a tree about 10,000 levels deep that the parser itself handled.
  Pre-existing; a follow-up in the Rust crate.
- abnf's `docs/design/array-repetition.md` still describes the helper
  as a push chain. Doc drift, one PR.

**The release (npm and the Go module) is deferred** by the Rust-only
instruction: only TypeScript and Go consumers need it. When it is
revived, the steps are in bnf's AGENTS.md, "Dispatch it; do not push
the tag": bump the five version sites to 0.1.23 (`ts/package.json`,
`ts/src/bnf.ts`, `go/bnf.go`, `make version-rs V=0.1.23`, the lock
follows), verify with `make downstream`, merge the bump through a PR,
wait for `main`'s CI, record `REL=$(git ls-remote origin
refs/heads/main | cut -f1)`, dispatch `release.yml` on `main` with
`go: true`, and confirm `npm view @tabnas/bnf@0.1.23 gitHead` equals
both tags and `REL`. Note that debug's TypeScript suite fails against
the loop compiler already (4.1); the release changes nothing there.

### 5.2 gbnf: done

gbnf#50 (`5c24c4d`) deleted DIVERGENCE.md 3, its pin and its row (later
entries keep their numbers), added
`a_repetition_adds_no_rule_depth_and_parses_in_linear_time` in
`rs/tests/perf_test.rs` over five shapes (ten thousand items reach the
depth the fewest items that run one full iteration do; four times the
input costs about four times the work), raised
`a_long_input_parses_without_hanging` to 10,000 and 100,000 characters,
and corrected the guides. Codex asked for a one-item baseline; the
measured depths (one item never enters a separator group, two do, and
10,000 sit at the same depth as two) show why the baseline is the
fewest items that run one iteration, and the thread records it.

### 5.3 aless: done

aless#12 (`5e2b7bf`): `MAX_CUSTOM_RULE_DEPTH` and `max_rule_depth` are
gone, a custom grammar runs under `MAX_RULE_DEPTH` (3,000), the value
measure stays, the hints and the prose are rewritten as history, and
two tests pin the depth (2,000 lines at depth two; a 10,000-line `hosts`
file at depth three). The lock move was `cargo update -p tabnas-bnf -p
tabnas`: the resolver ties the engine to the compiler, because the
compiler names the engine through the patch table and one git source
carries one revision in the lock, so tabnas moved from `198c7c33`
(0.12.4) to `d8d00c35` (0.12.5) with bnf to `d0a92270`; abnf stayed at
`8ae40a50` and compiles through the moved bnf. Nothing else moved.

### 5.4 aless: done

aless#13 (`2eec30f`) is the surface planned here, with one refinement
the review forced. Read its PR body for the full record; in short:

- `--alchemy FILE`, `--alchemy-expr TEXT`, `--explain`, `--render` as
  the renderer for a table or event result, the usage errors, the
  `ParserSource`/`LinesSource` choice, the four documentation sites in
  agreement, the tests listed in the plan and more.
- **One rule places a run's failures** (README, Programs, "Errors"):
  where a failure came from decides whose it is. The export layer
  records the origin (`Scope` notes when the inner sink errs;
  `classify` returns `ExportError::Program` for a program job's sink
  failures). From the program's sink, a language code is the program's
  (kind `alchemy`, status 2) and so is any positioned failure (events
  carry no positions, so the position is in the program), reported with
  the program's file, `format: null`, its line and column and `input`;
  a positionless sink failure (a renderer's over the program's rows) and
  every failure of the source's are the input's. The deadline covers
  the parse and the program: the alarm raises the program's abort flag
  in every mode; a timeout raised in the program's work on an item names
  the document with no position, one raised in the parse carries how
  far the parse got.
- **Review:** four lenses (contract, untrusted input, behaviour, code)
  with two refuters per blocker or major finding, alongside Codex's
  four; every fix commit was refuted twice before it landed. The tests
  that stop a slow program at the deadline give it cubic work over a
  300-number row and allow 1.5 to 2 s, because CI runs tests in
  parallel on two-core runners and a debug build's parse of a one-line
  TOML document there took over a tenth of a second.
- **Measured** (release build, 4 cores): 200 MB JSON Lines, 1,336,654
  lines, 10 MB peak on every path (echo 174 s CPU, a select-map-join
  program 351 s, `--render csv` 423 s, all under load); 25 MB JSON,
  312,194 records, on a quiet box: the spec's table program 26.0 s CPU,
  `--render csv --path` 17.5 s, `--json` 26.7 s, all at 2.79 to 3.03 GB
  peak. An earlier reading of 291 s for the table program was the box
  under a load of 22, not the code: alchemy's own binary scales linearly
  from 8 MB (9 s) to 25 MB (32 s).
- **Left as follow-ups,** all pre-existing on the first commit: the
  whole-value fallback re-runs a program whose own `STREAMABILITY_UNKNOWN`
  arrives before any output (the origin flag makes it a one-line
  exclusion with a test that counts chain builds); a parse-side timeout
  reports line 1, column 1 in every mode tried; `--timeout` does not end
  a run whose standard input never delivers a byte, under `--render`
  too; partial output on a failure is cut mid-record, under `--render`
  too; a program's output size is unbounded by default;
  `app::tests::explorer_parent_and_refresh` failed once on Windows (the
  explorer did not list a file written a moment before) and passed on
  re-run, a robustness gap in that test. aless#21 moved the alchemy
  pin to its main, with transduce and csv, which the move needs, so
  alchemy#5 onwards are in aless.

### 5.5 Throughput and memory: the key criterion

The numbers are in `tabnas/transduce/docs/BENCH.md` and were measured on
one core of the development container on 09-27. Re-measure before
quoting them.

- **The engine parses at about 1 MB/s** on record-shaped JSON, JSON
  Lines, CSV and YAML. That is about 20 µs of engine work per array
  element. The Rust port is not slower than the canonical TypeScript.
- **Peak resident memory is about 65 to 90 times the input,** 1.6 GB for
  a 24.6 MB records file and 2.8 GB for 30.9 MB of CSV.
- **Pruning streamed elements from the tree barely helps.** The retention
  is the engine's rule history: each rule keeps snapshots of the rule it
  replaced and of its children until the enclosing container closes, and
  the links are transitive.
- **The adapter is cheap:** 0 to 15% over a plain parse.
- **The line-chunked source bounds memory** for JSON Lines (about 10 MB
  peak for `aless --render csv` on 150,000 lines) but not speed: about
  1.4 MB/s in a release build.

So bounded memory for one large document, and speed, are the engine's to
deliver, not the transducer's. Two changes in tabnas/parser would move
both:

1. Bound the rule history. Drop `prev_rule` and child snapshots once a
   rule can no longer be rewound to, within the existing `rewind.history`
   cap.
2. Reduce the per-rule cost, which is allocation and cloning per step.

Both are engine changes across three runtimes with a parity contract.
The first is designed and merged in Rust (parser#252, below); do not
redo its profile or design. For the second, start with a profile of the
Rust port on the 24.6 MB records file and write the design in
tabnas/parser's DIVERGENCE and ADR terms; those two steps are Rust work.
The fleet lands an engine change TypeScript first, which the Rust-only
instruction defers: the landing order is the maintainer's to set once a
design is in. The transduce benches (`cargo bench` in `rs/`) and
aless's `--render` acceptance runs are the measurements to repeat
after each change, including a 2 GB JSON Lines export, which the
line-chunked source already bounds in memory.

Measured again on 09-28 in aless#13 (above): the program path is linear
in the document and costs about half again the plain renderer's time on
JSON; peak memory is the engine's rule history on every path, 2.8 GB
for 25 MB. aless#21 measured the same on 33 MB of JSON: 2.09 GB under
`--render json`, `csv` and `yaml` alike.

The first change is designed and merged in Rust: parser#252 (4.3), which
bounds the rule history and takes the 25 MB records file from 2,724 MB
to 795 MB under `history: 3` (the times, 223 s to 16 s, were read on a
loaded machine; unbounded took 29 s on a quiet one). aless and the
transducer do not set the option yet; doing so is the next measurable
step, once aless takes a parser pin with it (`3391d35` carries the
option without the rename; a pin at or after `c2dd3a6` meets the
rename, 4.4). The second change, the
per-rule cost, has no design yet. The TypeScript and Go ports, and what
lands in what order, are the maintainer's.

### 5.6 tabnas/debug: render replace loops correctly

The Rust port is merged, debug#63 (4.1). What it does, so the TypeScript
and Go ports can follow it line for line (the canonical is
`ts/src/debug.ts` `emitAbnf`; `docs/reference.md`'s section "The repeat
loop: the Rust port leads" records that the port leads until then):

- `has_content` is decided by what an alternative consumes (a token past
  its backtrack, a push, or a replace with another rule); `{ }`, the
  FOLLOW peek and, on a loop, the entry `{ r: H }` are epsilon. Any
  other self-replace keeps its content (`one = A [ one ]`).
- A rule whose open alternatives are the loop's whole scaffold is a
  loop: the entry first (no token, no push, `r` itself, the `n.rep == 0`
  guard, the counter set to 1), continues in the compiler's two shapes,
  each coming back to the rule having consumed or pushed, and the
  unconditional empty exit `{ }` with no continue after it; no
  alternative a function routes. Point 3 of the reference's section is
  the full list, and 4.1 summarises it. It is rendered wherever
  referenced as `*A` / `*( a b )` over its continue alternatives; its
  back edges render nothing; it and its helpers are never productions,
  but the start rule always is. The helpers are everything the
  iteration reaches short of a kept production (a user rule, another
  loop, an old-shape repetition helper, a synthetic rule that repeats
  by a cycle of its own, a `$alt` rule with an empty way through), so a
  group's own `$alt`/`$step` chain inlines and an old push-chain star
  inside stays a production.
- A `_plus` or `_rep` helper over the loop's item is written back as
  `1*A` / `n*A`; a repetition over an optional is `*[ A ]`; a helper's
  kind is read from its own name segment.
- The old push chain renders as before.

The second gap the first session listed (`hasContent` counting a
backtracked `s` as content, so alchemy's reader renders `line = form IN
block` for `line = form [ IN block ]`, and json's `elem` close as
mandatory) is covered by the first point in Rust; it remains in
TypeScript and Go. The coupling notes stand: six Rust crates depend on
debug by path, about twenty TypeScript packages and jsonic's Go module
use the other ports, and some pin rendered text (alchemy's
`rs/tests/debug_model_test.rs` accepts both renderings since alchemy#7);
survey and run them before a port merges, as 4.1 records for the Rust
one. Also found beside the differential, by a
hand-written grammar, and not this PR's (the differential's generator
spells every literal as `%s"…"`, so its recompile count of zero does
not cover this case): a case-insensitive ABNF literal (`"a"`) reaches the Rust
engine as a regex matcher whose `as_str()` carries no `(?i)`, so the
legend renders it as the prose-val `<regex /^a/>`, which the abnf crate
refuses to recompile; the round trip holds for `%s` and punctuation
literals.

### 5.7 alchemy: done

alchemy#5 (`e9e4daf`): the indentation scan restarts at a lone `\r`, the
layout token ends at the first lone `\r` on its form's row (columns
after it stay right), a lone `\r` before the first line is a line
start, the `line` rule's extra alternative is gone, and a scan before
the first form is cached (`LayoutState.checked`) after Codex showed the
rescans quadratic. Six fixture rows and rule 7's note carry it. Old and
new binaries agreed on every input without a lone `\r` over the
fixtures, the reference's blocks, `stdlib/*.alc` and 5,500 generated
inputs; the 85 that met "malformed reader output" parse.

### 5.8 The fleet's repetitions against the rule: audited

Two read-only audits classified every repetition as a replace loop, a
push from the close state (constant depth, but the rule broken in
letter), a push chain (depth grows with the item count), or real
nesting. The first (09-28) read the local checkouts and stopped on a
usage limit. The second (09-29, the Rust ports) measured the ten
repositories the first left and verified its suspects. It read the
engine's rule depth `d` through `subscribe_rules`, as
`rs/tests/repeat_test.rs` does, at 1, 2 and 10,000 items, and `d`
equalled the rule stack's length at every step. Four crates keep their
parser private (css, abnf, ebnf and gbnf); it measured copies of them
with a subscriber added and nothing else changed. An independent check
through the public APIs reproduced the toml, c and ebnf results below.

The second audit's checkouts: css `e0ea0b4`, c `baf1f8a`, proto
`ffb89e1`, expr `b34a559`, abnf `0fbe319`, ebnf `89b6772`, gbnf
`5c24c4d`, parser `a5b604a` (parser#252's head, engine 0.12.5), debug
`cdfd35b`, support `5c4b5f9`, jsonic `4be139f`, yaml `b7c28c4`, ini
`2b7f6c7`, toml `e5121ad`, semver `f4aed5d` and chess `54aa55b`, with
bnf at `d0a9227` (bnf#80 merged).

| Repository | Result |
|---|---|
| json, jsonl, jsonc, csv, xml, markdown, zon | every repetition a replace loop, ports agree |
| json5, feed, path, hoover | define no rules of their own |
| directive, multisource | rules, but no repetitions |
| debug | no rules |
| parser | the one grammar the crate ships, `make_json()`: elements and members are replace loops, `d` 3 at any length |
| support | `rs/adder`'s `1+1+…` is a replace loop, `d` 1 |
| proto | ABNF compiled at install through abnf and bnf `d0a9227`: every repetition is bnf#80's replace loop, constant from 2 items to 10,000; nested messages take 4 a level |
| semver | **fixed in Rust by bnf#80**: every port compiles the ABNF when it installs, and the Rust crate compiles it through the bnf beside it: its 26 repetition helpers are now replace loops, constant to 10,000. The TypeScript and Go ports take bnf's last release, 0.1.22, which predates #80, so they keep the first audit's 21 push chains until 5.1's release. Three Rust passages still say depth grows per character (`rs/AGENTS.md`, `rs/tests/perf_test.rs`, `rs/src/lib.rs`), and `rs/Cargo.lock` pins `tabnas` 0.12.4 against the engine's 0.12.5 |
| css | its own rule machine, not the engine, with the same `p` and `r`: every repetition a replace loop, `d` 4 to 7 at any length. Nested rules and `@media` take 4 and 3 a level, and the machine has no nesting cap |
| jsonic | **structure, confirmed**: a path-dive key `a:a:…:1` reaches `d` = 3N, exactly as `{a:{a:…}}` does, and is refused past 127 levels |
| yaml | the same, inherited: `a: a: …: 1` reaches 3N+1, as its flow form does, and is refused past 127 |
| expr | replace loops for infix, suffix, lists and arguments; the call chain `f(1)(1)…` pushes from the close state at constant depth; prefix and ternary chains nest as their operands do, bounded by `NODE_LIMIT` (127) and `RULE_LIMIT` (1,024) |
| abnf, gbnf | productions are a replace loop; alternatives and sequences push their item from the close state, `d` 5 at any length; groups nest, capped |
| ebnf | as abnf, plus one push chain: **`post`**, a run of postfix operators `"a"???…`, pushes a rule per operator (`d` = N+5) and is refused at 130 by `MAX_NEST_DEPTH`. The crate's own doc spells it as a repetition, `('?' \| '*' \| '+')*` |
| c | every repetition a replace loop or a push from the close state, constant to 10,000. Eight of those alternates also carry an `r:` that the push makes dead. Blocks and `while` nest, 3 and 2 a level, refused near 1,040 by `RULE_LIMIT` |
| chess | the game's tag section pushes from the close state (`game.close[0]`, `p:tag`): `d` 4 at any length. Games and moves are replace loops |
| ini | **push chain, confirmed**: `dive` (`[a.b.c]`) reaches `d` = N+1 and is refused past 127. The path it builds is flat: `@dive-push` copies the segment vector, and `open_section` walks it without recursion |
| toml | **push chain, confirmed, and unbounded**: `dive` (`a.b.c = 1`) reaches `d` = N+2: 1,002 at 1,000 segments, and 10,002 at 10,000, which took 82 to 92 s in a release build. A table header of the same path, `[a.a.…]`, builds the same nesting at a constant `d` 4 through `table`'s replace loop |

jsonic's and yaml's dives, expr's prefix and ternary chains, and the
nesting in c and css are structure: their depth follows the value's.
The fixes are below. Each is its own PR in its repository, with the depth
test that proves the rule. Each lands TypeScript first, so under the
Rust-only instruction each waits on the maintainer's word.

1. **toml `dive`**, the one unbounded chain: make it a replace loop, as
   `table` already is.
2. **ini `dive` and ebnf `post`**: make each a replace loop. ini's path
   is already flat. ebnf's syntax tree nests an element per operator,
   which the loop can build as it goes, with no rule per level.
3. **The pushes from the close state** (chess, the abnf, ebnf and gbnf
   readers, expr's call chain, and c): their depth is constant, so they
   break the rule's letter, not its bound. Convert each when its
   repository is next changed, and drop c's dead `r:`s.
4. **semver**: correct the three stale passages. Its lock moves with the
   next change there, and its TypeScript and Go ports are fixed by bnf's
   release (5.1).

The rule's text is in every affected repository's AGENTS.md,
"Repetition is replacement, never a push chain".

Found on the way, outside the rule, and not yet filed:

- **c refuses a cast of a cast**, `(int)(long)y` or `(int)((int)y)`, as
  `unexpected`. No fixture has one.
- **c's tree doubles per operator.** It repeats each binary node's
  operands under `children` and again under `left` and `right`, so the
  JSON of `x = 1+1+…` is 0.5 MB at 8 terms, 8 MB at 12 and 128 MB at 16.
- **c's `__asm__` operand, clobber and label lists stop after one
  item.** The rest is read as a second declaration, and the parse
  succeeds.
- **c's `preprocessor_line` is unreachable** in either mode.
- **c's declarators grow worse than linearly**: `int a0, …` takes 47 ms
  with 1,000 of them and 10 to 23 s with 10,000.
- **toml's dotted paths also grow worse than linearly**, `table`'s loop
  included: a header takes 1.5 ms at 100 segments, 100 ms at 1,000, and
  41 s at 10,000.

The independent check reproduced the cast, the doubling (at 4, 8 and
12 terms), and toml's depths and times up to 1,000 segments.

### 5.9 aless: ratatui, panes and highlighting: done

Three pull requests, in this order, and a fourth for a race the
second's Windows CI turned up.

**1. ratatui as the drawing layer: merged** (aless#22, 09-29 01:01,
`7232e41`). It went as planned, with these differences:

- **Rust 1.88** is the minimum, in `rust-version`, the CI job "rust 1.88
  (minimum supported)", the README and AGENTS.md.
- **`render::screen(app)`** returns the cells of a `TestBackend` draw.
  It stands where the planned `screen_text` shim would, and the render,
  app and app_flow tests read the screen through it.
- **Widths are measured by grapheme cluster,** with ratatui's
  `CellWidth`, as the buffer places text. The old per-character measure
  gave a family emoji joined by zero-width joiners six cells for the
  terminal's two, and a halfwidth kana with its sound mark one cell for
  two. The measures walk the clusters lazily. `tests/render_memory.rs`,
  a counting allocator in a binary of its own, holds measuring and
  clipping a million-character value to under 64 KiB; collecting the
  clusters first had held 36 MiB (Codex's finding).
- **`scripts/pty-smoke.py` reads the screen through a terminal model.**
  ratatui writes only the cells that changed, so a word on screen is no
  longer contiguous in the byte stream.
- **`tests/yaml_render.rs` reads `cargo metadata` offline, then online.**
  The lock names ratatui's optional packages, which a build never
  downloads, and the offline read failed on Windows CI.
- **An idle tick writes about 22 bytes**: the synchronized-update
  brackets and a hide-cursor, where the old loop wrote nothing.

**2. Panes: merged** (aless#23, 09-29 02:19, `1a73b87`), as planned
below, with these differences:

- **`s`**, the key aless already gave the source view, switches the
  focused pane between its tree and its text; the plan said `S`.
- **`C-w`** moves the focus, and a click focuses the pane under it.
- **The output pane's tree** is its text read back in its own format
  (CSV, JSON or a format's own), not a recorder of the protocol stream.
- **`r` in the program pane** reads the program again, watched or not.
  **`:only`** keeps the input pane alone.
- **The run happens on the viewer's thread,** under the parse timeout,
  keeping at most 16 MiB of output.
- **The output is written as the command line would write it:**
  `--indent` as given, a pane's new document folded to `--depth`, and a
  renderer given naming the output's format.
- **`Tab::apply` moves a tab's generation** with every document it
  takes, so the output follows `:format` as it follows a reload.
- **Each pane keeps its own place.** A change to a tab re-anchors it in
  the view of the pane that shows it (`pane_view`). A text view's
  scroll is clamped to a shorter text and to a taller pane; the single
  view had both gaps before.
- **Giving `--panes` means the viewer,** whatever it names; a value
  that names no pane is refused.

Codex reviewed it five times: the first four rounds' findings were
fixed, and the fifth found none. Its Windows CI showed a race in code
it does not touch. The explorer notices a new entry only by its
directory's modification time, and NTFS's clock ticks about every
16 ms. So a file made within a tick of the directory's last change
stays out of the tree, and `explorer_parent_and_refresh` failed once.

The fix is aless#25 (09-29 04:07, `e98e598`), merged after the third,
since the branch carries one pull request at a time. Codex reviewed it
four times: the first three rounds each found a gap, and the fourth
none.
[The comment on aless#23](https://github.com/rjrodger/aless/pull/23#issuecomment-5882152411)
gave the first version, which measured the window from the
directory's age; the rounds moved it to this:

- **The window runs on this machine's steady clock** (`Listing::racy_until`,
  an `Instant`): two seconds (`RACY`) from the reading that finds a
  directory's time, whatever that time says. Two seconds covers FAT,
  a second on ext3 and HFS+, and NTFS's tick. The tick that gave the
  directory its time ends within two seconds of any reading of it, but
  the time cannot say how long ago the change was: on a network share
  it is the server's clock, which may run behind this machine's (the
  age-based window never opened) or ahead of it, as a restored
  archive's dates do (the age-based window stayed open until the
  clocks met).
- **`Explorer::changed` reads again, and compares,** any listing with a
  window. The first check after the window reads the directory once
  more, since a change late in the window may have come after the last
  reading in it, and only then closes the window.
- **The window is kept, not renewed,** while the directory's time stays:
  a check and a refresh of the whole explorer (`relist_all`, which a
  change anywhere brings) both read again through `read_again`. So a
  directory is read again for two seconds at most after a new time is
  first seen: a few re-reads on the half-second tick, then none.
- **`Tab::stamp_changed` takes `&mut self`,** for the re-read.
- **Tests:** a change the clock does not show is seen; a change late in
  the window is seen by its last reading; a refresh keeps each window;
  a directory dated an hour behind and one dated an hour ahead (Unix,
  `File::set_modified`) are each read again for their window, and the
  one ahead no longer.

The plan it follows:

- **Model:** `Workspace { panes, arrangement: SideBySide | Stacked,
  focus }` and `Pane { role: Input | Output | Program, mode: Source |
  Structure }`.
  - Today's single view is a workspace of one Input pane.
  - `S` toggles the focused pane's mode, replacing the global
    `Mode::Source`.
- **The Output pane** computes the transduce-render of the input in
  memory, bounded by `max_output_bytes`, and recomputes on reload and on
  a program change.
  - Source mode shows the rendered text.
  - Structure mode shows a recorder of the same protocol stream as a
    tree.
- **The Program pane** shows the `.alc` source, or the `explain` report
  as its structure.
- **Commands and options:** `:vsplit`, `:split`, `:arrange`,
  `:pane out|program|close`, a focus key jless leaves free, `--panes
  out` and `--stacked`. Headless output is unchanged.

**3. Highlighting through tabnas/lsp's Rust crate: merged** (aless#24,
09-29 03:35, `2bce042`) after six Codex rounds, as planned below, with
these differences:

- **Colours are made off the viewer's thread,** on one worker, and the
  text shows plain until its colours come in. Colouring holds the whole
  lex trace: 0.9 MB of JSON took a second of a release build and peaked
  at 270 MB.
- **Each text is coloured on a thread of its own,** with the parse
  stack, and only while a pane shows it. The worker skips a queued text
  no pane shows, and one that leaves the panes mid-parse is called off
  through a flag its deadline reads (`load::Deadline::or_cancelled`).
  Making a large custom grammar's parser takes seconds no flag reaches,
  so a called-off thread is left to finish alone, at most two of them
  (`MAX_LEFT`). With two left, a shown text waits in the queue until
  one ends, or until its deadline and a second's grace, while the
  worker takes in newer texts and skips hidden ones. The deadline starts
  before the parser is made, as the loader's does.
- **Each grapheme cluster takes the style of its first character,** and
  a line is read only until what shows is settled, a chunk at a time,
  each twice the last. `tests/render_memory.rs` colours a minified line
  of a million characters under 64 KiB (the first version held
  38.8 MB); 200,000 combining marks colour in well under five seconds.
- **Without colour** (`--no-color`, `NO_COLOR`) nothing is lexed for it.
- **Limits:** text over 512 KiB stays plain, and one text's colouring
  stops after ten seconds, or the parse timeout when that is shorter.
- **The fallback is plain text,** not aless's value-kind colouring. Plain
  text and a grammar the registry marks speculative stay plain; no
  bundled grammar is marked so today.
- **The output is lexed by its own format's grammar,** the one it is
  read back with, and colours are keyed by tab, generation and grammar.
- **What lsp maps today** leaves TOML's string values and alchemy's
  `:name` keywords uncoloured; closing those gaps is lsp's work.

The plan it follows:

The crate is on lsp's `main` since #21. Taking it by git with no
feature on needs only the `tabnas` patch aless already keeps per
repository: cargo resolves a git dependency's optional path
dependencies only for the features a consumer enables (measured with
two toy crates under cargo 1.85; see 4.2). aless needs neither
`dialects` (it compiles grammars itself) nor `fleet` (it links the
grammars itself), so the crate split recorded in 4.2 is not on its
path.

- **How:** `tabnas_lsp::highlight` takes the engine's lex trace, keeps
  the newest token at each position, and maps each token to a fixed
  nine-type legend. The mapping comes from the canonical table, the
  prefix conventions and per-grammar overrides.
- **Where:** in every pane's Source mode: the tab's grammar, the output
  re-lexed with csv or json, the program with alchemy's grammar, and
  custom grammars. Cache per text and grammar.
- **Fallback:** grammars whose registry entry says their lex stream is
  speculative keep aless's value-kind colouring.

### 5.10 Upstream defects and candidates: filed

Every item below is now an issue (filed 09-28 15:00 to 15:20 UTC, one
per item, with its reproduction): 1 to 4 and 6 in tabnas/bnf (#81 to
#85), 5 and 7 in tabnas/abnf (#98, #99), the `options.rule.finish`
observation in tabnas/parser (#250), 8 to 11 and 15 in tabnas/bnf (#86
to #90), 12 in tabnas/debug (#64), 13 in tabnas/abnf (#100), 14 as six
issues in rjrodger/aless (#14 to #19). Item 10 (`Value::to_json`) lives
in the engine crate (`parser/rs/src/value.rs`), which bnf#88 names.
The lsp reviews added tabnas/parser#251 and lsp#22 (4.2). The
translation pilot (5.12) filed four: tabnas/admin#94 (the plugin
schema every manifest's `$schema` names is not published),
tabnas/yaml#86 (a quoted key at column 0 after a block sequence is read
into the sequence), tabnas/yaml#88 (a flow sequence first in an
indented block sequence replaces it, or fails the parse) and
tabnas/transduce#7 (the incremental YAML source streams a member's
value before its key when the key is a mapping).

Found while writing aless's Unix-format grammars against bnf 0.1.22 and
abnf 0.4.16. Reproductions are in rjrodger/aless
`tests/fixtures/grammars/README.md`.

1. The annotation planner counts a single-literal rule as a part, though
   it is lifted to a token.
2. The core rules LF, CRLF and SP count as members.
3. A literal-only repetition (`*" "`) counts as a part, though the
   message says literals produce none.
4. `1*( TX ) ; @array` concatenates into one element, while `1*word`
   collects.
5. Character-level class tokens are eager everywhere. This is a design
   limitation to document.
6. A bare-token rule `a = TX` vanishes as a non-leading member.
7. Emitted specs carry none of the lexer defaults they rely on, so plain
   text grammars need the caller's option patch. aless applies a preset;
   an abnf option (`plainText`) could emit it.

Engine observation: `options.rule.finish` is parsed but never read by the
Rust parser.

Found in the second session:

8. tabnas/bnf: the compiler (new and old) exhausts memory (13.8 GB,
   killed) compiling one random grammar with nested bounded
   repetitions, in `emit_grammar_spec` before any parse. The grammar is
   in debug#63's verification record; not shrunk.
9. tabnas/bnf: a loop whose item can match nothing after its dispatch
   token spins until the rule budget (5.1).
10. tabnas/bnf (Rust): `Value::to_json` overflows the stack on a tree
    about 10,000 levels deep (5.1).
11. tabnas/bnf (Go): builtins mode quadratic under the pinned engine
    until a Go engine release and `go.mod` bump (5.1).
12. tabnas/debug (Rust, and the ports): a case-insensitive literal's
    legend is prose the abnf crate cannot recompile (5.6).
13. tabnas/abnf: `docs/design/array-repetition.md` describes the old
    push chain (5.1).
14. rjrodger/aless: the follow-ups listed under 5.4.
15. tabnas/parser (TypeScript): the bnf builtins-mode 200,000-item
    repeat-depth test waits on an engine release with #249.

### 5.11 alchemy: known gaps, recorded in its PRs

- **One native/interpreted difference remains, pinned by a test.** When
  the metadata is selected twice, the native path reports
  `INPUT_ORDER_VIOLATION` and the interpreted path `INPUT_INVALID`. The
  standard binding cannot reach it.
- **Message text can differ between the paths;** codes and limit names
  agree.
- **A `no_match` caused by the document is `DSL_TYPE_ERROR` (exit 2)**
  and carries no input path.
- **The checker stops following a stream** past `MAX_APPLIED` definitions
  or through a function value. The runtime's single-lowering guard
  enforces single use instead.
- **The checker is lenient on items it cannot type.** A `Value` item
  passes where particular data is wanted and is checked at run time
  (alchemy#2's Codex round).

### 5.12 Any format to any other: the pilot, done

The design is transduce's `docs/translation.md`: each format declares
its shapes and loss in its manifest's `translate` object and ships its
render (and a lift, where its events do not carry its shape) as alchemy
text its crate embeds; the host composes lift, a shape adapter and
render per translation, so nothing is written per pair of formats.
The pilot writes any format aless reads as YAML (`aless --render yaml
FILE`). It landed as:

| Step | Pull requests |
|---|---|
| The design | transduce#4 |
| 0, the descriptor task keeps and checks the object | admin#93 |
| 1, `events` | alchemy#9 |
| 2, `Fail::file` and `compile_sources` | transduce#5, alchemy#10 |
| 3, YAML's render, the manifest's object, the accessors | alchemy#12 (the natives), yaml#87 |
| 4, the inferred binding | alchemy#11, transduce#6 |
| 5, the wiring in aless | aless#21 |
| What the pilot found | transduce#8 |

transduce's "What the pilot found" is the record. In short: no manifest
schema is published, so the descriptor task is the one check; the
render keeps one marker per open container, because keeping each
mapping's keys made its time quadratic in a mapping's width, and every
host holds a stream to a tree's events in front of it instead (aless's
`export::UniqueMembers`), falling back to the parsed value; a packaged
crate holds nothing outside `rs/`, so yaml embeds copies under
`rs/translate/`, held to the files by a test; the round trip runs in
aless, where 673 of the 694 inputs the reader reads come back the same
and 21 meet yaml#86 and #88, in a checked ledger that fails when the
reader is fixed; and YAML costs twice the native JSON export's time at
the same peak memory, the parse's.

Follow-ups, none started:

- **The reader defects** yaml#86 and #88. A fix fails aless's
  `tests/yaml_render.rs` until the ledger's lines for it go.
- **transduce#7**, the value before its key. The adapter could refuse
  it with `STREAMABILITY_UNKNOWN`, as it refuses a rewritten map, so
  `--render json` and programs fall back as `--render yaml` does.
- **admin#94**, a plugin schema for the whole descriptor.
- **The tree-contract check** moves into transduce as a shared sink
  when a second host translates.
- **A code for `fail`**, so a render can refuse a broken stream with
  `PROTOCOL_ORDER_ERROR` rather than `INPUT_INVALID`; a language change.
- **`--alchemy` with `--render yaml`** is refused until alchemy can feed
  one program's events into another program's sink.
- **A render that writes from records** is read and not run: aless
  composes the inferred table in front of one when a format ships one.
- **The lift side** waits for the first format whose events do not
  carry its shape, a Markdown table.
- **A native YAML renderer**, if twice the JSON export's time is too
  much: the maintainer's call.
- **TypeScript and Go** read the same parts, named by the manifest, once
  they run alchemy; deferred with the rest of those ports.

### 5.13 parser: a pushed child's snapshot links the pusher after the push

Found registering parser#252 (4.3), and registered in `DIVERGENCE.md`
("A pusher read back through its child's snapshot in Rust",
`pusher-through-child`): the push arm in `rs/src/parser.rs` sets the
child's `parent_rule` to a snapshot of the pusher before it links the
child, and the child's own snapshot, the one the pusher keeps as
`child`, keeps that pre-link pusher. TypeScript and Go link live rules.
Repair direction Rust: link the child's snapshot to the pusher once the
pusher has linked it, then delete the `pusher-through-child` row and
keep the bounded row with the history entry. No fleet grammar reads the
four-hop path. parser#252 listed this fix among what waits on the
maintainer's word on the design, and `DIVERGENCE.md` marks the entry
Deferred, so it waits on the maintainer. When it lands it is an engine
change: the parser's own gates (its Rust gate passes on `c2dd3a6`), and
the Rust dependents run by hand, since the fleet run covers only
TypeScript and Go; built from sibling checkouts of `main`, those wait on
the rename (4.4).

## 6. Conventions and traps

- **Commits:** by path, never `git add -A`; read `git status --short`
  first. Every commit ends with a `Co-Authored-By:` line and the
  `Claude-Session:` line. No model identifiers in code, docs or PR text.
  The fleet's AGENTS.md files hold two core principles:
  - **Dependencies change only on the maintainer's explicit
    instruction.** Adding, removing, re-pointing or re-versioning one is
    a dependency change. If a change would alter a dependency, stop and
    ask. The one standing instruction is "Versions track the latest
    release". tabnas/parser's AGENTS.md spells it out: moving a
    dependency to its latest published version needs no further
    instruction, while holding one back, adding, removing or re-pointing
    one still does. Nothing else is exempt.
  - **Transient tasks report progress,** a status line at least every 30
    seconds.
- **Reviews:** Codex reviews every PR on open. Treat each finding as a
  bug report: reproduce or refute it, fix it with a test that fails
  without the fix, reply once per thread with the commit, and resolve
  the thread.
- **Repositories test each other at their default branches.** A change
  whose test fires by design downstream (gbnf's DIVERGENCE.md 3 pin
  against bnf#80) deadlocks both. The way out is a downstream change
  that accepts both states, as gbnf#49 did, merged first. alchemy's CI
  tries a same-named sibling branch first; most others do not.
- **Some clones fetch only `main`,** so a stop hook reports "no remote
  branch" for pushed branches. Add the branch to `remote.origin.fetch`
  and set its upstream; nothing is unpushed.
- **Sibling lock drift:** with the parser checkout at 0.12.5 and a
  crate's lock at 0.12.4, cargo rewrites `rs/Cargo.lock` on every build.
  Restore it before committing. The fleet's `ci/rust/run.sh` scripts mask
  sibling versions for this reason.
- **Timing tests:** Windows' CPU clock ticks every 15.6 ms. A timing
  assertion repeats each sample in batches of at least 150 ms, takes the
  best of several, and judges ratios, never budgets.
- **aless's dev profile turns the engine's debug assertions off**
  (`[profile.dev.package.tabnas]`). With them on, a stack that grows
  with the input is quadratic. Keep it.
- **The container's disk allowance is fixed** and builds fill it.
  `rs/target` directories of finished work, and scratch copies, are the
  first to delete.
- **Shared CI runners run tests in parallel on two cores.** A deadline
  a debug build meets ten times over here failed there twice: the parse
  of one TOML line took over 100 ms, and a 19 KB JSON row over 500 ms.
  A test that must stop in a later stage gives the earlier stage a
  margin of ten, or makes the earlier stage trivially small (a
  300-number row under cubic per-item work), and proves itself by
  failing when the mechanism it tests is removed.
- **debug's TypeScript suite meets a bnf change at once, not at its
  release:** the shared CI builds the sibling abnf against bnf `main`.
  The same is true of any repository whose CI lists bnf among its
  sibling clones.
- **`cargo update -p X` on aless re-resolves the patched git sources.**
  A crate that names the engine through a `[patch]` table drags the
  engine's pin with it, and one git source carries one revision in the
  lock; moving tabnas-alchemy alone also conflicts on csv against
  transduce. Say in the PR what moved and why.
- **A review workflow's scratch can fill the disk:** one left 13 GB
  and a 7 GB node process reproducing a finding on a merged PR. Stop a
  workflow whose findings have all been acted on, and delete its
  scratch.
- **The pty smoke fails under heavy load** on its first interactive
  step and passes three times out of three when the box is quiet; run
  it again before treating it as a defect.
- **Quoting in `bash -c '…'`** breaks a heredoc holding an apostrophe;
  write commit messages to a file first.
- **The account's session limit stops every subagent at once** (a
  workflow's agents all fail with "session limit", the main session
  goes on): keep the reviewers' drivers in the scratch and verify by
  hand when it hits, as 4.2 did; the limit resets on the hour it names.
- **A PR another session opened cannot be subscribed to** from this one
  (the call fails); a scheduled check-in stands in. Before opening a PR
  for a branch, list the branch's open PRs: the first session's
  handover said "open one" of a PR it had already opened.
- **Cargo refuses `.` and `+` in a binary target name**, tested with a
  scratch crate; a generator that derives a binary name from a language
  id must refuse them first.
- **A crate packaged for crates.io holds nothing outside its own
  directory.** `include_str!` of a file at the repository's root
  compiles in a checkout and fails the verification build `cargo
  publish` runs: embed a copy under `rs/` and hold it to the file with
  a test, as yaml's `rs/translate/` is held.
- **A `scan-emit` state that grows per item is quadratic.** The state
  is measured again in full on every change, and `put` copies the
  record: a render that kept each mapping's keys took 10.9 s for 16,000
  keys. Keep per-item growth out of the state.
- **Several test filters go after `--`:** `cargo test --test agent a b
  c` ran nothing; `cargo test --test agent -- a b c` runs the three.
- **`cargo update -p X --precise <sha>`** fails when another pin
  conflicts with the one it asks for (tabnas-alchemy against csv): move
  the conflicting crate first, then pin.
- **The loaders review's workspace driver** (`lsp.py` in its scratch)
  takes server keys, not paths, and the workspace as an absolute path:
  a relative one finds no manifest and reports nothing served.
