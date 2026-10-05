/* Copyright (c) 2026 tabnas, MIT License */

// The routers alchemy's runtime builds its stages from, taken by
// injection.
//
// Alchemy declares what it constructs and implements none of it:
// `@tabnas/transduce` exports `routers`, an implementation of `Routers`,
// and a host passes it to `compile` (`{ routers, renderers }`). One method
// per transduce stage the lowering constructs, taking that constructor's
// parameters in its order, and answering what the lowering uses the stage
// as.

import { Duplicates } from './datum'
import { AbortFlag, Limits, Metrics } from './limits'
import { CaptureSpec, RouteSink } from './route'
import { Transition } from './scan'
import { Flow, Sink } from './sink'
import { TableBinding, TableSink } from './table'

// What `Routers.scanEmit` makes: the operator, fed its items with `item`
// and finished with `finish` exactly once, when the input completed
// successfully. A step or the finish throws a `Fail` to fail the run.
export interface Scan<I> {
  // One input item. Its outputs go downstream before this returns.
  item(item: I): Flow
  // The input completed: emit the closing items.
  finish(): Flow
}

export interface Routers {
  // A `Router`: every capture of `specs` recognized in one pass over the
  // events, each completed match handed to `downstream`. Two specs that may
  // overlap are refused unless both observe (`CAPTURE_OVERLAP_UNSUPPORTED`).
  router(
    specs: readonly CaptureSpec[],
    limits: Limits,
    duplicates: Duplicates,
    metrics: Metrics,
    downstream: RouteSink,
  ): Sink

  // A `TableFromJson`: the metadata-first table transducer, `JsonEvents/1`
  // in and `TableRows/1` out to `sink`, rows materialized under
  // `max_record_bytes` and metadata under `max_metadata_bytes`.
  tableFromJson(
    binding: TableBinding,
    limits: Limits,
    duplicates: Duplicates,
    metrics: Metrics,
    sink: TableSink,
  ): Sink

  // A `ScanEmit`: the state evolved from `initial` by `step` over each
  // item, `finish` turning the final state into the closing items, every
  // output handed to `out`.
  scanEmit<S, I, O>(
    initial: S,
    step: (state: S, item: I) => Transition<S, O>,
    finish: (state: S) => O[],
    out: (output: O) => Flow,
  ): Scan<I>

  // A `Guarded`: `inner` behind the source limits (`max_depth`,
  // `max_key_bytes`, `max_scalar_bytes`), the abort flag, polled per event,
  // and the source metrics, counted into `metrics`.
  guarded(inner: Sink, limits: Limits, abort: AbortFlag, metrics: Metrics): Sink
}
