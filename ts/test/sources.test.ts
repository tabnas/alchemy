/* Copyright (c) 2026 tabnas, MIT License */

// `compileSources` (rs/tests/sources_test.rs): several sources linked into
// one program, as a host links a format's parts (libraries of definitions
// prefixed by the format's name, with no `export`) with the program that
// calls them. The program runs as the same text in one file runs; a
// failure in any source carries that source's file, row and column, in the
// field and in the failure's display, at every stage that positions one:
// the reader, the desugarer, the resolver, the checker and the run.

import { describe, it } from 'node:test'
import assert from 'node:assert'

import { Program, Source } from '../dist/alchemy'

import { compile, compileSources, drive, ok, thrown } from './common'

// A render part: definitions prefixed by the format's name, no `export`.
const PART =
  '; A part of the lines format: each item quoted, one to a line.\ndef lines-line [item]\n  concat (quoted item) "\\n"\n\ndef lines-render [items]\n  concat-map lines-line items\n'

const PART_FILE = 'lines/render.alc'

// The program that calls the part.
const MAIN = 'def export [input]\n  lines-render (select (path each-index) input)\n'

const MAIN_FILE = 'main.alc'

function sources(list: Array<[string, string]>): Source[] {
  return list.map(([file, text]) => ({ file, text }))
}

// The part second, as the design's diagnostics test has it.
function linked(part: string): Program {
  return compileSources(
    sources([
      [MAIN_FILE, MAIN],
      [PART_FILE, part],
    ]),
  )
}

// The 1-based row and column of `needle` on the 1-based `row` of `text`,
// as a failure carries them: the column in characters.
function at(text: string, row: number, needle: string): [number, number] {
  const line = text.split('\n')[row - 1]
  const index = line.indexOf(needle)
  assert.ok(-1 !== index, `${needle} is on row ${row}`)
  return [row, [...line.substring(0, index)].length + 1]
}

// The failure names `file` and the position, in the fields and in its
// display.
function assertAt(fail: any, file: string, [row, col]: [number, number]): void {
  assert.equal(fail.file, file, String(fail))
  assert.deepStrictEqual([fail.row, fail.col], [row, col], String(fail))
  const shown = `(${file}:${row}:${col})`
  assert.ok(String(fail).endsWith(shown), `${fail} does not end ${shown}`)
  assert.equal(fail.toJSON().file, file, String(fail))
}

