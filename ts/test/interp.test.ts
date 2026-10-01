/* Copyright (c) 2026 tabnas, MIT License */

// The evaluator and the runtime values: rs/src/interp.rs's,
// rs/src/value.rs's and rs/src/stdlib/registry.rs's tests.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { AbortFlag, Datum, Selector, fromJSON, toText } from '@tabnas/transduce'

import {
  Runtime,
  Sources,
  Val,
  desugarProgram,
  fileOf,
  kindWord,
  LENGTH_CHUNK,
  native,
  numberText,
  outer,
  parseFile,
  partial,
  quote,
  quotedLen,
  resolve,
  run,
  shortestNumber,
  source,
  sourceFile,
  span,
  value,
} from '../dist/alchemy'

import { thrown } from './common'

const V = value

// A runtime over the program `src`, read as `t.alc`.
function runtime(src: string, file = 't.alc'): Runtime {
  const forms = desugarProgram(parseFile(src, file), src)
  const sources = Sources.one(file, src)
  return new Runtime(resolve(forms, sources, outer), sources)
}

// `expr` evaluated in the program `src`'s scope.
function evalIn(src: string, expr: string, rt: Runtime = runtime(src)): Val {
  const forms = desugarProgram(parseFile(expr, 'e.alc'), expr)
  return rt.evalProgramExpr(forms[0])
}

function evalFail(src: string, expr: string): any {
  return thrown(() => evalIn(src, expr))
}

function same(got: Val, want: Val, message?: string): void {
  assert.ok(V.valEquals(got, want), message ?? `${V.debugText(got)} is not ${V.debugText(want)}`)
}

const n = (x: number) => V.num(x)

describe('values', () => {
  it('a datum round trips with its lexemes', () => {
    const d = fromJSON({ a: [1, 'x', null, true], b: { c: 2.5 } })
    const v = V.fromDatum(d)
    assert.equal(toText(V.toDatum(v)), toText(d))
    const big = Datum.number(1.5, '1.50')
    assert.equal(V.toJsonText(V.fromDatum(big)), '1.50')
    assert.equal(V.toJsonText(v), '{"a":[1,"x",null,true],"b":{"c":2.5}}')
    // A keyword is its name, missing is null; a function has no data form.
    assert.equal(V.toJsonText(V.vectorVal([V.keyword('k'), V.missing()])), '["k",null]')
    assert.match(String(thrown(() => V.toJsonText(V.vectorVal([V.selectorVal(Selector.root())])))), /a selector has no data form/)
  })

  it('get-path walks records and vectors and answers missing', () => {
    const v = V.fromDatum(fromJSON({ a: [{ b: 1 }] }))
    same(V.getPath(v, ['a', 0, 'b']), n(1))
    assert.ok(V.isMissing(V.getPath(v, ['z'])))
    assert.ok(V.isMissing(V.getPath(v, [0])))
    same(V.getPath(v, []), v)
    assert.ok(V.isMissing(V.field(v, 'z') as Val))
    assert.equal(V.field(V.NULL, 'z'), undefined)
  })

  it('environments shadow innermost first', () => {
    const env = V.bind(null, 'x', n(1))
    const inner = V.bind(env, 'x', n(2))
    same(V.envGet(env, 'x') as Val, n(1))
    same(V.envGet(inner, 'x') as Val, n(2))
    assert.ok(!V.envHas(inner, 'y'))
    assert.deepStrictEqual(V.envNames(inner), ['x', 'x'])
  })

  it('liveness follows the input', () => {
    const at = span(sourceFile('t'), 0, 0)
    const live: value.Plan = { p: 'map', f: { fn: 'native', native: native('text')! }, source: { p: 'input' }, at }
    assert.ok(V.isLive(live))
    assert.equal(V.protocol(live), 'Items')
    const finite = V.concatPlan([V.str('a'), V.textVal({ p: 'lit', text: 'b' })])
    assert.ok(!V.isLive(finite))
    assert.ok(V.isText(finite))
    const mixed = V.concatPlan([V.str('a'), V.textVal(live)])
    assert.ok(V.isLive(mixed))
    assert.equal(V.protocol({ p: 'input' }), 'JsonEvents')
  })

  it('selector segments name one location only', () => {
    assert.deepStrictEqual(V.selectorSegments(Selector.root().property('a').index(2)), ['a', 2])
    assert.equal(V.selectorSegments(Selector.root().eachIndex()), undefined)
  })

  it('equality is structural for data and false for resources', () => {
    same(V.taggedVal('table-end'), V.taggedVal('table-end'))
    assert.ok(!V.valEquals(V.taggedVal('table-end'), V.taggedVal('no-schema')))
    same(V.num(1, '1.0'), n(1))
    assert.ok(!V.valEquals(V.str('a'), V.keyword('a')))
    const t = V.textVal({ p: 'lit', text: 'x' })
    assert.ok(!V.valEquals(t, t))
    assert.ok(!V.valEquals(n(NaN), n(NaN)))
    same(n(-0), n(0))
    // Records compare by their entries, in any order.
    same(
      V.record([
        ['a', n(1)],
        ['b', n(2)],
      ]),
      V.record([
        ['b', n(2)],
        ['a', n(1)],
      ]),
    )
    // A value nested far deeper than a call stack is compared all the same.
    let deep: Val = n(1)
    let other: Val = n(1)
    for (let i = 0; i < 100_000; i++) {
      deep = V.vectorVal([deep])
      other = V.vectorVal([other])
    }
    same(deep, other)
    assert.equal(V.toJsonText(deep).length, 200_001)
    assert.ok(V.debugText(deep, 10).startsWith('[[[[[[[[[[['))
  })
})

