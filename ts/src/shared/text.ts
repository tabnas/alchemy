/* Copyright (c) 2026 tabnas, MIT License */

// The text boundary: where rendered fragments go, and where a coalescing
// output sends its bytes.
//
// The interfaces of render's `text.ts`, here because a program's text
// stages write to them and a host hands one in: the outputs that implement
// them (`WriteOut`, `StringOut`, `Join`, `Concat`, `ReplaceText`) and the
// writers (`BytesWriter`, `FdWriter`) stay in render, which implements
// `Renderers` (`./renderers`). Every count is in UTF-8 bytes, never in
// string length.

// A consumer of text fragments.
//
// Fragments arrive in order and are concatenated; where the boundaries fall
// carries no meaning. `flush` pushes everything held so far to the final
// destination, and a renderer calls it exactly once, at the end of the
// protocol it renders, so that a document that failed half way is not
// flushed as if it were whole. Both throw a `Fail` to fail the run.
export interface TextOut {
  writeStr(s: string): void
  flush(): void

  // Whether any text has reached the final destination, so that a failure
  // found now leaves partial output behind. A renderer asks this when it
  // fails and reports `committedOutput` from the answer, which is how a
  // host knows to print `output: "partial"` rather than `"none"`. An
  // output that leaves it out gets the conservative answer, `true`
  // (render's `hasCommitted`, `Renderers.hasCommitted` here): whatever the
  // renderer handed over may be out.
  // `WriteOut` answers exactly, from the bytes its writer received; a
  // fragment that is still buffered is not committed, and `intoInner`
  // drops it rather than sending it after the fact.
  hasCommitted?(): boolean
}

// Where a `WriteOut` sends its bytes: the counterpart of Rust's
// `io::Write`.
//
// `write` takes what it can of `bytes` and returns how many it took; it
// may take fewer than offered (a short write), and `WriteOut` offers the
// rest again. Returning 0 means the writer can take nothing more, and is a
// failure, as `write_all` treats it. Throwing is a failure too. The bytes
// handed over are the writer's to keep: `WriteOut` never reuses a buffer
// it has handed over. `flush`, when present, pushes what the writer holds
// further on.
export interface Writer {
  write(bytes: Uint8Array): number
  flush?(): void
}
