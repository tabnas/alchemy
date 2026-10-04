/* Copyright (c) 2026 tabnas, MIT License */

// The lowering and the program around it: rs/src/lower.rs's and
// rs/src/program.rs's tests.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { make as makeJson } from '@tabnas/json'
import { BytesWriter, CsvOptions, StringOut } from '@tabnas/render'
import { Ev, Limits, Metrics, ParserSource, Prune, SourceMode, fromJSON, walkDatum } from '@tabnas/transduce'

import {
  Lowering,
  Renderer,
  Runtime,
  Sources,
  csvOptions,
  desugarProgram,
  outer,
  parseFile,
  resolve,
  run,
  value,
} from '../dist/alchemy'

import { EXPECTED_CSV, OPTIONS, PROGRAM, RECORDS, compile, thrown } from './common'

const V = value

function runtime(src: string, native: boolean): Runtime {
  const forms = desugarProgram(parseFile(src, 't.alc'), src)
  const sources = Sources.one('t.alc', src)
  return new Runtime(resolve(forms, sources, outer), sources, OPTIONS).withNative(native)
}

// Run `src` over the JSON `input` through the json grammar's incremental
// events; the output, or the failure thrown.
function runSrc(src: string, input: string, native: boolean, render?: Renderer): string {
  const rt = runtime(src, native)
  const result = run(rt.export())
  const limits = Limits.default()
  const metrics = new Metrics()
  const out = new StringOut()
  const sink = new Lowering(rt, limits, metrics).sink(result, out, render)
  new ParserSource(makeJson(), input)
    .grammar('json')
    .mode(SourceMode.incremental(Prune.never()))
    .limits(limits)
    .metrics(metrics)
    .run(sink)
  return out.text
}

describe('lower', () => {
  it('the worked example through the interpreted library', () => {
    assert.equal(runSrc(PROGRAM, RECORDS, false), EXPECTED_CSV)
  })

  it('the worked example through the native path', () => {
    assert.equal(runSrc(PROGRAM, RECORDS, true), EXPECTED_CSV)
  })

  it('json echoes the input with its lexemes', () => {
    const echo = 'def export [input] (json input)'
    assert.equal(runSrc(echo, RECORDS, true), `${RECORDS}\n`)
    assert.equal(runSrc(echo, '[1.50, 1e2]', false), '[1.50,1e2]\n')
  })

  it('select, map and concat-map stream items', () => {
    const src =
      'def export [input]\n  pipe input\n    select (path "a" each-index)\n    map (fn [x] (get :n x))\n    concat-map (fn [n] (concat (scalar-text csv-options n) ";"))'
    assert.equal(runSrc(src, '{"a":[{"n":1},{"n":2.50}],"b":3}', false), '1;2.50;')
    const joined = 'def export [input]\n  join ","\n    map (fn [x] (get :n x)) (select (path "a" each-index) input)'
    assert.equal(runSrc(joined, '{"a":[{"n":"x"},{"n":""},{"n":"y"}]}', false), 'x,,y')
    const framed = 'def export [input]\n  concat\n    "["\n    join "," (select (path each-index) input)\n    "]"\n    (text "!")'
    assert.equal(runSrc(framed, '["a","b"]', false), '[a,b]!')
    assert.equal(runSrc(framed, '[]', false), '[]!')
    assert.equal(
      runSrc(
        'def export [input]\n  concat-map (fn [x] (get :v x))\n    filter (fn [x] (get :keep x)) (select (path each-index) input)',
        '[{"keep":true,"v":"a"},{"keep":false,"v":"b"},{"keep":true,"v":"c"}]',
        false,
      ),
      'ac',
    )
  })

  it('replace-text over a live text crosses fragments', () => {
    const src = 'def export [input]\n  replace-text "ab" "X"\n    concat-map (fn [s] s) (select (path each-index) input)'
    assert.equal(runSrc(src, '["a","b","zab","a"]', false), 'XzXa')
  })

  it('a finite text result is written at the end', () => {
    assert.equal(runSrc('def export [input] "done"', '1', false), 'done')
  })

  it('a stream result is rendered by the host', () => {
    const table =
      'def export [input] (table-from-json api-binding input)\n' +
      PROGRAM.replace('def export [input]\n  pipe input\n    api-table\n    csv csv-options\n', '')
    assert.equal(runSrc(table, RECORDS, true), EXPECTED_CSV)
    assert.equal(runSrc(table, RECORDS, false, 'csv'), EXPECTED_CSV)
    const json =
      '[{"Identifier":123,"Full name":"Alice","Balance":50.25},{"Identifier":456,"Full name":"Bob","Balance":72}]'
    assert.equal(runSrc(table, RECORDS, true, 'json'), `${json}\n`)
    assert.equal(runSrc(table, RECORDS, false, 'json'), `${json}\n`)
    const echo = 'def export [input] input'
    assert.equal(runSrc(echo, '[1]', true), '[1]\n')
    let f = thrown(() => runSrc(echo, '[1]', true, 'csv'))
    assert.ok(f.message.startsWith('protocol_mismatch: '), String(f))
    f = thrown(() => runSrc('def export [input] (json input)', '1', true, 'json'))
    assert.ok(f.message.startsWith('render_of_text: '), String(f))
  })

  it('records of a table round trips through json', () => {
    const src = PROGRAM.replace('    csv csv-options\n', '    records\n    json\n')
    const json =
      '[{"Identifier":123,"Full name":"Alice","Balance":50.25},{"Identifier":456,"Full name":"Bob","Balance":72}]'
    assert.equal(runSrc(src, RECORDS, true), `${json}\n`)
    assert.equal(runSrc(src, RECORDS, false), `${json}\n`)
  })

  it('protocol mismatches are named', () => {
    let f = thrown(() => runSrc('def export [input] (csv csv-options input)', '1', true))
    assert.equal(f.code, 'DSL_TYPE_ERROR')
    assert.ok(f.message.startsWith('protocol_mismatch: '), String(f))
    f = thrown(() => runSrc('def export [input] (json (select (path each-index) input))', '[1]', true))
    assert.ok(f.message.startsWith('protocol_mismatch: '), String(f))
    f = thrown(() => runSrc('def export [input] (concat-map (fn [x] x) input)', '[1]', true))
    assert.ok(f.message.startsWith('protocol_mismatch: '), String(f))
  })

  it('the csv options map to the dialect or not', () => {
    const rt = runtime(
      'def o csv-options\ndef lf (record (entry :delimiter ";") (entry :newline "\\n") (entry :header false) (entry :null-text "NULL") (entry :missing "-"))\ndef bad (record (entry :delimiter "ab"))',
      true,
    )
    const o = run(rt.defValue('program', 'o')) as value.Val
    assert.deepStrictEqual(csvOptions(o), CsvOptions.default())
    const lf = run(rt.defValue('program', 'lf')) as value.Val
    const opts = csvOptions(lf)!
    assert.equal(opts.delimiter, ';')
    assert.equal(opts.newline, 'lf')
    assert.equal(opts.header, false)
    assert.equal(opts.nullText, 'NULL')
    assert.deepStrictEqual(opts.missing, { type: 'text', text: '-' })
    const bad = run(rt.defValue('program', 'bad')) as value.Val
    assert.equal(csvOptions(bad), undefined)
    assert.equal(csvOptions(V.NULL), undefined)
  })
})

