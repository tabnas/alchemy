/* Copyright (c) 2026 tabnas, MIT License */

// The syntax tree and its printers (rs/src/ast.rs's tests), and the
// desugarer (rs/src/desugar.rs's tests).

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { Fail } from '@tabnas/transduce'

import {
  Expr,
  MAX_NESTING,
  Sources,
  canonical,
  canonicalForm,
  desugarProgram,
  format,
  fromValue,
  parse,
  position,
  sameProgram,
  sameShape,
  sourceFile,
  span,
} from '../dist/alchemy'

import { finer, thrown } from './common'

const T = sourceFile('t')

function sym(name: string): Expr {
  return { kind: 'symbol', name, span: span(T, 0, 0) }
}

// The tagged tree for `x` nested in `depth` lists, as a layered grammar
// could hand over without the reader's own bound.
function nestedTree(depth: number): unknown {
  let tree: unknown = { $: 'sym', name: 'x', span: [0, 1] }
  for (let i = 0; i < depth; i++) tree = { $: 'list', items: [tree], span: [0, 1] }
  return tree
}

describe('ast', () => {
  // A span is positioned in its own file's text, and a program of several
  // sources names the file; one source, or a span of a file not among
  // them, names none.
  it('sources position a span in its file and name it when several', () => {
    const a = sourceFile('a.alc')
    const b = sourceFile('b.alc')
    let f = Sources.one('a.alc', 'x\ny').failAt(new Fail('INPUT_INVALID', 'm'), span(a, 2, 3))
    assert.deepStrictEqual([f.row, f.col, f.file], [2, 1, undefined])
    assert.equal(f.toString(), 'INPUT_INVALID: m (2:1)')
    const two = Sources.several([
      ['a.alc', 'x\ny'],
      ['b.alc', '\n\nz'],
    ])
    f = two.failAt(new Fail('INPUT_INVALID', 'm'), span(b, 2, 3))
    assert.deepStrictEqual([f.row, f.col, f.file], [3, 1, 'b.alc'])
    assert.equal(f.toString(), 'INPUT_INVALID: m (b.alc:3:1)')
    f = two.failAt(new Fail('INPUT_INVALID', 'm'), span(a, 2, 3))
    assert.deepStrictEqual([f.row, f.col, f.file], [2, 1, 'a.alc'])
    f = two.failAt(new Fail('INPUT_INVALID', 'm'), span(sourceFile('stdlib/table.alc'), 2, 3))
    assert.deepStrictEqual([f.row, f.col, f.file], [2, 1, undefined])
  })

  it('positions are one-based rows and character columns', () => {
    const src = 'ab\ncdé f\n'
    assert.deepStrictEqual(position(span(T, 0, 1), src), [1, 1])
    assert.deepStrictEqual(position(span(T, 3, 4), src), [2, 1])
    // `é` is one column (and, in a JavaScript string, one index).
    assert.deepStrictEqual(position(span(T, 7, 8), src), [2, 5])
    assert.deepStrictEqual(position(span(T, 99, 99), src), [3, 1])
    // A character outside the BMP is one column, two indexes.
    assert.deepStrictEqual(position(span(T, 3, 4), '🚀 x'), [1, 3])
    // A lone carriage return restarts the column and not the row; `\r\n`
    // is one line end.
    assert.deepStrictEqual(position(span(T, 3, 4), 'a\r b'), [1, 2])
    assert.deepStrictEqual(position(span(T, 3, 4), 'a\r\nb'), [2, 1])
  })

  it('same shape ignores spans and nothing else', () => {
    const a = parse('a (b 1) ["x"]')
    const b = parse('a\n  b 1\n  ["x"]')
    assert.ok(sameProgram(a, b))
    assert.ok(!sameProgram(a, parse('a (b 2) ["x"]')))
    assert.ok(!sameShape(sym('a'), { kind: 'keyword', name: 'a', span: span(T, 0, 0) }))
  })

  it('canonical prints every atom kind', () => {
    const program = parse('s :k "a\\"b\\n" -1.5e3 true false null [] ()')
    assert.equal(canonical(program), '(s :k "a\\"b\\n" -1.5e3 true false null [] ())')
    // Strings as serde_json writes them: controls escaped, lower-case hex,
    // DEL and everything above it as itself.
    assert.equal(canonical(parse('"\\u0001\\u001f\\u007f\\b\\f\\t\\r é"')), '"\\u0001\\u001f\u007f\\b\\f\\t\\r é"')
  })

  it('canonical joins top-level forms by line', () => {
    assert.equal(canonical(parse('join ","\n  map csv-field values\n\n(newline)')), '(join "," (map csv-field values))\n(newline)')
  })

  it('layout keeps explicit parens only where layout cannot express the shape', () => {
    const program = parse('(concat prefix (newline) suffix)\n((f x) y)\n(a (b c) d)')
    assert.equal(format(program), 'concat prefix\n  (newline)\n  suffix\n\n((f x) y)\n\na\n  b c\n  d\n')
    const def = parse('(def csv-row [values] (concat (join "," (map csv-field values)) (newline)))')
    assert.equal(format(def), 'def csv-row [values]\n  concat\n    join ","\n      map csv-field values\n    (newline)\n')
  })

  it('a tagged tree at the bound converts and one past it is too deep', () => {
    const atLimit = fromValue(nestedTree(MAX_NESTING), T)
    assert.equal(canonicalForm(atLimit), '('.repeat(MAX_NESTING) + 'x' + ')'.repeat(MAX_NESTING))
    const fail = thrown(() => fromValue(nestedTree(MAX_NESTING + 1), T))
    assert.equal(fail.code, 'DSL_PARSE_ERROR')
    assert.ok(fail.message.startsWith('too_deep: '), fail.message)
  })

  it('a far deeper tagged tree is refused without recursing into it', () => {
    const fail = thrown(() => fromValue(nestedTree(100_000), T))
    assert.ok(fail.message.startsWith('too_deep: '), fail.message)
  })

  it('malformed reader output is a parse error, not a crash', () => {
    const fail = thrown(() => fromValue('nope', T))
    assert.equal(fail.code, 'DSL_PARSE_ERROR')
    assert.ok(fail instanceof Fail)
    assert.throws(() => fromValue({ $: 'sym', span: [0] }, T), Fail)
    assert.throws(() => fromValue({ $: 'sym', name: 1, span: [0, 1] }, T), Fail)
    assert.throws(() => fromValue({ $: 'what', span: [0, 1] }, T), Fail)
    assert.throws(() => fromValue({ $: 'num', lexeme: '1', span: [0.5, 1] }, T), Fail)
  })
})