describe('sources', () => {
  // Two sources are one namespace: the program calls the part's
  // definitions, in either order of the sources, and runs as the same
  // texts in one file run.
  it('linked sources run as one program', () => {
    const input = '["a","b\\"c","d\\ne"]'
    const expected = '"a"\n"b\\"c"\n"d\\ne"\n'
    const mainFirst = linked(PART)
    assert.equal(ok(mainFirst, input), expected)
    const partFirst = compileSources(
      sources([
        [PART_FILE, PART],
        [MAIN_FILE, MAIN],
      ]),
    )
    assert.equal(ok(partFirst, input), expected)
    const one = compile(`${MAIN}\n${PART}`, 'one.alc')
    assert.equal(ok(one, input), expected)
    // The program is named by its first source, and its plan is reported
    // as one program's.
    assert.equal(mainFirst.file(), MAIN_FILE)
    assert.equal(partFirst.file(), PART_FILE)
    assert.equal(mainFirst.explain(), one.explain())
    // A part may call back into the program: the namespace is one.
    const back =
      'def lines-line [item]\n  concat (main-mark item) "\\n"\n\ndef lines-render [items]\n  concat-map lines-line items\n'
    const main = `${MAIN}\ndef main-mark [s] (concat "* " s)\n`
    const program = compileSources(
      sources([
        [MAIN_FILE, main],
        [PART_FILE, back],
      ]),
    )
    assert.equal(ok(program, '["x"]'), '* x\n')
  })

  // Every stage that positions a failure positions one in the second
  // source by that source's own rows and columns, and names its file.
  it('a failure in the second source names the second file', () => {
    // The resolver: a name nothing defines.
    const typo = PART.replace('(quoted item)', '(quoted itme)')
    let fail = thrown(() => linked(typo))
    assert.equal(fail.code, 'DSL_TYPE_ERROR', String(fail))
    assert.ok(fail.message.startsWith('unknown_name: '), String(fail))
    assertAt(fail, PART_FILE, at(typo, 3, 'itme'))

    // The checker: an argument of the wrong type.
    const number = PART.replace('(quoted item)', '(quoted 1)')
    fail = thrown(() => linked(number))
    assert.equal(fail.code, 'DSL_TYPE_ERROR', String(fail))
    assert.ok(fail.message.startsWith('type_mismatch: '), String(fail))
    assertAt(fail, PART_FILE, at(number, 3, '1)'))

    // The checker, following a stream into the part: a render handed the
    // wrong shape fails at the render's file and line, not at the call.
    const csv = 'def lines-render [items]\n  csv csv-options items\n'
    const evs = 'def export [input]\n  lines-render (events input)\n'
    fail = thrown(() =>
      compileSources(
        sources([
          [MAIN_FILE, evs],
          [PART_FILE, csv],
        ]),
      ),
    )
    assert.equal(fail.code, 'DSL_TYPE_ERROR', String(fail))
    assert.ok(fail.message.startsWith('protocol_mismatch: '), String(fail))
    assert.equal(fail.file, PART_FILE, String(fail))
    assert.equal(fail.row, 2, String(fail))

    // The reader: a list left open.
    const open = PART.replace('(quoted item)', '(quoted item')
    fail = thrown(() => linked(open))
    assert.equal(fail.code, 'DSL_PARSE_ERROR', String(fail))
    assert.equal(fail.file, PART_FILE, String(fail))
    assert.ok(undefined !== fail.row, String(fail))
    assert.ok(String(fail).includes(`(${PART_FILE}:`), String(fail))

    // The desugarer: a `pipe` step that is no call.
    const pipe = PART.replace('concat-map lines-line items', 'pipe items ()')
    fail = thrown(() => linked(pipe))
    assert.equal(fail.code, 'DSL_PARSE_ERROR', String(fail))
    assert.ok(fail.message.startsWith('empty_step: '), String(fail))
    assert.equal(fail.file, PART_FILE, String(fail))
    assert.equal(fail.row, 6, String(fail))
  })

  // A failure the run meets in a part, from a `fail` in its definition, is
  // the program's `INPUT_INVALID` at the part's file and position.
  it('a run time failure in a part names the part', () => {
    const strict =
      'def lines-line [item]\n  match item\n    case "bad" (fail "a bad item")\n    case _ (concat (quoted item) "\\n")\n\ndef lines-render [items]\n  concat-map lines-line items\n'
    const program = linked(strict)
    assert.equal(ok(program, '["ok"]'), '"ok"\n')
    const { fail } = drive(program, '["ok","bad"]')
    assert.equal(fail.code, 'INPUT_INVALID', String(fail))
    assert.equal(fail.message, 'a bad item', String(fail))
    assertAt(fail, PART_FILE, at(strict, 3, '(fail'))
    // The interpreted twin positions it the same way.
    assertAt(drive(program.withNative(false), '["bad"]').fail, PART_FILE, at(strict, 3, '(fail'))
  })

  // The linking's own refusals: a name defined in two sources, the one
  // file name given twice, and no `export` in any source.
  it('the linking refuses a collision and a program with no export', () => {
    // A definition in both: the second is refused, and the message says
    // where the first is.
    const twice = `${PART}\ndef export [input] (json input)\n`
    let fail = thrown(() => linked(twice))
    assert.equal(fail.code, 'DSL_TYPE_ERROR', String(fail))
    assert.ok(fail.message.startsWith('duplicate_def: export is defined twice; first at main.alc:1:1'), String(fail))
    assertAt(fail, PART_FILE, at(twice, 8, 'def export'))

    // Each source has its own name.
    fail = thrown(() =>
      compileSources(
        sources([
          [MAIN_FILE, MAIN],
          [MAIN_FILE, PART],
        ]),
      ),
    )
    assert.equal(fail.code, 'DSL_TYPE_ERROR', String(fail))
    assert.ok(fail.message.startsWith('duplicate_file: main.alc '), String(fail))
    assert.deepStrictEqual([fail.file, fail.row], [undefined, undefined])

    // A part alone is no program.
    fail = thrown(() => compileSources(sources([[PART_FILE, PART]])))
    assert.ok(fail.message.startsWith('no_export: '), String(fail))
    fail = thrown(() => compileSources([]))
    assert.ok(fail.message.startsWith('no_export: '), String(fail))
  })

  // One source is `compile`: its failures carry a row and a column and no
  // file, and display as they always have.
  it('one source names no file', () => {
    const text = 'def export [input]\n  csv csv-options (events input)\n'
    const fail = thrown(() => compileSources(sources([['one.alc', text]])))
    const same = thrown(() => compile(text, 'one.alc'))
    assert.deepStrictEqual(fail.toJSON(), same.toJSON())
    assert.equal(fail.file, undefined, String(fail))
    assert.ok(undefined !== fail.row, String(fail))
    assert.equal(fail.toJSON().file, undefined, String(fail))
    assert.ok(String(fail).endsWith(`(${fail.row}:${fail.col})`), String(fail))
  })

  // A program linked under another name feeds a format's render: its
  // `export` is the format's input, in one plan under one set of limits.
  it('a program linked under another name feeds a render', () => {
    const program = compileSources([
      { file: 'program.alc', text: 'def export [input] (select (path each-index) input)\n', exportAs: 'program-export' },
      { file: 'lines.alc', text: PART },
      { file: 'main.alc', text: 'def export [input] (lines-render (program-export input))\n' },
    ])
    assert.equal(ok(program, '["a","b"]'), '"a"\n"b"\n')
    assert.deepStrictEqual([...program.resolved.defs.keys()], [
      'program-export',
      'lines-line',
      'lines-render',
      'export',
    ])
    // A failure the run meets in the renamed source names its file.
    const failing = compileSources([
      {
        file: 'program.alc',
        text: 'def export [input] (map (fn [x] (fail "no")) (select (path each-index) input))\n',
        exportAs: 'program-export',
      },
      { file: 'lines.alc', text: PART },
      { file: 'main.alc', text: 'def export [input] (lines-render (program-export input))\n' },
    ])
    const { fail } = drive(failing, '["a"]')
    assert.equal(fail.code, 'INPUT_INVALID', String(fail))
    assert.equal(fail.file, 'program.alc', String(fail))
    assert.deepStrictEqual([fail.row, fail.col], [1, 33])
  })
})
