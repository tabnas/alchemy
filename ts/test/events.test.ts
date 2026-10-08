/* Copyright (c) 2026 tabnas, MIT License */

// `events` (rs/tests/events_test.rs): the source's events as items a
// program reads one by one, and the operators a renderer over them needs
// (`push`, `pop`, `top`, `count`, `quoted`, `repeat`). The proof is a
// renderer of arbitrary nesting, which nothing before `events` could
// express: a tiny YAML-like block form written by a `scan-emit` whose state
// is the stack of open containers, one keyword marker each, asserted byte
// for byte; then the affine rule over `events`, and the plan report.
//
// Only the affine rule is checked without running a program, so it is the
// one here; the renderer, the explicit keys, YAML's non-finite numbers,
// the plan report (which runs its program too) and the events a program
// builds run on transduce's routers and render's renderers, and are
// alchemy-cli's, in its ts/test/events.test.ts.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { compile, thrown } from './common'

describe('events', () => {
  // `events` consumes the input, so a program that reads it twice, or
  // captures it in a function, is refused as any affine stream is.
  it('events over the input is affine', () => {
    const twice =
      'def export [input]\n  concat\n    join "" (map (fn [e] "a") (events input))\n    join "" (map (fn [e] "b") (events input))\n'
    let fail = thrown(() => compile(twice, 'twice.alc'))
    assert.equal(fail.code, 'STREAM_REUSED', String(fail))
    assert.ok(fail.message.startsWith('reused: '), String(fail))
    assert.deepStrictEqual([fail.row, fail.col], [4, 39])
    const withJson = 'def export [input] (concat (json input) (join "" (map (fn [e] "") (events input))))'
    fail = thrown(() => compile(withJson, 'json.alc'))
    assert.equal(fail.code, 'STREAM_REUSED', String(fail))
    assert.ok(fail.message.startsWith('reused: '), String(fail))
    const captured = 'def export [input]\n  concat-map (fn [x] (join "" (map (fn [e] "") (events input)))) [1]\n'
    fail = thrown(() => compile(captured, 'captured.alc'))
    assert.equal(fail.code, 'STREAM_REUSED', String(fail))
    assert.ok(fail.message.startsWith('captured: '), String(fail))
    // The stream `events` yields is a stream of items: events, which `json`
    // takes back, but not table events for `csv` and not the output; a
    // stream of values is not events.
    for (const [src, finer] of [
      ['def export [input] (json (select (path each-index) input))', 'protocol_mismatch'],
      ['def export [input] (csv csv-options (events input))', 'protocol_mismatch'],
      ['def export [input] (events (select (path each-index) input))', 'protocol_mismatch'],
      ['def export [input] (events input)', 'bad_output'],
    ]) {
      fail = thrown(() => compile(src, 'bad.alc'))
      assert.equal(fail.code, 'DSL_TYPE_ERROR', `${src}: ${fail}`)
      assert.ok(fail.message.startsWith(`${finer}: `), `${src}: ${fail}`)
    }
  })
})
