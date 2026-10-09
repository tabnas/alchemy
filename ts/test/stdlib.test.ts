/* Copyright (c) 2026 tabnas, MIT License */

// The standard library and the natives (rs/src/stdlib/mod.rs's and
// rs/src/stdlib/registry.rs's tests, the parts that need no evaluator),
// and the library's text on the reference page (rs/tests/spec_test.rs).

import { describe, it } from 'node:test'
import assert from 'node:assert'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import {
  SOURCES,
  arityAccepts,
  defParams,
  fileOf,
  installCalls,
  nativeCall,
  native,
  natives,
  outer,
  parseFile,
  source,
  stdlib,
  unimplemented,
} from '../dist/alchemy'

import { REPO_ROOT, thrown } from './common'

describe('stdlib', () => {
  it('the library loads with the spec definitions', () => {
    const lib = stdlib()
    // By file: the embedded module lists the files by name.
    assert.deepStrictEqual(
      SOURCES.map(([file]) => file),
      ['stdlib/csv.alc', 'stdlib/root.alc', 'stdlib/table.alc'],
    )
    assert.deepStrictEqual(
      lib.files.map((r: any) => [...r.defs.keys()]),
      [
        ['csv-options', 'csv-field', 'csv-row', 'csv'],
        [
          'wrap-object-close',
          'wrap-object-step',
          'wrap-finish',
          'wrap-object',
          'wrap-array-close',
          'wrap-array-step',
          'wrap-array',
        ],
        [
          'public-column',
          'table-inferred-column',
          'table-row',
          'table-positional-column',
          'table-value-column',
          'table-inferred-columns',
          'table-first-row',
          'table-step',
          'table-finish',
          'table-finish-for',
          'table-captures',
          'table-from-json',
        ],
      ],
    )
    assert.equal(lib.names().length, 23)
    assert.deepStrictEqual(defParams(lib.get('table-from-json')!), ['binding', 'input'])
    assert.equal(defParams(lib.get('csv-options')!), undefined)
  })

  it('library names and natives are outer to a program', () => {
    assert.equal(outer('csv'), 'value')
    assert.equal(outer('map'), 'function')
    assert.equal(outer('table-end'), 'constant')
    assert.equal(outer('selected'), 'constructor')
    assert.equal(outer('nope'), undefined)
    assert.equal(source('stdlib/csv.alc'), SOURCES[0][1])
    assert.equal(source('nope'), undefined)
  })

  // A span names its library file by identity: a program that happens to
  // be named like a library file is not one.
  it('a span is the library\'s by identity, not by name', () => {
    const def = stdlib().get('csv-row')!
    assert.deepStrictEqual(fileOf(def.span), SOURCES[0])
    const [form] = parseFile('def x 1', 'stdlib/csv.alc')
    assert.equal(fileOf(form.span), undefined)
  })
})

describe('natives', () => {
  it('every native is found by name and names are unique', () => {
    const seen = new Set<string>()
    for (const n of natives()) {
      assert.ok(!seen.has(n.name), `${n.name} twice`)
      seen.add(n.name)
      assert.equal(native(n.name), n)
      assert.ok(n.signature.startsWith(n.name), `${n.name}: ${n.signature}`)
    }
    assert.equal(native('nope'), undefined)
    assert.equal(natives().length, 65)
  })

  // `signature` and `effect` are what the reference prints: every native
  // with a row of its own in docs/language.md's table reads there as it
  // does here (the row's code spans unquoted).
  it('a native reads as its reference row', () => {
    const doc = readFileSync(join(REPO_ROOT, 'docs', 'language.md'), 'utf8')
    const spans = (cell: string): string | undefined => {
      if (!cell.startsWith('`') || !cell.endsWith('`') || cell.length < 2) return undefined
      const inner = cell.substring(1, cell.length - 1)
      return inner.includes('`') ? undefined : inner
    }
    const rows = new Map<string, [string, string]>()
    for (const line of doc.split('\n')) {
      if (!line.startsWith('| ') || !line.endsWith(' |')) continue
      const cells = line.substring(2, line.length - 2).split(' | ')
      if (3 !== cells.length) continue
      const name = spans(cells[0])
      const signature = spans(cells[1])
      if (undefined !== name && undefined !== signature) rows.set(name, [signature, cells[2].split('`').join('')])
    }
    let compared = 0
    for (const n of natives()) {
      const row = rows.get(n.name)
      if (undefined === row) continue
      assert.equal(n.signature, row[0], `the signature of ${n.name}`)
      assert.equal(n.effect, row[1], `the effect of ${n.name}`)
      compared++
    }
    assert.equal(compared, 47)
  })

  it('constants take no arguments and constructors take their fields', () => {
    for (const n of natives()) {
      if ('constant' === n.kind) assert.deepStrictEqual(n.arity, { kind: 'exact', n: 0 }, n.name)
      if ('constructor' === n.kind) assert.ok(arityAccepts(n.arity, 2) || arityAccepts(n.arity, 1), n.name)
    }
  })

  // The seam the evaluator plugs into: implementations by name, for names
  // the table holds only.
  // Every native in the table has its implementation (rs/src/stdlib/
  // registry.rs keeps both in one table; here ./natives installs them by
  // name), and a name the table does not hold is refused.
  it('every native has its implementation, installed by name', () => {
    assert.deepStrictEqual(unimplemented(), [])
    for (const n of natives()) assert.equal(typeof nativeCall(n.name), 'function', n.name)
    const count = nativeCall('count')
    const impl = () => 0
    installCalls({ count: impl })
    assert.equal(nativeCall('count'), impl)
    installCalls({ count })
    assert.equal(nativeCall('count'), count)
    assert.match(String(thrown(() => installCalls({ nope: impl }))), /nope is not a native/)
  })

  // Every definition of the standard library appears on the reference
  // page as it is in stdlib/*.alc, word for word.
  it('the library definitions on the page are the library text', () => {
    const doc = readFileSync(join(REPO_ROOT, 'docs', 'language.md'), 'utf8')
    let shown = 0
    for (const [file, src] of SOURCES) {
      // A definition runs from its `def` line to the line before the next
      // blank line (comments stay out).
      const defs: string[][] = []
      for (const line of src.split('\n')) {
        if (line.startsWith('def ')) defs.push([line])
        else if ('' === line.trim() || line.startsWith(';')) {
          const last = defs[defs.length - 1]
          if (undefined !== last && 0 < last.length && '' !== last[last.length - 1]) defs.push([])
        } else if (0 < defs.length) defs[defs.length - 1].push(line)
      }
      for (const def of defs.filter((d) => 0 < d.length)) {
        const block = '```alchemy\n' + def.join('\n') + '\n```'
        assert.ok(doc.includes(block), `${file}: docs/language.md does not show ${JSON.stringify(def[0])} as the library has it`)
        shown++
      }
    }
    assert.equal(shown, stdlib().names().length)
  })
})
