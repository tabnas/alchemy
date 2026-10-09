/* Copyright (c) 2026 tabnas, MIT License */

// The natives' implementations: the half of rs/src/stdlib/registry.rs the
// front end does not need. `./registry` is the one table of natives (arity,
// kind, signature, effect); this module installs an implementation for
// every name in it (`installCalls`), and `implOf` answers it to the
// evaluator.
//
// Every operator takes its data last (spec section 9.3). The
// implementations are eager for values and build plans for streams and
// texts (spec section 10.3): `map` over a vector maps now, `map` over a
// stream answers a plan the runtime lowers once.
//
// Failures on data (a descriptor that is not an object, a path segment that
// is not a string) are `INPUT_INVALID`, the code the native table
// transducer raises for the same shapes, so the interpreted and the native
// standard library fail alike; a value of the wrong kind where the program,
// not the data, chose it is `DSL_TYPE_ERROR`.
//
// An implementation answers its value, or, when it applies functions
// (`map` and `filter` over a vector), the generator of a walk the
// evaluator drives on its own stack (`./trampoline`), so a function that
// maps itself is the evaluator's `recursion`, never JavaScript's stack.

import { CaptureSpec, Code, Fail, Limits, Selector, isJsonNumber, utf8Bytes } from '../shared'
import type { Renderers } from '../shared'

import { SourceSpan } from '../ast'
import type { Runtime } from '../interp'
import { G } from '../trampoline'
import {
  Func,
  Plan,
  Seq,
  Val,
  bool,
  concatPlan,
  debugText,
  displayNumber,
  field,
  fnVal,
  getPath as valGetPath,
  isLiveVal,
  isMissing,
  keyword,
  kindText,
  liveKind,
  missing as missingVal,
  num,
  record as recordVal,
  selectorSegments,
  selectorVal,
  str,
  streamVal,
  taggedVal,
  textVal,
  typeError,
  vectorVal,
} from '../value'
import { Native, call, installCalls, natives } from './registry'

// The implementation of a native: the runtime (to apply functions), the
// arguments, and the span of the form being evaluated, for a diagnostic.
export type NativeImpl = (rt: Runtime, args: ReadonlyArray<Val>, at: SourceSpan) => Val | G<Val>

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

// Whether a value is data (what a document can hold), as opposed to a
// function, a selector, a capture, a stream or a text.
function isData(v: Val): boolean {
  switch (v.v) {
    case 'null':
    case 'bool':
    case 'num':
    case 'str':
    case 'vector':
    case 'record':
      return true
    default:
      return isMissing(v)
  }
}

function asStr(op: string, what: string, v: Val): string {
  if ('str' === v.v) return v.value
  throw typeError(`${op}: ${what} must be a string, not ${kindText(v)}`)
}

function asKeyword(op: string, what: string, v: Val): string {
  if ('keyword' === v.v) return v.name
  throw typeError(`${op}: ${what} must be a keyword, not ${kindText(v)}`)
}

function asFn(op: string, what: string, v: Val): Func {
  if ('fn' === v.v) return v.f
  throw typeError(`${op}: ${what} must be a function, not ${kindText(v)}`)
}

function asSelector(op: string, what: string, v: Val): Selector {
  if ('selector' === v.v) return v.selector
  throw typeError(`${op}: ${what} must be a selector, not ${kindText(v)}`)
}

function asStream(op: string, v: Val): Plan {
  if ('stream' === v.v) return v.plan
  throw typeError(`${op}: the data must be a stream, not ${kindText(v)}`)
}

function asSeq(op: string, v: Val): Seq {
  if ('vector' === v.v) return { seq: 'vector', items: v.items }
  if ('stream' === v.v) return { seq: 'stream', plan: v.plan }
  throw typeError(`${op}: the data must be a vector or a stream, not ${kindText(v)}`)
}

// A string or a text: what the text algebra accepts as an item.
function textlike(op: string, v: Val): void {
  if ('str' === v.v || 'text' === v.v) return
  throw typeError(`${op}: expected a string or a text, not ${kindText(v)}`)
}

// The items of a vector the program built (its stack, its outputs), or the
// type error that names the operator: what is not a vector here is the
// program's mistake, not the data's (`as-vector` takes data).
function asItems(op: string, v: Val): ReadonlyArray<Val> {
  if ('vector' === v.v) return v.items
  throw typeError(`${op}: the data must be a vector, not ${kindText(v)}`)
}

