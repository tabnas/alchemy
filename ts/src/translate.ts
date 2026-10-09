/* Copyright (c) 2026 tabnas, MIT License */

// Translation between formats, composed from per-format parts (admin
// ADR-27; the design is tabnas/transduce's `docs/translation.md`). A port
// of rs/src/translate.rs.
//
// A format's package hands over its translation parts through a structural
// descriptor (`translate()` in each runtime): its manifest,
// `tabnas.plugin.json`, whose `translate` object names the shapes the
// format reads as and writes from, the root its render needs, the schema
// its events carry when they are not a plain tree, and the loss its render
// declares; and the alchemy text of its lift, its embed and its render.
// This module reads a descriptor into a `Part` and composes the program
// that translates a document of one format into another:
//
// 1. the source's events, through its lift when the target writes from
//    records and the source reads as records first;
// 2. for a target that writes from a tree: an embed into the target's
//    schema when the target has one and the source's events are not of it
//    (a schema-only target, one with no embed, refuses before any output),
//    else the root adapter when the target's render needs an object or an
//    array (`wrap-object`, `wrap-array`: the standard library's, which pass
//    a root of the right kind through);
// 3. for a target that writes from records, a tree's rows through the
//    inferred table, its root an array (`wrap-array`), or a program's table
//    as it is;
// 4. the target's render: a part's own, or alchemy's `json` or `csv`.
//
// The composed program is a one-line `export` linked with the parts'
// sources (`Composition.compile`). A host runs it as it runs any program,
// and keeps a tree's contract in front of a render that writes from one
// (`Front`). Nothing here knows a format by name: every decision is the
// descriptors'.

import { Fail } from './shared'

import { jsonString } from './ast'
import type { Output } from './output'
import { CompileOptions, Program, Source, compileSources } from './program'

// A shape a format reads as or writes from: the manifest's `reads` and
// `writes`. `tree`: a document's events; `records`: a table's rows.
export type Shape = 'tree' | 'records'

// The shape a manifest names.
function shapeNamed(name: unknown): Shape | undefined {
  return 'tree' === name || 'records' === name ? name : undefined
}

// The root a format's render needs: the manifest's `root`. `object`: a
// table or a section map at the root (TOML, INI); `array`: a sequence at
// the root (JSON Lines; and every render that writes from records, whose
// rows are the elements of the root array); `any`: any value.
export type Root = 'object' | 'array' | 'any'

// The root a manifest names.
function rootNamed(name: unknown): Root | undefined {
  return 'object' === name || 'array' === name || 'any' === name ? name : undefined
}

// One part as a format's package hands it over: the entry point a host
// calls, and its alchemy source (none for a render alchemy carries).
export type PartText = {
  readonly entry: string
  readonly source?: string
}

// A format package's structural descriptor: what its `translate()`
// answers, and the name its parts are named by in diagnostics (the
// package's, `tabnas-yaml`).
export type Descriptor = {
  readonly package: string
  readonly manifest: string
  readonly lift?: PartText
  readonly embed?: PartText
  readonly render?: PartText
}

// An alchemy part: the file it is named by in diagnostics, its entry
// point, and its text.
export type Alc = {
  readonly file: string
  readonly entry: string
  readonly text: string
}

// How a format is written: an alchemy render of its own, or one alchemy
// carries: `json`, the render package's JSON renderer, or `csv`, under the
// export's policies (`CSV_OPTIONS`).
export type Render =
  | { readonly kind: 'alc'; readonly alc: Alc }
  | { readonly kind: 'json' }
  | { readonly kind: 'csv' }

// A format's translation parts, as its manifest names them and its package
// hands them over.
export type Part = {
  // The manifest's `languageId`.
  readonly id: string
  // What the format reads as, in order of preference; never empty.
  readonly reads: ReadonlyArray<Shape>
  // What the render writes from.
  readonly writes: Shape
  // The root the render needs.
  readonly root: Root
  // The tree the format's events carry, when not a plain one.
  readonly schema?: string
  // From the events to the first read shape, where they do not carry it.
  readonly lift?: Alc
  // From a plain tree into the schema; its file also holds the reverse.
  readonly embed?: Alc
  readonly render: Render
  // What a written document does not keep, a sentence each.
  readonly loss: ReadonlyArray<string>
}

// A member of a JSON object, as the manifest holds it; undefined for an
// absent member, and for anything that is not an object.
function member(value: unknown, key: string): unknown {
  if (null === value || 'object' !== typeof value || Array.isArray(value)) return undefined
  return Object.prototype.hasOwnProperty.call(value, key) ? (value as Record<string, unknown>)[key] : undefined
}

