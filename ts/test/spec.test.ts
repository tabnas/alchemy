/* Copyright (c) 2026 tabnas, MIT License */

// The shared fixtures in ../../test/spec, run through @tabnas/support's
// runner as every grammar repository runs its own (rs/tests/spec_test.rs
// is the Rust half). What is specific to this package is what a row's
// input becomes: the canonical form of the parsed program (reader.tsv),
// or of the desugared program (pipe.tsv), as a JSON string; the plan
// report (check.tsv); the bytes a run writes (run.tsv).
//
// Every row of every file runs. run.tsv runs each row twice, with the
// standard compositions native and through the library's text
// (`withNative(false)`), and a row whose two paths differ in a byte, a
// code or a position fails whatever its expected cell says.

import { describe, it } from 'node:test'
import assert from 'node:assert'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

import { SpecRow, isErrorExpect, loadSpec, loadSpecDir, makeRunner, parseExpect } from '@tabnas/support'
import { make as makeJson } from '@tabnas/json'
import { BytesWriter } from '@tabnas/render'
import { Limits, Metrics, ParserSource, Prune, SourceMode } from '@tabnas/transduce'

import {
  Program,
  Renderer,
  canonical,
  desugarProgram,
  format,
  grammarDocument,
  make as makeAlchemy,
  parse,
  rendererNamed,
  sameProgram,
} from '../dist/alchemy'
import { isFail } from '../dist/fail'

import { OWN_CODES, REPO_ROOT, SPEC_DIR, compile, failCode } from './common'

// The finer code a row pins, and the position the failure names.
const runnerOptions = {
  errorCode: (err: unknown) => failCode(err),
  errorPos: (err: any) => ({ row: err?.row, col: err?.col }),
}

describe('fixtures', () => {
  // Every fixture the directory holds has a runner below; a new file added
  // without one fails here rather than passing silently.
  it('every fixture has a runner', () => {
    const files = loadSpecDir(SPEC_DIR)
      .map((spec) => spec.file)
      .sort()
    assert.deepStrictEqual(files, ['check.tsv', 'pipe.tsv', 'reader.tsv', 'run.tsv'])
    // And test/AGENTS.md, the fixtures' guide, describes each one.
    const guide = readFileSync(join(REPO_ROOT, 'test', 'AGENTS.md'), 'utf8')
    for (const file of files) {
      assert.ok(guide.includes(`[\`${file}\`](spec/${file})`), `test/AGENTS.md does not describe ${file}`)
    }
  })

  // The columns a run row reads by name, so a renamed header fails here
  // rather than reading empty cells.
  it('run.tsv has its columns', () => {
    const spec = loadSpec(join(SPEC_DIR, 'run.tsv'))
    assert.deepStrictEqual(spec.header, ['input', 'expected', 'doc', 'render'])
    assert.ok(spec.rows.every((row: SpecRow) => -1 !== row.index_of('doc') && -1 !== row.index_of('render')))
  })
})

makeRunner({
  ...runnerOptions,
  parse: (input: string) => canonical(parse(input)),
}).file(join(SPEC_DIR, 'reader.tsv'))

makeRunner({
  ...runnerOptions,
  parse: (input: string) => canonical(desugarProgram(parse(input), input)),
}).file(join(SPEC_DIR, 'pipe.tsv'))

// A program that checks prints its plan report; one that does not fails
// with the resolver's, the checker's or the plan evaluation's code, at the
// position it names.
makeRunner({
  ...runnerOptions,
  parse: (input: string) => compile(input, 'check').explain(),
}).file(join(SPEC_DIR, 'check.tsv'))

// Compile `program` and run it over `doc`, as `alchemy run` does: the
// document read by the JSON grammar incrementally, pruned under the
// program's row selector when it has one, with the default limits.
function runBoth(program: string, doc: string, render: Renderer | undefined, native: boolean): string {
  let compiled = compile(program, 'run')
  if (!native) compiled = compiled.withNative(false)
  return drive(compiled, doc, render)
}

