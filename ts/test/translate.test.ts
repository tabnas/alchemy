/* Copyright (c) 2026 tabnas, MIT License */

// The composition of a translation from the formats' parts
// (rs/src/translate.rs's tests and rs/tests/translate_test.rs): a
// descriptor read into a part, the main each route composes, word for word
// as the Rust crate composes it, and every route compiling, with the
// standard library's adapters, to a program whose output is a text. The
// parts here are stand-ins (a render that is `json`, an embed that hands
// its input on); running real parts needs transduce's routers and render's
// renderers, and is alchemy-cli's.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { Output, compileSources, translate } from '../dist/alchemy'

import { OPTIONS, thrown } from './common'

const T = translate

// The inferred table's binding and the CSV options, spelled out, so the
// mains below are held to the Rust crate's text byte for byte.
const INFERRED = '(record (entry :columns :infer) (entry :rows (path each-index)))'
const CSV_OPTIONS =
  '(record (entry :delimiter ",") (entry :newline "\\r\\n") (entry :header true) (entry :null-text "") (entry :missing "") (entry :non-finite :literal) (entry :no-columns :empty))'

function part(id: string, t: string, render?: translate.PartText): translate.Part | undefined {
  return T.Part.fromDescriptor({
    package: `tabnas-${id}`,
    manifest: `{"languageId": "${id}", "translate": ${t}}`,
    render,
  })
}

// A render of the format's own: `json` under another name.
function own(id: string): translate.PartText {
  return { entry: `${id}-render`, source: `def ${id}-render [input] (json input)` }
}

function tree(id: string, root: string): translate.Part {
  const p = part(id, `{"reads": "tree", "writes": "tree", "root": "${root}", "render": "alchemy/render.alc"}`, own(id))
  if (undefined === p) throw new Error(`${id}: a part`)
  return p
}

// A part as the integration tests make one: its render alchemy's `csv` or
// `json` when the manifest names one, else its own.
function full(id: string, t: string, lift?: translate.PartText, embed?: translate.PartText): translate.Part {
  let render: translate.PartText
  if (t.includes('"render": "csv"')) render = { entry: 'csv' }
  else if (t.includes('"render": "json"')) render = { entry: 'json' }
  else render = own(id)
  const p = T.Part.fromDescriptor({
    package: `tabnas-${id}`,
    manifest: `{"languageId": "${id}", "translate": ${t}}`,
    lift,
    embed,
    render,
  })
  if (undefined === p) throw new Error(`${id}: a part`)
  return p
}

