/* Copyright (c) 2026 tabnas, MIT License */

// Lowering: a plan becomes a chain of transduce and render sinks. A port of
// rs/src/lower.rs.
//
// The evaluator answers a plan; the host has a `JsonEvents/1` producer and
// a text output. This module joins them. The chain is push-based and
// synchronous, as the two packages are: the host calls the returned `Sink`
// once per event and once with `end`, each stage does its work and calls
// the next, and the output is flushed once, at the end, by the stage that
// owns it. Four kinds of stage exist, one per protocol boundary:
//
// - events (`JsonEvents/1`): the host's input, a `records` stage
//   (`RecordsToJson`) over table events, or an adapter (`TaggedToJson`)
//   reading the tagged events an interpreted stream yields, the reverse of
//   `events`;
// - table (`TableRows/1`): an adapter (`TaggedToTable`) reading the tagged
//   `schema`, `row` and `table-end` values a stream yields;
// - items (a stream of values): `Router` over events for `route` and
//   `select`, `EventsToItems` for `events`, `ScanEmit` for `scan-emit`,
//   per-item stages for `map` and `filter`, and an adapter turning native
//   table events into the tagged values a program pattern-matches;
// - text: `CsvRenderer` and `JsonRenderer` over their protocols,
//   `concat-map` and `join` over a stream of items, `replace-text` as
//   `ReplaceText`, and `concat` around one live text as a frame that writes
//   its prefix before the first fragment and its suffix at the flush.
//
// A text that does not reach the input is finite and is written whole
// wherever it lands (`writeFinite`): a row of an interpreted CSV is
// rendered into a scratch string and written as one fragment, so a failure
// in the middle of a row leaves no half row behind, as the host's CSV renderer
// promises. Errors are `Fail`s (`./shared`) with the transduce and render
// codes unchanged; a stream given to a stage of another protocol is
// `DSL_TYPE_ERROR` with the `protocol_mismatch` finer code, the same word
// the checker uses when it can see it first.
//
// This module constructs no stage of either package itself: it builds them
// through the routers and renderers the host passed to `compile`
// (`Runtime.routers`, `Runtime.renderers`), which alchemy declares
// (`Routers`, `Renderers`) and transduce and render implement.

import {
  CaptureSpec,
  Cell,
  CsvOptions,
  Datum,
  Ev,
  Fail,
  Flow,
  JoinOut,
  JsonEvent,
  JsonOptions,
  Limits,
  Metrics,
  NODE_BYTES,
  ParserEvent,
  ParserSink,
  PublicColumn,
  Renderers,
  RouteSink,
  RulePass,
  Scan,
  Selected,
  Sink,
  TableEvent,
  TableSink,
  TextOut,
  Transition,
  TreeContract,
  utf8Bytes,
} from './shared'

import { SourceSpan } from './ast'
import { isFail } from './fail'
import type { Bounds, Runtime } from './interp'
import type { Output } from './output'
import { captureBudget, getField, noColumnsEmpty, nonFinite, numberText, truth } from './stdlib/natives'
import { G, run } from './trampoline'
import {
  Func,
  NULL,
  NonFinite,
  Plan,
  Protocol,
  Val,
  bool,
  debugText,
  field,
  fromDatum,
  isLive,
  isLiveVal,
  isMissing,
  isRecordEnd,
  keyword,
  kindText,
  missing,
  nonFiniteWord,
  num,
  planName,
  protocol,
  record,
  same,
  selectorSegments,
  str,
  taggedVal,
  textVal,
  typeError,
  valEquals,
  vectorVal,
} from './value'

// A consumer of a stream's items.
export interface ItemSink {
  item(v: Val): Flow
  // The stream ended, validated. Exactly once, after the last item.
  end(): Flow
}

// How the host renders a stream result: CSV for table events (the
// default), JSON as records; JSON for a JSON-events result.
export type Renderer = 'csv' | 'json'

// The renderer by the name the command line uses.
export function rendererNamed(name: string): Renderer | undefined {
  return 'csv' === name || 'json' === name ? name : undefined
}

// The options the `json` renderer and the `--render json` echo use:
// compact, one document, a final newline.
export function jsonOptions(): JsonOptions {
  return { indent: null, trailingNewline: true }
}

function protocolMismatch(message: string): Fail {
  return new Fail('DSL_TYPE_ERROR', `protocol_mismatch: ${message}`)
}

// The `json` renderer's adapter for `:non-finite :null`: a number that is
// not finite goes on as null, everything else as it came.
class NullNonFinite implements Sink {
  constructor(private readonly next: Sink) {}

  event(ev: JsonEvent): Flow {
    if ('number' === ev.type && !Number.isFinite(ev.value)) return this.next.event(Ev.null)
    return this.next.event(ev)
  }
}

// A cell as the table protocol carries it, from the value a program
// produced: the scalars as themselves, `missing` as missing, a vector or a
// record as its compact JSON text, as `scalar-text` writes it, under the
// same `max_scalar_bytes`.
function cell(rt: Runtime, v: Val): Cell {
  switch (v.v) {
    case 'null':
      return Cell.null
    case 'bool':
      return Cell.bool(v.value)
    case 'num':
      return Cell.number(v.value, v.lexeme ?? null)
    case 'str':
      return Cell.string(v.value)
    case 'vector':
    case 'record':
      return Cell.string(rt.jsonText(v))
    default:
      if (isMissing(v)) return Cell.missing
      throw typeError(`a cell must be a scalar, a vector or a record, not ${kindText(v)}`)
  }
}

