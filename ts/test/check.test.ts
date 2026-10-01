/* Copyright (c) 2026 tabnas, MIT License */

// The checker (rs/src/check.rs's tests), through the front end a host
// calls: `analyze` parses, desugars, resolves and checks; and, where a
// test needs the plan built (a live text refused where a vector is
// built), `compile`.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { SOURCES, analyze, analyzeSources, checkStdlibFile, compile, stdlib, stdlibSignature, types } from '../dist/alchemy'

import { finer, thrown } from './common'

const T = types

function check(src: string) {
  return analyze(src, 't.alc').checked
}

function code(src: string): [string, string, number | undefined, number | undefined] {
  const f = thrown(() => check(src))
  return [f.code, finer(f), f.row, f.col]
}

// The column of the `n`th (from 1) occurrence of `needle` in the line of
// `src` that holds it, 1-based: where a diagnostic should point.
function col(src: string, needle: string, n: number): number | undefined {
  const line = src.split('\n').find((l) => l.includes(needle))
  if (undefined === line) return undefined
  let at = -1
  for (let i = 0; i < n; i++) at = line.indexOf(needle, at + 1)
  return at + 1
}

const BINDING =
  'def column-from-meta [source]\n  record\n    entry :label (get "title" source)\n    entry :source (as-path (get "path" source))\ndef api-binding\n  record\n    entry :columns (path "response" "metadata" "fields")\n    entry :rows (path "response" "payload" "deep" "records" each-index)\n    entry :column column-from-meta\n'

function eq(a: any, b: any, message?: string) {
  assert.ok(T.typeEq(a, b), message ?? `${T.typeText(a)} is not ${T.typeText(b)}`)
}

