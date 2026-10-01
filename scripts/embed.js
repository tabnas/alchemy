#!/usr/bin/env node

// Embed the shared sources into each runtime. Run via: make embed
// (or: node scripts/embed.js). `--check` writes nothing and exits 1 when
// a runtime's copy differs from its source, naming it.
//
// Two shared sources live at the repository root, and every runtime
// carries them VERBATIM:
//
// - alchemy-grammar.jsonic, the grammar document, copied between the
//   `--- BEGIN/END EMBEDDED alchemy-grammar.jsonic ---` markers of each
//   runtime's grammar source, as a string literal. Never hand-edit
//   between the markers.
// - stdlib/*.alc, the standard library's own definitions, copied file for
//   file into each runtime's package-local stdlib directory, because a
//   published package (a crate, a Go module) holds nothing above its own
//   root. Never edit a copy.
//
// Each runtime also holds its copies to the sources in its own tests
// (Rust: rs/tests/shared_sources_test.rs), so a forgotten embed fails
// there.
//
// Only the Rust crate exists today. The TypeScript and Go ports add their
// targets to GRAMMAR_TARGETS and STDLIB_DIRS below (a template literal in
// ts/src/alchemy.ts, a raw string in go/alchemy.go, `go/stdlib/` for
// go:embed); when ts/ arrives, this script moves to ts/embed-grammar.js
// and `npm run embed` runs it, as in every other plugin repository.
//
// No dependencies: Node's fs and path only.

const fs = require('fs')
const path = require('path')

const ROOT = path.join(__dirname, '..')
const GRAMMAR_FILE = path.join(ROOT, 'alchemy-grammar.jsonic')
const STDLIB_DIR = path.join(ROOT, 'stdlib')

const BEGIN = '// --- BEGIN EMBEDDED alchemy-grammar.jsonic ---'
const END = '// --- END EMBEDDED alchemy-grammar.jsonic ---'

const check = process.argv.includes('--check')
let drift = 0

const grammar = fs.readFileSync(GRAMMAR_FILE, 'utf8')

// A Rust raw string has no escapes, so the grammar goes in verbatim. Its
// token names open with `"#`, so two hashes is the floor.
function rustLiteral(text) {
  if (text.includes('"##')) {
    throw new Error('the grammar contains `"##`, incompatible with the r## raw string')
  }
  return 'pub const GRAMMAR_TEXT: &str = r##"\n' + text + '"##;\n'
}

const GRAMMAR_TARGETS = [
  { file: path.join(ROOT, 'rs', 'src', 'grammar.rs'), literal: rustLiteral },
]

const STDLIB_DIRS = [path.join(ROOT, 'rs', 'stdlib')]

function write(file, text) {
  const was = fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : null
  if (was === text) return
  const name = path.relative(ROOT, file)
  if (check) {
    console.error(`${name} differs from its source: run make embed`)
    drift++
    return
  }
  fs.writeFileSync(file, text)
  console.log('embedded', name)
}

for (const { file, literal } of GRAMMAR_TARGETS) {
  const src = fs.readFileSync(file, 'utf8')
  const start = src.indexOf(BEGIN)
  const end = src.indexOf(END)
  if (-1 === start || -1 === end || end < start) {
    throw new Error('embed markers not found in ' + file)
  }
  write(file, src.substring(0, start) + BEGIN + '\n' + literal(grammar) + src.substring(end))
}

const sources = fs.readdirSync(STDLIB_DIR).filter((name) => name.endsWith('.alc')).sort()
for (const dir of STDLIB_DIRS) {
  for (const name of sources) {
    write(path.join(dir, name), fs.readFileSync(path.join(STDLIB_DIR, name), 'utf8'))
  }
  for (const name of fs.readdirSync(dir).filter((name) => name.endsWith('.alc'))) {
    if (!sources.includes(name)) {
      const stale = path.relative(ROOT, path.join(dir, name))
      if (check) {
        console.error(`${stale} has no source in stdlib/: run make embed`)
        drift++
      } else {
        fs.unlinkSync(path.join(dir, name))
        console.log('removed', stale)
      }
    }
  }
}

if (drift > 0) process.exit(1)
if (check) console.log('embedded copies match their sources')
