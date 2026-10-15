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

// What a program's `export [input]` reads (rs/src/program.rs `Input`): the
// protocol the host pushes into the program's sink, which the host declares
// when it compiles the program (`CompileOptions.input`).
export type Input =
  // `JsonEvents/1`, a document's events, into `Program.sink`: the default.
  | 'JsonEvents/1'
  // `ParserEvents/1`, the parser's rule passes, into `Program.parserSink`:
  // what a transducer part reads, through `parser-events`.
  | 'ParserEvents/1'

// The output as the plan report names it.
export function outputText(output: Output): string {
  return output
}
