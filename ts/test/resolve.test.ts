/* Copyright (c) 2026 tabnas, MIT License */

// Scopes and linking (rs/src/resolve.rs's tests), and the types
// (rs/src/types.rs's tests).

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { NameKind, Sources, defParams, desugarProgram, parse, parseFile, resolve, types } from '../dist/alchemy'

import { finer, thrown } from './common'

const T = types

function outer(name: string): NameKind | undefined {
  switch (name) {
    case 'table-end':
    case 'no-schema':
    case 'missing':
      return 'constant'
    case 'selected':
    case 'schema':
    case 'row':
      return 'constructor'
    case 'map':
    case 'get':
    case 'csv':
    case 'table-from-json':
      return 'function'
    case 'csv-options':
      return 'value'
    default:
      return undefined
  }
}

function resolved(src: string) {
  return resolve(desugarProgram(parse(src), src), Sources.one('t', src), outer)
}

function code(src: string): [string, string, number, number] {
  const f = thrown(() => resolved(src))
  return [f.code, finer(f), f.row, f.col]
}

describe('resolve', () => {
  it('definitions see each other in any order', () => {
    const r = resolved(
      'def export [input]\n  csv csv-options (api-table input)\ndef api-table [input]\n  table-from-json binding input\ndef binding 1',
    )
    assert.equal(r.defs.size, 3)
    assert.deepStrictEqual(r.references.get('export'), ['api-table'])
    assert.deepStrictEqual(r.references.get('api-table'), ['binding'])
    assert.deepStrictEqual(r.reachable('export'), ['api-table', 'binding'])
    assert.equal(defParams(r.get('binding')!), undefined)
    assert.deepStrictEqual(defParams(r.get('export')!), ['input'])
    assert.deepStrictEqual(r.dependencyOrder(), ['binding', 'api-table', 'export'])
  })

  it('locals shadow and patterns bind', () => {
    resolved(
      'def f [x]\n  let [y (map x x)]\n    match y\n      case (selected :columns raw) raw\n      case [a b] (get a b)\n      case table-end y\n      case _ x',
    )
    // `raw` is only bound inside its clause.
    const [c, f, row, col] = code('def f [x]\n  match x\n    case (selected :a raw) raw\n    case _ raw')
    assert.equal(c, 'DSL_TYPE_ERROR')
    assert.equal(f, 'unknown_name')
    assert.deepStrictEqual([row, col], [4, 12])
  })

  it('the finer codes', () => {
    assert.equal(code('def f [x] (nope x)')[1], 'unknown_name')
    assert.equal(code('csv 1')[1], 'not_def')
    assert.equal(code('def x 1\ndef x 2')[1], 'duplicate_def')
    assert.equal(code('def if 1')[1], 'reserved')
    assert.equal(code('def f (fn x x)')[1], 'bad_fn')
    assert.equal(code('def f (fn [1] x)')[1], 'bad_fn')
    assert.equal(code('def f [x] (def y x)')[1], 'misplaced_def')
    assert.equal(code('def f [x] (match x (case (map a) a))')[1], 'bad_pattern')
    assert.equal(code('def f [x] (match x (case (1 a) a))')[1], 'bad_pattern')
    const [c, f, row] = code('def a [x] (b x)\ndef b [x] (c x)\ndef c [x] (a x)')
    assert.equal(c, 'STREAMABILITY_UNKNOWN')
    assert.equal(f, 'recursion')
    assert.equal(row, 1)
    assert.equal(code('def a [x] (a x)')[1], 'recursion')
    assert.equal(code('def a (fn [x] (b x))\ndef b [y] (a y)')[1], 'recursion')
    const fail = thrown(() => resolved('def a [x] (b x)\ndef b [x] (c x)\ndef c [x] (a x)'))
    assert.match(fail.message, /^recursion: a reaches itself through b, then c; strict mode refuses recursion$/)
  })

  it('a program may shadow a library name', () => {
    const r = resolved('def csv [x] x\ndef export [input] (csv input)')
    assert.deepStrictEqual(r.references.get('export'), ['csv'])
  })

  it('a duplicate across several sources names where the first is', () => {
    const a = 'def x 1'
    const b = '\ndef x 2'
    const linked = Sources.several([
      ['a.alc', a],
      ['b.alc', b],
    ])
    const fail = thrown(() =>
      resolve(
        [...desugarProgram(parseFile(a, 'a.alc'), a), ...desugarProgram(parseFile(b, 'b.alc'), b)],
        linked,
        outer,
      ),
    )
    assert.equal(fail.message, 'duplicate_def: x is defined twice; first at a.alc:1:1')
    assert.deepStrictEqual([fail.file, fail.row, fail.col], ['b.alc', 2, 1])
  })

  it('a long chain of definitions resolves without nesting', () => {
    let src = 'def a0 1\n'
    for (let i = 1; i < 10_000; i++) src += `def a${i} a${i - 1}\n`
    const r = resolved(src)
    assert.equal(r.reachable('a9999').length, 9999)
    assert.equal(r.dependencyOrder()[0], 'a0')
  })
})

