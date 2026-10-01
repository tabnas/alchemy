/* Copyright (c) 2026 tabnas, MIT License */

// The syntax tree: `Expr` with source spans, built from the reader's
// tagged value tree, and the two printers, canonical and layout. A port of
// rs/src/ast.rs.
//
// This module is the input of the checker, planner and interpreter that
// follow: they read `Expr` and never the tagged tree, so the grammar's
// output shape is the reader's business alone.
//
// A span is a pair of offsets into the source plus the file it came from;
// the 1-based row and column a diagnostic prints are derived from the
// source when needed rather than stored. Offsets are JavaScript string
// indexes (UTF-16 code units; the Rust crate's are bytes), and the column
// a position names counts characters, as the Rust crate's does.
//
// Every `Expr` this package builds nests at most MAX_NESTING levels: the
// reader refuses a deeper program before building anything, `fromValue`
// refuses a deeper tagged tree without recursing over it, and the
// desugarer refuses a rewrite that would nest deeper. That bound is what
// lets the printers and the desugarer recurse per level: 256 levels is a
// few hundred frames, far inside Node's default stack.

import { Fail } from '@tabnas/transduce'

// The most levels a form may nest: a list or vector inside a list or
// vector, this many times over. The reader counts a layout line, each
// indentation level and each open `(` or `[` as one and refuses the opener
// or the line that would pass the bound with `too_deep`; the desugarer
// counts what its rewrites add (a `pipe` nests one level per step).
export const MAX_NESTING = 256

// The message of a `too_deep` failure, one text for the grammar document,
// this module and the desugarer; a test holds it to MAX_NESTING.
export const TOO_DEEP = 'nesting deeper than 256 levels'

// The number of characters (code points) in `src` from `from` to `to`.
export function codePoints(src: string, from: number, to: number): number {
  let n = 0
  for (let i = from; i < to; i++) {
    const c = src.charCodeAt(i)
    // A high surrogate followed by a low one is one character.
    if (0xd800 <= c && c <= 0xdbff && i + 1 < to) {
      const d = src.charCodeAt(i + 1)
      if (0xdc00 <= d && d <= 0xdfff) i++
    }
    n++
  }
  return n
}

// The file a span names. One object per parse, shared by every span the
// parse made, so a span's file can be told apart by identity as well as by
// name: the standard library's spans are its own, whatever a program
// happens to be called (Rust: the shared `Arc<str>`).
export type SourceFile = { readonly name: string }

export function sourceFile(name: string): SourceFile {
  return Object.freeze({ name })
}

// Where a form came from: `file`, and the range `start..end` of its
// source text.
export type SourceSpan = {
  readonly file: SourceFile
  readonly start: number
  readonly end: number
}

export function span(file: SourceFile, start: number, end: number): SourceSpan {
  return { file, start, end }
}

// The 1-based row and column of `span.start` in `src`, the column counted
// in characters, as the engine counts them: a row ends at a line feed,
// and a column restarts at a line feed or at a carriage return, whether or
// not a line feed follows it. A start past the end of `src` answers the
// position just after its last character.
export function position(sp: SourceSpan, src: string): [number, number] {
  const start = Math.min(sp.start, src.length)
  let row = 1
  let lineStart = 0
  for (let i = 0; i < start; i++) {
    const c = src[i]
    if ('\n' === c) {
      row++
      lineStart = i + 1
    } else if ('\r' === c) {
      lineStart = i + 1
    }
  }
  return [row, codePoints(src, lineStart, start) + 1]
}

// `file:start..end`, as the Rust `Display` prints a span.
export function spanText(sp: SourceSpan): string {
  return `${sp.file.name}:${sp.start}..${sp.end}`
}

// The texts a program is compiled from, each under the file name its
// spans carry: one for `compile`, several for `compileSources`. A
// diagnostic positions a span in the text of the span's own file, and
// names the file when there are several.
export class Sources {
  readonly files: ReadonlyArray<readonly [string, string]>

  private constructor(files: ReadonlyArray<readonly [string, string]>) {
    this.files = files
  }

  // One text, named `file`.
  static one(file: string, text: string): Sources {
    return new Sources([[file, text]])
  }

  // Several texts, each under its name, in order; the caller holds the
  // names distinct, since a span names its file by name.
  static several(files: ReadonlyArray<readonly [string, string]>): Sources {
    if (0 === files.length) throw new Error('a program has at least one source')
    return new Sources(files.slice())
  }

