# Handover: streaming transducers, alchemy, and the work around them

Status as of 2026-09-28 09:45 UTC. This records the state of one working
session across the tabnas fleet and rjrodger/aless, so that whoever picks
it up can continue without the session's scratch files, which do not
survive it. Everything a later step needs is written here or linked from
here.

Read sections 1 to 3 for orientation, section 4 for what was running when
this was written, and section 5 for the next work in order. Section 6
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
  - Not started; see 5.9.
- **A full Rust port of tabnas/lsp,** used directly as a Rust dependency
  of aless. Nearly done; see 4.2.
- **The repetition rule:** "Star repetitions must always be alt.r not
  alt.p. This is fundamental. Make sure you have noted it in the agent
  guides." Noted in every affected guide. The compiler fix is bnf#80;
  see 4.1.

Standing instructions:

- Any tabnas repository may be cloned, and pull requests opened for minor
  bug fixes.
- Push often.
- Open pull requests ready for review, never as drafts.
- Merge green pull requests.
- Continue to completion.

## 2. What was built

| Repository | What it is | Read first |
|---|---|---|
| tabnas/transduce | `tabnas-transduce`: protocols, the event source (whole walk, incremental from rule events, line-chunked JSON Lines and CSV), router and matcher, captures, `ScanEmit`, `TableFromJson`, limits, stable error codes | `docs/architecture.md` (the canonical design), `docs/reference.md`, `docs/BENCH.md` |
| tabnas/render | `tabnas-render`: text algebra (`TextOut`, `WriteOut`, `StringOut`, `Join`, `ReplaceText`, `Concat`), the always-quoted CSV renderer, the JSON renderer, `RecordsToJson`, protocol validators | `docs/reference.md` |
| tabnas/alchemy | `tabnas-alchemy`: the language reader as a tabnas grammar plugin; desugar, resolve, check, effects and `explain`; the evaluator that builds plans; lowering onto transduce and render; the standard library; the `alchemy` binary (`canon`, `format`, `check`, `explain`, `run`) | `docs/language.md`, `AGENTS.md`, `rs/README.md` |
| rjrodger/aless | `--render csv\|json` streams a document's records through the transducer (aless#10); `--grammar NAME=FILE` and `--grammar-expr` read any text format through a custom ABNF grammar (aless#11) | README "Scripts and agents", `skills/aless/SKILL.md` |
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

All work uses the branch `claude/streaming-transducer-render-kzyqua` in
every repository. Merges use merge commits titled
`Merge pull request #N: <title>`.

| Repository | Merged in this effort | Open |
|---|---|---|
| tabnas/transduce | #1 the crate (09-27 14:44), #2 and #3 repetition-rule guide notes | none |
| tabnas/render | #1 the crate (09-27 14:47) | none |
| tabnas/alchemy | #1 the reader (09-27 14:49), #2 phase 3: checker, planner, interpreter, standard library (09-28 00:29, `4c9c2a2`), #3 the reader's repetitions as `r:` loops (09-28 01:16, `f67942d`) | this handover |
| rjrodger/aless | #10 `--render` (09-27 15:13), #11 custom ABNF grammars (09-27 23:50, `0cc8a94`) | none; the branch is at main |
| tabnas/parser | #245 and #246 repetition-rule guide notes; #248 Go builtins append `src` in amortized constant time (09-28 00:42, `8e60c46`) | none of ours. #247, release 0.12.5, was another session's |
| tabnas/bnf | none yet | **#80**, every repetition compiles to a same-depth replace loop (see 4.1) |
| tabnas/abnf | #97 guide note | none |
| tabnas/ebnf | #45 and #46 guide notes | none |
| tabnas/gbnf | #47 and #48 guide notes; #49 the DIVERGENCE.md 3 pin follows the compiler it is built against (09-28 00:55, `5ee4ff3`) | none; follow-up in 5.2 |
| tabnas/lsp | #20 the semantic-token crate (09-27 18:11, `b6339f9`) | none yet: the full port is 16 commits on the branch at `6688f76` (see 4.2) |

Another session, not this one, drives the parser 0.12.5 release and the
yaml, ini and csv releases that follow it. Leave those alone.

## 4. In flight at the time of writing

Both items below were being worked by background workflows that end with
the session. Treat each as a checklist to finish by hand if nothing has
landed.

### 4.1 tabnas/bnf#80: every repetition compiles to a same-depth replace loop

