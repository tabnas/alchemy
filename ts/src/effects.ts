/* Copyright (c) 2026 tabnas, MIT License */

// The effect summary and the plan report. A port of rs/src/effects.rs.
//
// The summary is computed from the plan the program built, not from its
// text: the plan knows which stages run and what each retains, and the
// registry's effect metadata says what each operator promises. Live
// retention requirements are summed rather than maxed.
//
// `explain` prints the report: the entry's chain of calls, then labelled
// lines (source reads, protocol chain, selection, what is retained, output
// order, the ordering contract and how it is enforced, the renderer's
// profile, external storage), then the guarantee and its qualification.
// `explainJson` is the same information as one object, for a host's
// `--explain`.
//
// The report reads a compiled program through `Explainable`: what the
// checker decided (`output`), the plan `export` answered (`result`), the
// definitions, and the duplicate-member policy. Pass 2's `Program` is one;
// until the evaluator exists, a test builds the plan by hand.

import type { Duplicates } from './shared'

import { Expr, symbolOf } from './ast'
import { Output } from './output'
import { Resolved } from './resolve'
import { Func, Plan, Val, field, isLive } from './value'

// What the report reads of a compiled program.
export interface Explainable {
  // What the checker decided from `export`'s type.
  readonly output: Output
  // The plan `export` answered: a text or a stream (or a value whose text
  // is the program's own).
  readonly result: Val
  readonly resolved: Resolved
  // The policy for a member name repeated in a captured scope.
  readonly duplicates: Duplicates
}

// What a stage keeps alive.
export type RetentionScope =
  // The column metadata, until the end.
  | 'metadata'
  // One selected row at a time.
  | 'record'
  // One selected scope, materialized whole.
  | 'subtree'
  // The state a `scan-emit` step returns.
  | 'state'

// One live retention requirement.
export type Retention = {
  scope: RetentionScope
  // The label the report prints it under.
  label: string
  reason: string
  // The selector whose matches are retained, when one applies, as the
  // report prints it.
  selector?: string
  // The `Limits` field that caps it, when the runtime caps it.
  limit?: string
}

// When output is ready.
export type Readiness = 'event' | 'record' | 'scope-end'

// An ordering the plan relies on, and who enforces it: `static` when the
// plan cannot violate it, `runtime` when the run checks it and fails.
export type OrderConstraint = { before: string; after: string; enforcement: 'static' | 'runtime' }

// How sure the analysis is of its guarantee.
export type Confidence = 'proven' | 'conditional'

// The CSV renderer's dialect, as the report names it (the fields of the
// render crate's `CsvOptions` the report prints).
export type CsvDialect = {
  delimiter: string
  newline: '\r\n' | '\n'
  header: boolean
  nullText: string
  missing: { kind: 'error' } | { kind: 'text'; text: string }
}

// The renderer's defaults: CRLF, a header, an empty null text, a missing
// cell an error.
export function defaultCsvDialect(): CsvDialect {
  return { delimiter: ',', newline: '\r\n', header: true, nullText: '', missing: { kind: 'error' } }
}

// The renderer's dialect for a `csv-options` record, when every field has
// a value the renderer accepts: `:delimiter` one character, `:newline`
// CRLF or LF, `:header` a boolean, `:null-text` a string, `:missing`
// `:error` or a string (rs/src/lower.rs `csv_options`). Any other record
// runs the library's own `csv`.
export function csvDialect(options: Val): CsvDialect | undefined {
  if ('record' !== options.v) return undefined
  const f = options.fields
  const delimiter = f.get('delimiter')
  if (undefined === delimiter || 'str' !== delimiter.v || 1 !== [...delimiter.value].length) return undefined
  const newline = f.get('newline')
  if (undefined === newline || 'str' !== newline.v || ('\r\n' !== newline.value && '\n' !== newline.value)) {
    return undefined
  }
  const header = f.get('header')
  if (undefined === header || 'bool' !== header.v) return undefined
  const nullText = f.get('null-text')
  if (undefined === nullText || 'str' !== nullText.v) return undefined
  const miss = f.get('missing')
  let missing: CsvDialect['missing']
  if (undefined !== miss && 'keyword' === miss.v && 'error' === miss.name) missing = { kind: 'error' }
  else if (undefined !== miss && 'str' === miss.v) missing = { kind: 'text', text: miss.value }
  else return undefined
  return { delimiter: delimiter.value, newline: newline.value as '\r\n' | '\n', header: header.value, nullText: nullText.value, missing }
}