describe('evaluation', () => {
  it('values, records and lookups', () => {
    same(evalIn('def opts\n  record\n    entry :delimiter ","\n    entry :header true', 'get :delimiter opts'), V.str(','))
    assert.ok(V.isMissing(evalIn('', 'get :x (record)')))
    same(evalIn('', 'get-path (as-path ["a" 0]) (record (entry :a [7]))'), V.num(7, '7'))
    same(evalIn('', 'map (fn [x] (get :a x)) [(record (entry :a 1)) (record (entry :a 2))]'), V.vectorVal([n(1), n(2)]))
  })

  it('let, if and match choose', () => {
    same(evalIn('', 'let [x true] (if x "yes" "no")'), V.str('yes'))
    const src =
      'def tag [v]\n  match v\n    case (selected :columns raw) raw\n    case table-end "end"\n    case [a b] b\n    case 1 "one"\n    case _ "other"'
    same(evalIn(src, 'tag (selected :columns 5)'), n(5))
    same(evalIn(src, 'tag table-end'), V.str('end'))
    same(evalIn(src, 'tag [1 2]'), n(2))
    same(evalIn(src, 'tag 1'), V.str('one'))
    same(evalIn(src, 'tag no-schema'), V.str('other'))
    const no = evalFail('', 'match 3 (case 1 1)')
    assert.ok(no.message.startsWith('no_match: '), String(no))
    // A value no case takes is named by its kind and a short prefix, so a
    // failure over a large document does not carry the document.
    const big = `match [${'"abcdefgh" '.repeat(10_000)}] (case 1 1)`
    const f = evalFail('', big)
    assert.ok(f.message.startsWith('no_match: no case matches a vector ('), f.message)
    assert.ok(f.message.length < 200, String(f.message.length))
    // A short one is named whole.
    assert.equal(evalFail('', 'match [1 "a" :k] (case 1 1)').message, 'no_match: no case matches [1 "a" :k]')
  })

  // What a render needs beyond the stack: `length` and `compare` see a key
  // past a length, and `number-class` names a number's class.
  it('the render operators', () => {
    for (const [expr, message] of [
      ['length 1', 'length: the string must be a string'],
      ['compare 1 "2"', 'compare: the second must be a number, not a string'],
      ['number-class "1"', 'number-class: the number must be a number, not a string'],
    ]) {
      const f = evalFail('', expr)
      assert.equal(f.code, 'DSL_TYPE_ERROR', expr)
      assert.ok(f.message.startsWith(`type_mismatch: ${message}`), `${expr}: ${f}`)
    }
    // `length` counts characters, not bytes, nor UTF-16 units.
    same(evalIn('', 'length "héllo 日本"'), n(8))
    same(evalIn('', 'length "🚀a"'), n(2))
    same(evalIn('', 'length ""'), n(0))
    for (const [expr, want] of [
      ['compare 1 2', 'less'],
      ['compare 2 2', 'equal'],
      ['compare 3 2.5', 'greater'],
      ['compare (length "abc") 3', 'equal'],
      ['number-class 1e3', 'finite'],
      ['number-class 1e400', 'infinity'],
      ['number-class -1e400', 'negative-infinity'],
    ]) {
      same(evalIn('', expr), V.keyword(want), expr)
    }
  })

  // The stack operators: data last, a new vector each time, bounded by the
  // vector's length; an empty vector refuses `pop` and `top` with a type
  // error naming the operator.
  it('the stack operators and the string forms', () => {
    same(evalIn('', 'push :b [:a]'), V.vectorVal([V.keyword('a'), V.keyword('b')]))
    same(evalIn('', 'pop [1 2 3]'), V.vectorVal([n(1), n(2)]))
    same(evalIn('', 'top [1 2 3]'), n(3))
    same(evalIn('', 'count [1 2 3]'), n(3))
    same(evalIn('', 'count []'), n(0))
    same(evalIn('', 'top (push "x" (pop [1 2]))'), V.str('x'))
    same(evalIn('', 'pop [1]'), V.vectorVal([]))
    for (const [expr, message] of [
      ['pop []', 'pop: the vector is empty'],
      ['top []', 'top: the vector is empty'],
      ['count 1', 'count: the data must be a vector'],
      ['push 1 :k', 'push: the data must be a vector'],
      ['pop (record)', 'pop: the data must be a vector'],
      ['quoted 1', 'quoted: the string must be a string'],
      ['repeat 2.5 "a"', 'repeat: the count must be a whole number of at least 0'],
      ['repeat 2 1', 'repeat: the string must be a string'],
    ]) {
      const f = evalFail('', expr)
      assert.equal(f.code, 'DSL_TYPE_ERROR', expr)
      assert.ok(f.message.startsWith(`type_mismatch: ${message}`), `${expr}: ${f}`)
    }
    same(evalIn('', 'quoted "a\\"b\\\\c\\n"'), V.str('"a\\"b\\\\c\\n"'))
    same(evalIn('', 'repeat 3 "ab"'), V.str('ababab'))
    same(evalIn('', 'repeat 0 "ab"'), V.str(''))
    same(evalIn('', 'repeat 2 ""'), V.str(''))
    // The result is one scalar of the output: past max_scalar_bytes it is
    // refused before it is built, naming the limit.
    let f = evalFail('', 'repeat 1000000000 "abcdefghij"')
    assert.equal(f.code, 'RESOURCE_LIMIT_EXCEEDED', String(f))
    assert.equal(f.limit.name, 'max_scalar_bytes')
    // A count past what an index holds is the limit's refusal too, judged
    // before the count is narrowed; a fraction or a negative count is a
    // type error.
    f = evalFail('', 'repeat 100000000000000000000 "a"')
    assert.equal(f.code, 'RESOURCE_LIMIT_EXCEEDED', String(f))
    assert.equal(f.limit.name, 'max_scalar_bytes')
    same(evalIn('', 'repeat 100000000000000000000 ""'), V.str(''))
    f = evalFail('', 'repeat 1.5 "a"')
    assert.equal(f.code, 'DSL_TYPE_ERROR', String(f))
    assert.match(f.message, /not 1\.5/)
    // `kind` passed as a function meets what the checker refuses where it
    // is named: a finite text is refused as a live one is, and no `:text`
    // or `:stream` is ever answered.
    same(evalIn('', 'map kind [1 "s"]'), V.vectorVal([V.keyword('number'), V.keyword('string')]))
    // `keys` answers a record's keys in its order, as strings; a value that
    // is no record is a type error.
    same(evalIn('', 'keys (record (entry :b 1) (entry :a 2))'), V.vectorVal([V.str('b'), V.str('a')]))
    same(evalIn('', 'keys (record)'), V.vectorVal([]))
    f = evalFail('', 'keys [1]')
    assert.ok(f.message.startsWith('type_mismatch: keys: the record must be a record, not a vector'), String(f))
    f = evalFail('', 'map kind [(text "x")]')
    assert.equal(f.code, 'DSL_TYPE_ERROR', String(f))
    assert.ok(f.message.startsWith('type_mismatch: kind: a text cannot be asked'), String(f))
    // The quoted form of a string can be six times the string: it is held
    // to the same limit, refused before it is built.
    f = evalFail('', 'quoted (repeat 3000000 "\\u0001")')
    assert.equal(f.code, 'RESOURCE_LIMIT_EXCEEDED', String(f))
    assert.equal(f.limit.name, 'max_scalar_bytes')
    assert.ok(f.message.startsWith('quoted:'), String(f))
    // The event values a program builds are the ones `events` delivers.
    same(evalIn('', 'key "k"'), V.taggedVal('key', [V.str('k')]))
    same(evalIn('', 'scalar null'), V.taggedVal('scalar', [V.NULL]))
    same(evalIn('', 'object-start'), V.taggedVal('object-start'))
    assert.ok(evalFail('', 'key :k').message.startsWith('type_mismatch: key: '))
    assert.ok(evalFail('', 'scalar [1]').message.startsWith('type_mismatch: scalar: '))
    // Matched as the table events are: the constants compare, the
    // constructors bind their one field.
    const src =
      'def tag [e]\n  match e\n    case object-start "{"\n    case array-end "]"\n    case (key name) name\n    case (scalar v) v\n    case _ "?"'
    same(evalIn(src, 'tag object-start'), V.str('{'))
    same(evalIn(src, 'tag array-end'), V.str(']'))
    same(evalIn(src, 'tag object-end'), V.str('?'))
    same(evalIn(src, 'tag (key "k")'), V.str('k'))
    same(evalIn(src, 'tag (scalar 5)'), n(5))
  })

  it('closures, partials and arity', () => {
    same(evalIn('def add [a b] [a b]', '(partial add 1) 2'), V.vectorVal([n(1), n(2)]))
    let f = evalFail('def add [a b] [a b]', 'add 1')
    assert.equal(f.code, 'DSL_TYPE_ERROR')
    assert.ok(f.message.startsWith('arity: fn add takes 2'), String(f))
    f = evalFail('', 'get 1')
    assert.ok(f.message.startsWith('arity: get takes 2'), String(f))
    f = evalFail('', '1 2')
    assert.match(f.message, /not a function/)
    // A partial of a partial supplies its arguments in order.
    same(evalIn('def three [a b c] [a b c]', '(partial (partial three 1) 2) 3'), V.vectorVal([n(1), n(2), n(3)]))
  })

  it('a program def shadows the library and the library keeps its own', () => {
    // The program's `csv-row` is not what the library's `csv` calls.
    const rt = runtime('def csv-row [o c] 1\ndef export [input] (csv csv-options (table-from-json b input))\ndef b 1')
    const lib = run(rt.defValue('stdlib', 'csv-row')) as Val
    const mine = run(rt.defValue('program', 'csv-row')) as Val
    assert.ok('fn' === lib.v && 'closure' === lib.f.fn && 'stdlib' === lib.f.closure.scope)
    assert.ok('fn' === mine.v && 'closure' === mine.f.fn && 'program' === mine.f.closure.scope)
  })

  it('fail names the form position in the right file', () => {
    let f = evalFail('def boom [x]\n  fail "no"', 'boom 1')
    assert.equal(f.code, 'INPUT_INVALID')
    assert.equal(f.message, 'no')
    assert.deepStrictEqual([f.row, f.col], [2, 3])
    // A failure inside the library names the library's file and line in
    // its message, and no row of the program's.
    f = evalFail('', 'table-finish no-schema')
    assert.equal(f.code, 'INPUT_INVALID')
    const lib = source('stdlib/table.alc') as string
    const lines = lib.split('\n')
    const index = lines.findIndex((l) => l.includes('fail "Required metadata was not found"'))
    const [row, col] = [index + 1, lines[index].indexOf('fail') + 1]
    assert.equal(f.message, `Required metadata was not found (at stdlib/table.alc:${row}:${col})`)
    assert.deepStrictEqual([f.row, f.col], [undefined, undefined])
    // A program named like a library file is still its own text.
    const src = 'def boom [x]\n  fail "no"'
    f = thrown(() => evalIn(src, 'boom 1', runtime(src, 'stdlib/table.alc')))
    assert.deepStrictEqual([f.message, f.row, f.col], ['no', 2, 3])
    // A library failure under a native the program called takes the
    // program's position too.
    f = evalFail('', 'map (fn [s] (table-finish s)) [no-schema]')
    assert.ok(f.message.endsWith(`(at stdlib/table.alc:${row}:${col})`), String(f))
    assert.deepStrictEqual([f.row, f.col], [1, 1])
    // A span is the library's by identity, not by name.
    assert.equal(fileOf(span(sourceFile('stdlib/table.alc'), 0, 0)), undefined)
  })

  it('streams are plans and export applies to the input', () => {
    const out = run(runtime('def export [input] (json input)').export())
    assert.ok('text' === out.v && 'json' === out.plan.p)
    let f = thrown(() => run(runtime('def x 1').export()))
    assert.ok(f.message.startsWith('no_export: '), String(f))
    f = thrown(() => run(runtime('def export 1').export()))
    assert.ok(f.message.startsWith('type_mismatch: '), String(f))
  })

  it('the fast paths switch', () => {
    const src =
      'def b\n  record\n    entry :columns (path "m")\n    entry :rows (path "r" each-index)\n    entry :column (fn [d] d)\ndef export [input] (csv csv-options (table-from-json b input))'
    const fast = run(runtime(src).export())
    assert.ok('text' === fast.v && 'csv' === fast.plan.p)
    const slow = run(runtime(src).withNative(false).export())
    assert.ok('text' === slow.v && 'concat-map' === slow.plan.p)
  })
})