- **Head:** `4403532`. Every CI check on it is green, including the
  downstream `rust (abnf)`, `rust (ebnf)` and `rust (gbnf)` jobs and the
  TypeScript and Go downstream suites.
- **The shape.** A star `*A` is now a loop rule `H`:
  - Its open alternatives are:
    - an entry `{c:{n.rep:0}, n:{rep:1}, r:H}` that allocates the node;
    - a continue `{s:FIRST(A), b:1, r:H$alt0}`;
    - an exit on the follow token;
    - an empty alternative.
  - `H$alt0` pushes the item with `n:{rep:0}` and replaces to
    `H$alt0$step1`, which replaces back to `H`.
  - `1*A` is `A` then the star. Rule names are unchanged, which abnf's
    `test/spec/alignment-abnf-rules.tsv` pins. A terminal item needs only
    the loop rule.
  - The shape is in TypeScript (`a97ae7c`), Go (`10c073d`, `c47558d`,
    `a264805`) and Rust (`7ade26e`, `4403532`).
- **The evidence.** In all three runtimes, depth tests assert that the
  deepest rule over 10,000 items equals the deepest over one. The parse
  time is linear. The new TypeScript compiler was compared with the
  0.1.22 push-chain compiler on 6,300 random grammars and about 176,000
  inputs, in both modes, with no difference in verdict or value.
- **Still to do before merging:**
  - The two reviews (parity and fleet, untrusted input) and their fixes.
  - Rewrite the PR's title and body. The title still reads "docs:
    repetition is replacement, never a push chain", from the guide note
    that opened the PR. It should become "Every repetition compiles to a
    same-depth replace loop", and the body should state the rule, the
    shape before and after, the depth evidence per runtime and the
    downstream results.
  - Answer the one open Codex thread (it says the loops were not
    implemented; name the commits that implement them) and resolve it.
  - Then merge.
- **Known and not fixed here: the `@tabnas/debug` round trip.** The
  loop's entry alternative consumes nothing and names the loop, so
  debug's ABNF renderer lists the loop as one of its own alternatives
  (`r-gen1-star-A = [ r-gen1-star-A / r-gen1-star-A-alt0 ]`). Recompiled,
  that grammar rejects inputs the original accepts. The fix belongs in
  tabnas/debug (5.6), not in the compiler.
- **After merging:**
  - A bnf release is needed before TypeScript and Go consumers see the
    loop, since they resolve bnf from npm and the Go proxy. Rust
    consumers take bnf's main by path. The release process is in
    tabnas/parser's AGENTS.md, "Releasing"; dispatch `release.yml`.
  - Then 5.2 and 5.3.

### 4.2 tabnas/lsp: the full Rust port

- **Done:** the modules and a parity stage on the branch at `6688f76`,
  16 commits above main.
  - Every section of `test/fixtures/lsp-conformance.json` passes in all
    three runtimes, including a new outlines section.
  - A random differential sweep against the TypeScript core
    (`a330a6a`), and one registered divergence in the block-comment case
    of completion (`78ab481`).
  - Two cargo features, both off by default. `dialects` links abnf, ebnf
    and gbnf. `fleet` links csv, feed, ini, json, json5, jsonc, jsonic,
    jsonl, toml, xml, yaml and zon.
  - `ci-rust` clones 18 siblings.
- **Still to do:**
  - Two reviews. One drives the binary through real LSP sessions
    (malformed and out-of-order messages, huge documents, didChange
    bursts, both position encodings, a hot reload, a quarantined
    grammar) and compares with the TypeScript server. The other attacks
    the loaders and the firewall with hostile specs and grammars.
  - Fix what they find.
  - Open ONE pull request from the branch, ready for review, describing
    the modules, the features, the generator target, parity per runtime
    and what is left. Then drive it green and merge it.
- **Flagged by the port's foundation:**
  - admin's `rollout/workflows/lsp__ci.yml` predates the `ci-rust` job
    and needs a sync by the maintainer.
  - The Rust engine keeps `is_builtin_action` crate-private, so the
    loader's firewall relies on the engine's own rejection of an unknown
    `@` action.
  - `Makefile` and `AGENTS.md` should say that `rs/` is the whole
    pipeline, if the port's last commit did not already.

## 5. Next work, in order

### 5.1 Finish and merge bnf#80

See 4.1.

### 5.2 gbnf: close DIVERGENCE.md 3

