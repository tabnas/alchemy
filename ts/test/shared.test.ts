/* Copyright (c) 2026 tabnas, MIT License */

// The shared types (src/shared): what transduce and render import from
// this package, `@tabnas/alchemy/shared`, loads that directory and nothing
// else; this package's own source imports neither transduce nor render,
// whose stages a host passes to `compile` as `{ routers, renderers }`.

import { describe, it } from 'node:test'
import assert from 'node:assert'
import { spawnSync } from 'node:child_process'
import { readFileSync, readdirSync } from 'node:fs'
import { join, sep } from 'node:path'

import * as alchemy from '../dist/alchemy'
import * as shared from '../dist/shared'

import { REPO_ROOT, thrown } from './common'

const TS = join(REPO_ROOT, 'ts')
const SRC = join(TS, 'src')

// The module specifiers a source file imports or re-exports.
function specifiers(file: string): string[] {
  const text = readFileSync(file, 'utf8')
  return [...text.matchAll(/^(?:import|export)\b[^'"]*?from\s+'([^']+)'/gm)].map((m) => m[1])
}

describe('shared', () => {
  it('the shared entry loads the shared unit and nothing else', () => {
    // A process of its own, so the module cache holds only what the entry
    // loads; the package's own name, so the `exports` entry is what
    // resolves it.
    const out = spawnSync(
      process.execPath,
      ['-e', "require('@tabnas/alchemy/shared'); console.log(JSON.stringify(Object.keys(require.cache)))"],
      { cwd: TS, encoding: 'utf8' },
    )
    assert.equal(out.status, 0, out.stderr)
    const loaded: string[] = JSON.parse(out.stdout)
    const unit = join(TS, 'dist', 'shared') + sep
    assert.ok(loaded.includes(join(unit, 'index.js')), JSON.stringify(loaded))
    assert.deepStrictEqual(
      loaded.filter((file) => !file.startsWith(unit)),
      [],
    )
  })

  it('the shared source imports nothing outside itself', () => {
    const dir = join(SRC, 'shared')
    for (const file of readdirSync(dir).filter((f) => f.endsWith('.ts'))) {
      for (const spec of specifiers(join(dir, file))) {
        assert.match(spec, /^\.\/[a-z-]+$/, `src/shared/${file} imports ${spec}`)
      }
    }
  })

  it("this package's source imports neither transduce nor render", () => {
    const importing = readdirSync(SRC, { recursive: true, encoding: 'utf8' })
      .filter((file) => file.endsWith('.ts'))
      .filter((file) =>
        specifiers(join(SRC, file)).some((spec) => '@tabnas/transduce' === spec || '@tabnas/render' === spec),
      )
    assert.deepStrictEqual(importing, [])
  })

  // Every shared name is the main entry's too, the same value, but the four
  // the main entry names after this package's own functions.
  it('the main entry re-exports the shared names', () => {
    const own: Record<string, unknown> = {
      jsonString: alchemy.jsonString,
      isJsonNumber: alchemy.isJsonNumber,
      numberText: alchemy.numberText,
      isFail: alchemy.isFail,
    }
    const main = alchemy as unknown as Record<string, unknown>
    for (const [name, value] of Object.entries(shared)) {
      if (name in own) {
        assert.notStrictEqual(main[name], value, name)
      } else {
        assert.strictEqual(main[name], value, name)
      }
    }
    assert.strictEqual(alchemy.Fail, shared.Fail)
  })

  it('compile takes the routers and renderers it builds a run from', () => {
    const compile = alchemy.compile as (src: string, file: string, options?: unknown) => alchemy.Program
    for (const options of [undefined, {}, { routers: {} }, { renderers: {} }]) {
      const err = thrown(() => compile('def export [input] (json input)', 'x.alc', options))
      assert.ok(err instanceof TypeError, String(err))
      assert.ok(/routers, renderers/.test(err.message), err.message)
    }
  })

  // The fixture encoding of a parser event round-trips, leaving an absent
  // field out, and a malformed one says what is wrong with it.
  it('the parser event encoding round-trips and leaves absent fields out', () => {
    const texts = [
      '["open",{"name":"map","d":1,"i":3,"alt":{"p":"pair","r":"","b":0,"g":["map","json"]},"tokens":[["#OB","{","{",1,1]],"forced":false,"n":{"pk":0}}]',
      '["close",{"name":"val","d":2,"i":4,"tokens":[],"forced":true,"n":{},"value":null}]',
      '["end"]',
    ]
    for (const text of texts) {
      assert.equal(JSON.stringify(shared.parserEventToJSON(shared.parserEventFromJSON(JSON.parse(text)))), text)
    }
    const open = shared.parserEventFromJSON(JSON.parse(texts[0]))
    assert.ok('open' === open.type && undefined === open.pass.value, 'an open pass has no value')
    for (const text of [
      '{"open":1}',
      '["start"]',
      '["open"]',
      '["close",{"name":"x","d":-1,"i":0,"tokens":[],"forced":false,"n":{}}]',
      '["close",{"name":"x","d":1,"i":0,"tokens":[["#ST"]],"forced":false,"n":{}}]',
    ]) {
      assert.throws(() => shared.parserEventFromJSON(JSON.parse(text)), Error, text)
    }
  })
})