// A non-negative whole number that fits an index.
function asIndex(v: Val): number | undefined {
  if ('num' !== v.v) return undefined
  const value = v.value
  return value >= 0 && 0 === value % 1 && value <= 0xffffffff ? value : undefined
}

// The field of the options record, or the type error that names it.
function option(op: string, options: Val, key: string): Val {
  if ('record' !== options.v) {
    throw typeError(`${op}: the options must be a record, not ${kindText(options)}`)
  }
  const v = field(options, key) as Val
  if (isMissing(v)) throw typeError(`${op}: the options have no :${key}`)
  return v
}

// A key or a lexeme as Rust's `Debug` quotes it, for messages.
function quotedText(s: string): string {
  return JSON.stringify(s)
}

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

// The magnitudes written positionally, as the renderers write them: from
// `1e-6` up to, not including, `1e21`.
const POSITIONAL_MIN = 1e-6
const POSITIONAL_MAX = 1e21

// The shortest text that reads back as `value`, which must be finite, laid
// out as the render package lays it out (`writeValue` in its number.ts):
// positional within the JavaScript range, exponent form outside it, with no
// `+` and no leading zeros in the exponent (`1e21`, `1e-7`), and negative
// zero as `-0`. This package's own, as the Rust crate's `shortest_number`
// is, so that building a plan asks nothing of the renderers a host passes.
// The digits are the shortest that round-trip, as `toExponential()` with no
// argument gives them; only the layout is chosen here. The two must agree
// byte for byte, which the differential test checks on every number without
// a lexeme (alchemy-cli runs it, on render's renderers).
//
// The form that takes the renderers first is the one 0.2.3 published,
// kept so a caller of it still gets the right text; they are not consulted.
export function shortestNumber(value: number): string
/** @deprecated The renderers are not consulted: pass the value alone. */
export function shortestNumber(renderers: Renderers, value: number): string
export function shortestNumber(first: number | Renderers, second?: number): string {
  const value = 'number' === typeof first ? first : (second as number)
  if (0 === value) return Object.is(value, -0) ? '-0' : '0'
  const neg = value < 0
  const magnitude = neg ? -value : value
  // `d.ddde±x`: the shortest round-trip digits and the decimal exponent.
  const exp = magnitude.toExponential()
  const at = exp.indexOf('e')
  const digits = exp.charAt(0) + exp.slice(2, at)
  const e = parseInt(exp.slice(at + 1), 10)
  const n = digits.length
  let text: string
  if (magnitude >= POSITIONAL_MIN && magnitude < POSITIONAL_MAX) {
    if (e >= n - 1) {
      text = digits + '0'.repeat(e - (n - 1))
    } else if (e >= 0) {
      text = digits.slice(0, e + 1) + '.' + digits.slice(e + 1)
    } else {
      text = '0.' + '0'.repeat(-e - 1) + digits
    }
  } else {
    text = (1 < n ? digits.charAt(0) + '.' + digits.slice(1) : digits) + 'e' + e
  }
  return neg ? '-' + text : text
}

// The text of a number as a renderer writes it, with the renderer's
// checks: a lexeme that is not a JSON number is `INVALID_NUMBER`, a
// non-finite value `TARGET_VALUE_UNREPRESENTABLE`. As with
// `shortestNumber`, the form that takes the renderers first is the one
// 0.2.3 published, kept and not consulted.
export function numberText(value: number, lexeme?: string): string
/** @deprecated The renderers are not consulted: pass the value and lexeme alone. */
export function numberText(renderers: Renderers, value: number, lexeme?: string): string
export function numberText(
  first: number | Renderers,
  second?: number | string,
  third?: string,
): string {
  const [value, lexeme] =
    'number' === typeof first
      ? [first, second as string | undefined]
      : [second as number, third]
  if (undefined !== lexeme && !isJsonNumber(lexeme)) {
    throw new Fail('INVALID_NUMBER', `${quotedText(lexeme)} is not a JSON number`)
  }
  if (!Number.isFinite(value)) {
    const message =
      undefined !== lexeme
        ? `${quotedText(lexeme)} is ${displayNumber(value)} as a number, which has no representation`
        : `${displayNumber(value)} has no representation as a number`
    throw new Fail('TARGET_VALUE_UNREPRESENTABLE', message)
  }
  return undefined !== lexeme ? lexeme : shortestNumber(value)
}

// ---------------------------------------------------------------------------
// The implementations
// ---------------------------------------------------------------------------