gbnf's Rust port parsed a repetition in quadratic time because of the
push-chain compiler. gbnf#49 made the pin
(`a_repetition_is_super_linear_until_the_compiler_emits_a_loop` in
`rs/tests/divergence_test.rs`) read the emitted rules:

- with no replacing alternate, it asserts the quadratic side;
- with the loop, it asserts that 10,000 items reach the depth of one and
  that four times the input costs under eight times the work.

gbnf's CI clones bnf's main, so once bnf#80 merges the pin takes the
linear side by itself. The follow-up is cleanup, on the gbnf branch reset
to main:

- Delete DIVERGENCE.md 3, its pin and its row in the pin table.
- Raise the ceilings in `rs/tests/untrusted_test.rs`. For example,
  `a_long_input_parses_without_hanging` from 1,000 and 2,000 characters
  to five figures, and the deep-nesting comment that cites entry 3.
- Correct `rs/AGENTS.md`, which says the suite is slow and input length
  bounded "because of DIVERGENCE.md 3".

### 5.3 aless: take the fixed compiler and restore the shared depth cap

aless#11 carries a workaround, named as one in AGENTS.md and
`tests/fixtures/grammars/README.md`. A `Format::Custom` parse runs under
`MAX_CUSTOM_RULE_DEPTH` (1,000,000 open rules), and nesting is measured
on the built value (`MAX_VALUE_DEPTH`). The old compiler spelled `*entry`
as a rule calling itself per item, and 1,500 lines of `hosts` were
refused.

Once bnf#80 is on bnf's main, on the aless branch:

1. Run `cargo update -p tabnas-bnf`; move abnf too if it has moved. Only
   those packages may change in `Cargo.lock`.
2. Return the custom cap to the shared `MAX_RULE_DEPTH` (3,000).
3. Flip `a_custom_grammars_repetition_is_not_nesting` to assert constant
   depth. Add a test that a 10,000-line `hosts` file parses under the
   shared cap, with the same maximum rule depth as a 3-line file.
4. Rewrite the workaround paragraphs as history.
5. Run the four gates (`cargo fmt --all --check`, `cargo clippy
   --all-targets --locked -- -D warnings`, `cargo test --locked`,
   `python3 scripts/pty-smoke.py target/debug/aless`), then open the PR.

This and 5.4 share the aless branch: land one before starting the other.

### 5.4 aless: `--alchemy`, `--alchemy-expr` and `--explain`

This is the part of the original request still open. `--render` exists
(`src/export.rs`); the program surface does not. The plan below was
agreed before alchemy phase 3 merged. Check alchemy's merged API
(`rs/src/program.rs`, `rs/src/bin/alchemy.rs`) against it first.

- **Dependencies:** add `tabnas-alchemy = { git =
  "https://github.com/tabnas/alchemy" }` with a
  `[patch."https://github.com/tabnas/alchemy"]` table redirecting its
  path dependencies (`tabnas`, `tabnas-transduce`, `tabnas-render` and
  `tabnas-json`; `tabnas-debug` is dev-only) to their git repositories, as
  the existing tables do for transduce and render.
- **Command line:**
  - `--alchemy <FILE>` and `--alchemy-expr <TEXT>` name the program;
    giving both is a usage error.
  - `--render csv|json` is the renderer for a table or event result.
  - `--explain` prints the plan report instead of running.
  - With `--json`, `--paths`, `--path` or `--at` it is the existing "both
    say what to print" usage error; the program does the selecting.
  - Add a block to `--help` under WITHOUT A SCREEN.
- **Library** (terminal-free, unit tested):
  - Input source: JSON Lines and CSV with a root-elements row selector
    read line by line, so the file is never read whole. Otherwise the
    `ParserSource` in incremental mode where
    `capability::incremental(format)` says so, and whole otherwise. Use
    the same fallback to the whole walk that `--render` already has, and
    the same `load::make_parser`, guards and `--timeout`.
  - `Format::Text` is a usage error.
  - Output goes through `tabnas_render::WriteOut` over a locked, buffered
    stdout. A write failure is `OUTPUT_FAILED`, status 3.
  - The result protocol decides the renderer. Text is written as is.
    TableRows uses `--render` or csv. JsonEvents uses `--render` or json.
    `--render` given for a Text program is a usage error.
