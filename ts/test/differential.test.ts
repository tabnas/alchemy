/* Copyright (c) 2026 tabnas, MIT License */

// The differential test (rs/tests/stdlib_test.rs): the standard library's
// own text, interpreted, against the native compositions the runtime
// substitutes for it. Over every transduce fixture a grammar these tests
// can read, and the generated documents transduce's tests and benches
// share, the spec's worked-example program produces the same bytes both
// ways, or fails with the same code. The library text is the reference;
// the native path is the optimization, and this is what makes it one.
//
// The comparisons run programs on transduce's routers and render's
// renderers, which this package does not depend on, so they are
// alchemy-cli's (its ts/test/differential.test.ts). What needs no run
// stays here: the library loads.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { stdlib } from '../dist/alchemy'

describe('differential', () => {
  it('the library loads', () => {
    const lib = stdlib()
    assert.ok(undefined !== lib.get('table-from-json'))
    assert.ok(undefined !== lib.get('csv'))
  })
})
