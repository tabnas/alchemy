/* Copyright (c) 2026 tabnas, MIT License */

// The checker's types. A port of rs/src/types.rs.
//
// The types tell three things apart that the syntax does not: a reusable
// value, a single-use resource, and a protocol. `Value` is any data a
// document can hold; `Vector<T>` a finite retained collection;
// `Stream<T>` an ordered single-use sequence, of which `TableEvents`
// (`Stream<TableEvent>`) and `Stream<Event>` are two; `JsonEvents` the
// single-use source; `Text` single-use incremental text; `String` a
// finite retained string. `Unknown` is what inference could not decide,
// and it is accepted everywhere: the checker is conservative. `Never` is
// the type of `fail`.
//
// Compatibility is `accepts`: whether a value of the actual type may be
// given where the expected type is wanted.

export type Type =
  | { readonly t: 'Unknown' }
  | { readonly t: 'Never' }
  | { readonly t: 'Null' }
  | { readonly t: 'Bool' }
  | { readonly t: 'Number' }
  | { readonly t: 'String' }
  | { readonly t: 'Keyword' }
  // Any data value (what a document holds, `missing` included).
  | { readonly t: 'Value' }
  | { readonly t: 'Vector'; readonly item: Type }
  | { readonly t: 'Record' }
  | { readonly t: 'Selector' }
  | { readonly t: 'CaptureSpec' }
  // A constructor's value, by the constructor's name.
  | { readonly t: 'Tagged'; readonly tag: string }
  // A `schema`, `row` or `table-end` value.
  | { readonly t: 'TableEvent' }
  // One event of the source as `events` delivers it.
  | { readonly t: 'Event' }
  // A function: its parameters and its result.
  | { readonly t: 'Fn'; readonly params: Type[]; readonly result: Type }
  // A single-use stream of items.
  | { readonly t: 'Stream'; readonly item: Type }
  // The single-use source.
  | { readonly t: 'JsonEvents' }
  // Single-use incremental text.
  | { readonly t: 'Text' }

export const Unknown: Type = { t: 'Unknown' }
export const Never: Type = { t: 'Never' }
export const Null: Type = { t: 'Null' }
export const Bool: Type = { t: 'Bool' }
export const Num: Type = { t: 'Number' }
export const Str: Type = { t: 'String' }
export const Keyword: Type = { t: 'Keyword' }
export const Value: Type = { t: 'Value' }
export const Record: Type = { t: 'Record' }
export const Selector: Type = { t: 'Selector' }
export const CaptureSpec: Type = { t: 'CaptureSpec' }
export const TableEvent: Type = { t: 'TableEvent' }
export const Event: Type = { t: 'Event' }
export const JsonEvents: Type = { t: 'JsonEvents' }
export const Text: Type = { t: 'Text' }

export function vectorOf(item: Type): Type {
  return { t: 'Vector', item }
}

export function streamOf(item: Type): Type {
  return { t: 'Stream', item }
}

// `Stream<TableEvent>`: the `TableRows/1` protocol.
export function tableEvents(): Type {
  return streamOf(TableEvent)
}

// `Stream<Event>`: the source's events as items, what `events` yields.
export function events(): Type {
  return streamOf(Event)
}

export function tagged(tag: string): Type {
  return { t: 'Tagged', tag }
}

export function func(params: Type[], result: Type): Type {
  return { t: 'Fn', params, result }
}

// A function of `n` parameters about which nothing more is known.
export function funcOf(n: number): Type {
  return func(new Array(n).fill(Unknown), Unknown)
}

// Structural equality, as the Rust `PartialEq` derive has it.
export function typeEq(a: Type, b: Type): boolean {
  if (a.t !== b.t) return false
  switch (a.t) {
    case 'Vector':
    case 'Stream':
      return typeEq(a.item, (b as typeof a).item)
    case 'Tagged':
      return a.tag === (b as typeof a).tag
    case 'Fn': {
      const f = b as typeof a
      return (
        a.params.length === f.params.length &&
        a.params.every((p, i) => typeEq(p, f.params[i])) &&
        typeEq(a.result, f.result)
      )
    }
    default:
      return true
  }
}

export function typesEq(a: Type[], b: Type[]): boolean {
  return a.length === b.length && a.every((x, i) => typeEq(x, b[i]))
}

// Whether a binding of this type is used at most once.
export function isAffine(ty: Type): boolean {
  return 'Stream' === ty.t || 'JsonEvents' === ty.t || 'Text' === ty.t
}

// Whether this is one of the protocols, so a mismatch is between protocols
// rather than between kinds of value.
export function isProtocol(ty: Type): boolean {
  return isAffine(ty)
}

// Whether this type is data: what a document can hold.
export function isData(ty: Type): boolean {
  switch (ty.t) {
    case 'Null':
    case 'Bool':
    case 'Number':
    case 'String':
    case 'Record':
    case 'Value':
    case 'Unknown':
    case 'Never':
      return true
    case 'Vector':
      return isData(ty.item)
    case 'Tagged':
      return 'missing' === ty.tag
    default:
      return false
  }
}

