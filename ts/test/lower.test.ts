/* Copyright (c) 2026 tabnas, MIT License */

// The lowering and the program around it: rs/src/lower.rs's and
// rs/src/program.rs's tests. The ones that lower a plan into a sink and run
// it need transduce's routers and render's renderers, so they are
// alchemy-cli's, in its ts/test/lower.test.ts; what is left here builds
// plans and reads them without running one.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import {
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
import { CsvOptions } from '../dist/shared'

import { OPTIONS, PROGRAM, compile, thrown } from './common'

const V = value

function runtime(src: string, native: boolean): Runtime {
  const forms = desugarProgram(parseFile(src, 't.alc'), src)
  const sources = Sources.one('t.alc', src)
  return new Runtime(resolve(forms, sources, outer), sources, OPTIONS).withNative(native)
}

describe('lower', () => {
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

  it('compile reports reader, resolver and export failures', () => {
    assert.equal(thrown(() => compile('(a b', 't.alc')).code, 'DSL_PARSE_ERROR')
    let f = thrown(() => compile('def export [input] (nope input)', 't.alc'))
    assert.ok(f.message.startsWith('unknown_name: '), String(f))
    f = thrown(() => compile('def x 1', 't.alc'))
    assert.ok(f.message.startsWith('no_export: '), String(f))
  })
})