  // The first file's name: the program's, when there is one.
  first(): string {
    return this.files[0][0]
  }

  // Whether a diagnostic names its file: when the sources are several.
  namesFiles(): boolean {
    return this.files.length > 1
  }

  // The text of `sp`'s file, when it is one of these.
  find(sp: SourceSpan): string | undefined {
    const found = this.files.find(([file]) => file === sp.file.name)
    return found?.[1]
  }

  // The text a span is positioned in: its file's, or the first when its
  // file is not among these (a span of the standard library, or of an
  // unnamed parse).
  textOf(sp: SourceSpan): string {
    return this.find(sp) ?? this.files[0][1]
  }

  // The 1-based row and column of `sp` in its file's text.
  position(sp: SourceSpan): [number, number] {
    return position(sp, this.textOf(sp))
  }

  // `fail` at `sp`: its row and column, and its file when the sources are
  // several and the span is in one of them.
  failAt(fail: Fail, sp: SourceSpan): Fail {
    const [row, col] = this.position(sp)
    fail.at(row, col)
    if (this.namesFiles() && undefined !== this.find(sp)) {
      fail.inFile(sp.file.name)
    }
    return fail
  }
}

// One form of a program. A number keeps its lexeme rather than a value:
// the language's numbers are JSON numbers, and what a renderer prints is
// the text the author wrote.
export type Expr =
  | { readonly kind: 'symbol'; readonly name: string; readonly span: SourceSpan }
  | { readonly kind: 'keyword'; readonly name: string; readonly span: SourceSpan }
  | { readonly kind: 'str'; readonly value: string; readonly span: SourceSpan }
  | { readonly kind: 'num'; readonly lexeme: string; readonly span: SourceSpan }
  | { readonly kind: 'bool'; readonly value: boolean; readonly span: SourceSpan }
  | { readonly kind: 'null'; readonly span: SourceSpan }
  | { readonly kind: 'list'; readonly items: Expr[]; readonly span: SourceSpan }
  | { readonly kind: 'vector'; readonly items: Expr[]; readonly span: SourceSpan }

export type ListExpr = Extract<Expr, { kind: 'list' }>
export type VectorExpr = Extract<Expr, { kind: 'vector' }>

export function sym(name: string, sp: SourceSpan): Expr {
  return { kind: 'symbol', name, span: sp }
}

export function list(items: Expr[], sp: SourceSpan): Expr {
  return { kind: 'list', items, span: sp }
}

export function vector(items: Expr[], sp: SourceSpan): Expr {
  return { kind: 'vector', items, span: sp }
}

// The symbol's name, when this form is a symbol.
export function symbolOf(expr: Expr | undefined): string | undefined {
  return undefined !== expr && 'symbol' === expr.kind ? expr.name : undefined
}

// Whether this form is an atom: anything but a list or a vector.
export function isAtom(expr: Expr): boolean {
  return 'list' !== expr.kind && 'vector' !== expr.kind
}

function malformed(what: string): Fail {
  return new Fail('DSL_PARSE_ERROR', `malformed reader output: ${what}`)
}

function show(value: unknown): string {
  try {
    return JSON.stringify(value) ?? String(value)
  } catch (_e) {
    return String(value)
  }
}

function field(fields: Record<string, unknown>, name: string): unknown {
  if (!Object.prototype.hasOwnProperty.call(fields, name)) {
    throw malformed(`a node without ${JSON.stringify(name)}`)
  }
  return fields[name]
}

function text(fields: Record<string, unknown>, name: string): string {
  const value = field(fields, name)
  if ('string' !== typeof value) {
    throw malformed(`${JSON.stringify(name)} is not a string: ${show(value)}`)
  }
  return value
}

function offset(value: unknown): number {
  if ('number' === typeof value && Number.isFinite(value) && value >= 0 && Number.isInteger(value)) {
    return value
  }
  throw malformed(`a span offset that is not a whole number: ${show(value)}`)
}

// One node of the tagged tree, read before its items are: an atom is a
// finished form, a container still has its items to build.
type Node =
  | { atom: Expr }
  | { list: boolean; span: SourceSpan; items: unknown[] }