// Whether this is a stream or the source: what a vector can never hold. A
// text is affine too, but it may be finite, so a vector may hold one; the
// runtime refuses a live one where the vector is built.
export function isStreamOrSource(ty: Type): boolean {
  return 'Stream' === ty.t || 'JsonEvents' === ty.t
}

// Whether this type is a stream: `Stream<T>` in any shape.
export function isStream(ty: Type): boolean {
  return 'Stream' === ty.t
}

// Whether a text combinator takes a value of this type as an item.
export function isTextlike(ty: Type): boolean {
  return 'String' === ty.t || 'Text' === ty.t || 'Value' === ty.t || 'Unknown' === ty.t || 'Never' === ty.t
}

const TABLE_EVENT_TAGS = ['schema', 'row', 'table-end']
const EVENT_TAGS = ['object-start', 'object-end', 'array-start', 'array-end', 'key', 'scalar']

// Whether a value of type `actual` may be given where `expected` is
// expected. `Value` is any data, so it may be the particular data wanted
// and passes, for the runtime to check, as `Unknown` does; a type that can
// never be it does not.
export function accepts(expected: Type, actual: Type): boolean {
  if ('Unknown' === expected.t || 'Unknown' === actual.t || 'Never' === actual.t) return true
  if ('Value' === expected.t) return isData(actual)
  if ('Value' === actual.t) return isData(expected)
  if ('Vector' === expected.t && 'Vector' === actual.t) return accepts(expected.item, actual.item)
  if ('Stream' === expected.t && 'Stream' === actual.t) return accepts(expected.item, actual.item)
  if ('TableEvent' === expected.t && 'Tagged' === actual.t) return TABLE_EVENT_TAGS.includes(actual.tag)
  if ('Event' === expected.t && 'Tagged' === actual.t) return EVENT_TAGS.includes(actual.tag)
  if ('Fn' === expected.t && 'Fn' === actual.t) {
    return expected.params.length === actual.params.length && accepts(expected.result, actual.result)
  }
  // A stream of events reaches any taker of JSON events.
  if ('JsonEvents' === expected.t && 'Stream' === actual.t) return accepts(Event, actual.item)
  return typeEq(expected, actual)
}

// The type of a value that is one of two: the same type when they agree,
// a text when one is a text and the other a string, else unknown.
export function join(a: Type, b: Type): Type {
  if ('Never' === a.t) return b
  if ('Never' === b.t) return a
  if (typeEq(a, b)) return a
  if (('Text' === a.t && 'String' === b.t) || ('String' === a.t && 'Text' === b.t)) return Text
  if ('Vector' === a.t && 'Vector' === b.t) return vectorOf(join(a.item, b.item))
  if (isData(a) && isData(b)) return Value
  return Unknown
}

// The type of the items of a `Vector` or `Stream`.
export function itemOf(ty: Type): Type | undefined {
  return 'Vector' === ty.t || 'Stream' === ty.t ? ty.item : undefined
}

// What a value of this type is, in the words the runtime names a value's
// kind with (`kindText`): `a stream` for a stream or the source, `a text`,
// `a number`, and so on; `data` for `Value`, which is any of several (and
// for `Unknown` and `Never`, which every place accepts, so no message names
// them). A message the checker and the runtime both write (a value that
// cannot be held, a callee that cannot be called) names the value this way
// at both stages.
export function typeKindText(ty: Type): string {
  switch (ty.t) {
    case 'Null':
      return 'null'
    case 'Bool':
      return 'a boolean'
    case 'Number':
      return 'a number'
    case 'String':
      return 'a string'
    case 'Keyword':
      return 'a keyword'
    case 'Value':
    case 'Unknown':
    case 'Never':
      return 'data'
    case 'Vector':
      return 'a vector'
    case 'Record':
      return 'a record'
    case 'Selector':
      return 'a selector'
    case 'CaptureSpec':
      return 'a capture'
    case 'Tagged':
      return 'missing' === ty.tag ? 'missing' : 'a tagged value'
    case 'TableEvent':
    case 'Event':
      return 'a tagged value'
    case 'Fn':
      return 'a function'
    case 'Stream':
    case 'JsonEvents':
      return 'a stream'
    case 'Text':
      return 'a text'
  }
}

// The type as the messages print it.
export function typeText(ty: Type): string {
  switch (ty.t) {
    case 'Vector':
      return `Vector<${typeText(ty.item)}>`
    case 'Tagged':
      return ty.tag
    case 'Fn':
      return `Fn(${ty.params.map(typeText).join(' ')} -> ${typeText(ty.result)})`
    case 'Stream':
      return 'TableEvent' === ty.item.t ? 'TableEvents' : `Stream<${typeText(ty.item)}>`
    default:
      return ty.t
  }
}
