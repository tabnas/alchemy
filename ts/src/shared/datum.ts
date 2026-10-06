/* Copyright (c) 2026 tabnas, MIT License */

// The retained value type.
//
// What a capture materializes and what a projected cell holds. Distinct
// from the engine's value on purpose: a datum keeps a number's lexeme,
// keeps member order whatever the keys look like (a `Map`, since a plain
// object enumerates integer-like keys first), measures its own size
// against limits in UTF-8 bytes, and is owned by the transducer rather
// than shared with a parse.

import { Fail } from './error'
import { Ev, JsonEvent } from './event'
import { jsonNumber, jsonString } from './json'
import { NODE_BYTES, utf8Bytes } from './limits'
import { Segment } from './selector'
import { Flow, Sink } from './sink'
import { engineKeys, engineScalar } from './value'

// A retained JSON-like value. Members of an object are in source order; a
// repeated member replaces the earlier one in its place (last value wins)
// unless the builder's policy rejected it first.
export type Datum =
  | { readonly type: 'null' }
  | { readonly type: 'bool'; readonly value: boolean }
  | { readonly type: 'number'; readonly value: number; readonly lexeme: string | null }
  | { readonly type: 'string'; readonly value: string }
  | { readonly type: 'array'; readonly items: Datum[] }
  | { readonly type: 'object'; readonly members: Map<string, Datum> }

const NULL: Datum = Object.freeze({ type: 'null' })
const TRUE: Datum = Object.freeze({ type: 'bool', value: true })
const FALSE: Datum = Object.freeze({ type: 'bool', value: false })

export const Datum = Object.freeze({
  null: NULL,
  bool(value: boolean): Datum {
    return value ? TRUE : FALSE
  },
  number(value: number, lexeme?: string | null): Datum {
    return { type: 'number', value, lexeme: lexeme ?? null }
  },
  string(value: string): Datum {
    return { type: 'string', value }
  },
  array(items: Datum[] = []): Datum {
    return { type: 'array', items }
  },
  object(members: Map<string, Datum> | Iterable<[string, Datum]> = []): Datum {
    return { type: 'object', members: members instanceof Map ? members : new Map(members) }
  },

  // Payload bytes plus `NODE_BYTES` per node: the measure limits use.
  byteSize,
  getPath,
  takePath,
  isContainer(d: Datum): boolean {
    return 'array' === d.type || 'object' === d.type
  },

  // From an engine value: `undefined` is `null`, as the engine serializes
  // it; a number has no lexeme.
  fromTabnas,

  // As a plain JavaScript value, for oracles and tests. An object becomes a
  // plain object (integer-like keys reorder), a number its value.
  toJSON,

  // From a plain JavaScript value, for tests. A number's lexeme is its
  // shortest text.
  fromJSON,

  // Compact JSON, keeping number lexemes.
  toText,
})

export function byteSize(d: Datum): number {
  switch (d.type) {
    case 'null':
    case 'bool':
      return NODE_BYTES
    case 'number':
      return NODE_BYTES + (null != d.lexeme ? utf8Bytes(d.lexeme) : 8)
    case 'string':
      return NODE_BYTES + utf8Bytes(d.value)
    case 'array': {
      let n = NODE_BYTES
      for (const item of d.items) n += byteSize(item)
      return n
    }
    case 'object': {
      let n = NODE_BYTES
      for (const [k, v] of d.members) n += utf8Bytes(k) + byteSize(v)
      return n
    }
  }
}

// The value at a concrete path below this one.
export function getPath(d: Datum, path: readonly Segment[]): Datum | undefined {
  let here: Datum | undefined = d
  for (const seg of path) {
    if (undefined === here) return undefined
    if ('string' === typeof seg) {
      here = 'object' === here.type ? here.members.get(seg) : undefined
    } else {
      here = 'array' === here.type ? here.items[seg] : undefined
    }
  }
  return here
}

// The value at a concrete path below this one, moved out and replaced by
// `null` in its container. The root itself cannot be replaced in place, so
// an empty path returns the datum and leaves it alone.
export function takePath(d: Datum, path: readonly Segment[]): Datum | undefined {
  if (0 === path.length) return d
  const parent = getPath(d, path.slice(0, -1))
  if (undefined === parent) return undefined
  const last = path[path.length - 1]
  if ('string' === typeof last) {
    if ('object' !== parent.type || !parent.members.has(last)) return undefined
    const v = parent.members.get(last) as Datum
    parent.members.set(last, NULL)
    return v
  }
  if ('array' !== parent.type || last >= parent.items.length) return undefined
  const v = parent.items[last]
  parent.items[last] = NULL
  return v
}