// `get key data`: a record's field, `missing` for an absent one or from
// `missing`; `INPUT_INVALID` from other data, `type_mismatch` from what is
// not data. The native table binds a column record by the same rule, so a
// column function that answers something other than a record fails alike
// both ways.
export function getField(key: string, data: Val): Val {
  if ('record' === data.v) return field(data, key) as Val
  if (isMissing(data)) return missingVal()
  if (isData(data)) {
    throw Fail.input(`get: ${kindText(data)} has no member ${quotedText(key)}; an object was expected`)
  }
  throw typeError(`get: a record was expected, not ${kindText(data)}`)
}

function get(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const k = a[0]
  let key: string
  if ('keyword' === k.v) key = k.name
  else if ('str' === k.v) key = k.value
  else throw typeError(`get: the key must be a keyword or a string, not ${kindText(k)}`)
  return getField(key, a[1])
}

function getPath(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const selector = asSelector('get-path', 'the path', a[0])
  const segments = selectorSegments(selector)
  if (undefined === segments) {
    throw typeError(`get-path: the path must name one location, not ${selector}`)
  }
  if (!isData(a[1])) throw typeError(`get-path: the data must be a value, not ${kindText(a[1])}`)
  return valGetPath(a[1], segments)
}

function asPath(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const v = a[0]
  if ('vector' !== v.v) {
    throw Fail.input(`as-path: a path must be an array of segments, not ${kindText(v)}`)
  }
  let selector = Selector.root()
  for (const item of v.items) {
    if ('str' === item.v) {
      selector = selector.property(item.value)
      continue
    }
    const i = asIndex(item)
    if (undefined === i) {
      throw Fail.input(
        `as-path: a path segment must be a string or a non-negative integer, not ${kindText(item)}`,
      )
    }
    selector = selector.index(i)
  }
  return selectorVal(selector)
}

function asVector(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const v = a[0]
  if ('vector' === v.v) return v
  if (isData(v)) throw Fail.input(`as-vector: an array was expected, not ${kindText(v)}`)
  throw typeError(`as-vector: a vector was expected, not ${kindText(v)}`)
}

function record(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const fields = new Map<string, Val>()
  for (const item of a) {
    if ('tagged' === item.v && 'entry' === item.tag && 2 === item.fields.length) {
      const key = asKeyword('record', "an entry's key", item.fields[0])
      if (fields.has(key)) {
        throw new Fail('DSL_TYPE_ERROR', `duplicate_key: record has two entries for :${key}`)
      }
      fields.set(key, item.fields[1])
      continue
    }
    throw typeError(`record: every argument must be an entry, not ${kindText(item)}`)
  }
  return recordVal(fields)
}

function entry(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  asKeyword('entry', 'the key', a[0])
  return taggedVal('entry', a.slice())
}

function vector(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const live = a.find(isLiveVal)
  if (undefined !== live) {
    throw typeError(`vector: a vector cannot hold ${liveKind(live)}; a stream is used once, where it is`)
  }
  return vectorVal(a.slice())
}

// `push item vector`: a new vector, the item last. Bounded by the vector's
// length, as the three below are: a stack of markers is as long as the
// document's nesting, never its length.
function push(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  if (isLiveVal(a[0])) {
    throw typeError(`push: a vector cannot hold ${liveKind(a[0])}; a stream is used once, where it is`)
  }
  const items = asItems('push', a[1])
  return vectorVal([...items, a[0]])
}

function pop(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const items = asItems('pop', a[0])
  if (0 === items.length) throw typeError('pop: the vector is empty; there is no last item to remove')
  return vectorVal(items.slice(0, items.length - 1))
}

function top(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const items = asItems('top', a[0])
  if (0 === items.length) throw typeError('top: the vector is empty; there is no last item')
  return items[items.length - 1]
}

function count(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  return num(asItems('count', a[0]).length)
}

// `keys record`: the record's keys as strings, in its order, which for a
// captured object is the document's. A bounded operation over one record,
// not a fold: the library's inferred table binding reads its columns from
// the first row with it.
function keys(rt: Runtime, a: ReadonlyArray<Val>): Val {
  const v = a[0]
  if ('record' !== v.v) throw typeError(`keys: the record must be a record, not ${kindText(v)}`)
  // A step per key, so the host's abort flag stops a wide record's copy as
  // it stops any other long evaluation.
  const out: Val[] = []
  for (const k of v.fields.keys()) {
    rt.tick()
    out.push(str(k))
  }
  return vectorVal(out)
}

