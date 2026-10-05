/* Copyright (c) 2026 tabnas, MIT License */

// Whether a thrown value is a `Fail` (`./shared`), whichever copy of the
// shared unit made it. TypeScript only: the Rust crate has nothing to port.
//
// A failure this package catches is not always one its own shared unit
// made: transduce and render make their failures with the
// `@tabnas/alchemy/shared` they resolve, which is a second copy whenever
// one of them carries an alchemy of its own. A workflow that copies a
// sibling checkout where it cannot symlink one, as the shared polyglot CI
// does on Windows, copies its node_modules with it. Two copies are two
// `Fail` classes, and a failure from the other one is not `instanceof`
// this one's: the command reported render's MISSING_VALUE as an internal
// error, status 101, where it exits 1, when the class was transduce's and
// render carried a transduce of its own. The shared unit's own `isFail`
// asks `instanceof`, so it does not see past the copy either.
//
// What every copy shares is what its constructor sets: the name `Fail`
// and a string `code`. That is what is tested, after `instanceof`; with
// one copy every failure is an instance, and the answer is the one
// `instanceof` gives.

import { Fail } from './shared'

// Whether `err` is a `Fail`: an instance of this package's shared `Fail`,
// or an `Error` named `Fail` that carries a string `code`, as every copy of
// the shared unit makes its failures. Use it wherever a failure is told
// from a defect, never `instanceof Fail`.
export function isFail(err: unknown): err is Fail {
  if (err instanceof Fail) return true
  return err instanceof Error && 'Fail' === err.name && 'string' === typeof (err as { code?: unknown }).code
}