// The text of a column's label, one policy for every table: a string as it
// is, a number by its lexeme, a boolean by its name (as `scalar-text` would
// write the same header cell); `missing` is `MISSING_VALUE`, and anything
// else `INPUT_INVALID`. The adapter reading a program's `schema` values
// takes it, and `csv-table` validates the library's schema by it, so a
// label is accepted or refused
// alike whichever renders it.
function labelText(label: Val): string {
  switch (label.v) {
    case 'str':
      return label.value
    case 'num':
      return numberText(label.value, label.lexeme)
    case 'bool':
      return label.value ? 'true' : 'false'
    default:
      if (isMissing(label)) throw new Fail('MISSING_VALUE', 'a column has no label')
      throw Fail.input(`a column's label must be a string, a number or a boolean, not ${kindText(label)}`)
  }
}

// The label of a `schema` value's column: a record's `:label`.
function label(column: Val): string {
  const l = field(column, 'label')
  if (undefined === l) throw Fail.protocol('a schema column is not a record')
  return labelText(l)
}

// The columns of a `schema` value, as the table protocol carries them: a
// vector of column records, at most the runtime's `max_columns` of them.
function schemaColumns(rt: Runtime, fields: ReadonlyArray<Val>): PublicColumn[] {
  const limits = rt.limits
  const columns = fields[0]
  if ('vector' !== columns.v) throw Fail.protocol('schema takes a vector of columns')
  if (columns.items.length > limits.max_columns) {
    throw Fail.limit(
      'max_columns',
      limits.max_columns,
      `the schema declares ${columns.items.length} columns, more than ${limits.max_columns}`,
    )
  }
  return columns.items.map((c) => ({ label: label(c) }))
}

// The cells of a `row` value, as the table protocol carries them.
function rowCells(rt: Runtime, fields: ReadonlyArray<Val>): Cell[] {
  const values = fields[0]
  if ('vector' !== values.v) throw Fail.protocol('row takes a vector of cells')
  return values.items.map((c) => cell(rt, c))
}

// A value for a message: its kind, and at most a short prefix of its text,
// so a failure over a large document does not carry the document.
export function brief(v: Val): string {
  const MAX = 60
  // Past this many UTF-16 units the text holds more than MAX characters,
  // so the debug text need go no further.
  const text = debugText(v, 4 * MAX)
  const chars = [...text]
  if (chars.length <= MAX) return text
  return `${kindText(v)} (${chars.slice(0, MAX).join('')}...)`
}

// Write a finite text or string to `out`, whole. Every node takes an
// evaluation step and a level (`tick`, `enter`), so the host's abort flag
// stops a text of exponentially many fragments, and a `concat-map` whose
// function answers a text that applies it again is the `recursion` failure
// rather than a stack overflow. A walk on the evaluator's explicit stack:
// `run(writeFinite(...))` writes it. A `join` writes `:record-end` among
// its items as `TextOut.recordEnd`, no item and no text.
export function* writeFinite(rt: Runtime, v: Val, out: TextOut): G<void> {
  rt.tick()
  const at = 'text' === v.v && 'concat-map' === v.plan.p ? v.plan.at : undefined
  rt.enter(at)
  try {
    if ('str' === v.v) {
      out.writeStr(v.value)
      return
    }
    if ('text' !== v.v) throw typeError(`a string or a text was expected, not ${kindText(v)}`)
    const plan = v.plan
    switch (plan.p) {
      case 'lit':
        out.writeStr(plan.text)
        return
      case 'concat':
        for (const item of plan.items) yield writeFinite(rt, item, out)
        return
      case 'join':
        if ('vector' !== plan.items.seq) break
        {
          const join = rt.renderers.join(out, plan.sep)
          for (const item of plan.items.items) {
            if (isRecordEnd(item)) {
              join.recordEnd?.()
              continue
            }
            join.itemStart()
            yield writeFinite(rt, item, new Unmarked(join, rt.renderers))
            join.itemEnd()
          }
        }
        return
      case 'concat-map':
        if ('vector' !== plan.items.seq) break
        for (const item of plan.items.items) {
          const text: Val = yield rt.apply(plan.f, [item], plan.at)
          yield writeFinite(rt, text, new Unmarked(out, rt.renderers))
          out.recordEnd?.()
        }
        return
      // Streamed through the replacer, as a live text is: it holds at most
      // the literal's length, never the text.
      case 'replace': {
        const replace = rt.renderers.replaceText(new Held(out, rt.renderers), plan.from, plan.to)
        yield writeFinite(rt, plan.source, replace)
        replace.flush()
        return
      }
      default:
        break
    }
    throw typeError(`${planName(plan)} is a live text and cannot be written as a value`)
  } finally {
    rt.leave()
  }
}

// An output that passes fragments on and keeps its flush: a combinator
// written into the middle of a text (`replace-text` inside a `concat`)
// hands over what it holds at its own flush, and the text around it goes
// on.
class Held implements TextOut {
  constructor(
    private readonly inner: TextOut,
    private readonly renderers: Renderers,
  ) {}

  writeStr(s: string): void {
    this.inner.writeStr(s)
  }

  flush(): void {}

  hasCommitted(): boolean {
    return this.renderers.hasCommitted(this.inner)
  }

  recordEnd(): void {
    this.inner.recordEnd?.()
  }
}