function drive(program: Program, doc: string, render: Renderer | undefined): string {
  const limits = Limits.default()
  const metrics = new Metrics()
  const writer = new BytesWriter()
  const sink = program.sink(writer, render, limits, metrics)
  const selector = program.rowSelector()
  const prune = undefined === selector ? Prune.never() : Prune.under(selector)
  new ParserSource(makeJson(), doc)
    .grammar('json')
    .mode(SourceMode.incremental(prune))
    .limits(limits)
    .metrics(metrics)
    .run(sink)
  return writer.text()
}

type Outcome = { ok: true; text: string } | { ok: false; fail: unknown }

function outcome(work: () => string): Outcome {
  try {
    return { ok: true, text: work() }
  } catch (fail) {
    return { ok: false, fail }
  }
}

function shown(o: Outcome): string {
  return o.ok ? JSON.stringify(o.text) : String(o.fail)
}

// A program run over a JSON document: the bytes it writes, or the failure.
// See test/AGENTS.md for the columns. Both paths, compared before the
// expected cell is.
makeRunner({
  ...runnerOptions,
  parse: (program: string, row: SpecRow) => {
    const cell = row.unescNamed('doc')
    const doc = '' === cell ? 'null' : cell
    const name = row.named('render')
    let render: Renderer | undefined
    if ('' !== name) {
      render = rendererNamed(name)
      if (undefined === render) throw new Error(`${row.where()}: render is csv, json or empty`)
    }
    const native = outcome(() => runBoth(program, doc, render, true))
    const interpreted = outcome(() => runBoth(program, doc, render, false))
    let agree: boolean
    if (native.ok && interpreted.ok) {
      agree = native.text === interpreted.text
    } else if (!native.ok && !interpreted.ok) {
      // Failures both, whichever copy of transduce made each (the native
      // path's renderers are render's): one code, at one position.
      const a: any = native.fail
      const b: any = interpreted.fail
      agree =
        isFail(a) &&
        isFail(b) &&
        failCode(a) === failCode(b) &&
        a.row === b.row &&
        a.col === b.col
    } else {
      agree = false
    }
    if (!agree) {
      throw new Error(
        `native and interpreted runs disagree:\n  native:      ${shown(native)}\n  interpreted: ${shown(interpreted)}`,
      )
    }
    if (native.ok) return native.text
    throw native.fail
  },
}).file(join(SPEC_DIR, 'run.tsv'))

// The failures a row's program meets: the reader's (reader.tsv), the
// desugarer's (pipe.tsv), compile's (check.tsv), or a run's, both paths
// (run.tsv), as the runners above meet them.
function rowFailures(file: string, input: string, row: SpecRow): unknown[] {
  const met = (work: () => unknown): unknown[] => {
    try {
      work()
      return []
    } catch (fail) {
      return [fail]
    }
  }
  switch (file) {
    case 'reader.tsv':
      return met(() => parse(input))
    case 'pipe.tsv':
      return met(() => desugarProgram(parse(input), input))
    case 'check.tsv':
      return met(() => compile(input, 'check'))
    default: {
      const cell = row.unescNamed('doc')
      const doc = '' === cell ? 'null' : cell
      const name = row.named('render')
      const render = '' === name ? undefined : rendererNamed(name)
      return [true, false].flatMap((native) => met(() => runBoth(input, doc, render, native)))
    }
  }
}

// Whether `text` is an instance of the template `line`. The engine trims
// the messages it writes from a template, so a `{name}` that ends a line
// may take the space before it with it.
function instanceOf(line: string, text: string): boolean {
  if (fills(line, text)) return true
  const bare = line.replace(/\s*\{[a-z_]+\}$/, '')
  return bare !== line && fills(bare, text)
}

