/* Copyright (c) 2026 tabnas, MIT License */

// Runtime values: the data model of rs/src/value.rs. Streams and texts are
// plans: evaluating `route`, `scan-emit`, `map` over a stream, `concat`,
// `join` or `csv` builds a `Plan` and consumes nothing; the runtime lowers
// the plan to a chain of transduce and render sinks once, when the host
// asks for the program's sink (`./lower`). A plan is live when it reaches
// the host's input (`input`); a finite text (a literal, a join over a
// vector) is a plan too, written whole wherever it lands.
//
// A transduce `Datum` converts to a `Val` and back, so `get`, `get-path`,
// `as-vector` and `as-path` work on captured JSON values and on a
// program's own records alike. A value the source had no member for is
// the constant tagged value `missing`, distinct from `null`, as the spec
// asks (section 18.3); a policy maps it later.
//
// Values are immutable once built, and a `Val` is shared freely: two
// references to one vector are the same retained value (`same`). Every walk
// over a value here (equality, conversion, the JSON text, the debug text)
// is iterative, on a stack of its own: how deep a value nests is bounded by
// the evaluation depth and the host's `max_depth`, not by JavaScript's
// stack.

import { CaptureSpec, Datum, Fail, Segment, Selector, jsonNumber, jsonString } from './shared'

import { Expr, SourceSpan } from './ast'
import { Native, arityExact } from './stdlib/registry'

// The tag of the constant tagged value a lookup answers for an absent
// member.
export const MISSING = 'missing'

// One runtime value.
export type Val =
  | { readonly v: 'null' }
  | { readonly v: 'bool'; readonly value: boolean }
  // A number as the source or the program spelled it, when known: a
  // renderer writes the lexeme, so `50.25` stays `50.25`.
  | { readonly v: 'num'; readonly value: number; readonly lexeme?: string }
  | { readonly v: 'str'; readonly value: string }
  | { readonly v: 'keyword'; readonly name: string }
  | { readonly v: 'vector'; readonly items: ReadonlyArray<Val> }
  // Fields in construction order; keys are the keyword names.
  | { readonly v: 'record'; readonly fields: ReadonlyMap<string, Val> }
  | { readonly v: 'fn'; readonly f: Func }
  | { readonly v: 'selector'; readonly selector: Selector }
  | { readonly v: 'capture'; readonly spec: CaptureSpec }
  // A constructor's value: `schema`, `row`, `table-end`, `no-schema`,
  // `ready`, `selected`, `transition`, `entry`, `missing`, and the events
  // `events` delivers (`object-start`, `object-end`, `array-start`,
  // `array-end`, `key`, `scalar`).
  | { readonly v: 'tagged'; readonly tag: string; readonly fields: ReadonlyArray<Val> }
  // A single-use stream: a plan producing items, table events or JSON
  // events.
  | { readonly v: 'stream'; readonly plan: Plan }
  // A single-use text: a plan producing fragments.
  | { readonly v: 'text'; readonly plan: Plan }

// Which global scope a closure's free names resolve in: a standard library
// definition sees the library and the natives, a program's definition sees
// the program first. Lexical, so a program's `csv-row` never hijacks the
// library's `csv`.
export type Scope = 'program' | 'stdlib'

// The local environment: a persistent chain of bindings, shared by the
// closures that captured it; `null` is the empty one.
export type Env = EnvFrame | null

export type EnvFrame = {
  readonly name: string
  readonly value: Val
  readonly next: Env
}

// `env` with one more binding, innermost.
export function bind(env: Env, name: string, value: Val): Env {
  return { name, value, next: env }
}

// The innermost binding of `name`, or undefined.
export function envGet(env: Env, name: string): Val | undefined {
  for (let here = env; null !== here; here = here.next) {
    if (here.name === name) return here.value
  }
  return undefined
}

// Whether `name` is bound in `env`.
export function envHas(env: Env, name: string): boolean {
  return undefined !== envGet(env, name)
}