// An item of a `join` or a `concat-map` over a vector, written through to
// the output beneath with its own record ends dropped (it has no
// `recordEnd`), as the stream stages' `Scratch` drops them: an item is
// written whole, and the marks of the text are the ones between its items.
// Its flush is kept, as `Held`'s is.
class Unmarked implements TextOut {
  constructor(
    private readonly inner: TextOut,
    private readonly renderers: Renderers,
  ) {}

  writeStr(s: string): void {
    this.inner.writeStr(s)
  }

  flush(): void {}

  hasCommitted(): boolean {
    return this.renderers.hasCommitted(this.inner)
  }
}

// One item's text, assembled whole before it is written, so a failure half
// way through an item (a missing cell in a row) leaves nothing of it
// behind. It is bounded by `max_output_bytes`: an item longer than the
// whole output may be could never be written, and assembling it anyway is
// what would let a small program exhaust memory under an output limit. A
// record end inside an item marks nothing (it has no `recordEnd`): the item
// is written whole.
class Scratch implements TextOut {
  text = ''
  private bytes = 0
  private readonly max: number | null

  constructor(limits: Limits) {
    this.max = limits.max_output_bytes ?? null
  }

  clear(): void {
    this.text = ''
    this.bytes = 0
  }

  writeStr(s: string): void {
    const n = utf8Bytes(s)
    if (null !== this.max && this.bytes + n > this.max) {
      const len = this.bytes + n
      throw Fail.limit(
        'max_output_bytes',
        this.max,
        `one item's text reached ${len} bytes; the output may not exceed ${this.max}`,
      )
    }
    this.text += s
    this.bytes += n
  }

  flush(): void {}

  hasCommitted(): boolean {
    return false
  }
}

// ---------------------------------------------------------------------------
// Item stages
// ---------------------------------------------------------------------------

// `route` and `select`: what the router delivers, as items.
class RouteToItems implements RouteSink {
  constructor(
    private readonly down: ItemSink,
    // `select` delivers the values; `route` wraps each in `selected`.
    private readonly valuesOnly: boolean,
  ) {}

  selected(selected: Selected): Flow {
    const value = fromDatum(selected.value ?? Datum.null)
    const item = this.valuesOnly ? value : taggedVal('selected', [keyword(selected.tag), value])
    return this.down.item(item)
  }

  end(): Flow {
    return this.down.end()
  }
}

// `events`: every event of `JsonEvents/1` as the tagged value a program
// matches on, pushed down as it arrives, and `end` as the stream's end.
// Nothing is kept between events: a key is delivered as it is read, twice
// when the source repeats it, and a scalar with its lexeme.
class EventsToItems implements Sink {
  constructor(private readonly down: ItemSink) {}

  event(ev: JsonEvent): Flow {
    let item: Val
    switch (ev.type) {
      case 'object_start':
        item = taggedVal('object-start')
        break
      case 'object_end':
        item = taggedVal('object-end')
        break
      case 'array_start':
        item = taggedVal('array-start')
        break
      case 'array_end':
        item = taggedVal('array-end')
        break
      case 'key':
        item = taggedVal('key', [str(ev.key)])
        break
      case 'null':
        item = taggedVal('scalar', [NULL])
        break
      case 'bool':
        item = taggedVal('scalar', [bool(ev.value)])
        break
      case 'number':
        item = taggedVal('scalar', [num(ev.value, ev.lexeme ?? undefined)])
        break
      case 'string':
        item = taggedVal('scalar', [str(ev.value)])
        break
      case 'end':
        return this.down.end()
    }
    return this.down.item(item)
  }
}

// The parser's rule passes as the items `parser-events` yields: `(open
// rule)` and `(close rule)`, the rule a record of the pass's fields, a
// field the pass lacks left out (`get` answers `missing` for it). The
// parse's `end` is the stream's end. The head of a program's chain when
// the host declares parser events: the host pushes each pass into it.
export class ParserEventsToItems implements ParserSink {
  constructor(private readonly down: ItemSink) {}

  event(ev: ParserEvent): Flow {
    if ('end' === ev.type) return this.down.end()
    // A host builds the event, and one the types do not describe (from
    // JavaScript, say) is refused, not read.
    if ('open' !== ev.type && 'close' !== ev.type) throw Fail.protocol('an unknown parser event kind')
    if (null == ev.pass) throw Fail.protocol(`a parser event's ${ev.type} carries no rule pass`)
    return this.down.item(taggedVal(ev.type, [ruleRecord(ev.pass)]))
  }
}

// A rule pass as the record `(open rule)` and `(close rule)` carry.
function ruleRecord(pass: RulePass): Val {
  const fields: Array<[string, Val]> = [
    ['name', str(pass.name)],
    ['d', num(pass.d)],
    ['i', num(pass.i)],
  ]
  if (undefined !== pass.alt) {
    fields.push([
      'alt',
      record([
        ['p', str(pass.alt.p)],
        ['r', str(pass.alt.r)],
        ['b', num(pass.alt.b)],
        ['g', vectorVal(pass.alt.g.map((g) => str(g)))],
      ]),
    ])
  }
  fields.push([
    'tokens',
    vectorVal(
      pass.tokens.map((t) =>
        record([
          ['name', str(t.name)],
          ['val', fromDatum(t.val)],
          ['src', str(t.src)],
          ['row', num(t.row)],
          ['col', num(t.col)],
        ]),
      ),
    ),
  ])
  fields.push(['forced', bool(pass.forced)])
  fields.push(['n', record(pass.n.map(([k, v]): [string, Val] => [k, num(v)]))])
  if (undefined !== pass.value) fields.push(['value', fromDatum(pass.value)])
  return record(fields)
}

