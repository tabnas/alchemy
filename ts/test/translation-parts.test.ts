/* Copyright (c) 2026 tabnas, MIT License */

// Fleet conformance for the package-local structural translation interface.
// No grammar depends on alchemy or on a shared interface package: importing
// each accessor into this one structural type is the compile-time contract,
// while the assertions hold its values to the embedded descriptor.
//
// The conformance of each grammar's parts imports the grammars and runs
// their programs on transduce's routers and render's renderers, so it is
// alchemy-cli's, in its ts/test/translation-parts.test.ts. What compiles
// without running is here: a text-shaped render composes through the Text
// protocol.

import assert from 'node:assert/strict'
import { describe, it } from 'node:test'

import { Program, Source } from '../dist/alchemy'

import { compileSources } from './common'

type TranslationPart = {
  readonly entry: string
  readonly source?: string
}

type TranslationParts = {
  readonly manifest: string
  readonly lift?: TranslationPart
  readonly render?: TranslationPart
}

type Descriptor = {
  readonly languageId: string
  readonly translate: {
    readonly reads: string | readonly string[]
    readonly writes: string
    readonly lift?: string
    readonly render: string
  }
}

function compileParts(
  format: string,
  descriptor: Descriptor,
  parts: TranslationParts,
  reads: string,
  lifted: boolean,
): Program {
  const embedded: ReadonlyArray<readonly ['lift' | 'render', TranslationPart | undefined]> = [
    ['lift', parts.lift],
    ['render', parts.render],
  ]
  const render = parts.render as TranslationPart
  // The producer is structural fact: only the preferred shape may come
  // from a lift; every other shape is the grammar's raw tree. The adapter
  // is selected from the descriptor, so a false reads value type-fails.
  const source = lifted
    ? `${(parts.lift as TranslationPart).entry} (events input)`
    : 'events input'
  const writes = descriptor.translate.writes
  let definitions = ''
  let adapted: string
  if (reads === writes) {
    adapted = source
  } else if ('tree' === reads && 'records' === writes) {
    definitions = 'def conformance-binding\n  record\n    entry :columns :infer\n' +
      '    entry :rows (path each-index)\n\n'
    adapted = `table-from-json conformance-binding (${source})`
  } else if ('records' === reads && 'tree' === writes) {
    adapted = `records (${source})`
  } else if ('text' === writes) {
    definitions = 'def conformance-text [input]\n' +
      '  join ""\n' +
      '    map\n' +
      '      fn [event]\n' +
      '        match event\n' +
      '          case (scalar value) (scalar-text csv-options value)\n' +
      '          case _ ""\n' +
      '      input\n\n'
    adapted = `conformance-text (${source})`
  } else {
    throw new Error(`${format} has no ${reads}-to-${writes} conformance adapter`)
  }
  const call = 'csv' === render.entry
    ? `csv csv-options (${adapted})`
    : `${render.entry} (${adapted})`
  const main = `${definitions}def export [input]\n  ${call}\n`
  const sources: Source[] = [{ file: `${format}/${reads}-conformance.alc`, text: main }]
  for (const [kind, part] of embedded) {
    if (undefined === part?.source) continue
    const file = descriptor.translate[kind]
    assert.ok(file, `${format} ${kind} source has no descriptor path`)
    sources.push({ file, text: part.source })
  }
  const program = compileSources(sources)
  for (const [kind, part] of embedded) {
    if (undefined === part?.source) continue
    assert.ok(
      program.resolved.get(part.entry),
      `${format} ${kind} source does not define ${part.entry}`,
    )
  }
  return program
}

describe('structural translation parts', () => {
  it('composes a text-shaped render through the Text protocol', () => {
    const manifest: Descriptor = {
      languageId: 'textual',
      translate: {
        reads: 'tree',
        writes: 'text',
        render: 'alchemy/render.alc',
      },
    }
    const parts: TranslationParts = {
      manifest: JSON.stringify(manifest),
      render: {
        entry: 'textual-render',
        source: 'def textual-render [input]\n  input\n',
      },
    }
    assert.ok(compileParts('textual', manifest, parts, 'tree', false).resolved.get('textual-render'))
  })
})
