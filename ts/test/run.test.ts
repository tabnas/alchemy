/* Copyright (c) 2026 tabnas, MIT License */

// The public API end to end (rs/tests/run_test.rs): the spec's worked
// example (section 5) run through the spec's program (sections 12.1 and
// 13.4), natively and interpreted, byte for byte; the streaming behaviours
// of the spec's section 19.5 that apply to a run over one document; the
// `json` echo of every fixture against the walk's own rendering; and
// `records` of a table round-tripping through JSON.

import { describe, it } from 'node:test'
import assert from 'node:assert'
import { readFileSync, readdirSync } from 'node:fs'
import { extname, join } from 'node:path'

import { Tabnas } from '@tabnas/parser'
import { jsonic } from '@tabnas/jsonic'
import { make as makeCsv } from '@tabnas/csv'
import { make as makeJson } from '@tabnas/json'
import { make as makeJsonl } from '@tabnas/jsonl'
import { Yaml } from '@tabnas/yaml'
import { BytesWriter, JsonRenderer, StringOut, WriteOut } from '@tabnas/render'
import { AbortFlag, Limits, Metrics, ParserSource, replay } from '@tabnas/transduce'

import { Program, compile } from '../dist/alchemy'

import { EXPECTED_CSV, PROGRAM, RECORDS, drive, err, events, ok, thrown } from './common'
import { FIXTURES, METADATA, record, recordsJson } from './support'

function both(): [Program, Program] {
  const native = compile(PROGRAM, 'export.alc')
  return [native, native.withNative(false)]
}

function shaped(records: string): string {
  return `{"response":{"metadata":${METADATA},"payload":{"deep":{"records":[${records}]}}}}`
}

// Definitions `a0` to `a{levels}`, each the concatenation of the one before
// it with itself: a text of 10 * 2^levels bytes, built of shared parts in
// linear time.
function doubling(levels: number): string {
  let src = 'def a0 "0123456789"\n'
  for (let i = 1; i <= levels; i++) src += `def a${i} (concat a${i - 1} a${i - 1})\n`
  return src
}