// The names bound in `env`, innermost first, for messages and tests.
export function envNames(env: Env): string[] {
  const names: string[] = []
  for (let here = env; null !== here; here = here.next) names.push(here.name)
  return names
}

// A `fn` value: its parameters, its body and the environment it closed
// over. `name` is the `def` it came from, for messages.
export type Closure = {
  readonly name?: string
  readonly params: ReadonlyArray<string>
  readonly body: Expr
  readonly env: Env
  readonly scope: Scope
  readonly span: SourceSpan
}

// `partial f a b`: `f` with its first arguments supplied.
export type Partial = { readonly f: Func; readonly args: ReadonlyArray<Val> }

// A function value.
export type Func =
  | { readonly fn: 'closure'; readonly closure: Closure }
  | { readonly fn: 'native'; readonly native: Native }
  | { readonly fn: 'partial'; readonly partial: Partial }

// The arguments the function still takes, when that is fixed.
export function funcArity(f: Func): number | undefined {
  switch (f.fn) {
    case 'closure':
      return f.closure.params.length
    case 'native':
      return arityExact(f.native.arity)
    case 'partial': {
      const n = funcArity(f.partial.f)
      return undefined === n ? undefined : Math.max(0, n - f.partial.args.length)
    }
  }
}

// A name for messages: `fn name`, `fn [a b]`, a native's name, `partial
// ...`.
export function funcDescribe(f: Func): string {
  let prefix = ''
  let here = f
  while ('partial' === here.fn) {
    prefix += 'partial '
    here = here.partial.f
  }
  if ('native' === here.fn) return prefix + here.native.name
  const c = here.closure
  return prefix + (undefined !== c.name ? `fn ${c.name}` : `fn [${c.params.join(' ')}]`)
}

// A finite or single-use sequence an operator was given.
export type Seq =
  | { readonly seq: 'vector'; readonly items: ReadonlyArray<Val> }
  | { readonly seq: 'stream'; readonly plan: Plan }

// A stream or text, as a description of how to produce it.
export type Plan =
  // The host's `JsonEvents/1`.
  | { readonly p: 'input' }
  // `route specs input`: a stream of `selected` values.
  | { readonly p: 'route'; readonly specs: ReadonlyArray<CaptureSpec>; readonly source: Plan }
  // `select selector input`: a stream of the selected values.
  | { readonly p: 'select'; readonly selector: Selector; readonly source: Plan }
  // `events input`: every event of `JsonEvents/1` as one tagged item, the
  // container events as constants and `key` and `scalar` with their one
  // field; `end` is the stream's end, not an item.
  | { readonly p: 'events'; readonly source: Plan }
  // `as-events items`: a stream of items the program built, each an
  // event, as `JsonEvents/1`: the reverse of `events`, which any taker of
  // JSON events already applies to such a stream; here the program says
  // so, where the checker could not tell the items' type.
  | { readonly p: 'as-events'; readonly source: Plan }
  // `scan-emit init step finish stream`.
  | {
      readonly p: 'scan-emit'
      readonly init: Val
      readonly step: Func
      readonly finish: Func
      readonly source: Plan
      readonly at: SourceSpan
    }
  | { readonly p: 'map'; readonly f: Func; readonly source: Plan; readonly at: SourceSpan }
  | { readonly p: 'filter'; readonly f: Func; readonly source: Plan; readonly at: SourceSpan }
  // The standard table transducer, run natively: `TableRows/1` from
  // `JsonEvents/1`.
  | { readonly p: 'table-from-json'; readonly binding: Val; readonly source: Plan; readonly at: SourceSpan }
  // `records table-events`: `JsonEvents/1` from `TableRows/1`.
  | { readonly p: 'records'; readonly source: Plan }
  // `csv-table options events`: the tagged table events, validated as the
  // CSV renderer validates them, passed on unchanged.
  | { readonly p: 'csv-table'; readonly options: Val; readonly source: Plan }
  // A literal text.
  | { readonly p: 'lit'; readonly text: string }
  // `concat items...`: each a string or a text; `live` is the position of
  // the one item that reaches the input, found once when the plan is built
  // (`concatPlan`), so asking whether a concat is live never walks its
  // items again: a program that nests `concat` over shared definitions
  // builds a plan whose items, unshared, are exponentially many.
  | { readonly p: 'concat'; readonly items: ReadonlyArray<Val>; readonly live?: number }
  // `join separator items`.
  | { readonly p: 'join'; readonly sep: string; readonly items: Seq }
  // `concat-map f items`: `f` answers a string or a text per item.
  | { readonly p: 'concat-map'; readonly f: Func; readonly items: Seq; readonly at: SourceSpan }
  // `replace-text from to text`: `source` is a string or a text.
  | { readonly p: 'replace'; readonly from: string; readonly to: string; readonly source: Val }
  // The standard CSV renderer, run natively.
  | { readonly p: 'csv'; readonly options: Val; readonly source: Plan }
  // `json events`.
  | { readonly p: 'json'; readonly source: Plan }

