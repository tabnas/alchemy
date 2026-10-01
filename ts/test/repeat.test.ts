/* Copyright (c) 2026 tabnas, MIT License */

// The reader repeats by replacement, never by a push chain
// (rs/tests/repeat_test.rs): every repetition in the grammar is a replace
// loop (`r`), whose items all run in one frame, so rule depth follows a
// program's nesting and never its length. The engine's rule depth `d` is
// the observable, read here with a rule subscriber on `make()`: the
// maximum over ten thousand items of each repetition is what one item
// needs, while real nesting still grows it.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { make } from '../dist/alchemy'

// The maximum rule depth `d` any rule reaches while `src` parses.
function maxDepth(src: string): number {
  const tn = make()
  let max = 0
  tn.sub({
    rule: (rule: any) => {
      if (rule.d > max) max = rule.d
    },
  })
  tn.parse(src)
  return max
}

const ITEMS = 10_000

// Each repetition, as one item and as ten thousand, with the depth one
// item needs.
const REPETITIONS: Array<[string, string, string, number]> = [
  ['top-level lines', 'x', 'x\n'.repeat(ITEMS), 2],
  ['child lines', 'p\n  x', 'p\n' + '  x\n'.repeat(ITEMS), 4],
  ['inline forms', 'f', 'f' + ' x'.repeat(ITEMS), 2],
  ['items of ( )', '(x)', '(' + 'x '.repeat(ITEMS) + ')', 4],
  ['items of [ ]', '[x]', '[' + 'x '.repeat(ITEMS) + ']', 4],
]

describe('repeat', () => {
  it('every repetition stays at the depth of one item', () => {
    for (const [name, one, many, depth] of REPETITIONS) {
      assert.equal(maxDepth(one), depth, `${name}: one item`)
      assert.equal(maxDepth(many), depth, `${name}: ${ITEMS} items reach deeper than one`)
    }
  })

  // Real recursion still nests: each `(` is a form and the list it opens,
  // two frames deeper than the one around it.
  it('nesting still grows the depth', () => {
    const nested = (levels: number) => '('.repeat(levels) + ')'.repeat(levels)
    assert.equal(maxDepth(nested(1)), 3)
    assert.equal(maxDepth(nested(10)), 21)
    assert.equal(maxDepth(nested(100)), 201)
  })

  // The installed grammar says the same: the only close-phase push is a
  // line's block, and the two loops are the line and the form replacing
  // themselves.
  it('the repetitions are replace loops', () => {
    const tn = make()
    const pushes = new Set<string>()
    const replaces = new Set<string>()
    for (const [name, spec] of Object.entries(tn.rule() as Record<string, any>)) {
      for (const phase of ['open', 'close']) {
        for (const alt of spec.def[phase]) {
          if ('string' === typeof alt.p) pushes.add(`${name} ${phase} p:${alt.p}`)
          if ('string' === typeof alt.r) replaces.add(`${name} ${phase} r:${alt.r}`)
        }
      }
    }
    assert.deepStrictEqual([...pushes].sort(), [
      'block open p:line',
      'bracket open p:form',
      'form open p:bracket',
      'form open p:paren',
      'line close p:block',
      'line open p:form',
      'paren open p:form',
      'program open p:line',
    ])
    assert.deepStrictEqual([...replaces].sort(), ['form close r:form', 'line close r:line'])
  })

  // Ten times the lines take about ten times as long. Quadratic work would
  // take a hundred; the bound is generous so a busy machine cannot fail it.
  it('parse time grows linearly with the lines', () => {
    const tn = make()
    const fastest = (src: string) => {
      let best = Infinity
      for (let i = 0; i < 3; i++) {
        const start = process.hrtime.bigint()
        tn.parse(src)
        best = Math.min(best, Number(process.hrtime.bigint() - start))
      }
      return best
    }
    // Warm the engine first, so the small parse is not the one that
    // pays for compilation.
    fastest('f a 1\n'.repeat(1_000))
    const small = fastest('f a 1\n'.repeat(1_000))
    const large = fastest('f a 1\n'.repeat(10_000))
    const ratio = large / Math.max(small, 1)
    assert.ok(ratio < 30, `1,000 lines took ${small}ns and 10,000 took ${large}ns: ${ratio.toFixed(1)}x`)
  })
})