function readNode(value: unknown, file: SourceFile): Node {
  if (null == value || 'object' !== typeof value || Array.isArray(value)) {
    throw malformed(`a node that is not an object: ${show(value)}`)
  }
  const fields = value as Record<string, unknown>
  const pair = field(fields, 'span')
  if (!Array.isArray(pair) || 2 !== pair.length) {
    throw malformed(`a span that is not a pair: ${show(pair)}`)
  }
  const sp = span(file, offset(pair[0]), offset(pair[1]))
  const tag = text(fields, '$')
  switch (tag) {
    case 'sym':
      return { atom: { kind: 'symbol', name: text(fields, 'name'), span: sp } }
    case 'kw':
      return { atom: { kind: 'keyword', name: text(fields, 'name'), span: sp } }
    case 'str':
      return { atom: { kind: 'str', value: text(fields, 'value'), span: sp } }
    case 'num':
      return { atom: { kind: 'num', lexeme: text(fields, 'lexeme'), span: sp } }
    case 'bool': {
      const value = field(fields, 'value')
      if ('boolean' !== typeof value) throw malformed(`a bool that is not a bool: ${show(value)}`)
      return { atom: { kind: 'bool', value, span: sp } }
    }
    case 'null':
      return { atom: { kind: 'null', span: sp } }
    case 'list':
    case 'vector': {
      const items = field(fields, 'items')
      if (!Array.isArray(items)) throw malformed(`items that are not an array: ${show(items)}`)
      return { list: 'list' === tag, span: sp, items }
    }
    default:
      throw malformed(`an unknown tag ${JSON.stringify(tag)}`)
  }
}

function tooDeep(sp: SourceSpan): Fail {
  return new Fail('DSL_PARSE_ERROR', `too_deep: ${TOO_DEEP} (at ${spanText(sp)})`)
}

// A container whose items are being built, on `fromValue`'s own stack
// rather than the call stack.
type Frame = { list: boolean; span: SourceSpan; items: unknown[]; next: number; built: Expr[] }

function finish(frame: Frame): Expr {
  return frame.list ? list(frame.built, frame.span) : vector(frame.built, frame.span)
}

// Build a form from one node of the reader's tagged tree. A tree the
// reader did not build may be malformed; that is a DSL_PARSE_ERROR thrown
// as a `Fail`, never anything else.
//
// Iterative, over an explicit stack of open containers: a grammar layered
// on the reader could hand over a deeper tree than the reader builds, and
// the conversion refuses it as `too_deep` (past MAX_NESTING open
// containers) rather than recursing into it.
export function fromValue(value: unknown, file: SourceFile): Expr {
  const stack: Frame[] = []
  let pending: unknown = value
  for (;;) {
    let built: Expr
    const node = readNode(pending, file)
    if ('atom' in node) {
      built = node.atom
    } else {
      if (stack.length >= MAX_NESTING) throw tooDeep(node.span)
      const frame: Frame = { list: node.list, span: node.span, items: node.items, next: 0, built: [] }
      if (frame.next < frame.items.length) {
        pending = frame.items[frame.next++]
        stack.push(frame)
        continue
      }
      built = finish(frame)
    }
    // A finished form belongs to the innermost open container, whose next
    // item is built next; a container out of items is finished in turn, up
    // to the root.
    for (;;) {
      const frame = stack.pop()
      if (undefined === frame) return built
      frame.built.push(built)
      if (frame.next < frame.items.length) {
        pending = frame.items[frame.next++]
        stack.push(frame)
        break
      }
      built = finish(frame)
    }
  }
}

// Build a whole program from the reader's array of top-level nodes.
export function programFromValue(value: unknown, file: SourceFile): Expr[] {
  if (!Array.isArray(value)) {
    throw malformed(`a program that is not an array: ${show(value)}`)
  }
  return value.map((form) => fromValue(form, file))
}

// Structural equality that ignores spans: the same forms with the same
// names, values and lexemes, wherever they came from.
export function sameShape(a: Expr, b: Expr): boolean {
  switch (a.kind) {
    case 'symbol':
    case 'keyword':
      return a.kind === b.kind && a.name === (b as typeof a).name
    case 'str':
      return 'str' === b.kind && a.value === b.value
    case 'num':
      return 'num' === b.kind && a.lexeme === b.lexeme
    case 'bool':
      return 'bool' === b.kind && a.value === b.value
    case 'null':
      return 'null' === b.kind
    case 'list':
    case 'vector':
      return a.kind === b.kind && sameProgram(a.items, (b as typeof a).items)
  }
}

