# Handover: dmog, the review of the `whole` formats, and the css, ini and toml parts, 2026-10-25 to 2026-11-02

This is the dated working record of one session across the tabnas fleet,
written on 2026-11-03 for whoever picks the work up. The session ran as a
Claude Code cloud session on the maintainer's account, with the fleet's
repositories cloned beside each other; its container is gone with it, so
everything a later step needs is written here or linked from here. It
continued the wave of ADR-31 ([`docs/streaming-plan.md`](streaming-plan.md),
section 12): the maintainer's ruling of 2026-10-24 that a part has one
implementation, its alchemy text, and that aless no longer carries the
streaming exports. The previous handover, on the streaming plan's steps
10, 12 and 13, is admin's
[`notes/2026-10-23-aless-streaming-handover.md`](https://github.com/tabnas/admin/blob/main/notes/2026-10-23-aless-streaming-handover.md).
As a dated record this file names each thing as it was named when the
work was done: the command and its packages are dmog, and
tabnas/alchemy-cli is the repository's GitHub name until the maintainer
renames it.

## The maintainer's rulings

On 2026-10-25:

1. **alchemy-cli is dmog, data mogrify.** The utility that performs the
   data format streaming aless no longer carries is renamed across the
   fleet; alchemy, the language, keeps its name. Recorded as admin ADR-32
   in [`DECISIONS.md`](https://github.com/tabnas/admin/blob/main/DECISIONS.md).
2. **Review alchemy's suitability** as that utility's language. The
   review was given in the session and written nowhere else; it is
   summarised below.
3. **Add tabnas repositories to the session as needed**, and **merge and
   report**: the session merges its own green pull requests and reports.

On 2026-10-26 and 2026-10-27:

4. **Any to any.** Asked whether alchemy now translates any format to any,
   with each package exporting the alchemy text of its transducer and its
   render: yes, through dmog's `translate`, for every format whose package
   carries a `translate` manifest, composed from the parts each exports,
   with the translation matrix as the proof; twelve formats were then read
   whole.
5. **Why `whole`?** Examine each of the twelve `whole` manifests against
   ADR-31's test: does the sentence say why the format's events cannot
   come from its rule passes, or only that no one had written the part?
   The review is section 12.1 of the streaming plan.
6. **Stream where possible.** Write the parts the review found possible:
   css, then ini and toml. feed waits on the language.

On 2026-11-02:

7. **Merge and report**, the parts: css#73, ini#110, toml#114 and
   alchemy#112 were merged; dmog's #63 was blocked on released modules,
   and reported so.
8. **Release css, ini and toml**, so dmog takes the parts in Go and
   TypeScript and alchemy's fixture rows can exist.
9. **Write a handover document in alchemy**: this file.

## What landed

Every pull request below is merged, except admin#160, which waits for the
GitHub rename (under "Open").

| Wave | Pull request | Commit | What |
|---|---|---|---|
| rename | tabnas/alchemy-cli#62 | `cbf3cb1` | The command is `dmog`; the packages are `@tabnas/dmog`, `github.com/tabnas/dmog/go` and `tabnas-dmog`. The workflow files are a patch on the pull request, since the session's credential cannot write them. |
| rename | tabnas/alchemy#111 | `916feb6` | The guide, the README, the language reference, the fixture guide and the comments name dmog; `downstream.yml`'s change is a patch on the pull request. |
| rename | rjrodger/aless#98 | `6a9cfe2` | The README, the guide and the skill point at dmog. |
| rename | tabnas/admin#159 | `c585b6c` | ADR-32; `publish.sh`'s `ORDER`, the trusted-publisher lists and the workflow templates (`dmog__*.yml`). |
| rename | tabnas/web#60 | `f069a26` | The package registry lists dmog. |
| rename | tabnas/transduce#72 | `a819324` | The documents name dmog as the host that runs the parts. |
| rename | tabnas/render#30 | `4669084` | The documents name dmog as the host that runs the text. |
| rename | tabnas/admin#160 | open | `docs/deps` regenerated with the repository named dmog; its `port-deps` check is red until the GitHub rename. |
| parts | tabnas/css#73 | `a99b094` | `css-transduce`: a stylesheet a statement at a time; an at-rule token's value is the node's head. Codex's finding on the review, an empty at-keyword's `""` member dropped, was fixed before the merge. |
| parts | tabnas/ini#110 | `ed53c46` | `ini-transduce`: a document a member at a time; the lexer marks an array key `k[]`. Codex's finding, the lookup of an existing `k[]` member lost in all three runtimes, was fixed with rows in `test/spec/arrays.tsv`; the part refuses `k[][]`. |
| parts | tabnas/toml#114 | `a96584b` | `toml-transduce`: a document a member at a time, a frame per open table. Codex's finding, a quadratic prefix comparison and a reversed vector per header, was replaced by linear forms. |
| parts | tabnas/alchemy#112 | `fa75a3b` | Section 12.1 of the plan: the review of the twelve `whole` manifests. |
| release | tabnas/css#74 | `3c656a8` | css 0.5.17, the first release whose Go module has `Translate().Transduce`. |
| release | tabnas/css#75 | `e03fdf9` | The Go lexer builds an at-rule's head as `*tabnas.OrderedMap`, since a plain map reached the part in no fixed order. |
| release | tabnas/css#76 | `da19f69` | css 0.5.18, with that fix. |
| release | tabnas/ini#111 | `87f779f` | ini 0.5.21. |
| release | tabnas/toml#115 | `85fd9eb` | toml 0.5.19. |
| dmog | tabnas/alchemy-cli#63 | `05cc637` | Rust and Go read css, ini and toml through their parts; `go.mod` moves to the three releases and nothing else moves; the `formats` tests hold each format to the package the build has. |
| rows | tabnas/alchemy#113 | `7cfff11` | Thirteen rows in `test/spec/transduce.tsv` (css 4, ini 5, toml 4), and the plan's note that they exist. |

The npm packages, Go modules and crates of css 0.5.17 and 0.5.18, ini
0.5.21 and toml 0.5.19 were published by each repository's `release.yml`,
dispatched on `main` with `go` true; the `ts/v` and `go/v` tags, the
crates.io versions and the Go proxy's records were checked after each.
dmog itself is not released under its new names (under "Open").

## alchemy as dmog's language: the review

Written on 2026-10-25 from the language reference, the library, the
fixtures, transduce's `docs/BENCH.md` and the streaming plan. The verdict:
alchemy is the right engine for a streaming any-to-any converter, it is
less suited as a general transformation language for dmog's users, and
the interpreter's speed, not the language's design, is what limits dmog
on large inputs.

What it does well for the purpose: the streaming guarantee is static
(affine streams, no stream captured by a `fn`, every retained value
measured against `Limits` as it is built, strict mode refusing what it
cannot analyse, `explain` saying what is retained and under which limit);
one text runs in three runtimes, held byte for byte by the fixtures and
the matrix, where a fleet of 22 formats would otherwise maintain 66
transducers and 66 renderers natively; a part reads the rule passes, not
a tree, so `transduce-tree` makes a tree-shaped format a four-line
declaration; the parts compose, and `translate --with` puts a program
between the source and the render under the same limits; no I/O, no eval
and bounded evaluation, so dmog can run programs it did not write.

Where it falls short: throughput (an interpreted part costs three to four
times the bare parse on the plan's measurements; the lever ADR-31 leaves
is the evaluator, where a pass record is materialised for every pass
though most specs read one name and one token); the primitives the
library bends around (no `+`, only `inc`; `let` binds one name; no
flatten, so `transduce-records` is two chained scans; no `get-or`, `sort`,
`starts-with`, `contains`, trim or first-split; an optional field read by
nested `match ... case missing`); ergonomics for a command-line user
(every program is a file with `def export [input]`, and there is no
expression form); the streaming from a reader is Rust's today, where
TypeScript and Go read whole; and one pass over one source, by design.

Recommended: keep alchemy as dmog's only language, with neither a second
language nor a native path; make the evaluator's per-pass cost the next
performance target, against `docs/BENCH.md`; add as natives, with fixtures
in all three runtimes, `flatten`, a multi-binding `let`, `get-or`, exact
`add` and `sub`, `sort` over a finite vector, `starts-with`, `ends-with`,
`contains`, a trim and a first-split; give dmog an expression form
(`dmog run -e`); and state the contract on dmog's README: one pass,
bounded memory, exact, from a reader in Rust.

## The parts

Section 12.1 of the plan records the review: six `whole` formats are
inherent (abnf, ebnf, gbnf, expr, semver, proto), two are blocked by
their parsers (c, markdown), four were not inherent, of which css, ini
and toml now ship parts and feed waits. How the three were written is
what a fourth part needs.

- **The passes are read, not guessed.** `dmog run --grammar <format>`
  over a program that scans `(parser-events input)` and writes each pass
  as a line (the pass dump under "Local state") prints every open and
  close with its rule, its depth, its alternate and its tokens; read
  beside the grammar's actions, that is the specification. The whole
  route's tree, for the comparison, is `(json input)` over the same
  document. A part sees a token's value and never its `use` bag, so
  what a node needs from its token must be the token's value: css's lexer
  now puts an at-rule's head there (the type and the fields read from the
  at-keyword and the prelude, in the node's order), and ini's marks an
  array key `k[]` by carrying the name before the brackets in a
  one-element array.
- **Two scans.** A step's outputs are one vector and the language cannot
  flatten, so the first scan yields events and compound items (a member
  as a `[name value]` pair in css, the frames to end in toml) and the
  second writes an item as its events. The first scan's state is what the
  close passes need: a marker per open rule whose close will come (none
  for a rule replaced at its open, which has no close pass) and the open
  containers. Nothing grows with the document's length, only with its
  nesting, under `max_metadata_bytes`.
- **Linear forms only.** The first index at which two paths differ is
  `filter` over `(indices a)` followed by `get-path (path 0)`, or `count`
  when none differs; the tables a header does not continue end innermost
  first through an `[:ending frames]` item and one `[:end]` per frame,
  the second scan holding the frames as its state and popping one per
  `[:end]`. Codex's review found the quadratic forms these replaced, and
  the two defects in the table above; it was right all three times.
- **What a part cannot stream, it refuses.** `fail :unstreamable` is
  `STREAMABILITY_UNKNOWN` at the part's position, and dmog reads the
  document whole. A member written twice (a re-entered INI section, a
  TOML table re-entered by a later header or dotted key, a repeated key)
  is not the part's to detect: the tree contract after it refuses with
  `DUPLICATE_MEMBER`, and dmog reads whole.
- **The oracle is the walk.** Every default-options fixture row of the
  format ran through the part (`run --grammar <format> part.alc`) and
  through the unchanged whole route (`translate --from <format> --to
  json`, before the descriptor took the part), and the two had to agree
  byte for byte; an error row had to be refused on both.

## Measurements

The differentials, as the pull requests record them:

| Format | Identical through the part | Refused on both routes | Read whole |
|---|---|---|---|
| css | 90 documents | 93 | 0 |
| ini | 115 | 7 | 1 unstreamable (`'[1,2]'`), 11 repeated members |
| toml | 128 | 80 | 2 repeated members |

dmog's Rust translation matrix, run after each part was wired in: 2,750
pairs of 132 documents, 0 failures, 7 sources refused by their own
reader, 1,000 pairs refused as their targets declare (875 by a schema-only
target, 125 by Semantic Versioning's embedding); 212 to 229 seconds a run.

## The releases, and dmog taking them

The format repositories' "Releasing" sections were followed as written:
every version site bumped together (`ts/package.json`, `const VERSION` in
`ts/src/<format>.ts` and in `go/<format>.go`, `version` in `rs/Cargo.toml`
and `pub const VERSION` in `rs/src/lib.rs`, and the crate's own entry in
`rs/Cargo.lock`); the three suites run against the published dependencies
(a clean `npm install` and `npm test`; `GOWORK=off go test -count=1 ./...`
with no `replace`; `ci/rust/run.sh`); the bump pull request squash-merged;
`release.yml` dispatched on `main` with `go` true; the tags and the
registries checked. `make version-rs` in ini and toml also moved the
sibling crates' `Cargo.lock` entries: the lock was restored and only the
crate's own entry set.

dmog's gate runs in two states, and a change must pass both: the `ci`
workflow installs the registry packages, and the `Rust` workflow's "Gate
TypeScript and Go" step links every sibling TypeScript checkout and
builds a `go.work` over every sibling module at `main` before "Gate
Rust". The failures #63 met in turn, each a test to know:

- TypeScript's `formats` test pinned a list of the `whole` formats; it now
  holds css, ini and toml to the installed package, a transducer entry or
  a `whole` sentence, never both or neither.
- Go's end-to-end `TestStructuralTranslationParts` wants a part a manifest
  names exposed by the module, which only a released module with
  `Translate().Transduce` is: the reason #63 waited for the releases.
- The Rust library's unit test lists the formats with parts: run
  `cargo test --all-targets`, not the integration tests alone.
- Go's `TestAReadersMemberOrderIsPinned` listed css among the readers
  whose member order is not the document's, which its part made false.
- dmog's TypeScript runner failed alchemy's ini rows until `node_modules`
  held ini 0.5.21: `npm install` keeps an installed version under a `"*"`
  requirement, so `npm install --no-save @tabnas/ini@0.5.21`.

The Go runner then failed alchemy's css rows for a reason worth knowing:
css's Go lexer built an at-rule's head as a plain `map[string]any`, and
alchemy's Go runtime keeps the order of a `*tabnas.OrderedMap` but copies
a plain map as a plain map (`go/alchemy.go`, the two cases), whose
iteration order is Go's, random. css#75 builds the heads as ordered maps,
and 0.5.18 carries it.

## Open

1. **The GitHub rename is the maintainer's.** Rename tabnas/alchemy-cli to
   tabnas/dmog (Settings, General); then apply the workflow patches on
   tabnas/alchemy-cli#62 and tabnas/alchemy#111 (their files name
   `tabnas/dmog`, so they apply after the rename, which GitHub redirects
   the old name to); then re-run admin#160's check and merge it; then
   dmog's first release under the new names, as ADR-32 lists it:
   `@tabnas/dmog` and `tabnas-dmog` first published by hand, their
   trusted publishers naming `tabnas/dmog` and `release.yml`, and
   `npm deprecate` of `@tabnas/alchemy-cli` if wanted. The parts and the
   test comments in 20 format repositories, and `transduce.tsv`'s rows
   that copy their text, still say alchemy-cli; ADR-32 leaves them to a
   sweep of their own.
2. **feed's part** waits on a trim and a first-split primitive (section
   12.1). The review's list of primitives above is the same ask, each
   with fixtures in three runtimes; `flatten` would make every part one
   scan.
3. **A TOML grammar defect**, noticed on toml#114 and reproduced on
   2026-11-03 with toml 0.5.19: a plain key after a dotted key in the same
   table is refused with `unexpected` (`a.b = 1` then `c = 2` fails at the
   second line's first character; `c = 2` then `a.b = 1` parses), where
   TOML allows it. The part serves what the grammar reads, so it is
   unaffected. It needs an issue in tabnas/toml.
4. **Member order through a part in Go.** A grammar whose Go actions or
   lexer build a plain `map[string]any` hands a part its members in
   random order (the css case above). Either alchemy's Go runtime refuses
   or sorts a plain map it is given, or every grammar's Go code that
   feeds a part uses `*tabnas.OrderedMap`; dmog's Go runner over
   `transduce.tsv` is what catches it today.
5. **The evaluator's per-pass cost** is the next performance target (the
   review), under ADR-31: a faster interpreter, never a native twin.
6. **Remote branches left.** tabnas/alchemy-cli `claude/css-part` (the css
   descriptor change before it was folded into #63) and tabnas/alchemy
   `claude/transduce-rows` (older; a push to the name was refused, so
   #113 went up as `claude/parts-transduce-rows`) can be deleted.
7. **aless's `ci.yml`** still carried two steps 0.5.0 removed, which fail
   every run on `main`. The removal is the patch in rjrodger/aless#99's
   body, the maintainer's own pull request of 2026-11-02, which also made
   aless a pure viewer (0.6.0): `--json` and the agent interface are gone,
   so aless is no longer a tree oracle for a part; the fixtures' expected
   column and dmog's whole route are.

## Local state

Nothing of the session's container survives it. Every branch it made is
merged except admin's `claude/deps-after-dmog-rename` (#160). Its scratch
scripts, described so they can be written again:

- the pass dump, the program every part began from, run as
  `dmog run --grammar <format> dump.alc document`:

  ```
  ; Dump every rule pass as one line of text: the phase, the rule, its alternate and its tokens.
  def spell [v]
    match (kind v)
      case :string v
      case :record (string-join "" ["{" (string-join "," (keys v)) "}"])
      case :vector "[..]"
      case :missing "-"
      case :null "null"
      case :boolean (match v (case true "true") (case _ "false"))
      case _ (scalar-text (record) v)

  def tok [t]
    string-join "" [(get :name t) "=" (spell (get :val t)) "|" (spell (get :src t))]

  def alt-text [rule]
    match (get :alt rule)
      case missing "noalt"
      case alt (string-join "" ["p:" (get :p alt) " r:" (get :r alt) " b:" (scalar-text (record) (get :b alt)) " g:" (string-join "," (get :g alt))])

  def line [phase rule]
    string-join " " [phase (get :name rule) (string-join "" ["d" (scalar-text (record) (get :d rule))]) (alt-text rule) (string-join " " (map tok (get :tokens rule))) (string-join "" ["value=" (spell (get :value rule))])]

  def one [s event]
    match s
      case false (transition true [array-start event])
      case _ (transition true [event])

  def step [s pass]
    match pass
      case (open rule) (one s (scalar (line "OPEN " rule)))
      case (close rule) (one s (scalar (line "CLOSE" rule)))

  def finish [s]
    match s
      case false [array-start array-end]
      case _ [array-end]

  def export [input]
    as-events (scan-emit false step finish (parser-events input))
  ```

- a differential runner for one format: every default-options row of its
  `test/spec/*.tsv` written to a file and run through dmog's `run
  --grammar <format> part.alc` and through the whole route (or compared
  with the row's expected column), counted as identical, refused on both,
  unstreamable or a repeated member;
- a suite runner per repository (embed, the TypeScript build and tests,
  `gofmt`, `go vet`, `go test`, `cargo fmt --check`, `cargo test
  --all-targets` and `--doc`, clippy), one line per step with a
  percentage, as the fleet's progress rule asks;
- a release verifier: a branch from `origin/main`, every version site
  bumped, the three suites against the published dependencies;
- a row generator: documents per format run through the dmog binary, the
  output escaped into `transduce.tsv`'s columns;
- a `go.work` over every sibling checkout's Go module, for dmog's Go tests
  against the siblings' `main`;
- a fleet folder of the org's public repositories, for admin's
  `tasks/port-deps.js --write`.

## Working method that held up

- **Rulings are short; each is the whole instruction.** "Merge and
  report", "stream where possible" and "release css, ini and toml" each
  meant the full sequence the fleet's documents describe, with the report
  at the end.
- **The fixtures are the oracle, and the walk the second one.** A part is
  right when every row agrees with the whole route byte for byte; a row's
  expected value is checked against what the behaviour should be, never
  copied from the code.
- **An error row carries the finer code and the position**:
  `ERROR:unstreamable@143:23`, never the code alone.
- **Run both of dmog's states** before pushing a change there: the
  registry install, and the siblings at `main`.
- **Dependencies move only on instruction.** The releases of css, ini and
  toml authorised `go.mod`'s move to them, since versions track the latest
  release; nothing else moved, and `rs/Cargo.lock`'s sibling-version drift
  was never committed (`git checkout -- rs/Cargo.lock`).
- **GitHub only through the tools, and workflow files never in a
  commit.** The credential has no `workflow` scope, so a workflow change
  goes on the pull request as a patch for the maintainer.
- **Clones are single-branch.** A pushed branch is tracked only after the
  refspec `+refs/heads/B:refs/remotes/origin/B` is added to
  `remote.origin.fetch`, a fetch, and `git branch -u origin/B`.
- **macOS runners are scarce.** A job queued past 15 minutes is cancelled;
  re-run the failed jobs once, and a second failure is real.
- **The registry lags.** `npm view` shows a version minutes after
  `release.yml` publishes it; the Go proxy answers sooner.
- **Progress every 30 seconds**, a line with a percentage where one can
  be made, from every script and every background run.