export const Part = Object.freeze({
  // The part a descriptor describes, or undefined when its manifest names
  // no `translate` object (a format read and not written) or one this
  // module cannot take: a shape or a root it does not know, a part the
  // package does not hand over, a render that is neither an alchemy file
  // nor a render alchemy carries.
  fromDescriptor(d: Descriptor): Part | undefined {
    let manifest: unknown
    try {
      manifest = JSON.parse(d.manifest)
    } catch {
      return undefined
    }
    const id = member(manifest, 'languageId')
    if ('string' !== typeof id) return undefined
    const t = member(manifest, 'translate')
    const named = member(t, 'reads')
    const reads: Shape[] = []
    if ('string' === typeof named) {
      const one = shapeNamed(named)
      if (undefined === one) return undefined
      reads.push(one)
    } else if (Array.isArray(named)) {
      for (const v of named) {
        const shape = shapeNamed(v)
        if (undefined === shape) return undefined
        reads.push(shape)
      }
    } else {
      return undefined
    }
    if (0 === reads.length) return undefined
    const writes = shapeNamed(member(t, 'writes'))
    if (undefined === writes) return undefined
    let root: Root = 'any'
    const rootName = member(t, 'root')
    if (undefined !== rootName) {
      const r = rootNamed(rootName)
      if (undefined === r) return undefined
      root = r
    }
    const schemaName = member(t, 'schema')
    if (undefined !== schemaName && ('string' !== typeof schemaName || '' === schemaName)) return undefined
    const schema = schemaName as string | undefined
    const file = (path: string) => `${d.package}/${path}`
    // The alchemy part the manifest names under `key`, as the package hands
    // it over: `{}` when neither names one, `{ alc }` when both do and it is
    // an alchemy file with an entry and a text, and undefined (no part) for
    // anything else.
    const alc = (key: string, part: PartText | undefined): { alc?: Alc } | undefined => {
      const path = member(t, key)
      if (undefined === path && null == part) return {}
      if ('string' === typeof path && path.endsWith('.alc') && null != part && '' !== part.entry && null != part.source) {
        return { alc: { file: file(path), entry: part.entry, text: part.source } }
      }
      return undefined
    }
    const lift = alc('lift', d.lift)
    const embed = alc('embed', d.embed)
    if (undefined === lift || undefined === embed) return undefined
    if (undefined !== embed.alc && undefined === schema) return undefined
    const part = d.render
    if (null == part || '' === part.entry) return undefined
    const renderName = member(t, 'render')
    if ('string' !== typeof renderName) return undefined
    let render: Render
    if ('json' === renderName && 'json' === part.entry && null == part.source) {
      render = { kind: 'json' }
    } else if ('csv' === renderName && 'csv' === part.entry && null == part.source) {
      render = { kind: 'csv' }
    } else if (renderName.endsWith('.alc') && null != part.source) {
      render = { kind: 'alc', alc: { file: file(renderName), entry: part.entry, text: part.source } }
    } else {
      return undefined
    }
    const lines = member(t, 'loss')
    const loss = Array.isArray(lines) ? lines.filter((l): l is string => 'string' === typeof l) : []
    return {
      id,
      reads,
      writes,
      root,
      ...(undefined === schema ? {} : { schema }),
      ...(undefined === lift.alc ? {} : { lift: lift.alc }),
      ...(undefined === embed.alc ? {} : { embed: embed.alc }),
      render,
      loss,
    }
  },

  // The part a program's output stands for, in a source's place: JSON
  // events are a plain tree, a table is records, and a text is no source
  // (`render_of_text`).
  ofOutput(output: Output): Part {
    let reads: Shape
    switch (output) {
      case 'JsonEvents/1':
        reads = 'tree'
        break
      case 'TableRows/1':
        reads = 'records'
        break
      case 'Text':
        throw new Fail('DSL_TYPE_ERROR', 'render_of_text: the program renders its own text, which no render takes')
    }
    return { id: 'program', reads: [reads], writes: reads, root: 'any', render: { kind: 'json' }, loss: [] }
  },
})

// What a composition runs between the source's events and the render:
// `wrap-object`, a root that is not an object as the one member `key` of
// one; `wrap-array`, a root that is not an array as the one element of
// one; `embed`, a plain tree into the target's schema, by its embed;
// `inferred-table`, a tree's rows as records; `records`, records as a
// tree, one object per row, keyed by the labels.
export type Adapter =
  | { readonly kind: 'wrap-object'; readonly key: string }
  | { readonly kind: 'wrap-array' }
  | { readonly kind: 'embed' }
  | { readonly kind: 'inferred-table' }
  | { readonly kind: 'records' }