describe('types', () => {
  it('unknown and never are accepted everywhere', () => {
    for (const t of [T.Text, T.JsonEvents, T.Record, T.tableEvents()]) {
      assert.ok(T.accepts(t, T.Unknown))
      assert.ok(T.accepts(t, T.Never))
      assert.ok(T.accepts(T.Unknown, t))
    }
  })

  it('value accepts data and nothing else', () => {
    assert.ok(T.accepts(T.Value, T.Num))
    assert.ok(T.accepts(T.Value, T.vectorOf(T.Str)))
    assert.ok(T.accepts(T.Value, T.tagged('missing')))
    assert.ok(!T.accepts(T.Value, T.Selector))
    assert.ok(!T.accepts(T.Value, T.Text))
    assert.ok(!T.accepts(T.Value, T.funcOf(1)))
  })

  it('a value passes where particular data is wanted', () => {
    for (const t of [T.Record, T.Num, T.Str, T.Null, T.vectorOf(T.Value), T.vectorOf(T.Record), T.tagged('missing')]) {
      assert.ok(T.accepts(t, T.Value), T.typeText(t))
    }
    assert.ok(T.accepts(T.vectorOf(T.Record), T.vectorOf(T.Value)))
    for (const t of [
      T.Text,
      T.Selector,
      T.Keyword,
      T.TableEvent,
      T.JsonEvents,
      T.streamOf(T.Value),
      T.funcOf(1),
      T.tagged('row'),
    ]) {
      assert.ok(!T.accepts(t, T.Value), T.typeText(t))
    }
    assert.ok(!T.accepts(T.Record, T.Num))
    assert.ok(!T.accepts(T.vectorOf(T.Value), T.Record))
    assert.ok(!T.accepts(T.Record, T.vectorOf(T.Value)))
  })

  it('streams and table events', () => {
    const table = T.tableEvents()
    assert.ok(T.accepts(table, T.streamOf(T.Unknown)))
    assert.ok(T.accepts(table, T.streamOf(T.tagged('row'))))
    assert.ok(!T.accepts(table, T.streamOf(T.tagged('selected'))))
    assert.ok(!T.accepts(table, T.JsonEvents))
    assert.ok(!T.accepts(T.JsonEvents, table))
    const events = T.events()
    assert.ok(T.accepts(events, T.streamOf(T.tagged('key'))))
    assert.ok(T.accepts(events, T.streamOf(T.tagged('object-end'))))
    assert.ok(!T.accepts(events, T.streamOf(T.tagged('row'))))
    assert.ok(!T.accepts(table, events))
    assert.ok(!T.accepts(events, table))
    assert.ok(!T.accepts(events, T.JsonEvents))
    assert.ok(T.accepts(T.JsonEvents, events))
    assert.ok(T.accepts(T.JsonEvents, T.streamOf(T.Unknown)))
    assert.ok(!T.accepts(T.JsonEvents, T.streamOf(T.Value)))
    assert.ok(T.isAffine(events) && T.isProtocol(events))
    assert.equal(T.typeText(events), 'Stream<Event>')
    assert.ok(T.isAffine(table) && T.isAffine(T.Text) && !T.isAffine(T.Str))
    assert.equal(T.typeText(table), 'TableEvents')
    assert.equal(T.typeText(T.streamOf(T.Value)), 'Stream<Value>')
    assert.equal(T.typeText(T.func([T.Record, T.JsonEvents], table)), 'Fn(Record JsonEvents -> TableEvents)')
    assert.equal(T.typeText(T.func([], T.Unknown)), 'Fn( -> Unknown)')
  })

  it('functions by arity and result', () => {
    const f = T.func([T.Unknown, T.Unknown], T.Text)
    assert.ok(T.accepts(f, T.func([T.Record, T.Value], T.Text)))
    assert.ok(!T.accepts(f, T.funcOf(1)))
    assert.ok(T.accepts(T.funcOf(2), f))
  })

  it('joins', () => {
    assert.ok(T.typeEq(T.join(T.Never, T.Text), T.Text))
    assert.ok(T.typeEq(T.join(T.Text, T.Str), T.Text))
    assert.ok(T.typeEq(T.join(T.Num, T.Str), T.Value))
    assert.ok(T.typeEq(T.join(T.Text, T.Record), T.Unknown))
    assert.ok(T.typeEq(T.join(T.vectorOf(T.Num), T.vectorOf(T.Null)), T.vectorOf(T.Value)))
  })
})