// Whether a table binding infers its columns from the first row
// (rs/src/lower.rs `is_inferred`).
export function isInferred(binding: Val): boolean {
  const columns = field(binding, 'columns')
  return undefined !== columns && 'keyword' === columns.v && 'infer' === columns.name
}

// The renderer that writes the text, when one does.
export type RendererProfile =
  // The CSV renderer, always quoted, in the dialect it is built with: the
  // program's own options for its `csv`, the defaults when the host chose
  // it for a table result (`host`).
  | { kind: 'csv'; host: boolean; options: CsvDialect }
  // The JSON renderer: compact, one document, a trailing newline.
  | { kind: 'json'; host: boolean }
  // The program's own text algebra.
  | { kind: 'text' }

// The effect summary with the report's other facts.
export type EffectSummary = {
  // The definition the host applies.
  entry: string
  // Whether the result is a text that never reaches the input.
  finite: boolean
  // The calls on the data-last spine of the entry's body, innermost first.
  chain: string[]
  passes: number
  protocol: string[]
  selection: string
  // What happens to a member name repeated in an object of the source.
  duplicates: string
  retention: Retention[]
  readiness: Readiness
  outputOrder: string
  orderConstraints: OrderConstraint[]
  renderer: RendererProfile
  externalStorage: string
  confidence: Confidence
  guarantee: string
  qualification: string[]
  output: Output
}

// The stages of a plan from the input outward.
function stages(plan: Plan): Plan[] {
  const out: Plan[] = []
  let here: Plan = plan
  for (;;) {
    out.push(here)
    let next: Plan | undefined
    switch (here.p) {
      case 'input':
      case 'lit':
        next = undefined
        break
      case 'route':
      case 'select':
      case 'events':
      case 'scan-emit':
      case 'map':
      case 'filter':
      case 'table-from-json':
      case 'records':
      case 'csv-table':
      case 'csv':
      case 'json':
        next = here.source
        break
      case 'concat-map':
      case 'join':
        next = 'stream' === here.items.seq ? here.items.plan : undefined
        break
      case 'concat': {
        const inner = undefined === here.live ? undefined : here.items[here.live]
        next = undefined !== inner && ('text' === inner.v || 'stream' === inner.v) ? inner.plan : undefined
        break
      }
      case 'replace':
        next = 'text' === here.source.v ? here.source.plan : undefined
        break
    }
    if (undefined === next) break
    here = next
  }
  return out.reverse()
}

// The protocol a stage produces, as the report names it.
function protocolOf(stage: Plan): string {
  switch (stage.p) {
    case 'input':
    case 'records':
      return 'JsonEvents/1'
    case 'table-from-json':
    case 'csv-table':
      return 'TableRows/1'
    case 'route':
      return 'Stream<Selected>'
    case 'select':
      return 'Stream<Value>'
    case 'events':
      return 'Stream<Event>'
    case 'scan-emit':
    case 'map':
    case 'filter':
      return 'Stream<Value>'
    default:
      return 'Text'
  }
}

// The calls on the data-last spine of an `export` body, innermost first,
// ending where the spine reaches the parameter or a special form.
function chainOf(body: Expr): string[] {
  const chain: string[] = []
  let here = body
  while ('list' === here.kind) {
    const head = symbolOf(here.items[0])
    if (undefined === head) break
    if ('fn' === head || 'let' === head || 'if' === head || 'match' === head || 'def' === head || here.items.length < 2) {
      break
    }
    chain.push(head)
    here = here.items[here.items.length - 1]
  }
  return chain.reverse()
}

// Whether the plan produces table events, natively or as tagged values
// the host's adapter reads.
function tabular(stages: Plan[], output: Output): boolean {
  return (
    stages.some((s) => 'table-from-json' === s.p || 'csv-table' === s.p || 'csv' === s.p || 'records' === s.p) ||
    'TableRows/1' === output
  )
}

