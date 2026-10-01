/* Copyright (c) 2026 tabnas, MIT License */

// The layout lex matcher (rs/src/lex.rs's tests): words, indentation as
// tokens, the delimiter-depth scan, the nesting bound, and the state the
// matcher keeps in the parse's context bag.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { MAX_NESTING, make, parseValue } from '../dist/alchemy'
import { DE, Decision, decide, depthAt, initialState, isJsonNumber, scans } from '../dist/lex'

// Drive `decide` the way the lexer would over a source with no strings or
// comments: this matcher's tokens by name and source, and `_` for a
// position it passed on (advanced by one character).
function words(src: string): Array<[string, string]> {
  const state = initialState()
  const out: Array<[string, string]> = []
  let si = 0
  while (si < src.length) {
    const d = decide(src, si, state)
    if ('pass' === d.kind) {
      out.push(['_', src[si]])
      si += 1
    } else if ('word' === d.kind || 'layout' === d.kind) {
      out.push([d.name, src.substring(si, si + d.len)])
      si += d.len
    } else {
      out.push(['BAD', d.code])
      break
    }
  }
  return out
}

function names(src: string): string {
  return words(src)
    .map(([name, text]) => {
      if ('_' === name) return text
      if ('#IN' === name || '#DE' === name || '#NL' === name) return ` ${name} `
      if ('BAD' === name) return ` BAD:${text} `
      return `${name}(${text})`
    })
    .join('')
}

// `levels + 1` lines, each two spaces deeper than the one before.
function staircase(levels: number): string {
  const lines: string[] = []
  for (let level = 0; level <= levels; level++) lines.push('  '.repeat(level) + 'x')
  return lines.join('\n')
}

