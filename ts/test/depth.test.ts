/* Copyright (c) 2026 tabnas, MIT License */

// The recursive walks are safe at the nesting the reader allows, in
// Node's default stack (AGENTS.md, "Untrusted input"): MAX_NESTING (256)
// levels bound what the printers, the desugarer, the resolver and the
// checker recurse over, and the checker follows a stream into at most
// MAX_APPLIED bodies, each of them that deep. These run the worst
// programs those bounds admit through every stage of the front end.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { MAX_APPLIED, MAX_NESTING, analyze, canonical, desugarProgram, format, parse } from '../dist/alchemy'

import { finer, thrown } from './common'

// `levels` nested calls of `head` around `inner`.
function nest(levels: number, head: string, inner: string): string {
  return `(${head} `.repeat(levels) + inner + ')'.repeat(levels)
}

describe('depth', () => {
  it('a program at the reader\'s bound prints, desugars and checks', () => {
    // A layout line, `def`'s own list, the `fn` the desugarer adds, and
    // the body's levels: as deep as the bound allows.
    const levels = MAX_NESTING - 3
    const src = `def export [input] ${nest(levels, 'concat', '(json input)')}`
    const program = parse(src)
    assert.equal(canonical(parse(format(program))), canonical(program))
    const core = desugarProgram(program, src)
    assert.equal(canonical(core).split('(concat').length - 1, levels)
    assert.equal(analyze(src, 'deep').checked.output, 'Text')
    // A stream used twice at the bottom is found there.
    const twice = `def export [input] ${nest(levels - 1, 'concat', '(json input) (json input)')}`
    assert.equal(finer(thrown(() => analyze(twice, 'deep'))), 'reused')
    // Vectors and lets nest the same walks.
    const vectors = `def v ${'['.repeat(MAX_NESTING - 2)}1${']'.repeat(MAX_NESTING - 2)}\ndef export [input] (json input)`
    assert.equal(analyze(vectors, 'deep').checked.output, 'Text')
    const lets = `def export [input] ${nest(80, 'let [x 1]', '(json input)').split('(let [x 1] ').join('(let [x (count [1])] ')}`
    assert.equal(analyze(lets, 'deep').checked.output, 'Text')
  })

  it('the deepest layout program reads and prints', () => {
    const lines: string[] = []
    for (let level = 0; level < MAX_NESTING; level++) lines.push('  '.repeat(level) + `f${level}`)
    const program = parse(lines.join('\n'))
    assert.equal(canonical(parse(format(program))), canonical(program))
  })

  // MAX_APPLIED definitions, each passing the stream on from the bottom of
  // a body as deep as a definition's can be: the checker types each body
  // again inside the call that reached it.
  it('a stream followed through MAX_APPLIED deep bodies', () => {
    const levels = MAX_NESTING - 4
    let src = 'def f0 [s] (json s)\n'
    for (let i = 1; i <= MAX_APPLIED + 2; i++) {
      src += `def f${i} [s] ${nest(levels, 'concat', `(f${i - 1} s)`)}\n`
    }
    src += `def export [input] (f${MAX_APPLIED + 2} input)\n`
    assert.equal(analyze(src, 'applied').checked.output, 'Text')
  })
})