// The duplicate-member line: what the plan does with a member name an
// object of the source repeats.
function duplicatesOf(stages: Plan[], policy: Duplicates, finite: boolean): string {
  if (finite) return 'not examined; the input is only validated'
  const captured = stages.some((s) => 'route' === s.p || 'select' === s.p || 'table-from-json' === s.p)
  if (!captured) return 'preserved; the events are copied as they arrive, not mapped by key'
  switch (policy) {
    case 'reject':
      return 'rejected in captured scopes (DUPLICATE_MEMBER)'
    case 'last_wins':
      return 'the last value wins in captured scopes'
    case 'first_wins':
      return 'the first value wins in captured scopes'
  }
}

// Whether a scan is the standard library's table: the library's
// `table-step`, bare or partially applied, from the state before the
// metadata (`no-schema`).
function isLibraryTable(init: Val, step: Func): boolean {
  const isTableStep = (f: Func): boolean => {
    switch (f.fn) {
      case 'partial':
        return isTableStep(f.partial.f)
      case 'closure':
        return 'stdlib' === f.closure.scope && 'table-step' === f.closure.name
      case 'native':
        return false
    }
  }
  const unbound = 'tagged' === init.v && 'no-schema' === init.tag && 0 === init.fields.length
  return unbound && isTableStep(step)
}

// The selection summary of a plan that selects nothing.
const PASS_THROUGH = 'none; every event passes through'

function count(n: number): string {
  return ['no', 'one', 'two', 'three'][n] ?? String(n)
}

