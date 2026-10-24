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

import { Lowering, ParserEventsOnly, ParserEventsToItems } from '../dist/lower'
import { Datum, Limits, Metrics, fromJSON, parserEventFromJSON } from '../dist/shared'
import type { ParserEvent, TableEvent, TableSink } from '../dist/shared'

import { OPTIONS, PROGRAM, compile, thrown } from './common'

const V = value

function runtime(src: string): Runtime {
  const forms = desugarProgram(parseFile(src, 't.alc'), src)
  const sources = Sources.one('t.alc', src)
  return new Runtime(resolve(forms, sources, outer), sources, OPTIONS)
}

describe('program', () => {
  it('a program knows its output and row selector', () => {
    const p = compile(PROGRAM, 't.alc')
    assert.equal(p.output, 'Text')
    assert.equal(p.rowSelector()!.toString(), '.response.payload.deep.records[*]')
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
    for (const program of [p, table, echo, select, tree, said]) {
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
    const here: any = new Lowering(runtime('def export [input] (parser-events input)'), Limits.default(), new Metrics())
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
    const nothing: any = new Lowering(runtime('def export [input] "done"'), Limits.default(), new Metrics())
    const finite = nothing.parserHead({
      event(ev: any) {
        text.push(ev.type)
        return 'continue'
      },
    })
    for (const pass of passes) finite.event(pass)
    assert.deepStrictEqual(text, ['end'])
  })

  // tabnas-json's passes, as a parser-events source makes them: each
  // pass's own tokens (those its alternate gave back are the next rule's),
  // a number token's value with its source text as the lexeme, a close
  // pass's scalar node as its value, then one end.
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

  // The scan a `transduce-tree` builds, run here by hand over tabnas-json's
  // own rule passes: `step` applied to each item as transduce's scan-emit
  // applies it, from `init`, and `finish` to the last state, since running
  // the plan itself needs transduce's stages (rs/src/lower.rs's
  // `tree_scan`).
  const treeScan = (rt: Runtime, specName: string, stepName: string, finishName: string, init: value.Val, src: string): string => {
    const spec = run(rt.defValue('program', specName))!
    const step: any = run(rt.defValue('stdlib', stepName))
    const finish: any = run(rt.defValue('stdlib', finishName))
    const at = span(sourceFile('t'), 0, 0)
    let state = init
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

  // The json grammar's part, as the plan writes it: `transduce-tree` over
  // tabnas-json's own rule passes yields the document's events, lexemes
  // kept, in the order a walk of its value gives them, through
  // transduce-step and through transduce-tree's own step alike
  // (rs/src/lower.rs's test).
  it('transduce-tree makes JSON events of tabnas-json rule passes', () => {
    const rt = runtime(
      'def json-spec\n  record\n    entry :map (record (entry :container :object))\n    entry :list (record (entry :container :array))\n    entry :pair (record (entry :key 0))\n    entry :val (record (entry :scalar 0))\n\ndef export [input] (transduce-tree json-spec input)\n',
    )
    const tree = () => V.vectorVal([V.vectorVal([]), V.bool(false)])
    const stepped = (src: string) => treeScan(rt, 'json-spec', 'transduce-step', 'transduce-finish', tree(), src)
    const passed = (src: string) =>
      treeScan(rt, 'json-spec', 'transduce-pass', 'transduce-done', V.vectorVal([tree(), V.vectorVal([])]), src)
    for (const events of [stepped, passed]) {
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
    }
  })

  // transduce-tree's own step reads `:member` and `:text`: a member after
  // the container its rule begins, and a `:text` rule's text, the `:src`
  // of the tokens the passes between its open and its close consumed,
  // first on its close; a `:text` rule closed inside another joins its
  // text to the one around it, with its own passes' tokens
  // (rs/src/lower.rs's test).
  it('transduce-tree reads a member and a text', () => {
    const rt = runtime(
      'def values\n  record\n    entry :map (record (entry :container :object))\n    entry :pair (record (entry :key 0) (entry :text true))\n\ndef lists\n  record\n    entry :list (record (entry :container :object) (entry :member "items") (entry :text true))\n\ndef export [input] (transduce-tree values input)\n',
    )
    const init = () => V.vectorVal([V.vectorVal([V.vectorVal([]), V.bool(false)]), V.vectorVal([])])
    const events = (spec: string, src: string) => treeScan(rt, spec, 'transduce-pass', 'transduce-done', init(), src)
    assert.equal(
      events('values', '{"a":[1, 2],"b":"x","d":7}'),
      'object-start (key "a") (scalar "[1,2]") (key "b") (scalar "\\"x\\"") (key "d") (scalar "7") object-end',
    )
    assert.equal(
      events('lists', '[1,[2,3],"x"]'),
      'object-start (key "items") object-start (key "items") (scalar "2,3") object-end (scalar "1,[2,3],\\"x\\"") object-end',
    )
    assert.equal(events('lists', '[]'), 'object-start (key "items") (scalar "") object-end')
  })
})
