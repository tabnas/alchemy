/* Copyright (c) 2026 tabnas, MIT License */

// `JsonEvents/1`: the source protocol every transducer consumes.
//
// A source (a tabnas parse, a parsed value, a line-delimited reader) emits
// the events of one document in order: container boundaries, keys, whole
// scalars, and one `end` after the root value. Events are plain frozen
// objects, so a recorder keeps them as they are and a stage that keeps
// anything keeps the event itself.
//
// Scalars arrive whole because the tabnas lexer produces whole tokens. A
// later `JsonEvents/2` may chunk strings; it will be a separately named
// protocol, never a change to this one.

// One event of `JsonEvents/1`, a tagged union on `type`.
//
// `number` carries the machine value and, when the source could hand it
// over, its `lexeme` (the source text), else `null`; a renderer falls back
// to the value. Keeping both is how `50.25` stays `50.25` and how a number
// beyond a double's exact range keeps its digits.
export type JsonEvent =
  | { readonly type: 'object_start' }
  | { readonly type: 'object_end' }
  | { readonly type: 'array_start' }
  | { readonly type: 'array_end' }
  | { readonly type: 'key'; readonly key: string }
  | { readonly type: 'null' }
  | { readonly type: 'bool'; readonly value: boolean }
  | { readonly type: 'number'; readonly value: number; readonly lexeme: string | null }
  | { readonly type: 'string'; readonly value: string }
  | { readonly type: 'end' }

export type JsonEventType = JsonEvent['type']

// The events without a payload are shared, so emitting one allocates
// nothing.
const OBJECT_START: JsonEvent = Object.freeze({ type: 'object_start' })
const OBJECT_END: JsonEvent = Object.freeze({ type: 'object_end' })
const ARRAY_START: JsonEvent = Object.freeze({ type: 'array_start' })
const ARRAY_END: JsonEvent = Object.freeze({ type: 'array_end' })
const NULL: JsonEvent = Object.freeze({ type: 'null' })
const TRUE: JsonEvent = Object.freeze({ type: 'bool', value: true })
const FALSE: JsonEvent = Object.freeze({ type: 'bool', value: false })
const END: JsonEvent = Object.freeze({ type: 'end' })

// Constructors for every event.
export const Ev = Object.freeze({
  objectStart: OBJECT_START,
  objectEnd: OBJECT_END,
  arrayStart: ARRAY_START,
  arrayEnd: ARRAY_END,
  null: NULL,
  end: END,
  key(key: string): JsonEvent {
    return { type: 'key', key }
  },
  bool(value: boolean): JsonEvent {
    return value ? TRUE : FALSE
  },
  number(value: number, lexeme?: string | null): JsonEvent {
    return { type: 'number', value, lexeme: lexeme ?? null }
  },
  string(value: string): JsonEvent {
    return { type: 'string', value }
  },
})

// Whether the event opens a container.
export function isStart(ev: JsonEvent): boolean {
  return 'object_start' === ev.type || 'array_start' === ev.type
}

// Whether the event closes a container.
export function isEnd(ev: JsonEvent): boolean {
  return 'object_end' === ev.type || 'array_end' === ev.type
}

// Whether the event is a whole scalar value.
export function isScalar(ev: JsonEvent): boolean {
  return (
    'null' === ev.type ||
    'bool' === ev.type ||
    'number' === ev.type ||
    'string' === ev.type
  )
}

// Two events are equal: same type and payload. Numbers compare by
// `Object.is` (so `-0` is not `0`) and lexeme.
export function eventEquals(a: JsonEvent, b: JsonEvent): boolean {
  if (a.type !== b.type) return false
  switch (a.type) {
    case 'key':
      return a.key === (b as any).key
    case 'bool':
    case 'string':
      return a.value === (b as any).value
    case 'number':
      return (
        (Object.is(a.value, (b as any).value) ||
          (Number.isNaN(a.value) && Number.isNaN((b as any).value))) &&
        a.lexeme === (b as any).lexeme
      )
    default:
      return true
  }
}

// The event as short text, for messages: `{`, `}`, `[`, `]`, `key "a"`,
// a scalar as JSON, `end`.
export function eventText(ev: JsonEvent): string {
  switch (ev.type) {
    case 'object_start':
      return '{'
    case 'object_end':
      return '}'
    case 'array_start':
      return '['
    case 'array_end':
      return ']'
    case 'key':
      return 'key ' + JSON.stringify(ev.key)
    case 'null':
      return 'null'
    case 'bool':
      return String(ev.value)
    case 'number':
      return null != ev.lexeme ? ev.lexeme : String(ev.value)
    case 'string':
      return JSON.stringify(ev.value)
    case 'end':
      return 'end'
  }
}