// What a plan produces: `JsonEvents`, `TableRows` (natively), or `Items`
// for a stream of values whose shape the runtime learns item by item.
export type Protocol = 'JsonEvents' | 'TableRows' | 'Items' | 'Text'

// A `concat` of `items`, with its live item found once.
export function concatPlan(items: ReadonlyArray<Val>): Plan {
  const live = items.findIndex(isLiveVal)
  return -1 === live ? { p: 'concat', items } : { p: 'concat', items, live }
}

// Whether the plan reaches the host's input: a live plan runs as the
// events arrive; a plan that does not is finite and is written whole.
// Iterative along the chain of sources.
export function isLive(plan: Plan): boolean {
  let here: Plan = plan
  for (;;) {
    switch (here.p) {
      case 'input':
        return true
      case 'route':
      case 'select':
      case 'events':
      case 'as-events':
      case 'scan-emit':
      case 'map':
      case 'filter':
      case 'table-from-json':
      case 'records':
      case 'csv-table':
      case 'csv':
      case 'json':
        here = here.source
        continue
      case 'lit':
        return false
      case 'concat':
        return undefined !== here.live
      case 'join':
      case 'concat-map':
        return 'stream' === here.items.seq
      case 'replace': {
        const source = here.source
        if ('stream' !== source.v && 'text' !== source.v) return false
        here = source.plan
        continue
      }
    }
  }
}

// Whether the plan is a text rather than a stream.
export function isText(plan: Plan): boolean {
  switch (plan.p) {
    case 'lit':
    case 'concat':
    case 'join':
    case 'concat-map':
    case 'replace':
    case 'csv':
    case 'json':
      return true
    default:
      return false
  }
}

// The protocol the plan produces, when it is a stream.
export function protocol(plan: Plan): Protocol {
  switch (plan.p) {
    case 'input':
    case 'records':
    case 'as-events':
      return 'JsonEvents'
    case 'table-from-json':
      return 'TableRows'
    case 'route':
    case 'select':
    case 'events':
    case 'scan-emit':
    case 'map':
    case 'filter':
    case 'csv-table':
      return 'Items'
    default:
      return 'Text'
  }
}

// The operator at the root of a plan, for messages.
export function planName(plan: Plan): string {
  switch (plan.p) {
    case 'lit':
      return 'text'
    case 'replace':
      return 'replace-text'
    default:
      return plan.p
  }
}

export const NULL: Val = Object.freeze({ v: 'null' })
export const TRUE: Val = Object.freeze({ v: 'bool', value: true })
export const FALSE: Val = Object.freeze({ v: 'bool', value: false })

export function bool(value: boolean): Val {
  return value ? TRUE : FALSE
}

export function str(value: string): Val {
  return { v: 'str', value }
}

export function keyword(name: string): Val {
  return { v: 'keyword', name }
}

