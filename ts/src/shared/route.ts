/* Copyright (c) 2026 tabnas, MIT License */

// Captures, as data: what a router is asked to recognize, and what it
// hands back.
//
// The types of transduce's router (its `route.ts`) and the capture id of
// its matcher (`matcher.ts`), here because a program builds captures and
// consumes their matches without a router of its own: the `Router` that
// recognizes selected scopes and delivers them, complete, in source order
// stays in transduce, which implements `Routers` (`./routers`).

import { Datum } from './datum'
import { Flow } from './sink'
import { Path, Selector } from './selector'

// Which selector matched: its position in the list given to the matcher.
export type CaptureId = number

// What a capture keeps of its match: build the value and deliver it whole
// (`materialize`), or deliver only that the value occurred, and where,
// when it ends (`observe`).
export type CaptureMode = 'materialize' | 'observe'

// The byte budget one materialized capture may not exceed, named after the
// `Limits` field the failure reports.
export type Budget = { bytes: number; name: string }

// One capture: a tag for the consumer, the selector to match, the mode,
// and for a `materialize` capture an optional budget (else the router's
// `max_capture_bytes`).
export class CaptureSpec {
  readonly tag: string
  readonly selector: Selector
  readonly mode: CaptureMode
  budget: Budget | null = null

  constructor(tag: string, selector: Selector, mode: CaptureMode) {
    this.tag = tag
    this.selector = selector
    this.mode = mode
  }

  static materialize(tag: string, selector: Selector): CaptureSpec {
    return new CaptureSpec(tag, selector, 'materialize')
  }

  static observe(tag: string, selector: Selector): CaptureSpec {
    return new CaptureSpec(tag, selector, 'observe')
  }

  // This spec with its own budget, whose failure names `name`.
  withBudget(bytes: number, name: string): CaptureSpec {
    this.budget = { bytes, name }
    return this
  }
}

// One completed match: the spec's position in the router's list, its tag,
// the concrete path, and the value for a `materialize` capture (`null`
// for `observe`).
export type Selected = {
  id: CaptureId
  tag: string
  path: Path
  value: Datum | null
}

// The consumer of a router's matches.
export interface RouteSink {
  // A capture's value is beginning. Nothing has been retained for it yet,
  // so a consumer that knows the value is out of order can refuse it here
  // at no cost (throw a `Fail`); the router adds the path to a failure
  // that has none.
  began?(id: CaptureId, tag: string): void

  selected(selected: Selected): Flow

  // The document ended, validated. Exactly once, after the last match.
  end(): Flow
}
