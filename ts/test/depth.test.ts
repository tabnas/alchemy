/* Copyright (c) 2026 tabnas, MIT License */

// The recursive walks are safe at the nesting the reader allows, in
// Node's default stack (AGENTS.md, "Untrusted input"): MAX_NESTING (256)
// levels bound what the printers, the desugarer, the resolver and the
// checker recurse over, and the checker follows a stream into at most
// MAX_APPLIED bodies, each of them that deep. These run the worst
// programs those bounds admit through every stage of the front end.
//
// Evaluation nests at most MAX_EVAL_DEPTH (1,000) levels, `recursion`
// past it, which a function applied to itself reaches. The Rust crate
// reaches the bound before the stack's end only on a thread of 64 MiB;
// here the evaluator runs on an explicit stack (`./trampoline`), so the
// bound is reached in Node's default stack, as a `Fail`, never as a
// `RangeError`, through every path that nests evaluation: a body inside
// the call that applied it, a native that applies a function, a finite
// text written as it applies its function, a chain of definitions; at
// compile time and per item. The boundaries pinned below are the Rust
// binary's, compared program for program: depth is counted as it counts
// it. The tests here stop at compile time; the ones that run a program
// (the bound per item, the deepest chains run, the bound with half the
// stack used) are alchemy-cli's, in its ts/test/depth.test.ts.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { MAX_APPLIED, MAX_EVAL_DEPTH, MAX_NESTING, analyze, canonical, desugarProgram, format, parse } from '../dist/alchemy'

import { compile, finer, thrown } from './common'

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

  // A function applied to itself, at compile time: evaluation nests a
  // level per form and per body applied until the bound, which is the
  // failure, positioned at the form that passed it.
  it('a function applied to itself is recursion, in the default stack', () => {
    assert.equal(MAX_EVAL_DEPTH, 1000)
    const omega = 'def w [f] (f f)\ndef export [input]\n  let [x (w w)]\n    json input\n'
    const fail = thrown(() => compile(omega, 'omega.alc'))
    assert.equal(fail.code, 'STREAMABILITY_UNKNOWN', String(fail))
    assert.equal(finer(fail), 'recursion')
    assert.deepStrictEqual([fail.row, fail.col], [1, 12])
    // Through a native that applies the function: `map` over a vector is
    // a walk on the same stack.
    const viaMap = 'def w [f] (map (fn [x] (f f)) [1])\ndef export [input]\n  let [x (w w)]\n    json input\n'
    assert.equal(finer(thrown(() => compile(viaMap, 'map.alc'))), 'recursion')
    // Through a pattern match and a partial.
    const viaMatch = 'def w [f] (match 1 (case 1 ((partial f f))))\ndef export [input]\n  let [x (w w)]\n    json input\n'
    assert.equal(finer(thrown(() => compile(viaMatch, 'match.alc'))), 'recursion')
  })
})
