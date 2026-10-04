/* Copyright (c) 2026 tabnas, MIT License */

// Limits, metrics and cancellation.
//
// Limits are part of a plan, not advice: a stage that would exceed one
// fails with `RESOURCE_LIMIT_EXCEEDED` naming the field, rather than
// switching to a slower or larger algorithm. Values are payload bytes
// (UTF-8 lengths of keys and scalars, and the sum of them for a retained
// value plus a fixed per-node allowance), which is a stable and portable
// measure, the same in every runtime; they are not a heap measurement. A
// JavaScript string is UTF-16, so every count here goes through
// `utf8Bytes`, never `length`.

// Per-run limits. Every field is named in a failure as written here, which
// is why the fields keep the snake_case names every runtime shares.
export type Limits = {
  // Container nesting the source may reach.
  max_depth: number
  // Bytes in one object key.
  max_key_bytes: number
  // Bytes in one scalar (a string's text, a number's lexeme).
  max_scalar_bytes: number
  // Bytes retained for metadata (the column descriptors).
  max_metadata_bytes: number
  // Columns a schema may declare.
  max_columns: number
  // Bytes retained for one row before projection.
  max_record_bytes: number
  // Bytes retained for any one materialized capture.
  max_capture_bytes: number
  // Bytes the run may write; `null` for no limit.
  max_output_bytes: number | null
}

// The names of the `Limits` fields, in declaration order.
export const LIMIT_NAMES: readonly (keyof Limits)[] = Object.freeze([
  'max_depth',
  'max_key_bytes',
  'max_scalar_bytes',
  'max_metadata_bytes',
  'max_columns',
  'max_record_bytes',
  'max_capture_bytes',
  'max_output_bytes',
])

export const Limits = Object.freeze({
  // The defaults: 256, 64 KiB, 16 MiB, 16 MiB, 10 000, 64 MiB, 64 MiB, none.
  default(): Limits {
    return {
      max_depth: 256,
      max_key_bytes: 64 * 1024,
      max_scalar_bytes: 16 * 1024 * 1024,
      max_metadata_bytes: 16 * 1024 * 1024,
      max_columns: 10_000,
      max_record_bytes: 64 * 1024 * 1024,
      max_capture_bytes: 64 * 1024 * 1024,
      max_output_bytes: null,
    }
  },

  // No limit on anything that can be unlimited, and the largest values
  // otherwise. For tests and trusted, measured inputs only.
  unlimited(): Limits {
    const max = Number.MAX_SAFE_INTEGER
    return {
      max_depth: max,
      max_key_bytes: max,
      max_scalar_bytes: max,
      max_metadata_bytes: max,
      max_columns: max,
      max_record_bytes: max,
      max_capture_bytes: max,
      max_output_bytes: null,
    }
  },

  // The defaults with `fields` applied over them.
  with(fields: Partial<Limits>): Limits {
    return { ...Limits.default(), ...fields }
  },
})

// The fixed allowance counted for every retained node (a container, a
// member, an element) on top of its payload bytes.
export const NODE_BYTES = 16

// The UTF-8 length of a string: `é` is 2, `日` 3, `😀` 4. A lone
// surrogate counts 3, as its replacement character would.
export function utf8Bytes(s: string): number {
  const n = s.length
  // ASCII fast path: most keys and scalars never leave it.
  let i = 0
  for (; i < n; i++) {
    if (s.charCodeAt(i) >= 0x80) break
  }
  if (i === n) return n
  return Buffer.byteLength(s, 'utf8')
}

// Counters and high-water marks a run reports, shared between stages.
export class Metrics {
  // Source events seen.
  events = 0
  // Object keys seen.
  keys = 0
  // Scalars seen.
  scalars = 0
  // Rows delivered to the table protocol.
  rows = 0
  // Bytes currently held by materialized captures.
  captured_bytes = 0
  // The most bytes ever held by captures at once.
  captured_bytes_high = 0
  // The most bytes ever retained at once across every retaining stage.
  retained_bytes_high = 0
  // Bytes written to the output.
  output_bytes = 0

  // Account `bytes` as captured now, and raise the high-water marks.
  capture(bytes: number): void {
    this.captured_bytes += bytes
    if (this.captured_bytes > this.captured_bytes_high) {
      this.captured_bytes_high = this.captured_bytes
    }
    if (this.captured_bytes > this.retained_bytes_high) {
      this.retained_bytes_high = this.captured_bytes
    }
  }

  // Release `bytes` captured earlier.
  release(bytes: number): void {
    this.captured_bytes -= bytes
  }

  // The metrics as a plain object, one field per counter.
  toJSON(): Record<string, number> {
    return {
      events: this.events,
      keys: this.keys,
      scalars: this.scalars,
      rows: this.rows,
      captured_bytes: this.captured_bytes,
      captured_bytes_high: this.captured_bytes_high,
      retained_bytes_high: this.retained_bytes_high,
      output_bytes: this.output_bytes,
    }
  }
}

// A cancellation flag shared by the caller, the source and the stages.
//
// The source polls it between parse steps through the engine's parse
// budget and stops with `ABORTED`; long loops in stages poll it too.
export class AbortFlag {
  private aborted = false

  abort(): void {
    this.aborted = true
  }

  isAborted(): boolean {
    return this.aborted
  }
}