// `sameShape` over two programs, form by form.
export function sameProgram(a: Expr[], b: Expr[]): boolean {
  return a.length === b.length && a.every((x, i) => sameShape(x, b[i]))
}

// ---------------------------------------------------------------------------
// Canonical form
// ---------------------------------------------------------------------------

// One form, fully parenthesized on one line: strings as JSON literals,
// numbers by lexeme, vectors in brackets, keywords with their colon.
// Recursive per level, which MAX_NESTING bounds for every form this
// package builds.
export function canonicalForm(expr: Expr): string {
  const out: string[] = []
  writeCanonical(expr, out)
  return out.join('')
}

function writeCanonical(expr: Expr, out: string[]) {
  switch (expr.kind) {
    case 'symbol':
      out.push(expr.name)
      break
    case 'keyword':
      out.push(':', expr.name)
      break
    case 'str':
      out.push(jsonString(expr.value))
      break
    case 'num':
      out.push(expr.lexeme)
      break
    case 'bool':
      out.push(expr.value ? 'true' : 'false')
      break
    case 'null':
      out.push('null')
      break
    case 'list':
      writeItems(expr.items, '(', ')', out)
      break
    case 'vector':
      writeItems(expr.items, '[', ']', out)
      break
  }
}

function writeItems(items: Expr[], open: string, close: string, out: string[]) {
  out.push(open)
  items.forEach((item, index) => {
    if (index > 0) out.push(' ')
    writeCanonical(item, out)
  })
  out.push(close)
}

// The JSON literal for `value`, as serde_json writes it: `"` and `\`
// escaped, the controls as `\b \f \n \r \t` or `\u00XX` (lower-case hex),
// everything else (DEL, the C1 controls, every other character, a lone
// surrogate included) as itself. `JSON.stringify` would escape a lone
// surrogate where serde_json cannot hold one, so the escaping is spelled
// out.
export function jsonString(value: string): string {
  let out = '"'
  for (let i = 0; i < value.length; i++) {
    const c = value[i]
    const n = value.charCodeAt(i)
    if ('"' === c) out += '\\"'
    else if ('\\' === c) out += '\\\\'
    else if (n < 0x20) {
      switch (c) {
        case '\b':
          out += '\\b'
          break
        case '\f':
          out += '\\f'
          break
        case '\n':
          out += '\\n'
          break
        case '\r':
          out += '\\r'
          break
        case '\t':
          out += '\\t'
          break
        default:
          out += '\\u' + n.toString(16).padStart(4, '0')
      }
    } else out += c
  }
  return out + '"'
}

// A whole program in canonical form: one top-level form per line.
export function canonical(program: Expr[]): string {
  return program.map(canonicalForm).join('\n')
}

// ---------------------------------------------------------------------------
// Layout form
// ---------------------------------------------------------------------------

// A whole program in layout form, the shape the reader reads back to the
// same program.
//
// The rules: an atom or a vector prints on one line in canonical form. A
// list with fewer than two items, or whose first item is a list, prints
// canonically with explicit parens, because a layout line cannot express
// it. Any other list puts its leading atoms and vectors on one line and
// each remaining item on its own line two spaces deeper. Top-level forms
// are separated by a blank line.
export function format(program: Expr[]): string {
  const out: string[] = []
  program.forEach((form, index) => {
    if (index > 0) out.push('\n')
    writeLayout(form, 0, out)
  })
  return out.join('')
}

// Whether `expr` can sit on a layout line beside other forms without
// changing what the line means.
function isInline(expr: Expr): boolean {
  return 'list' !== expr.kind
}

function writeLayout(expr: Expr, depth: number, out: string[]) {
  const indent = '  '.repeat(depth)
  if ('list' !== expr.kind) {
    out.push(indent, canonicalForm(expr), '\n')
    return
  }
  const items = expr.items
  let head = 0
  while (head < items.length && isInline(items[head])) head++
  if (items.length < 2 || 0 === head) {
    out.push(indent, canonicalForm(expr), '\n')
    return
  }
  out.push(indent, items.slice(0, head).map(canonicalForm).join(' '), '\n')
  for (const child of items.slice(head)) {
    writeLayout(child, depth + 1, out)
  }
}