// What the host pushes parser events into: the chain's head
// (`ParserEventsToItems`) when the program reads them, and otherwise (a
// text of the program's own, which reads nothing of its input) the
// parse's end alone, handed to the sink that writes the text once the
// input has validated.
class ParserHead implements ParserSink {
  constructor(
    private readonly head: ParserSink | undefined,
    private readonly events: Sink,
  ) {}

  event(ev: ParserEvent): Flow {
    if (undefined !== this.head) return this.head.event(ev)
    return 'end' === ev.type ? this.events.event(Ev.end) : 'continue'
  }
}

// The JSON-events sink `Lowering.items` answers where it lowers
// `parser-events`: the host pushes the rule passes into the head it keeps
// instead (`Lowering.parserHead`), never JSON events into this.
export class ParserEventsOnly implements Sink {
  event(_ev: JsonEvent): Flow {
    throw protocolMismatch('the program reads parser events; push them into Program.parserSink')
  }
}

// The tagged events an interpreted stream yields, as the JSON events a
// taker of `JsonEvents` reads (`json`, `table-from-json`, `select`,
// `route`, `events` again): the reverse of `EventsToItems`, so a program
// can rewrite a document event by event (`events` through a `scan-emit`),
// or build events with their constructors, and hand them on. Each item must
// be an event of the shape the constructors make; anything else is
// `PROTOCOL_ORDER_ERROR` naming it. The sequence is the taker's to
// validate, as the source's own events are, and the stream's end is the
// events' `end`. The sink beneath is the source's guard (`Guarded`), so the
// source's limits hold on these events as on its own.
class TaggedToJson implements ItemSink {
  constructor(private readonly down: Sink) {}

  item(v: Val): Flow {
    if ('tagged' !== v.v) throw Fail.protocol(`an event was expected, not ${kindText(v)}`)
    const fields = v.fields
    const one = fields[0]
    let event: JsonEvent | undefined
    if (0 === fields.length) {
      switch (v.tag) {
        case 'object-start':
          event = Ev.objectStart
          break
        case 'object-end':
          event = Ev.objectEnd
          break
        case 'array-start':
          event = Ev.arrayStart
          break
        case 'array-end':
          event = Ev.arrayEnd
          break
      }
    } else if (1 === fields.length && 'key' === v.tag && 'str' === one.v) {
      event = Ev.key(one.value)
    } else if (1 === fields.length && 'scalar' === v.tag) {
      switch (one.v) {
        case 'null':
          event = Ev.null
          break
        case 'bool':
          event = Ev.bool(one.value)
          break
        case 'num':
          event = Ev.number(one.value, one.lexeme ?? null)
          break
        case 'str':
          event = Ev.string(one.value)
          break
      }
    }
    if (undefined === event) throw Fail.protocol(`an event was expected, not ${brief(v)}`)
    return this.down.event(event)
  }

  end(): Flow {
    return this.down.event(Ev.end)
  }
}

// What a `scan-emit` state may hold: what a stage keeps from row to row, as
// the library's table keeps its bound columns, so `max_metadata_bytes` in the
// transduce measure; and no deeper than a captured value may be,
// `max_depth`, since a state that wraps itself once per item would
// otherwise nest without bound.
export function stateBounds(limits: Limits): Bounds {
  return {
    nodeBytes: NODE_BYTES,
    maxBytes: limits.max_metadata_bytes,
    bytesLimit: 'max_metadata_bytes',
    maxDepth: limits.max_depth,
    what: 'the scan-emit state',
  }
}

// Raise `retained_bytes_high` to what the captures hold now and `bytes`.
function retain(metrics: Metrics, bytes: number): void {
  metrics.retained_bytes_high = Math.max(metrics.retained_bytes_high, metrics.captured_bytes + bytes)
}

// `failAt` for whatever was thrown: a `Fail` from any copy of the shared
// unit (`isFail`), and nothing else.
function positioned(rt: Runtime, err: unknown, at: SourceSpan): unknown {
  return isFail(err) ? rt.failAt(err, at) : err
}

// `scan-emit`: transduce's operator over the program's step and finish.
class ScanStage implements ItemSink {
  private readonly scan: Scan<Val>

  constructor(
    rt: Runtime,
    init: Val,
    step: Func,
    finish: Func,
    at: SourceSpan,
    metrics: Metrics,
    private readonly down: ItemSink,
  ) {
    // The initial state is retained like any the step returns, and the step
    // measures only a state that changed: a step that hands the same one
    // back, or a source with no items, would never measure it, so it is
    // measured here, before anything is read.
    const measure = (state: Val): number => {
      try {
        return rt.measure(state, stateBounds(rt.limits)).bytes
      } catch (err) {
        throw positioned(rt, err, at)
      }
    }
    retain(metrics, measure(init))
    const stepFn = (state: Val, item: Val): Transition<Val, Val> => {
      const before = state
      const t = rt.applyNow(step, [state, item], at)
      if ('tagged' === t.v && 'transition' === t.tag && 2 === t.fields.length) {
        const outputs = t.fields[1]
        if ('vector' !== outputs.v) {
          throw rt.failAt(typeError("scan-emit: a transition's outputs must be a vector"), at)
        }
        // The state is what this stage retains across items: measured when
        // it changes, against the limits that bound what a stage keeps from
        // row to row, and reported with the captures in
        // `retained_bytes_high`.
        if (!same(t.fields[0], before)) retain(metrics, measure(t.fields[0]))
        return Transition.of(t.fields[0], outputs.items.slice())
      }
      throw rt.failAt(typeError(`scan-emit: the step must answer a transition, not ${kindText(t)}`), at)
    }
    const finishFn = (state: Val): Val[] => {
      const outputs = rt.applyNow(finish, [state], at)
      if ('vector' === outputs.v) return outputs.items.slice()
      throw rt.failAt(typeError(`scan-emit: finish must answer a vector of outputs, not ${kindText(outputs)}`), at)
    }
    this.scan = rt.routers.scanEmit<Val, Val, Val>(init, stepFn, finishFn, (o) => this.down.item(o))
  }

