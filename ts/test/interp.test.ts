/* Copyright (c) 2026 tabnas, MIT License */

// The evaluator and the runtime values: rs/src/interp.rs's,
// rs/src/value.rs's and rs/src/stdlib/registry.rs's tests.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import {
  MESSAGES,
  Runtime,
  Sources,
  Val,
  desugarProgram,
  fileOf,
  kindWord,
  CHARS_WITHIN_STEP,
  LENGTH_CHUNK,
  NUMBER_STEP,
  UNQUOTED_STEP,
  native,
  numberText,
  outer,
  parseFile,
  partial,
  noColumnsEmpty,
  nonFinite,
  quote,
  quotedLen,
  resolve,
  run,
  shortestNumber,
  source,
  sourceFile,
  span,
  unquote,
  value,
} from '../dist/alchemy'
import { AbortFlag, Datum, Selector, fromJSON, toText } from '../dist/shared'

import { OPTIONS, compile, thrown } from './common'

const V = value

// A runtime over the program `src`, read as `t.alc`.
function runtime(src: string, file = 't.alc'): Runtime {
  const forms = desugarProgram(parseFile(src, file), src)
  const sources = Sources.one(file, src)
  return new Runtime(resolve(forms, sources, outer), sources, OPTIONS)
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

  // The evaluator's own checks of the `let`, `if` and `match` shapes, which
  // the desugarer and the resolver always meet first, fail as theirs do:
  // the desugarer's finer code and text, DSL_PARSE_ERROR. And a value
  // called that is not a function is named as the checker names it: the
  // callee as written, and its kind.
  it('the evaluator fails as the stages before it', () => {
    const rt = runtime('')
    for (const [expr, code] of [
      ['(let [x 1])', 'bad_let'],
      ['(let [x 1 2] x)', 'bad_let'],
      ['(if true 1)', 'bad_if'],
      ['(match)', 'bad_match'],
    ]) {
      // Read and not desugared, so the desugarer does not meet it.
      const f = thrown(() => rt.evalProgramExpr(parseFile(expr, 'e.alc')[0]))
      assert.equal(f.code, 'DSL_PARSE_ERROR', `${expr}: ${f}`)
      assert.equal(f.message, `${code}: ${MESSAGES.find(([known]) => known === code)?.[1]}`, `${expr}: ${f}`)
    }
    const f = evalFail('', '(get :f (record (entry :f 1))) 2')
    assert.equal(f.code, 'DSL_TYPE_ERROR', String(f))
    assert.equal(f.message, 'type_mismatch: (get :f (record (entry :f 1))) is a number and cannot be called')
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
    assert.equal(f.message, 'type_mismatch: 1 is a number and cannot be called')
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

  // `fail` takes a code before the message: one of three keywords, each
  // its code; any other keyword, or a code that is not one, is a type
  // error, as the checker's is for a literal keyword.
  it('fail names its code', () => {
    const src = 'def boom [k] (fail k "no")'
    for (const [expr, code] of [
      ['boom :unrepresentable', 'TARGET_VALUE_UNREPRESENTABLE'],
      ['boom :protocol-order', 'PROTOCOL_ORDER_ERROR'],
      ['boom :invalid', 'INPUT_INVALID'],
    ]) {
      const f = evalFail(src, expr)
      assert.deepStrictEqual([f.code, f.message, f.row, f.col], [code, 'no', 1, 14], expr)
    }
    for (const [program, expr, message] of [
      [src, 'boom :nope', 'the code of fail must be :invalid, :unrepresentable or :protocol-order, not :nope'],
      [src, 'boom 1', 'fail: the code must be a keyword, not a number'],
      ['def boom [k] (fail k)', 'boom 1', 'fail: the message must be a string, not a number'],
    ]) {
      const f = evalFail(program, expr)
      assert.deepStrictEqual([f.code, f.message], ['DSL_TYPE_ERROR', `type_mismatch: ${message}`], expr)
    }
  })

  // The inferred table's columns come from the first row of any kind
  // (stdlib/table.alc): an object's members by name, an array's cells by
  // position (`indices`), a scalar as the one column `value`; a later row
  // of another kind projects through those sources, missing where a path
  // does not apply to it.
  it('the inferred columns take every row kind', () => {
    const labels = (row: string) => evalIn('', `map (fn [c] (get :label c)) (table-inferred-columns ${row})`)
    same(labels('(record (entry :a 1) (entry :b 2))'), V.vectorVal([V.str('a'), V.str('b')]))
    same(labels('[10 20 30]'), V.vectorVal([V.str('0'), V.str('1'), V.str('2')]))
    same(labels('"x"'), V.vectorVal([V.str('value')]))
    same(labels('[]'), V.vectorVal([]))
    const row = (first: string, later: string) => evalIn('', `table-row (table-inferred-columns ${first}) ${later}`)
    same(row('[10 20]', '[30 40]'), V.taggedVal('row', [V.vectorVal([n(30), n(40)])]))
    same(row('5', '"six"'), V.taggedVal('row', [V.vectorVal([V.str('six')])]))
    same(row('(record (entry :a 1))', '(record (entry :a 2) (entry :b 3))'), V.taggedVal('row', [V.vectorVal([n(2)])]))
    same(row('[10 20]', '(record (entry :a 1))'), V.taggedVal('row', [V.vectorVal([V.missing(), V.missing()])]))
    // The first row binds the columns and writes the schema before itself.
    same(
      evalIn('', 'match (table-first-row (record (entry :columns :infer)) no-schema [10 20]) (case (transition s out) out)'),
      evalIn('', '[(schema [(record (entry :label "0")) (record (entry :label "1"))]) (row [10 20])]'),
    )
  })

  // The root adapters (stdlib/root.alc), driven event by event as
  // scan-emit drives them: each decides at the first event, passes a root
  // of the kind its target needs through, wraps any other in one, and
  // refuses events that are no tree's (PROTOCOL_ORDER_ERROR).
  it('the root adapters wrap the root a target needs', () => {
    const rt = runtime('')
    const at = span(sourceFile('t'), 0, 0)
    const fn = (expr: string): value.Func => {
      const f = evalIn('', expr, rt)
      if ('fn' !== f.v) throw new Error(`${expr} is not a function`)
      return f.f
    }
    const items = (v: Val): ReadonlyArray<Val> => {
      if ('vector' !== v.v) throw new Error(`${V.debugText(v)} is not a vector`)
      return v.items
    }
    const drive = (step: string, events: string): Val => {
      const s = fn(step)
      let state = evalIn('', '[:start]', rt)
      const out: Val[] = []
      for (const event of items(evalIn('', events, rt))) {
        const t = rt.applyNow(s, [state, event], at)
        if ('tagged' !== t.v || 'transition' !== t.tag) throw new Error(`${V.debugText(t)} is not a transition`)
        state = t.fields[0]
        out.push(...items(t.fields[1]))
      }
      out.push(...items(rt.applyNow(fn('wrap-finish'), [state], at)))
      return V.vectorVal(out)
    }
    const object = 'partial wrap-object-step "doc"'
    const array = 'wrap-array-step'
    for (const [step, events, want] of [
      // An object root passes through; an array or a scalar is the member
      // `doc` of an object, closed when the wrapped root closes.
      [object, '[object-start (key "a") (scalar 1) object-end]', '[object-start (key "a") (scalar 1) object-end]'],
      [
        object,
        '[array-start array-start array-end (scalar 1) array-end]',
        '[object-start (key "doc") array-start array-start array-end (scalar 1) array-end object-end]',
      ],
      [object, '[(scalar "x")]', '[object-start (key "doc") (scalar "x") object-end]'],
      // An array root passes through; an object or a scalar is the one
      // element of an array.
      [array, '[array-start (scalar 1) array-end]', '[array-start (scalar 1) array-end]'],
      [
        array,
        '[object-start (key "a") object-start object-end object-end]',
        '[array-start object-start (key "a") object-start object-end object-end array-end]',
      ],
      [array, '[(scalar null)]', '[array-start (scalar null) array-end]'],
    ]) {
      same(drive(step, events), evalIn('', want), `${step} ${events}`)
    }
    // A failure in the library names its form there.
    for (const [step, events, message] of [
      [object, '[object-end]', "the events begin with an end or a key, which a tree's never do (at stdlib/root.alc:36:16)"],
      [array, '[(key "a")]', "the events begin with an end or a key, which a tree's never do (at stdlib/root.alc:72:16)"],
      [object, '[]', "the events hold no value, where a tree's hold one (at stdlib/root.alc:49:19)"],
      [array, '[object-start (key "a")]', "the events ended inside a container, which a tree's never do (at stdlib/root.alc:52:12)"],
      [object, '[(scalar 1) (scalar 2)]', "the events hold more after the root value, which a tree's never do (at stdlib/root.alc:45:12)"],
      [
        array,
        '[object-start object-end array-start]',
        "the events hold more after the root value, which a tree's never do (at stdlib/root.alc:81:12)",
      ],
    ]) {
      const f: any = thrown(() => drive(step, events))
      assert.deepStrictEqual([f.code, f.message], ['PROTOCOL_ORDER_ERROR', message], `${step} ${events}`)
    }
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
    for (const s of ['', 'plain', 'q" \\ \n \u001f é', 'del\u007f pad\u0080 日本 🚀', 'nonchar\ufffe\uffff\ufffd']) {
      assert.equal(quotedLen(s), Buffer.byteLength(quote(s), 'utf8'), JSON.stringify(s))
    }
    // The two noncharacters YAML's printable set and XML's characters
    // exclude are escaped; U+FFFD, a character, is not.
    assert.equal(quote('\ufffe\uffff\ufffd'), '"\\ufffe\\uffff\ufffd"')
  })

  it('unquote reads back what quote writes', () => {
    for (const s of [
      '',
      'plain',
      'q" b\\ n\n r\r t\t bs\u0008 ff\u000c nul\u0000 us\u001f',
      'del\u007f pad\u0080 apc\u009f nbsp  é 日本 🚀 /',
      '\ufffe\uffff',
    ]) {
      assert.equal(unquote(quote(s)), s, JSON.stringify(s))
    }
    // JSON's other spellings: the solidus, uppercase hex, a surrogate pair.
    assert.equal(unquote('"\\/\\u00E9\\ud83d\\ude80"'), '/é🚀')
    // Not a double-quoted form; a surrogate on its own is refused although
    // a JavaScript string could hold one.
    for (const text of [
      '',
      '"',
      'plain',
      '"open',
      '"in"side"',
      '"raw\nline"',
      '"bad \\x escape"',
      '"short \\u12"',
      '"lone \\ud800"',
      '"low \\udc00 first"',
      '"pair \\ud800\\u0041"',
    ]) {
      assert.equal(unquote(text), undefined, JSON.stringify(text))
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

  // `indices` gives the positions of a vector's items, the labels the
  // inferred table gives an array row's cells, and takes an evaluation
  // step per item, so the host's abort flag stops a long vector's count.
  it('indices gives the positions of a vector\'s items in steps the abort flag reads', () => {
    same(evalIn('', 'indices [:a :b :c]'), V.vectorVal([n(0), n(1), n(2)]))
    same(evalIn('', 'indices []'), V.vectorVal([]))
    const f = evalFail('', 'indices (record)')
    assert.deepStrictEqual(
      [f.code, f.message],
      ['DSL_TYPE_ERROR', 'type_mismatch: indices: a vector was expected, not a record'],
    )
    const at = span(sourceFile('t'), 0, 0)
    const indices: value.Func = { fn: 'native', native: native('indices')! }
    const long = V.vectorVal(Array.from({ length: 1000 }, () => V.NULL))
    const counted = runtime('').applyNow(indices, [long], at)
    assert.ok('vector' === counted.v && 1000 === counted.items.length)
    const flag = new AbortFlag()
    const rt = runtime('').withAbort(flag)
    flag.abort()
    assert.equal((thrown(() => rt.applyNow(indices, [long], at)) as any).code, 'ABORTED')
  })

  // `unquoted` reads a double-quoted form back, and refuses any other text
  // as data the program was given, naming it.
  it('unquoted reads a double-quoted form or refuses the text', () => {
    const at = span(sourceFile('t'), 0, 0)
    const unquoted: value.Func = { fn: 'native', native: native('unquoted')! }
    same(runtime('').applyNow(unquoted, [V.str('"a\\nb \\ud83d\\ude00"')], at), V.str('a\nb 😀'))
    let f = thrown(() => runtime('').applyNow(unquoted, [V.str('x')], at)) as any
    assert.deepStrictEqual([f.code, f.message], ['INPUT_INVALID', 'unquoted: "x" is not a double-quoted string'])
    f = thrown(() => runtime('').applyNow(unquoted, [V.str('"\\ud800"')], at)) as any
    assert.equal(f.code, 'INPUT_INVALID')
    f = evalFail('', 'unquoted 1')
    assert.equal(f.message, 'type_mismatch: unquoted: the string must be a string, not a number')
  })

  // `chars-within` tests every character, a code point at a time, against
  // ranges it validates first, and takes an evaluation step every 4096
  // characters, so the host's abort flag stops a long string's test.
  it('chars-within tests every character in steps the abort flag reads', () => {
    const xml = '[[9 10] [13 13] [32 55295] [57344 65533] [65536 1114111]]'
    same(evalIn('', `chars-within ${xml} "tab\\there 😀"`), V.bool(true))
    same(evalIn('', `chars-within ${xml} "nul\\u0000"`), V.bool(false))
    same(evalIn('', `chars-within ${xml} "\\uffff"`), V.bool(false))
    same(evalIn('', `chars-within ${xml} ""`), V.bool(true))
    same(evalIn('', 'chars-within [] ""'), V.bool(true))
    same(evalIn('', 'chars-within [] "a"'), V.bool(false))
    // A character above U+FFFF is one code point, not two halves.
    same(evalIn('', 'chars-within [[128512 128512]] "😀"'), V.bool(true))
    same(evalIn('', 'chars-within [[0 65535]] "😀"'), V.bool(false))
    for (const [expr, message] of [
      ['chars-within [[5 3]] "a"', 'a range must be a vector [low high] with low at most high, not [5 3]'],
      ['chars-within [[0 1.5]] "a"', "a range's bound must be a code point from 0 to 1114111, not 1.5"],
      ['chars-within [[-1 2]] "a"', "a range's bound must be a code point from 0 to 1114111, not -1"],
      ['chars-within [[0 1114112]] "a"', "a range's bound must be a code point from 0 to 1114111, not 1114112"],
      ['chars-within [[0 (number "Infinity")]] "a"', "a range's bound must be a code point from 0 to 1114111, not inf"],
      ['chars-within [["a" 1]] "a"', "a range's bound must be a number, not a string"],
      ['chars-within [[0]] "a"', 'a range must be a vector [low high], not a vector of 1 items'],
      ['chars-within [1] "a"', 'a range must be a vector [low high], not a number'],
      ['chars-within 1 "a"', 'the data must be a vector, not a number'],
      ['chars-within [[5 3]] 1', 'a range must be a vector [low high] with low at most high, not [5 3]'],
      ['chars-within [[0 3]] 1', 'the string must be a string, not a number'],
    ]) {
      const f = evalFail('', expr)
      assert.deepStrictEqual([f.code, f.message], ['DSL_TYPE_ERROR', `type_mismatch: chars-within: ${message}`], expr)
    }
    const at = span(sourceFile('t'), 0, 0)
    const charsWithin: value.Func = { fn: 'native', native: native('chars-within')! }
    const all = V.vectorVal([V.vectorVal([n(0), n(1114111)])])
    const ascii = V.vectorVal([V.vectorVal([n(0), n(127)])])
    const long = 'k'.repeat(4096 * 64)
    const huge = V.str(long)
    same(runtime('').applyNow(charsWithin, [all, huge], at), V.bool(true))
    const flag = new AbortFlag()
    const rt = runtime('').withAbort(flag)
    flag.abort()
    assert.equal((thrown(() => rt.applyNow(charsWithin, [all, huge], at)) as any).code, 'ABORTED')
    // The first character outside the ranges answers at once.
    same(rt.applyNow(charsWithin, [ascii, V.str('é' + long)], at), V.bool(false))
  })

  // `chars-within` counts its work in comparisons, so ranges from data as
  // many as they are take evaluation steps, and a bound on steps (the fuel,
  // the abort flag) stops a test that has not yet read 4,096 characters.
  it('chars-within takes a step per comparisons', () => {
    const rt = runtime('def export [input] input').withFuel(10)
    const range = V.vectorVal([n(5), n(5)])
    const ranges = V.vectorVal(Array.from({ length: 100_000 }, () => range))
    const at = span(sourceFile('t.alc'), 0, 0)
    const charsWithin: value.Func = { fn: 'native', native: native('chars-within')! }
    const f = thrown(() => rt.applyNow(charsWithin, [ranges, V.str('a')], at)) as any
    assert.equal(f.limit?.name, 'max_plan_steps', String(f))
    // A long string against a few ranges is counted the same way.
    const few = V.vectorVal([V.vectorVal([n(0), n(1)]), V.vectorVal([n(97), n(97)])])
    const g = thrown(() =>
      runtime('').withFuel(10).applyNow(charsWithin, [few, V.str('a'.repeat(CHARS_WITHIN_STEP * 6))], at),
    ) as any
    assert.equal(g.limit?.name, 'max_plan_steps', String(g))
  })

  // `unquoted` reads its input an evaluation step per `UNQUOTED_STEP`
  // characters as it goes, as `length` counts a long string in steps: ten
  // steps of fuel read ten times that many characters and no more, whether
  // or not the string turns out to be a quoted form, and a string refused
  // at its start costs no more than its first step.
  it('unquoted takes a step per characters read', () => {
    const at = span(sourceFile('t.alc'), 0, 0)
    const unquoted: value.Func = { fn: 'native', native: native('unquoted')! }
    const run = (text: string) => runtime('').withFuel(10).applyNow(unquoted, [V.str(text)], at)
    // A quoted form `n` characters long, its quotes included.
    const quoted = (n: number, c = 'a') => `"${c.repeat(n - 2)}"`
    same(run(quoted(10 * UNQUOTED_STEP)), V.str('a'.repeat(10 * UNQUOTED_STEP - 2)))
    // A character is a code point: a surrogate pair is one.
    same(run(quoted(10 * UNQUOTED_STEP, '\u{1f680}')), V.str('\u{1f680}'.repeat(10 * UNQUOTED_STEP - 2)))
    // One character more takes an eleventh step.
    let f = thrown(() => run(quoted(10 * UNQUOTED_STEP + 1))) as any
    assert.equal(f.limit?.name, 'max_plan_steps', String(f))
    // A string with no closing quote is read as far before it is refused.
    f = thrown(() => run(`"${'a'.repeat(10 * UNQUOTED_STEP)}`)) as any
    assert.equal(f.limit?.name, 'max_plan_steps', String(f))
    f = thrown(() => run(`x${'a'.repeat(100 * UNQUOTED_STEP)}`)) as any
    assert.equal(f.code, 'INPUT_INVALID', String(f))
  })

  // `is-number` answers whether `number` reads a string, and `number` fails
  // exactly where it answers false; a string that is not one is the same
  // type error to both.
  it('is-number answers whether number reads the string', () => {
    const at = span(sourceFile('t'), 0, 0)
    const isNumber: value.Func = { fn: 'native', native: native('is-number')! }
    const number: value.Func = { fn: 'native', native: native('number')! }
    const numbers = ['0', '-0', '12', '1.5', '-1.5e10', '2E-3', '1e+5', '0e0', '1e5', '1e999']
    numbers.push('12345678901234567890', 'Infinity', '-Infinity', 'NaN')
    const others = ['', '-', '01', '-01', '1.', '.5', '+1', '1e', '1e+', '1-2', ' 1', '1 ', '0x10']
    others.push('1_000', '1.5e', '-NaN', 'nan', 'infinity', 'Inf', 'Infinityx', 'NaNN', '١')
    for (const [texts, reads] of [
      [numbers, true],
      [others, false],
    ] as const) {
      for (const text of texts) {
        same(runtime('').applyNow(isNumber, [V.str(text)], at), V.bool(reads), JSON.stringify(text))
        if (reads) runtime('').applyNow(number, [V.str(text)], at)
        else assert.equal(thrown(() => runtime('').applyNow(number, [V.str(text)], at)).code, 'INPUT_INVALID', text)
      }
    }
    // A finite number keeps its text as the lexeme; a non-finite one by
    // name has none.
    assert.deepStrictEqual(runtime('').applyNow(number, [V.str('1.50')], at), V.num(1.5, '1.50'))
    assert.deepStrictEqual(runtime('').applyNow(number, [V.str('-Infinity')], at), V.num(-Infinity))
    for (const name of ['is-number', 'number']) {
      const f = evalFail('', `${name} 1`)
      assert.deepStrictEqual(
        [f.code, f.message],
        ['DSL_TYPE_ERROR', `type_mismatch: ${name}: the string must be a string, not a number`],
      )
    }
  })

  // `number` and `is-number` read their input an evaluation step per
  // `NUMBER_STEP` characters as they go, as `unquoted` reads its own: ten
  // steps of fuel read ten times that many characters and no more, whether
  // or not the string turns out to spell a number, and a string refused at
  // its start costs no more than its first step.
  it('number and is-number take a step per characters read', () => {
    const at = span(sourceFile('t.alc'), 0, 0)
    const isNumber: value.Func = { fn: 'native', native: native('is-number')! }
    const number: value.Func = { fn: 'native', native: native('number')! }
    const is = (text: string) => runtime('').withFuel(10).applyNow(isNumber, [V.str(text)], at)
    const read = (text: string) => runtime('').withFuel(10).applyNow(number, [V.str(text)], at)
    // A number `n` characters long.
    const long = (n: number) => `0.${'5'.repeat(n - 2)}`
    same(is(long(10 * NUMBER_STEP)), V.bool(true))
    assert.deepStrictEqual(read(long(10 * NUMBER_STEP)), V.num(Number(long(10 * NUMBER_STEP)), long(10 * NUMBER_STEP)))
    // One character more takes an eleventh step, and so does a character
    // after as many that ends the spelling: the string is read as far
    // before it is refused.
    for (const text of [long(10 * NUMBER_STEP + 1), `${long(10 * NUMBER_STEP)}x`]) {
      for (const f of [thrown(() => is(text)), thrown(() => read(text))]) {
        assert.equal(f.limit?.name, 'max_plan_steps', String(f))
      }
    }
    const early = `x${long(100 * NUMBER_STEP)}`
    same(is(early), V.bool(false))
    assert.equal(thrown(() => read(early)).code, 'INPUT_INVALID')
  })

  // A CSV options record's `:no-columns`: `:refuse`, the default, or
  // `:empty`; any other value is a type error where it is read.
  it('no-columns names a policy for a table of no columns', () => {
    assert.equal(noColumnsEmpty(V.NULL), false)
    assert.equal(noColumnsEmpty(evalIn('', '(record)')), false)
    assert.equal(noColumnsEmpty(evalIn('', '(record (entry :no-columns :refuse))')), false)
    assert.equal(noColumnsEmpty(evalIn('', '(record (entry :no-columns :empty))')), true)
    let f = thrown(() => noColumnsEmpty(evalIn('', '(record (entry :no-columns :nope))'))) as any
    assert.deepStrictEqual(
      [f.code, f.message],
      ['DSL_TYPE_ERROR', 'type_mismatch: :no-columns must be :refuse or :empty, not :nope'],
    )
    f = thrown(() => noColumnsEmpty(evalIn('', '(record (entry :no-columns "empty"))'))) as any
    assert.equal(f.message, 'type_mismatch: :no-columns must be :refuse or :empty, not a string')
  })

  // A number that is not finite is refused by `scalar-text`, as the
  // renderers refuse it, unless the options' `:non-finite` names what to
  // write instead; a policy that is none is a type error where a number
  // needs it, and only there.
  it('scalar-text writes a number that is not finite by the options policy', () => {
    const options = (policy: string) =>
      `(record (entry :null-text "-") (entry :missing :error)${'' === policy ? '' : ` (entry :non-finite ${policy})`})`
    const text = (policy: string, x: string) => evalIn('', `scalar-text ${options(policy)} (number "${x}")`)
    same(text(':null', 'NaN'), V.str('-'))
    same(text(':literal', 'NaN'), V.str('NaN'))
    same(text(':literal', 'Infinity'), V.str('Infinity'))
    same(text(':literal', '-Infinity'), V.str('-Infinity'))
    same(text(':reject', '2.50'), V.str('2.50'))
    same(text(':nope', '2.50'), V.str('2.50'))
    for (const policy of ['', ':reject']) {
      const f = evalFail('', `scalar-text ${options(policy)} (number "Infinity")`)
      assert.equal(f.code, 'TARGET_VALUE_UNREPRESENTABLE', policy)
    }
    let f = evalFail('', `scalar-text ${options(':nope')} (number "NaN")`)
    assert.deepStrictEqual(
      [f.code, f.message],
      ['DSL_TYPE_ERROR', 'type_mismatch: :non-finite must be :reject, :null or :literal, not :nope'],
    )
    f = evalFail('', `scalar-text ${options('1')} (number "NaN")`)
    assert.equal(f.message, 'type_mismatch: :non-finite must be :reject, :null or :literal, not a number')
    // The policy of a record, and of what is no record: none.
    assert.equal(nonFinite(V.NULL), 'reject')
    assert.equal(nonFinite(evalIn('', '(record)')), 'reject')
    assert.equal(nonFinite(evalIn('', '(record (entry :non-finite missing))')), 'reject')
    assert.equal(nonFinite(evalIn('', '(record (entry :non-finite :literal))')), 'literal')
    assert.equal(V.nonFiniteWord(-Infinity), '-Infinity')
    assert.equal(V.nonFiniteNamed('nope'), undefined)
  })

  // `json` takes an options record first, which holds `:non-finite` and
  // nothing else, `:reject` or `:null`; the plan carries the policy.
  it('json takes an options record before its events', () => {
    const policy = (options: string) => {
      const result = compile(`def export [input] (json ${options}input)`, 't.alc').result
      if ('text' !== result.v || 'json' !== result.plan.p) throw new Error('json answers its plan')
      return result.plan.nonFinite
    }
    assert.equal(policy(''), 'reject')
    assert.equal(policy('(record) '), 'reject')
    assert.equal(policy('(record (entry :non-finite :reject)) '), 'reject')
    assert.equal(policy('(record (entry :non-finite :null)) '), 'null')
    for (const [options, message] of [
      ['(record (entry :non-finite :literal))', 'json: :non-finite must be :reject or :null, not :literal'],
      ['(record (entry :non-finite :nope))', ':non-finite must be :reject, :null or :literal, not :nope'],
      ['(record (entry :non-finite :null) (entry :indent 2))', 'json: an option must be :non-finite, not :indent'],
    ]) {
      const f = thrown(() => compile(`def export [input] (json ${options} input)`, 't.alc'))
      assert.deepStrictEqual([f.code, f.message, f.row, f.col], ['DSL_TYPE_ERROR', `type_mismatch: ${message}`, 1, 20])
    }
  })

  // This package's own number text, as the Rust crate's `shortest_number`
  // is (rs/src/stdlib/registry.rs's test of the same name, case for case):
  // laid out as render's `writeValue` lays it out, which alchemy-cli's
  // differential tests hold it to.
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
    // The form 0.2.3 published, renderers first, still answers; the
    // renderers are not consulted.
    const unused = {} as any
    assert.equal(shortestNumber(unused, 50.25), '50.25')
    assert.equal(shortestNumber(unused, -0), '-0')
    assert.equal(numberText(unused, 2.5), '2.5')
    assert.equal(numberText(unused, 1.5, '1.50'), '1.50')
    assert.equal((thrown(() => numberText(unused, 1, '01')) as any).code, 'INVALID_NUMBER')
  })

  it('a datum value keeps its member order', () => {
    // A member named like an index stays where the document put it.
    const v = V.fromDatum(Datum.object([['b', Datum.null], ['1', Datum.null], ['a', Datum.null]]))
    assert.equal(V.toJsonText(v), '{"b":null,"1":null,"a":null}')
  })
})
