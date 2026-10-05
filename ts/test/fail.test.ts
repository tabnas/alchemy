/* Copyright (c) 2026 tabnas, MIT License */

// `isFail` (src/fail.ts): a failure is told from a defect by what every
// copy of the shared unit gives its failures, so a `Fail` from a second
// copy, as render or transduce makes them when an install gives one of
// them its own alchemy, is a failure here too, and nothing else is.

import { describe, it } from 'node:test'
import assert from 'node:assert'
import { readFileSync, readdirSync } from 'node:fs'
import { dirname, join, sep } from 'node:path'

import { make as makeJson } from '@tabnas/json'

import { failFrom } from '../dist/alchemy'
import { isFail } from '../dist/fail'
import { Fail, isFail as instanceOfFail } from '../dist/shared'

import { REPO_ROOT, failCode, thrown } from './common'

// A second copy of the shared unit: its files loaded again as modules of
// their own, with a `Fail` class of their own, as the shared unit of an
// alchemy nested under render or transduce is. The module cache is put
// back as it was, so nothing else loads it.
function secondShared(): typeof import('../dist/shared') {
  const main = require.resolve('../dist/shared')
  const dir = dirname(main) + sep
  const loaded = () => Object.keys(require.cache).filter((file) => file.startsWith(dir))
  const saved = loaded().map((file) => [file, require.cache[file]] as const)
  for (const [file] of saved) delete require.cache[file]
  try {
    return require(main)
  } finally {
    for (const file of loaded()) delete require.cache[file]
    for (const [file, mod] of saved) require.cache[file] = mod
  }
}

describe('fail', () => {
  it("a Fail is a failure, from this package's shared unit or another copy", () => {
    assert.ok(isFail(new Fail('INPUT_INVALID', 'not JSON')))
    assert.ok(isFail(Fail.limit('max_output_bytes', 20, 'the output would exceed 20 bytes')))
    const other = secondShared()
    assert.notStrictEqual(other.Fail, Fail, 'the copy has a Fail class of its own')
    for (const fail of [
      new other.Fail('MISSING_VALUE', 'row 1 has no value for column "b"'),
      other.Fail.limit('max_output_bytes', 20, 'the output would exceed 20 bytes'),
      other.Fail.protocol('a row before the schema'),
    ]) {
      // What `instanceof`, and the shared unit's own `isFail`, do not see.
      assert.ok(!(fail instanceof Fail), String(fail))
      assert.ok(!instanceOfFail(fail), String(fail))
      assert.ok(isFail(fail), String(fail))
    }
  })

  it('anything else is not a failure', () => {
    // The engine's own error carries a string code too, under its own name.
    const engine = thrown(() => makeJson().parse('{'))
    assert.equal(typeof engine.code, 'string')
    const coded = Object.assign(new Error('no such file'), { code: 'ENOENT' })
    const named = new Error('a failure by name alone')
    named.name = 'Fail'
    const plain = { name: 'Fail', code: 'INPUT_INVALID', message: 'not an Error' }
    for (const err of [
      new Error('a defect'),
      new TypeError('a defect'),
      engine,
      coded,
      named,
      plain,
      'Fail',
      null,
      undefined,
    ]) {
      assert.ok(!isFail(err), String(err))
    }
  })

  // The reader hands a failure back as it is, and the fixtures' runner
  // reads its code, whichever copy made it.
  it("another copy's failure keeps its code", () => {
    const fail = new (secondShared().Fail)('MISSING_VALUE', 'row 1 has no value for column "b"')
    assert.strictEqual(failFrom(fail), fail)
    assert.equal(failCode(fail), 'MISSING_VALUE')
  })

  // `instanceof Fail` passes while there is one copy of the shared unit and
  // misses the second, which only an install with two shows, so no source
  // but src/fail.ts asks it. The shared unit itself is the exception: its
  // own `isFail`, which transduce re-exports, asks `instanceof` by design.
  it('no source tells a failure by instanceof', () => {
    const src = join(REPO_ROOT, 'ts', 'src')
    const asking = readdirSync(src, { recursive: true, encoding: 'utf8' })
      .filter((file) => file.endsWith('.ts') && 'fail.ts' !== file && !file.startsWith(`shared${sep}`))
      .filter((file) => /\binstanceof\s+Fail\b/.test(readFileSync(join(src, file), 'utf8')))
    assert.deepStrictEqual(asking, [])
  })
})
