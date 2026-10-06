/* Copyright (c) 2026 tabnas, MIT License */

// The push boundary between stages.
//
// A pipeline is a chain of sinks. The source calls the first sink once per
// event, synchronously, inside the parse; each stage does its work and
// calls the next. Nothing is queued between stages, so a slow writer at the
// end slows the parser at the start: that is the backpressure, and it
// costs no buffer. A stage that must stop early (a `take`) answers
// `'stop'`, which the source turns into a cancelled parse. A stage that
// fails throws a `Fail`; the source stops the parse and the failure
// reaches the caller unchanged.

import { Fail } from './error'
import { JsonEvent } from './event'
import { Path, Segment } from './selector'

// What a stage wants next: `'continue'`, or `'stop'` (the stage has all it
// needs; the source stops the parse, releases what it holds and reports
// nothing further; not an error).
export type Flow = 'continue' | 'stop'

// A consumer of `JsonEvents/1`. `event` throws a `Fail` to fail the run.
export interface Sink {
  event(ev: JsonEvent): Flow
}

// A sink that records every event, for tests and small results.
export class EventRecorder implements Sink {
  readonly events: JsonEvent[] = []

  event(ev: JsonEvent): Flow {
    this.events.push(ev)
    return 'continue'
  }
}

// A sink made of a function. A function that returns nothing continues.
export class FnSink implements Sink {
  fn: (ev: JsonEvent) => Flow | void

  constructor(fn: (ev: JsonEvent) => Flow | void) {
    this.fn = fn
  }

  event(ev: JsonEvent): Flow {
    return this.fn(ev) || 'continue'
  }
}

// A sink that counts events and drops them: the cheapest consumer, for
// measuring a source on its own.
export class CountSink implements Sink {
  events = 0

  event(_ev: JsonEvent): Flow {
    this.events++
    return 'continue'
  }
}

// Replay a recording into a sink, stopping where the sink stops.
export function replay(events: Iterable<JsonEvent>, sink: Sink): Flow {
  for (const ev of events) {
    if ('stop' === sink.event(ev)) return 'stop'
  }
  return 'continue'
}

// A container `TreeContract` has open: an object with the keys it has had
// (the last of them the member whose value is due unless `keyDue`), or an
// array with the index of the element due next.
type Open =
  | { array: false; keys: Set<string>; last: string | null; keyDue: boolean }
  | { array: true; next: number }

// A tree's events, held to their contract in front of a sink that takes
// them as one (a render that writes a document from them): one root value,
// and in each object a key and then its value, each key once. A value
// walked from a parsed tree keeps it by construction. A parse streamed as
// it proceeds may not: it hands on a member its grammar reads twice
// (JSON's `{"a":1,"a":2}`, whose value keeps the last) where a tree has
// one, and the rule-event adapter refuses most shapes it cannot follow but
// not every one a grammar can produce. A repeated key in one object is
// refused with `DUPLICATE_MEMBER`, and events no tree has (a value where a
// key is due, a key outside an object, a close with nothing open, a second
// root) with `STREAMABILITY_UNKNOWN`, each at the path of the object
// concerned; the event is not passed on. Each open object keeps the keys
// it has had, dropped when it closes.
export class TreeContract<S extends Sink> implements Sink {
  readonly inner: S
  open: Open[] = []
  rootDone = false

  constructor(inner: S) {
    this.inner = inner
  }

  // The path of the value due next: the open containers, each by the member
  // or element open in it, and in the innermost object its last key when
  // that member's value is due.
  path(): Path {
    const segments: Segment[] = []
    const last = this.open.length - 1
    this.open.forEach((open, i) => {
      if (open.array) {
        segments.push(open.next)
      } else if ((i !== last || !open.keyDue) && null != open.last) {
        // A container open inside this object is its last key's value,
        // whatever `keyDue` says: the member was counted as taken when its
        // value opened.
        segments.push(open.last)
      }
    })
    return new Path(segments)
  }

  notATree(what: string): Fail {
    return new Fail(
      'STREAMABILITY_UNKNOWN',
      `the stream holds ${what}, which a tree's events never do, so it is not a tree's`,
    ).atPath(this.path().toString())
  }

  // A value is complete: the next in its array is due, or the root is.
  closed(): void {
    const top = this.open[this.open.length - 1]
    if (undefined === top) this.rootDone = true
    else if (top.array) top.next++
  }

  event(ev: JsonEvent): Flow {
    const top = this.open[this.open.length - 1]
    switch (ev.type) {
      case 'key': {
        if (undefined !== top && !top.array && top.keyDue) {
          if (top.keys.has(ev.key)) {
            const path = this.path()
            path.push(ev.key)
            throw new Fail(
              'DUPLICATE_MEMBER',
              `member ${JSON.stringify(ev.key)} appears twice in one object, and a tree's ` +
                `events hold each key once`,
            ).atPath(path.toString())
          }
          top.keys.add(ev.key)
          top.last = ev.key
          top.keyDue = false
        } else if (undefined !== top && !top.array) {
          throw this.notATree('a key where a value is due')
        } else {
          throw this.notATree('a key outside an object')
        }
        break
      }
      case 'object_end':
        if (undefined !== top && !top.array && top.keyDue) {
          this.open.pop()
          this.closed()
        } else if (undefined !== top && !top.array) {
          throw this.notATree("an object's end where a value is due")
        } else {
          throw this.notATree("an object's end where none is due")
        }
        break
      case 'array_end':
        if (undefined !== top && top.array) {
          this.open.pop()
          this.closed()
        } else {
          throw this.notATree("an array's end where none is due")
        }
        break
      case 'end':
        if (0 < this.open.length) {
          throw this.notATree('its end inside an open container')
        }
        break
      default: {
        // A value: in an object, only once its key is in; at the root,
        // only once.
        if (undefined === top) {
          if (this.rootDone) throw this.notATree('a second root value')
        } else if (!top.array) {
          if (top.keyDue) throw this.notATree('a value where a key is due')
          top.keyDue = true
        }
        if ('object_start' === ev.type) {
          this.open.push({ array: false, keys: new Set(), last: null, keyDue: true })
        } else if ('array_start' === ev.type) {
          this.open.push({ array: true, next: 0 })
        } else {
          this.closed()
        }
      }
    }
    return this.inner.event(ev)
  }
}
