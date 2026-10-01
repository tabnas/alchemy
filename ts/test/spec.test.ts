/* Copyright (c) 2026 tabnas, MIT License */

// The shared fixtures in ../../test/spec, run through @tabnas/support's
// runner as every grammar repository runs its own (rs/tests/spec_test.rs
// is the Rust half). What is specific to this package is what a row's
// input becomes: the canonical form of the parsed program (reader.tsv),
// or of the desugared program (pipe.tsv), as a JSON string; the checker's
// verdict (check.tsv); the bytes a run writes (run.tsv).
//
// This is pass 1 of the TypeScript port: the front end. Building the plan
// is the evaluator's work, so the check.tsv rows that need it (a plan
// report, or a failure only evaluation meets) are skipped BY NAME below,
// each with its reason, and run.tsv is skipped as a whole file. Pass 2
// removes the skips. Every other row runs.

import { describe, it } from 'node:test'
import assert from 'node:assert'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import {
  SpecRow,
  isErrorExpect,
  loadSpec,
  loadSpecDir,
  makeRunner,
  parseExpect,
} from '@tabnas/support'

import { analyze, canonical, desugarProgram, format, parse, sameProgram } from '../dist/alchemy'

import { REPO_ROOT, SPEC_DIR, failCode } from './common'

const PASS_2 = 'needs interpreter (pass 2)'

// The finer code a row pins, and the position the failure names.
const runnerOptions = {
  errorCode: (err: unknown) => failCode(err),
  errorPos: (err: any) => ({ row: err?.row, col: err?.col }),
}

describe('fixtures', () => {
  // Every fixture the directory holds has a runner below (or is skipped
  // by name, as run.tsv is until pass 2); a new file added without one
  // fails here rather than passing silently.
  it('every fixture has a runner', () => {
    const files = loadSpecDir(SPEC_DIR)
      .map((spec) => spec.file)
      .sort()
    assert.deepStrictEqual(files, ['check.tsv', 'pipe.tsv', 'reader.tsv', 'run.tsv'])
    // And test/AGENTS.md, the fixtures' guide, describes each one.
    const guide = readFileSync(join(REPO_ROOT, 'test', 'AGENTS.md'), 'utf8')
    for (const file of files) {
      assert.ok(guide.includes(`[\`${file}\`](spec/${file})`), `test/AGENTS.md does not describe ${file}`)
    }
  })
})

makeRunner({
  ...runnerOptions,
  parse: (input: string) => canonical(parse(input)),
}).file(join(SPEC_DIR, 'reader.tsv'))

makeRunner({
  ...runnerOptions,
  parse: (input: string) => canonical(desugarProgram(parse(input), input)),
}).file(join(SPEC_DIR, 'pipe.tsv'))

// The check.tsv rows the front end cannot decide, by their input (the
// row's name: its program). A plan report needs the plan, and these
// failures are met only where evaluating `export` to build the plan
// reaches them. Each entry must name a row of the file, and each such
// program must pass the front end, or the skip is wrong; a row not named
// here runs, so a new report row fails until it is listed (or pass 2
// lands).
const WORKED_EXAMPLE =
  'def column-from-meta [source]\n  record\n    entry :label (get "title" source)\n    entry :source\n      as-path\n        get "path" source\n\n' +
  'def api-binding\n  record\n    entry :columns\n      path "response" "metadata" "fields"\n    entry :rows\n      path "response" "payload" "deep" "records" each-index\n    entry :column column-from-meta\n\n' +
  'def api-table [input]\n  table-from-json api-binding input\n\n' +
  'def export [input]\n  pipe input\n    api-table\n    csv csv-options'
const KEY_LINES =
  'def key-lines [s event]\n  match event\n    case (key name) (transition s [(quoted name) "\\n"])\n    case _ (transition s [])\n\n' +
  'def export [input]\n  join ""\n    scan-emit [] key-lines (fn [s] []) (events input)'
const ROWS_BINDING =
  'def rows-binding\n  record\n    entry :columns :infer\n    entry :rows (path each-index)\n\n' +
  'def export [input]\n  pipe input\n    table-from-json rows-binding\n    csv csv-options'
const CHECK_SKIPS: ReadonlyArray<readonly [string, string]> = [
  // The evaluator's `recursion`: a function applied to itself.
  ['def w [f] (f f)\ndef export [input]\n  let [x (w w)]\n    json input', 'evaluator recursion'],
  // A live text refused where the vector is built.
  ['def export [input] (join "," [(json input)])', 'a live text in a vector'],
  ['def export [input] (concat (get :a (record (entry :a "1") (entry :a "2"))) (json input))', 'duplicate_key'],
  ['def export [input] (concat (match 5 (case 1 "one")) (json input))', 'no_match'],
  ['def export [input] (let [x (pop [])] (json input))', 'pop of an empty vector'],
  ['def export [input] (let [x (top [])] (json input))', 'top of an empty vector'],
  ['def export [input] (let [x (repeat -1 "ab")] (json input))', 'repeat of a negative count'],
  ['def export [input] (let [x (string-join "," ["a" 1])] (json input))', 'string-join of a non-string'],
  // The plan reports (test/effects.test.ts holds each to its row, from a
  // plan built by hand).
  [WORKED_EXAMPLE, 'the plan report'],
  ['def export [input]\n  json input', 'the plan report'],
  ['def export [input] input', 'the plan report'],
  ['def export [input] "done"', 'the plan report'],
  [KEY_LINES, 'the plan report'],
  [ROWS_BINDING, 'the plan report'],
]

