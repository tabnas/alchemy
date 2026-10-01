/* Copyright (c) 2026 tabnas, MIT License */

// The plan report (rs/src/effects.rs). Until the evaluator builds plans
// (pass 2), each program's plan is built here by hand, as the evaluator's
// native path builds it, and the report is held to the check.tsv row that
// pins it: the same text, byte for byte. The front end decides the rest
// of what the report reads (the output, the definitions).

import { describe, it } from 'node:test'
import assert from 'node:assert'
import { join } from 'node:path'

import { loadSpec, parseExpect } from '@tabnas/support'
import { Selector } from '@tabnas/transduce'

import { Plan, Val, analyze, explain, explainJson, stdlib, summarize, value } from '../dist/alchemy'

import { SPEC_DIR } from './common'

const V = value

// The report check.tsv pins for `program`.
function pinned(program: string): string {
  const row = loadSpec(join(SPEC_DIR, 'check.tsv')).rows.find((r: any) => r.unesc(r.resolve(0)) === program)
  assert.ok(row, `check.tsv has a row for ${JSON.stringify(program)}`)
  return parseExpect(row.col(1)) as string
}

// A compiled program as the report reads it: the front end's verdict and
// a plan built by hand.
function program(src: string, result: Val, duplicates: 'reject' | 'last_wins' | 'first_wins' = 'reject') {
  const analyzed = analyze(src, 'check')
  return { output: analyzed.checked.output, resolved: analyzed.resolved, result, duplicates }
}

const stream = (plan: Plan): Val => ({ v: 'stream', plan })
const text = (plan: Plan): Val => ({ v: 'text', plan })
const INPUT: Plan = { p: 'input' }

const SP = { file: { name: 'check' }, start: 0, end: 0 }

// The library's `csv-options`, as the record it evaluates to.
const CSV_OPTIONS = V.record([
  ['delimiter', V.str(',')],
  ['newline', V.str('\r\n')],
  ['header', { v: 'bool', value: true }],
  ['null-text', V.str('')],
  ['missing', V.keyword('error')],
])

const sel = (s: Selector): Val => ({ v: 'selector', selector: s })

