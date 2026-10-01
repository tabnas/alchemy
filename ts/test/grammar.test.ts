/* Copyright (c) 2026 tabnas, MIT License */

// The grammar plugin (rs/src/grammar.rs's tests): the tagged tree the
// reader builds, its errors with their codes and positions, and the
// document held to the code that installs it.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { Tabnas } from '@tabnas/parser'

import {
  MATCHER,
  MAX_NESTING,
  MESSAGES,
  TOO_DEEP,
  alchemy,
  grammarDocument,
  make,
  parse,
  parseValue,
  refs,
} from '../dist/alchemy'

import { finer, thrown } from './common'

function json(src: string): string {
  return JSON.stringify(parseValue(src))
}

function code(src: string): string {
  const err = thrown(() => parseValue(src))
  return err.code
}

describe('grammar', () => {
  it('an empty program is an empty array', () => {
    assert.equal(json(''), '[]')
    assert.equal(json('\n\n; only a comment\n  \n'), '[]')
    // Each empty parse answers its own array.
    const a = parseValue('')
    a.push(1)
    assert.equal(json(''), '[]')
  })

  it('a line with one form is that form', () => {
    assert.equal(json('x'), '[{"$":"sym","name":"x","span":[0,1]}]')
    assert.equal(json('"a\\nb"'), '[{"$":"str","value":"a\\nb","span":[0,6]}]')
    assert.equal(json('-1.5e3'), '[{"$":"num","lexeme":"-1.5e3","span":[0,6]}]')
    assert.equal(json('true'), '[{"$":"bool","value":true,"span":[0,4]}]')
    assert.equal(json('null'), '[{"$":"null","span":[0,4]}]')
    assert.equal(json(':k'), '[{"$":"kw","name":"k","span":[0,2]}]')
  })

  it('several inline forms make a list', () => {
    assert.equal(
      json('a b'),
      '[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[2,3]}],"span":[0,3]}]',
    )
  })

  it('explicit delimiters carry their own spans', () => {
    assert.equal(json('( a )'), '[{"$":"list","items":[{"$":"sym","name":"a","span":[2,3]}],"span":[0,5]}]')
    assert.equal(json('()'), '[{"$":"list","items":[],"span":[0,2]}]')
    assert.equal(json('[]'), '[{"$":"vector","items":[],"span":[0,2]}]')
    assert.equal(
      json('[1 [2]]'),
      '[{"$":"vector","items":[{"$":"num","lexeme":"1","span":[1,2]},{"$":"vector","items":[{"$":"num","lexeme":"2","span":[4,5]}],"span":[3,6]}],"span":[0,7]}]',
    )
  })

  it('children follow the inline forms', () => {
    assert.equal(
      json('a\n  b\n  c d\ne'),
      '[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[4,5]},{"$":"list","items":[{"$":"sym","name":"c","span":[8,9]},{"$":"sym","name":"d","span":[10,11]}],"span":[8,11]}],"span":[0,11]},{"$":"sym","name":"e","span":[12,13]}]',
    )
  })

  it('a dedent of several levels closes each block', () => {
    assert.equal(
      json('a\n  b\n    c\nd'),
      '[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"list","items":[{"$":"sym","name":"b","span":[4,5]},{"$":"sym","name":"c","span":[10,11]}],"span":[4,11]}],"span":[0,11]},{"$":"sym","name":"d","span":[12,13]}]',
    )
  })

  it('a lone carriage return at a line start is whitespace', () => {
    assert.equal(
      json('f x\n\r'),
      '[{"$":"list","items":[{"$":"sym","name":"f","span":[0,1]},{"$":"sym","name":"x","span":[2,3]}],"span":[0,3]}]',
    )
    assert.equal(
      json('a\n  b\n  \r;c\nd'),
      '[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[4,5]}],"span":[0,5]},{"$":"sym","name":"d","span":[12,13]}]',
    )
    assert.equal(
      json('a\n  b\n\r;x\n  c'),
      '[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[4,5]},{"$":"sym","name":"c","span":[12,13]}],"span":[0,13]}]',
    )
    assert.equal(
      json('a\n\r  b'),
      '[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[5,6]}],"span":[0,6]}]',
    )
    assert.equal(json('a\n  \r  \rb'), '[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[8,9]}]')
    assert.equal(
      json('a\n;x\r  c\r;y\r  d'),
      '[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"list","items":[{"$":"sym","name":"c","span":[7,8]},{"$":"sym","name":"d","span":[14,15]}],"span":[7,15]}],"span":[0,15]}]',
    )
    // The column a diagnostic names counts from the carriage return, as the
    // engine counts it.
    const at = (src: string) => {
      const fail = thrown(() => parse(src))
      return [finer(fail), fail.row, fail.col]
    }
    assert.deepStrictEqual(at('\r  a'), ['bad_indent', 1, 3])
    assert.deepStrictEqual(at('a\n  b\n  \r   c'), ['bad_indent', 3, 4])
    assert.deepStrictEqual(at('a\n\r \tb'), ['tab_indent', 2, 2])
    assert.deepStrictEqual(at('\r \r   a'), ['bad_indent', 1, 4])
    assert.deepStrictEqual(at('a\n  \r \r   b'), ['bad_indent', 2, 4])
  })

  it('the errors carry the grammar codes', () => {
    assert.equal(code('a\n\tb'), 'tab_indent')
    assert.equal(code('  a'), 'bad_indent')
    assert.equal(code('a\n   b'), 'bad_indent')
    assert.equal(code('a\n  b\n c'), 'bad_dedent')
    assert.equal(code('a )'), 'unbalanced')
    assert.equal(code('(a b'), 'unbalanced')
    assert.equal(code('[a b'), 'unbalanced')
    assert.equal(code('(a]'), 'unbalanced')
    assert.equal(code('"abc'), 'unterminated_string')
    assert.equal(code('a { b'), 'unexpected')
    assert.equal(code('"\\q"'), 'unexpected')
    assert.equal(code('('.repeat(MAX_NESTING)), 'too_deep')
  })

  // The bound is named in the message and the hint the document declares.
  it('the too_deep texts name the bound', () => {
    const doc = grammarDocument()
    const bound = String(MAX_NESTING)
    assert.ok(TOO_DEEP.includes(bound))
    assert.ok(doc.options.hint.too_deep.includes(bound))
    assert.equal(doc.options.error.too_deep, TOO_DEEP)
  })

  // The desugaring codes are declared in the document so a fixture can pin
  // them like the reader's, and raised in the desugarer with its own text;
  // the two are one message.
  it('the desugaring messages match the document', () => {
    const doc = grammarDocument()
    for (const [c, message] of MESSAGES) {
      assert.equal(doc.options.error[c], message, `options.error.${c}`)
      assert.equal(typeof doc.options.hint[c], 'string', `options.hint.${c} is declared`)
    }
  })

  it('the fail carries the code and the position', () => {
    const fail = thrown(() => parse('a\n  b\n c'))
    assert.equal(fail.code, 'DSL_PARSE_ERROR')
    assert.ok(fail.message.startsWith('bad_dedent: '), fail.message)
    assert.deepStrictEqual([fail.row, fail.col], [3, 2])
  })

  // Every `@` reference the document names is one this package registers,
  // and every one it registers is named: the grammar text and the code
  // that installs it cannot drift apart.
  it('the refs the document names are the ones registered', () => {
    const named = new Set<string>()
    const walk = (value: unknown) => {
      if ('string' === typeof value && value.startsWith('@')) named.add(value)
      else if (Array.isArray(value)) value.forEach(walk)
      else if (null != value && 'object' === typeof value) Object.values(value).forEach(walk)
    }
    walk(grammarDocument())
    const registered = Object.keys(refs())
    assert.equal(registered.length, 14)
    assert.ok(registered.includes(MATCHER))
    assert.deepStrictEqual([...named].sort(), [...registered].sort())
  })

  // Whole numbers come back as integers (`b` is an integer field), and the
  // document is JSON with `#` comments: it reads without jsonic.
  it('the document reads with integer fields', () => {
    const doc = grammarDocument()
    const back = doc.rule.form.close.find((alt: any) => 1 === alt.b)
    assert.ok(Number.isInteger(back.b))
    assert.equal(doc.options.lex.match.alchemy.order, 100000)
    assert.deepStrictEqual(doc.ruleOrder, ['program', 'line', 'block', 'form', 'paren', 'bracket'])
    // A fresh copy each time: the engine's normalisation cannot reach the
    // cached document.
    doc.rule.program = null
    assert.notEqual(grammarDocument().rule.program, null)
  })

  it('the plugin installs on a bare engine, as make does', () => {
    const tn = new Tabnas().use(alchemy)
    assert.equal(JSON.stringify(tn.parse('def x 1')), JSON.stringify(make().parse('def x 1')))
    assert.equal(tn.parse('def x 1')[0].$, 'list')
    assert.throws(() => make().parse('(a b'))
  })
})
