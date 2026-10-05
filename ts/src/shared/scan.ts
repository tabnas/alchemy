/* Copyright (c) 2026 tabnas, MIT License */

// `scan-emit`'s step result, as data.
//
// The type of transduce's `scan.ts` that a program's step answers: the
// operator that owns the state and the iteration, `ScanEmit`, stays in
// transduce, which implements `Routers` (`./routers`).

// The result of one step: the next state and what to emit for it.
export type Transition<S, O> = { state: S; outputs: O[] }

export const Transition = Object.freeze({
  of<S, O>(state: S, outputs: O[]): Transition<S, O> {
    return { state, outputs }
  },
  // A step that emits nothing.
  stay<S, O>(state: S): Transition<S, O> {
    return { state, outputs: [] }
  },
  // A step that emits one item.
  emit<S, O>(state: S, output: O): Transition<S, O> {
    return { state, outputs: [output] }
  },
})
