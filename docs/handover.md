# Handover: streaming transducers, alchemy, and the work around them

Status as of 2026-09-28 14:50 UTC. This records the state of one working
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

Requests that came in the second session (2026-09-28, from 12:50 UTC):

- **"Focus on the Rust implementation; TypeScript and Go are
  deferred."** Every port and release that only TypeScript and Go
  consumers need waits for a new instruction: the bnf npm and Go release
  (5.1), the TypeScript and Go ports of the debug renderer (5.6), and
  the Go engine release that bnf's `go.mod` would take. The Rust chain
  needs none of them: aless, gbnf and debug's Rust crate take bnf by git
  revision or sibling path.
- **A design question, discussed and not started:** how aless will
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
  architecture document were proposed. The maintainer has not decided.

## 2. What was built

| Repository | What it is | Read first |
|---|---|---|
| tabnas/transduce | `tabnas-transduce`: protocols, the event source (whole walk, incremental from rule events, line-chunked JSON Lines and CSV), router and matcher, captures, `ScanEmit`, `TableFromJson`, limits, stable error codes | `docs/architecture.md` (the canonical design), `docs/reference.md`, `docs/BENCH.md` |
| tabnas/render | `tabnas-render`: text algebra (`TextOut`, `WriteOut`, `StringOut`, `Join`, `ReplaceText`, `Concat`), the always-quoted CSV renderer, the JSON renderer, `RecordsToJson`, protocol validators | `docs/reference.md` |
| tabnas/alchemy | `tabnas-alchemy`: the language reader as a tabnas grammar plugin; desugar, resolve, check, effects and `explain`; the evaluator that builds plans; lowering onto transduce and render; the standard library; the `alchemy` binary (`canon`, `format`, `check`, `explain`, `run`) | `docs/language.md`, `AGENTS.md`, `rs/README.md` |
| rjrodger/aless | `--render csv\|json` streams a document's records through the transducer (aless#10); `--grammar NAME=FILE` and `--grammar-expr` read any text format through a custom ABNF grammar (aless#11); `--alchemy FILE`, `--alchemy-expr TEXT` and `--explain` run a program over the input (aless#13); a custom grammar runs under the shared depth cap (aless#12) | README "Scripts and agents", `skills/aless/SKILL.md` |
| tabnas/debug | `rs/src/abnf.rs`: the ABNF emitter reads the bnf#80 repeat loop by shape and renders it as `*A` / `*( a b )` / `1*A` (debug#63, open) | `docs/reference.md`, "The repeat loop: the Rust port leads" |
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
in aless, alchemy, parser, gbnf and debug, and finished bnf#80 on its
original branch. Merges use merge commits titled
`Merge pull request #N: <title>`.

| Repository | Merged | Open |
|---|---|---|
| tabnas/transduce | #1 the crate, #2 and #3 repetition-rule guide notes | none |
| tabnas/render | #1 the crate | none |
| tabnas/alchemy | #1 the reader, #2 phase 3 (`4c9c2a2`), #3 the reader's repetitions as `r:` loops (`f67942d`), #5 a lone carriage return at a line start is whitespace (09-28 12:08, `e9e4daf`) | this handover |
| rjrodger/aless | #10 `--render`, #11 custom ABNF grammars (`0cc8a94`), #12 a grammar from the command line runs under the shared depth cap (09-28 12:53, `5e2b7bf`), #13 `--alchemy`, `--alchemy-expr` and `--explain` (09-28 14:45, `2eec30f`) | none; the branch is at main |
| tabnas/parser | #245, #246 guide notes; #248 Go builtins append `src` in amortized constant time (`8e60c46`); #249 TypeScript `@capture$` and `@fold$` append a child's kids one at a time, over the count the child had (09-28 12:36, `d8d00c3`) | none |
| tabnas/bnf | **#80** every repetition compiles to a same-depth replace loop (09-28 12:40, `d0a9227`) | none; the release is deferred (5.1) |
| tabnas/abnf | #97 guide note | none |
| tabnas/ebnf | #45, #46 guide notes | none |
| tabnas/gbnf | #47, #48 guide notes; #49 the DIVERGENCE.md 3 pin follows the compiler (`5ee4ff3`); #50 close DIVERGENCE.md 3 and hold the linear side as a standing check (09-28 12:51, `5c24c4d`) | none |
| tabnas/debug | none | **#63** the Rust emitter renders the repeat loop (head `cdfd35b`); Rust, Go and prose gates green, the three `ci / ts` jobs red on every PR in the repository since bnf#80 merged (see 4.1 and 5.6) |
| tabnas/lsp | #20 the semantic-token crate (`b6339f9`) | none yet: the full port is 16 commits on the first session's branch at `6688f76`, untouched since (see 4.2) |

Another session, not these, drove the parser 0.12.5 release and the
yaml, ini and csv releases after it. Leave those alone.