export function num(value: number, lexeme?: string): Val {
  return undefined === lexeme ? { v: 'num', value } : { v: 'num', value, lexeme }
}

export function vectorVal(items: ReadonlyArray<Val>): Val {
  return { v: 'vector', items }
}

export function record(fields: ReadonlyMap<string, Val> | ReadonlyArray<readonly [string, Val]>): Val {
  return { v: 'record', fields: fields instanceof Map ? fields : new Map(fields as Array<[string, Val]>) }
}

export function taggedVal(tag: string, fields: ReadonlyArray<Val> = []): Val {
  return { v: 'tagged', tag, fields }
}

export function fnVal(f: Func): Val {
  return { v: 'fn', f }
}

export function selectorVal(selector: Selector): Val {
  return { v: 'selector', selector }
}

export function streamVal(plan: Plan): Val {
  return { v: 'stream', plan }
}

export function textVal(plan: Plan): Val {
  return { v: 'text', plan }
}

// The constant a lookup answers for an absent member.
export function missing(): Val {
  return taggedVal(MISSING)
}

export function isMissing(val: Val): boolean {
  return 'tagged' === val.v && MISSING === val.tag && 0 === val.fields.length
}

// Whether this value holds a live stream or text: the one-shot resources a
// vector may not hide and a `fn` may not capture.
export function isLiveVal(val: Val): boolean {
  return ('stream' === val.v || 'text' === val.v) && isLive(val.plan)
}

// Whether two values are the same retained value, not merely equal: one
// allocation behind both. A scalar is never the same as anything, which
// only costs a re-measure where this is asked.
export function same(a: Val, b: Val): boolean {
  switch (a.v) {
    case 'vector':
      return 'vector' === b.v && a.items === b.items
    case 'record':
      return 'record' === b.v && a.fields === b.fields
    case 'tagged':
      return 'tagged' === b.v && a.fields === b.fields
    case 'selector':
      return 'selector' === b.v && a.selector === b.selector
    case 'stream':
    case 'text':
      return a.v === b.v && a.plan === (b as typeof a).plan
    default:
      return false
  }
}

// A record's field, `missing` when the record has none; undefined for a
// value that is not a record.
export function field(val: Val, key: string): Val | undefined {
  return 'record' === val.v ? (val.fields.get(key) ?? missing()) : undefined
}

// A one-word name of the value's kind, for messages.
export function kindText(val: Val): string {
  switch (val.v) {
    case 'null':
      return 'null'
    case 'bool':
      return 'a boolean'
    case 'num':
      return 'a number'
    case 'str':
      return 'a string'
    case 'keyword':
      return 'a keyword'
    case 'vector':
      return 'a vector'
    case 'record':
      return 'a record'
    case 'fn':
      return 'a function'
    case 'selector':
      return 'a selector'
    case 'capture':
      return 'a capture'
    case 'tagged':
      return isMissing(val) ? 'missing' : 'a tagged value'
    case 'stream':
      return 'a stream'
    case 'text':
      return 'a text'
  }
}

// What a live value is, for a message that refuses to hold it: a text is
// named live, since a finite one could have been held.
export function liveKind(val: Val): string {
  return 'text' === val.v ? 'a live text' : kindText(val)
}

// A `DSL_TYPE_ERROR` found while running, with the `type_mismatch` finer
// code the checker also uses.
export function typeError(message: string): Fail {
  return new Fail('DSL_TYPE_ERROR', `type_mismatch: ${message}`)
}

