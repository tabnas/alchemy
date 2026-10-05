/* Copyright (c) 2026 tabnas, MIT License */

// `events` (rs/tests/events_test.rs): the source's events as items a
// program reads one by one, and the operators a renderer over them needs
// (`push`, `pop`, `top`, `count`, `quoted`, `repeat`). The proof is a
// renderer of arbitrary nesting, which nothing before `events` could
// express: a tiny YAML-like block form written by a `scan-emit` whose state
// is the stack of open containers, one keyword marker each, asserted byte
// for byte; then the affine rule over `events`, and the plan report.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { Tabnas } from '@tabnas/parser'
import { jsonic } from '@tabnas/jsonic'
import { Yaml } from '@tabnas/yaml'
import { BytesWriter } from '@tabnas/render'
import { Limits, Metrics, ParserSource } from '@tabnas/transduce'

import { compile, drive, ok, thrown } from './common'

// A YAML-like block renderer over the events. The state is the stack of
// open containers, one marker each: `:object-open` and `:array-open` for a
// container with nothing written yet (its opening line is held until its
// first child or its end, so an empty one writes as `{}` or `[]` on the
// key's line), `:object` for an object waiting for a key, `:member` for one
// whose key is written and waits for the value, `:array` for an array
// waiting for an item.
const RENDER = `def indent [ctx]
  repeat (count ctx) "  "

def replace-top [marker s]
  push marker (pop s)

; A value completed: an object waiting for its member's value waits for
; its next key.
def done [s]
  match (count s)
    case 0 s
    case _
      match (top s)
        case :member (replace-top :object s)
        case _ s

; What comes before a value whose parent is the top of ctx: a space on
; the key's line, or a dash, for a scalar or an empty container; a line
; break for a mapping or a sequence under a key; a dash and the first key
; for a mapping in a sequence; a dash alone for a sequence in a sequence.
def lead [kind ctx]
  match (count ctx)
    case 0 ""
    case _
      match (top ctx)
        case :member
          match kind
            case :mapping (concat "\\n" (indent ctx))
            case :sequence "\\n"
            case _ " "
        case :array
          match kind
            case :sequence (concat (indent (pop ctx)) "-\\n")
            case _ (concat (indent (pop ctx)) "- ")

; A held array is opened as a sequence by its first item.
def opened [s]
  match (count s)
    case 0 s
    case _
      match (top s)
        case :array-open (replace-top :array s)
        case _ s

def opening [s]
  match (count s)
    case 0 ""
    case _
      match (top s)
        case :array-open (lead :sequence (pop s))
        case _ ""

def yaml-scalar [value]
  match (kind value)
    case :null "null"
    case :boolean
      match value
        case true "true"
        case false "false"
    case :number (scalar-text csv-options value)
    case _ (quoted value)

def step [s event]
  match event
    case object-start
      transition (push :object-open (opened s)) [(opening s)]
    case array-start
      transition (push :array-open (opened s)) [(opening s)]
    case (key name)
      match (top s)
        case :object-open
          transition (replace-top :member s) [(lead :mapping (pop s)) (quoted name) ":"]
        case :object
          transition (replace-top :member s) [(indent (pop s)) (quoted name) ":"]
    case (scalar value)
      let [s2 (opened s)]
        transition (done s2) [(opening s) (lead :scalar s2) (yaml-scalar value) "\\n"]
    case object-end
      match (top s)
        case :object-open
          transition (done (pop s)) [(lead :scalar (pop s)) "{}\\n"]
        case _
          transition (done (pop s)) []
    case array-end
      match (top s)
        case :array-open
          transition (done (pop s)) [(lead :scalar (pop s)) "[]\\n"]
        case _
          transition (done (pop s)) []

def finish [s]
  match (count s)
    case 0 []
    case _ (fail "the events ended inside a container")

def export [input]
  join ""
    scan-emit [] step finish (events input)
`

// A program that writes each key on its own line, in the explicit form past
// YAML's 1024-character limit on an implicit key (`length` and `compare`),
// and each number in YAML's spellings, the non-finite ones included
// (`number-class`), as a YAML render does.
const KEY_FORMS = `def key-line [name]
  match (compare (length (quoted name)) 1024)
    case :greater ["? " (quoted name) "\\n"]
    case _ [(quoted name) ":\\n"]

def number-line [n]
  match (number-class n)
    case :finite [(scalar-text csv-options n) "\\n"]
    case :infinity [".inf\\n"]
    case :negative-infinity ["-.inf\\n"]
    case :nan [".nan\\n"]

def step [s event]
  match event
    case (key name) (transition s (key-line name))
    case (scalar v)
      match (kind v)
        case :number (transition s (number-line v))
        case _ (transition s [])
    case _ (transition s [])

def finish [s] []

def export [input]
  join ""
    scan-emit [] step finish (events input)
`