// `indices vector`: the positions of a vector's items, as numbers: what
// the interpreted inferred table labels an array row's cells by. A bounded
// operation over one vector, like `keys` over one record.
function indices(rt: Runtime, a: ReadonlyArray<Val>): Val {
  const v = a[0]
  if ('vector' !== v.v) throw typeError(`indices: a vector was expected, not ${kindText(v)}`)
  // A step per item, as `keys` takes one per key, so the host's abort flag
  // stops a long vector's count like any evaluation.
  const out: Val[] = []
  for (let i = 0; i < v.items.length; i++) {
    rt.tick()
    out.push(num(i))
  }
  return vectorVal(out)
}

// How many bytes of a string `length` counts per evaluation step.
export const LENGTH_CHUNK = 64 * 1024

// `number string`: the number a string spells. A JSON number keeps the
// text as its lexeme, so a renderer writes it as it was spelled; the three
// non-finite numbers are spelled `Infinity`, `-Infinity` and `NaN`.
function number(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const s = asStr('number', 'the string', a[0])
  switch (s) {
    case 'Infinity':
      return num(Infinity)
    case '-Infinity':
      return num(-Infinity)
    case 'NaN':
      return num(NaN)
  }
  if (!isJsonNumber(s)) throw Fail.input(`number: ${JSON.stringify(s)} spells no number`)
  return num(Number(s), s)
}

// `length string`: how many characters the string holds, as a column
// counts them (Unicode scalar values; a surrogate pair is one). A long
// string takes an evaluation step per chunk of its UTF-8 bytes, so the
// host's abort flag stops the count as it stops any long evaluation.
function length(rt: Runtime, a: ReadonlyArray<Val>): Val {
  const s = asStr('length', 'the string', a[0])
  const chunks = Math.ceil(utf8Bytes(s) / LENGTH_CHUNK)
  for (let k = 0; k < chunks; k++) rt.tick()
  let n = 0
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i)
    if (0xd800 <= c && c <= 0xdbff && i + 1 < s.length) {
      const d = s.charCodeAt(i + 1)
      if (0xdc00 <= d && d <= 0xdfff) i++
    }
    n++
  }
  return num(n)
}

// `compare a b`: how two numbers are ordered, `:less`, `:equal` or
// `:greater`, and `:unordered` when either is NaN; -0 and 0 are equal.
function compare(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const number = (v: Val, which: string): number => {
    if ('num' === v.v) return v.value
    throw typeError(`compare: the ${which} must be a number, not ${kindText(v)}`)
  }
  const x = number(a[0], 'first')
  const y = number(a[1], 'second')
  if (x < y) return keyword('less')
  if (x === y) return keyword('equal')
  if (x > y) return keyword('greater')
  return keyword('unordered')
}

// `number-class number`: `:finite`, `:infinity`, `:negative-infinity` or
// `:nan`, so that a program writing a format with spellings for the numbers
// JSON has none for (YAML's `.inf`, `-.inf` and `.nan`) can choose them;
// `scalar-text` refuses those numbers, as JSON and CSV must.
function numberClass(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const v = a[0]
  if ('num' !== v.v) throw typeError(`number-class: the number must be a number, not ${kindText(v)}`)
  if (Number.isNaN(v.value)) return keyword('nan')
  if (Infinity === v.value) return keyword('infinity')
  if (-Infinity === v.value) return keyword('negative-infinity')
  return keyword('finite')
}

// The word `kind` answers for a retained value; undefined for a stream or a
// text, live or finite, which `kind` refuses.
export function kindWord(v: Val): string | undefined {
  switch (v.v) {
    case 'null':
      return 'null'
    case 'bool':
      return 'boolean'
    case 'num':
      return 'number'
    case 'str':
      return 'string'
    case 'keyword':
      return 'keyword'
    case 'vector':
      return 'vector'
    case 'record':
      return 'record'
    case 'fn':
      return 'function'
    case 'selector':
      return 'selector'
    case 'capture':
      return 'capture'
    case 'tagged':
      return isMissing(v) ? 'missing' : 'tagged'
    case 'stream':
    case 'text':
      return undefined
  }
}

// The kind of a value as a keyword, so that a program can tell a string
// from a number, which no `match` pattern does.
function kind(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  // The checker refuses a stream or a text where `kind` is named; this is
  // where `kind` passed as a function meets one (`map kind [(text "x")]`),
  // and a finite text is refused as a live one is.
  const word = kindWord(a[0])
  if (undefined === word) {
    throw typeError(
      `kind: ${kindText(a[0])} cannot be asked; a stream or a text is used where it is, not inspected`,
    )
  }
  return keyword(word)
}

