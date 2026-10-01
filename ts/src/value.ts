/* Copyright (c) 2026 tabnas, MIT License */

// Runtime values: the data model of rs/src/value.rs. Streams and texts are
// plans: evaluating `route`, `scan-emit`, `map` over a stream, `concat`,
// `join` or `csv` builds a `Plan` and consumes nothing; the runtime lowers
// the plan to sinks once. A plan is live when it reaches the host's input
// (`Input`); a finite text is a plan too, written whole.
//
// The front end owns this model because the plan report (`./effects`)
// reads it. Pass 2 of the port (the evaluator in `./interp`, the lowering
// in `./lower`) builds and runs these values; nothing here evaluates.

import type { CaptureSpec, Selector } from '@tabnas/transduce'

import { Expr, SourceSpan } from './ast'
import { Native } from './stdlib/registry'

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
  // `events` delivers.
  | { readonly v: 'tagged'; readonly tag: string; readonly fields: ReadonlyArray<Val> }
  // A single-use stream: a plan producing items, table events or JSON
  // events.
  | { readonly v: 'stream'; readonly plan: Plan }
  // A single-use text: a plan producing fragments.
  | { readonly v: 'text'; readonly plan: Plan }

// Which global scope a closure's free names resolve in.
export type Scope = 'program' | 'stdlib'

// A local environment, as the evaluator keeps it; opaque to the front end.
export type Env = unknown

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
  // `events input`: every event of `JsonEvents/1` as one tagged item.
  | { readonly p: 'events'; readonly source: Plan }
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
  // The standard table transducer, run natively.
  | { readonly p: 'table-from-json'; readonly binding: Val; readonly source: Plan; readonly at: SourceSpan }
  // `records table-events`: `JsonEvents/1` from `TableRows/1`.
  | { readonly p: 'records'; readonly source: Plan }
  // `csv-table options events`.
  | { readonly p: 'csv-table'; readonly options: Val; readonly source: Plan }
  // A literal text.
  | { readonly p: 'lit'; readonly text: string }
  // `concat items...`; `live` is the position of the one item that reaches
  // the input, found once when the plan is built (`concat`).
  | { readonly p: 'concat'; readonly items: ReadonlyArray<Val>; readonly live?: number }
  // `join separator items`.
  | { readonly p: 'join'; readonly sep: string; readonly items: Seq }
  // `concat-map f items`.
  | { readonly p: 'concat-map'; readonly f: Func; readonly items: Seq; readonly at: SourceSpan }
  // `replace-text from to text`: `source` is a string or a text.
  | { readonly p: 'replace'; readonly from: string; readonly to: string; readonly source: Val }
  // The standard CSV renderer, run natively.
  | { readonly p: 'csv'; readonly options: Val; readonly source: Plan }
  // `json events`.
  | { readonly p: 'json'; readonly source: Plan }

// What a plan produces.
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
      case 'replace':
        return isLiveVal(here.source)
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

export const NULL: Val = { v: 'null' }

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

// The constant a lookup answers for an absent member.
export function missing(): Val {
  return taggedVal(MISSING)
}

export function isMissing(val: Val): boolean {
  return 'tagged' === val.v && MISSING === val.tag && 0 === val.fields.length
}

// Whether this value holds a live stream or text.
export function isLiveVal(val: Val): boolean {
  return ('stream' === val.v || 'text' === val.v) && isLive(val.plan)
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