// Whether `text` fills the template `line`: its fixed parts in order, each
// `{name}` any text, none included.
function fills(line: string, text: string): boolean {
  const parts = line.split(/\{[a-z_]+\}/)
  if (1 === parts.length) return text === line
  const first = parts[0]
  const last = parts[parts.length - 1]
  const end = text.length - last.length
  if (end < first.length || !text.startsWith(first) || !text.endsWith(last)) return false
  let at = first.length
  for (const part of parts.slice(1, -1)) {
    const found = text.indexOf(part, at)
    if (-1 === found || found + part.length > end) return false
    at = found + part.length
  }
  return true
}

// The fixed text of a template: what a more specific line has more of. A
// failure meets the most specific line it is an instance of, the first of
// equals.
function fixedLength(line: string): number {
  return line.replace(/\{[a-z_]+\}/g, '').length
}

// Every failure the shared fixtures meet is declared in the grammar
// document (rs/tests/spec_test.rs, `the_raised_messages_match_the_document`):
// the finer code that leads the message is a key of the installed
// `options.error`, the engine's and this grammar's, and the text after it is
// a line of that entry, each `{name}` standing for what the raising site
// fills in (a failure raised inside the standard library ends with its
// position there, ` (at stdlib/...)`, which is not part of the text). Each
// line of each code the document declares is the most specific line some
// row meets, so the document holds no text nothing raises, and each of its
// codes has a hint. A finer code comes with the same code wherever it is
// raised (`bad_let` is a DSL_PARSE_ERROR from the desugarer and from the
// resolver alike). A raising site whose code or text drifts fails here.
describe('the catalogue', () => {
  it('the raised messages match the document', () => {
    const catalogue: Record<string, string> = makeAlchemy().options.error
    const document = grammarDocument()
    const met = new Set<string>()
    // The codes each finer code is raised with: one, wherever it is raised.
    const raisedAs = new Map<string, Set<string>>()
    const problems: string[] = []
    let failures = 0
    for (const spec of loadSpecDir(SPEC_DIR)) {
      for (const row of spec.rows) {
        if (!isErrorExpect(row.col(1))) continue
        const input = row.unesc(row.resolve(0))
        for (const fail of rowFailures(spec.file, input, row)) {
          if (!isFail(fail) || !OWN_CODES.includes(fail.code)) continue
          failures++
          const split = fail.message.indexOf(': ')
          if (-1 === split) {
            problems.push(`${row.where()}: ${fail.code} has no finer code: ${fail.message}`)
            continue
          }
          const code = fail.message.substring(0, split)
          if (!raisedAs.has(code)) raisedAs.set(code, new Set())
          raisedAs.get(code)?.add(fail.code)
          let text = fail.message.substring(split + 2)
          const library = text.lastIndexOf(' (at stdlib/')
          if (-1 !== library && text.endsWith(')')) text = text.substring(0, library)
          const entry = catalogue[code]
          if ('string' !== typeof entry) {
            problems.push(`${row.where()}: ${code} is not declared in options.error`)
            continue
          }
          const lines = entry
            .split('\n')
            .filter((line) => instanceOf(line, text))
            .sort((a, b) => fixedLength(b) - fixedLength(a))
          if (0 === lines.length) {
            problems.push(`${row.where()}: no line of options.error.${code} is ${JSON.stringify(text)}`)
          } else {
            met.add(code + '\n' + lines[0])
          }
        }
      }
    }
    assert.ok(failures > 0, 'the fixtures meet failures')
    for (const [code, uppers] of raisedAs) {
      if (uppers.size > 1) problems.push(`${code} is raised as ${[...uppers].sort().join(' and ')}; a finer code has one code`)
    }
    for (const [code, entry] of Object.entries<string>(document.options.error)) {
      if ('string' !== typeof document.options.hint[code]) problems.push(`options.hint.${code} is not declared`)
      for (const line of entry.split('\n')) {
        if (!met.has(code + '\n' + line)) problems.push(`options.error.${code}: no fixture row meets ${JSON.stringify(line)}`)
      }
    }
    assert.deepStrictEqual(problems, [])
  })
})