// The summary of a compiled program.
export function summarize(program: Explainable): EffectSummary {
  const output = program.output
  const result = program.result
  const plan: Plan | undefined = 'stream' === result.v || 'text' === result.v ? result.plan : undefined
  // A result that never reaches the input (a string, or a text of the
  // program's own) is written at the end; the source is only validated.
  const finite = !(undefined !== plan && isLive(plan))
  const staged: Plan[] = finite || undefined === plan ? [] : stages(plan)
  const exp = program.resolved.get('export')
  const chain =
    undefined !== exp && 'list' === exp.value.kind && 3 === exp.value.items.length ? chainOf(exp.value.items[2]) : []

  if (finite) {
    return {
      entry: 'export',
      finite,
      chain,
      passes: 1,
      protocol: ['Text'],
      selection: 'none; the input is read and validated, and nothing of it is used',
      duplicates: duplicatesOf(staged, program.duplicates, finite),
      retention: [],
      readiness: 'scope-end',
      outputOrder: "the program's own text, written once the input has validated",
      orderConstraints: [],
      renderer: { kind: 'text' },
      externalStorage: 'disabled',
      confidence: 'proven',
      guarantee:
        "Memory is independent of the document's size under the configured\n  depth and scalar/key limits: nothing of the input is kept. The text\n  is the program's own, written under the output limit.",
      qualification: [
        'The text is written only after the whole input has validated; an\n  invalid input writes nothing.',
      ],
      output,
    }
  }
  const protocol: string[] = ['JsonEvents/1']
  for (const stage of staged.slice(1)) {
    const p = protocolOf(stage)
    if (protocol[protocol.length - 1] !== p) protocol.push(p)
  }
  const last = staged[staged.length - 1]
  let renderer: RendererProfile
  if (undefined !== last && 'csv' === last.p) {
    // The fast path builds a `csv` plan only for options that map onto the
    // renderer's dialect, and lowering builds the renderer with them.
    renderer = { kind: 'csv', host: false, options: csvDialect(last.options) ?? defaultCsvDialect() }
  } else if (undefined !== last && 'json' === last.p) {
    renderer = { kind: 'json', host: false }
  } else if ('TableRows/1' === output) {
    protocol.push('Text')
    // The host's renderer, as lowering builds it.
    renderer = { kind: 'csv', host: true, options: defaultCsvDialect() }
  } else if ('JsonEvents/1' === output) {
    protocol.push('Text')
    renderer = { kind: 'json', host: true }
  } else {
    renderer = { kind: 'text' }
  }

  let selection = PASS_THROUGH
  const retention: Retention[] = []
  const orderConstraints: OrderConstraint[] = []
  let readiness: Readiness = 'event'
  let confidence: Confidence = 'proven'
  let outputOrder = 'source order'
  for (const stage of staged) {
    switch (stage.p) {
      case 'table-from-json': {
        const selector = (key: string): string | undefined => {
          const v = field(stage.binding, key)
          return undefined !== v && 'selector' === v.v ? v.selector.toString() : undefined
        }
        const inferred = isInferred(stage.binding)
        readiness = 'record'
        outputOrder = 'schema first; cells in schema order'
        if (inferred) {
          selection = 'shared prefix matcher, one capture route'
          retention.push({
            scope: 'metadata',
            label: 'Inferred columns:',
            reason: "the first row's keys, taken as the columns once it completes, at most max_columns of them",
            limit: 'max_metadata_bytes',
          })
        } else {
          selection = 'shared prefix matcher, two capture routes'
          retention.push({
            scope: 'metadata',
            label: 'Retained metadata:',
            reason: 'the column descriptors, bound once they complete',
            selector: selector('columns'),
            limit: 'max_metadata_bytes',
          })
        }
        retention.push({
          scope: 'record',
          label: 'Row capture:',
          reason: 'one row at a time, projected into schema order and released',
          selector: selector('rows'),
          limit: 'max_record_bytes',
        })
        if (!inferred) {
          orderConstraints.push({ before: 'metadata completes', after: 'first row begins', enforcement: 'runtime' })
        }
        break
      }
      case 'route': {
        const specs = stage.specs
        selection = `shared prefix matcher, ${count(specs.length)} capture route${1 === specs.length ? '' : 's'}`
        readiness = 'record'
        outputOrder = 'selection order: completed matches, in source order'
        for (const spec of specs) {
          const multi = spec.selector.isMulti()
          retention.push({
            scope: multi ? 'record' : 'subtree',
            label: multi ? 'Row capture:' : 'Capture:',
            reason: `${multi ? 'one at a time,' : ''} one selected scope, materialized whole and released after delivery`.trimStart(),
            selector: spec.selector.toString(),
            limit: spec.budget?.name ?? 'max_capture_bytes',
          })
        }
        break
      }
      case 'select':
        selection = 'shared prefix matcher, one capture route'
        readiness = 'record'
        outputOrder = 'selection order: completed matches, in source order'
        retention.push({
          scope: 'record',
          label: 'Row capture:',
          reason: 'one selected value at a time, released after delivery',
          selector: stage.selector.toString(),
          limit: 'max_capture_bytes',
        })
        break
      // Every event becomes one item as it arrives: no matcher, no
      // capture, nothing retained. A stage before it that selected keeps
      // its own summary.
      case 'events':
        if (PASS_THROUGH === selection) selection = 'none; every event is delivered as an item'
        break
      case 'scan-emit': {
        confidence = 'conditional'
        const reason = isLibraryTable(stage.init, stage.step)
          ? "the table's columns once bound, at most max_columns of them, no deeper than max_depth"
          : 'what the step returns, no deeper than max_depth'
        retention.push({ scope: 'state', label: 'Retained state:', reason, limit: 'max_metadata_bytes' })
        orderConstraints.push({ before: 'each item', after: 'its outputs', enforcement: 'static' })
        break
      }
      default:
        break
    }
  }
  if (tabular(staged, output) && 'table-from-json' !== staged[0]?.p) {
    // Table events reach a renderer: it validates the protocol.
    if (!orderConstraints.some((c) => 'schema' === c.before)) {
      orderConstraints.push({ before: 'schema', after: 'rows, then one end', enforcement: 'runtime' })
    }
  }
  if ('csv' === renderer.kind && 'source order' === outputOrder) {
    outputOrder = 'schema first; cells in schema order'
  }

  let guarantee: string
  if (0 === retention.length) {
    guarantee =
      "Memory is independent of the document's size under the configured\n  depth, scalar/key and output limits: nothing is retained beyond the\n  renderer's nesting stack."
  } else if ('proven' === confidence) {
    guarantee =
      'Memory is independent of the number of rows under the configured\n  depth, metadata, record, scalar/key and output limits.'
  } else {
    guarantee =
      "Memory is independent of the number of rows under the configured\n  depth, capture, metadata, scalar/key and output limits: the state the\n  step returns is capped at max_metadata_bytes. What one step computes\n  is the program's, bounded by the host's abort flag."
  }
  const qualification: string[] = []
  if (orderConstraints.some((c) => c.before.startsWith('metadata'))) {
    qualification.push('Valid JSON that violates the metadata-first contract is rejected.')
  }
  qualification.push('A later error can occur after earlier output has been written.')

  return {
    entry: 'export',
    finite,
    chain,
    passes: 1,
    protocol,
    selection,
    duplicates: duplicatesOf(staged, program.duplicates, finite),
    retention,
    readiness,
    outputOrder,
    orderConstraints,
    renderer,
    externalStorage: 'disabled',
    confidence,
    guarantee,
    qualification,
    output,
  }
}

