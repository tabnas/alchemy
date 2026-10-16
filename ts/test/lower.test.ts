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
  sourceFile,
  span,
  value,
} from '../dist/alchemy'
import { make as makeJson } from '@tabnas/json'

import { Lowering, NoColumns, ParserEventsOnly, ParserEventsToItems, treeSpec } from '../dist/lower'
import { CsvOptions, Datum, Limits, Metrics, fromJSON, parserEventFromJSON } from '../dist/shared'
import type { ParserEvent, TableEvent, TableSink } from '../dist/shared'

import { OPTIONS, PROGRAM, compile, thrown } from './common'

const V = value

// The json grammar's spec, as the plan writes it.
const JSON_SPEC =
  'def json-spec\n  record\n    entry :map (record (entry :container :object))\n    entry :list (record (entry :container :array))\n    entry :pair (record (entry :key 0))\n    entry :val (record (entry :scalar 0))\n'

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

  // `:non-finite` and `:no-columns`, when the record has them, are
  // policies, which the lowering applies around the renderer; one that is
  // none maps to no dialect, and the library's `csv` runs, which refuses
  // it where it is read.
  it('a csv options record maps with a non-finite policy and not with another value', () => {
    const base = '(entry :delimiter ",") (entry :newline "\\r\\n") (entry :header true) (entry :null-text "") (entry :missing :error)'
    const rt = runtime(
      `def lit (record ${base} (entry :non-finite :literal))\ndef nul (record ${base} (entry :non-finite :null))\ndef nope (record ${base} (entry :non-finite :nope))\ndef one (record ${base} (entry :non-finite 1))\ndef empty (record ${base} (entry :no-columns :empty))\ndef refuse (record ${base} (entry :no-columns :refuse))\ndef nocols (record ${base} (entry :no-columns :nope))`,
      true,
    )
    const options = (name: string) => csvOptions(run(rt.defValue('program', name)) as value.Val)
    assert.deepStrictEqual(options('lit'), CsvOptions.default())
    assert.deepStrictEqual(options('nul'), CsvOptions.default())
    assert.equal(options('nope'), undefined)
    assert.equal(options('one'), undefined)
    // `:no-columns` the same way: a policy maps, anything else does not.
    assert.deepStrictEqual(options('empty'), CsvOptions.default())
    assert.deepStrictEqual(options('refuse'), CsvOptions.default())
    assert.equal(options('nocols'), undefined)
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
    // A rewritten tree the program says is events: the rows are still the
    // select's, behind as-events.
    const rewritten = compile(
      'def export [input] (as-events (map (fn [x] (scalar x)) (select (path "a" each-index) input)))',
      't.alc',
    )
    assert.equal(rewritten.output, 'JsonEvents/1')
    assert.equal(rewritten.rowSelector()!.toString(), '.a[*]')
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

  // The table `:no-columns :empty` keeps back from the renderer is held to
  // what the library's `csv-table` holds it to, with its texts: a clean
  // empty table reaches the renderer not at all, and a table of columns
  // reaches it whole, a later schema too, for the renderer to refuse.
  it('no-columns holds the table it keeps back', () => {
    // The events through a fresh adapter: what reached the renderer, an
    // event to a line, or the failure's text.
    const run = (events: TableEvent[]): string[] | string => {
      const seen: string[] = []
      const log: TableSink = {
        tableEvent(ev: TableEvent) {
          seen.push(
            'schema' === ev.type ? `schema ${ev.columns.length}` : 'row' === ev.type ? `row ${ev.cells.length}` : 'end',
          )
          return 'continue'
        },
      }
      const n = new NoColumns(log)
      try {
        for (const ev of events) n.tableEvent(ev)
      } catch (e: any) {
        assert.equal(e.code, 'PROTOCOL_ORDER_ERROR', String(e))
        return e.message
      }
      return seen
    }
    const schema = (...labels: string[]): TableEvent => ({
      type: 'schema',
      columns: labels.map((label) => ({ label })),
    })
    const row = (...cells: number[]): TableEvent => ({
      type: 'row',
      cells: cells.map((value) => ({ type: 'number', value, lexeme: null })),
    })
    const end: TableEvent = { type: 'end' }
    // A clean empty table: nothing reaches the renderer.
    assert.deepStrictEqual(run([schema(), row(), row(), end]), [])
    // Each refusal, with the library's text.
    assert.equal(run([schema(), row(), row(1)]), 'row 2 has 1 cells; the schema has 0 columns')
    assert.equal(run([schema(), schema('a')]), 'a second schema')
    assert.equal(run([schema(), end, schema()]), 'a schema after the end')
    assert.equal(run([schema(), end, row()]), 'a row after the end')
    assert.equal(run([schema(), end, end]), 'a second end')
    // A table of columns passes as it came, and so does a later empty
    // schema, for the renderer to refuse.
    assert.deepStrictEqual(run([schema('a'), row(1), end, schema()]), ['schema 1', 'row 1', 'end', 'schema 0'])
  })

  // The host pushes the parser's rule passes into the head `parser-events`
  // is lowered to: each pass reaches the items below as `(open rule)` or
  // `(close rule)`, a field the pass lacks left out of the rule, and the
  // parse's end as the stream's end. A program that reads nothing of its
  // input hands the end alone to the sink that writes its text.
  it('parser events are items pushed into the head', () => {
    const passes = [
      parserEventFromJSON([
        'open',
        { name: 'val', d: 1, i: 1, alt: { p: '', r: '', b: 0, g: ['json'] }, tokens: [['#NR', 2.5, '2.50', 1, 1]], forced: false, n: { dmap: 1 } },
      ]),
      parserEventFromJSON(['close', { name: 'val', d: 1, i: 1, tokens: [], forced: false, n: {}, value: 2.5 }]),
      parserEventFromJSON(['end']),
    ]
    const plan = { p: 'parser-events', source: { p: 'parser-input' } }
    const seen: string[] = []
    const down = {
      item(v: any) {
        seen.push(V.debugText(v))
        return 'continue'
      },
      end() {
        seen.push('end')
        return 'continue'
      },
    }
    const here: any = new Lowering(runtime('def export [input] (parser-events input)', true), Limits.default(), new Metrics())
    const events = here.items(plan, down, false)
    // The parser events are read once.
    assert.equal(thrown(() => here.items(plan, down, false)).code, 'STREAM_REUSED')
    const head = here.parserHead(events)
    for (const pass of passes) head.event(pass)
    assert.deepStrictEqual(seen, [
      '(open (record (entry :name "val") (entry :d 1) (entry :i 1) (entry :alt (record (entry :p "") (entry :r "") (entry :b 0) (entry :g ["json"]))) (entry :tokens [(record (entry :name "#NR") (entry :val 2.5) (entry :src "2.50") (entry :row 1) (entry :col 1))]) (entry :forced false) (entry :n (record (entry :dmap 1)))))',
      '(close (record (entry :name "val") (entry :d 1) (entry :i 1) (entry :tokens []) (entry :forced false) (entry :n (record)) (entry :value 2.5)))',
      'end',
    ])
    // The JSON-events side of a parser program is never driven.
    const err = thrown(() => new ParserEventsOnly().event({ type: 'null' }))
    assert.ok(err.message.startsWith('protocol_mismatch: '), String(err))
    // A host's malformed event is refused, never read: an open or a close
    // without its pass, and a kind that is none.
    for (const bad of [{ type: 'open' }, { type: 'close', pass: null }, { type: 'nope', pass: {} }]) {
      const fail = thrown(() => new ParserEventsToItems(down as any).event(bad as any))
      assert.equal(fail.code, 'PROTOCOL_ORDER_ERROR', JSON.stringify(bad))
    }

    const text: string[] = []
    const nothing: any = new Lowering(runtime('def export [input] "done"', true), Limits.default(), new Metrics())
    const finite = nothing.parserHead({
      event(ev: any) {
        text.push(ev.type)
        return 'continue'
      },
    })
    for (const pass of passes) finite.event(pass)
    assert.deepStrictEqual(text, ['end'])
  })

  // A spec of the standard shape is read as one; a spec of any other shape
  // is not, and runs the library's text (rs/src/lower.rs's test).
  it('a transduce-tree spec of the standard shape is read', () => {
    const rt = runtime(
      JSON_SPEC +
        'def named (record (entry :doc (record (entry :key "k") (entry :scalar :value) (entry :note 1))))\n' +
        'def odd-container (record (entry :map (record (entry :container :set))))\n' +
        'def odd-key (record (entry :pair (record (entry :key 1.5))))\n' +
        'def odd-scalar (record (entry :val (record (entry :scalar "x"))))\n' +
        'def odd-entry (record (entry :val 1))\n',
      true,
    )
    const val = (name: string) => run(rt.defValue('program', name)) as value.Val
    assert.deepStrictEqual(
      treeSpec(val('json-spec')),
      {
        rules: new Map<string, unknown>([
          ['map', { container: 'object' }],
          ['list', { container: 'array' }],
          ['pair', { key: { token: 0 } }],
          ['val', { scalar: { token: 0 } }],
        ]),
      },
    )
    // A field the library never reads is not read here either.
    assert.deepStrictEqual(treeSpec(val('named')), {
      rules: new Map([['doc', { key: { name: 'k' }, scalar: 'value' }]]),
    })
    for (const odd of ['odd-container', 'odd-key', 'odd-scalar', 'odd-entry']) {
      assert.equal(treeSpec(val(odd)), undefined, odd)
    }
    assert.equal(treeSpec(V.NULL), undefined)
  })

  // The library's `transduce-tree` over the host's parser events runs
  // natively where the routers offer it, the source's guard in front of
  // what it writes, and through the library's scan where they decline,
  // where the natives are off, and where the spec is not of the standard
  // shape. A program that writes the library's scan itself is run the same
  // way; one whose step is its own is not (rs/src/lower.rs's test).
  it('transduce-tree runs natively where the routers offer it', () => {
    const lower = (exp: string, native: boolean, offer: boolean | 'absent'): string[] => {
      const log: string[] = []
      const routers: any = {
        ...OPTIONS.routers,
        scanEmit() {
          log.push('scan-emit')
          return { item: () => 'continue', finish: () => 'continue' }
        },
        guarded(inner: any) {
          log.push('guarded')
          return inner
        },
      }
      if ('absent' !== offer) {
        routers.transduceTree = (spec: any, downstream: any) => {
          log.push(`transduce-tree of ${spec.rules.size} rules`)
          if (!offer) return undefined
          return {
            event(ev: any) {
              return 'end' === ev.type ? downstream.event({ type: 'end' }) : 'continue'
            },
          }
        }
      }
      const src = JSON_SPEC + exp
      const forms = desugarProgram(parseFile(src, 't.alc'), src)
      const sources = Sources.one('t.alc', src)
      const rt = new Runtime(resolve(forms, sources, outer), sources, { ...OPTIONS, routers }).withNative(native)
      const result = run(rt.export('ParserEvents/1')) as any
      const lowering: any = new Lowering(rt, Limits.default(), new Metrics())
      const events = lowering.events(result.plan, {
        event(ev: any) {
          log.push(ev.type)
          return 'continue'
        },
      })
      lowering.parserHead(events).event({ type: 'end' })
      return log
    }
    const tree = 'def export [input] (transduce-tree json-spec input)\n'
    const native = ['guarded', 'transduce-tree of 4 rules', 'end']
    const scan = ['guarded', 'scan-emit', 'end']
    assert.deepStrictEqual(lower(tree, true, true), native)
    assert.deepStrictEqual(lower(tree, true, false), ['guarded', 'transduce-tree of 4 rules', 'scan-emit', 'end'])
    assert.deepStrictEqual(lower(tree, true, 'absent'), scan)
    assert.deepStrictEqual(lower(tree, false, true), scan)
    assert.deepStrictEqual(
      lower('def export [input] (transduce-tree (record (entry :map (record (entry :container :set)))) input)\n', true, true),
      scan,
    )
    assert.deepStrictEqual(
      lower(
        'def export [input] (as-events (scan-emit [[] false] (partial transduce-step json-spec) transduce-finish (parser-events input)))\n',
        true,
        true,
      ),
      native,
    )
    assert.deepStrictEqual(
      lower(
        'def my-step [spec state pass] (transduce-step spec state pass)\ndef export [input] (as-events (scan-emit [[] false] (partial my-step json-spec) transduce-finish (parser-events input)))\n',
        true,
        true,
      ),
      scan,
    )
  })

  // The json grammar's part, as the plan writes it: `transduce-tree` over
  // tabnas-json's own rule passes yields the document's events, lexemes
  // kept, in the order a walk of its value gives them (rs/src/lower.rs's
  // test). The scan runs here by hand, the step applied to each item as
  // transduce's scan-emit applies it, since running the plan itself needs
  // transduce's stages.
  it('transduce-tree makes JSON events of tabnas-json rule passes', () => {
    const rt = runtime(
      'def json-spec\n  record\n    entry :map (record (entry :container :object))\n    entry :list (record (entry :container :array))\n    entry :pair (record (entry :key 0))\n    entry :val (record (entry :scalar 0))\n\ndef export [input] (transduce-tree json-spec input)\n',
      true,
    )
    const spec = run(rt.defValue('program', 'json-spec'))!
    const step: any = run(rt.defValue('stdlib', 'transduce-step'))
    const finish: any = run(rt.defValue('stdlib', 'transduce-finish'))
    const at = span(sourceFile('t'), 0, 0)
    // tabnas-json's passes, as a parser-events source makes them: each
    // pass's own tokens (those its alternate gave back are the next
    // rule's), a number token's value with its source text as the lexeme,
    // a close pass's scalar node as its value, then one end.
    const passes = (src: string): ParserEvent[] => {
      const out: ParserEvent[] = []
      const parser = makeJson()
      parser.sub({
        ruleDone: (r: any, _ctx: any, done: any) => {
          const open = 'o' === done.state
          const all: any[] = open ? r.o : r.c
          const b = null == done.alt ? 0 : done.alt.b
          const tokens = all.slice(0, all.length - b).map((t: any) => ({
            name: t.name,
            val: '#NR' === t.name ? Datum.number(t.val, t.src) : undefined === t.val ? Datum.null : fromJSON(t.val),
            src: t.src,
            row: t.rI,
            col: t.cI,
          }))
          const node = r.node
          const scalar = !open && undefined !== node && (null === node || 'object' !== typeof node)
          out.push({
            type: open ? 'open' : 'close',
            pass: {
              name: r.name,
              d: r.d,
              i: r.i,
              ...(null == done.alt
                ? {}
                : { alt: { p: done.alt.p, r: done.alt.r, b: done.alt.b, g: done.alt.g } }),
              tokens,
              forced: true === done.forced,
              n: Object.entries(r.n as Record<string, number>).sort(),
              ...(scalar ? { value: fromJSON(node) } : {}),
            },
          })
        },
      })
      parser.parse(src)
      out.push({ type: 'end' })
      return out
    }
    const events = (src: string): string => {
      let state = V.vectorVal([V.vectorVal([]), V.bool(false)])
      const out: string[] = []
      let pending: any
      const items = new ParserEventsToItems({
        item(v: any) {
          pending = v
          return 'continue'
        },
        end() {
          return 'continue'
        },
      })
      for (const ev of passes(src)) {
        if ('end' === ev.type) {
          const tail: any = run(rt.apply(finish.f, [state], at))
          out.push(...tail.items.map((v: any) => V.debugText(v)))
          break
        }
        items.event(ev)
        const next: any = run(rt.apply(step.f, [spec, state, pending], at))
        assert.equal(next.tag, 'transition')
        out.push(...next.fields[1].items.map((v: any) => V.debugText(v)))
        state = next.fields[0]
      }
      return out.join(' ')
    }
    assert.equal(
      events('{"a":[1,2.50,"x",{"b":null}],"c":{},"d":[],"e":1e21,"f":true}'),
      'object-start (key "a") array-start (scalar 1) (scalar 2.50) (scalar "x") object-start (key "b") (scalar null) object-end array-end (key "c") object-start object-end (key "d") array-start array-end (key "e") (scalar 1e21) (key "f") (scalar true) object-end',
    )
    assert.equal(events('7'), '(scalar 7)')
    assert.equal(events('"s"'), '(scalar "s")')
    assert.equal(events('[]'), 'array-start array-end')
    assert.equal(
      events('[[[{}]]]'),
      'array-start array-start array-start object-start object-end array-end array-end array-end',
    )
  })
})
