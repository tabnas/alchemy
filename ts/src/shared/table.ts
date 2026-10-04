/* Copyright (c) 2026 tabnas, MIT License */

// `TableRows/1`: one schema, ordered finite rows, completion.
//
// The protocol is flat and budgeted: a row is a finite vector of cells,
// already projected into schema order by the transducer, and a renderer
// never sees a source path. Public columns carry a label; the source
// binding (`BoundColumn`) stays on the transducer's side of the boundary.

import { Datum, toText } from './datum'
import { Fail } from './error'
import { jsonNumber, jsonString } from './json'
import { NODE_BYTES, utf8Bytes } from './limits'
import { Segment, Selector } from './selector'
import { Flow } from './sink'

// One projected value. `missing`: the source had no value at the column's
// path; not null, not the empty string, not zero: a policy maps or rejects
// it later.
export type Cell =
  | { readonly type: 'null' }
  | { readonly type: 'bool'; readonly value: boolean }
  | { readonly type: 'number'; readonly value: number; readonly lexeme: string | null }
  | { readonly type: 'string'; readonly value: string }
  | { readonly type: 'missing' }

const NULL: Cell = Object.freeze({ type: 'null' })
const MISSING: Cell = Object.freeze({ type: 'missing' })

export const Cell = Object.freeze({
  null: NULL,
  missing: MISSING,
  bool(value: boolean): Cell {
    return { type: 'bool', value }
  },
  number(value: number, lexeme?: string | null): Cell {
    return { type: 'number', value, lexeme: lexeme ?? null }
  },
  string(value: string): Cell {
    return { type: 'string', value }
  },

  // From a retained value. A container is not a cell; it is serialized as
  // compact JSON text, the lossy but unambiguous choice the standard
  // binding makes and documents.
  fromDatum(d: Datum): Cell {
    switch (d.type) {
      case 'null':
        return NULL
      case 'bool':
        return { type: 'bool', value: d.value }
      case 'number':
        return { type: 'number', value: d.value, lexeme: d.lexeme }
      case 'string':
        return { type: 'string', value: d.value }
      default:
        return { type: 'string', value: toText(d) }
    }
  },

  // The bytes this cell retains, on the same basis as a datum's size.
  byteSize(c: Cell): number {
    switch (c.type) {
      case 'number':
        return NODE_BYTES + (null != c.lexeme ? utf8Bytes(c.lexeme) : 8)
      case 'string':
        return NODE_BYTES + utf8Bytes(c.value)
      default:
        return NODE_BYTES
    }
  },

  // The cell as JSON text; `missing` prints as `missing`.
  toText(c: Cell): string {
    switch (c.type) {
      case 'null':
        return 'null'
      case 'bool':
        return String(c.value)
      case 'number':
        return jsonNumber(c.value, c.lexeme)
      case 'string':
        return jsonString(c.value)
      case 'missing':
        return 'missing'
    }
  },
})

// What a renderer knows about a column.
export type PublicColumn = { label: string }

// One event of `TableRows/1`: the `schema` exactly once and first, a `row`
// per row (each exactly as wide as the schema), the `end` exactly once and
// last, and only after the source validated to its end.
export type TableEvent =
  | { readonly type: 'schema'; readonly columns: readonly PublicColumn[] }
  | { readonly type: 'row'; readonly cells: readonly Cell[] }
  | { readonly type: 'end' }

// A consumer of `TableRows/1`. Throws a `Fail` to fail the run.
export interface TableSink {
  tableEvent(ev: TableEvent): Flow
}

// An owned recording of a table, for tests and small results.
export class Table implements TableSink {
  columns: PublicColumn[] = []
  rows: Cell[][] = []
  ended = false

  tableEvent(ev: TableEvent): Flow {
    switch (ev.type) {
      case 'schema':
        this.columns = ev.columns.slice()
        break
      case 'row':
        this.rows.push(ev.cells.slice())
        break
      case 'end':
        this.ended = true
        break
    }
    return 'continue'
  }
}

// What to do when a row has no value at a column's path: deliver a
// `missing` cell (the default; the renderer's policy decides), deliver
// `null`, or fail the run with `MISSING_VALUE`.
export type MissingPolicy = 'missing' | 'null' | 'error'

// A column as the transducer binds it: the public label, and the source
// path projected from each row. Never crosses into a renderer.
export type BoundColumn = {
  label: string
  source: Segment[]
  missing: MissingPolicy
}

export function boundColumn(
  label: string,
  source: Segment[],
  missing: MissingPolicy = 'missing',
): BoundColumn {
  return { label, source, missing }
}

// Maps one metadata descriptor to a bound column; throws a `Fail` for a
// descriptor it cannot read.
export type ColumnMapper = (meta: Datum) => BoundColumn

// Where a table's columns come from: declared by the caller (`static`),
// selected from the source and mapped one descriptor at a time
// (`metadata`; the metadata must complete before the first row begins),
// or the first row's member names in its order (`infer`; data-dependent: a
// later row's extra members are dropped, its absent ones are `missing`).
export type Schema =
  | { type: 'static'; columns: BoundColumn[] }
  | { type: 'metadata'; columns: Selector; column: ColumnMapper }
  | { type: 'infer' }

export const Schema = Object.freeze({
  static(columns: BoundColumn[]): Schema {
    return { type: 'static', columns }
  },
  fromMetadata(columns: Selector, column: ColumnMapper = columnFromMeta): Schema {
    return { type: 'metadata', columns, column }
  },
  infer(): Schema {
    return { type: 'infer' }
  },
})

// A table transducer's source binding: the schema, and the rows (each
// location `rows` names is one row).
export type TableBinding = {
  schema: Schema
  rows: Selector
}

// The standard mapping from a metadata descriptor to a column: the spec's
// `column-from-meta`, reading `title` and a `path` of segments.
export function columnFromMeta(meta: Datum): BoundColumn {
  if ('object' !== meta.type) {
    throw Fail.input('a column descriptor is not an object')
  }
  const title = meta.members.get('title')
  if (undefined === title || 'string' !== title.type) {
    throw Fail.input('a column descriptor has no string "title"')
  }
  const label = title.value
  const path = meta.members.get('path')
  if (undefined === path || 'array' !== path.type) {
    throw Fail.input(`column ${JSON.stringify(label)} has no "path" array`)
  }
  const source: Segment[] = path.items.map((seg) => {
    if ('string' === seg.type) return seg.value
    if (
      'number' === seg.type &&
      seg.value >= 0 &&
      Number.isInteger(seg.value) &&
      seg.value <= 0xffffffff
    ) {
      return seg.value
    }
    throw Fail.input(
      `column ${JSON.stringify(label)} has a path segment that is neither a string nor a ` +
        `non-negative integer: ${toText(seg)}`,
    )
  })
  return boundColumn(label, source)
}
