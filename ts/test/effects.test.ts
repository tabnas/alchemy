/* Copyright (c) 2026 tabnas, MIT License */

// The plan report (rs/src/effects.rs). Each program's plan is built here
// by hand, as the evaluator's native path builds it, and the report is
// held to the check.tsv row that pins it: the same text, byte for byte, so
// the summary is tested apart from the evaluator. The front end decides
// the rest of what the report reads (the output, the definitions). The
// last test holds each hand-built plan's report to the one the compiled
// program's own plan gives (and spec.test.ts runs check.tsv through
// `compile`).

import { describe, it } from 'node:test'
import assert from 'node:assert'
import { join } from 'node:path'

import { loadSpec, parseExpect } from '@tabnas/support'
import { Selector } from '@tabnas/transduce'

import { Plan, Val, analyze, compile, explain, explainJson, stdlib, summarize, summaryText, value } from '../dist/alchemy'

import { PROGRAM, RECORDS, SPEC_DIR, ok } from './common'

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
    const step = { fn: 'closure', closure: { name: 'key-lines', params: ['s', 'event'], body: p.resolved.get('key-lines')!.value, env: null, scope: 'program', span: SP } } as const
    const finish = { fn: 'closure', closure: { params: ['s'], body: p.resolved.get('key-lines')!.value, env: null, scope: 'program', span: SP } } as const
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
        f: { fn: 'closure', closure: { name: 'table-step', params: ['binding', 'state', 'event'], body: tableStep.value, env: null, scope: 'stdlib', span: tableStep.span } },
        args: [],
      },
    } as const
    const finish = { fn: 'closure', closure: { params: ['s'], body: tableStep.value, env: null, scope: 'program', span: SP } } as const
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
  // The reports above, from plans built by hand, are the ones the compiled
  // programs' own plans give: the evaluator builds what the hand built.
  it('a compiled program reports as its hand-built plan does', () => {
    const programs = [
      'def export [input]\n  json input',
      'def export [input] input',
      'def export [input] "done"',
      'def export [input] (replace-text "a" "b" (concat "x" "a"))',
      'def key-lines [s event]\n  match event\n    case (key name) (transition s [(quoted name) "\\n"])\n    case _ (transition s [])\n\ndef export [input]\n  join ""\n    scan-emit [] key-lines (fn [s] []) (events input)',
      'def rows-binding\n  record\n    entry :columns :infer\n    entry :rows (path each-index)\n\ndef export [input]\n  pipe input\n    table-from-json rows-binding\n    csv csv-options',
    ]
    for (const src of programs) {
      const compiled = compile(src, 'check')
      const p = program(src, compiled.result)
      assert.equal(compiled.explain(), explain(p), src)
      assert.deepStrictEqual(compiled.explainJson(), explainJson(p), src)
    }
    for (const src of programs.slice(0, 3).concat(programs.slice(4))) {
      assert.equal(compile(src, 'check').explain(), pinned(src), src)
    }
    // The custom dialect, compiled: its own options, as it runs.
    const lf =
      'def lf\n  record\n    entry :delimiter ";"\n    entry :newline "\\n"\n    entry :header false\n    entry :null-text "NULL"\n    entry :missing "-"\n\n' +
      'def rows-binding\n  record\n    entry :columns :infer\n    entry :rows (path each-index)\n\n' +
      'def export [input] (csv lf (table-from-json rows-binding input))'
    const r: any = (compile(lf, 'lf.alc').explainJson() as any).renderer
    assert.deepStrictEqual([r.delimiter, r.newline, r.header, r.null_text, r.missing, r.missing_text, r.host], [
      ';',
      '\n',
      false,
      'NULL',
      'text',
      '-',
      false,
    ])
  })
})