describe('measure', () => {
  // A partial whose function is itself a partial holds the one before it,
  // so a chain of them (a scan-emit state that wraps itself once per item)
  // is measured link by link: each link one node and one level, and every
  // link's arguments counted.
  it('a chain of partials is measured', () => {
    const rt = runtime('')
    const arg = V.str('a'.repeat(100))
    const chain = (links: number): Val => {
      let f: value.Func = { fn: 'native', native: native('vector')! }
      for (let i = 0; i < links; i++) {
        const next = partial(f, [arg])
        if ('fn' !== next.v) throw new Error('a partial is a function')
        f = next.f
      }
      return V.fnVal(f)
    }
    const bounds = (maxBytes: number, maxDepth: number) => ({
      nodeBytes: 16,
      maxBytes,
      bytesLimit: 'max_metadata_bytes',
      maxDepth,
      what: 'the state',
    })
    assert.deepStrictEqual(rt.measure(chain(3), bounds(Infinity, Infinity)), { bytes: 6 * 16 + 300, depth: 4 })
    const long = chain(1000)
    assert.equal((thrown(() => rt.measure(long, bounds(1 << 30, 256))) as any).limit.name, 'max_depth')
    assert.equal((thrown(() => rt.measure(long, bounds(4096, Infinity))) as any).limit.name, 'max_metadata_bytes')
  })

  // A finite text keeps the function its concat-map applies, so a state
  // that hides the one before it in that function's partial is walked
  // through it: vector, text, plan, function, then the state before, each
  // a level.
  it('the function a finite text applies is measured', () => {
    const rt = runtime('')
    const at = span(sourceFile('t.alc'), 0, 0)
    const g = (): value.Func => ({ fn: 'native', native: native('vector')! })
    const link = (prev: Val): Val => {
      const p = partial(g(), [prev])
      if ('fn' !== p.v) throw new Error('a partial is a function')
      const text = V.textVal({ p: 'concat-map', f: p.f, items: { seq: 'vector', items: [V.str('x')] }, at })
      return V.vectorVal([text])
    }
    const chain = (links: number): Val => {
      let v = V.vectorVal([])
      for (let i = 0; i < links; i++) v = link(v)
      return v
    }
    const bounds = (maxBytes: number, maxDepth: number) => ({
      nodeBytes: 16,
      maxBytes,
      bytesLimit: 'max_metadata_bytes',
      maxDepth,
      what: 'the state',
    })
    // One link: the vector (1), its text (2), the plan (3), the function
    // and the item "x" (4), the state before (5).
    assert.deepStrictEqual(rt.measure(chain(1), bounds(Infinity, Infinity)), { bytes: 6 * 16 + 1, depth: 5 })
    // Each further link is four more levels and five more nodes.
    assert.deepStrictEqual(rt.measure(chain(3), bounds(Infinity, Infinity)), { bytes: 16 * 16 + 3, depth: 13 })
    const long = chain(1000)
    assert.equal((thrown(() => rt.measure(long, bounds(1 << 30, 256))) as any).limit.name, 'max_depth')
    assert.equal((thrown(() => rt.measure(long, bounds(4096, Infinity))) as any).limit.name, 'max_metadata_bytes')
  })

  it('the measure counts UTF-8 bytes', () => {
    const rt = runtime('')
    const bounds = { nodeBytes: 16, maxBytes: Infinity, bytesLimit: 'max_metadata_bytes', maxDepth: Infinity, what: 'x' }
    assert.deepStrictEqual(rt.measure(V.str('é日😀'), bounds), { bytes: 16 + 2 + 3 + 4, depth: 1 })
  })
})

