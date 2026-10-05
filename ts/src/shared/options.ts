/* Copyright (c) 2026 tabnas, MIT License */

// The renderers' options: the CSV dialect, the JSON profile, and what a
// `missing` cell becomes in a record.
//
// The option types of render's `csv.ts`, `json.ts` and `records.ts`, here
// because a program chooses a dialect and a host's renderers take it: the
// renderers themselves (`CsvRenderer`, `JsonRenderer`, `RecordsToJson`)
// stay in render, which implements `Renderers` (`./renderers`).

// The record terminator: `crlf`, RFC 4180's and the standard profile's, or
// `lf`.
export type Newline = 'crlf' | 'lf'

export const Newline = Object.freeze({
  CRLF: 'crlf' as Newline,
  LF: 'lf' as Newline,
  // The terminator's text.
  text(newline: Newline): string {
    return 'lf' === newline ? '\n' : '\r\n'
  },
})

// When a field is quoted: `always`, the standard profile, or `minimal`,
// only a field holding the delimiter, `"`, CR or LF. Under `minimal` an
// empty field is written as nothing, so the empty string and an empty null
// text read back the same; that is the dialect's trade-off, not a defect.
export type Quoting = 'always' | 'minimal'

// What a `missing` cell becomes: an `error` (`MISSING_VALUE`: a table that
// promised a column and did not deliver it is not silently padded), or
// this `text` instead.
export type MissingText = { readonly type: 'error' } | { readonly type: 'text'; readonly text: string }

export const MissingText = Object.freeze({
  error: Object.freeze({ type: 'error' }) as MissingText,
  text(text: string): MissingText {
    return Object.freeze({ type: 'text', text })
  },
})

// The CSV dialect.
export type CsvOptions = {
  // One character, and not `"`, CR, LF or NUL: those would make the output
  // unreadable by construction, and are refused when the renderer is
  // built.
  delimiter: string
  newline: Newline
  // Write the labels as the first record.
  header: boolean
  // The text of a `null` cell; empty by default.
  nullText: string
  missing: MissingText
  quoting: Quoting
}

export const CsvOptions = Object.freeze({
  // The standard profile: `,`, CRLF, a header, `null` as the empty text,
  // `missing` an error, every field quoted.
  default(): CsvOptions {
    return {
      delimiter: ',',
      newline: 'crlf',
      header: true,
      nullText: '',
      missing: MissingText.error,
      quoting: 'always',
    }
  },
})

// The JSON profile.
export type JsonOptions = {
  // Spaces per nesting level, with a newline before every item and every
  // closing bracket of a non-empty container. `null` or `0` is compact: no
  // whitespace at all.
  indent: number | null
  // Write a newline after the root value, at the end.
  trailingNewline: boolean
}

export const JsonOptions = Object.freeze({
  // Compact, no trailing newline.
  default(): JsonOptions {
    return { indent: null, trailingNewline: false }
  },
})

// What a `missing` cell becomes in a record: `skip` leaves the member out
// (the record says nothing where the source had nothing, which is what an
// absent path meant), `null` writes the member with a `null` value, and
// `error` fails the run with `MISSING_VALUE`.
export type MissingRecord = 'skip' | 'null' | 'error'