function path(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  let selector = Selector.root()
  for (const item of a) {
    if ('str' === item.v) {
      selector = selector.property(item.value)
      continue
    }
    if ('selector' === item.v) {
      selector = selector.compose(item.selector)
      continue
    }
    const i = asIndex(item)
    if (undefined === i) {
      throw typeError(
        `path: a segment must be a string, a non-negative integer or a selector, not ${kindText(item)}`,
      )
    }
    selector = selector.index(i)
  }
  return selectorVal(selector)
}

function root(): Val {
  return selectorVal(Selector.root())
}

function eachIndex(): Val {
  return selectorVal(Selector.root().eachIndex())
}

function eachMember(): Val {
  return selectorVal(Selector.root().eachMember())
}

function property(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  return selectorVal(Selector.root().property(asStr('property', 'the name', a[0])))
}

function index(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const i = asIndex(a[0])
  if (undefined === i) {
    throw typeError(`index: the position must be a non-negative integer, not ${debugText(a[0])}`)
  }
  return selectorVal(Selector.root().index(i))
}

function compose(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const first = asSelector('compose', 'the first selector', a[0])
  const second = asSelector('compose', 'the second selector', a[1])
  return selectorVal(first.compose(second))
}

// The `Limits` fields a capture may be bounded by, by the keyword that
// names each; the byte count is the host's, read when the plan is lowered
// (`captureBudget`).
export const CAPTURE_LIMITS: ReadonlyArray<string> = ['max_capture_bytes', 'max_metadata_bytes', 'max_record_bytes']

// The bytes the host's `limits` give the capture limit `name`.
export function captureBudget(limits: Limits, name: string): number | undefined {
  switch (name) {
    case 'max_capture_bytes':
      return limits.max_capture_bytes
    case 'max_metadata_bytes':
      return limits.max_metadata_bytes
    case 'max_record_bytes':
      return limits.max_record_bytes
    default:
      return undefined
  }
}

function capture(rt: Runtime, a: ReadonlyArray<Val>): Val {
  const tag = asKeyword('capture', 'the tag', a[0])
  const selector = asSelector('capture', 'the selector', a[1])
  const spec = CaptureSpec.materialize(tag, selector)
  if (undefined !== a[2]) {
    const limit = asKeyword('capture', 'the limit', a[2])
    if (!CAPTURE_LIMITS.includes(limit)) {
      throw typeError(`capture: the limit must be one of :${CAPTURE_LIMITS.join(', :')}, not :${limit}`)
    }
    spec.withBudget(captureBudget(rt.limits, limit) ?? Number.MAX_SAFE_INTEGER, limit)
  }
  return { v: 'capture', spec }
}

function route(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const v = a[0]
  if ('vector' !== v.v) throw typeError(`route: the captures must be a vector, not ${kindText(v)}`)
  const specs = v.items.map((item) => {
    if ('capture' === item.v) return item.spec
    throw typeError(`route: every capture must be a capture, not ${kindText(item)}`)
  })
  const source = asStream('route', a[1])
  return streamVal({ p: 'route', specs, source })
}

function select(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const selector = asSelector('select', 'the selector', a[0])
  const source = asStream('select', a[1])
  return streamVal({ p: 'select', selector, source })
}

function scanEmit(_rt: Runtime, a: ReadonlyArray<Val>, at: SourceSpan): Val {
  const step = asFn('scan-emit', 'the step', a[1])
  const finish = asFn('scan-emit', 'the finish', a[2])
  const source = asStream('scan-emit', a[3])
  return streamVal({ p: 'scan-emit', init: a[0], step, finish, source, at })
}

function transition(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  if ('vector' !== a[1].v) {
    throw typeError(`transition: the outputs must be a vector, not ${kindText(a[1])}`)
  }
  return taggedVal('transition', a.slice())
}

function partial(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const f = asFn('partial', 'the function', a[0])
  return fnVal({ fn: 'partial', partial: { f, args: a.slice(1) } })
}

function* mapVector(rt: Runtime, f: Func, items: ReadonlyArray<Val>, at: SourceSpan): G<Val> {
  const out: Val[] = []
  for (const item of items) out.push(yield rt.apply(f, [item], at))
  return vectorVal(out)
}

