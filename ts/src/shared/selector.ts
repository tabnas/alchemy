/* Copyright (c) 2026 tabnas, MIT License */

// Selectors: reusable descriptions of where in a document to look.
//
// A selector is data, never code: it is built from constructors or from
// validated path segments (`Selector.fromSegments`), and a matcher
// interprets it. A concrete `Path` names one location; a `Selector` may
// name many (`each_index`, `each_member`). Both print in jq syntax, which
// is what the rest of the fleet prints and accepts.

import { jsonString } from './json'

// One step of a concrete path: a string is a member's key, a non-negative
// integer an element's index.
export type Segment = string | number

// One key as jq writes it: bare when it is an ASCII identifier, a JSON
// string otherwise.
export function keyText(key: string): string {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(key) ? '.' + key : '.' + jsonString(key)
}

// A concrete location in a document.
export class Path {
  readonly segments: Segment[]

  constructor(segments: Segment[] = []) {
    this.segments = segments
  }

  static root(): Path {
    return new Path([])
  }

  push(seg: Segment): void {
    this.segments.push(seg)
  }

  pop(): Segment | undefined {
    return this.segments.pop()
  }

  depth(): number {
    return this.segments.length
  }

  clone(): Path {
    return new Path(this.segments.slice())
  }

  // jq syntax: `.a[0]."b-c"`, and `.` for the root.
  toString(): string {
    return pathText(this.segments)
  }
}

// Segments in jq syntax.
export function pathText(segments: readonly Segment[]): string {
  if (0 === segments.length) return '.'
  let out = ''
  for (const seg of segments) {
    out += 'number' === typeof seg ? `[${seg}]` : keyText(seg)
  }
  return out
}

// One step of a selector.
export type Step =
  // The member with this name, inside an object.
  | { readonly type: 'property'; readonly name: string }
  // The element at this position, inside an array.
  | { readonly type: 'index'; readonly index: number }
  // Every element of an array.
  | { readonly type: 'each_index' }
  // Every member value of an object.
  | { readonly type: 'each_member' }

const EACH_INDEX: Step = Object.freeze({ type: 'each_index' })
const EACH_MEMBER: Step = Object.freeze({ type: 'each_member' })

// A description of locations: the root, narrowed step by step. Immutable:
// every builder returns a new selector.
export class Selector {
  readonly steps: readonly Step[]

  constructor(steps: readonly Step[] = []) {
    this.steps = Object.freeze(steps.slice())
  }

  // The document itself.
  static root(): Selector {
    return new Selector([])
  }

  property(name: string): Selector {
    return new Selector([...this.steps, { type: 'property', name }])
  }

  index(index: number): Selector {
    if (!Number.isInteger(index) || index < 0) {
      throw new TypeError('an index step is a non-negative integer: ' + index)
    }
    return new Selector([...this.steps, { type: 'index', index }])
  }

  eachIndex(): Selector {
    return new Selector([...this.steps, EACH_INDEX])
  }

  eachMember(): Selector {
    return new Selector([...this.steps, EACH_MEMBER])
  }

  // `this`, then `other` below every location `this` names.
  compose(other: Selector): Selector {
    return new Selector([...this.steps, ...other.steps])
  }

  // A selector naming exactly one location: `as-path` over data.
  static fromSegments(segments: readonly Segment[]): Selector {
    let s = Selector.root()
    for (const seg of segments) {
      s = 'number' === typeof seg ? s.index(seg) : s.property(seg)
    }
    return s
  }

  isRoot(): boolean {
    return 0 === this.steps.length
  }

  // Whether the selector can name more than one location.
  isMulti(): boolean {
    return this.steps.some((s) => 'each_index' === s.type || 'each_member' === s.type)
  }

  // Whether this selector names a location strictly inside a location
  // `other` names, or the same one: the test a router uses to refuse
  // overlapping captures.
  mayOverlap(other: Selector): boolean {
    const [short, long] =
      this.steps.length <= other.steps.length ? [this, other] : [other, this]
    for (let i = 0; i < short.steps.length; i++) {
      if (!stepMayMatchSame(short.steps[i], long.steps[i])) return false
    }
    return true
  }

  equals(other: Selector): boolean {
    return this.toString() === other.toString()
  }

  // jq syntax: `.response.records[*]`, `."odd key"`, `[3]`, `[]` for
  // every member, `.` for the root.
  toString(): string {
    if (0 === this.steps.length) return '.'
    let out = ''
    for (const step of this.steps) {
      switch (step.type) {
        case 'property':
          out += keyText(step.name)
          break
        case 'index':
          out += `[${step.index}]`
          break
        case 'each_index':
          out += '[*]'
          break
        case 'each_member':
          out += '[]'
          break
      }
    }
    return out
  }
}

function stepMayMatchSame(a: Step, b: Step): boolean {
  switch (a.type) {
    case 'property':
      return (
        ('property' === b.type && a.name === b.name) || 'each_member' === b.type
      )
    case 'each_member':
      return 'property' === b.type || 'each_member' === b.type
    case 'index':
      return ('index' === b.type && a.index === b.index) || 'each_index' === b.type
    case 'each_index':
      return 'index' === b.type || 'each_index' === b.type
  }
}
