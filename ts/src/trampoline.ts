/* Copyright (c) 2026 tabnas, MIT License */

// The explicit stack the recursive walks run on.
//
// The checker's walks and the evaluator recurse per level of a form, into
// the bodies a function is applied to, and (the evaluator's) through a
// function handed itself until MAX_EVAL_DEPTH ends it. The Rust crate runs
// them on a thread of 64 MiB (`STACK_BYTES`); JavaScript has no such
// thread, and Node's default stack ends a few thousand frames in. So each
// walk is a generator, and a recursive call is a request: `yield
// this.eval(...)` hands the callee's generator to `run`, which drives every
// pending call on an array of its own and resumes the caller with the
// result, or throws the callee's failure into it, so a `finally` that
// gives back an evaluation level still runs. The JavaScript stack stays a
// few frames deep however deep the walk goes, and the bounds the language
// documents are reached as failures, never as a `RangeError`.

// A walk that answers `T`: it yields the generators of the calls it makes
// and is resumed with each one's answer.
export type G<T> = Generator<unknown, T, any>

// Drive `root` and every call it makes to completion, on an explicit
// stack; answer its result or throw its failure.
export function run<T>(root: G<T>): T {
  const stack: G<any>[] = [root]
  let input: any = undefined
  let failed = false
  let failure: unknown = undefined
  for (;;) {
    const top = stack[stack.length - 1]
    let step: IteratorResult<unknown, any>
    try {
      step = failed ? top.throw(failure) : top.next(input)
      failed = false
    } catch (err) {
      stack.pop()
      if (0 === stack.length) throw err
      failed = true
      failure = err
      continue
    }
    if (step.done) {
      stack.pop()
      if (0 === stack.length) return step.value
      input = step.value
    } else {
      stack.push(step.value as G<any>)
      input = undefined
    }
  }
}

// Whether `value` is a walk's generator rather than an answer: what a
// native that applies functions returns instead of its value.
export function isWalk(value: unknown): value is G<unknown> {
  return (
    null !== value &&
    'object' === typeof value &&
    'function' === typeof (value as any).next &&
    'function' === typeof (value as any).throw
  )
}
