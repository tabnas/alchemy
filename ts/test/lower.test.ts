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

  // `:non-finite`, when the record has it, is one of three policies, which
  // the lowering applies around the renderer; one that is none maps to no
  // dialect, and the library's `csv` runs, whose `scalar-text` refuses it.
  it('a csv options record maps with a non-finite policy and not with another value', () => {
    const base = '(entry :delimiter ",") (entry :newline "\\r\\n") (entry :header true) (entry :null-text "") (entry :missing :error)'
    const rt = runtime(
      `def lit (record ${base} (entry :non-finite :literal))\ndef nul (record ${base} (entry :non-finite :null))\ndef nope (record ${base} (entry :non-finite :nope))\ndef one (record ${base} (entry :non-finite 1))`,
      true,
    )
    const options = (name: string) => csvOptions(run(rt.defValue('program', name)) as value.Val)
    assert.deepStrictEqual(options('lit'), CsvOptions.default())
    assert.deepStrictEqual(options('nul'), CsvOptions.default())
    assert.equal(options('nope'), undefined)
    assert.equal(options('one'), undefined)
    // The fast path takes the first two, and declines the others for the
    // library's text.
    const binding = 'def b\n  record\n    entry :columns (path "m")\n    entry :rows (path "r" each-index)\n    entry :column (fn [d] d)\n'
    for (const [policy, fast] of [[':literal', true], [':null', true], [':nope', false]] as const) {
      const result = compile(
        `${binding}def o (record ${base} (entry :non-finite ${policy}))\ndef export [input] (csv o (table-from-json b input))`,
        't.alc',
      ).result
      assert.equal('text' === result.v && 'csv' === result.plan.p, fast, policy)
    }
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
    // A rewritten tree: a stream of items the checker typed as events, or
    // one the program said are events, is JSON events for the host.
    const tree = compile('def export [input] (map (fn [e] e) (events input))', 't.alc')
    assert.equal(tree.output, 'JsonEvents/1')
    const said = compile(
      'def step [s x] (transition s [x])\ndef fin [s] []\ndef export [input] (as-events (scan-emit null step fin (events input)))',
      't.alc',
    )
    assert.equal(said.output, 'JsonEvents/1')
    // The checker's output and the plan's agree.
    for (const program of [p, slow, table, echo, select, tree, said]) {
      assert.equal(program.planOutput(), program.output)
    }
  })

  it('compile reports reader, resolver and export failures', () => {
    assert.equal(thrown(() => compile('(a b', 't.alc')).code, 'DSL_PARSE_ERROR')
    let f = thrown(() => compile('def export [input] (nope input)', 't.alc'))
    assert.ok(f.message.startsWith('unknown_name: '), String(f))
    f = thrown(() => compile('def x 1', 't.alc'))
    assert.ok(f.message.startsWith('no_export: '), String(f))
  })
})