describe('events', () => {
  // The renderer writes a nested document, with an empty object and an
  // empty array, strings that need escapes, and a scalar root, byte for
  // byte as the block form has them.
  it('events render a nested document as a YAML-like block', () => {
    const program = compile(RENDER, 'render.alc')
    assert.equal(program.output, 'Text')
    // Every event is needed, so nothing is pruned.
    assert.equal(program.rowSelector(), undefined)
    const nested = '{"a":"x","b":["y",null,{"c":"q\\"x\\n","d":{}}],"e":[],"f":{"g":[[]]},"n":1.5e3,"z":-0}'
    assert.equal(
      ok(program, nested),
      '"a": "x"\n' +
        '"b":\n' +
        '  - "y"\n' +
        '  - null\n' +
        '  - "c": "q\\"x\\n"\n' +
        '    "d": {}\n' +
        '"e": []\n' +
        '"f":\n' +
        '  "g":\n' +
        '    - []\n' +
        '"n": 1.5e3\n' +
        '"z": -0\n',
    )
    // A number is told from a string by `kind` and written by its lexeme;
    // a string that spells a number stays quoted.
    assert.equal(ok(program, '42'), '42\n')
    assert.equal(ok(program, '["42",42]'), '- "42"\n- 42\n')
    // A scalar root stands alone; its escapes are JSON's, with DEL and the
    // C1 controls escaped too, and everything else as itself.
    assert.equal(
      ok(program, '"tab\\there \\u0001 \\u007f \\u0085 caf\\u00e9 \\\\ \\""'),
      '"tab\\there \\u0001 \\u007f \\u0085 café \\\\ \\""\n',
    )
    assert.equal(ok(program, 'true'), 'true\n')
    assert.equal(ok(program, 'null'), 'null\n')
    // Empty containers at the root, and a sequence in a sequence.
    assert.equal(ok(program, '{}'), '{}\n')
    assert.equal(ok(program, '[]'), '[]\n')
    assert.equal(ok(program, '[["a"],[],{"k":[false]}]'), '-\n  - "a"\n- []\n- "k":\n    - false\n')
    // A key the source repeats is delivered as it arrives, twice: nothing
    // is mapped by key between events.
    assert.equal(ok(program, '{"a":"1","a":"2"}'), '"a": "1"\n"a": "2"\n')
    // The stack is what the stage retains: a document nested deeper than
    // `max_depth` is refused by the source before the state could grow to
    // it, and the failure names the limit.
    const deep = `${'['.repeat(40)}1${']'.repeat(40)}`
    const { fail } = drive(program, deep, undefined, Limits.with({ max_depth: 16 }))
    assert.equal(fail.code, 'RESOURCE_LIMIT_EXCEEDED', String(fail))
    assert.equal(fail.limit.name, 'max_depth')
  })

  // Past 1024 characters, quotes included, a key takes the explicit form;
  // at 1024 it is still implicit.
  it('a key past the implicit limit takes the explicit form', () => {
    const program = compile(KEY_FORMS, 'keys.alc')
    const long = 'k'.repeat(1023)
    assert.equal(ok(program, `{"${long.substring(0, 1022)}":1}`), `"${long.substring(0, 1022)}":\n1\n`)
    assert.equal(ok(program, `{"${long}":1}`), `? "${long}"\n1\n`)
  })

  // YAML's non-finite numbers arrive as numbers with no lexeme, and a
  // program writes them in YAML's spellings, where `scalar-text` would
  // refuse them as JSON and CSV must; a quoted `'.inf'` is a string.
  it('the non-finite numbers are written in YAML\'s spellings', () => {
    const program = compile(KEY_FORMS, 'keys.alc')
    const writer = new BytesWriter()
    const sink = program.sink(writer, undefined, Limits.default(), new Metrics())
    new ParserSource(new Tabnas().use(jsonic).use(Yaml), "- .inf\n- -.inf\n- .nan\n- 1.5\n- '.inf'\n").run(sink)
    assert.equal(writer.text(), '.inf\n-.inf\n.nan\n1.5\n')
  })

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

  // The report names the stage: every event delivered as an item, nothing
  // retained by it, and the `scan-emit` after it conditional on its state.
  it('explain reports an events pipeline', () => {
    const program = compile(RENDER, 'render.alc')
    const text = program.explain()
    assert.ok(text.startsWith('export: events → scan-emit → join\n'), text)
    assert.ok(text.includes('Protocol:              JsonEvents/1 → Stream<Event> → Stream<Value> → Text\n'), text)
    assert.ok(text.includes('Selection:             none; every event is delivered as an item\n'), text)
    assert.ok(
      text.includes(
        'Duplicate members:     preserved; the events are copied as they arrive, not mapped by key\n',
      ),
      text,
    )
    assert.ok(
      text.includes(
        'Retained state:        what the step returns, no deeper than max_depth, capped at max_metadata_bytes\n',
      ),
      text,
    )
    assert.ok(text.includes('Output order:          source order\n'), text)
    let j: any = program.explainJson()
    assert.deepStrictEqual(j.chain, ['events', 'scan-emit', 'join'])
    assert.deepStrictEqual(j.protocol, ['JsonEvents/1', 'Stream<Event>', 'Stream<Value>', 'Text'])
    assert.equal(j.confidence, 'conditional')
    assert.equal(j.readiness, 'event')
    assert.equal(j.retention.length, 1)
    assert.equal(j.retention[0].scope, 'state')
    assert.equal(j.renderer.name, 'text')
    assert.equal(j.output, 'Text')
    // A map over the events alone retains nothing at all.
    const keys = compile(
      'def export [input]\n  join ""\n    map\n      fn [event]\n        match event\n          case (key name) (concat (quoted name) "\\n")\n          case _ ""\n      events input\n',
      'keys.alc',
    )
    j = keys.explainJson()
    assert.equal(j.confidence, 'proven')
    assert.deepStrictEqual(j.retention, [])
    assert.equal(ok(keys, '{"a":1,"b":{"c":[2,{"d":3}]},"a":4}'), '"a"\n"b"\n"c"\n"d"\n"a"\n')
  })

  // A stream of events a program built reaches any taker of JSON events,
  // the reverse of `events`: the events and back through `json` is the
  // document again, lexemes kept; a `scan-emit` over them rewrites it on
  // the way; `table-from-json` reads the rows a step made; an item that is
  // not an event is refused as the stream runs, naming it; and a stream of
  // values is refused before anything runs.
  it('events a program builds feed any taker of JSON events', () => {
    const back = compile('def export [input] (json (events input))', 'back.alc')
    assert.equal(back.output, 'Text')
    const nested = '{"a":"x","b":["y",null,{"c":1.50,"d":{}}],"e":[],"n":-0,"t":true}'
    assert.equal(ok(back, nested), `${nested}\n`)
    const rename = compile(
      'def step [s e]\n  match e\n    case (key "a") (transition s [(key "b")])\n    case _ (transition s [e])\n\ndef export [input]\n  json (scan-emit [] step (fn [s] []) (events input))',
      'rename.alc',
    )
    assert.equal(ok(rename, '{"a":1,"x":{"a":[2]}}'), '{"b":1,"x":{"b":[2]}}\n')
    // The rows a step made: each scalar of the root array wrapped as an
    // object of one member, read by table-from-json, written by records.
    const wrap = compile(
      'def step [s e]\n  match s\n    case 0 (transition 1 [e])\n    case _\n      match e\n        case (scalar v) (transition 1 [object-start (key "value") e object-end])\n        case _ (transition 1 [e])\n\ndef rows (record (entry :columns :infer) (entry :rows (path each-index)))\n\ndef export [input]\n  json (records (table-from-json rows (scan-emit 0 step (fn [s] []) (events input))))',
      'wrap.alc',
    )
    assert.equal(ok(wrap, '[1,"two",true]'), '[{"value":1},{"value":"two"},{"value":true}]\n')
    // An item that is not an event is refused where it arrives.
    const bad = compile(
      'def export [input] (json (scan-emit [] (fn [s e] (transition s [1])) (fn [s] []) (events input)))',
      'bad.alc',
    )
    const { fail, out } = drive(bad, '[1]')
    assert.equal(fail.code, 'PROTOCOL_ORDER_ERROR', String(fail))
    assert.ok(fail.message.startsWith('an event was expected, not'), String(fail))
    assert.equal(out, '')
    // A stream of values is not a stream of events: the checker says so.
    const values = thrown(() => compile('def export [input] (json (select (path each-index) input))', 'values.alc'))
    assert.ok(values.message.startsWith('protocol_mismatch'), String(values))
    // The source's limits hold on the events a program made: a document
    // built deeper than max_depth from a flat input, or a key longer than
    // max_key_bytes, is refused where the limit is passed, as the source's
    // own events would be.
    const deepen = compile(
      'def step [s e]\n  match e\n    case (scalar true) (transition s [array-start])\n    case (scalar false) (transition s [array-end])\n    case _ (transition s [])\n\ndef export [input]\n  json (scan-emit [] step (fn [s] []) (events input))',
      'deepen.alc',
    )
    const flat = (n: number) => `[${[...new Array(n).fill('true'), ...new Array(n).fill('false')].join(',')}]`
    const limits = Limits.with({ max_depth: 8 })
    const eight = drive(deepen, flat(8), undefined, limits)
    assert.equal(eight.fail, undefined, String(eight.fail))
    assert.equal(eight.out, `${'['.repeat(8)}${']'.repeat(8)}\n`)
    const nine = drive(deepen, flat(9), undefined, limits)
    assert.equal(nine.fail.code, 'RESOURCE_LIMIT_EXCEEDED', String(nine.fail))
    assert.equal(nine.fail.limit.name, 'max_depth', String(nine.fail))
    const widen = compile(
      'def export [input] (json (scan-emit [] (fn [s e] (transition s [object-start (key (repeat 40 "k")) e object-end])) (fn [s] []) (events input)))',
      'widen.alc',
    )
    const wide = drive(widen, '1', undefined, Limits.with({ max_key_bytes: 16 }))
    assert.equal(wide.fail.code, 'RESOURCE_LIMIT_EXCEEDED', String(wide.fail))
    assert.equal(wide.fail.limit.name, 'max_key_bytes', String(wide.fail))
  })
})
