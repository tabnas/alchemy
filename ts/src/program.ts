/* Copyright (c) 2026 tabnas, MIT License */

// The API a host embeds (rs/src/program.rs): `compile` a program (or
// `compileSources`, several files linked into one), ask it what it
// produces, and take a `Sink` to push the source's events into.
//
// The front end runs first (`analyze`, `analyzeSources`: one or several
// sources parsed, desugared, linked into one namespace, resolved and
// checked); then the plan is built: the evaluator applies `export` to the
// input's plan (`./interp`), which evaluates everything a stream does not
// defer, bounded by MAX_PLAN_STEPS and MAX_EVAL_DEPTH. The host pushes
// `JsonEvent`s into the sink a `Program` makes and one `end`; the sink
// writes the output as it goes and flushes it at `end`. A failure is
// thrown from the event call that found it, as a transduce `Fail` whose
// `committedOutput` says whether bytes had already reached the writer.

import { AbortFlag, Duplicates, Fail, Limits, Metrics, Selector, Sink } from '@tabnas/transduce'
import { TextOut, WriteOut, Writer } from '@tabnas/render'

import { Expr, SourceSpan, Sources } from './ast'
import { Checked, checkProgram, noExport } from './check'
import { desugarProgram } from './desugar'
import { explain, explainJson } from './effects'
import { parseFile } from './grammar'
import { MAX_PLAN_STEPS, Runtime } from './interp'
import { Lowering, Renderer } from './lower'
import { Output } from './output'
import { Resolved, resolve } from './resolve'
import { outer } from './stdlib'
import { run } from './trampoline'
import { Plan, Val, protocol as protocolOf } from './value'

export type { Output } from './output'
export type { Renderer } from './lower'

// One of the sources a program is compiled from: the file it is named by
// in diagnostics, its text, and the name its own `export` is linked under
// when another source's `export` is the program's.
export type Source = {
  readonly file: string
  readonly text: string
  // When set, this source's `export` is defined under this name in the
  // linked namespace, and every mention of `export` in this source names
  // it. The source must define `export` (`no_export` otherwise) and must
  // not mention the name already (`duplicate_def`).
  readonly exportAs?: string
}

// What the front end learned about a program: its definitions, the texts
// its spans are positioned in, and the checker's verdict.
export type Analyzed = {
  readonly resolved: Resolved
  readonly sources: Sources
  readonly checked: Checked
}

// Parse, desugar, resolve and check `src`, named `file` in diagnostics.
export function analyze(src: string, file: string): Analyzed {
  return analyzeSources([{ file, text: src }])
}

// Several sources linked into one namespace and checked as one program.
// A failure is thrown as a `Fail` naming its file when the sources are
// several.
export function analyzeSources(sources: ReadonlyArray<Source>): Analyzed {
  const named: Array<[string, string]> = []
  for (const source of sources) {
    if (named.some(([file]) => file === source.file)) {
      throw new Fail(
        'DSL_TYPE_ERROR',
        `duplicate_file: ${source.file} is given twice; each source has its own name`,
      )
    }
    named.push([source.file, source.text])
  }
  if (0 === named.length) throw noExport()
  const linked = Sources.several(named)
  const forms: Expr[] = []
  for (const source of sources) {
    // The reader and the desugarer see one file at a time, so the file is
    // added here; the stages after them position through `linked`.
    const inFile = (fail: unknown): unknown => {
      if (fail instanceof Fail && linked.namesFiles()) fail.inFile(source.file)
      return fail
    }
    let own: Expr[]
    try {
      own = desugarProgram(parseFile(source.text, source.file), source.text)
    } catch (fail) {
      throw inFile(fail)
    }
    const name = source.exportAs
    if (undefined !== name) {
      if (!own.some((form) => defines(form, 'export'))) {
        throw inFile(
          new Fail('DSL_TYPE_ERROR', `no_export: ${source.file} defines no export to link as ${name}`),
        )
      }
      // Renaming every `export` is a consistent renaming only while
      // nothing in the source is already called `name`.
      for (const form of own) {
        const taken = mentions(form, name)
        if (undefined !== taken) {
          const [row, col] = linked.position(taken)
          throw inFile(
            new Fail(
              'DSL_TYPE_ERROR',
              `duplicate_def: ${source.file} already names ${name}, so its export cannot ` +
                `be linked under it; link it under another name`,
            ).at(row, col),
          )
        }
      }
      own = own.map((form) => renameSymbol(form, 'export', name))
    }
    forms.push(...own)
  }
  const resolved = resolve(forms, linked, outer)
  const checked = checkProgram(resolved, linked)
  return { resolved, sources: linked, checked }
}

