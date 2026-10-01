/* Copyright (c) 2026 tabnas, MIT License */

// The front end of the API a host embeds (rs/src/program.rs, up to the
// checker): one or several sources parsed, desugared, linked into one
// namespace, resolved and checked. `analyze` and `analyzeSources` answer
// what the checker learned; they are what `compile` and
// `compileSources` do before they build the plan.
//
// Pass 2 of the TypeScript port adds `compile`, `compileSources` and
// `Program` here, on top of `analyzeSources`: the evaluator applies
// `export` to the input's plan (`./interp`), the plan is lowered to sinks
// (`./lower`), and `Program.explain` hands `./effects` the plan view it
// summarizes.

import { Fail } from '@tabnas/transduce'

import { Expr, SourceSpan, Sources } from './ast'
import { Checked, checkProgram, noExport } from './check'
import { desugarProgram } from './desugar'
import { parseFile } from './grammar'
import { Resolved, resolve } from './resolve'
import { outer } from './stdlib'

export type { Output } from './output'

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
