/* Copyright (c) 2026 tabnas, MIT License */

// The differential test (rs/tests/stdlib_test.rs): the standard library's
// own text, interpreted, against the native compositions the runtime
// substitutes for it. Over every transduce fixture a grammar these tests
// can read, and the generated documents transduce's tests and benches
// share, the spec's worked-example program produces the same bytes both
// ways, or fails with the same code. The library text is the reference;
// the native path is the optimization, and this is what makes it one.

import { describe, it } from 'node:test'
import assert from 'node:assert'
import { readFileSync, readdirSync } from 'node:fs'
import { extname, join } from 'node:path'

import { Tabnas } from '@tabnas/parser'
import { jsonic } from '@tabnas/jsonic'
import { make as makeCsv } from '@tabnas/csv'
import { make as makeJson } from '@tabnas/json'
import { make as makeJsonl } from '@tabnas/jsonl'
import { Yaml } from '@tabnas/yaml'
import { BytesWriter } from '@tabnas/render'
import { Code, Ev, Limits, Metrics, replay } from '@tabnas/transduce'

import { Program, stdlib } from '../dist/alchemy'

import { EXPECTED_CSV, PROGRAM, RECORDS, compile, events, replayed } from './common'
import { FIXTURES, METADATA, record, recordsCsv, recordsJson, recordsJsonl, recordsYaml } from './support'

type Make = () => any

// The grammar for a fixture, by extension, among those these tests take;
// undefined skips the fixture.
function grammarFor(path: string): Make | undefined {
  switch (extname(path)) {
    case '.json':
      return () => makeJson()
    case '.jsonl':
      return () => makeJsonl()
    case '.yaml':
      return () => new Tabnas().use(jsonic).use(Yaml)
    case '.csv':
    case '.tsv':
      return () => makeCsv()
    default:
      return undefined
  }
}

type Outcome = { ok: true; text: string } | { ok: false; fail: any }

function attempt(work: () => string): Outcome {
  try {
    return { ok: true, text: work() }
  } catch (fail) {
    return { ok: false, fail }
  }
}

// One document, both ways: the same bytes, or the same code; a report of
// the difference otherwise.
function differential(name: string, program: Program, evs: any[]): string | undefined {
  const interpreted = program.withNative(false)
  assert.ok(program.native && !interpreted.native)
  const a = attempt(() => replayed(program, evs))
  const b = attempt(() => replayed(interpreted, evs))
  if (a.ok && b.ok) {
    return a.text === b.text
      ? undefined
      : `${name}: the bytes differ\n  native:      ${JSON.stringify(a.text)}\n  interpreted: ${JSON.stringify(b.text)}`
  }
  if (!a.ok && !b.ok) {
    return a.fail?.code === b.fail?.code
      ? undefined
      : `${name}: the codes differ\n  native:      ${a.fail}\n  interpreted: ${b.fail}`
  }
  if (a.ok && !b.ok) return `${name}: native produced ${JSON.stringify(a.text)}, interpreted failed: ${b.fail}`
  return `${name}: interpreted produced ${JSON.stringify((b as any).text)}, native failed: ${(a as any).fail}`
}

// A document to compare: its name, the grammar that reads it, its text.
type Document = [string, Make, string]

// Both ways, one outcome: the same bytes and the same rows counted, or the
// same code and the same limit named; `expected` is that outcome.
function agree(
  name: string,
  program: Program,
  evs: any[],
  limits: Limits,
  expected: { ok: string } | { code: Code },
): void {
  const interpreted = program.withNative(false)
  const nativeRows = new Metrics()
  const interpretedRows = new Metrics()
  const a = attempt(() => replayed(program, evs, undefined, limits, nativeRows))
  const b = attempt(() => replayed(interpreted, evs, undefined, limits, interpretedRows))
  if ('ok' in expected) {
    assert.ok(a.ok && b.ok, `${name}: expected ${JSON.stringify(expected.ok)}\n  native: ${a.ok ? a.text : a.fail}\n  interpreted: ${b.ok ? b.text : b.fail}`)
    assert.equal(a.text, expected.ok, `${name}: native`)
    assert.equal(b.text, expected.ok, `${name}: interpreted`)
    assert.equal(nativeRows.rows, interpretedRows.rows, `${name}: rows counted`)
    return
  }
  assert.ok(!a.ok && !b.ok, `${name}: expected ${expected.code}\n  native: ${a.ok ? JSON.stringify(a.text) : a.fail}\n  interpreted: ${b.ok ? JSON.stringify(b.text) : b.fail}`)
  assert.deepStrictEqual([a.fail.code, b.fail.code], [expected.code, expected.code], `${name}:\n  native: ${a.fail}\n  interpreted: ${b.fail}`)
  assert.equal(a.fail.limit?.name, b.fail.limit?.name, `${name}: the limit named`)
}