// `format` prints a program the reader reads back to the same forms, spans
// aside, for every program the fixtures parse.
describe('layout round trip', () => {
  it('format round-trips every fixture row', () => {
    const failures: string[] = []
    let rows = 0
    for (const spec of loadSpecDir(SPEC_DIR)) {
      for (const row of spec.rows) {
        if (isErrorExpect(row.col(1))) continue
        rows++
        const input = row.unesc(row.resolve(0))
        let program
        try {
          program = parse(input)
        } catch (_e) {
          // The fixture runner reports a parse that should succeed.
          continue
        }
        const layout = format(program)
        try {
          const again = parse(layout)
          if (!sameProgram(program, again)) {
            failures.push(
              `${row.where()}: format changed the program\n  layout: ${JSON.stringify(layout)}\n  read:   ${canonical(again)}\n  was:    ${canonical(program)}`,
            )
          }
        } catch (fail) {
          failures.push(`${row.where()}: the layout form does not parse: ${fail}\n  layout: ${JSON.stringify(layout)}`)
        }
      }
    }
    assert.ok(rows > 0, 'the fixtures hold value rows')
    assert.deepStrictEqual(failures, [])
  })
})

// The fenced examples of docs/language.md, each `alchemy` block with the
// `canonical`, `core` or `check` block after it, are fixture rows: the
// input is a row of reader.tsv, pipe.tsv or check.tsv, and the result
// shown is that row's expected value.
describe('the language reference', () => {
  it('every example is a fixture row', () => {
    const doc = readFileSync(join(REPO_ROOT, 'docs', 'language.md'), 'utf8')
    const rows = (file: string) =>
      new Map(
        loadSpec(join(SPEC_DIR, file)).rows.map((row: SpecRow) => [
          row.unesc(row.resolve(0)).replace(/\n+$/, ''),
          row.col(1),
        ]),
      )
    const fixtures: Record<string, Map<string, string>> = {
      canonical: rows('reader.tsv'),
      core: rows('pipe.tsv'),
      check: rows('check.tsv'),
    }
    const blocks: Array<[string, string]> = []
    let open: [string, string[]] | undefined
    for (const line of doc.split('\n')) {
      const fence = line.startsWith('```') ? line.substring(3) : undefined
      if (undefined === open && undefined !== fence && '' !== fence) {
        open = [fence, []]
      } else if (undefined !== open && '' === fence) {
        blocks.push([open[0], open[1].join('\n')])
        open = undefined
      } else if (undefined !== open) {
        open[1].push(line)
      }
    }
    let examples = 0
    const failures: string[] = []
    blocks.forEach(([language, input], index) => {
      if ('alchemy' !== language) return
      examples++
      const next = blocks[index + 1]
      const fixture = undefined === next ? undefined : fixtures[next[0]]
      if (undefined === fixture) {
        failures.push(`${JSON.stringify(input)}: an alchemy block is followed by a canonical, core or check block`)
        return
      }
      const cell = fixture.get(input.replace(/\n+$/, ''))
      if (undefined === cell) {
        failures.push(`${JSON.stringify(input)}: not a fixture row`)
        return
      }
      const shown = isErrorExpect(cell) ? cell : parseExpect(cell)
      if ('string' !== typeof shown) {
        failures.push(`${JSON.stringify(input)}: the fixture expects ${JSON.stringify(shown)}`)
        return
      }
      // A report ends with a line feed; a fenced block has none.
      if (shown.replace(/\n+$/, '') !== next[1]) {
        failures.push(`${JSON.stringify(input)}: the page shows ${JSON.stringify(next[1])}, the fixture pins ${JSON.stringify(shown)}`)
      }
    })
    assert.ok(examples > 0, 'the reference holds examples')
    assert.deepStrictEqual(failures, [])
  })
})
