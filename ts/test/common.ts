/* Copyright (c) 2026 tabnas, MIT License */

// Shared helpers for the tests: where the repository and its fixtures
// are, and what a failure's finer code is (rs/tests/common/mod.rs).
//
// This package depends on neither transduce nor render, so its tests
// compile, check, explain and evaluate, and never run a program: `compile`
// here takes inert stages (`OPTIONS`). The tests that run programs on
// transduce's routers and render's renderers, and run.tsv's rows, are
// alchemy-cli's (its ts/test), the composition root that depends on all
// three.

import { join } from 'node:path'

import { findSpecDir } from '@tabnas/support'

import * as alchemy from '../dist/alchemy'
import { isFail } from '../dist/fail'

// The repository root: two levels above dist-test/.
export const REPO_ROOT = join(__dirname, '..', '..')

// What every method of the inert stages does: a compile-only test that
// reaches a stage fails saying so, rather than running on nothing.
function noStage(): never {
  throw new Error('compile-only test: no stages')
}

// What a compile-only test passes `compile`: routers and renderers that
// build nothing. Building a plan asks nothing of them; only a sink does.
export const OPTIONS: alchemy.CompileOptions = {
  routers: {
    router: noStage,
    tableFromJson: noStage,
    scanEmit: noStage,
    guarded: noStage,
  },
  renderers: {
    json: noStage,
    csv: noStage,
    recordsToJson: noStage,
    join: noStage,
    replaceText: noStage,
    writeOut: noStage,
    hasCommitted: noStage,
    writeValue: noStage,
  },
}

// `compile` with the inert stages, `OPTIONS`.
export function compile(src: string, file: string): alchemy.Program {
  return alchemy.compile(src, file, OPTIONS)
}

// `compileSources` with the inert stages, `OPTIONS`.
export function compileSources(sources: ReadonlyArray<alchemy.Source>): alchemy.Program {
  return alchemy.compileSources(sources, OPTIONS)
}

// test/spec, found as every runtime finds it.
export const SPEC_DIR = findSpecDir(__dirname)

// The codes whose failures carry a finer code as the first word of the
// message, before `: `.
export const OWN_CODES = ['DSL_PARSE_ERROR', 'DSL_TYPE_ERROR', 'STREAM_REUSED', 'STREAMABILITY_UNKNOWN']

// The code a fixture pins for a failure: the finer code for this
// package's own codes, the transduce code itself for any other. A failure
// another copy of the shared unit made counts (`isFail`).
export function failCode(err: unknown): string | undefined {
  if (!isFail(err)) return undefined
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

// The spec's program (sections 12.1 and 13.4).
export const PROGRAM =
  'def column-from-meta [source]\n  record\n    entry :label (get "title" source)\n    entry :source\n      as-path\n        get "path" source\n\ndef api-binding\n  record\n    entry :columns\n      path "response" "metadata" "fields"\n    entry :rows\n      path "response" "payload" "deep" "records" each-index\n    entry :column column-from-meta\n\ndef api-table [input]\n  table-from-json api-binding input\n\ndef export [input]\n  pipe input\n    api-table\n    csv csv-options\n'