describe('natives', () => {
  // The JSON string form, with the C1 controls escaped as well, in the
  // render package's lowercase hex; everything else as itself.
  it('quoted is the JSON string form with the C1 controls escaped', () => {
    assert.equal(quote(''), '""')
    assert.equal(quote('plain'), '"plain"')
    assert.equal(
      quote('q" b\\ n\n r\r t\t bs\u0008 ff\u000c nul\u0000 c1\u0001 us\u001f'),
      '"q\\" b\\\\ n\\n r\\r t\\t bs\\b ff\\f nul\\u0000 c1\\u0001 us\\u001f"',
    )
    assert.equal(
      quote('del\u007f pad\u0080 apc\u009f nbsp  é 日本 🚀 /'),
      '"del\\u007f pad\\u0080 apc\\u009f nbsp  é 日本 🚀 /"',
    )
    // What transduce writes for the same string, where both escape: the
    // two agree on JSON's own escapes.
    assert.equal(quote('q" \\ \n \u001f é'), toText(Datum.string('q" \\ \n \u001f é')))
    // The length counted before building is the length built, in UTF-8
    // bytes.
    for (const s of ['', 'plain', 'q" \\ \n \u001f é', 'del\u007f pad\u0080 日本 🚀']) {
      assert.equal(quotedLen(s), Buffer.byteLength(quote(s), 'utf8'), JSON.stringify(s))
    }
  })

  // `kind` names every retained value's kind by one keyword, the word the
  // messages use.
  it('kind names a value by one keyword', () => {
    assert.equal(kindWord(V.NULL), 'null')
    assert.equal(kindWord(V.bool(true)), 'boolean')
    assert.equal(kindWord(n(1.5)), 'number')
    assert.equal(kindWord(V.str('s')), 'string')
    assert.equal(kindWord(V.keyword('k')), 'keyword')
    assert.equal(kindWord(V.vectorVal([])), 'vector')
    assert.equal(kindWord(V.missing()), 'missing')
    assert.equal(kindWord(V.taggedVal('key', [V.str('a')])), 'tagged')
    assert.equal(kindWord(V.textVal({ p: 'lit', text: 'x' })), undefined)
  })

  // NaN and the infinities, which no literal spells: `number-class` names
  // each, and `compare` orders the infinities and leaves NaN unordered.
  it('number-class and compare see the non-finite numbers', () => {
    const rt = runtime('')
    const at = span(sourceFile('t'), 0, 0)
    const call = (name: string, args: Val[]) => {
      const f = V.fnVal({ fn: 'native', native: native(name)! })
      if ('fn' !== f.v) throw new Error('a native is a function')
      const out = rt.applyNow(f.f, args, at)
      return 'keyword' === out.v ? out.name : V.debugText(out)
    }
    assert.equal(call('number-class', [n(1.5)]), 'finite')
    assert.equal(call('number-class', [n(-0)]), 'finite')
    assert.equal(call('number-class', [n(Infinity)]), 'infinity')
    assert.equal(call('number-class', [n(-Infinity)]), 'negative-infinity')
    assert.equal(call('number-class', [n(NaN)]), 'nan')
    assert.equal(call('compare', [n(NaN), n(1)]), 'unordered')
    assert.equal(call('compare', [n(1), n(NaN)]), 'unordered')
    assert.equal(call('compare', [n(-0), n(0)]), 'equal')
    assert.equal(call('compare', [n(-Infinity), n(-1e308)]), 'less')
    assert.equal(call('compare', [n(Infinity), n(Infinity)]), 'equal')
  })

  // `length` counts characters a chunk at a time, and takes an evaluation
  // step per chunk, so the host's abort flag stops the count of a long
  // string.
  it('length counts characters in steps the abort flag reads', () => {
    const at = span(sourceFile('t'), 0, 0)
    const length: value.Func = { fn: 'native', native: native('length')! }
    const long = V.str('héllo 日本'.repeat(Math.floor(LENGTH_CHUNK / 3)))
    same(runtime('').applyNow(length, [long], at), n(8 * Math.floor(LENGTH_CHUNK / 3)))
    const flag = new AbortFlag()
    const rt = runtime('').withAbort(flag)
    flag.abort()
    const huge = V.str('k'.repeat(LENGTH_CHUNK * 64))
    assert.equal((thrown(() => rt.applyNow(length, [huge], at)) as any).code, 'ABORTED')
  })

  it('numbers print as the renderers print them', () => {
    assert.equal(shortestNumber(1), '1')
    assert.equal(shortestNumber(50.25), '50.25')
    assert.equal(shortestNumber(1e20), '100000000000000000000')
    assert.equal(shortestNumber(1e21), '1e21')
    assert.equal(shortestNumber(1e-7), '1e-7')
    assert.equal(shortestNumber(-0), '-0')
    assert.equal(numberText(1.5, '1.50'), '1.50')
    assert.equal((thrown(() => numberText(1, '01')) as any).code, 'INVALID_NUMBER')
    assert.equal((thrown(() => numberText(Infinity, '1e999')) as any).code, 'TARGET_VALUE_UNREPRESENTABLE')
    assert.equal((thrown(() => numberText(NaN)) as any).code, 'TARGET_VALUE_UNREPRESENTABLE')
  })

  it('a datum value keeps its member order', () => {
    // A member named like an index stays where the document put it.
    const v = V.fromDatum(Datum.object([['b', Datum.null], ['1', Datum.null], ['a', Datum.null]]))
    assert.equal(V.toJsonText(v), '{"b":null,"1":null,"a":null}')
  })
})