// From a retained transduce value. Numbers keep their lexemes. Iterative.
export function fromDatum(d: Datum): Val {
  type Open = { d: Datum; items: Val[]; keys: string[]; next: number; all: Datum[] }
  const leaf = (x: Datum): Val | undefined => {
    switch (x.type) {
      case 'null':
        return NULL
      case 'bool':
        return bool(x.value)
      case 'number':
        return null == x.lexeme ? num(x.value) : num(x.value, x.lexeme)
      case 'string':
        return str(x.value)
      default:
        return undefined
    }
  }
  const open = (x: Datum): Open =>
    'array' === x.type
      ? { d: x, items: [], keys: [], next: 0, all: x.items }
      : {
          d: x,
          items: [],
          keys: [...(x as Extract<Datum, { type: 'object' }>).members.keys()],
          next: 0,
          all: [...(x as Extract<Datum, { type: 'object' }>).members.values()],
        }
  const close = (o: Open): Val =>
    'array' === o.d.type
      ? vectorVal(o.items)
      : record(new Map(o.keys.map((k, i) => [k, o.items[i]] as [string, Val])))
  const root = leaf(d)
  if (undefined !== root) return root
  const stack: Open[] = [open(d)]
  for (;;) {
    const top = stack[stack.length - 1]
    if (top.next < top.all.length) {
      const child = top.all[top.next++]
      const v = leaf(child)
      if (undefined !== v) top.items.push(v)
      else stack.push(open(child))
      continue
    }
    stack.pop()
    const built = close(top)
    if (0 === stack.length) return built
    stack[stack.length - 1].items.push(built)
  }
}

// One node of an iterative walk that turns a value into data: its
// children in order, and how to finish it.
type DataOpen = { val: Val; children: ReadonlyArray<Val>; keys?: string[]; next: number; done: Datum[] }

// As a retained transduce value: the data kinds convert, a keyword becomes
// its name, `missing` becomes `null`, and a function, a selector, a
// capture, a stream or a text has no data form. Iterative.
export function toDatum(v: Val): Datum {
  const leaf = (x: Val): Datum | undefined => {
    switch (x.v) {
      case 'null':
        return Datum.null
      case 'bool':
        return Datum.bool(x.value)
      case 'num':
        return Datum.number(x.value, x.lexeme ?? null)
      case 'str':
        return Datum.string(x.value)
      case 'keyword':
        return Datum.string(x.name)
      case 'vector':
      case 'record':
        return undefined
      default:
        if (isMissing(x)) return Datum.null
        throw typeError(`${kindText(x)} has no data form`)
    }
  }
  const open = (x: Val): DataOpen =>
    'vector' === x.v
      ? { val: x, children: x.items, next: 0, done: [] }
      : {
          val: x,
          children: [...(x as Extract<Val, { v: 'record' }>).fields.values()],
          keys: [...(x as Extract<Val, { v: 'record' }>).fields.keys()],
          next: 0,
          done: [],
        }
  const root = leaf(v)
  if (undefined !== root) return root
  const stack: DataOpen[] = [open(v)]
  for (;;) {
    const top = stack[stack.length - 1]
    if (top.next < top.children.length) {
      const child = top.children[top.next++]
      const d = leaf(child)
      if (undefined !== d) top.done.push(d)
      else stack.push(open(child))
      continue
    }
    stack.pop()
    const built =
      undefined === top.keys
        ? Datum.array(top.done)
        : Datum.object(new Map(top.keys.map((k, i) => [k, top.done[i]] as [string, Datum])))
    if (0 === stack.length) return built
    stack[stack.length - 1].done.push(built)
  }
}

