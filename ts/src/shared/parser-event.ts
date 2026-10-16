/* Copyright (c) 2026 tabnas, MIT License */

// `ParserEvents/1`: the parser's rule passes, as a transducer part reads
// them. A port of rs/src/shared/parser_event.rs.
//
// A source over a tabnas parse (transduce's parser-event source) relays
// each rule pass the engine reports to its `ruleDone` subscribers as one
// `ParserEvent`: an `open` when a rule's open pass has run, a `close` when
// its close pass has, and one `end` after the last pass, only once the
// engine has returned the parse's value, so a document is complete only
// when it validated. A pass made inside a probe (the engine's
// `done.probe`) is speculative, and the source drops it.
//
// Containers never travel in parser events: a rule's node is handed on
// only as a scalar (`RulePass.value`), so a transducer that wants a
// container emits its start and end from the passes that open and close
// it, and nothing in the stream grows with the document.
//
// The fixture encoding (`parserEventToJSON`) follows the JSON events'
// encoding: an array whose first element names the event, `["open",
// {"name": "map", "d": 1, "i": 3, "alt": {"p": "pair", "r": "", "b": 0,
// "g": ["map", "json"]}, "tokens": [["#OB", "{", "{", 1, 1]], "forced":
// false, "n": {"pk": 0}}]`, and `["end"]`. A field that is absent (`alt`
// when the state declared no alternates, `value` when the node is not a
// scalar or the pass is an open) is left out, so `"value": null` is always
// a null scalar.

import { Datum, fromJSON, toJSON } from './datum'
import type { Flow } from './sink'

// The alternate a pass matched.
export interface PassAlt {
  // The rule it pushed, or empty.
  readonly p: string
  // The rule it replaced the current one with, or empty.
  readonly r: string
  // How many tokens it gave back.
  readonly b: number
  // Its groups.
  readonly g: ReadonlyArray<string>
}

// A token a pass consumed.
export interface PassToken {
  // The token's name (`#OB`, `#ST`, `#NR`).
  readonly name: string
  // The token's value: a string's contents, a number's value.
  readonly val: Datum
  // The source text the token was lexed from.
  readonly src: string
  readonly row: number
  readonly col: number
}

// What one rule pass reports.
export interface RulePass {
  // The rule's name (`map`, `pair`, `val`).
  readonly name: string
  // The rule's depth on the stack: one more than the rule that pushed it,
  // the same as the rule it replaced.
  readonly d: number
  // The rule's instance number in the parse.
  readonly i: number
  // The alternate that matched; absent when the state declared none.
  readonly alt?: PassAlt
  // The tokens the pass consumed.
  readonly tokens: ReadonlyArray<PassToken>
  // A close synthesized by recovery.
  readonly forced: boolean
  // The rule's named counters, by name.
  readonly n: ReadonlyArray<readonly [string, number]>
  // The node's value, when it is a scalar at a close pass.
  readonly value?: Datum
}

// One item of `ParserEvents/1`.
export type ParserEvent =
  | { readonly type: 'open'; readonly pass: RulePass }
  | { readonly type: 'close'; readonly pass: RulePass }
  | { readonly type: 'end' }

// A consumer of `ParserEvents/1`: what a transducer part's head is, and
// what a source pushes each rule pass into, synchronously, on the thread
// that parses. As with `Sink`, `event` throws a `Fail` to fail the run and
// answers `'stop'` to ask the source to stop the parse.
export interface ParserSink {
  event(ev: ParserEvent): Flow
}

// The fixture encoding: `["open", {...}]`, `["close", {...}]` or
// `["end"]`.
export function parserEventToJSON(ev: ParserEvent): unknown {
  if ('end' === ev.type) return ['end']
  const pass = ev.pass
  const rule: Record<string, unknown> = { name: pass.name, d: pass.d, i: pass.i }
  if (undefined !== pass.alt) {
    rule.alt = { p: pass.alt.p, r: pass.alt.r, b: pass.alt.b, g: [...pass.alt.g] }
  }
  rule.tokens = pass.tokens.map((t) => [t.name, toJSON(t.val), t.src, t.row, t.col])
  rule.forced = pass.forced
  rule.n = Object.fromEntries(pass.n)
  if (undefined !== pass.value) rule.value = toJSON(pass.value)
  return [ev.type, rule]
}

// An event from its fixture encoding; throws an `Error` saying what is
// wrong with it.
export function parserEventFromJSON(value: unknown): ParserEvent {
  const text = JSON.stringify(value)
  const wrong = (what: string): never => {
    throw new Error(`${what}: ${text}`)
  }
  if (!Array.isArray(value)) return wrong('a parser event is an array')
  const name = value[0]
  if ('end' === name && 1 === value.length) return { type: 'end' }
  if (('open' !== name && 'close' !== name) || 2 !== value.length) return wrong('not a parser event')
  const rule = value[1]
  if (null == rule || 'object' !== typeof rule || Array.isArray(rule)) return wrong('a rule pass is an object')
  const count = (v: unknown, key: string): number =>
    Number.isInteger(v) && (v as number) >= 0 ? (v as number) : wrong(`a rule pass's :${key} is a count`)
  const string = (v: unknown, key: string): string =>
    'string' === typeof v ? v : wrong(`a rule pass's :${key} is a string`)
  let alt: PassAlt | undefined
  if (undefined !== rule.alt) {
    const a = rule.alt
    if (!Array.isArray(a.g) || !a.g.every((g: unknown) => 'string' === typeof g)) {
      return wrong("an alternate's :g is a vector of strings")
    }
    alt = { p: string(a.p, 'p'), r: string(a.r, 'r'), b: count(a.b, 'b'), g: a.g }
  }
  if (!Array.isArray(rule.tokens)) return wrong("a rule pass's :tokens is a vector")
  const tokens = rule.tokens.map((t: unknown): PassToken => {
    if (!Array.isArray(t) || 5 !== t.length) return wrong('a token is [name, val, src, row, col]')
    return {
      name: string(t[0], 'name'),
      val: fromJSON(t[1]),
      src: string(t[2], 'src'),
      row: count(t[3], 'row'),
      col: count(t[4], 'col'),
    }
  })
  if (null == rule.n || 'object' !== typeof rule.n || Array.isArray(rule.n)) {
    return wrong("a rule pass's :n is a record")
  }
  const n = Object.entries(rule.n).map(([k, v]): readonly [string, number] =>
    Number.isInteger(v) ? [k, v as number] : wrong("a rule pass's counters are integers"),
  )
  if ('boolean' !== typeof rule.forced) return wrong("a rule pass's :forced is a boolean")
  const pass: RulePass = {
    name: string(rule.name, 'name'),
    d: count(rule.d, 'd'),
    i: count(rule.i, 'i'),
    ...(undefined === alt ? {} : { alt }),
    tokens,
    forced: rule.forced,
    n,
    ...(undefined === rule.value ? {} : { value: fromJSON(rule.value) }),
  }
  return { type: name, pass }
}