// Whether a check.tsv row is skipped, and why.
function checkSkip(input: string): string | undefined {
  const named = CHECK_SKIPS.find(([program]) => program === input)
  return undefined === named ? undefined : `${PASS_2}: ${named[1]}`
}

describe('spec: check.tsv', () => {
  const spec = loadSpec(join(SPEC_DIR, 'check.tsv'))
  const runner = makeRunner({
    ...runnerOptions,
    parse: (input: string) => {
      analyze(input, 'check')
      throw new Error(`${PASS_2}: the program checks, and its plan report needs the plan`)
    },
  })
  runner.checkSpec(spec)
  for (const row of spec.rows) {
    const input = row.unesc(row.resolve(0))
    const expected = row.col(row.resolve(1))
    const skip = checkSkip(input)
    it(`row ${row.line}: ${JSON.stringify(input)}`, { skip }, () => runner.row(row, input, expected))
  }

  it('every skip names a row the front end accepts', () => {
    const inputs = spec.rows.map((row: SpecRow) => row.unesc(row.resolve(0)))
    for (const [program, why] of CHECK_SKIPS) {
      assert.ok(inputs.includes(program), `the skip ${JSON.stringify(why)} names no row`)
    }
    // A skipped row is one only the evaluator decides: the front end
    // accepts its program. One the front end refuses runs above instead.
    for (const row of spec.rows) {
      const input = row.unesc(row.resolve(0))
      if (undefined === checkSkip(input)) continue
      assert.doesNotThrow(() => analyze(input, 'check'), `row ${row.line} passes the front end`)
    }
  })
})

describe('spec: run.tsv', { skip: `${PASS_2}: every row runs a program` }, () => {
  it('runs', () => {})
})

// `format` prints a program the reader reads back to the same forms, spans
// aside, for every program the fixtures parse.
describe('layout round trip', () => {
  it('format round-trips every fixture row', () => {
    const failures: string[] = []
    let rows = 0
    for (const spec of loadSpecDir(SPEC_DIR)) {
      for (const row of spec.rows) {
        if (isErrorExpect(row.col(1))) continue
        rows++
        const input = row.unesc(row.resolve(0))
        let program
        try {
          program = parse(input)
        } catch (_e) {
          // The fixture runner reports a parse that should succeed.
          continue
        }
        const layout = format(program)
        try {
          const again = parse(layout)
          if (!sameProgram(program, again)) {
            failures.push(
              `${row.where()}: format changed the program\n  layout: ${JSON.stringify(layout)}\n  read:   ${canonical(again)}\n  was:    ${canonical(program)}`,
            )
          }
        } catch (fail) {
          failures.push(`${row.where()}: the layout form does not parse: ${fail}\n  layout: ${JSON.stringify(layout)}`)
        }
      }
    }
    assert.ok(rows > 0, 'the fixtures hold value rows')
    assert.deepStrictEqual(failures, [])
  })
})

// The fenced examples of docs/language.md, each `alchemy` block with the
// `canonical`, `core` or `check` block after it, are fixture rows: the
// input is a row of reader.tsv, pipe.tsv or check.tsv, and the result
// shown is that row's expected value.
describe('the language reference', () => {
  it('every example is a fixture row', () => {
    const doc = readFileSync(join(REPO_ROOT, 'docs', 'language.md'), 'utf8')
    const rows = (file: string) =>
      new Map(
        loadSpec(join(SPEC_DIR, file)).rows.map((row: SpecRow) => [
          row.unesc(row.resolve(0)).replace(/\n+$/, ''),
          row.col(1),
        ]),
      )
    const fixtures: Record<string, Map<string, string>> = {
      canonical: rows('reader.tsv'),
      core: rows('pipe.tsv'),
      check: rows('check.tsv'),
    }
    const blocks: Array<[string, string]> = []
    let open: [string, string[]] | undefined
    for (const line of doc.split('\n')) {
      const fence = line.startsWith('```') ? line.substring(3) : undefined
      if (undefined === open && undefined !== fence && '' !== fence) {
        open = [fence, []]
      } else if (undefined !== open && '' === fence) {
        blocks.push([open[0], open[1].join('\n')])
        open = undefined
      } else if (undefined !== open) {
        open[1].push(line)
      }
    }
    let examples = 0
    const failures: string[] = []
    blocks.forEach(([language, input], index) => {
      if ('alchemy' !== language) return
      examples++
      const next = blocks[index + 1]
      const fixture = undefined === next ? undefined : fixtures[next[0]]
      if (undefined === fixture) {
        failures.push(`${JSON.stringify(input)}: an alchemy block is followed by a canonical, core or check block`)
        return
      }
      const cell = fixture.get(input.replace(/\n+$/, ''))
      if (undefined === cell) {
        failures.push(`${JSON.stringify(input)}: not a fixture row`)
        return
      }
      const shown = isErrorExpect(cell) ? cell : parseExpect(cell)
      if ('string' !== typeof shown) {
        failures.push(`${JSON.stringify(input)}: the fixture expects ${JSON.stringify(shown)}`)
        return
      }
      // A report ends with a line feed; a fenced block has none.
      if (shown.replace(/\n+$/, '') !== next[1]) {
        failures.push(`${JSON.stringify(input)}: the page shows ${JSON.stringify(next[1])}, the fixture pins ${JSON.stringify(shown)}`)
      }
    })
    assert.ok(examples > 0, 'the reference holds examples')
    assert.deepStrictEqual(failures, [])
  })
})