describe('check', () => {
  it('the standard library checks clean against its signatures', () => {
    const lib = stdlib()
    lib.files.forEach((resolved: any, i: number) => checkStdlibFile(resolved, SOURCES[i][1]))
    for (const name of lib.names()) assert.ok(undefined !== stdlibSignature(name), `${name} has a signature`)
  })

  it('the worked example is a text over table events', () => {
    const checked = check(
      `${BINDING}def api-table [input]\n  table-from-json api-binding input\ndef export [input]\n  pipe input\n    api-table\n    csv csv-options\n`,
    )
    assert.equal(checked.output, 'Text')
    eq(checked.export, T.Text)
    eq(checked.defs.get('api-binding'), T.Record)
    eq(checked.defs.get('api-table'), T.func([T.Unknown], T.tableEvents()))
    eq(checked.defs.get('column-from-meta'), T.func([T.Unknown], T.Record))
  })

  it('outputs', () => {
    assert.equal(check('def export [input] input').output, 'JsonEvents/1')
    assert.equal(check('def export [input] (json input)').output, 'Text')
    assert.equal(check('def export [input] "x"').output, 'Text')
    assert.equal(check(`${BINDING}def export [input] (table-from-json api-binding input)`).output, 'TableRows/1')
    assert.equal(check(`${BINDING}def export [input] (records (table-from-json api-binding input))`).output, 'JsonEvents/1')
    // A user's own scan-emit is a stream of unknown items: rendered as a
    // table, checked at run time.
    const scan =
      'def step [s x] (transition s [(row [x])])\ndef fin [s] [table-end]\ndef export [input] (scan-emit null step fin (select (path each-index) input))'
    assert.equal(check(scan).output, 'TableRows/1')
    assert.equal(code('def export [input] (select (path each-index) input)')[1], 'bad_output')
    assert.equal(code('def export [input] 1')[1], 'bad_output')
    assert.equal(code('def export [input] csv-options')[1], 'bad_output')
    const [c, f] = code('def export [input] (get :x csv-options)')
    assert.deepStrictEqual([c, f], ['STREAMABILITY_UNKNOWN', 'unknown_output'])
    assert.equal(code('def x 1')[1], 'no_export')
    assert.equal(code('def export 1')[1], 'type_mismatch')
    assert.equal(code('def export [a b] a')[1], 'arity')
  })

  it('arity, type and protocol mismatches', () => {
    assert.equal(code('def export [input] (json input 1)')[1], 'arity')
    assert.equal(code('def export [input] (csv csv-options)')[1], 'arity')
    assert.equal(code('def f [a b] a\ndef export [input] (json (f input))')[1], 'arity')
    assert.equal(code('def export [input] (text 1 (json input))')[1], 'arity')
    const [c, f, row, column] = code('def export [input] (json (text 1))')
    assert.deepStrictEqual([c, f, row, column], ['DSL_TYPE_ERROR', 'type_mismatch', 1, 32])
    assert.equal(code('def export [input] (if 1 (json input) (json input))')[1], 'type_mismatch')
    assert.equal(code('def export [input] (csv csv-options input)')[1], 'protocol_mismatch')
    assert.equal(code('def export [input] (table-from-json csv-options (json input))')[1], 'protocol_mismatch')
    assert.equal(code('def export [input] (json (select (path each-index) input))')[1], 'protocol_mismatch')
    assert.equal(code('def export [input] (map (fn [x] x) input)')[1], 'protocol_mismatch')
    assert.equal(code('def export [input] (records input)')[1], 'protocol_mismatch')
    assert.equal(code('def export [input] (json [input])')[1], 'type_mismatch')
    assert.equal(code('def export [input] (json (vector input))')[1], 'type_mismatch')
    assert.equal(code('def export [input] (concat (record (entry :x input)))')[1], 'type_mismatch')
    assert.equal(code('def export [input] (concat 1 (json input))')[1], 'type_mismatch')
    assert.equal(code('def export [input] (1 input)')[1], 'type_mismatch')
    assert.equal(code('def export [input] (csv-options input)')[1], 'type_mismatch')
    assert.equal(code('def export [input] (concat-map (fn [x] 1) (select (path each-index) input))')[1], 'type_mismatch')
    assert.equal(code('def export [input] (join 1 (select (path each-index) input))')[1], 'type_mismatch')
    assert.equal(code('def export [input] (match input (case (selected :a) "x"))')[1], 'arity')
  })

  it('streams are affine', () => {
    const [c, f, row, column] = code('def export [input] (concat (json input) (json input))')
    assert.deepStrictEqual([c, f, row, column], ['STREAM_REUSED', 'reused', 1, 47])
    assert.deepStrictEqual(
      code('def export [input]\n  concat-map (fn [x] (json input)) (select (path each-index) input)').slice(0, 2),
      ['STREAM_REUSED', 'captured'],
    )
    assert.deepStrictEqual(
      code('def export [input]\n  let [s (select (path each-index) input)]\n    concat (join "," s) (join ";" s)').slice(0, 2),
      ['STREAM_REUSED', 'reused'],
    )
    // Alternatives are not two uses.
    check('def export [input] (if true (json input) (json input))')
    check('def export [input]\n  match 1\n    case 1 (json input)\n    case _ (json input)')
    check('def export [input]\n  let [s (select (path each-index) input)]\n    join "," s')
    // Shadowing ends the scope.
    check('def export [input] (concat (json input) (concat-map (fn [input] input) ["a"]))')
  })

  it('a stream passed to a definition is affine in its body', () => {
    let src = 'def g [s] (concat-map (fn [x] (json s)) [1])\ndef export [input] (g input)'
    let r = code(src)
    assert.deepStrictEqual(r, ['STREAM_REUSED', 'captured', 1, col(src, 's)', 1)])
    assert.equal(code('def g [s] (join "," (map (fn [x] (json s)) [1 2]))\ndef export [input] (g input)')[1], 'captured')
    // Returned inside a fn, to be applied later.
    src = 'def mk [s] (fn [] s)\ndef export [input]\n  let [g (mk input)]\n    concat (json (g)) "x"'
    r = code(src)
    assert.deepStrictEqual(r, ['STREAM_REUSED', 'captured', 1, col(src, 's)', 1)])
    // Used twice in the body.
    src = 'def twice [s] (concat (json s) (json s))\ndef export [input] (twice input)'
    r = code(src)
    assert.deepStrictEqual(r, ['STREAM_REUSED', 'reused', 1, col(src, 's)', 2)])
    // Held by a partial application or a record, to be used again.
    assert.equal(code('def mk [s] (partial json s)\ndef export [input]\n  let [g (mk input)]\n    concat (g) (g)')[1], 'type_mismatch')
    assert.equal(
      code('def mk [s] (record (entry :s s))\ndef export [input]\n  let [r (mk input)]\n    concat (json (get :s r)) (json (get :s r))')[1],
      'type_mismatch',
    )
    // Through a chain of definitions, at the use that captures it.
    r = code('def h [t] (concat-map (fn [x] (json t)) [1])\ndef g [s] (h s)\ndef export [input] (g input)')
    assert.deepStrictEqual(r.slice(0, 3), ['STREAM_REUSED', 'captured', 1])
    assert.equal(code('def twice [t] (concat t t)\ndef export [input] (twice (json input))')[1], 'reused')
    assert.equal(code('def export [input] ((fn [s] (concat (json s) (json s))) input)')[1], 'reused')
    // Passed once, it is fine.
    assert.equal(check('def g [s] s\ndef export [input] (g input)').output, 'JsonEvents/1')
    eq(check('def g [s] (json s)\ndef export [input] (g input)').export, T.Text)
    check('def g [s] (if true (json s) (json s))\ndef export [input] (g input)')
  })

  it('a vector may hold a finite text and never a stream', () => {
    check('def export [input] (concat (join "," [(text "i")]) (json input))')
    check('def export [input] (concat (join "," (vector (text "i") "j")) (json input))')
    check('def export [input] (concat (join "," (map text ["a" "b"])) (json input))')
    assert.equal(code('def export [input] (json [input])')[1], 'type_mismatch')
    assert.equal(code('def export [input] (join "," [(select (path each-index) input)])')[1], 'type_mismatch')
    assert.equal(code('def export [input] (join "," (vector (select (path each-index) input)))')[1], 'type_mismatch')
    // A live text in a vector is refused where the vector is built, with
    // its name: the evaluator's; the front end passes it.
    assert.equal(check('def export [input] (join "," [(json input)])').output, 'Text')
    let f = thrown(() => compile('def export [input] (join "," [(json input)])', 't.alc'))
    assert.equal(f.code, 'DSL_TYPE_ERROR')
    assert.match(f.message, /cannot hold a live text/)
    f = thrown(() => compile('def export [input] (join "," (vector (json input)))', 't.alc'))
    assert.match(f.message, /cannot hold a live text/)
    assert.equal(compile('def export [input] (concat (join "," [(text "i") "j"]) (json input))', 't.alc').output, 'Text')
  })

  it('a chain of definitions is typed without nesting the checker', () => {
    let src = 'def a0 1\n'
    for (let i = 1; i < 10_000; i++) src += `def a${i} a${i - 1}\n`
    src += 'def export [input] (if false (let [y a9999] (json input)) (json input))\n'
    eq(check(src).defs.get('a9999'), T.Num)
    // A stream passed down five hundred definitions is followed to
    // MAX_APPLIED of them.
    src = 'def f0 [s] (json s)\n'
    for (let i = 1; i < 500; i++) src += `def f${i} [s] (f${i - 1} s)\n`
    src += 'def export [input] (f499 input)\n'
    check(src)
  })

  it('strict mode wants static functions over streams', () => {
    const [c, f] = code(
      'def go [f input] (concat-map f (select (path each-index) input))\ndef export [input] (go text input)',
    )
    assert.deepStrictEqual([c, f], ['STREAMABILITY_UNKNOWN', 'dynamic'])
    assert.equal(
      code('def export [input] (scan-emit null (get :f csv-options) (fn [s] []) (select (path each-index) input))')[1],
      'dynamic',
    )
    check(
      'def step [b s x] (transition s [x])\ndef fin [s] []\ndef export [input] (join "," (scan-emit null (partial step 1) fin (select (path each-index) input)))',
    )
    check('def export [input] (concat-map text (select (path each-index) input))')
    check('def export [input] (concat-map (get :f csv-options) ["a"])')
    assert.equal(
      code('def step [s x] [x]\ndef fin [s] []\ndef export [input] (join "," (scan-emit null step fin (select (path each-index) input)))')[1],
      'type_mismatch',
    )
    assert.equal(
      code('def step [s] s\ndef fin [s] []\ndef export [input] (join "," (scan-emit null step fin (select (path each-index) input)))')[1],
      'arity',
    )
  })

  it('a definition over items is checked as its eta expansion', () => {
    const [c, f, row, column] = code('def bad (map public-column [1])\ndef export [input] (json input)')
    assert.deepStrictEqual([c, f, row, column], ['DSL_TYPE_ERROR', 'type_mismatch', 1, 28])
    const fail = thrown(() => check('def bad (filter public-column [1])\ndef export [input] (json input)'))
    assert.ok(
      fail.message.startsWith('type_mismatch: an item given to the function of filter must be Record, not Number'),
      fail.message,
    )
    assert.deepStrictEqual([fail.row, fail.col], [1, 31])
    assert.equal(
      code('def export [input]\n  concat-map (partial csv-row csv-options) (map (fn [v] 1) (select (path each-index) input))')[1],
      'type_mismatch',
    )
    assert.equal(
      code(
        'def export [input]\n  concat-map (fn [r] (scalar-text csv-options (get :label r))) (map public-column (map (fn [v] "s") (select (path each-index) input)))',
      )[1],
      'type_mismatch',
    )
    for (const fn of ['public-column', '(fn [x] (public-column x))']) {
      assert.equal(code(`def bad (map ${fn} [1])\ndef export [input] (json input)`)[1], 'type_mismatch', fn)
      check(`def ok (map ${fn} [(record (entry :label "x"))])\ndef export [input] (json input)`)
      check(`def ok [xs] (map ${fn} xs)\ndef export [input] (json input)`)
      check(
        `def cols [input] (map ${fn} (select (path "cols" each-index) input))\ndef export [input] (join "," (map (fn [c] (get :label c)) (cols input)))`,
      )
    }
    for (const fn of ['(partial csv-row csv-options)', '(fn [cells] (csv-row csv-options cells))']) {
      check(`def export [input]\n  concat-map ${fn} (select (path "rows" each-index) input)`)
    }
    check('def ok (map (partial get :a) [1])\ndef export [input] (json input)')
    assert.equal(code('def bad (map (fn [x] (get :a x)) [1])\ndef export [input] (json input)')[1], 'type_mismatch')
  })

  it('events and the stack operators', () => {
    const render =
      'def export [input]\n  join ""\n    map\n      fn [e]\n        match e\n          case (key n) (quoted n)\n          case (scalar v) (scalar-text csv-options v)\n          case object-start "{"\n          case _ ""\n      events input'
    eq(check(render).export, T.Text)
    const keep =
      'def keep [s e] (transition (push e s) [])\ndef fin [s] [(repeat (count s) "  ") (quoted (top s))]\ndef export [input] (join "" (scan-emit [] keep fin (events input)))'
    const checked = check(keep)
    assert.equal(checked.output, 'Text')
    eq(checked.defs.get('fin'), T.func([T.Unknown], T.vectorOf(T.Str)))
    const defs = check('def s (push :b [:a])\ndef n (count s)\ndef t (top s)\ndef p (pop s)\ndef export [input] (json input)').defs
    assert.deepStrictEqual([...defs.keys()], ['s', 'n', 't', 'p'])
    eq(defs.get('s'), T.vectorOf(T.Keyword))
    eq(defs.get('n'), T.Num)
    eq(defs.get('t'), T.Keyword)
    eq(defs.get('p'), T.vectorOf(T.Keyword))
    assert.equal(code('def export [input] (events input)')[1], 'bad_output')
    eq(check('def export [input] (json (events input))').export, T.Text)
    assert.equal(code('def export [input] (json (select (path each-index) input))')[1], 'protocol_mismatch')
    assert.equal(code('def export [input] (csv csv-options (events input))')[1], 'protocol_mismatch')
    assert.equal(code('def export [input] (events (select (path each-index) input))')[1], 'protocol_mismatch')
    assert.deepStrictEqual(
      code('def export [input] (concat (json input) (join "" (map (fn [e] "") (events input))))').slice(0, 2),
      ['STREAM_REUSED', 'reused'],
    )
    for (const bad of [
      'def export [input] (let [x (push (events input) [])] "done")',
      'def export [input] (let [x (count 1)] (json input))',
      'def export [input] (let [x (top csv-options)] (json input))',
      'def export [input] (let [x (quoted 1)] (json input))',
      'def export [input] (let [x (repeat "a" 1)] (json input))',
      'def export [input] (let [x (key :a)] (json input))',
      'def export [input] (let [x (scalar csv-options)] (json input))',
    ]) {
      assert.equal(code(bad)[1], 'type_mismatch', bad)
    }
    assert.equal(
      code('def export [input] (join "" (map (fn [e] (match e (case (key a b) a) (case _ ""))) (events input)))')[1],
      'arity',
    )
  })

  it('patterns type their bindings', () => {
    check(
      'def export [input]\n  concat-map\n    fn [e]\n      match e\n        case (row cells) (join "," (map (fn [c] (scalar-text csv-options c)) cells))\n        case _ ""\n    select (path each-index) input',
    )
    check(
      'def export [input]\n  concat-map\n    fn [e]\n      match e\n        case table-end "end"\n        case other (scalar-text csv-options other)\n    select (path each-index) input',
    )
  })
})

