/* Copyright (c) 2026 tabnas, MIT License */

// What a program's `export` produces (rs/src/program.rs `Output`): decided
// by the checker from `export`'s type, and so what the host renders.
export type Output =
  // The program renders its own text.
  | 'Text'
  // `TableRows/1`: the host renders it (CSV by default, or JSON records).
  | 'TableRows/1'
  // `JsonEvents/1`: the host renders it as JSON.
  | 'JsonEvents/1'

// The output as the plan report names it.
export function outputText(output: Output): string {
  return output
}
