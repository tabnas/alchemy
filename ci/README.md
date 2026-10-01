# ci/

The script the Rust gate runs, [`rust/run.sh`](rust/run.sh), and notes on
this repository's own CI workflows. `.github/workflows/rust.yml` calls
that script, and you can run it locally too (`make gate`).

The workflows themselves live in `.github/workflows/`. To change CI, edit
them there in a reviewed pull request: session credentials can push
workflow changes (admin `DECISIONS.md` ADR-8, as amended on 2026-09-24),
so staging a workflow here for a maintainer to promote is optional.

`ci.yml`, `crates-release.yml`, `release.yml`, `notify-status.yml`,
`scorecard.yml` and `deps-gate.yml` are the fleet's shapes, and their
templates belong in admin `rollout/workflows/alchemy__<file>`. Mirror a
change there in the same change: admin `scripts/verify.sh` compares the
two, and `rollout/apply-workflows.sh --apply` pushes the template's text
back. `clib.yml` and `clib-release.yml` are stamped from admin
`tasks/clib-template/` by `tasks/adopt-clib.sh`, from this repository's
row in `tasks/clib-rollout.tsv`, so change the template and restamp.

Sessions still cannot push tags. Releases therefore go through
`workflow_dispatch`, and a workflow that runs only on a tag push needs a
maintainer to push that tag.

## The workflows

- **`ci.yml`** calls the org's `polyglot-ci.yml` for the TypeScript and
  Go ports. It clones the siblings beside this checkout, in dependency
  order: `transduce` and `render`, the protocols a program lowers to, and
  the grammars and tools their builds and the tests need. Every runtime's
  tests read transduce's fixture documents from that clone.
- **`rust.yml`**, the Rust gate: `ci/rust/run.sh` (formatting, build,
  tests, doctests, clippy, and a lockfile check that exempts only the
  sibling crates' versions) on the MSRV pinned in `rs/Cargo.toml`. It
  clones the siblings the script's `SIBLINGS` names, each from the branch
  of this pull request's name when it has one, else from `main`, because
  the crate takes them as path dependencies.
- **`clib.yml`** builds the uniform C library (`go/clib`) on a pull
  request that touches `go/` and runs its contract tests under `-race`.
- **`release.yml`** publishes npm, tags `ts/v` and `go/v`, and calls
  **`clib-release.yml`** (the GitHub Release carrying the C library and
  `manifest.json`) and **`crates-release.yml`** (the crate on crates.io,
  from the release tag). See `AGENTS.md`, "Releasing".
- **`deps-gate.yml`**, **`notify-status.yml`** and **`scorecard.yml`** call
  the org's shared workflows.

There is no `docs.yml` prose gate yet: it needs a Vale configuration and
vocabulary, a style guide, the gated-page list and its recorded alert
counts, which this repository does not have.