// The report, in the layout of spec 15.5.
export function summaryText(s: EffectSummary): string {
  const lines: Array<[string, string]> = [
    ['Source reads:', String(s.passes)],
    ['Protocol:', s.protocol.join(' → ')],
    ['Selection:', s.selection],
    ['Duplicate members:', s.duplicates],
  ]
  for (const r of s.retention) {
    let value = ''
    if (undefined !== r.selector) {
      if ('record' === r.scope) value += 'one '
      value += r.selector
    } else {
      value += r.reason
    }
    if (undefined !== r.limit) value += `, capped at ${r.limit}`
    lines.push([r.label, value])
  }
  lines.push(['Output order:', s.outputOrder])
  const first = s.orderConstraints[0]
  const contract = undefined === first ? 'none' : `${first.before} before ${first.after}`
  const verification =
    undefined === first ? 'static' : s.orderConstraints.some((c) => 'runtime' === c.enforcement) ? 'runtime' : 'static'
  lines.push(['Ordering contract:', contract])
  lines.push(['Contract verification:', verification])
  const r = s.renderer
  if ('csv' === r.kind) {
    lines.push(['CSV quoting:', r.host ? "always (the host's renderer)" : 'always'])
  } else if ('json' === r.kind) {
    lines.push([
      'JSON profile:',
      r.host
        ? "compact, one document, trailing newline (the host's renderer)"
        : 'compact, one document, trailing newline',
    ])
  }
  lines.push(['External storage:', s.externalStorage])

  let out = s.entry + ': '
  if (0 === s.chain.length) out += s.finite ? 'a text of its own' : 'input'
  out += s.chain.join(' → ')
  out += '\n\n'
  for (const [label, value] of lines) out += label.padEnd(23) + value + '\n'
  out += '\nGuarantee:\n  '
  out += s.guarantee
  out += '\n\nQualification:\n'
  for (const q of s.qualification) out += '  ' + q + '\n'
  return out
}

// The same facts as one JSON object, its keys in the Rust report's order.
export function summaryJson(s: EffectSummary): Record<string, unknown> {
  const r = s.renderer
  let renderer: Record<string, unknown>
  if ('csv' === r.kind) {
    renderer = {
      name: 'csv',
      quoting: 'always',
      delimiter: r.options.delimiter,
      newline: r.options.newline,
      header: r.options.header,
      null_text: r.options.nullText,
      missing: r.options.missing.kind,
      missing_text: 'text' === r.options.missing.kind ? r.options.missing.text : null,
      host: r.host,
    }
  } else if ('json' === r.kind) {
    renderer = { name: 'json', indent: null, trailing_newline: true, host: r.host }
  } else {
    renderer = { name: 'text' }
  }
  return {
    entry: s.entry,
    finite: s.finite,
    chain: s.chain,
    output: s.output,
    passes: s.passes,
    protocol: s.protocol,
    selection: s.selection,
    duplicates: s.duplicates,
    retention: s.retention.map((x) => ({
      scope: x.scope,
      reason: x.reason,
      selector: x.selector ?? null,
      limit: x.limit ?? null,
    })),
    readiness: s.readiness,
    output_order: s.outputOrder,
    order_constraints: s.orderConstraints.map((c) => ({ before: c.before, after: c.after, enforcement: c.enforcement })),
    renderer,
    external_storage: s.externalStorage,
    confidence: s.confidence,
    guarantee: s.guarantee.split('\n  ').join(' '),
    qualification: s.qualification.map((q) => q.split('\n  ').join(' ')),
  }
}

// The report of spec 15.5 for a compiled program.
export function explain(program: Explainable): string {
  return summaryText(summarize(program))
}

// The report as one JSON object.
export function explainJson(program: Explainable): Record<string, unknown> {
  return summaryJson(summarize(program))
}