// Whether `form` is a top-level `def` of `name`, as the desugarer leaves
// one: `(def name value)`.
function defines(form: Expr, name: string): boolean {
  if ('list' !== form.kind || 3 !== form.items.length) return false
  const [head, defined] = form.items
  return 'symbol' === head.kind && 'def' === head.name && 'symbol' === defined.kind && name === defined.name
}

// The span of the first symbol `name` in `form`, when it mentions one.
// Recursive per level, which MAX_NESTING bounds.
function mentions(form: Expr, name: string): SourceSpan | undefined {
  if ('symbol' === form.kind) return form.name === name ? form.span : undefined
  if ('list' === form.kind || 'vector' === form.kind) {
    for (const item of form.items) {
      const found = mentions(item, name)
      if (undefined !== found) return found
    }
  }
  return undefined
}

// Every symbol `from` in `form` becomes `to`: the name a `def` binds and
// every mention of it alike. Recursive per level, which MAX_NESTING
// bounds.
function renameSymbol(form: Expr, from: string, to: string): Expr {
  if ('symbol' === form.kind) return form.name === from ? { ...form, name: to } : form
  if ('list' === form.kind || 'vector' === form.kind) {
    return { ...form, items: form.items.map((item) => renameSymbol(item, from, to)) }
  }
  return form
}

// ---------------------------------------------------------------------------
// Compiling: the plan built, and the program around it
// ---------------------------------------------------------------------------

// Parse, desugar, resolve and check `src`, named `file` in diagnostics, and
// build its plan: `export` applied to the input's plan, which evaluates
// everything a stream does not defer, so a `fail` or a `no_match` on that
// path is reported here, with its own code. Building the plan is bounded by
// MAX_PLAN_STEPS and MAX_EVAL_DEPTH, and runs on the evaluator's explicit
// stack, so the bounds hold in Node's default stack. A failure is thrown
// as a `Fail`.
export function compile(src: string, file: string): Program {
  return compileSources([{ file, text: src }])
}

// Compile a program from several sources linked into one namespace: a
// definition in any of them is in scope in all, `export` is defined in
// one, and a name defined twice, in one file or across two, is
// `duplicate_def`. Each source is named by its file, and the names are
// distinct (`duplicate_file` otherwise). A failure in a program of several
// sources carries the file its position is in, `Fail.file`, displayed
// `(file:row:col)`, beside the row and column; a program of one source is
// `compile`, which positions without a file. A source whose `export` is
// not the program's is linked with `exportAs`: its `export` is defined
// under that name instead, so that the program's `export` can call it.
export function compileSources(sources: ReadonlyArray<Source>): Program {
  const analyzed = analyzeSources(sources)
  return Program.build(analyzed.resolved, analyzed.sources, analyzed.checked.output, true, 'reject', new AbortFlag())
}

// The stage that reads the input directly: the plan whose source is the
// input, or the input itself.
function rootStage(plan: Plan): Plan {
  let here = plan
  for (;;) {
    let next: Plan | undefined
    switch (here.p) {
      case 'input':
        return here
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
        if ('input' === here.source.p) return here
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
      default:
        next = undefined
    }
    if (undefined === next) return here
    here = next
  }
}

// A compiled program: parsed, desugared, resolved and checked, with its
// plan built. Immutable; the `with` methods answer another program.
export class Program {
  private constructor(
    readonly resolved: Resolved,
    readonly sources: Sources,
    // What the checker decided from `export`'s type, and so what the host
    // renders.
    readonly output: Output,
    // Whether the standard compositions run natively (the default) or
    // through the library's own definitions.
    readonly native: boolean,
    // The policy for a member name repeated in a captured scope.
    readonly duplicates: Duplicates,
    // The host's cancellation, handed to every sink's runtime.
    readonly abort: AbortFlag,
    // `export` applied to the input plan under the flags above: a text or
    // a stream.
    readonly result: Val,
  ) {}