- **Errors** keep aless's `{"error": {...}}` shape:
  - `kind: "alchemy"` for `DSL_PARSE_ERROR`, `DSL_TYPE_ERROR`,
    `STREAM_REUSED` and `STREAMABILITY_UNKNOWN` (status 2), with `code`,
    `message`, `file` and `line`/`col`.
  - `kind: "transduce"` for the rest, with `code`, `message`, `file`,
    `path`, `limit` and `output` (`"partial"` or `"none"`).
  - `RESOURCE_LIMIT_EXCEEDED` is 5, `ABORTED` (timeout) is 6,
    `OUTPUT_FAILED` is 3, and the input and protocol codes are 1.
- **Docs:** the four agent-interface sites must agree: the README's
  "Scripts and agents" section, `--help`, `skills/aless/SKILL.md`, and
  the tests in `src/headless.rs` and `tests/agent.rs`. Fields may be
  added, never renamed.
- **Tests, end to end:**
  - The design's worked example (`tests/fixtures/records.json` plus the
    `export` program) gives exactly its 77 bytes, CRLF and all quoted.
  - `--render json` round-trips with `--json --compact`.
  - `--explain` prints a JSON object.
  - Each usage error.
  - A DSL parse error gives status 2 with `line`/`col`.
  - Rows before the metadata give `INPUT_ORDER_VIOLATION` with `output:
    "none"`.
- **Acceptance:** a large export (see 5.5), with its throughput and peak
  memory reported in the PR.

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
Start with a profile of the Rust port on the 24.6 MB records file, write
the design in tabnas/parser's DIVERGENCE and ADR terms, and land it
TypeScript first. The transduce benches (`cargo bench` in `rs/`) and
aless's `--render` acceptance runs are the measurements to repeat
afterwards, including a 2 GB JSON Lines export, which the line-chunked
source already bounds in memory.

### 5.6 tabnas/debug: render replace loops correctly

debug's ABNF emitter (`ts/src/debug.ts` `emitAbnf`, canonical, with twins
in `go/` and `rs/src/abnf.rs`) has two gaps.

- **It renders a loop's entry alternative as content.** An alternative
  that consumes no tokens and replaces the rule with itself is listed as
  an alternative of its own rule. That is bnf#80's round-trip failure.
  The renderer should skip such an alternative, and ideally fold the
  bnf loop shape (`H`, `H$alt0`, `H$alt0$step1`) back to `*( A )`.