export function fromTabnas(value: unknown): Datum {
  if (Array.isArray(value)) return Datum.array(value.map(fromTabnas))
  const scalar = engineScalar(value)
  if (undefined !== scalar) {
    switch (scalar.type) {
      case 'null':
        return NULL
      case 'bool':
        return Datum.bool(scalar.value)
      case 'number':
        return Datum.number(scalar.value)
      case 'string':
        return Datum.string(scalar.value)
    }
  }
  const members = new Map<string, Datum>()
  for (const k of engineKeys(value)) members.set(k, fromTabnas((value as any)[k]))
  return { type: 'object', members }
}

export function toJSON(d: Datum): unknown {
  switch (d.type) {
    case 'null':
      return null
    case 'bool':
    case 'string':
      return d.value
    case 'number':
      return null != d.lexeme ? Number(d.lexeme) : d.value
    case 'array':
      return d.items.map(toJSON)
    case 'object': {
      const out: Record<string, unknown> = {}
      for (const [k, v] of d.members) out[k] = toJSON(v)
      return out
    }
  }
}

export function fromJSON(value: unknown): Datum {
  if (null == value) return NULL
  if ('boolean' === typeof value) return Datum.bool(value)
  if ('number' === typeof value) return Datum.number(value, String(value))
  if ('string' === typeof value) return Datum.string(value)
  if (Array.isArray(value)) return Datum.array(value.map(fromJSON))
  const members = new Map<string, Datum>()
  if (value instanceof Map) {
    for (const [k, v] of value) members.set(String(k), fromJSON(v))
  } else {
    for (const k of Object.keys(value as object)) {
      members.set(k, fromJSON((value as any)[k]))
    }
  }
  return { type: 'object', members }
}

export function toText(d: Datum): string {
  switch (d.type) {
    case 'null':
      return 'null'
    case 'bool':
      return d.value ? 'true' : 'false'
    case 'number':
      return jsonNumber(d.value, d.lexeme)
    case 'string':
      return jsonString(d.value)
    case 'array': {
      let out = '['
      d.items.forEach((item, i) => {
        if (i > 0) out += ','
        out += toText(item)
      })
      return out + ']'
    }
    case 'object': {
      let out = '{'
      let i = 0
      for (const [k, v] of d.members) {
        if (i++ > 0) out += ','
        out += jsonString(k) + ':' + toText(v)
      }
      return out + '}'
    }
  }
}

// Emit a datum as `JsonEvents/1` (without the final `end`, so a datum can
// stand in for any part of a document).
export function walkDatum(d: Datum, sink: Sink): Flow {
  switch (d.type) {
    case 'null':
      return sink.event(Ev.null)
    case 'bool':
      return sink.event(Ev.bool(d.value))
    case 'number':
      return sink.event(Ev.number(d.value, d.lexeme))
    case 'string':
      return sink.event(Ev.string(d.value))
    case 'array':
      if ('stop' === sink.event(Ev.arrayStart)) return 'stop'
      for (const item of d.items) {
        if ('stop' === walkDatum(item, sink)) return 'stop'
      }
      return sink.event(Ev.arrayEnd)
    case 'object':
      if ('stop' === sink.event(Ev.objectStart)) return 'stop'
      for (const [k, v] of d.members) {
        if ('stop' === sink.event(Ev.key(k))) return 'stop'
        if ('stop' === walkDatum(v, sink)) return 'stop'
      }
      return sink.event(Ev.objectEnd)
  }
}

// How a builder treats a repeated member name: `reject` fails with
// `DUPLICATE_MEMBER`, `last_wins` replaces the earlier value, `first_wins`
// keeps it.
export type Duplicates = 'reject' | 'last_wins' | 'first_wins'

type Frame =
  | { array: true; items: Datum[] }
  | { array: false; members: Map<string, Datum>; key: string | null }

