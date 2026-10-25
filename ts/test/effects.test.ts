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

import { Plan, Val, analyze, explain, explainJson, stdlib, summarize, summaryText, value } from '../dist/alchemy'
import { Selector } from '../dist/shared'

import { PROGRAM, SPEC_DIR, compile } from './common'

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
  // A rewritten tree: the items a scan-emit hands back, said to be events,
  // are JSON events for the host, and the report names the protocol.
  it('a rewritten tree is JSON events for the host', () => {
    const src =
      'def step [s event]\n  match event\n    case (scalar "secret") (transition s [(scalar "***")])\n    case _ (transition s [event])\n\ndef export [input]\n  as-events (scan-emit [] step (fn [s] []) (events input))'
    const p = compile(src, 'check')
    assert.equal(p.output, 'JsonEvents/1')
    assert.equal(p.planOutput(), 'JsonEvents/1')
    const j: any = p.explainJson()
    assert.deepStrictEqual(j.chain, ['events', 'scan-emit', 'as-events'])
    assert.deepStrictEqual(j.protocol, ['JsonEvents/1', 'Stream<Event>', 'Stream<Value>', 'JsonEvents/1', 'Text'])
    assert.deepStrictEqual(j.renderer, { name: 'json', indent: null, trailing_newline: true, host: true })
    assert.equal(j.confidence, 'conditional')
    const report = p.explain()
    assert.ok(report.startsWith('export: events → scan-emit → as-events\n\n'), report)
    assert.ok(report.includes('Protocol:              JsonEvents/1 → Stream<Event> → Stream<Value> → JsonEvents/1 → Text\n'), report)
    // The stream `events` yields is a rewritten tree as it is.
    const tree = compile('def export [input] (events input)', 'check')
    assert.equal(tree.output, 'JsonEvents/1')
    assert.deepStrictEqual(tree.explainJson().protocol, ['JsonEvents/1', 'Stream<Event>', 'Text'])
  })

  it('the JSON echo, the identity and a text of its own', () => {
    const echo = 'def export [input]\n  json input'
    assert.equal(explain(program(echo, text({ p: 'json', source: INPUT, nonFinite: 'reject' }))), pinned(echo))
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

  // The reports above, from plans built by hand, are the ones the compiled
  // programs' own plans give: the evaluator builds what the hand built.
  it('a compiled program reports as its hand-built plan does', () => {
    const programs = [
      'def export [input]\n  json input',
      'def export [input] input',
      'def export [input] "done"',
      'def export [input] (replace-text "a" "b" (concat "x" "a"))',
      'def key-lines [s event]\n  match event\n    case (key name) (transition s [(quoted name) "\\n"])\n    case _ (transition s [])\n\ndef export [input]\n  join ""\n    scan-emit [] key-lines (fn [s] []) (events input)',
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
  })
})

// rs/src/effects.rs's tests, over compiled programs: the plan the
// evaluator built. The one that
// also runs its program (a custom csv dialect is reported as it runs, and
// writes what it reports) is dmog's, in its ts/test/effects.test.ts.
describe('explain, compiled', () => {
  it('the worked example reports in the spec layout', () => {
    const program = compile(PROGRAM, 'export.alc')
    assert.equal(
      explain(program),
      'export: api-table → csv\n' +
        '\n' +
        'Source reads:          1\n' +
        'Protocol:              JsonEvents/1 → Stream<Selected> → Stream<Value> → TableRows/1 → Text\n' +
        'Selection:             shared prefix matcher, two capture routes\n' +
        'Duplicate members:     rejected in captured scopes (DUPLICATE_MEMBER)\n' +
        'Capture:               .response.metadata.fields, capped at max_metadata_bytes\n' +
        'Row capture:           one .response.payload.deep.records[*], capped at max_record_bytes\n' +
        "Retained state:        the table's columns once bound, at most max_columns of them, no deeper than max_depth, capped at max_metadata_bytes\n" +
        'Output order:          schema first; cells in schema order\n' +
        'Ordering contract:     each item before its outputs\n' +
        'Contract verification: runtime\n' +
        'External storage:      disabled\n' +
        '\n' +
        'Guarantee:\n' +
        '  Memory is independent of the number of rows under the configured\n' +
        '  depth, capture, metadata, scalar/key and output limits: the state the\n' +
        '  step returns is capped at max_metadata_bytes. What one step computes\n' +
        "  is the program's, bounded by the host's abort flag.\n" +
        '\n' +
        'Qualification:\n' +
        '  A later error can occur after earlier output has been written.\n',
    )
    const j: any = explainJson(program)
    assert.deepStrictEqual(j.chain, ['api-table', 'csv'])
    assert.deepStrictEqual(j.protocol, ['JsonEvents/1', 'Stream<Selected>', 'Stream<Value>', 'TableRows/1', 'Text'])
    assert.equal(j.retention[0].scope, 'subtree')
    assert.equal(j.retention[1].limit, 'max_record_bytes')
    assert.equal(j.retention[2].scope, 'state')
    assert.equal(j.confidence, 'conditional')
    assert.equal(j.readiness, 'record')
    assert.equal(j.renderer.name, 'text')
    assert.equal(j.output, 'Text')
    assert.equal(j.order_constraints[0].enforcement, 'static')
    assert.equal(j.order_constraints[1].enforcement, 'runtime')
  })

  // An inferred binding routes one capture, the rows, keeps the columns it
  // took from the first row as its state under `max_columns`, and orders
  // nothing before the rows, since there is no metadata.
  it('an inferred binding reports one route', () => {
    const src =
      'def b (record (entry :columns :infer) (entry :rows (path each-index)))\ndef export [input] (csv csv-options (table-from-json b input))'
    const program = compile(src, 'inferred.alc')
    const s = summarize(program)
    assert.equal(s.selection, 'shared prefix matcher, one capture route')
    assert.deepStrictEqual(
      s.retention.map((r) => [r.scope, r.label, r.limit]),
      [
        ['record', 'Row capture:', 'max_record_bytes'],
        ['state', 'Retained state:', 'max_metadata_bytes'],
      ],
    )
    const state = s.retention.find((r) => 'state' === r.scope)!
    assert.ok(state.reason.includes('at most max_columns'), state.reason)
    assert.ok(s.orderConstraints.every((o) => 'metadata completes' !== o.before))
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
      ['subtree', 'record', 'state'],
    )
    const plain = compile('def export [input] (join "" (map (fn [e] "x") (events input)))', 'plain.alc')
    assert.equal(summarize(plain).selection, 'none; every event is delivered as an item')
  })

  it('the library table reports its route and state', () => {
    const program = compile(PROGRAM, 'export.alc')
    const s = summarize(program)
    // The library's `csv` validates the table events it renders
    // (`csv-table`), so the chain passes through TableRows/1.
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
    assert.ok(text.includes('Protocol:              JsonEvents/1 → Stream<Selected> → Stream<Value> → TableRows/1 → Text\n'), text)
    assert.ok(text.includes("CSV quoting:           always (the host's renderer)\n"), text)
    assert.equal((explainJson(table) as any).renderer.host, true)
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