  item(v: Val): Flow {
    return this.scan.item(v)
  }

  end(): Flow {
    if ('stop' === this.scan.finish()) return 'stop'
    return this.down.end()
  }
}

// `map` over a stream.
class MapStage implements ItemSink {
  constructor(
    private readonly rt: Runtime,
    private readonly f: Func,
    private readonly at: SourceSpan,
    private readonly down: ItemSink,
  ) {}

  item(v: Val): Flow {
    const mapped = this.rt.applyNow(this.f, [v], this.at)
    if (isLiveVal(mapped)) {
      throw this.rt.failAt(typeError('map: the function must answer a value per item, not a live stream'), this.at)
    }
    return this.down.item(mapped)
  }

  end(): Flow {
    return this.down.end()
  }
}

// `filter` over a stream.
class FilterStage implements ItemSink {
  constructor(
    private readonly rt: Runtime,
    private readonly f: Func,
    private readonly at: SourceSpan,
    private readonly down: ItemSink,
  ) {}

  item(v: Val): Flow {
    const keep = this.rt.applyNow(this.f, [v], this.at)
    let kept: boolean
    try {
      kept = truth('filter', keep)
    } catch (err) {
      throw positioned(this.rt, err, this.at)
    }
    return kept ? this.down.item(v) : 'continue'
  }

  end(): Flow {
    return this.down.end()
  }
}

// The tagged values a stream yields, as table events. One schema first, rows, one `table-end`: the renderer beneath validates
// the sequence; what is validated here is that each item is a table event
// at all, of the table protocol's shape (at most `max_columns` columns,
// labels by `labelText`), and that the stream did end with `table-end`. It
// is the last table stage before a renderer, so it counts each row in
// `metrics.rows`, unless `countRows` says a later stage does
// (`Lowering.items`).
class TaggedToTable implements ItemSink {
  private ended = false

  constructor(
    private readonly rt: Runtime,
    private readonly metrics: Metrics,
    private readonly countRows: boolean,
    private readonly table: TableSink,
  ) {}

  item(v: Val): Flow {
    if ('tagged' !== v.v) throw Fail.protocol(`a table event was expected, not ${kindText(v)}`)
    if ('schema' === v.tag && 1 === v.fields.length) {
      return this.table.tableEvent({ type: 'schema', columns: schemaColumns(this.rt, v.fields) })
    }
    if ('row' === v.tag && 1 === v.fields.length) {
      const cells = rowCells(this.rt, v.fields)
      if (this.countRows) this.metrics.rows += 1
      return this.table.tableEvent({ type: 'row', cells })
    }
    if ('table-end' === v.tag && 0 === v.fields.length) {
      this.ended = true
      return this.table.tableEvent({ type: 'end' })
    }
    throw Fail.protocol(`a table event was expected, not ${brief(v)}`)
  }

  end(): Flow {
    if (this.ended) return 'continue'
    throw Fail.protocol('the table events ended without table-end')
  }
}

type Phase = 'before_schema' | 'rows' | 'done'

// `csv-table options events`: the protocol validator of spec section 13.2
// in the library's own `csv`. It checks what the host's CSV renderer checks
// of a table result, in the same order and with the same codes (the adapter
// above, then the renderer: the shape of each event, `max_columns` and the
// labels, then one schema first, at least one column, rows as wide as the
// schema, one `table-end`), and passes each event on unchanged for the text
// to render; the cells' own checks (a number's lexeme, a missing value) are
// the text's `scalar-text`, as they are the renderer's.
class CsvTableStage implements ItemSink {
  private phase: Phase = 'before_schema'
  private width = 0
  private rows = 0

  constructor(
    private readonly rt: Runtime,
    private readonly metrics: Metrics,
    // The options say `(entry :no-columns :empty)`: a table of no columns
    // passes, for the text to write as the empty document.
    private readonly noColumns: boolean,
    // Whether this stage counts the rows in `metrics.rows`: when no later
    // table stage does (`Lowering.items`).
    private readonly countRows: boolean,
    private readonly down: ItemSink,
  ) {}