// Builds one `Datum` from the events of one value, under a byte limit.
//
// Feed it every event from the value's first to its last; `finished()`
// says when the value is complete. Bytes are counted as they arrive, and
// the limit fails at the first byte over it rather than after the value is
// whole. The builder knows nothing of where in a document its value sits:
// a failure leaves `path` unset and the stage that placed the builder adds
// it.
export class DatumBuilder {
  stack: Frame[] = []
  done: Datum | undefined = undefined
  held = 0
  readonly limit: number
  readonly limitName: string
  readonly duplicates: Duplicates

  // A builder whose limit failure names `limitName` (a `Limits` field).
  constructor(limit: number, limitName: string, duplicates: Duplicates) {
    this.limit = limit
    this.limitName = limitName
    this.duplicates = duplicates
  }

  bytes(): number {
    return this.held
  }

  finished(): boolean {
    return undefined !== this.done
  }

  // The value, once finished. Probing an unfinished builder returns
  // `undefined` and leaves the charge for the partial value in place.
  take(): Datum | undefined {
    const done = this.done
    if (undefined !== done) {
      this.done = undefined
      this.held = 0
    }
    return done
  }

  charge(n: number): void {
    this.held += n
    if (this.held > this.limit) {
      throw Fail.limit(
        this.limitName,
        this.limit,
        `a value is larger than ${this.limit} bytes`,
      )
    }
  }

  place(value: Datum): void {
    const top = this.stack[this.stack.length - 1]
    if (undefined === top) {
      this.done = value
      return
    }
    if (top.array) {
      top.items.push(value)
      return
    }
    const key = top.key
    if (null === key) {
      throw Fail.protocol('a value arrived inside an object without a key')
    }
    top.key = null
    // A repeated member was charged in full as its events arrived, which is
    // right: for a moment both were held. Whichever one goes now gives its
    // bytes back, so an object that keeps repeating a small key does not
    // grow towards the limit while the value it holds stays the same size.
    const existing = top.members.get(key)
    let released = 0
    if (undefined !== existing) {
      switch (this.duplicates) {
        case 'reject':
          throw new Fail('DUPLICATE_MEMBER', `member ${JSON.stringify(key)} appears twice`)
        case 'first_wins':
          this.held = Math.max(0, this.held - (utf8Bytes(key) + byteSize(value)))
          return
        case 'last_wins':
          released = utf8Bytes(key) + byteSize(existing)
      }
    }
    top.members.set(key, value)
    this.held = Math.max(0, this.held - released)
  }

  // One event of the value being built.
  event(ev: JsonEvent): void {
    if (undefined !== this.done) {
      throw Fail.protocol('an event arrived after the value was complete')
    }
    switch (ev.type) {
      case 'object_start':
        this.charge(NODE_BYTES)
        this.stack.push({ array: false, members: new Map(), key: null })
        break
      case 'array_start':
        this.charge(NODE_BYTES)
        this.stack.push({ array: true, items: [] })
        break
      case 'key': {
        this.charge(utf8Bytes(ev.key))
        const top = this.stack[this.stack.length - 1]
        if (undefined === top || top.array || null !== top.key) {
          throw Fail.protocol('a key arrived where no member was expected')
        }
        top.key = ev.key
        break
      }
      case 'object_end': {
        const top = this.stack.pop()
        if (undefined === top || top.array) {
          throw Fail.protocol('an object ended that had not started')
        }
        if (null !== top.key) {
          throw Fail.protocol('an object ended after a key without its value')
        }
        this.place({ type: 'object', members: top.members })
        break
      }
      case 'array_end': {
        const top = this.stack.pop()
        if (undefined === top || !top.array) {
          throw Fail.protocol('an array ended that had not started')
        }
        this.place({ type: 'array', items: top.items })
        break
      }
      case 'null':
        this.charge(NODE_BYTES)
        this.place(NULL)
        break
      case 'bool':
        this.charge(NODE_BYTES)
        this.place(Datum.bool(ev.value))
        break
      case 'number':
        this.charge(NODE_BYTES + (null != ev.lexeme ? utf8Bytes(ev.lexeme) : 8))
        this.place(Datum.number(ev.value, ev.lexeme))
        break
      case 'string':
        this.charge(NODE_BYTES + utf8Bytes(ev.value))
        this.place(Datum.string(ev.value))
        break
      case 'end':
        throw Fail.protocol('the document ended inside a value being captured')
    }
  }
}