  // Build the plan of a checked program.
  static build(
    resolved: Resolved,
    sources: Sources,
    output: Output,
    native: boolean,
    duplicates: Duplicates,
    abort: AbortFlag,
  ): Program {
    const rt = new Runtime(resolved, sources)
      .withNative(native)
      .withDuplicates(duplicates)
      .withFuel(MAX_PLAN_STEPS)
      .withAbort(abort)
    const result = run(rt.export())
    return new Program(resolved, sources, output, native, duplicates, abort, result)
  }

  // The same program with the standard compositions run through the
  // library's own definitions (`false`) or natively (`true`, the default).
  // The differential test runs both.
  withNative(native: boolean): Program {
    return Program.build(this.resolved, this.sources, this.output, native, this.duplicates, this.abort)
  }

  // The same program with another policy for repeated member names in
  // captured values; the default rejects them.
  withDuplicates(duplicates: Duplicates): Program {
    return Program.build(this.resolved, this.sources, this.output, this.native, duplicates, this.abort)
  }

  // The same program with the host's cancellation: every sink it makes
  // reads the flag as the program's functions run, so a timeout stops a
  // long computation on one item with `ABORTED` rather than waiting for the
  // item to finish. The source takes the same flag (`ParserSource.abort`)
  // to stop between events.
  withAbort(abort: AbortFlag): Program {
    return new Program(this.resolved, this.sources, this.output, this.native, this.duplicates, abort, this.result)
  }

  // The file name the program was compiled under: the first, of several.
  file(): string {
    return this.resolved.file
  }

  // The protocol the built plan produces; the checker's `output` agrees
  // with it, and a test holds the two together.
  planOutput(): Output {
    if ('stream' === this.result.v) {
      return 'JsonEvents' === protocolOf(this.result.plan) ? 'JsonEvents/1' : 'TableRows/1'
    }
    return 'Text'
  }

  private plan(): Plan | undefined {
    return 'stream' === this.result.v || 'text' === this.result.v ? this.result.plan : undefined
  }

  // The selector under which the source is read one row at a time, when
  // the plan makes one known: the table binding's `:rows`, a `select`'s
  // selector, or the one multi-location capture of a `route`. A host that
  // streams a verified grammar prunes the parse under it.
  rowSelector(): Selector | undefined {
    const plan = this.plan()
    if (undefined === plan) return undefined
    const stage = rootStage(plan)
    switch (stage.p) {
      case 'table-from-json': {
        if ('record' !== stage.binding.v) return undefined
        const rows = stage.binding.fields.get('rows')
        return undefined !== rows && 'selector' === rows.v ? rows.selector : undefined
      }
      case 'select':
        return stage.selector
      case 'route': {
        const multi = stage.specs.map((s) => s.selector).filter((s) => s.isMulti())
        if (1 === multi.length) return multi[0]
        if (0 === multi.length && 1 === stage.specs.length) return stage.specs[0].selector
        return undefined
      }
      default:
        return undefined
    }
  }

  // The plan report of spec section 15.5: the chain of calls, the
  // protocols, what is retained and under which limits, the ordering
  // contract, the renderer, the guarantee and its qualification.
  explain(): string {
    return explain(this)
  }

  // The same report as one JSON object, for a host's `--explain`.
  explainJson(): Record<string, unknown> {
    return explainJson(this)
  }

  // The sink for one run: the host pushes the source's events into it and
  // one `end`, and the output reaches `writer` through a coalescing writer
  // with `limits.max_output_bytes` enforced and `output_bytes` counted in
  // `metrics`. `render` chooses how a table or JSON-events result is
  // rendered (CSV or JSON; undefined for the default); a program that
  // renders its own text takes none (`render_of_text`).
  sink(writer: Writer, render?: Renderer, limits: Limits = Limits.default(), metrics: Metrics = new Metrics()): Sink {
    const out = new WriteOut(writer).withLimits(limits).withMetrics(metrics)
    return this.sinkOut(out, render, limits, metrics)
  }

  // `sink` over any text output: for a host that has its own writer stage,
  // and for tests that read the text back.
  sinkOut(out: TextOut, render?: Renderer, limits: Limits = Limits.default(), metrics: Metrics = new Metrics()): Sink {
    const rt = new Runtime(this.resolved, this.sources)
      .withNative(this.native)
      .withDuplicates(this.duplicates)
      .withLimits(limits)
      .withAbort(this.abort)
    return new Lowering(rt, limits, metrics).sink(this.result, out, render)
  }
}
