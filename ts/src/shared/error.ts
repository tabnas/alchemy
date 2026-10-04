/* Copyright (c) 2026 tabnas, MIT License */

// Failures, with stable codes.
//
// The code is the contract: scripts and agents branch on it, and every
// renderer, transducer and host uses the same set. The message, path and
// limit are informative. A code is never renamed, removed or repurposed;
// one may be added.

const CODES = [
  'DSL_PARSE_ERROR',
  'DSL_TYPE_ERROR',
  'STREAM_REUSED',
  'STREAMABILITY_UNKNOWN',
  'INPUT_ORDER_VIOLATION',
  'CAPTURE_OVERLAP_UNSUPPORTED',
  'MISSING_VALUE',
  'DUPLICATE_MEMBER',
  'INVALID_NUMBER',
  'PROTOCOL_ORDER_ERROR',
  'TARGET_VALUE_UNREPRESENTABLE',
  'RESOURCE_LIMIT_EXCEEDED',
  'INPUT_INVALID',
  'OUTPUT_FAILED',
  'ABORTED',
] as const

// One of the stable failure codes, written in `SCREAMING_SNAKE_CASE`.
export type Code = (typeof CODES)[number]

type CodeTable = { readonly [K in Code]: K } & {
  // Every code, in declaration order (the Rust `Code::ALL`).
  readonly ALL: readonly Code[]
  // A code by its written form, or `undefined`.
  parse(text: string): Code | undefined
}

const table: any = {}
for (const c of CODES) table[c] = c
table.ALL = Object.freeze([...CODES])
table.parse = (text: string): Code | undefined =>
  (CODES as readonly string[]).includes(text) ? (text as Code) : undefined

// The codes as constants: `Code.INPUT_INVALID`, and `Code.ALL`.
export const Code: CodeTable = Object.freeze(table)

// The limit a `RESOURCE_LIMIT_EXCEEDED` failure names.
export type Limit = {
  // The `Limits` field, as written there (`max_record_bytes`).
  name: string
  value: number
}

// A failure: a code, and what is known about where and why.
//
// `committedOutput` says whether bytes had already been written when the
// failure was found: an incremental export cannot take them back, and the
// caller must be told the output may be partial.
export class Fail extends Error {
  code: Code
  // The input path the failure concerns, in jq syntax, when one applies.
  path?: string
  limit?: Limit
  // 1-based source row and column, when the failure has a position.
  row?: number
  col?: number
  // The source file the position is in, when the program the failure came
  // from was compiled from several.
  file?: string
  committedOutput: boolean

  constructor(code: Code, message: string) {
    super(message)
    this.name = 'Fail'
    this.code = code
    this.committedOutput = false
  }

  // The failure with the source file its position is in.
  inFile(file: string): this {
    this.file = file
    return this
  }

  atPath(path: string): this {
    this.path = path
    return this
  }

  at(row: number, col: number): this {
    this.row = row
    this.col = col
    return this
  }

  committed(): this {
    this.committedOutput = true
    return this
  }

  // A limit failure, named after the `Limits` field that was passed.
  static limit(name: string, value: number, message: string): Fail {
    const f = new Fail('RESOURCE_LIMIT_EXCEEDED', message)
    f.limit = { name, value }
    return f
  }

  static protocol(message: string): Fail {
    return new Fail('PROTOCOL_ORDER_ERROR', message)
  }

  static input(message: string): Fail {
    return new Fail('INPUT_INVALID', message)
  }

  static output(message: string): Fail {
    return new Fail('OUTPUT_FAILED', message)
  }

  static aborted(): Fail {
    return new Fail('ABORTED', 'the run was cancelled')
  }

  // The engine's own error, as an input failure carrying its code,
  // position and report.
  static fromTabnas(e: any): Fail {
    const f = new Fail('INPUT_INVALID', `${engineCode(e)}: ${engineDetail(e)}`)
    const pos = enginePosition(e)
    if (pos) {
      f.row = pos.row
      f.col = pos.col
    }
    return f
  }

  // The failure as a plain object: `code`, `message`, and `path`, `limit`
  // (`{name, value}`), `row`, `col`, `file`, `output` ("partial" or
  // "none") when they apply. This is the shape hosts print.
  toJSON(): Record<string, unknown> {
    const out: Record<string, unknown> = {
      code: this.code,
      message: this.message,
    }
    if (null != this.path) out.path = this.path
    if (null != this.limit) out.limit = { name: this.limit.name, value: this.limit.value }
    if (null != this.row) out.row = this.row
    if (null != this.col) out.col = this.col
    if (null != this.file) out.file = this.file
    out.output = this.committedOutput ? 'partial' : 'none'
    return out
  }

  toString(): string {
    let s = `${this.code}: ${this.message}`
    if (null != this.path) s += ` at ${this.path}`
    if (null != this.file && null != this.row && null != this.col) {
      s += ` (${this.file}:${this.row}:${this.col})`
    } else if (null != this.row && null != this.col) {
      s += ` (${this.row}:${this.col})`
    } else if (null != this.file) {
      s += ` (in ${this.file})`
    }
    if (null != this.limit) s += ` [${this.limit.name} = ${this.limit.value}]`
    return s
  }
}

// Whether a thrown value is a `Fail`.
export function isFail(e: unknown): e is Fail {
  return e instanceof Fail
}

// The engine error's code (`unexpected`, `cancel`, ...).
export function engineCode(e: any): string {
  return 'string' === typeof e?.code ? e.code : 'unknown'
}

// The engine error's one-line description, without the code.
export function engineDetail(e: any): string {
  let detail = ''
  try {
    if ('function' === typeof e?.toJSON) detail = e.toJSON()?.message ?? ''
  } catch (_err) {
    detail = ''
  }
  if ('' === detail) detail = String(e?.message ?? e)
  return detail.trimEnd()
}

// The engine error's 1-based position, when it has one.
export function enginePosition(e: any): { row: number; col: number } | undefined {
  const row = e?.lineNumber
  const col = e?.columnNumber
  if ('number' === typeof row && 0 < row) {
    return { row, col: 'number' === typeof col ? col : 0 }
  }
  return undefined
}
