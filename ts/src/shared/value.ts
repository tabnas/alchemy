/* Copyright (c) 2026 tabnas, MIT License */

// Reading the engine's values: what a tabnas parse returns in TypeScript.
//
// The engine builds plain JavaScript values: `null` and `undefined`,
// booleans, numbers, strings, arrays and objects. A few grammars hand over
// host types, which read as the scalars they stand for: a boxed `String`
// (the engine's text annotation), a `BigInt`, and a TOML `Date`, which
// reads as the source text it carries (`__toml__.src`), as the Rust
// runtime's date does.
//
// An object's member order is its insertion order. A plain object
// enumerates integer-like keys first whatever order they were written in,
// so the engine records the order on the node under `map.ordered` (read
// with `KEY_ORDER`); the sources switch that on, and `engineKeys` reads
// it, falling back to `Object.keys`.

// The engine's key-order side channel (`keyOrder` in @tabnas/parser).
const KEY_ORDER = Symbol.for('tabnas.keyOrder')

export type EngineScalar =
  | { type: 'null' }
  | { type: 'bool'; value: boolean }
  | { type: 'number'; value: number }
  | { type: 'string'; value: string }

const NULL: EngineScalar = { type: 'null' }

// The value as a scalar, or `undefined` when it is a container.
export function engineScalar(v: unknown): EngineScalar | undefined {
  switch (typeof v) {
    case 'undefined':
      return NULL
    case 'boolean':
      return { type: 'bool', value: v }
    case 'number':
      return { type: 'number', value: v }
    case 'bigint':
      return { type: 'number', value: Number(v) }
    case 'string':
      return { type: 'string', value: v }
    case 'object':
      if (null === v) return NULL
      if (Array.isArray(v)) return undefined
      if (v instanceof String) return { type: 'string', value: v.valueOf() }
      if (v instanceof Number) return { type: 'number', value: v.valueOf() }
      if (v instanceof Boolean) return { type: 'bool', value: v.valueOf() }
      if (v instanceof Date) {
        const src = (v as any).__toml__?.src
        return { type: 'string', value: 'string' === typeof src ? src : v.toJSON() ?? 'null' }
      }
      return undefined
    default:
      // A function or a symbol has no JSON form.
      return NULL
  }
}

// Whether the value is a container: an array, or an object that is no
// scalar's host type.
export function isEngineContainer(v: unknown): boolean {
  return null !== v && 'object' === typeof v && undefined === engineScalar(v)
}

// Whether the value is a map (a container that is not an array).
export function isEngineMap(v: unknown): v is Record<string, unknown> {
  return isEngineContainer(v) && !Array.isArray(v)
}

// A map's member names, in insertion order: the engine's recorded order
// where it kept one, each name once and only while the map still holds it,
// then any member the record missed.
export function engineKeys(v: unknown): string[] {
  if (null === v || 'object' !== typeof v) return []
  const own = Object.keys(v as object)
  const order: unknown = (v as any)[KEY_ORDER]
  if (!Array.isArray(order) || 0 === order.length) return own
  const has = Object.prototype.hasOwnProperty
  const seen = new Set<string>()
  const out: string[] = []
  for (const k of order) {
    const key = String(k)
    if (seen.has(key) || !has.call(v, key)) continue
    seen.add(key)
    out.push(key)
  }
  if (out.length !== own.length) {
    for (const k of own) {
      if (!seen.has(k)) out.push(k)
    }
  }
  return out
}