describe('lex', () => {
  it('json numbers are recognised exactly', () => {
    for (const ok of ['0', '-0', '12', '1.5', '-1.5e10', '2E-3', '0.0']) assert.ok(isJsonNumber(ok), ok)
    for (const bad of ['01', '1.', '.5', '+1', '1e', '-', '1_000', '0x1f', '1a']) assert.ok(!isJsonNumber(bad), bad)
  })

  it('words split into numbers, values and symbols', () => {
    assert.equal(
      names('def a-b? -1 1.5e3 true null :key x'),
      '#TX(def) #TX(a-b?) #NR(-1) #NR(1.5e3) #VL(true) #VL(null) #KW(:key) #TX(x)',
    )
  })

  it('indentation becomes structural tokens', () => {
    assert.equal(names('a\n  b\n    c\n  d\ne'), '#TX(a) #IN #TX(b) #IN #TX(c) #DE #TX(d) #DE #TX(e)')
  })

  it('a dedent of two levels is issued one token per call', () => {
    const state = initialState()
    const src = 'a\n  b\n    c\nd'
    const at = src.indexOf('c\n') + 1
    for (let si = 0; si < at; si++) decide(src, si, state)
    assert.equal(state.levels, 2)
    assert.deepStrictEqual(decide(src, at, state), { kind: 'layout', name: DE, len: 1, pending: 1 } as Decision)
    assert.deepStrictEqual(decide(src, at + 1, state), { kind: 'layout', name: DE, len: 0, pending: 0 } as Decision)
    assert.equal(state.levels, 0)
    const next = decide(src, at + 1, state)
    assert.ok('word' === next.kind && '#TX' === next.name)
  })

  it('blank and comment lines do not count', () => {
    assert.equal(names('a\n\n  ; note\n   \n  b\n\n'), '#TX(a) #IN #TX(b)\n\n')
  })

  it('a line ends at a line feed and a lone carriage return is whitespace', () => {
    assert.equal(names('a\r\n  b\r\nc'), '#TX(a) #IN #TX(b) #DE #TX(c)')
    assert.equal(names('a\r  b\r c'), '#TX(a)\r  #TX(b)\r #TX(c)')
    assert.equal(names('a \r b'), '#TX(a) \r #TX(b)')
    assert.equal(names('a\n\r  b'), '#TX(a) #IN \r  #TX(b)')
    assert.equal(names('a\n  \r  b'), '#TX(a) #IN \r  #TX(b)')
    assert.equal(names('a\n  b\n  \rc'), '#TX(a) #IN #TX(b) #DE \r#TX(c)')
    assert.equal(names('a\n  b\n  \r  \r  c'), '#TX(a) #IN #TX(b) #NL \r  \r  #TX(c)')
    assert.equal(names('a\n\t\r  b'), '#TX(a) #IN \r  #TX(b)')
    assert.equal(names('\r  a'), ' BAD:bad_indent ')
    assert.equal(names('\ra'), '\r#TX(a)')
    assert.equal(names('a\r\n\r  b'), '#TX(a) #IN \r  #TX(b)')
  })

  it('the trivia before the first form is scanned once', () => {
    for (const sep of ['\r', '\n', '\r\n']) {
      const src = `${sep};comment`.repeat(10_000) + '\na b'
      scans.count = 0
      const value = parseValue(src)
      assert.equal(JSON.stringify(value).split('"a"').length - 1, 1)
      assert.equal(scans.count, 1, JSON.stringify(sep))
    }
    let state = initialState()
    const src = '\r;x\r;y\n  \nab'
    assert.equal(decide(src, 0, state).kind, 'pass')
    assert.equal(state.checked, src.length - 2)
    assert.equal(decide(src, 3, state).kind, 'pass')
    assert.equal(decide(src, 6, state).kind, 'pass')
    assert.equal(state.checked, src.length - 2)
    state = initialState()
    assert.equal(decide('\r;x\n;y', 0, state).kind, 'pass')
    assert.equal(state.checked, 6)
    scans.count = 0
    words('a\n  b\n  c (d\n e)\nf')
    assert.equal(scans.count, 3)
  })

  it('a line holding only trivia after a lone carriage return is blank', () => {
    assert.equal(names('f x\n\r'), '#TX(f) #TX(x)\n\r')
    assert.equal(names('f x\n  \r  '), '#TX(f) #TX(x)\n  \r  ')
    assert.equal(names('a\n  b\n  \r;c\nd'), '#TX(a) #IN #TX(b) #DE #TX(d)')
    assert.equal(names('a\n  b\n\r;x\n  c'), '#TX(a) #IN #TX(b) #NL #TX(c)')
    assert.equal(names('a\n;x\r  c'), '#TX(a) #IN \r  #TX(c)')
    assert.equal(names('\r;x\n  a'), ' BAD:bad_indent ')
  })

  it('an error after a lone carriage return counts its column from the last', () => {
    assert.deepStrictEqual(decide('  \r  a', 0, initialState()), { kind: 'bad', code: 'bad_indent', at: 5, rowCr: 2 })
    assert.deepStrictEqual(decide('\r \r   a', 0, initialState()), {
      kind: 'bad',
      code: 'bad_indent',
      at: 6,
      rowCr: 2,
    })
    assert.deepStrictEqual(decide('a\n\r \tb', 1, { ...initialState(), seen: true }), {
      kind: 'bad',
      code: 'tab_indent',
      at: 4,
      rowCr: 2,
    })
    assert.deepStrictEqual(decide('a\n  \r   b', 1, { ...initialState(), seen: true }), {
      kind: 'bad',
      code: 'bad_indent',
      at: 8,
      rowCr: 4,
    })
  })

  it('layout is suspended inside delimiters', () => {
    assert.equal(names('a (b\n  c)\n  d'), '#TX(a) (#TX(b)\n  #TX(c)) #IN #TX(d)')
    assert.equal(names('[1\n2]'), '[#NR(1)\n#NR(2)]')
  })

  it('a stray closer is unbalanced', () => {
    assert.equal(names('a )'), '#TX(a)  BAD:unbalanced ')
    assert.equal(names('(a))'), '(#TX(a)) BAD:unbalanced ')
  })

  it('indentation errors have their own codes', () => {
    assert.equal(names('a\n\tb'), '#TX(a) BAD:tab_indent ')
    assert.equal(names('a\n   b'), '#TX(a) BAD:bad_indent ')
    assert.equal(names('a\n  b\n c'), '#TX(a) #IN #TX(b) BAD:bad_dedent ')
    assert.equal(names('  a'), ' BAD:bad_indent ')
    assert.equal(names('\n\n  a'), ' BAD:bad_indent ')
  })

  it('nesting past the bound is too deep at the opener or the line', () => {
    // A layout line and 255 open parens are 256 levels; the 256th paren is
    // one more.
    const atLimit = '('.repeat(MAX_NESTING - 1)
    assert.equal(names(atLimit), atLimit)
    assert.equal(names('('.repeat(MAX_NESTING)), `${atLimit} BAD:too_deep `)
    assert.ok(!names(staircase(MAX_NESTING - 1)).includes('BAD'))
    assert.ok(names(staircase(MAX_NESTING)).endsWith(' BAD:too_deep '))
    const mixed = `${staircase(2)}\n${'  '.repeat(3)}[`
    assert.ok(names(mixed + '('.repeat(MAX_NESTING - 5)).endsWith('('))
    assert.ok(names(mixed + '('.repeat(MAX_NESTING - 4)).endsWith(' BAD:too_deep '))
  })

  it('leading trivia before the first line issues nothing', () => {
    assert.equal(names('\n\n  \na'), '\n\n  \n#TX(a)')
  })

  it('the depth scan ignores strings and comments', () => {
    const state = initialState()
    const src = '("a)" ; )\n)'
    assert.equal(depthAt(state, src, src.length - 1), 1)
    assert.equal(depthAt(state, src, src.length), 0)
    // Asking about an earlier position restarts the scan.
    assert.equal(depthAt(state, src, 1), 1)
  })

  // The bag belongs to a live parse, so the state is observed from a lexer
  // subscriber over the real grammar.
  it('the state lives in the parse context, one object under one key', () => {
    const tn = make()
    const seen: any[] = []
    const objects = new Set<unknown>()
    tn.sub({
      lex: (_token: any, _rule: any, ctx: any) => {
        const state = ctx.u.alchemyLayout
        if (undefined !== state) {
          objects.add(state)
          seen.push({ ...state })
        }
      },
    })
    tn.parse('a\n  b\n    c\nd')
    assert.ok(seen.some((s) => 2 === s.levels && s.seen), 'some token was lexed three levels deep')
    const last = seen[seen.length - 1]
    assert.deepStrictEqual([last.seen, last.levels, last.pending, last.depth], [true, 0, 0, 0])
    assert.equal(objects.size, 1, 'one state object throughout a parse')
    // A second parse starts from a fresh state.
    tn.parse('x')
    assert.equal(objects.size, 2)
  })
})
