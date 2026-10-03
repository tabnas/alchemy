/* Copyright (c) 2026 tabnas, MIT License */

// Whether a thrown value is a transduce `Fail`, whichever copy of
// transduce made it. TypeScript only: the Rust crate has nothing to port.
//
// A failure this package catches is not always one its own transduce
// made: render makes its failures with the transduce it resolves, which
// is a second copy whenever render carries one of its own. A workflow
// that copies a sibling checkout where it cannot symlink one, as the
// shared polyglot CI does on Windows, copies render's node_modules with
// it. Two copies are two `Fail` classes, and a failure from the other one
// is not `instanceof` this one's: the command reported render's
// MISSING_VALUE as an internal error, status 101, where it exits 1.
// transduce's own `isFail` asks `instanceof`, so it does not see past the
// copy either.
//
// What every copy shares is what its constructor sets: the name `Fail`
// and a string `code`. That is what is tested, after `instanceof`; with
// one copy every failure is an instance, and the answer is the one
// `instanceof` gives.

import { Fail } from '@tabnas/transduce'

// Whether `err` is a `Fail`: an instance of this package's transduce
// `Fail`, or an `Error` named `Fail` that carries a string `code`, as
// every copy of transduce makes its failures. Use it wherever a failure is
// told from a defect, never `instanceof Fail`.
export function isFail(err: unknown): err is Fail {
  if (err instanceof Fail) return true
  return err instanceof Error && 'Fail' === err.name && 'string' === typeof (err as { code?: unknown }).code
}