export const Adapter = Object.freeze({
  // The adapter as a host names it.
  name(a: Adapter): string {
    switch (a.kind) {
      case 'wrap-object':
        return 'wrap-object'
      case 'wrap-array':
        return 'wrap-array'
      case 'embed':
        return 'embed'
      case 'inferred-table':
        return 'the inferred table'
      case 'records':
        return 'records'
    }
  },

  // What the adapter changes, the host's sentences, printed when it runs;
  // an embed's are its format's own loss declaration.
  loss(a: Adapter): string[] {
    switch (a.kind) {
      case 'wrap-object':
        return [
          `A document whose root is not an object is written as the one member ${quoted(a.key)} of an ` +
            "object, since the format's document is one.",
        ]
      case 'wrap-array':
        return [
          'A document whose root is not an array is written as the one element of an ' +
            "array, since the format's document is a sequence.",
        ]
      case 'embed':
        return []
      case 'inferred-table':
        return [
          "The rows are the elements of the root array: an object row's members are its " +
            "cells, an array row's cells are its positions, and a scalar row is one cell " +
            'named value.',
          "The columns are the first row's: a member or a cell a later row adds is not " +
            'written, one it lacks is written empty, a member repeated in a row keeps its ' +
            'last value, and a row of another kind than the first has a cell only where the ' +
            "first row's columns find one.",
        ]
      case 'records':
        return [
          'Each row is written as an object keyed by the column labels: a cell the row ' +
            'lacks is an absent member, and of two columns with one label the last gives ' +
            'the member.',
        ]
    }
  },
})

// What a host holds the events to in front of the composed program:
// `tree`, the source's events, which a render that writes from a tree
// takes: each key once per object, one root value (transduce's
// `TreeContract`); `none`, nothing: the program's own events, or a table's
// rows.
export type Front = 'tree' | 'none'

// How the composition may be run by a host's own renderer instead, when
// the route is the identity into alchemy's `json`: `json`, the source's
// events as they are, into the JSON renderer. The composed `json` writes a
// number that is not finite as `null` (JSON has no spelling for one, and
// the target declares the loss), so a host that runs its own JSON renderer
// instead writes such a number as `null` too.
export type Native = 'json'

// The composition's options: what the host chooses. `key` is the member a
// root that is not an object is written under, for a target that needs an
// object.
export type Options = {
  readonly key: string
}

export const Options = Object.freeze({
  default(): Options {
    return { key: 'items' }
  },
})

// The CSV options a composed `csv` runs under: the library's, with the
// export's policy for an absent member, an empty field, and a number that
// is not finite written as its word, `Infinity`, `-Infinity` or `NaN`,
// since every CSV cell is text.
export const CSV_OPTIONS =
  '(record (entry :delimiter ",") (entry :newline "\\r\\n") ' +
  '(entry :header true) (entry :null-text "") (entry :missing "") ' +
  '(entry :non-finite :literal))'

// What a composed `json` runs under: a number that is not finite, which
// JSON has no spelling for, is written as `null`.
export const JSON_OPTIONS = '(record (entry :non-finite :null))'

// The binding of the inferred table: the rows are the root array's
// elements, the columns the first row's.
const INFERRED = '(record (entry :columns :infer) (entry :rows (path each-index)))'

// The name a program is linked under when its output feeds a render.
export const PROGRAM_EXPORT = 'program-export'

// A translation ready to compile: the parts' sources, the one-line main,
// what runs between, and what the host keeps in front.
export class Composition {
  constructor(
    // Each part's source, in link order: the render, the embed, the lift.
    readonly sources: ReadonlyArray<Source>,
    // The main's file name in diagnostics, and its text.
    readonly mainFile: string,
    readonly main: string,
    readonly adapters: ReadonlyArray<Adapter>,
    readonly front: Front,
    readonly native: Native | undefined,
    // The render's loss declaration, then each adapter's.
    readonly loss: ReadonlyArray<string>,
  ) {}

