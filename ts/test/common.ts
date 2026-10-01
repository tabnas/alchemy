/* Copyright (c) 2026 tabnas, MIT License */

// Shared helpers for the tests: where the repository and its fixtures
// are, and what a failure's finer code is (rs/tests/common/mod.rs).

import { join } from 'node:path'

import { findSpecDir } from '@tabnas/support'
import { Fail } from '@tabnas/transduce'

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