function core(src: string): string {
  return canonical(desugarProgram(parse(src), src))
}

function failure(src: string): any {
  const forms = parse(src)
  return thrown(() => desugarProgram(forms, src))
}

describe('desugar', () => {
  it('a def with parameters becomes a def of a fn', () => {
    assert.equal(core('def csv-field [value]\n  scalar-text value'), '(def csv-field (fn [value] (scalar-text value)))')
    assert.equal(core('def sep ","'), '(def sep ",")')
  })

  it('the generated fn carries the def span', () => {
    const src = 'def f [x]\n  x'
    const [def] = desugarProgram(parse(src), src) as any[]
    const fn = def.items[2]
    assert.deepStrictEqual(fn.span, def.span)
    assert.deepStrictEqual(fn.items[0].span, def.span)
    assert.deepStrictEqual([fn.items[1].span.start, fn.items[1].span.end], [6, 9])
    assert.deepStrictEqual([fn.items[2].span.start, fn.items[2].span.end], [12, 13])
  })

  it('pipe threads the value data-last', () => {
    assert.equal(
      core('pipe input (select (path "payload" "records" each-index)) (map normalize) (filter active?)'),
      '(filter active? (map normalize (select (path "payload" "records" each-index) input)))',
    )
    assert.equal(core('pipe x f g'), '(g (f x))')
    assert.equal(core('pipe x'), 'x')
    assert.equal(core('(pipe)'), '(pipe)')
  })

  it('a pipe inside a def desugars both', () => {
    assert.equal(
      core('def export [input]\n  pipe input\n    table-from-json api-binding\n    csv csv-options'),
      '(def export (fn [input] (csv csv-options (table-from-json api-binding input))))',
    )
  })

  it('a step that is not applicable is an empty step error', () => {
    for (const [src, col] of [
      ['pipe x ()', 8],
      ['pipe x 1', 8],
      ['pipe x "s"', 8],
      ['pipe x [f]', 8],
      ['pipe x :k', 8],
      ['pipe x\n  f\n  []', 3],
      // A lone carriage return restarts the column, as the engine counts it.
      ['pipe x\r ()', 2],
    ] as Array<[string, number]>) {
      const fail = failure(src)
      assert.equal(fail.code, 'DSL_PARSE_ERROR', src)
      assert.equal(finer(fail), 'empty_step', src)
      assert.equal(fail.col, col, src)
    }
    assert.equal(failure('pipe x\n  f\n  []').row, 3)
  })

  it('the core form shapes are checked', () => {
    assert.equal(core('let [x 1] x'), '(let [x 1] x)')
    assert.equal(core('if a b c'), '(if a b c)')
    assert.equal(core('match v\n  case 1 "one"\n  case _ "other"'), '(match v (case 1 "one") (case _ "other"))')
    assert.equal(core('match v'), '(match v)')
    for (const [src, c] of [
      ['let [x] x', 'bad_let'],
      ['let [x 1 2] x', 'bad_let'],
      ['let [1 x] x', 'bad_let'],
      ['let [x 1]', 'bad_let'],
      ['if a b', 'bad_if'],
      ['if a b c d', 'bad_if'],
      ['(match)', 'bad_match'],
      ['match v (case 1)', 'bad_match'],
      ['match v (when 1 2)', 'bad_match'],
      ['match v 1', 'bad_match'],
      ['(def)', 'bad_def'],
      ['def x', 'bad_def'],
      ['def 1 2', 'bad_def'],
      ['def x y z', 'bad_def'],
      ['def x [a] b c', 'bad_def'],
    ]) {
      const fail = failure(src)
      assert.equal(finer(fail), c, src)
      assert.deepStrictEqual([fail.row, fail.col], [1, 1], src)
    }
  })

  it('a rewrite that nests past the bound is too deep at the form', () => {
    // A pipe nests one level per step: 256 steps reach the bound.
    const atLimit = 'pipe x' + ' f'.repeat(MAX_NESTING)
    assert.equal(core(atLimit), '(f '.repeat(MAX_NESTING) + 'x' + ')'.repeat(MAX_NESTING))
    const fail = failure('pipe x' + ' f'.repeat(MAX_NESTING + 1))
    assert.equal(finer(fail), 'too_deep')
    assert.deepStrictEqual([fail.row, fail.col], [1, 1])
    // A pipe of a hundred thousand steps reads flat and fails at the 257th
    // step, without building the chain.
    assert.equal(finer(failure('pipe x' + ' f'.repeat(100_000))), 'too_deep')
    // `def` adds the `fn` level.
    const body = (depth: number) => '('.repeat(depth) + 'x' + ')'.repeat(depth)
    assert.ok(core(`def f [x] ${body(MAX_NESTING - 2)}`).startsWith('(def f (fn [x] '))
    assert.equal(finer(failure(`def f [x] ${body(MAX_NESTING - 1)}`)), 'too_deep')
  })

  it('other forms pass through unchanged', () => {
    assert.equal(
      core('fn [x] (get :label x)\n(case 1 2)\ndef x (pipe y f)'),
      '(fn [x] (get :label x))\n(case 1 2)\n(def x (f y))',
    )
  })
})