// The compact JSON text of a data value, keeping number lexemes: what
// `toDatum` then transduce's `toText` would write, written without
// building the datum and without recursing. A value with no data form is
// the same `type_mismatch` `toDatum` raises, for the first one in the
// text's order.
export function toJsonText(v: Val): string {
  const parts: string[] = []
  // Work items: a value to write, or a literal to emit.
  const pending: Array<Val | string> = [v]
  while (0 < pending.length) {
    const next = pending.pop() as Val | string
    if ('string' === typeof next) {
      parts.push(next)
      continue
    }
    switch (next.v) {
      case 'null':
        parts.push('null')
        break
      case 'bool':
        parts.push(next.value ? 'true' : 'false')
        break
      case 'num':
        parts.push(jsonNumber(next.value, next.lexeme ?? null))
        break
      case 'str':
        parts.push(jsonString(next.value))
        break
      case 'keyword':
        parts.push(jsonString(next.name))
        break
      case 'vector': {
        parts.push('[')
        pending.push(']')
        for (let i = next.items.length - 1; i >= 0; i--) {
          pending.push(next.items[i])
          if (i > 0) pending.push(',')
        }
        break
      }
      case 'record': {
        parts.push('{')
        pending.push('}')
        const entries = [...next.fields]
        for (let i = entries.length - 1; i >= 0; i--) {
          pending.push(entries[i][1])
          pending.push(jsonString(entries[i][0]) + ':')
          if (i > 0) pending.push(',')
        }
        break
      }
      default:
        if (isMissing(next)) {
          parts.push('null')
          break
        }
        throw typeError(`${kindText(next)} has no data form`)
    }
  }
  return parts.join('')
}

// The value at a concrete path below `v`, walking records by key and
// vectors by index; `missing` where the path leaves the data.
export function getPath(v: Val, segments: ReadonlyArray<Segment>): Val {
  let here = v
  for (const segment of segments) {
    if ('string' === typeof segment) {
      if ('record' !== here.v) return missing()
      const next = here.fields.get(segment)
      if (undefined === next) return missing()
      here = next
    } else {
      if ('vector' !== here.v) return missing()
      const next = here.items[segment]
      if (undefined === next) return missing()
      here = next
    }
  }
  return here
}

// A selector that names one location, as segments; undefined when it names
// many (`each-index`, `each-member`).
export function selectorSegments(selector: Selector): Segment[] | undefined {
  const out: Segment[] = []
  for (const step of selector.steps) {
    switch (step.type) {
      case 'property':
        out.push(step.name)
        break
      case 'index':
        out.push(step.index)
        break
      default:
        return undefined
    }
  }
  return out
}

function captureEquals(a: CaptureSpec, b: CaptureSpec): boolean {
  return (
    a.tag === b.tag &&
    a.mode === b.mode &&
    a.selector.equals(b.selector) &&
    (a.budget?.bytes ?? null) === (b.budget?.bytes ?? null) &&
    (a.budget?.name ?? null) === (b.budget?.name ?? null)
  )
}

// Structural equality of data; numbers by value (so NaN equals nothing and
// -0 equals 0); functions, streams and texts never equal, not even
// themselves. Iterative.
export function valEquals(a: Val, b: Val): boolean {
  const pending: Array<[Val, Val]> = [[a, b]]
  while (0 < pending.length) {
    const [x, y] = pending.pop() as [Val, Val]
    switch (x.v) {
      case 'null':
        if ('null' !== y.v) return false
        break
      case 'bool':
        if ('bool' !== y.v || x.value !== y.value) return false
        break
      case 'num':
        if ('num' !== y.v || !(x.value === y.value)) return false
        break
      case 'str':
        if ('str' !== y.v || x.value !== y.value) return false
        break
      case 'keyword':
        if ('keyword' !== y.v || x.name !== y.name) return false
        break
      case 'vector':
        if ('vector' !== y.v || x.items.length !== y.items.length) return false
        for (let i = x.items.length - 1; i >= 0; i--) pending.push([x.items[i], y.items[i]])
        break
      case 'record': {
        if ('record' !== y.v || x.fields.size !== y.fields.size) return false
        const pairs: Array<[Val, Val]> = []
        for (const [k, xv] of x.fields) {
          const yv = y.fields.get(k)
          if (undefined === yv) return false
          pairs.push([xv, yv])
        }
        for (let i = pairs.length - 1; i >= 0; i--) pending.push(pairs[i])
        break
      }
      case 'selector':
        if ('selector' !== y.v || !x.selector.equals(y.selector)) return false
        break
      case 'capture':
        if ('capture' !== y.v || !captureEquals(x.spec, y.spec)) return false
        break
      case 'tagged':
        if ('tagged' !== y.v || x.tag !== y.tag || x.fields.length !== y.fields.length) return false
        for (let i = x.fields.length - 1; i >= 0; i--) pending.push([x.fields[i], y.fields[i]])
        break
      default:
        return false
    }
  }
  return true
}