## 4. In flight at the time of writing

### 4.1 tabnas/debug#63: the Rust emitter renders the repeat loop

- **Head:** `cdfd35b`, five commits: `43b4a14` the emitter, `796cc08`
  prose, `bf03bca` the two defects its own verification found (a
  repetition over an optional emitted an empty `[  ]`; `1*A` written as
  `A *A` did not round-trip on a nullable item, so it is written back as
  `1*A`), `6714c7f` Codex's three findings (the loop entry is the whole
  compiler shape with its `n.rep == 0` guard and counter; only that
  entry is an empty self-replace; a loop's helpers bounded), `cdfd35b`
  the helpers bounded by kept productions after the name bound stopped
  inlining a group's own `$alt` chain. Every Codex thread is answered
  and resolved; the PR body has the tests, the differential figures and
  the follow-ups.
- **Verification:** two verifiers on the first commit (a 500-grammar
  differential through the Rust abnf crate on the new and the old
  compiler, round-tripped through the emitter and recompiled; a
  conformance and regression review against a build of `main`), and a
  refuter with the differential re-run on each later commit. On
  `cdfd35b`: RFC-shape failures 0, recompile failures 0, recognition
  mismatches 87 of 500, the same finding set as before the Codex round,
  every one pre-existing and none involving the loop. The harness lived
  in the session's scratch and is gone: it was a scratch crate over
  `/home/user/parser/rs`, `/home/user/debug/rs` and the Rust abnf crate,
  generating random ABNF, compiling, installing the debug plugin,
  calling `abnf()`, checking the two RFC 5234 rules, recompiling and
  comparing recognition on random inputs. Rebuild it the same way if a
  further change to the emitter needs it.
- **What blocks the merge:** the three `ci / ts` jobs. debug's shared CI
  builds the sibling abnf against bnf `main`, whose compiler now emits
  the loop, and the TypeScript emitter still lists the loop's entry as
  one of the rule's own alternatives, so `round-trips repetition` and
  `keeps repetition as a production` in `ts/test/abnf.test.js` fail, on
  `main` too from its next run. The TypeScript port of the Rust change
  is deferred by the maintainer's instruction. The choice is the
  maintainer's: lift the deferral for this one port (the design and the
  Rust implementation in `rs/src/abnf.rs` are the template, and the
  earlier TypeScript-first attempt left `docs/reference.md`'s section as
  the spec), or merge the Rust PR over the red TypeScript jobs, or leave
  it open. Nothing else in the fleet depends on it.

### 4.2 tabnas/lsp: the full Rust port

Untouched in the second session. The state below is the first session's.

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

Items 5.1 to 5.4 and 5.7 are done; what remains of each is listed.
The Rust-only instruction (section 1) orders the rest: 5.6's TypeScript
port waits on the maintainer, 5.5 is Rust engine work, 5.8 is Rust
grammars first, 5.9 is aless.

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
  re-run, a robustness gap in that test. aless's lock pins alchemy at
  `2bc4beb`, before alchemy#5; `cargo update -p tabnas-alchemy` alone
  conflicts on the engine and csv pins, so a coordinated move is a small
  PR of its own when wanted.

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

Measured again on 09-28 in aless#13 (above): the program path is linear
in the document and costs about half again the plain renderer's time on
JSON; peak memory is the engine's rule history on every path, 2.8 GB
for 25 MB. The plan here stands, and it is Rust engine work, within the
Rust-only instruction.

### 5.6 tabnas/debug: render replace loops correctly

The Rust port is done in debug#63 (4.1). What it does, so the TypeScript
and Go ports can follow it line for line (the canonical is
`ts/src/debug.ts` `emitAbnf`; `docs/reference.md`'s section "The repeat
loop: the Rust port leads" records that the port leads until then):

- `has_content` is decided by what an alternative consumes (a token past
  its backtrack, a push, or a replace with another rule); `{ }`, the
  FOLLOW peek and, on a loop, the entry `{ r: H }` are epsilon. Any
  other self-replace keeps its content (`one = A [ one ]`).
- A rule whose open alternatives carry the whole entry shape (no token,
  no push, `r` itself, the `n.rep == 0` guard, the counter set to 1) is
  a loop, rendered wherever referenced as `*A` / `*( a b )` over its
  continue alternatives; its back edges render nothing; it and its
  helpers are never productions. The helpers are everything the
  iteration reaches short of a kept production (a user rule, another
  loop, an old-shape repetition helper), so a group's own `$alt`/`$step`
  chain inlines and an old push-chain star inside stays a production.
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
`rs/tests/debug_model_test.rs` pins `line = form IN block`); survey and
run them before a port merges. Also found by the differential and not
this PR's: a case-insensitive ABNF literal (`"a"`) reaches the Rust
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

Found in the second session, also not yet filed:

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
