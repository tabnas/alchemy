/* Copyright (c) 2026 tabnas, MIT License */

// Shared helpers for the tests: where the repository and its fixtures
// are, and what a failure's finer code is (rs/tests/common/mod.rs).

import { join } from 'node:path'

import { make as makeJson } from '@tabnas/json'
import { BytesWriter } from '@tabnas/render'
import { findSpecDir } from '@tabnas/support'
import { EventRecorder, Fail, Limits, Metrics, ParserSource, Prune, SourceMode, replay } from '@tabnas/transduce'

// The repository root: two levels above dist-test/.
export const REPO_ROOT = join(__dirname, '..', '..')

// test/spec, found as every runtime finds it.
export const SPEC_DIR = findSpecDir(__dirname)

// The codes whose failures carry a finer code as the first word of the
// message, before `: `.
const OWN_CODES = ['DSL_PARSE_ERROR', 'DSL_TYPE_ERROR', 'STREAM_REUSED', 'STREAMABILITY_UNKNOWN']

// The code a fixture pins for a failure: the finer code for this
// package's own codes, the transduce code itself for any other.
export function failCode(err: unknown): string | undefined {
  if (!(err instanceof Fail)) return undefined
  if (OWN_CODES.includes(err.code)) {
    const at = err.message.indexOf(': ')
    return -1 === at ? err.message : err.message.substring(0, at)
  }
  return err.code
}

// The finer code of a failure that must be a `Fail` of one of this
// package's codes, or a description of what was thrown instead.
export function finer(err: unknown): string {
  return failCode(err) ?? `not a Fail: ${err}`
}

// What `fn` throws; fails the test when it returns.
export function thrown(fn: () => unknown): any {
  try {
    fn()
  } catch (err) {
    return err
  }
  throw new Error('expected a failure')
}

// The spec's worked example, byte for byte as aless's fixture has it
// (329 bytes; the metadata before the rows; Bob's members in another
// order; `50.25` and `72` as written).
export const RECORDS =
  '{"response":{"metadata":{"fields":[{"title":"Identifier","path":["id"]},{"title":"Full name","path":["person","name"]},{"title":"Balance","path":["account","balance"]}]},"payload":{"deep":{"records":[{"id":123,"person":{"name":"Alice"},"account":{"balance":50.25}},{"account":{"balance":72},"person":{"name":"Bob"},"id":456}]}}}}'

export const EXPECTED_CSV = '"Identifier","Full name","Balance"\r\n"123","Alice","50.25"\r\n"456","Bob","72"\r\n'

// The spec's program (sections 12.1 and 13.4).
export const PROGRAM =
  'def column-from-meta [source]\n  record\n    entry :label (get "title" source)\n    entry :source\n      as-path\n        get "path" source\n\ndef api-binding\n  record\n    entry :columns\n      path "response" "metadata" "fields"\n    entry :rows\n      path "response" "payload" "deep" "records" each-index\n    entry :column column-from-meta\n\ndef api-table [input]\n  table-from-json api-binding input\n\ndef export [input]\n  pipe input\n    api-table\n    csv csv-options\n'

// What a host does with a program and a JSON text (rs/tests/run_test.rs's
// `drive`): parse the text with the json grammar incrementally, pruned
// under the program's row selector when it has one, push the events into
// the program's sink, and mark a failure as leaving partial output when
// `output_bytes` says bytes reached the writer. The outcome, and the text
// the writer received.
export function drive(
  program: any,
  text: string,
  render?: 'csv' | 'json',
  limits: any = Limits.default(),
  metrics: any = new Metrics(),
): { fail?: any; out: string } {
  const writer = new BytesWriter()
  let sink: any
  try {
    sink = program.sink(writer, render, limits, metrics)
  } catch (fail) {
    return { fail, out: '' }
  }
  const selector = program.rowSelector()
  const prune = undefined === selector ? Prune.never() : Prune.under(selector)
  try {
    new ParserSource(makeJson(), text)
      .grammar('json')
      .mode(SourceMode.incremental(prune))
      .limits(limits)
      .metrics(metrics)
      .run(sink)
  } catch (fail) {
    if (fail instanceof Fail && metrics.output_bytes > 0 && !fail.committedOutput) fail.committed()
    return { fail, out: writer.text() }
  }
  return { out: writer.text() }
}

// The output of a run that must succeed.
export function ok(program: any, text: string, render?: 'csv' | 'json'): string {
  const { fail, out } = drive(program, text, render)
  if (undefined !== fail) throw fail
  return out
}

// The failure of a run that must fail, and what had been written.
export function err(program: any, text: string, limits: any = Limits.default()): { fail: any; out: string } {
  const { fail, out } = drive(program, text, undefined, limits)
  if (undefined === fail) throw new Error(`the run should fail, and wrote ${JSON.stringify(out)}`)
  return { fail, out }
}

// The events of one document, recorded once so several runs replay the
// same stream: the walk of the grammar's value (materialize mode), sound
// for every grammar. Undefined when the grammar refuses the document.
export function events(parser: any, text: string): any[] | undefined {
  const recorder = new EventRecorder()
  try {
    new ParserSource(parser, text).run(recorder)
  } catch (_fail) {
    return undefined
  }
  return recorder.events
}

// Replay `events` through `program`'s sink; the output, or the failure
// thrown.
export function replayed(
  program: any,
  evs: any[],
  render?: 'csv' | 'json',
  limits: any = Limits.default(),
  metrics: any = new Metrics(),
): string {
  const writer = new BytesWriter()
  const sink = program.sink(writer, render, limits, metrics)
  replay(evs, sink)
  return writer.text()
}
