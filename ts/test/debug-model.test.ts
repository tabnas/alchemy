/* Copyright (c) 2026 tabnas, MIT License */

// Composition test: the alchemy grammar plugin layered with the
// @tabnas/debug introspection plugin, as every grammar in the fleet
// carries (rs/tests/debug_model_test.rs). @tabnas/debug is a
// devDependency, so a missing one fails here rather than reporting green
// having run nothing.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { Debug } from '@tabnas/debug'

import { make } from '../dist/alchemy'

// An alchemy instance with the debug plugin layered on top, quiet.
function build(): any {
  const tn: any = make()
  tn.use(Debug, { print: false, trace: false })
  return tn
}

describe('debug model', () => {
  it('parses normally with the debug plugin installed', () => {
    const value = build().parse('def x [a]\n  f a 1')
    assert.equal(value[0].$, 'list')
    assert.equal(value[0].items[0].name, 'def')
    assert.equal(value[0].items[3].$, 'list')
  })

  it('the model is the structured alchemy grammar', () => {
    const m = build().debug.model()
    assert.deepStrictEqual(m.rules.map((r: any) => r.name).sort(), ['block', 'bracket', 'form', 'line', 'paren', 'program'])
    assert.equal(m.config.start, 'program')
    // The layout matcher is the one custom lexer entry, below the first
    // built-in band.
    const layout = m.lexer.find((x: any) => 'alchemy' === x.matcher)
    assert.ok(layout, 'the layout matcher is listed')
    assert.ok(layout.order < 1e6, `order ${layout.order} is below the bands`)
    // The rule graph: every repetition is a replace loop; the one push left
    // in a close phase is a line's block.
    const edge = (name: string) => {
      const e = m.graph.find((x: any) => x.name === name)
      assert.ok(e, `an edge entry for ${name}`)
      return e
    }
    assert.deepStrictEqual(edge('program').openPush, ['line'])
    assert.deepStrictEqual(edge('program').closePush, [])
    assert.deepStrictEqual(edge('line').openPush, ['form'])
    assert.deepStrictEqual(edge('line').closePush, ['block'])
    assert.deepStrictEqual(edge('line').closeReplace, ['line'])
    assert.deepStrictEqual(edge('block').openPush, ['line'])
    assert.deepStrictEqual(edge('block').closePush, [])
    assert.deepStrictEqual([...edge('form').openPush].sort(), ['bracket', 'paren'])
    assert.deepStrictEqual(edge('form').closeReplace, ['form'])
    for (const sequence of ['paren', 'bracket']) {
      assert.deepStrictEqual(edge(sequence).openPush, ['form'], sequence)
      assert.deepStrictEqual(edge(sequence).closePush, [], sequence)
    }
    for (const e of m.graph) assert.deepStrictEqual(e.openReplace, [], e.name)
    // The structural tokens the matcher emits are registered.
    const tokens = m.tokens.map((t: any) => t.name)
    for (const name of ['#IN', '#DE', '#NL', '#KW', '#OP', '#CP']) assert.ok(tokens.includes(name), name)
  })

  // The grammar portion of the model, serialised as a tool would receive
  // it and read back, still describes this grammar.
  it('the grammar portion serialises and reads back with its content', () => {
    const m = build().debug.model()
    const back = JSON.parse(
      JSON.stringify({ tokens: m.tokens, rules: m.rules, graph: m.graph, config: m.config, abnf: m.abnf }),
    )
    assert.deepStrictEqual(back.rules.map((r: any) => r.name).sort(), ['block', 'bracket', 'form', 'line', 'paren', 'program'])
    // A line opens on a form; it closes on `#IN` (pushing its block), on
    // `#NL` (replacing itself with the next line), on `#DE` or `#ZZ` left
    // for the rule above, or, on the condition that it took a block, on the
    // next line's first token (replacing itself again).
    const line = back.rules.find((r: any) => 'line' === r.name)
    const alts = (phase: string) =>
      line[phase].map((alt: any) => [alt.seq, alt.push ?? null, alt.replace ?? null, alt.back ?? null, alt.cond])
    assert.deepStrictEqual(alts('open'), [[[], 'form', null, null, false]])
    assert.deepStrictEqual(alts('close'), [
      [['#IN'], 'block', null, null, false],
      [['#NL'], null, 'line', null, false],
      [['#DE'], null, null, 1, false],
      [['#ZZ'], null, null, 1, false],
      [[], null, 'line', null, true],
    ])
    // The layout matcher's tokens are registered, the delimiters are the
    // only fixed tokens, and the engine lexes strings and comments but
    // leaves words to the matcher.
    const tokens = back.tokens.map((t: any) => [t.name, t.fixed ?? null])
    for (const name of ['#IN', '#DE', '#NL', '#KW']) {
      assert.ok(tokens.some(([n, f]: any) => n === name && null === f), name)
    }
    assert.deepStrictEqual(
      tokens.filter(([, f]: any) => null !== f),
      [
        ['#OS', '['],
        ['#CS', ']'],
        ['#OP', '('],
        ['#CP', ')'],
      ],
    )
    assert.equal(back.config.start, 'program')
    assert.deepStrictEqual(back.config.lex, {
      fixed: true,
      space: true,
      line: true,
      text: false,
      number: false,
      comment: true,
      string: true,
      value: false,
    })
    // The block is optional, `line = form [ IN block ]`; debug's ABNF
    // emitter renders it as mandatory until tabnas/debug#63 lands in its
    // TypeScript half, so either is accepted, as the Rust test does.
    assert.ok(
      back.abnf.includes('line = form [ IN block ]') || back.abnf.includes('line = form IN block'),
      back.abnf,
    )
  })
})