function map(rt: Runtime, a: ReadonlyArray<Val>, at: SourceSpan): Val | G<Val> {
  const f = asFn('map', 'the function', a[0])
  const seq = asSeq('map', a[1])
  if ('vector' === seq.seq) return mapVector(rt, f, seq.items, at)
  return streamVal({ p: 'map', f, source: seq.plan, at })
}

// The boolean an `if`, a `filter` or a `case` decides by; anything else is
// a type error, since nothing is implicitly true or false.
export function truth(op: string, v: Val): boolean {
  if ('bool' === v.v) return v.value
  throw typeError(`${op}: a boolean was expected, not ${kindText(v)}`)
}

function* filterVector(rt: Runtime, f: Func, items: ReadonlyArray<Val>, at: SourceSpan): G<Val> {
  const out: Val[] = []
  for (const item of items) {
    if (truth('filter', yield rt.apply(f, [item], at))) out.push(item)
  }
  return vectorVal(out)
}

function filter(rt: Runtime, a: ReadonlyArray<Val>, at: SourceSpan): Val | G<Val> {
  const f = asFn('filter', 'the predicate', a[0])
  const seq = asSeq('filter', a[1])
  if ('vector' === seq.seq) return filterVector(rt, f, seq.items, at)
  return streamVal({ p: 'filter', f, source: seq.plan, at })
}

function concatMap(_rt: Runtime, a: ReadonlyArray<Val>, at: SourceSpan): Val {
  const f = asFn('concat-map', 'the function', a[0])
  const items = asSeq('concat-map', a[1])
  return textVal({ p: 'concat-map', f, items, at })
}

function join(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const sep = asStr('join', 'the separator', a[0])
  const items = asSeq('join', a[1])
  if ('vector' === items.seq) {
    for (const item of items.items) textlike('join', item)
  }
  return textVal({ p: 'join', sep, items })
}

function concat(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  for (const item of a) textlike('concat', item)
  if (a.filter(isLiveVal).length > 1) {
    throw new Fail('STREAM_REUSED', 'reused: concat was given two live texts; the input is consumed once')
  }
  return textVal(concatPlan(a.slice()))
}

function text(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  return textVal({ p: 'lit', text: asStr('text', 'the argument', a[0]) })
}

function replaceText(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const from = asStr('replace-text', 'the literal to find', a[0])
  const to = asStr('replace-text', 'the replacement', a[1])
  textlike('replace-text', a[2])
  return textVal({ p: 'replace', from, to, source: a[2] })
}

// The text of a scalar cell under the options' policies: what the CSV
// renderer writes for the same cell, so that the interpreted and the native
// `csv` agree byte for byte.
function scalarText(rt: Runtime, a: ReadonlyArray<Val>): Val {
  const options = a[0]
  const cell = a[1]
  switch (cell.v) {
    case 'null':
      return str(asStr('scalar-text', ':null-text', option('scalar-text', options, 'null-text')))
    case 'bool':
      return str(cell.value ? 'true' : 'false')
    case 'num':
      return str(numberText(cell.value, cell.lexeme))
    case 'str':
      return cell
    case 'vector':
    case 'record':
      return str(rt.jsonText(cell))
    default: {
      if (isMissing(cell)) {
        const policy = option('scalar-text', options, 'missing')
        if ('keyword' === policy.v && 'error' === policy.name) {
          throw new Fail('MISSING_VALUE', 'a cell has no value and the options map missing to :error')
        }
        if ('str' === policy.v) return policy
        throw typeError(`scalar-text: :missing must be :error or a string, not ${kindText(policy)}`)
      }
      throw typeError(`scalar-text: a scalar was expected, not ${kindText(cell)}`)
    }
  }
}

// The escape `quote` writes for the code point `c`, or undefined when it
// is written as itself.
function quoteEscape(c: number): string | undefined {
  switch (c) {
    case 0x22:
      return '\\"'
    case 0x5c:
      return '\\\\'
    case 0x0a:
      return '\\n'
    case 0x09:
      return '\\t'
    case 0x0d:
      return '\\r'
    case 0x08:
      return '\\b'
    case 0x0c:
      return '\\f'
    default:
      return c < 0x20 || (0x7f <= c && c <= 0x9f) ? '\\u' + c.toString(16).padStart(4, '0') : undefined
  }
}