  item(v: Val): Flow {
    if ('tagged' !== v.v) throw Fail.protocol(`a table event was expected, not ${kindText(v)}`)
    if ('schema' === v.tag && 1 === v.fields.length) {
      const columns = schemaColumns(this.rt, v.fields)
      if ('rows' === this.phase) throw Fail.protocol('a second schema')
      if ('done' === this.phase) throw Fail.protocol('a schema after the end')
      if (0 === columns.length && !this.noColumns) {
        throw new Fail('TARGET_VALUE_UNREPRESENTABLE', 'a table with no columns has no CSV form')
      }
      this.width = columns.length
      this.phase = 'rows'
    } else if ('row' === v.tag && 1 === v.fields.length) {
      const cells = rowCells(this.rt, v.fields)
      if ('before_schema' === this.phase) throw Fail.protocol('a row before the schema')
      if ('done' === this.phase) throw Fail.protocol('a row after the end')
      if (cells.length !== this.width) {
        throw Fail.protocol(`row ${this.rows + 1} has ${cells.length} cells; the schema has ${this.width} columns`)
      }
      this.rows += 1
      if (this.countRows) this.metrics.rows += 1
    } else if ('table-end' === v.tag && 0 === v.fields.length) {
      if ('before_schema' === this.phase) throw Fail.protocol('the end before the schema')
      if ('done' === this.phase) throw Fail.protocol('a second end')
      this.phase = 'done'
    } else {
      throw Fail.protocol(`a table event was expected, not ${brief(v)}`)
    }
    return this.down.item(v)
  }

  end(): Flow {
    if ('done' !== this.phase) throw Fail.protocol('the table events ended without table-end')
    return this.down.end()
  }
}