const json = (text: string): any[] => events(makeJson(), text) as any[]

describe('differential', () => {
  it('the library loads', () => {
    const lib = stdlib()
    assert.ok(undefined !== lib.get('table-from-json'))
    assert.ok(undefined !== lib.get('csv'))
  })

  // Every transduce fixture a grammar here reads, plus the generated
  // documents in every shape: identical bytes or identical codes.
  it('interpreted and native agree on every fixture and generated document', () => {
    const program = compile(PROGRAM, 'export.alc')
    const documents: Document[] = []
    const entries = readdirSync(FIXTURES)
      .map((f) => join(FIXTURES, f))
      .sort()
    for (const path of entries) {
      const make = grammarFor(path)
      if (undefined === make) continue
      documents.push([path, make, readFileSync(path, 'utf8')])
    }
    for (const n of [0, 1, 2, 3, 50]) documents.push([`recordsJson(${n})`, () => makeJson(), recordsJson(n)])
    documents.push(['recordsYaml(3)', () => new Tabnas().use(jsonic).use(Yaml), recordsYaml(3)])
    documents.push(['recordsJsonl(3)', () => makeJsonl(), recordsJsonl(3)])
    documents.push(['recordsCsv(3)', () => makeCsv(), recordsCsv(3)])
    // Cells of every kind, in the worked example's shape: null, missing
    // members (a failure under the standard options), nested containers,
    // booleans, quotes, delimiters, line breaks and non-ASCII text.
    const shaped = (records: string) =>
      `{"response":{"metadata":${METADATA},"payload":{"deep":{"records":[${records}]}}}}`
    for (const [name, records] of [
      ['nulls', '{"id":null,"person":{"name":null},"account":{"balance":null}}'],
      ['missing', '{"id":1,"person":{},"account":{"balance":2}}'],
      ['containers', '{"id":[1,2.50],"person":{"name":{"first":"A","last":"B"}},"account":{"balance":{}}}'],
      ['booleans', '{"id":true,"person":{"name":false},"account":{"balance":0}}'],
      ['quotes', '{"id":"say \\"hi\\"","person":{"name":"a,b"},"account":{"balance":"line\\r\\nbreak"}}'],
      ['unicode', '{"id":"caf\\u00e9","person":{"name":"日本"},"account":{"balance":"\\ud83d\\ude00"}}'],
      ['lexemes', '{"id":1e2,"person":{"name":"x"},"account":{"balance":-0.0}}'],
      ['big', '{"id":12345678901234567890123,"person":{"name":"x"},"account":{"balance":1E+2}}'],
      [
        'two rows out of order',
        '{"account":{"balance":1},"id":2,"person":{"name":"z"}},{"id":3,"person":{"name":"y"},"account":{"balance":4}}',
      ],
    ]) {
      documents.push([name, () => makeJson(), shaped(records)])
    }
    // The order contract and the absent shapes.
    documents.push([
      'metadata after rows',
      () => makeJson(),
      `{"response":{"payload":{"deep":{"records":[${record(1)}]}},"metadata":${METADATA}}}`,
    ])
    documents.push(['no metadata', () => makeJson(), `{"response":{"payload":{"deep":{"records":[${record(1)}]}}}}`])
    documents.push(['no records', () => makeJson(), `{"response":{"metadata":${METADATA}}}`])
    documents.push(['metadata not an array', () => makeJson(), '{"response":{"metadata":{"fields":{}}}}'])
    documents.push(['descriptor without title', () => makeJson(), '{"response":{"metadata":{"fields":[{"path":["a"]}]}}}'])
    documents.push([
      'descriptor with a bad segment',
      () => makeJson(),
      '{"response":{"metadata":{"fields":[{"title":"a","path":[true]}]}}}',
    ])
    documents.push(['row that is not an object', () => makeJson(), shaped('1')])

    const total = documents.length
    const failures: string[] = []
    let skipped = 0
    documents.forEach(([name, make, text], i) => {
      if (0 === i % 10) console.log(`differential: ${i} of ${total} (${Math.floor((i * 100) / total)}%)`)
      const evs = events(make(), text)
      if (undefined === evs) {
        // The grammar refused the document: nothing reached a program, so
        // there is nothing to compare.
        skipped++
        return
      }
      const report = differential(name, program, evs)
      if (undefined !== report) failures.push(report)
    })
    console.log(`differential: ${total} of ${total} (100%), ${skipped} unreadable`)
    assert.ok(total - skipped > 20, 'enough documents were read')
    assert.deepStrictEqual(failures, [], `${failures.length} document(s) differ`)
  })

  // The same, with the JSON renderer over the table both ways (`records`
  // then `json`), and with a program whose options record changes the
  // dialect the native renderer takes.
  it('interpreted and native agree on other renderings', () => {
    const program = compile(PROGRAM.replace('    csv csv-options\n', '    records\n    json\n'), 'records.alc')
    const dialect = compile(
      PROGRAM.replace(
        '    csv csv-options\n',
        '    csv options\n\ndef options\n  record\n    entry :delimiter ";"\n    entry :newline "\\n"\n    entry :header false\n    entry :null-text "NULL"\n    entry :missing "-"\n',
      ),
      'dialect.alc',
    )
    const docs = [
      recordsJson(3),
      `{"response":{"metadata":${METADATA},"payload":{"deep":{"records":[{"id":null,"person":{},"account":{"balance":"a;b"}}]}}}}`,
    ]
    docs.forEach((doc, i) => {
      const evs = json(doc)
      assert.equal(differential(`records/json ${i}`, program, evs), undefined)
      assert.equal(differential(`dialect ${i}`, dialect, evs), undefined)
    })
    assert.equal(replayed(dialect, json(docs[1])), '"NULL";"-";"a;b"\n')
  })

  // The library's `csv` validates the table events it renders as the
  // native renderer does (spec 13.2: the protocol validator is not
  // omitted), so a program that produces its own table events, or a
  // document or an options record the renderer refuses, fails with the
  // same code both ways, and a sound one prints the same bytes.
  it('the library csv validates what the renderer validates', () => {
    const items = json('{"items":[{"n":"a","v":1},{"n":"b\\"q","v":-2},{"n":"c","v":null}],"tail":"t"}')
    const limits = Limits.default()
    const label = (l: string) => `(record (entry :label "${l}"))`
    const table = (step: string, finish: string) =>
      `def export [input] (csv csv-options (scan-emit no-schema ${step} ${finish} (select (path "items" each-index) input)))`
    // One schema on the first item, a row per item: the sound shape.
    const first = (schema: string, row: string) =>
      `(fn [s x] (transition (ready []) (if (is-ready s) [(row ${row})] [(schema ${schema}) (row ${row})])))`
    const n = label('N')
    const v = label('V')
    const cases: Array<[string, string, { ok: string } | { code: Code }]> = [
      [
        'sound',
        table(first(`[${n} ${v}]`, '[(get :n x) (get :v x)]'), '(fn [s] [table-end])'),
        { ok: '"N","V"\r\n"a","1"\r\n"b""q","-2"\r\n"c",""\r\n' },
      ],
      ['rows without a schema', table('(fn [s x] (transition s [(row [(get :n x)])]))', '(fn [s] [table-end])'), { code: 'PROTOCOL_ORDER_ERROR' }],
      ['a schema per item', table(`(fn [s x] (transition s [(schema [${n}])]))`, '(fn [s] [table-end])'), { code: 'PROTOCOL_ORDER_ERROR' }],
      ['a row wider than the schema', table(first(`[${n}]`, '[(get :n x) (get :v x)]'), '(fn [s] [table-end])'), { code: 'PROTOCOL_ORDER_ERROR' }],
      ['no table-end', table(first(`[${n}]`, '[(get :n x)]'), '(fn [s] [])'), { code: 'PROTOCOL_ORDER_ERROR' }],
      ['two table-ends', table(first(`[${n}]`, '[(get :n x)]'), '(fn [s] [table-end table-end])'), { code: 'PROTOCOL_ORDER_ERROR' }],
      [
        'an item that is not a table event',
        'def export [input] (csv csv-options (map (fn [x] (get :n x)) (select (path "items" each-index) input)))',
        { code: 'PROTOCOL_ORDER_ERROR' },
      ],
      [
        'no items at all',
        'def export [input] (csv csv-options (scan-emit no-schema (fn [s x] (transition s [x])) (fn [s] []) (select (path "none" each-index) input)))',
        { code: 'PROTOCOL_ORDER_ERROR' },
      ],
      ['a schema of no columns', table(first('[]', '[]'), '(fn [s] [table-end])'), { code: 'TARGET_VALUE_UNREPRESENTABLE' }],
      ['a column without a label', table(first('[(record (entry :x "N"))]', '[(get :n x)]'), '(fn [s] [table-end])'), { code: 'MISSING_VALUE' }],
      ['a label that is a record', table(first('[(record (entry :label (record)))]', '[(get :n x)]'), '(fn [s] [table-end])'), { code: 'INPUT_INVALID' }],
    ]
    for (const [name, src, expected] of cases) {
      agree(name, compile(src, 'table.alc'), items, limits, expected)
    }

    // The worked example over a document whose metadata is empty: a table
    // of no columns has no CSV form, whichever path renders it.
    const program = compile(PROGRAM, 'export.alc')
    agree(
      'no columns',
      program,
      json('{"response":{"metadata":{"fields":[]},"payload":{"deep":{"records":[{"id":1}]}}}}'),
      limits,
      { code: 'TARGET_VALUE_UNREPRESENTABLE' },
    )

    // A delimiter no CSV reader could take is refused before anything
    // runs, natively (the renderer) and interpreted (`csv-table`).
    const records = json(RECORDS)
    for (const delimiter of ['\\"', '\\n', '\\r']) {
      const src = PROGRAM.replace(
        '    csv csv-options\n',
        `    csv (record (entry :delimiter "${delimiter}") (entry :newline "\\r\\n") (entry :header true) (entry :null-text "") (entry :missing :error))\n`,
      )
      agree(`delimiter ${delimiter}`, compile(src, 'delimiter.alc'), records, limits, {
        code: 'TARGET_VALUE_UNREPRESENTABLE',
      })
    }

    // A column function that answers something other than a record fails
    // as `get` does, both ways.
    const keyword = compile(
      'def b (record (entry :columns (path "m")) (entry :rows (path "r" each-index)) (entry :column (fn [d] :oops)))\ndef export [input] (csv csv-options (table-from-json b input))',
      'keyword.alc',
    )
    agree('a column that is a keyword', keyword, json('{"m":[{"title":"t","path":["a"]}],"r":[{"a":"x"}]}'), limits, {
      code: 'DSL_TYPE_ERROR',
    })
  })

  // The library's table holds the scopes it captures to the limits the
  // native table does (the metadata under `max_metadata_bytes`, each row
  // under `max_record_bytes`, at most `max_columns` columns), so under the
  // host's own limits both ways fail alike or print alike, and count the
  // same rows.
  it('the limits hold alike both ways', () => {
    const program = compile(PROGRAM, 'export.alc')
    const records = json(RECORDS)
    const cases: Array<[string, Limits, { ok: string } | { code: Code }]> = [
      ['defaults', Limits.default(), { ok: EXPECTED_CSV }],
      ['max_record_bytes', Limits.with({ max_record_bytes: 100 }), { code: 'RESOURCE_LIMIT_EXCEEDED' }],
      ['max_metadata_bytes', Limits.with({ max_metadata_bytes: 50 }), { code: 'RESOURCE_LIMIT_EXCEEDED' }],
      ['max_columns', Limits.with({ max_columns: 2 }), { code: 'RESOURCE_LIMIT_EXCEEDED' }],
      ['max_capture_bytes decides neither table', Limits.with({ max_capture_bytes: 100 }), { ok: EXPECTED_CSV }],
    ]
    for (const [name, limits, expected] of cases) agree(name, program, records, limits, expected)
    // A cell that is a vector is its compact JSON text, one scalar of the
    // output, held to max_scalar_bytes both ways.
    const containers = json(RECORDS.replace('"id":123', '"id":[1,2,3,4,5,6]'))
    agree('a container cell under max_scalar_bytes', program, containers, Limits.with({ max_scalar_bytes: 12 }), {
      code: 'RESOURCE_LIMIT_EXCEEDED',
    })
    agree('a container cell within it', program, containers, Limits.with({ max_scalar_bytes: 13 }), {
      ok: '"Identifier","Full name","Balance"\r\n"[1,2,3,4,5,6]","Alice","50.25"\r\n"456","Bob","72"\r\n',
    })
    const metrics = new Metrics()
    replayed(program.withNative(false), records, undefined, Limits.default(), metrics)
    assert.equal(metrics.rows, 2)
  })

  // A program over an inferred binding (`:columns :infer`): a root array of
  // rows, written as CSV with a missing cell as an empty field. The columns
  // are the first row's keys, in its order, each reading its own key.
  // Natively it is transduce's `Schema.infer()`; the library's text infers
  // them with `keys`. Both ways write the same bytes, fail with the same
  // code, and hold the same limits, over every shape a first row and a
  // later row can take.
  it('an inferred binding agrees both ways', () => {
    const INFERRED =
      'def options\n  record\n    entry :delimiter ","\n    entry :newline "\\r\\n"\n    entry :header true\n    entry :null-text ""\n    entry :missing ""\n\n' +
      'def rows-binding\n  record\n    entry :columns :infer\n    entry :rows (path each-index)\n\n' +
      'def export [input]\n  pipe input\n    table-from-json rows-binding\n    csv options\n'
    const program = compile(INFERRED, 'inferred.alc')
    const ok = (name: string, text: string, want: string) => agree(name, program, json(text), Limits.default(), { ok: want })
    const err = (name: string, text: string, code: Code) => agree(name, program, json(text), Limits.default(), { code })
    ok('the first row\'s keys, in its order', '[{"b":1,"a":"x"},{"a":"y","b":2}]', '"b","a"\r\n"1","x"\r\n"2","y"\r\n')
    ok('a key a later row lacks is a missing cell', '[{"a":1,"b":2},{"a":3}]', '"a","b"\r\n"1","2"\r\n"3",""\r\n')
    ok('a key only a later row has is no column', '[{"a":1},{"c":3,"a":2}]', '"a"\r\n"1"\r\n"2"\r\n')
    ok(
      'cells of every kind',
      '[{"s":"q\\"x","n":1.5,"t":true,"z":null,"o":{"k":[1,2]}}]',
      '"s","n","t","z","o"\r\n"q""x","1.5","true","","{""k"":[1,2]}"\r\n',
    )
    ok('a key that spells an index names a member', '[{"0":"zero","1":"one"}]', '"0","1"\r\n"zero","one"\r\n')
    ok('a later row that is not an object has missing cells', '[{"a":1},2]', '"a"\r\n"1"\r\n""\r\n')
    err('a first row that is a scalar', '[1,{"a":1}]', 'INPUT_INVALID')
    err('a first row that is an array', '[[1],{"a":1}]', 'INPUT_INVALID')
    // A table of no columns, from no rows or from an empty first row, is
    // one the CSV renderer refuses, as it refuses any.
    err('no rows', '[]', 'TARGET_VALUE_UNREPRESENTABLE')
    err('an empty first row', '[{},{"a":1}]', 'TARGET_VALUE_UNREPRESENTABLE')
    const three = json('[{"a":1,"b":2,"c":3}]')
    agree('max_columns holds the inferred columns', program, three, Limits.with({ max_columns: 2 }), {
      code: 'RESOURCE_LIMIT_EXCEEDED',
    })
    // Three one-byte names take 16 + 3 * (16 + 1) = 67 bytes natively; the
    // library's state holding them is larger still.
    agree('max_metadata_bytes holds the inferred columns', program, three, Limits.with({ max_metadata_bytes: 40 }), {
      code: 'RESOURCE_LIMIT_EXCEEDED',
    })
    agree('max_record_bytes holds each row', program, three, Limits.with({ max_record_bytes: 8 }), {
      code: 'RESOURCE_LIMIT_EXCEEDED',
    })
    // The column count holds where the schema is built, so it holds when
    // the table events reach no renderer that would check them.
    for (const [name, binding] of [
      ['an inferred table\'s events as items', '(record (entry :columns :infer) (entry :rows (path each-index)))'],
      [
        'a described table\'s events as items',
        '(record (entry :columns (path "m")) (entry :rows (path "r" each-index)) (entry :column (fn [d] (record (entry :label d) (entry :source (path d))))))',
      ],
    ]) {
      const items = compile(`def b ${binding}\ndef export [input] (join "" (map (fn [e] "x") (table-from-json b input)))`, 'items.alc')
      const evs = name.startsWith('an inferred') ? three : json('{"m":["a","b","c"],"r":[{"a":1,"b":2,"c":3}]}')
      agree(name, items, evs, Limits.with({ max_columns: 2 }), { code: 'RESOURCE_LIMIT_EXCEEDED' })
      agree(name, items, evs, Limits.default(), { ok: 'xxx' })
    }
    // The worked example's documents, bound by inference from their rows.
    const api = compile(
      INFERRED.replace('(path each-index)', '(path "response" "payload" "deep" "records" each-index)'),
      'inferred-api.alc',
    )
    const failures: string[] = []
    for (const n of [0, 1, 2, 3, 50]) {
      const report = differential(`recordsJson(${n})`, api, json(recordsJson(n)))
      if (undefined !== report) failures.push(report)
    }
    assert.deepStrictEqual(failures, [])
  })

  // Each row counts once in `metrics.rows`, by the last table stage it
  // passes: the adapter to a renderer, a `csv-table` whose rows reach no
  // later table stage, or the native table when it hands its rows straight
  // to a renderer. A `map`, a `filter` or a `scan-emit` between two table
  // stages hands the count on, so a row a filter drops is never counted,
  // and rows that only ever become a text of items are not rows of any
  // table; natively and interpreted alike, with the same bytes.
  it('each row counts once across composed table stages', () => {
    const tail = (t: string) => PROGRAM.replace('    csv csv-options\n', t)
    const identity = '    map (fn [e] e)\n'
    const dropRows = '    filter (fn [e] (match e (case (row cells) false) (case _ true)))\n'
    const passScan = '    scan-emit null (fn [s e] (transition s [e])) (fn [s] []) \n'
    const asText = '    map (fn [e] "x")\n    join ","\n'
    const cases: Array<[string, string, 'csv' | 'json' | undefined, number]> = [
      ['csv', tail('    csv csv-options\n'), undefined, 2],
      ['csv-table as the result', tail('    csv-table csv-options\n'), undefined, 2],
      ['csv-table as the result, as json', tail('    csv-table csv-options\n'), 'json', 2],
      ['csv over csv-table', tail('    csv-table csv-options\n    csv csv-options\n'), undefined, 2],
      ['records over csv-table', tail('    csv-table csv-options\n    records\n    json\n'), undefined, 2],
      ['csv-table twice', tail('    csv-table csv-options\n    csv-table csv-options\n    csv csv-options\n'), undefined, 2],
      [
        'the library csv over the native table',
        tail(
          '    csv opts\n\ndef opts\n  record\n    entry :delimiter "||"\n    entry :newline "\\r\\n"\n    entry :header true\n    entry :null-text ""\n    entry :missing :error\n',
        ),
        undefined,
        2,
      ],
      ['a map, then the program\'s csv', tail(`${identity}    csv csv-options\n`), undefined, 2],
      ['a map, csv-table, csv', tail(`${identity}    csv-table csv-options\n    csv csv-options\n`), undefined, 2],
      ['csv-table, a map, csv', tail(`    csv-table csv-options\n${identity}    csv csv-options\n`), undefined, 2],
      ['a map, the host\'s renderer', tail(identity), 'csv', 2],
      ['a map, the host\'s json', tail(identity), 'json', 2],
      ['a filter that keeps every row, the host\'s renderer', tail('    filter (fn [e] true)\n'), 'csv', 2],
      ['a scan that passes each event on, then csv', tail(`${passScan}    csv csv-options\n`), undefined, 2],
      ['a filter that drops every row, then csv', tail(`${dropRows}    csv csv-options\n`), undefined, 0],
      ['a filter that drops every row, the host\'s renderer', tail(dropRows), 'csv', 0],
      ['the table\'s events as a text: no table stage', tail(asText), undefined, 0],
      ['csv-table\'s events as a text', tail(`    csv-table csv-options\n${asText}`), undefined, 2],
    ]
    const records = json(RECORDS)
    for (const [name, src, render, rows] of cases) {
      const program = compile(src, 'rows.alc')
      const outputs: string[] = []
      for (const native of [true, false]) {
        const metrics = new Metrics()
        let out: string
        try {
          out = replayed(program.withNative(native), records, render, Limits.default(), metrics)
        } catch (fail) {
          assert.fail(`${name} (native: ${native}): ${fail}`)
        }
        assert.equal(metrics.rows, rows, `${name} (native: ${native})`)
        outputs.push(out)
      }
      assert.equal(outputs[0], outputs[1], `${name}: the bytes differ`)
    }
  })

  // The two paths differ, knowingly, in one place the standard shapes
  // never reach; pinned so a change to either is seen.
  it('the known differences are pinned', () => {
    // Metadata selected twice: the native transducer calls it an order
    // violation (a table has one schema); the library text says `fail`,
    // which is INPUT_INVALID.
    const twice = compile(
      PROGRAM.replace('      path "response" "metadata" "fields"\n', '      path "response" "metadata" each-index\n'),
      'twice.alc',
    )
    const doc = `{"response":{"metadata":[${'[{"title":"a","path":["id"]}]'},${'[{"title":"b","path":["id"]}]'}],"payload":{"deep":{"records":[${record(1)}]}}}}`
    const evs = json(doc)
    assert.equal(attemptFail(() => replayed(twice, evs)).code, 'INPUT_ORDER_VIOLATION')
    assert.equal(attemptFail(() => replayed(twice.withNative(false), evs)).code, 'INPUT_INVALID')
    // A numeric title was a second difference; one label policy now serves
    // every table (a string as it is, a number by its lexeme, a boolean by
    // its name), so both ways render the lexeme.
    const program = compile(PROGRAM, 'export.alc')
    const numeric = json(
      `{"response":{"metadata":{"fields":[{"title":42,"path":["id"]}]},"payload":{"deep":{"records":[${record(1)}]}}}}`,
    )
    agree('a numeric title', program, numeric, Limits.default(), { ok: '"42"\r\n"1"\r\n' })
  })

  // The events end with `end`, as every source promises; a recording that
  // does not is a source defect, and the sink says so rather than dropping
  // the output silently: nothing is flushed.
  it('a stream without end writes nothing', () => {
    const program = compile(PROGRAM, 'export.alc')
    const evs = json(recordsJson(2))
    assert.deepStrictEqual(evs.pop(), Ev.end)
    for (const native of [true, false]) {
      const writer = new BytesWriter()
      const sink = program.withNative(native).sink(writer, undefined, Limits.default(), new Metrics())
      replay(evs, sink)
      assert.equal(writer.text(), '', `native=${native}`)
      // And the end then completes it.
      sink.event(Ev.end)
      assert.notEqual(writer.text(), '', `native=${native}`)
    }
  })
})

function attemptFail(work: () => unknown): any {
  try {
    work()
  } catch (fail) {
    return fail
  }
  throw new Error('expected a failure')
}