// The double-quoted form of `s`: the JSON string form (RFC 8259's escapes
// for the quote, the backslash and U+0000 to U+001F, the short ones where
// they exist, `\u00xx` otherwise, in the render package's lowercase) with
// U+007F to U+009F escaped the same way, since YAML's double-quoted style
// reads JSON's escapes but its printable set excludes the C1 controls.
// Every other character is written as itself.
export function quote(s: string): string {
  let out = '"'
  for (const c of s) {
    out += quoteEscape(c.codePointAt(0) as number) ?? c
  }
  return out + '"'
}

// The length of `quote`'s result in UTF-8 bytes, counted before it is
// built: two quotes, and each character as itself, as a two-byte escape,
// or as the six bytes of `\u00XX`.
export function quotedLen(s: string): number {
  let n = 2
  for (const c of s) {
    const escaped = quoteEscape(c.codePointAt(0) as number)
    n += undefined === escaped ? utf8Bytes(c) : escaped.length
  }
  return n
}

// `quoted string`: the double-quoted form. The result is one scalar of the
// output, up to six bytes per byte of the string, so it is held to
// `max_scalar_bytes`, refused before it is built.
function quoted(rt: Runtime, a: ReadonlyArray<Val>): Val {
  const s = asStr('quoted', 'the string', a[0])
  const max = rt.limits.max_scalar_bytes
  const len = quotedLen(s)
  if (len > max) {
    throw Fail.limit('max_scalar_bytes', max, `quoted: the quoted form is ${len} bytes, more than ${max}`)
  }
  return str(quote(s))
}

// `string-join separator strings`: the strings of a vector joined into one
// string, the separator between them. The result is one scalar, so it is
// held to `max_scalar_bytes`, refused before it is built.
function stringJoin(rt: Runtime, a: ReadonlyArray<Val>): Val {
  const separator = asStr('string-join', 'the separator', a[0])
  const items = asItems('string-join', a[1])
  const sepBytes = utf8Bytes(separator)
  let len = 0
  const parts: string[] = []
  items.forEach((item, i) => {
    if ('str' !== item.v) {
      throw typeError(`string-join: every item must be a string, not ${kindText(item)}`)
    }
    len += utf8Bytes(item.value) + (i > 0 ? sepBytes : 0)
    parts.push(item.value)
  })
  const max = rt.limits.max_scalar_bytes
  if (len > max) {
    throw Fail.limit('max_scalar_bytes', max, `string-join: the joined string is ${len} bytes, more than ${max}`)
  }
  return str(parts.join(separator))
}

// `repeat count string`: the string `count` times over. The result is one
// scalar of the output (a line's indentation), so it is held to
// `max_scalar_bytes`, refused before it is built.
function repeat(rt: Runtime, a: ReadonlyArray<Val>): Val {
  // The count is judged before it is narrowed, so that any whole
  // non-negative count past the limit is the limit's refusal, and only a
  // negative or a fractional one is a type error.
  const c = a[0]
  if ('num' !== c.v) throw typeError(`repeat: the count must be a number, not ${kindText(c)}`)
  const count = c.value
  if (!(count >= 0 && 0 === count % 1)) {
    const spelled = undefined !== c.lexeme ? c.lexeme : displayNumber(count)
    throw typeError(`repeat: the count must be a whole number of at least 0, not ${spelled}`)
  }
  const s = asStr('repeat', 'the string', a[1])
  const max = rt.limits.max_scalar_bytes
  const bytes = utf8Bytes(s)
  if (count * bytes > max) {
    throw Fail.limit('max_scalar_bytes', max, `repeat: ${displayNumber(count)} times ${bytes} bytes is more than ${max}`)
  }
  // Within the limit, the count is small, or the string is empty.
  return str(0 === bytes ? '' : s.repeat(count))
}

function fail(rt: Runtime, a: ReadonlyArray<Val>, at: SourceSpan): Val {
  // `(fail message)` is INPUT_INVALID, the code of a document the program
  // refuses; `(fail :code message)` names what the failure is: a value the
  // target cannot carry, or a stream that breaks its protocol.
  let code: Code
  let message: Val
  if (1 === a.length) {
    code = 'INPUT_INVALID'
    message = a[0]
  } else if (2 === a.length) {
    const k = a[0]
    if ('keyword' !== k.v) throw typeError(`fail: the code must be a keyword, not ${kindText(k)}`)
    switch (k.name) {
      case 'invalid':
        code = 'INPUT_INVALID'
        break
      case 'unrepresentable':
        code = 'TARGET_VALUE_UNREPRESENTABLE'
        break
      case 'protocol-order':
        code = 'PROTOCOL_ORDER_ERROR'
        break
      default:
        throw typeError(`the code of fail must be :invalid, :unrepresentable or :protocol-order, not :${k.name}`)
    }
    message = a[1]
  } else {
    throw typeError('fail takes a message, or a code and a message')
  }
  const text = asStr('fail', 'the message', message)
  throw rt.failAt(new Fail(code, text), at)
}