  // Link the parts with the main, and `program`'s source when the
  // composition is over a program's output, into one program under the
  // policies the adapters need: the inferred table keeps a repeated
  // member's last value, as an export does. The program's stages are built
  // from `options`' routers and renderers.
  compile(program: Source | undefined, options: CompileOptions): Program {
    const sources: Source[] = this.sources.map(({ file, text }) => ({ file, text }))
    if (undefined !== program) sources.push({ ...program, exportAs: PROGRAM_EXPORT })
    sources.push({ file: this.mainFile, text: this.main })
    const compiled = compileSources(sources, options)
    if (this.adapters.some((a) => 'inferred-table' === a.kind)) return compiled.withDuplicates('last_wins')
    return compiled
  }
}

// Compose the translation of a document read as `source` describes (a
// plain tree when undefined: a format with no parts, or a value a host
// selected below the root) into `target`. `mainFile` names the main in
// diagnostics.
export function compose(source: Part | undefined, target: Part, options: Options, mainFile: string): Composition {
  return composeOver(source, 'input', target, options, mainFile, 'tree')
}

// Compose a program's output into `target`, in the source's place: JSON
// events are a tree and a table is records (`Part.ofOutput`). The program
// is linked under `PROGRAM_EXPORT` by `Composition.compile`.
export function composeProgram(output: Output, target: Part, options: Options, mainFile: string): Composition {
  const source = Part.ofOutput(output)
  return composeOver(source, `(${PROGRAM_EXPORT} input)`, target, options, mainFile, 'none')
}

function composeOver(
  source: Part | undefined,
  input: string,
  target: Part,
  options: Options,
  mainFile: string,
  front: Front,
): Composition {
  const reads: ReadonlyArray<Shape> = undefined === source ? ['tree'] : source.reads
  const lift = source?.lift
  const sourceSchema = source?.schema
  const sources: Source[] = []
  const adapters: Adapter[] = []
  let expr = input
  let treeEvents = true
  if ('records' === target.writes) {
    if ('records' === reads[0]) {
      // A format read as records first: through its lift, or as its events
      // are (a program's table).
      if (undefined !== lift) {
        expr = `(${lift.entry} ${expr})`
        sources.push({ file: lift.file, text: lift.text })
      }
      treeEvents = false
    } else {
      // A tree's rows: the elements of the root array.
      if ('array' === target.root) {
        expr = `(wrap-array ${expr})`
        adapters.push({ kind: 'wrap-array' })
      }
      expr = `(table-from-json ${INFERRED} ${expr})`
      adapters.push({ kind: 'inferred-table' })
      treeEvents = 1 === adapters.length
    }
  } else {
    if (!reads.includes('tree')) {
      // Records only (a program's table): one object per row.
      expr = `(records ${expr})`
      adapters.push({ kind: 'records' })
      treeEvents = false
    }
    const schema = target.schema
    if (undefined !== schema && sourceSchema !== schema) {
      const embed = target.embed
      if (undefined === embed) {
        throw new Fail(
          'TARGET_VALUE_UNREPRESENTABLE',
          `schema_only: ${target.id} writes a ${schema} tree, the tree its own documents ` +
            'read as, and this document is not one; a program that makes one ' +
            'can be composed with the render',
        )
      }
      expr = `(${embed.entry} ${expr})`
      sources.push({ file: embed.file, text: embed.text })
      adapters.push({ kind: 'embed' })
    } else if ('object' === target.root) {
      expr = `(wrap-object ${quoted(options.key)} ${expr})`
      adapters.push({ kind: 'wrap-object', key: options.key })
    } else if ('array' === target.root) {
      expr = `(wrap-array ${expr})`
      adapters.push({ kind: 'wrap-array' })
    }
  }
  const native: Native | undefined = 'json' === target.render.kind && expr === input ? 'json' : undefined
  let render: string
  switch (target.render.kind) {
    case 'json':
      render = `json ${JSON_OPTIONS}`
      break
    case 'csv':
      render = `csv ${CSV_OPTIONS}`
      break
    case 'alc': {
      const alc = target.render.alc
      sources.unshift({ file: alc.file, text: alc.text })
      render = alc.entry
      break
    }
  }
  const main = `def export [input] (${render} ${expr})`
  const loss = [...target.loss]
  for (const adapter of adapters) loss.push(...Adapter.loss(adapter))
  // The source's events reach a tree's render as a tree only when no
  // adapter took them apart first; a records render, and a program's
  // events, have nothing in front.
  const kept: Front = 'tree' === front && treeEvents && 'tree' === target.writes ? 'tree' : 'none'
  return new Composition(sources, mainFile, main, adapters, kept, native, loss)
}

// A string as an alchemy string literal (JSON's escapes).
function quoted(s: string): string {
  return jsonString(s)
}
