/* Copyright (c) 2026 tabnas, MIT License */

// The shared sources every runtime embeds, held to the files at the
// repository root (rs/tests/shared_sources_test.rs is the Rust half).
//
// `alchemy-grammar.jsonic`, the grammar document, is carried between the
// `BEGIN/END EMBEDDED` markers of src/grammar.ts as a template literal;
// `stdlib/*.alc`, the library's own definitions, as the generated module
// src/stdlib/sources.ts. `npm run embed` (embed-grammar.js) writes both.
// This fails when a copy and its source differ, or when a file is in one
// place and not the other, so a forgotten embed is red rather than a
// quiet drift.

import { describe, it } from 'node:test'
import assert from 'node:assert'
import { execFileSync } from 'node:child_process'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

import { SOURCES, grammarSource } from '../dist/alchemy'

import { REPO_ROOT } from './common'

describe('shared sources', () => {
  it('the embedded grammar is the authored one', () => {
    // The literal opens with the newline the embedder writes after the
    // backtick, as the Rust raw string does.
    const authored = readFileSync(join(REPO_ROOT, 'alchemy-grammar.jsonic'), 'utf8')
    assert.ok(
      grammarSource() === '\n' + authored,
      'src/grammar.ts embeds a different grammar from alchemy-grammar.jsonic: run npm run embed',
    )
  })

  it('the embedded stdlib is the repository\'s, file for file', () => {
    const canonical = readdirSync(join(REPO_ROOT, 'stdlib'))
      .filter((name) => name.endsWith('.alc'))
      .sort()
    assert.ok(0 < canonical.length, 'stdlib/ holds the library')
    assert.deepStrictEqual(
      SOURCES.map(([path]) => path),
      canonical.map((name) => 'stdlib/' + name),
      'src/stdlib/sources.ts does not embed every stdlib/*.alc: run npm run embed',
    )
    for (const [path, text] of SOURCES) {
      const source = readFileSync(join(REPO_ROOT, path), 'utf8')
      assert.ok(text === source, `src/stdlib/sources.ts holds a different ${path}: run npm run embed`)
    }
  })

  // The embedder itself agrees: every runtime's copy is current (the Rust
  // crate's too, which its own test also holds).
  it('embed-grammar.js --check finds no drift', () => {
    const out = execFileSync(process.execPath, [join(__dirname, '..', 'embed-grammar.js'), '--check'], {
      encoding: 'utf8',
    })
    assert.match(out, /embedded copies match their sources/)
  })
})