- **"Content" should mean consuming input.** `hasContent` counts an
  alternative with any `s` as content even when `b` backtracks all of
  it. So `{s:#DE, b:1}` does not make a close continuation optional, and
  `closeCont` renders only the first content alternative.
  - Because of this, alchemy's reader now renders as `line = form IN
    block` where `line = form [ IN block ]` is meant.
  - json's `elem` close (`{s:#CA, r:elem}` beside `{s:#CS, b:1}`)
    renders as a mandatory continuation.
  - Seqs already count consumed tokens (`len(s) - b`); `hasContent` and
    the epsilon test should do the same.

Coupling:

- Six Rust crates depend on debug by path: alchemy, expr, feed,
  multisource, semver and zon.
- About twenty TypeScript packages use `@tabnas/debug`, and jsonic's Go
  module uses its Go port.
- Some of them pin rendered text. alchemy's `rs/tests/debug_model_test.rs`
  pins `line = form IN block`.

Before merging:

1. Survey every dependent's tests for pinned renderings.
2. Run their suites against the fixed debug.
3. Prepare the follow-ups.

alchemy's CI first tries to clone each sibling at the PR's own branch
name, so a debug fix and alchemy's test update pushed on the same branch
name test together.

### 5.7 alchemy: a lone carriage return at the start of a line

- **The bug:** a layout line whose first character is a lone `\r` (not
  followed by `\n`) counts as a line to the layout matcher but as a space
  to the engine's lexer. The matcher emits `#NL` with no form after it.
- **Consequences:**
  - `line.close` needs an extra alternative, `{ s: ["#NL", "#DE #ZZ"],
    b: 1 }`, to keep inputs like `"f x\n\r"` parsing.
  - A block indented under such a line becomes a raw array item.
    `"a\n  b\n\r;x\n  c"` fails with "malformed reader output", an
    internal shape error that reaches the user.
  - The unit test `a_layout_line_with_no_form_on_it` pins the current
    behaviour on purpose.
- **Earlier related fix:** positions already treat a lone `\r` as a line
  ending (alchemy#1's review).
- **The fix:** choose one treatment across the layout matcher, the
  lexer and positions, so that no input yields "malformed reader output".
  Remove the extra alternative if it becomes unnecessary. Replace the
  pinning test with tests of the fixed behaviour. Compare old and new
  over the fixtures, the docs' code blocks, `stdlib/*.alc` and generated
  inputs with lone `\r`s: only inputs containing a lone `\r` may change.

### 5.8 The fleet's repetitions against the rule

A read-only audit classified every repetition in the local checkouts as
a replace loop, a push from the close state (constant depth, but the rule
broken in letter), or a push chain (depth grows with the item count). It
measured depth at 1 and 10,000 items.

| Repository | Result |
|---|---|
| json, jsonl, jsonc, csv, xml, markdown, zon | every repetition a replace loop, ports agree |
| json5, feed, path, hoover | define no rules of their own |
| directive, multisource | rules, but no repetitions |
| jsonic | replace loops, except path-dive keys (`a:b:c:1`): depth grows with the key count, but so does the value's nesting, so this is structure, not a repetition. Confirm and record it as such. |
| yaml | the same path dive, inherited from jsonic (`a: b: c: 1`) |
| ini | `dive` (dotted section path `[a.b.c]`) is a push chain: depth 4 for one segment, 128 for many. The value nests too; decide whether this is structure or a loop to convert. |
| toml | `dive` (dotted key `a.b.c = 1`) is a push chain: depth 4, then 10,002. Same question as ini. |
| semver | 21 push chains, all ABNF repetitions compiled by the old bnf (depth up to 20,009). They are fixed by regenerating with bnf#80's compiler once released. |
| chess | the game's tag section is a push from the close state (`{s:#OS, p:tag, b:1, c:@more-tags}`): constant depth, but convert it to an `r:` loop |

Not audited, because the run stopped on a usage limit: css, c, proto,
expr, abnf, ebnf, gbnf, parser, debug and support. The suspects above
also still need their independent verification. Each fix in a grammar
repository is its own PR, TypeScript first, with the depth test that
proves the rule. The rule's text is in every affected repository's
AGENTS.md, "Repetition is replacement, never a push chain".

### 5.9 aless: ratatui, panes and highlighting

Three pull requests, in this order.

**1. Adopt ratatui as the drawing layer, with no new behaviour.**

- **Version and minimum Rust:** ratatui 0.30 with the `crossterm_0_29`
  feature needs Rust 1.88. Raise `rust-version`, the CI job "rust 1.86
  (minimum supported)", the README and AGENTS.md, and say why in the PR.
- **Port:**
  - aless's `Style`, `Color`, `Span` and `Line` become ratatui's.
  - `render(app)` becomes `draw(frame, app)`, composing widgets: the
    tree pane, the source view, the error panel, the status bar, the
    prompt and the overlay.
  - `paint` is replaced by `terminal.draw`.
- **Keep:**
  - `sanitize` at every point text enters a `Span`.
  - The horizontal `skip` of the focused row.
  - aless's own panic hook (not `ratatui::init`).
  - The key map, mouse support, and the headless contract.
- **Tests:** a `screen_text` shim over `TestBackend` keeps every existing
  render, app and app_flow assertion text. Add a CJK and emoji
  column-arithmetic test.

**2. Panes.**

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

**3. Highlighting through tabnas/lsp's Rust crate.**

- **How:** `tabnas_lsp::highlight` takes the engine's lex trace, keeps
  the newest token at each position, and maps each token to a fixed
  nine-type legend. The mapping comes from the canonical table, the
  prefix conventions and per-grammar overrides.
- **Where:** in every pane's Source mode: the tab's grammar, the output
  re-lexed with csv or json, the program with alchemy's grammar, and
  custom grammars. Cache per text and grammar.
- **Fallback:** grammars whose registry entry says their lex stream is
  speculative keep aless's value-kind colouring.

### 5.10 Upstream defects and candidates, not yet filed

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

## 6. Conventions and traps

- **Commits:** by path, never `git add -A`; read `git status --short`
  first. Every commit ends with a `Co-Authored-By:` line and the
  `Claude-Session:` line. No model identifiers in code, docs or PR text.
  The fleet's AGENTS.md files hold the dependency rule (dependencies
  change only on instruction, except moving to the latest release) and
  the progress rule (a status line at least every 30 seconds).
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
