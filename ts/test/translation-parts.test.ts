/* Copyright (c) 2026 tabnas, MIT License */

// Fleet conformance for the package-local structural translation interface.
// No grammar depends on alchemy or on a shared interface package: importing
// each accessor into this one structural type is the compile-time contract,
// while the assertions hold its values to the embedded descriptor.

import assert from 'node:assert/strict'
import { describe, it } from 'node:test'

import { make as makeCsv, translate as csv } from '@tabnas/csv'
import { Ini, translate as ini } from '@tabnas/ini'
import { make as makeJson, translate as json } from '@tabnas/json'
import { Json5, translate as json5 } from '@tabnas/json5'
import { Jsonc, translate as jsonc } from '@tabnas/jsonc'
import { jsonic as jsonicPlugin, make as makeJsonic, translate as jsonic } from '@tabnas/jsonic'
import { make as makeJsonl, translate as jsonl } from '@tabnas/jsonl'
import { Markdown, translate as markdown } from '@tabnas/markdown'
import { Tabnas } from '@tabnas/parser'
import { Toml, translate as toml } from '@tabnas/toml'
import { Xml, translate as xml } from '@tabnas/xml'
import { Yaml, translate as yaml } from '@tabnas/yaml'
import { Zon, translate as zon } from '@tabnas/zon'

import { Program, Source, compileSources } from '../dist/alchemy'

import { events, replayed } from './common'

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

type Format = {
  readonly name: string
  readonly parts: TranslationParts | undefined
  readonly parser: () => any
  readonly sample: string
}

function withJsonic(plugin: any): Tabnas {
  return new Tabnas().use(jsonicPlugin).use(plugin)
}

const formats: readonly Format[] = [
  { name: 'csv', parts: csv(), parser: makeCsv, sample: 'a,b\n1,x\n2,y\n' },
  { name: 'ini', parts: ini(), parser: () => withJsonic(Ini), sample: 'a=1\nb=x\n' },
  { name: 'json', parts: json(), parser: makeJson, sample: '{"a":1,"b":[true,null]}\n' },
  { name: 'json5', parts: json5(), parser: () => withJsonic(Json5), sample: "{a:1,b:['x',true]}\n" },
  { name: 'jsonc', parts: jsonc(), parser: () => withJsonic(Jsonc), sample: '{"a":1,/* c */"b":true}\n' },
  { name: 'jsonic', parts: jsonic(), parser: makeJsonic, sample: 'a:1,b:true' },
  { name: 'jsonl', parts: jsonl(), parser: makeJsonl, sample: '{"a":1}\n{"a":2}\n' },
  { name: 'markdown', parts: markdown(), parser: () => new Tabnas().use(Markdown), sample: '| a | b |\n| - | - |\n| 1 | x |\n' },
  { name: 'toml', parts: toml(), parser: () => withJsonic(Toml), sample: 'a = 1\nb = "x"\n' },
  { name: 'xml', parts: xml(), parser: () => withJsonic(Xml), sample: '<a x="1">text</a>' },
  { name: 'yaml', parts: yaml(), parser: () => withJsonic(Yaml), sample: 'a: 1\nb:\n  - x\n  - y\n' },
  { name: 'zon', parts: zon(), parser: () => withJsonic(Zon), sample: '.{ .a = 1, .b = .{ true, false } }' },
]

const shapes = new Set(['records', 'tree'])

function descriptor(parts: TranslationParts): Descriptor {
  return JSON.parse(parts.manifest) as Descriptor
}

function assertPart(
  format: string,
  kind: 'lift' | 'render',
  declared: string | undefined,
  part: TranslationPart | undefined,
): void {
  if (undefined === declared) {
    assert.equal(part, undefined, `${format} exposes an undeclared ${kind}`)
    return
  }
  assert.ok(part, `${format} does not expose its declared ${kind}`)
  assert.ok(0 < part.entry.length, `${format} ${kind} has no explicit entry`)
  if (declared.endsWith('.alc')) {
    assert.ok(part.source, `${format} ${kind} does not embed ${declared}`)
  } else {
    assert.equal(part.entry, declared, `${format} ${kind} does not name its builtin`)
    assert.equal(part.source, undefined, `${format} builtin ${kind} unexpectedly has source`)
  }
}

function compileParts(format: string, descriptor: Descriptor, parts: TranslationParts): Program {
  const embedded: ReadonlyArray<readonly ['lift' | 'render', TranslationPart | undefined]> = [
    ['lift', parts.lift],
    ['render', parts.render],
  ]
  const render = parts.render as TranslationPart
  let main: string
  if (parts.lift) {
    main = `def export [input]\n  ${render.entry} (${parts.lift.entry} (events input))\n`
  } else if ('records' === descriptor.translate.writes) {
    const call = 'csv' === render.entry
      ? 'csv csv-options (table-from-json conformance-binding input)'
      : `${render.entry} (table-from-json conformance-binding input)`
    main = 'def conformance-binding\n  record\n    entry :columns :infer\n' +
      '    entry :rows (path each-index)\n\n' +
      `def export [input]\n  ${call}\n`
  } else {
    main = `def export [input]\n  ${render.entry} (events input)\n`
  }
  const sources: Source[] = [{ file: `${format}/conformance.alc`, text: main }]
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
  for (const format of formats) {
    const { name, parts: maybeParts } = format
    it(`${name} agrees with its descriptor and round trips through its parts`, () => {
      assert.ok(maybeParts, `${name} exposes no translation parts`)
      const parts = maybeParts
      const manifest = descriptor(parts)
      assert.equal(manifest.languageId, name)
      const reads = 'string' === typeof manifest.translate.reads
        ? [manifest.translate.reads]
        : manifest.translate.reads
      assert.ok(0 < reads.length, `${name} declares no read shape`)
      for (const shape of reads) assert.ok(shapes.has(shape), `${name} reads unknown shape ${shape}`)
      assert.ok(shapes.has(manifest.translate.writes), `${name} writes unknown shape`)
      assertPart(name, 'lift', manifest.translate.lift, parts.lift)
      assertPart(name, 'render', manifest.translate.render, parts.render)
      const program = compileParts(name, manifest, parts)
      const first = events(format.parser(), format.sample)
      assert.ok(first, `${name} does not parse its conformance sample`)
      const rendered = replayed(program, first)
      const second = events(format.parser(), rendered)
      assert.ok(second, `${name} does not parse its own render ${JSON.stringify(rendered)}`)
      assert.equal(replayed(program, second), rendered, `${name} render is not round-trip stable`)
    })
  }
})