function isReadyValue(v: Val): boolean {
  return 'tagged' === v.v && 'ready' === v.tag && 1 === v.fields.length
}

function isReady(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  return bool(isReadyValue(a[0]))
}

function requireColumns(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const v = a[0]
  if ('tagged' === v.v && 'ready' === v.tag && 1 === v.fields.length) return v.fields[0]
  throw new Fail(
    'INPUT_ORDER_VIOLATION',
    'a row began before the column metadata had completed; rows must follow their metadata',
  )
}

// A constructor: the tagged value with the operator's name as its tag.
function constructor(name: string): NativeImpl {
  return (_rt, a) => taggedVal(name, a.slice())
}

// `schema columns`: the table's one schema. A table has at most
// `max_columns` columns, so a schema past it is refused where it is built,
// whatever consumes it: the native table binds no more, and the library's
// twin of it, whose events may reach no renderer that would check them, is
// held to the same count.
function schema(rt: Runtime, a: ReadonlyArray<Val>): Val {
  const columns = a[0]
  if ('vector' === columns.v) {
    const max = rt.limits.max_columns
    if (columns.items.length > max) {
      throw Fail.limit('max_columns', max, `the schema declares ${columns.items.length} columns, more than ${max}`)
    }
  }
  return taggedVal('schema', a.slice())
}

function key(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  asStr('key', 'the name', a[0])
  return taggedVal('key', a.slice())
}

function scalar(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const v = a[0]
  if ('null' === v.v || 'bool' === v.v || 'num' === v.v || 'str' === v.v) return taggedVal('scalar', a.slice())
  throw typeError(`scalar: the value must be null, a boolean, a number or a string, not ${kindText(v)}`)
}

function events(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  return streamVal({ p: 'events', source: asStream('events', a[0]) })
}

function asEvents(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  return streamVal({ p: 'as-events', source: asStream('as-events', a[0]) })
}

function csvTable(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  const source = asStream('csv-table', a[1])
  return streamVal({ p: 'csv-table', options: a[0], source })
}

function json(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  return textVal({ p: 'json', source: asStream('json', a[0]) })
}

function records(_rt: Runtime, a: ReadonlyArray<Val>): Val {
  return streamVal({ p: 'records', source: asStream('records', a[0]) })
}

// Every native's implementation, by its name in the table.
const IMPLS: Record<string, NativeImpl> = {
  get,
  'get-path': getPath,
  'as-path': asPath,
  'as-vector': asVector,
  record,
  entry,
  vector,
  push,
  pop,
  top,
  count,
  keys,
  length,
  number,
  compare,
  'number-class': numberClass,
  kind,
  path,
  root,
  'each-index': eachIndex,
  'each-member': eachMember,
  property,
  index,
  compose,
  capture,
  route,
  select,
  events,
  indices,
  'as-events': asEvents,
  'scan-emit': scanEmit,
  transition,
  partial,
  map,
  filter,
  'concat-map': concatMap,
  join,
  concat,
  text,
  'replace-text': replaceText,
  'scalar-text': scalarText,
  quoted,
  'string-join': stringJoin,
  repeat,
  fail,
  'is-ready': isReady,
  'require-columns': requireColumns,
  schema,
  row: constructor('row'),
  ready: constructor('ready'),
  selected: constructor('selected'),
  'table-end': constructor('table-end'),
  'no-schema': constructor('no-schema'),
  missing: constructor('missing'),
  'object-start': constructor('object-start'),
  'object-end': constructor('object-end'),
  'array-start': constructor('array-start'),
  'array-end': constructor('array-end'),
  key,
  scalar,
  json,
  records,
  'csv-table': csvTable,
}

installCalls(IMPLS)

// The implementation of a native. Every native in the table has one (a
// test holds the two lists together), so a missing one is a defect of
// this package.
export function implOf(n: Native): NativeImpl {
  const impl = call(n.name)
  if ('function' !== typeof impl) throw new Error(`the native ${n.name} has no implementation`)
  return impl as NativeImpl
}

// The natives the table lists without an implementation here: none.
export function unimplemented(): string[] {
  return natives()
    .map((n) => n.name)
    .filter((name) => 'function' !== typeof call(name))
}
