/* Copyright (c) 2026 tabnas, MIT License */

// The renderers alchemy's runtime builds its text stages from, taken by
// injection.
//
// Alchemy declares what it constructs and calls and implements none of it:
// `@tabnas/render` exports `renderers`, an implementation of `Renderers`,
// and a host passes it to `compile` (`{ routers, renderers }`). One method
// per renderer or text stage the lowering constructs, taking that
// constructor's parameters in its order and answering what the lowering
// uses it as, and one per render function the runtime calls, taking that
// function's.

import { Limits, Metrics } from './limits'
import { CsvOptions, JsonOptions, MissingRecord } from './options'
import { Sink } from './sink'
import { TableSink } from './table'
import { TextOut, Writer } from './text'

// What `Renderers.join` makes: a `TextOut` whose text is a sequence of
// logical items, with the separator before every item but the first and
// never between the fragments of one item. A fragment written outside an
// item is an item of its own.
export interface JoinOut extends TextOut {
  // Begin an item: the separator is written now if an item came before.
  // Starting an item inside an item is `PROTOCOL_ORDER_ERROR`.
  itemStart(): void
  // End the current item. Ending when no item is open is
  // `PROTOCOL_ORDER_ERROR`.
  itemEnd(): void
}

// What `Renderers.writeOut` makes: a `TextOut` that coalesces fragments
// and writes them to a `Writer`, enforcing `limits.max_output_bytes` once
// given the limits and counting `output_bytes` into the metrics it is
// given.
export interface CoalescingOut extends TextOut {
  withLimits(limits: Limits): this
  withMetrics(metrics: Metrics): this
}

export interface Renderers {
  // A `JsonRenderer`: `JsonEvents/1` as JSON text on `out`, with `options`
  // over the compact profile.
  json(out: TextOut, options?: Partial<JsonOptions>): Sink

  // A `CsvRenderer`: `TableRows/1` as CSV on `out`, with `options` over the
  // standard profile. Throws `TARGET_VALUE_UNREPRESENTABLE` for a delimiter
  // no CSV reader could take.
  csv(out: TextOut, options?: Partial<CsvOptions>): TableSink

  // A `RecordsToJson`: `TableRows/1` as `JsonEvents/1` into `sink`, an
  // array of objects keyed by label, `missing` cells as the policy says
  // (`skip` by default).
  recordsToJson(sink: Sink, missing?: MissingRecord): TableSink

  // A `Join`: `separator` written to `out` between logical items.
  join(out: TextOut, separator: string): JoinOut

  // A `ReplaceText`: every occurrence of the literal `from` written to
  // `out` as `to`, across fragment boundaries.
  replaceText(out: TextOut, from: string, to: string): TextOut

  // A `WriteOut`: fragments coalesced to a byte budget and written to
  // `writer`.
  writeOut(writer: Writer): CoalescingOut

  // What `out` says about committed text, `true` when it cannot tell.
  hasCommitted(out: TextOut): boolean

  // The shortest text that reads back as `value`, which must be finite,
  // laid out as the renderers write a number without a lexeme.
  writeValue(value: number): string
}