describe('translate', () => {
  it('a descriptor reads into a part', () => {
    const toml = tree('toml', 'object')
    assert.equal(toml.root, 'object')
    assert.deepStrictEqual(toml.reads, ['tree'])
    assert.deepStrictEqual(toml.render, {
      kind: 'alc',
      alc: { file: 'tabnas-toml/alchemy/render.alc', entry: 'toml-render', text: 'def toml-render [input] (json input)' },
    })
    const json = part('json', '{"reads": "tree", "writes": "tree", "render": "json"}', { entry: 'json' })!
    assert.deepStrictEqual([json.render, json.root], [{ kind: 'json' }, 'any'])
    // A root, a shape or a render this module cannot take is no part; an
    // embed with no schema is none either.
    assert.equal(
      part('x', '{"reads": "tree", "writes": "tree", "root": "table", "render": "alchemy/render.alc"}', own('x')),
      undefined,
    )
    assert.equal(part('x', '{"reads": "blob", "writes": "tree", "render": "alchemy/render.alc"}', own('x')), undefined)
    assert.equal(part('x', '{"reads": "tree", "writes": "tree", "render": "x.txt"}', own('x')), undefined)
    assert.equal(part('x', '{"reads": "tree"}', own('x')), undefined)
    const embedded = T.Part.fromDescriptor({
      package: 'tabnas-xml',
      manifest:
        '{"languageId": "xml", "translate": {"reads": "tree", "writes": "tree", "root": "any", "embed": "alchemy/embed.alc", "render": "alchemy/render.alc"}}',
      embed: { entry: 'xml-embed', source: 'def xml-embed [input] input' },
      render: own('xml'),
    })
    assert.equal(embedded, undefined, 'an embed names a schema')
    // A manifest that is no JSON, or whose part the package does not hand
    // over, is no part either; the loss keeps its sentences.
    assert.equal(T.Part.fromDescriptor({ package: 'tabnas-x', manifest: '{', render: own('x') }), undefined)
    assert.equal(
      part('x', '{"reads": "tree", "writes": "tree", "lift": "alchemy/lift.alc", "render": "alchemy/render.alc"}', own('x')),
      undefined,
    )
    const lossy = part(
      'x',
      '{"reads": ["tree"], "writes": "tree", "render": "alchemy/render.alc", "loss": ["One.", 2, "Two."]}',
      own('x'),
    )!
    assert.deepStrictEqual(lossy.loss, ['One.', 'Two.'])
  })

  it('the route wraps a root and embeds into a schema', () => {
    const o = T.Options.default()
    const toml = tree('toml', 'object')
    let c = T.compose(undefined, toml, o, 'main')
    assert.equal(c.main, 'def export [input] (toml-render (wrap-object "items" input))')
    assert.deepStrictEqual(c.adapters, [{ kind: 'wrap-object', key: 'items' }])
    assert.equal(c.front, 'tree')
    assert.equal(c.sources[0].file, 'tabnas-toml/alchemy/render.alc')
    const jsonl = tree('jsonl', 'array')
    c = T.compose(undefined, jsonl, o, 'main')
    assert.equal(c.main, 'def export [input] (jsonl-render (wrap-array input))')
    const yaml = tree('yaml', 'any')
    c = T.compose(undefined, yaml, o, 'main')
    assert.equal(c.main, 'def export [input] (yaml-render input)')
    assert.deepStrictEqual(c.adapters, [])
    // A schema the source's events are not of: through the embed.
    const xml: translate.Part = {
      ...tree('xml', 'any'),
      schema: 'xml-element',
      embed: { file: 'tabnas-xml/alchemy/embed.alc', entry: 'xml-embed', text: 'def xml-embed [input] input' },
    }
    c = T.compose(undefined, xml, o, 'main')
    assert.equal(c.main, 'def export [input] (xml-render (xml-embed input))')
    assert.equal(c.sources.length, 2)
    // A source of that schema: as it is.
    c = T.compose(xml, xml, o, 'main')
    assert.equal(c.main, 'def export [input] (xml-render input)')
    // A schema-only target refuses another tree before any output.
    const css: translate.Part = { ...tree('css', 'any'), schema: 'css' }
    const f = thrown(() => T.compose(undefined, css, o, 'main'))
    assert.equal(f.code, 'TARGET_VALUE_UNREPRESENTABLE')
    assert.ok(f.message.startsWith('schema_only: css writes a css tree'), String(f))
  })

  it("records targets take a tree's rows and a lifted format's records", () => {
    const o = T.Options.default()
    const csv = part('csv', '{"reads": "tree", "writes": "records", "root": "array", "render": "csv"}', {
      entry: 'csv',
    })!
    let c = T.compose(undefined, csv, o, 'main')
    assert.equal(c.main, `def export [input] (csv ${CSV_OPTIONS} (table-from-json ${INFERRED} (wrap-array input)))`)
    assert.deepStrictEqual(c.adapters, [{ kind: 'wrap-array' }, { kind: 'inferred-table' }])
    assert.equal(c.front, 'none')
    assert.equal(T.CSV_OPTIONS, CSV_OPTIONS)
    const md = T.Part.fromDescriptor({
      package: 'tabnas-markdown',
      manifest:
        '{"languageId": "markdown", "translate": {"reads": ["records", "tree"], "writes": "records", "root": "array", "lift": "alchemy/lift.alc", "render": "alchemy/render.alc"}}',
      lift: { entry: 'markdown-lift', source: 'def markdown-lift [input] input' },
      render: own('markdown'),
    })!
    c = T.compose(md, csv, o, 'main')
    assert.equal(c.main, `def export [input] (csv ${CSV_OPTIONS} (markdown-lift input))`)
    assert.deepStrictEqual(c.adapters, [])
    // A lifted format into a tree's render: its tree as it is.
    const yaml = tree('yaml', 'any')
    c = T.compose(md, yaml, o, 'main')
    assert.equal(c.main, 'def export [input] (yaml-render input)')
    // JSON over the source's events as they are: the host's renderer.
    const json = part('json', '{"reads": "tree", "writes": "tree", "render": "json"}', { entry: 'json' })!
    c = T.compose(undefined, json, o, 'main')
    assert.equal(c.native, 'json')
    // A number that is not finite is written as null, which JSON has a
    // spelling for.
    assert.equal(c.main, 'def export [input] (json (record (entry :non-finite :null)) input)')
  })

  it("a program's output stands in the source's place", () => {
    const o = T.Options.default()
    const toml = tree('toml', 'object')
    let c = T.composeProgram('TableRows/1', toml, o, 'main')
    assert.equal(c.main, 'def export [input] (toml-render (wrap-object "items" (records (program-export input))))')
    assert.equal(c.front, 'none')
    c = T.composeProgram('JsonEvents/1', toml, o, 'main')
    assert.equal(c.main, 'def export [input] (toml-render (wrap-object "items" (program-export input)))')
    const f = thrown(() => T.composeProgram('Text', toml, o, 'main'))
    assert.ok(f.message.startsWith('render_of_text'), String(f))
    const key: translate.Options = { key: 'a "b"' }
    c = T.compose(undefined, toml, key, 'main')
    assert.equal(c.main, 'def export [input] (toml-render (wrap-object "a \\"b\\"" input))')
    assert.ok(c.loss.some((l) => l.includes('"a \\"b\\""')))
  })

  // A program writing to a schema-only target makes that schema's tree:
  // such a target, which refuses another format's tree, takes a program's
  // events into its render. Into a target with an embed a program's output
  // is a plain tree, embedded like any source's, its table through
  // `records`; the shape adapters apply as before.
  it("a program makes a schema-only target's tree", () => {
    const o = T.Options.default()
    const css: translate.Part = { ...tree('css', 'any'), schema: 'css' }
    let c = T.composeProgram('JsonEvents/1', css, o, 'main')
    assert.equal(c.main, 'def export [input] (css-render (program-export input))')
    assert.deepStrictEqual(c.adapters, [])
    c = T.composeProgram('TableRows/1', css, o, 'main')
    assert.deepStrictEqual(c.adapters, [{ kind: 'records' }])
    // A root of the wrong kind is wrapped for a schema-only target too.
    const object: translate.Part = { ...tree('x', 'object'), schema: 'x-tree' }
    c = T.composeProgram('JsonEvents/1', object, o, 'main')
    assert.deepStrictEqual(c.adapters, [{ kind: 'wrap-object', key: 'items' }])
    const xml: translate.Part = {
      ...tree('xml', 'any'),
      schema: 'xml-element',
      embed: { file: 'tabnas-xml/alchemy/embed.alc', entry: 'xml-embed', text: 'def xml-embed [input] input' },
    }
    c = T.composeProgram('JsonEvents/1', xml, o, 'main')
    assert.equal(c.main, 'def export [input] (xml-render (xml-embed (program-export input)))')
    assert.deepStrictEqual(c.adapters, [{ kind: 'embed' }])
    assert.equal(c.sources.length, 2)
    c = T.composeProgram('TableRows/1', xml, o, 'main')
    assert.equal(c.main, 'def export [input] (xml-render (xml-embed (records (program-export input))))')
    assert.deepStrictEqual(c.adapters, [{ kind: 'records' }, { kind: 'embed' }])
  })

  // Every route the module composes compiles with the standard library's
  // adapters, and its output is a text: the root adapters, the inferred
  // table over a wrapped root, an embed, a lift, a program's events and
  // its table, into a schema-only target as into any other.
  it('every route compiles to a text', () => {
    const o = T.Options.default()
    const csv = full('csv', '{"reads": "tree", "writes": "records", "root": "array", "render": "csv"}')
    const md = full(
      'markdown',
      '{"reads": ["records", "tree"], "writes": "records", "root": "array", "lift": "alchemy/lift.alc", "render": "alchemy/render.alc"}',
      {
        entry: 'markdown-lift',
        source: `def markdown-lift [input] (table-from-json ${INFERRED} input)`,
      },
    )
    const xml = full(
      'xml',
      '{"reads": "tree", "writes": "tree", "root": "any", "schema": "xml-element", "embed": "alchemy/embed.alc", "render": "alchemy/render.alc"}',
      undefined,
      { entry: 'xml-embed', source: 'def xml-embed [input] (wrap-array input)' },
    )
    const targets = [tree('toml', 'object'), tree('jsonl', 'array'), tree('yaml', 'any'), csv, xml]
    const sources: Array<translate.Part | undefined> = [undefined, csv, md, xml]
    // A program's events and its table into `target`.
    const programs = (target: translate.Part): void => {
      for (const output of ['JsonEvents/1', 'TableRows/1'] as const) {
        const c = T.composeProgram(output, target, o, 'main')
        const user = {
          file: 'p.alc',
          text:
            'TableRows/1' === output
              ? `def export [input] (table-from-json ${INFERRED} input)`
              : 'def export [input] input',
        }
        let compiled: Output
        try {
          compiled = c.compile(user, OPTIONS).output
        } catch (f) {
          throw new Error(`${output} into ${target.id}: ${f}\n${c.main}`)
        }
        assert.equal(compiled, 'Text', c.main)
        assert.equal(c.front, 'none')
      }
    }
    for (const target of targets) {
      for (const source of sources) {
        const c = T.compose(source, target, o, 'main')
        let output: Output
        try {
          output = c.compile(undefined, OPTIONS).output
        } catch (f) {
          throw new Error(`${source?.id ?? 'tree'} into ${target.id}: ${f}\n${c.main}`)
        }
        assert.equal(output, 'Text', c.main)
      }
      programs(target)
    }
    // A schema-only target refuses each source above, and takes a program's
    // output, which makes its tree.
    const css = full(
      'css',
      '{"reads": "tree", "writes": "tree", "root": "any", "schema": "css", "render": "alchemy/render.alc"}',
    )
    for (const source of sources) thrown(() => T.compose(source, css, o, 'main'))
    programs(css)
  })

  // The inferred table keeps a repeated member's last value, as an export
  // does; a route without it keeps the default, which refuses one.
  it('a route through the inferred table takes the last of a repeated member', () => {
    const o = T.Options.default()
    const csv = full('csv', '{"reads": "tree", "writes": "records", "root": "array", "render": "csv"}')
    assert.equal(T.compose(undefined, csv, o, 'main').compile(undefined, OPTIONS).duplicates, 'last_wins')
    assert.equal(T.compose(undefined, tree('yaml', 'any'), o, 'main').compile(undefined, OPTIONS).duplicates, 'reject')
    // The composed program is the linked sources, the main last.
    const c = T.compose(undefined, tree('toml', 'object'), o, 'main')
    const linked = compileSources([...c.sources, { file: c.mainFile, text: c.main }], OPTIONS)
    assert.equal(linked.output, c.compile(undefined, OPTIONS).output)
  })

  // A part whose render does not compile is a failure naming its file.
  it('a render that does not compile names its file', () => {
    const broken = T.Part.fromDescriptor({
      package: 'tabnas-x',
      manifest: '{"languageId": "x", "translate": {"reads": "tree", "writes": "tree", "render": "alchemy/render.alc"}}',
      render: { entry: 'x-render', source: 'def x-render [input]\n  (nope input)' },
    })!
    const c = T.compose(undefined, broken, T.Options.default(), 'main')
    const fail = thrown(() => c.compile(undefined, OPTIONS))
    assert.ok(fail.message.startsWith('unknown_name'), String(fail))
    assert.equal(fail.file, 'tabnas-x/alchemy/render.alc')
    assert.deepStrictEqual([fail.row, fail.col], [2, 4])
  })
})