// rs/src/effects.rs's tests, over compiled programs: the plan the
// evaluator built, natively and through the library's text.
describe('explain, compiled', () => {
  it('the worked example reports in the spec layout', () => {
    const program = compile(PROGRAM, 'export.alc')
    assert.equal(
      explain(program),
      'export: api-table → csv\n' +
        '\n' +
        'Source reads:          1\n' +
        'Protocol:              JsonEvents/1 → TableRows/1 → Text\n' +
        'Selection:             shared prefix matcher, two capture routes\n' +
        'Duplicate members:     rejected in captured scopes (DUPLICATE_MEMBER)\n' +
        'Retained metadata:     .response.metadata.fields, capped at max_metadata_bytes\n' +
        'Row capture:           one .response.payload.deep.records[*], capped at max_record_bytes\n' +
        'Output order:          schema first; cells in schema order\n' +
        'Ordering contract:     metadata completes before first row begins\n' +
        'Contract verification: runtime\n' +
        'CSV quoting:           always\n' +
        'External storage:      disabled\n' +
        '\n' +
        'Guarantee:\n' +
        '  Memory is independent of the number of rows under the configured\n' +
        '  depth, metadata, record, scalar/key and output limits.\n' +
        '\n' +
        'Qualification:\n' +
        '  Valid JSON that violates the metadata-first contract is rejected.\n' +
        '  A later error can occur after earlier output has been written.\n',
    )
    const j: any = explainJson(program)
    assert.deepStrictEqual(j.chain, ['api-table', 'csv'])
    assert.deepStrictEqual(j.protocol, ['JsonEvents/1', 'TableRows/1', 'Text'])
    assert.equal(j.retention[0].scope, 'metadata')
    assert.equal(j.retention[1].limit, 'max_record_bytes')
    assert.equal(j.confidence, 'proven')
    assert.equal(j.readiness, 'record')
    assert.equal(j.renderer.name, 'csv')
    assert.equal(j.output, 'Text')
    assert.equal(j.order_constraints[0].enforcement, 'runtime')
  })

  // An inferred binding reads one capture route, retains the columns it
  // took from the first row under `max_columns`, and orders nothing before
  // the rows; the library's twin routes one capture too.
  it('an inferred binding reports one route', () => {
    const src =
      'def b (record (entry :columns :infer) (entry :rows (path each-index)))\ndef export [input] (csv csv-options (table-from-json b input))'
    const program = compile(src, 'inferred.alc')
    const s = summarize(program)
    assert.equal(s.selection, 'shared prefix matcher, one capture route')
    assert.deepStrictEqual(
      s.retention.map((r) => [r.scope, r.label, r.limit]),
      [
        ['metadata', 'Inferred columns:', 'max_metadata_bytes'],
        ['record', 'Row capture:', 'max_record_bytes'],
      ],
    )
    assert.ok(s.orderConstraints.every((o) => 'metadata completes' !== o.before))
    assert.ok(program.explain().includes('Inferred columns:'))
    const twin = summarize(program.withNative(false))
    assert.equal(twin.selection, 'shared prefix matcher, one capture route')
    // The twin's state is the table's columns, and it reports their
    // count's cap as the native table does.
    const state = twin.retention.find((r) => 'state' === r.scope)!
    assert.ok(state.reason.includes('at most max_columns'), state.reason)
    assert.equal(state.limit, 'max_metadata_bytes')
  })

  // A program may run the library's `table-step` itself. Seeded with
  // columns already bound, the scan builds no `schema`, so nothing holds
  // the columns to `max_columns` and the report does not claim it; seeded
  // with `no-schema`, it is the library's table and does.
  it('a table step seeded with columns claims no column cap', () => {
    const src = (init: string) =>
      'def b (record (entry :columns :infer) (entry :rows (path each-index)))\n' +
      'def export [input]\n  join ""\n    map (fn [e] "x")\n      ' +
      `scan-emit ${init} (partial table-step b) (fn [s] [])\n        ` +
      'route (table-captures b) input'
    const state = (init: string) =>
      summarize(compile(src(init), 'step.alc')).retention.find((r) => 'state' === r.scope)!
    const seeded = state('(ready [])')
    assert.equal(seeded.reason, 'what the step returns, no deeper than max_depth')
    assert.equal(seeded.limit, 'max_metadata_bytes')
    assert.ok(state('no-schema').reason.includes('at most max_columns'))
  })

  // `events` after a stage that selected keeps that stage's selection in
  // the report: the stages are read from the input outward.
  it('events after a selecting stage keep its selection', () => {
    const program = compile(
      PROGRAM.replace('    csv csv-options\n', '    records\n    events\n    map (fn [e] "x")\n    join ""\n'),
      'events.alc',
    )
    const s = summarize(program)
    assert.equal(s.selection, 'shared prefix matcher, two capture routes')
    assert.ok(s.protocol.includes('Stream<Event>'), String(s.protocol))
    assert.deepStrictEqual(
      s.retention.map((r) => r.scope),
      ['metadata', 'record'],
    )
    const plain = compile('def export [input] (join "" (map (fn [e] "x") (events input)))', 'plain.alc')
    assert.equal(summarize(plain).selection, 'none; every event is delivered as an item')
  })

  it('the interpreted library reports its route and state', () => {
    const program = compile(PROGRAM, 'export.alc').withNative(false)
    const s = summarize(program)
    // The library's `csv` validates the table events it renders
    // (`csv-table`), so the chain passes through TableRows/1 as the native
    // one does.
    assert.deepStrictEqual(s.protocol, ['JsonEvents/1', 'Stream<Selected>', 'Stream<Value>', 'TableRows/1', 'Text'])
    assert.equal(s.selection, 'shared prefix matcher, two capture routes')
    assert.equal(s.confidence, 'conditional')
    assert.deepStrictEqual(
      s.retention.map((r) => r.scope),
      ['subtree', 'record', 'state'],
    )
    const text = summaryText(s)
    assert.ok(text.includes('Capture:               .response.metadata.fields, capped at max_metadata_bytes\n'), text)
    assert.ok(
      text.includes('Row capture:           one .response.payload.deep.records[*], capped at max_record_bytes\n'),
      text,
    )
    assert.ok(
      text.includes(
        "Retained state:        the table's columns once bound, at most max_columns of them, no deeper than max_depth, capped at max_metadata_bytes\n",
      ),
      text,
    )
    assert.ok(text.includes('Contract verification: runtime\n'), text)
    assert.ok(text.includes('Ordering contract:     each item before its outputs\n'), text)
  })

  it('the json echo and a host-rendered table', () => {
    const identity = compile('def export [input] input', 'id.alc')
    let text = explain(identity)
    assert.ok(text.startsWith('export: input\n'), text)
    assert.ok(
      text.includes("JSON profile:          compact, one document, trailing newline (the host's renderer)\n"),
      text,
    )
    const table = compile(PROGRAM.replace('    csv csv-options\n', ''), 'table.alc')
    text = explain(table)
    assert.ok(text.startsWith('export: api-table\n'), text)
    assert.ok(text.includes('Protocol:              JsonEvents/1 → TableRows/1 → Text\n'), text)
    assert.ok(text.includes("CSV quoting:           always (the host's renderer)\n"), text)
    assert.equal((explainJson(table) as any).renderer.host, true)
  })

  // The CSV renderer is reported with the dialect it is built with, and
  // what it reports is what it writes.
  it('a custom csv dialect is reported as it runs', () => {
    const lf =
      PROGRAM.replace('    csv csv-options\n', '    csv lf\n') +
      '\ndef lf (record (entry :delimiter ";") (entry :newline "\\n") (entry :header false) (entry :null-text "NULL") (entry :missing "-"))\n'
    const program = compile(lf, 'lf.alc')
    assert.ok(program.native)
    const r: any = (explainJson(program) as any).renderer
    assert.deepStrictEqual(
      [r.name, r.host, r.newline, r.header, r.delimiter, r.null_text, r.missing, r.missing_text],
      ['csv', false, '\n', false, ';', 'NULL', 'text', '-'],
    )
    assert.equal(ok(program, RECORDS), '"123";"Alice";"50.25"\n"456";"Bob";"72"\n')
    // The null and missing texts reach the output as reported.
    const sparse = RECORDS.replace(
      '{"account":{"balance":72},"person":{"name":"Bob"},"id":456}',
      '{"person":{"name":null},"id":456}',
    )
    assert.notEqual(sparse, RECORDS)
    assert.equal(ok(program, sparse), '"123";"Alice";"50.25"\n"456";"NULL";"-"\n')
    // The default dialect, and the host's renderer, report the defaults.
    for (const [name, p, isHost] of [
      ['default', compile(PROGRAM, 'export.alc'), false],
      ['host', compile(PROGRAM.replace('    csv csv-options\n', ''), 'host.alc'), true],
    ] as const) {
      const d: any = (explainJson(p) as any).renderer
      assert.deepStrictEqual(
        [d.name, d.host, d.delimiter, d.newline, d.header, d.null_text, d.missing, d.missing_text],
        ['csv', isHost, ',', '\r\n', true, '', 'error', null],
        name,
      )
    }
  })

  it('duplicate members follow the policy where scopes are captured', () => {
    const program = compile(PROGRAM, 'export.alc')
    assert.equal(summarize(program).duplicates, 'rejected in captured scopes (DUPLICATE_MEMBER)')
    assert.equal(
      (explainJson(program.withDuplicates('last_wins')) as any).duplicates,
      'the last value wins in captured scopes',
    )
    assert.equal(summarize(program.withDuplicates('first_wins')).duplicates, 'the first value wins in captured scopes')
    assert.equal(
      summarize(compile('def export [input] input', 'id.alc')).duplicates,
      'preserved; the events are copied as they arrive, not mapped by key',
    )
  })
})