describe('explain', () => {
  it('the worked example reports in the spec layout', () => {
    const src =
      'def column-from-meta [source]\n  record\n    entry :label (get "title" source)\n    entry :source\n      as-path\n        get "path" source\n\ndef api-binding\n  record\n    entry :columns\n      path "response" "metadata" "fields"\n    entry :rows\n      path "response" "payload" "deep" "records" each-index\n    entry :column column-from-meta\n\ndef api-table [input]\n  table-from-json api-binding input\n\ndef export [input]\n  pipe input\n    api-table\n    csv csv-options'
    const binding = V.record([
      ['columns', sel(Selector.root().property('response').property('metadata').property('fields'))],
      [
        'rows',
        sel(Selector.root().property('response').property('payload').property('deep').property('records').eachIndex()),
      ],
    ])
    const plan: Plan = {
      p: 'csv',
      options: CSV_OPTIONS,
      source: { p: 'table-from-json', binding, source: INPUT, at: SP },
    }
    const p = program(src, text(plan))
    assert.equal(explain(p), pinned(src))
    const j: any = explainJson(p)
    assert.deepStrictEqual(j.chain, ['api-table', 'csv'])
    assert.deepStrictEqual(j.protocol, ['JsonEvents/1', 'TableRows/1', 'Text'])
    assert.equal(j.retention[0].scope, 'metadata')
    assert.equal(j.retention[1].limit, 'max_record_bytes')
    assert.equal(j.confidence, 'proven')
    assert.equal(j.readiness, 'record')
    assert.equal(j.renderer.name, 'csv')
    assert.equal(j.output, 'Text')
    assert.equal(j.order_constraints[0].enforcement, 'runtime')
    // The JSON report's keys, in the Rust report's order.
    assert.deepStrictEqual(Object.keys(j), [
      'entry',
      'finite',
      'chain',
      'output',
      'passes',
      'protocol',
      'selection',
      'duplicates',
      'retention',
      'readiness',
      'output_order',
      'order_constraints',
      'renderer',
      'external_storage',
      'confidence',
      'guarantee',
      'qualification',
    ])
    assert.deepStrictEqual(j.renderer, {
      name: 'csv',
      quoting: 'always',
      delimiter: ',',
      newline: '\r\n',
      header: true,
      null_text: '',
      missing: 'error',
      missing_text: null,
      host: false,
    })
    // The policy names what happens to a repeated member.
    assert.equal(
      (explainJson(program(src, text(plan), 'last_wins')) as any).duplicates,
      'the last value wins in captured scopes',
    )
    assert.equal(summarize(program(src, text(plan), 'first_wins')).duplicates, 'the first value wins in captured scopes')
  })

  it('the JSON echo, the identity and a text of its own', () => {
    const echo = 'def export [input]\n  json input'
    assert.equal(explain(program(echo, text({ p: 'json', source: INPUT }))), pinned(echo))
    const identity = 'def export [input] input'
    const id = program(identity, stream(INPUT))
    assert.equal(explain(id), pinned(identity))
    assert.equal((explainJson(id) as any).renderer.host, true)
    const done = 'def export [input] "done"'
    const d = program(done, V.str('done'))
    assert.equal(explain(d), pinned(done))
    const j: any = explainJson(d)
    assert.equal(j.finite, true)
    assert.deepStrictEqual(j.protocol, ['Text'])
    assert.equal(j.readiness, 'scope-end')
    assert.deepStrictEqual(j.qualification, [
      'The text is written only after the whole input has validated; an invalid input writes nothing.',
    ])
    // A finite text built of calls names them.
    const built = 'def export [input] (replace-text "a" "b" (concat "x" "a"))'
    const plan: Plan = {
      p: 'replace',
      from: 'a',
      to: 'b',
      source: text(V.concatPlan([V.str('x'), V.str('a')])),
    }
    const report = explain(program(built, text(plan)))
    assert.ok(report.startsWith('export: concat → replace-text\n'), report)
    assert.ok(report.includes('Protocol:              Text\n'), report)
  })

  it('a program over the events: nothing retained by events, the state capped', () => {
    const src =
      'def key-lines [s event]\n  match event\n    case (key name) (transition s [(quoted name) "\\n"])\n    case _ (transition s [])\n\ndef export [input]\n  join ""\n    scan-emit [] key-lines (fn [s] []) (events input)'
    const p = program(src, V.str('placeholder'))
    const step = { fn: 'closure', closure: { name: 'key-lines', params: ['s', 'event'], body: p.resolved.get('key-lines')!.value, env: undefined, scope: 'program', span: SP } } as const
    const finish = { fn: 'closure', closure: { params: ['s'], body: p.resolved.get('key-lines')!.value, env: undefined, scope: 'program', span: SP } } as const
    const plan: Plan = {
      p: 'join',
      sep: '',
      items: {
        seq: 'stream',
        plan: { p: 'scan-emit', init: V.vectorVal([]), step, finish, source: { p: 'events', source: INPUT }, at: SP },
      },
    }
    assert.equal(explain({ ...p, result: text(plan) }), pinned(src))
  })

  it('an inferred binding reports one route', () => {
    const src =
      'def rows-binding\n  record\n    entry :columns :infer\n    entry :rows (path each-index)\n\ndef export [input]\n  pipe input\n    table-from-json rows-binding\n    csv csv-options'
    const binding = V.record([
      ['columns', V.keyword('infer')],
      ['rows', sel(Selector.root().eachIndex())],
    ])
    const plan: Plan = {
      p: 'csv',
      options: CSV_OPTIONS,
      source: { p: 'table-from-json', binding, source: INPUT, at: SP },
    }
    const p = program(src, text(plan))
    assert.equal(explain(p), pinned(src))
    const s = summarize(p)
    assert.deepStrictEqual(
      s.retention.map((r) => [r.scope, r.label, r.limit]),
      [
        ['metadata', 'Inferred columns:', 'max_metadata_bytes'],
        ['record', 'Row capture:', 'max_record_bytes'],
      ],
    )
    assert.ok(s.orderConstraints.every((o) => 'metadata completes' !== o.before))
  })

  // The library's twin of the table: its `table-step`, from `no-schema`,
  // claims the column cap; seeded with columns already bound it does not.
  it('a table step seeded with columns claims no column cap', () => {
    const src = 'def export [input] (json input)'
    const p = program(src, V.str('x'))
    const tableStep = stdlib().get('table-step')!
    const step = {
      fn: 'partial',
      partial: {
        f: { fn: 'closure', closure: { name: 'table-step', params: ['binding', 'state', 'event'], body: tableStep.value, env: undefined, scope: 'stdlib', span: tableStep.span } },
        args: [],
      },
    } as const
    const finish = { fn: 'closure', closure: { params: ['s'], body: tableStep.value, env: undefined, scope: 'program', span: SP } } as const
    const state = (init: Val) => {
      const plan: Plan = {
        p: 'join',
        sep: '',
        items: { seq: 'stream', plan: { p: 'scan-emit', init, step, finish, source: INPUT, at: SP } },
      }
      return summarize({ ...p, result: text(plan) }).retention.find((r) => 'state' === r.scope)!
    }
    assert.equal(state(V.taggedVal('ready', [V.vectorVal([])])).reason, 'what the step returns, no deeper than max_depth')
    assert.ok(state(V.taggedVal('no-schema')).reason.includes('at most max_columns'))
  })

  it('a custom csv dialect is reported as it runs, and the host renders a table', () => {
    const src = 'def export [input] (json input)'
    const p = program(src, V.str('x'))
    const lf = V.record([
      ['delimiter', V.str(';')],
      ['newline', V.str('\n')],
      ['header', { v: 'bool', value: false }],
      ['null-text', V.str('NULL')],
      ['missing', V.str('-')],
    ])
    const binding = V.record([
      ['columns', V.keyword('infer')],
      ['rows', sel(Selector.root().eachIndex())],
    ])
    const table: Plan = { p: 'table-from-json', binding, source: INPUT, at: SP }
    const r: any = (explainJson({ ...p, result: text({ p: 'csv', options: lf, source: table }) }) as any).renderer
    assert.deepStrictEqual(r, {
      name: 'csv',
      quoting: 'always',
      delimiter: ';',
      newline: '\n',
      header: false,
      null_text: 'NULL',
      missing: 'text',
      missing_text: '-',
      host: false,
    })
    const host = { ...p, output: 'TableRows/1' as const, result: stream(table) }
    const report = explain(host)
    assert.ok(report.includes('Protocol:              JsonEvents/1 → TableRows/1 → Text\n'), report)
    assert.ok(report.includes("CSV quoting:           always (the host's renderer)\n"), report)
    assert.equal((explainJson(host) as any).renderer.host, true)
  })
})