// The linking of several sources (rs/tests/sources_test.rs, the front
// end's part): a failure names its file, and the linking's refusals.
describe('sources', () => {
  it('a failure in one of several sources names its file and position', () => {
    const fail = thrown(() =>
      analyzeSources([
        { file: 'a.alc', text: 'def helper [x] (json x)' },
        { file: 'b.alc', text: 'def export [input]\n  nope input' },
      ]),
    )
    assert.equal(finer(fail), 'unknown_name')
    assert.deepStrictEqual([fail.file, fail.row, fail.col], ['b.alc', 2, 3])
    const parseFail = thrown(() =>
      analyzeSources([
        { file: 'a.alc', text: 'def export [input] (json input)' },
        { file: 'b.alc', text: '(a b' },
      ]),
    )
    assert.equal(finer(parseFail), 'unbalanced')
    assert.equal(parseFail.file, 'b.alc')
    // One source names no file.
    assert.equal(thrown(() => analyze('def export [input]\n  nope input', 'one.alc')).file, undefined)
  })

  it('the linking refuses a file given twice, and an export it cannot link', () => {
    assert.equal(
      finer(
        thrown(() =>
          analyzeSources([
            { file: 'a.alc', text: 'def x 1' },
            { file: 'a.alc', text: 'def y 2' },
          ]),
        ),
      ),
      'duplicate_file',
    )
    assert.equal(finer(thrown(() => analyzeSources([]))), 'no_export')
    const noExport = thrown(() =>
      analyzeSources([
        { file: 'p.alc', text: 'def x 1', exportAs: 'program-export' },
        { file: 'f.alc', text: 'def export [input] input' },
      ]),
    )
    assert.equal(finer(noExport), 'no_export')
    assert.equal(noExport.file, 'p.alc')
    const taken = thrown(() =>
      analyzeSources([
        { file: 'p.alc', text: 'def export [program-export] (json program-export)', exportAs: 'program-export' },
        { file: 'f.alc', text: 'def export [input] input' },
      ]),
    )
    assert.equal(finer(taken), 'duplicate_def')
    assert.deepStrictEqual([taken.file, taken.row, taken.col], ['p.alc', 1, 13])
  })

  it('a program linked under another name feeds the export', () => {
    const analyzed = analyzeSources([
      { file: 'program.alc', text: 'def export [input] (json input)', exportAs: 'program-export' },
      { file: 'format.alc', text: 'def export [input] (concat "<" (program-export input) ">")' },
    ])
    assert.equal(analyzed.checked.output, 'Text')
    assert.deepStrictEqual([...analyzed.resolved.defs.keys()], ['program-export', 'export'])
  })
})