// A number without a lexeme as Rust's `f64` `Display` writes it: the
// shortest digits, positionally, `NaN`, `inf` and `-inf`.
export function displayNumber(value: number): string {
  if (Number.isNaN(value)) return 'NaN'
  if (Infinity === value) return 'inf'
  if (-Infinity === value) return '-inf'
  return jsonNumber(value, null)
}

// A string as Rust's `Debug` writes it, for messages: quoted, with the
// quote, the backslash, and the common controls escaped, and every other
// control character as `\u{..}`.
function debugString(s: string): string {
  let out = '"'
  for (const c of s) {
    switch (c) {
      case '"':
        out += '\\"'
        break
      case '\\':
        out += '\\\\'
        break
      case '\n':
        out += '\\n'
        break
      case '\r':
        out += '\\r'
        break
      case '\t':
        out += '\\t'
        break
      case '\0':
        out += '\\0'
        break
      default: {
        const code = c.codePointAt(0) as number
        out += code < 0x20 || (code >= 0x7f && code <= 0x9f) ? `\\u{${code.toString(16)}}` : c
      }
    }
  }
  return out + '"'
}

// The debug text of a function, as rs/src/value.rs writes `<fn ...>`.
function debugFunc(f: Func): string {
  return `<${funcDescribe(f)}>`
}

// The value as Rust's `Debug` writes it (rs/src/value.rs): `null`, a
// number by its lexeme, a string quoted, `:keyword`, `[a b]`, `(record
// (entry :k v))`, `(tag f ...)`, `<fn ...>`, `<selector ...>`, `<stream
// ...>`. Iterative, and it stops once the text is longer than `max`
// characters when one is given: what `brief` needs is a prefix.
export function debugText(v: Val, max?: number): string {
  let out = ''
  let chars = 0
  const limit = undefined === max ? Infinity : max
  const pending: Array<Val | string> = [v]
  const put = (s: string) => {
    out += s
    chars += s.length
  }
  while (0 < pending.length && chars <= limit) {
    const next = pending.pop() as Val | string
    if ('string' === typeof next) {
      put(next)
      continue
    }
    switch (next.v) {
      case 'null':
        put('null')
        break
      case 'bool':
        put(next.value ? 'true' : 'false')
        break
      case 'num':
        put(undefined !== next.lexeme ? next.lexeme : displayNumber(next.value))
        break
      case 'str':
        put(debugString(next.value))
        break
      case 'keyword':
        put(':' + next.name)
        break
      case 'vector':
        put('[')
        pending.push(']')
        for (let i = next.items.length - 1; i >= 0; i--) {
          pending.push(next.items[i])
          if (i > 0) pending.push(' ')
        }
        break
      case 'record': {
        put('(record')
        pending.push(')')
        const entries = [...next.fields]
        for (let i = entries.length - 1; i >= 0; i--) {
          pending.push(')')
          pending.push(entries[i][1])
          pending.push(` (entry :${entries[i][0]} `)
        }
        break
      }
      case 'fn':
        put(debugFunc(next.f))
        break
      case 'selector':
        put(`<selector ${next.selector}>`)
        break
      case 'capture':
        put(`<capture :${next.spec.tag} ${next.spec.selector}>`)
        break
      case 'tagged':
        if (0 === next.fields.length) {
          put(next.tag)
          break
        }
        put('(' + next.tag)
        pending.push(')')
        for (let i = next.fields.length - 1; i >= 0; i--) {
          pending.push(next.fields[i])
          pending.push(' ')
        }
        break
      case 'stream':
        put(`<stream ${planName(next.plan)}>`)
        break
      case 'text':
        put(`<text ${planName(next.plan)}>`)
        break
    }
  }
  return out
}