// The delimiter a `csv-table`'s options name, refused when no CSV reader
// could take it: the quote, a line break or NUL, as the renderer refuses
// one when it is built. A delimiter that is not a string is the text's to
// refuse, where it joins the fields.
function checkDelimiter(options: Val): void {
  const d = field(options, 'delimiter')
  if (undefined !== d && 'str' === d.v && /["\r\n\0]/.test(d.value)) {
    throw new Fail(
      'TARGET_VALUE_UNREPRESENTABLE',
      `${JSON.stringify(d.value)} cannot be a CSV delimiter: it holds the quote, a line break or NUL`,
    )
  }
}

// ---------------------------------------------------------------------------
// Text stages
// ---------------------------------------------------------------------------

// `concat-map` over a stream: each item's text, rendered whole, and the end
// of a record after it.
class ConcatMapStage implements ItemSink {
  private readonly scratch: Scratch

  constructor(
    private readonly rt: Runtime,
    private readonly f: Func,
    private readonly at: SourceSpan,
    private readonly out: TextOut,
    limits: Limits,
  ) {
    this.scratch = new Scratch(limits)
  }

  item(v: Val): Flow {
    const text = this.rt.applyNow(this.f, [v], this.at)
    this.scratch.clear()
    try {
      run(writeFinite(this.rt, text, this.scratch))
    } catch (err) {
      throw positioned(this.rt, err, this.at)
    }
    this.out.writeStr(this.scratch.text)
    this.out.recordEnd?.()
    return 'continue'
  }

  end(): Flow {
    this.out.flush()
    return 'continue'
  }
}

// `join` over a stream: each item is one logical item of the join;
// `:record-end` is none, and marks the end of a record.
class JoinStage implements ItemSink {
  private readonly scratch: Scratch
  private readonly join: JoinOut

  constructor(
    private readonly rt: Runtime,
    out: TextOut,
    sep: string,
    limits: Limits,
  ) {
    this.join = rt.renderers.join(out, sep)
    this.scratch = new Scratch(limits)
  }

  item(v: Val): Flow {
    if (isRecordEnd(v)) {
      this.join.recordEnd?.()
      return 'continue'
    }
    this.scratch.clear()
    run(writeFinite(this.rt, v, this.scratch))
    this.join.itemStart()
    this.join.writeStr(this.scratch.text)
    this.join.itemEnd()
    return 'continue'
  }

  end(): Flow {
    this.join.flush()
    return 'continue'
  }
}

// `concat` around one live text: the finite items before it are written
// before its first fragment or record end (or at the flush, when it wrote
// nothing), the items after it at the flush, before the output beneath is
// flushed.
class Framed implements TextOut {
  constructor(
    private readonly rt: Runtime,
    private readonly inner: TextOut,
    private prefix: ReadonlyArray<Val> | null,
    private suffix: ReadonlyArray<Val> | null,
  ) {}

  private start(): void {
    const prefix = this.prefix
    if (null === prefix) return
    this.prefix = null
    for (const item of prefix) run(writeFinite(this.rt, item, this.inner))
  }

  writeStr(s: string): void {
    this.start()
    this.inner.writeStr(s)
  }

  flush(): void {
    this.start()
    const suffix = this.suffix
    if (null !== suffix) {
      this.suffix = null
      for (const item of suffix) run(writeFinite(this.rt, item, this.inner))
    }
    this.inner.flush()
  }

  hasCommitted(): boolean {
    return this.rt.renderers.hasCommitted(this.inner)
  }

  recordEnd(): void {
    this.start()
    this.inner.recordEnd?.()
  }
}

// A text that never reaches the input: the events are consumed and the text
// is written at the end.
class FiniteTextSink implements Sink {
  constructor(
    private readonly rt: Runtime,
    private readonly text: Val,
    private readonly out: TextOut,
  ) {}

  event(ev: JsonEvent): Flow {
    if ('end' === ev.type) {
      run(writeFinite(this.rt, this.text, this.out))
      this.out.flush()
    }
    return 'continue'
  }
}

// ---------------------------------------------------------------------------
// The lowering
// ---------------------------------------------------------------------------

// Lowers plans for one run: the runtime the stages call back into, whose
// routers and renderers they are built from, and the limits and metrics the
// transduce stages take.
export class Lowering {
  // The head of the chain when the program reads parser events: set where
  // `parser-events` is lowered, taken by `parserHead`.
  private head: ParserSink | undefined

  constructor(
    private readonly rt: Runtime,
    private readonly limits: Limits,
    private readonly metrics: Metrics,
  ) {}

  // Keep `head` for the host to push the parser events into: the one reader
  // of them a program may have. The JSON-events side of the chain the
  // caller answers is never driven.
  private keepParserHead(head: ParserSink): Sink {
    if (undefined !== this.head) {
      throw new Fail('STREAM_REUSED', 'reused: the parser events are read twice')
    }
    this.head = head
    return new ParserEventsOnly()
  }

  // What the host pushes parser events into, once `sinkAs` has answered
  // `events`, the chain's JSON-events head: the head `parser-events` was
  // lowered to, or, when the program reads nothing of its input, the
  // parse's end alone into `events`.
  parserHead(events: Sink): ParserSink {
    const head = this.head
    this.head = undefined
    return new ParserHead(head, events)
  }

  // The sink for a program's result over `out`. `render` is the host's
  // choice for a stream result; a text result takes none
  // (`render_of_text`). A stream of items is taken for a table's rows, as
  // the runtime cannot tell an item's shape before it arrives; `sinkAs`
  // takes the checker's word instead.
  sink(result: Val, out: TextOut, render?: Renderer): Sink {
    const output: Output =
      'stream' === result.v ? ('JsonEvents' === protocol(result.plan) ? 'JsonEvents/1' : 'TableRows/1') : 'Text'
    return this.sinkAs(result, out, render, output)
  }

  // `sink` with `output`, what the checker decided the result is: a stream
  // of items the runtime cannot tell the protocol of is JSON events when
  // the checker typed every item an event (a rewritten tree), and a
  // table's rows otherwise.
  sinkAs(result: Val, out: TextOut, render: Renderer | undefined, output: Output): Sink {
    switch (result.v) {
      // A string is a text where a text is expected (spec 10.4).
      case 'str':
        return this.sinkAs(textVal({ p: 'lit', text: result.value }), out, render, output)
      case 'text':
        if (undefined !== render) {
          throw new Fail(
            'DSL_TYPE_ERROR',
            'render_of_text: the program renders its own text; --render applies to a table or JSON events result',
          )
        }
        return this.text(result.plan, out)
      case 'stream': {
        const plan = result.plan
        const built = protocol(plan)
        // A rewritten tree: the checker saw every item an event.
        const proto: Protocol = 'Items' === built && 'JsonEvents/1' === output ? 'JsonEvents' : built
        const chosen: Renderer = render ?? ('JsonEvents' === proto ? 'json' : 'csv')
        const renderers = this.rt.renderers
        if ('JsonEvents' === proto) {
          if ('json' === chosen) return this.events(plan, renderers.json(out, jsonOptions()))
          throw protocolMismatch(
            'csv renders table events; the program\'s result is JSON events (render it as json, or make a table of it with table-from-json)',
          )
        }
        if ('csv' === chosen) return this.table(plan, renderers.csv(out, CsvOptions.default()), false)
        return this.table(plan, renderers.recordsToJson(renderers.json(out, jsonOptions())), false)
      }
      default:
        throw typeError(`export must answer a text or a stream, not ${kindText(result)}`)
    }
  }

  // The sink for a program's JSON-events result whose events go into the
  // host's own `down` rather than to a renderer: the host's stages take
  // them as a source's (a renderer of its own, a scope over a path), and
  // `down` sees the program's `end` after the root. `output` is the
  // checker's word, as for `sinkAs`. A result that is a text or a table is
  // refused (`protocol_mismatch`).
  eventsInto(result: Val, output: Output, down: Sink): Sink {
    if ('stream' !== result.v) {
      throw protocolMismatch(
        `the program's result is ${kindText(result)}, and a host takes JSON events into its own sink`,
      )
    }
    const built = protocol(result.plan)
    if ('JsonEvents' === built || ('Items' === built && 'JsonEvents/1' === output)) {
      return this.events(result.plan, down)
    }
    throw protocolMismatch("the program's result is a table, and a host takes JSON events into its own sink")
  }

  private text(plan: Plan, out: TextOut): Sink {
    if (!isLive(plan)) return new FiniteTextSink(this.rt, textVal(plan), out)
    switch (plan.p) {
      case 'json': {
        const json = this.rt.renderers.json(out, jsonOptions())
        if ('null' === plan.nonFinite) return this.events(plan.source, new NullNonFinite(json))
        return this.events(plan.source, json)
      }
      case 'concat-map':
        if ('stream' !== plan.items.seq) break
        return this.items(plan.items.plan, new ConcatMapStage(this.rt, plan.f, plan.at, out, this.limits), false)
      case 'join':
        if ('stream' !== plan.items.seq) break
        return this.items(plan.items.plan, new JoinStage(this.rt, out, plan.sep, this.limits), false)
      case 'concat': {
        const live = plan.live as number
        const inner = plan.items[live]
        if ('text' !== inner.v) throw typeError('concat: a stream is not a text')
        const framed = new Framed(this.rt, out, plan.items.slice(0, live), plan.items.slice(live + 1))
        return this.text(inner.plan, framed)
      }
      case 'replace':
        if ('text' !== plan.source.v) break
        return this.text(plan.source.plan, this.rt.renderers.replaceText(out, plan.from, plan.to))
      default:
        break
    }
    throw typeError(`${planName(plan)} is not a text`)
  }

  private events(plan: Plan, sink: Sink): Sink {
    switch (plan.p) {
      case 'input':
        return sink
      // `records` ends a table: after it the rows are JSON events, of which
      // a later table makes rows of its own.
      case 'records':
        return this.table(plan.source, this.rt.renderers.recordsToJson(sink), false)
      // `as-events` names what the arm below does for any items plan: the
      // program's items, each an event, as JSON events.
      case 'as-events':
        return this.events(plan.source, sink)
      // The events as they are, held to a tree's contract in front of
      // whatever takes them.
      case 'tree-contract':
        return this.events(plan.source, new TreeContract(sink))
      // A stream whose items may be events (`events` itself, or a
      // `scan-emit`, `map` or `filter` over anything): each item is turned
      // back into an event as the stream runs, the reverse of `events`, so
      // a program can rewrite a document event by event, or build events,
      // and hand them to any taker of JSON events.
      case 'events':
      case 'scan-emit':
      case 'map':
      case 'filter': {
        // The source's three limits hold on the events a program made as on
        // the source's own, through the source's guard: a document built
        // deeper than `max_depth`, a key longer than `max_key_bytes` or a
        // scalar past `max_scalar_bytes` is refused where it arrives,
        // whatever the input held. The guard counts into metrics of its
        // own: the source's count the source's events, and these are the
        // program's.
        const guarded = this.rt.routers.guarded(sink, this.limits, this.rt.abort, new Metrics())
        return this.items(plan, new TaggedToJson(guarded), false)
      }
      // A stream that never yields events: `select`'s values, `route`'s
      // selections, a table's events as items.
      default:
        throw protocolMismatch(`${planName(plan)} yields a stream of items where JSON events were expected`)
    }
  }

  // The sink for the table `plan` yields, handing its events to `table`.
  // `countedLater` is as for `items`: whether the rows `table` receives are
  // counted after it rather than here.
  private table(plan: Plan, table: TableSink, countedLater: boolean): Sink {
    switch (plan.p) {
      case 'input':
      case 'records':
      case 'as-events':
      case 'tree-contract':
        throw protocolMismatch(
          'table events were expected, not JSON events (table-from-json makes a table of them)',
        )
      default:
        return this.items(plan, new TaggedToTable(this.rt, this.metrics, !countedLater, table), true)
    }
  }

  // The sink for the stream of items `plan` yields, handing them to `down`.
  //
  // `countedLater` says whether a table stage after `down` counts the rows
  // these items carry. Each row is counted in `metrics.rows` once, by the
  // last table stage it passes: the adapter to a renderer, or a `csv-table`
  // whose rows reach no later table stage. The flag passes through a `map`,
  // a `filter` and a `scan-emit` unchanged, so a row a filter drops is not
  // counted and one a scan adds is; a table stage sets it for its own
  // source; a text over items, and `records`, clear it. The library's
  // table, a `scan-emit`, counts none.
  private items(plan: Plan, down: ItemSink, countedLater: boolean): Sink {
    switch (plan.p) {
      case 'route': {
        // A capture bounded by a named limit takes the host's value for it:
        // the plan was built under the defaults.
        const specs = plan.specs.map((spec) => {
          const copy = new CaptureSpec(spec.tag, spec.selector, spec.mode)
          if (null !== spec.budget) {
            copy.withBudget(captureBudget(this.limits, spec.budget.name) ?? spec.budget.bytes, spec.budget.name)
          }
          return copy
        })
        const router = this.rt.routers.router(
          specs,
          this.limits,
          this.rt.duplicates,
          this.metrics,
          new RouteToItems(down, false),
        )
        return this.events(plan.source, router)
      }
      case 'select': {
        const router = this.rt.routers.router(
          [CaptureSpec.materialize('selected', plan.selector)],
          this.limits,
          this.rt.duplicates,
          this.metrics,
          new RouteToItems(down, true),
        )
        return this.events(plan.source, router)
      }
      case 'events':
        return this.events(plan.source, new EventsToItems(down))
      // The host's parser events, the one source `parser-events` reads (its
      // native refuses any other): the head is kept for the host, which
      // pushes the rule passes into it, and the JSON events side of the
      // chain is never driven.
      case 'parser-events':
        if ('parser-input' !== plan.source.p) {
          throw protocolMismatch("parser-events reads the host's parser events alone")
        }
        return this.keepParserHead(new ParserEventsToItems(down))
      case 'parser-input':
        throw protocolMismatch("the parser's rule passes are read item by item through parser-events")
      case 'scan-emit':
        return this.items(
          plan.source,
          new ScanStage(this.rt, plan.init, plan.step, plan.finish, plan.at, this.metrics, down),
          countedLater,
        )
      case 'map':
        return this.items(plan.source, new MapStage(this.rt, plan.f, plan.at, down), countedLater)
      case 'filter':
        return this.items(plan.source, new FilterStage(this.rt, plan.f, plan.at, down), countedLater)
      case 'csv-table': {
        checkDelimiter(plan.options)
        const noColumns = noColumnsEmpty(plan.options)
        return this.items(plan.source, new CsvTableStage(this.rt, this.metrics, noColumns, !countedLater, down), true)
      }
      case 'input':
      case 'records':
      case 'as-events':
      case 'tree-contract':
        throw protocolMismatch(
          'JSON events cannot be read item by item; select or route what the stream should yield, or read its events',
        )
      default:
        throw typeError(`${planName(plan)} is a text, not a stream`)
    }
  }

}