describe('program', () => {
  it('a program knows its output and row selector', () => {
    const p = compile(PROGRAM, 't.alc')
    assert.equal(p.output, 'Text')
    assert.equal(p.rowSelector()!.toString(), '.response.payload.deep.records[*]')
    assert.ok(p.native)
    const slow = p.withNative(false)
    assert.ok(!slow.native)
    assert.equal(slow.rowSelector()?.toString(), '.response.payload.deep.records[*]')
    const table = compile(PROGRAM.replace('    csv csv-options\n', ''), 't.alc')
    assert.equal(table.output, 'TableRows/1')
    const echo = compile('def export [input] input', 't.alc')
    assert.equal(echo.output, 'JsonEvents/1')
    assert.equal(echo.rowSelector(), undefined)
    const select = compile('def export [input] (join "," (select (path "a" each-index) input))', 't.alc')
    assert.equal(select.output, 'Text')
    assert.equal(select.rowSelector()!.toString(), '.a[*]')
    // The checker's output and the plan's agree.
    for (const program of [p, slow, table, echo, select]) assert.equal(program.planOutput(), program.output)
  })

  it('the sink writes through a writer', () => {
    const p = compile(PROGRAM, 't.alc')
    const writer = new BytesWriter()
    const sink = p.sink(writer, undefined, Limits.default(), new Metrics())
    walkDatum(fromJSON(JSON.parse(RECORDS)), sink)
    sink.event(Ev.end)
    assert.equal(writer.text(), EXPECTED_CSV)
  })

  it('compile reports reader, resolver and export failures', () => {
    assert.equal(thrown(() => compile('(a b', 't.alc')).code, 'DSL_PARSE_ERROR')
    let f = thrown(() => compile('def export [input] (nope input)', 't.alc'))
    assert.ok(f.message.startsWith('unknown_name: '), String(f))
    f = thrown(() => compile('def x 1', 't.alc'))
    assert.ok(f.message.startsWith('no_export: '), String(f))
  })
})