describe('run', () => {
  // Acceptance 3: the worked example prints the spec's bytes, both ways.
  it('the worked example prints the spec csv both ways', () => {
    for (const program of both()) {
      assert.equal(program.output, 'Text')
      assert.equal(program.rowSelector()!.toString(), '.response.payload.deep.records[*]')
      assert.equal(ok(program, RECORDS), EXPECTED_CSV, `native=${program.native}`)
    }
  })

  // Spec 19.5: metadata after rows is rejected under the metadata-first
  // policy, before any row is retained, and no CSV is written.
  it('metadata after rows is an input order violation', () => {
    const doc = `{"response":{"payload":{"deep":{"records":[${record(1)}]}},"metadata":${METADATA}}}`
    for (const program of both()) {
      const { fail, out } = err(program, doc)
      assert.equal(fail.code, 'INPUT_ORDER_VIOLATION', `native=${program.native}`)
      assert.ok(!fail.committedOutput)
      assert.equal(out, '')
    }
  })

  // Spec 19.5: cells follow the schema's order whatever the row's.
  it('cells follow schema order not member order', () => {
    const doc = shaped('{"account":{"balance":1},"person":{"name":"z"},"id":2}')
    for (const program of both()) {
      assert.equal(ok(program, doc), '"Identifier","Full name","Balance"\r\n"2","z","1"\r\n')
    }
  })

  // Spec 19.5: no matching rows is a valid empty table: the header alone.
  it('no matching rows prints the header only', () => {
    for (const doc of [shaped(''), `{"response":{"metadata":${METADATA}}}`]) {
      for (const program of both()) {
        assert.equal(ok(program, doc), '"Identifier","Full name","Balance"\r\n', `native=${program.native}`)
      }
    }
  })

  // Spec 19.5: invalid trailing input fails the run after rows were
  // exported; with enough rows to pass the writer's budget, the failure
  // says the output is partial, and what was committed is a prefix of the
  // whole output.
  it('invalid trailing input fails after rows were exported', () => {
    const whole = recordsJson(2000)
    const doc = `${whole} x`
    for (const program of both()) {
      const full = ok(program, whole)
      const { fail, out } = err(program, doc)
      assert.equal(fail.code, 'INPUT_INVALID', `native=${program.native}`)
      assert.ok(fail.committedOutput, `native=${program.native}`)
      assert.ok(out.startsWith('"Identifier","Full name","Balance"\r\n'))
      assert.ok(Buffer.byteLength(out) > 32 * 1024)
      // The writer coalesces by fragment, never holding a row back to end
      // on a record boundary (spec 17.4): what was committed is a prefix of
      // the whole output, and may end inside a record.
      assert.ok(full.startsWith(out), `native=${program.native}`)
      assert.ok(out.length < full.length)
    }
    // A small document: the rows were buffered, not committed, so nothing
    // reached the writer and the failure says so.
    const { fail, out } = err(both()[0], `${RECORDS} x`)
    assert.equal(fail.code, 'INPUT_INVALID')
    assert.ok(!fail.committedOutput)
    assert.equal(out, '')
  })

  // Spec 19.5: a very large selected row fails clearly under the limit that
  // bounds it, the same one both ways: the native transducer materializes
  // rows under `max_record_bytes`, and the library's `table-from-json`
  // captures its rows under the same limit, so the generic
  // `max_capture_bytes` does not decide either.
  it('a very large selected row names the limit', () => {
    const big = `{"id":1,"person":{"name":"${'x'.repeat(4096)}"},"account":{"balance":2}}`
    const doc = shaped(`${record(0)},${big}`)
    const limits = Limits.with({ max_record_bytes: 1024 })
    for (const program of both()) {
      const { fail } = err(program, doc, limits)
      assert.equal(fail.code, 'RESOURCE_LIMIT_EXCEEDED')
      assert.equal(fail.limit.name, 'max_record_bytes', `native=${program.native}`)
      assert.equal(fail.path, '.response.payload.deep.records[1]')
    }
    // A small generic capture limit decides neither: both print the rows.
    const capture = Limits.with({ max_capture_bytes: 64 })
    for (const program of both()) {
      const { fail, out } = drive(program, RECORDS, undefined, capture)
      assert.equal(fail, undefined, `native=${program.native}: ${fail}`)
      assert.equal(out, EXPECTED_CSV)
    }
  })

  // `string-join` builds one string from a vector of strings, the separator
  // between them. An item that is not a string is a type error where the
  // plan is built, and the joined string is held to `max_scalar_bytes`
  // before it is built.
  it('string-join builds one string from several', () => {
    const program = compile(
      'def export [input]\n  concat\n    string-join ", " ["a" "b" "c"]\n    "|"\n    string-join "-" []\n    "|"\n    string-join "" ["x"]\n    "|"\n    string-join " " [(quoted "k") "holds" (scalar-text csv-options 1.5)]\n    "\\n"\n',
      'join.alc',
    )
    assert.equal(program.output, 'Text')
    assert.equal(ok(program, 'null'), 'a, b, c||x|"k" holds 1.5\n')
    // A failure that names a key, built from the parts.
    const fail = thrown(() =>
      compile(
        'def export [input] (let [m (fail (string-join " " ["no value under" (quoted "k")]))] (json input))',
        'named.alc',
      ),
    )
    assert.equal(fail.code, 'INPUT_INVALID', String(fail))
    assert.equal(fail.message, 'no value under "k"', String(fail))
    // The joined string is one scalar, held to max_scalar_bytes: here one
    // built at the end of a scan over the events, under the run's limits.
    const big = compile(
      'def step [s e] (transition (push "abcd" s) [])\ndef fin [s] [(string-join "" s)]\ndef export [input]\n  join "" (scan-emit [] step fin (events input))\n',
      'big.alc',
    )
    const { fail: f, out } = err(big, '[1,2,3,4,5,6,7,8]', Limits.with({ max_scalar_bytes: 16 }))
    assert.equal(f.code, 'RESOURCE_LIMIT_EXCEEDED', String(f))
    assert.equal(f.limit.name, 'max_scalar_bytes')
    assert.match(f.message, /string-join/)
    assert.equal(out, '')
  })

  // A missing cell under the standard options is `MISSING_VALUE`, and a
  // null is the empty string, both ways.
  it('missing and null cells follow the options', () => {
    for (const program of both()) {
      const { fail } = err(program, shaped('{"id":1,"person":{},"account":{"balance":2}}'))
      assert.equal(fail.code, 'MISSING_VALUE', `native=${program.native}`)
      assert.equal(
        ok(program, shaped('{"id":null,"person":{"name":"n"},"account":{"balance":null}}')),
        '"Identifier","Full name","Balance"\r\n"","n",""\r\n',
      )
    }
  })

  // `json input` echoes every fixture as the JSON renderer renders the
  // walk's events: the plan passes the events through untouched.
  it('json echo of every fixture equals the walk\'s rendering', () => {
    const echo = compile('def export [input] (json input)', 'echo.alc')
    assert.equal(echo.output, 'Text')
    const identity = compile('def export [input] input', 'id.alc')
    assert.equal(identity.output, 'JsonEvents/1')
    const grammarFor = (path: string): (() => any) | undefined => {
      switch (extname(path)) {
        case '.json':
          return () => makeJson()
        case '.jsonl':
          return () => makeJsonl()
        case '.yaml':
          return () => new Tabnas().use(jsonic).use(Yaml)
        case '.csv':
        case '.tsv':
          return () => makeCsv()
        default:
          return undefined
      }
    }
    let compared = 0
    for (const name of readdirSync(FIXTURES).sort()) {
      const path = join(FIXTURES, name)
      const make = grammarFor(path)
      if (undefined === make) continue
      const evs = events(make(), readFileSync(path, 'utf8'))
      if (undefined === evs) continue
      const reference = new JsonRenderer(new StringOut(), { indent: null, trailingNewline: true })
      try {
        replay(evs, reference)
      } catch (_fail) {
        // A document the renderer refuses (a non-finite number, say) is
        // refused the same way through the program; not compared.
        continue
      }
      const expected = reference.intoInner().text
      for (const program of [echo, identity]) {
        const writer = new BytesWriter()
        const sink = program.sink(writer, undefined, Limits.default(), new Metrics())
        replay(evs, sink)
        assert.equal(writer.text(), expected, path)
      }
      compared++
    }
    assert.ok(compared > 10, `${compared} fixtures compared`)
  })

  // `records` of a table is JSON events: rendered as JSON they read back as
  // the rows, keyed by label, with the lexemes kept.
  it('records of a table round trips', () => {
    const program = compile(PROGRAM.replace('    csv csv-options\n', '    records\n    json\n'), 'records.alc')
    assert.equal(program.output, 'Text')
    const expected =
      '[{"Identifier":123,"Full name":"Alice","Balance":50.25},{"Identifier":456,"Full name":"Bob","Balance":72}]'
    for (const p of [program, program.withNative(false)]) {
      const out = ok(p, RECORDS)
      assert.equal(out, `${expected}\n`)
      const parsed = JSON.parse(out)
      assert.equal(parsed[1]['Full name'], 'Bob')
      assert.equal(parsed[0].Balance, 50.25)
    }
    // The table result rendered by the host as JSON is the same document,
    // and as CSV the spec's bytes.
    const table = compile(PROGRAM.replace('    csv csv-options\n', ''), 'table.alc')
    assert.equal(table.output, 'TableRows/1')
    assert.equal(ok(table, RECORDS, 'json'), `${expected}\n`)
    assert.equal(ok(table, RECORDS, 'csv'), EXPECTED_CSV)
    assert.equal(ok(table, RECORDS), EXPECTED_CSV)
    // A renderer for a program that renders its own text is refused.
    const { fail } = drive(program, RECORDS, 'json')
    assert.equal(fail.code, 'DSL_TYPE_ERROR')
    assert.ok(fail.message.startsWith('render_of_text: '), String(fail))
  })

  // The output limit is the writer's: a run that would exceed it fails
  // with the limit named, and nothing past it is written.
  it('the output limit is enforced by the writer', () => {
    const { fail } = err(both()[0], recordsJson(50), Limits.with({ max_output_bytes: 200 }))
    assert.equal(fail.code, 'RESOURCE_LIMIT_EXCEEDED')
    assert.equal(fail.limit.name, 'max_output_bytes')
  })

  // `sinkOut` takes any text output: a writer with no budget commits every
  // fragment as it is written.
  it('sinkOut takes the host\'s own text output', () => {
    const program = compile(PROGRAM, 'export.alc')
    const writer = new BytesWriter()
    const sink = program.sinkOut(new WriteOut(writer).withBudget(0), undefined, Limits.default(), new Metrics())
    replay(events(makeJson(), RECORDS)!, sink)
    assert.equal(writer.text(), EXPECTED_CSV)
  })

  // A program is code, and code can ask for too much before it reads a
  // byte. Building the plan is bounded: its steps by `max_plan_steps`, its
  // nesting by the evaluation depth (a function applied to itself is
  // `recursion`, not a stack overflow), and a `concat` over shared parts
  // knows which item is live without walking them again.
  it('building the plan is bounded', () => {
    const d = `${'(d '.repeat(40)}id${')'.repeat(40)}`
    const expo = `def id [x] x\ndef d [g] (fn [x] (g (g x)))\ndef export [input]\n  let [y (${d} 1)]\n    json input\n`
    let fail = thrown(() => compile(expo, 'expo.alc'))
    assert.equal(fail.code, 'RESOURCE_LIMIT_EXCEEDED', String(fail))
    assert.equal(fail.limit.name, 'max_plan_steps')
    let wide = 'def v0 [1 2 3]\n'
    for (let i = 1; i <= 40; i++) wide += `def v${i} (vector v${i - 1} v${i - 1})\n`
    wide += 'def export [input] (concat (scalar-text csv-options v40) (json input))\n'
    fail = thrown(() => compile(wide, 'wide.alc'))
    assert.equal(fail.code, 'RESOURCE_LIMIT_EXCEEDED', String(fail))
    const omega = 'def w [f] (f f)\ndef export [input]\n  let [x (w w)]\n    json input\n'
    fail = thrown(() => compile(omega, 'omega.alc'))
    assert.equal(fail.code, 'STREAMABILITY_UNKNOWN', String(fail))
    assert.ok(fail.message.startsWith('recursion: '), String(fail))
    assert.deepStrictEqual([fail.row, fail.col], [1, 12])
    const start = Date.now()
    const program = compile(doubling(40) + 'def export [input] (concat a40 (json input))\n', 'shared.alc')
    assert.ok(Date.now() - start < 20_000, `${Date.now() - start} ms`)
    // Its prefix is 10 TB; the output limit ends it.
    const { fail: f, out } = err(program, '1', Limits.with({ max_output_bytes: 1000 }))
    assert.equal(f.limit.name, 'max_output_bytes')
    assert.ok(out.length <= 1000, String(out.length))
  })

  // A function applied to itself per item fails with `recursion` at the
  // item, rather than aborting the host: in Node's default stack, which
  // the Rust crate needs a thread of 64 MiB for.
  it('self-application per item is recursion at run time', () => {
    const src =
      'def w [f] (f f)\ndef export [input]\n  pipe input\n    select (path each-index)\n    map (fn [x] (w w))\n    join ","\n'
    const program = compile(src, 'omega.alc')
    const { fail, out } = err(program, '[1]')
    assert.equal(fail.code, 'STREAMABILITY_UNKNOWN', String(fail))
    assert.ok(fail.message.startsWith('recursion: '), String(fail))
    assert.equal(out, '')
  })

  // The host's abort flag reaches the program's own functions: a long
  // computation on one item stops with `ABORTED` at the next evaluation
  // step, not when the item is done. The source is not given the flag
  // here, so only the program can have seen it.
  it('the abort flag stops a long computation on one item', () => {
    const d = `${'(d '.repeat(30)}id${')'.repeat(30)}`
    const src = `def id [x] x\ndef d [g] (fn [x] (g (g x)))\ndef export [input]\n  join ","\n    map (fn [x] (${d} x)) (select (path each-index) input)\n`
    const flag = new AbortFlag()
    const program = compile(src, 'long.alc').withAbort(flag)
    const evs = events(makeJson(), '[1,2,3]')!
    const sink = program.sink(new BytesWriter(), undefined, Limits.default(), new Metrics())
    flag.abort()
    const start = Date.now()
    const fail = thrown(() => replay(evs, sink))
    assert.equal(fail.code, 'ABORTED', String(fail))
    assert.ok(Date.now() - start < 20_000, `${Date.now() - start} ms`)
  })

  // The output limit bounds what a finite text holds, not only what is
  // written: `replace-text` over a finite text streams through the replacer
  // (it holds at most the literal), so the limit stops it at its first
  // byte past; a `join` or `concat-map` item is assembled whole to write it
  // atomically, and the assembly fails as soon as it passes the limit,
  // before anything of it is written.
  it('the output limit bounds a finite text', () => {
    const limits = Limits.with({ max_output_bytes: 100_000 })
    const replace = compile(doubling(25) + 'def export [input] (replace-text "0" "x" a25)\n', 'replace.alc')
    const { fail, out } = err(replace, '1', limits)
    assert.equal(fail.limit.name, 'max_output_bytes')
    // The writer refuses the fragment that would cross the limit, so what
    // reached it is the text up to there.
    assert.ok(fail.committedOutput, String(fail))
    assert.ok(out.length > 50_000 && out.length <= 100_000, String(out.length))
    assert.ok(out.startsWith('x123456789x123456789'), out.substring(0, 20))
    for (const step of ['join ","', 'concat-map (fn [t] t)']) {
      const program = compile(
        doubling(25) + `def export [input] (${step} (map (fn [x] a25) (select (path each-index) input)))\n`,
        'items.alc',
      )
      const { fail: f, out: o } = err(program, '[1,2]', limits)
      assert.equal(f.limit.name, 'max_output_bytes', step)
      assert.equal(o, '', step)
    }
  })

  // A `scan-emit` state is what a stage retains from item to item, so it is
  // measured as it changes: no deeper than `max_depth`, no larger than
  // `max_metadata_bytes`, and reported in `retained_bytes_high`.
  it('a scan-emit state is measured and capped', () => {
    const grow =
      'def step [s x] (transition [s x] [])\ndef fin [s] ["done"]\ndef export [input]\n  join ","\n    scan-emit null step fin (select (path each-index) input)\n'
    let program = compile(grow, 'grow.alc')
    const numbers = `[${new Array(300).fill('1').join(',')}]`
    let { fail } = err(program, numbers)
    assert.equal(fail.limit.name, 'max_depth', String(fail))
    assert.deepStrictEqual([fail.row, fail.col], [5, 5])
    // The same state under a small byte limit.
    const small = Limits.with({ max_metadata_bytes: 256 })
    ;({ fail } = err(program, numbers, small))
    assert.equal(fail.limit.name, 'max_metadata_bytes', String(fail))
    // A state that wraps itself in a partial once per item holds the one
    // before it inside the function, not the arguments: measured link by
    // link, it fails as the vector does.
    const chain =
      'def step [s x] (transition (partial s x) [])\ndef fin [s] ["done"]\ndef export [input]\n  join ","\n    scan-emit (fn [a] a) step fin (select (path each-index) input)\n'
    program = compile(chain, 'chain.alc')
    ;({ fail } = err(program, numbers))
    assert.equal(fail.limit.name, 'max_depth', String(fail))
    assert.deepStrictEqual([fail.row, fail.col], [5, 5])
    const strings = `[${new Array(200).fill(`"${'a'.repeat(100)}"`).join(',')}]`
    ;({ fail } = err(program, strings, small))
    assert.equal(fail.limit.name, 'max_metadata_bytes', String(fail))
    // A state that keeps the one before inside the partial a finite text's
    // concat-map applies is walked through that function: it fails as the
    // vector does, rather than growing a level per item.
    const text =
      'def g [prev y] y\ndef step [s x] (transition [(concat-map (partial g s) ["x"])] [])\ndef fin [s] ["done"]\ndef export [input]\n  join ","\n    scan-emit [] step fin (select (path each-index) input)\n'
    program = compile(text, 'text.alc')
    ;({ fail } = err(program, numbers))
    assert.equal(fail.limit.name, 'max_depth', String(fail))
    assert.deepStrictEqual([fail.row, fail.col], [6, 5])
    ;({ fail } = err(program, numbers, small))
    assert.equal(fail.limit.name, 'max_metadata_bytes', String(fail))
    // A state that keeps the last item is measured and reported.
    const last =
      'def step [s x] (transition x [])\ndef fin [s] ["done"]\ndef export [input]\n  join ","\n    scan-emit null step fin (select (path each-index) input)\n'
    program = compile(last, 'last.alc')
    const metrics = new Metrics()
    const writer = new BytesWriter()
    const sink = program.sink(writer, undefined, Limits.default(), metrics)
    new ParserSource(makeJson(), `[{"k":"${'a'.repeat(500)}"},{"k":"b"}]`).metrics(metrics).run(sink)
    assert.equal(writer.text(), 'done')
    assert.ok(metrics.retained_bytes_high >= 500, String(metrics.retained_bytes_high))
  })

  // The initial state is retained like any other: measured when the stage
  // is built, so neither a step that hands the same state back nor a source
  // with no items carries one past the limits, and the failure comes before
  // anything is read or written.
  it('a scan-emit initial state is measured and capped', () => {
    const tail = 'def fin [s] ["done"]\ndef export [input]\n  join ","\n    scan-emit init step fin (select (path each-index) input)\n'
    const keep = 'def step [s x] (transition s [])\n'
    const big = `def init ["${'a'.repeat(1000)}"]\n${keep}${tail}`
    let program = compile(big, 'big.alc')
    const small = Limits.with({ max_metadata_bytes: 256 })
    for (const input of ['[1,2,3]', '[]']) {
      const { fail, out } = err(program, input, small)
      assert.equal(fail.limit.name, 'max_metadata_bytes', `${input}: ${fail}`)
      assert.deepStrictEqual([fail.row, fail.col], [6, 5], input)
      assert.equal(out, '', input)
      assert.ok(!fail.committedOutput, input)
    }
    const deep = `def init ${'['.repeat(10)}1${']'.repeat(10)}\n${keep}${tail}`
    program = compile(deep, 'deep.alc')
    const shallow = Limits.with({ max_depth: 8 })
    for (const input of ['[1,2,3]', '[]']) {
      const { fail, out } = err(program, input, shallow)
      assert.equal(fail.limit.name, 'max_depth', `${input}: ${fail}`)
      assert.equal(out, '', input)
    }
    // Under the defaults it runs, and the state it retains is reported.
    program = compile(big, 'big.alc')
    for (const input of ['[1,2,3]', '[]']) {
      const metrics = new Metrics()
      const writer = new BytesWriter()
      const sink = program.sink(writer, undefined, Limits.default(), metrics)
      new ParserSource(makeJson(), input).metrics(metrics).run(sink)
      assert.equal(writer.text(), 'done', input)
      assert.ok(metrics.retained_bytes_high >= 1000, `${input}: ${metrics.retained_bytes_high}`)
    }
  })
})
