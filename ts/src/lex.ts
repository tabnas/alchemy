/* Copyright (c) 2026 tabnas, MIT License */

// The layout lex matcher: indentation as tokens, and the word tokens of
// the language. A port of rs/src/lex.rs; the module docs there are the
// full account, and this file follows them decision for decision.
//
// The engine's lexer emits whole tokens and drops the IGNORE set (spaces,
// line ends, comments) between rules, so a rule never sees where a line
// begins. Layout needs exactly that, so this matcher runs ahead of the
// engine's built-in bands (`options.lex.match` order 1e5, below the first
// band at 1e6) and, at each line end outside explicit delimiters, reads
// the indentation of the next non-blank, non-comment line and turns it
// into a structural token:
//
// - `#IN` -- the line is exactly two spaces deeper than the enclosing
//   level: a block of children opens;
// - `#NL` -- the line is at the same level: a new logical line;
// - `#DE` -- the line is shallower: one token per level closed.
//
// One matcher call returns one token, and the engine calls a matcher again
// at the same position only when the cursor stayed put, so a dedent of
// several levels is issued one token per call: the first `#DE` consumes
// the line end and the indentation, the count still owed is kept in the
// parse's context bag (`ctx.u`), and the following calls, made at the
// first content character of the line, return one `#DE` each with an
// empty source until the count is zero.
//
// Levels still open at the end of the source are not closed here: the
// lexer answers `#ZZ` at the end without consulting any matcher, so the
// grammar closes every open block on `#ZZ` as well as on `#DE`.
//
// A line ends at a line feed, alone or after a carriage return. A lone
// carriage return anywhere else is whitespace with no layout meaning, as
// the engine counts rows by line feeds alone; in a line's leading
// whitespace it restarts the indentation, as it restarts the engine's
// column. A layout token ends at the first lone `\r` on its form's row,
// and an error's column counts from the last.
//
// Layout is suspended while the explicit-delimiter depth (`(` and `[`
// opened minus `)` and `]` closed, strings and comments excluded) is above
// zero. Nesting is bounded here, before anything is built: at most
// MAX_NESTING levels, counting a layout line, each indentation level and
// each open delimiter as one; the opener or the indented line that would
// pass the bound is `too_deep`.
//
// The same matcher owns the word tokens: a maximal run of the symbol
// alphabet is a JSON number (`#NR`, the lexeme kept as the token source),
// `true`, `false` or `null` (`#VL`), or a symbol (`#TX`); `:name` is a
// keyword (`#KW`, the name as the value).
//
// Offsets here are JavaScript string indexes (UTF-16 code units) where
// the Rust crate's are bytes; the two agree on every ASCII source, and
// everything this matcher decides on (layout characters, delimiters, the
// symbol alphabet) is ASCII.

import type { Lex, Rule, Token, Config, TabnasOptions } from '@tabnas/parser'

import { MAX_NESTING, codePoints } from './ast'

// The reference name the grammar document's `options.lex.match` uses.
export const MATCHER = '@alchemy-layout'

// The structural tokens, registered by name before the grammar names them.
export const IN = '#IN'
export const DE = '#DE'
export const NL = '#NL'
// A keyword token; its value is the name without the colon.
export const KW = '#KW'

// The one key this matcher owns in the context bag, prefixed so a grammar
// layered on this one cannot collide with it by accident.
const K_STATE = 'alchemyLayout'

// Whether `c` (one character) belongs to the symbol alphabet
// `[A-Za-z0-9_\-?!*+/<>=.$%&|^~@]`.
export function isSymbolChar(c: string): boolean {
  const n = c.charCodeAt(0)
  if ((48 <= n && n <= 57) || (65 <= n && n <= 90) || (97 <= n && n <= 122)) return true
  return 1 === c.length && -1 !== '_-?!*+/<>=.$%&|^~@'.indexOf(c)
}

// Whether `text` is exactly a JSON number:
// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
export function isJsonNumber(text: string): boolean {
  return /^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?$/.test(text)
}

// The layout state of one parse, kept between calls in `ctx.u`, because
// the instance is shared between parses and nothing written during a
// parse may live on it. The initial state, before any token, is every
// field at zero.
export type LayoutState = {
  // Whether any content token has been lexed yet.
  seen: boolean
  // The open indentation levels below the top one: the open levels are
  // always 0, 2, ..., 2 * levels.
  levels: number
  // `#DE` tokens still owed for the dedent in progress.
  pending: number
  // The delimiter-depth scan: the depth at `depthPos`, and whether that
  // position is inside a string or a comment.
  depth: number
  depthPos: number
  inString: boolean
  inComment: boolean
  // Before the first form: the index up to which the source has been
  // scanned and found to hold trivia only, or the first form at
  // indentation zero.
  checked: number
}

export function initialState(): LayoutState {
  return {
    seen: false,
    levels: 0,
    pending: 0,
    depth: 0,
    depthPos: 0,
    inString: false,
    inComment: false,
    checked: 0,
  }
}

// The indentation, in spaces, of the innermost open level.
function top(state: LayoutState): number {
  return 2 * state.levels
}

// The explicit-delimiter depth just before index `target` of `src`,
// ignoring delimiters inside a string (JSON escapes honoured) or a `;`
// comment. The scan continues from where it last stopped; asking about an
// earlier position restarts it from the beginning.
export function depthAt(state: LayoutState, src: string, target: number): number {
  target = Math.min(target, src.length)
  if (target < state.depthPos) {
    state.depth = 0
    state.depthPos = 0
    state.inString = false
    state.inComment = false
  }
  let i = state.depthPos
  while (i < target) {
    const c = src[i]
    if (state.inString) {
      if ('\\' === c) i++
      else if ('"' === c || '\n' === c || '\r' === c) state.inString = false
    } else if (state.inComment) {
      // The engine ends a line comment at either character.
      if ('\n' === c || '\r' === c) state.inComment = false
    } else if ('(' === c || '[' === c) {
      state.depth++
    } else if (')' === c || ']' === c) {
      state.depth = Math.max(0, state.depth - 1)
    } else if ('"' === c) {
      state.inString = true
    } else if (';' === c) {
      state.inComment = true
    }
    i++
  }
  state.depthPos = target
  return state.depth
}

// What one matcher call decided.
export type Decision =
  // Not this matcher's: leave the position to the engine's bands.
  | { kind: 'pass' }
  // A word or keyword token over `len` characters of source.
  | { kind: 'word'; name: string; value: unknown; len: number }
  // A layout token whose source is the `len` characters of line end and
  // indentation it consumed; `pending` further `#DE` are owed.
  | { kind: 'layout'; name: string; len: number; pending: number }
  // An error with this matcher's code, reported at index `at`. When a
  // lone `\r` precedes `at` on its row, `rowCr` is the last one, and the
  // column counts from the character after it.
  | { kind: 'bad'; code: string; at: number; rowCr: number | undefined }

const PASS: Decision = { kind: 'pass' }

function bad(code: string, at: number, rowCr?: number): Decision {
  return { kind: 'bad', code, at, rowCr }
}

// How many times `lineStart` scanned ahead, for the test that holds the
// leading trivia to one scan.
export const scans = { count: 0 }

// Decide what `src` holds at index `si`, updating `state`.
export function decide(src: string, si: number, state: LayoutState): Decision {
  if (state.pending > 0) {
    state.pending--
    return { kind: 'layout', name: DE, len: 0, pending: state.pending }
  }
  if (si >= src.length) return PASS
  const first = src[si]

  // The start of the source is a line start with no line end before it.
  if (0 === si && !state.seen && (' ' === first || '\t' === first)) {
    return lineStart(src, 0, false, state)
  }
  // Before the first form, a line end or a lone `\r` within the stretch a
  // scan has covered is trivia the engine eats.
  if (!state.seen && si < state.checked && ('\n' === first || '\r' === first)) {
    return PASS
  }

  if ('\n' === first) return lineEnd(src, si, state)
  if ('\r' === first) {
    if ('\n' === src[si + 1]) return lineEnd(src, si, state)
    // A carriage return with no line feed after it is whitespace. Before
    // the first line it restarts the indentation the first line must not
    // have, so it is read as a line start there.
    if (!state.seen) return lineStart(src, si, false, state)
    return PASS
  }
  if (' ' === first || '\t' === first || ';' === first) return PASS
  if ('(' === first || '[' === first) {
    state.seen = true
    // The depth just after this opener, with the layout line and the
    // indentation levels around it, is the nesting it opens.
    const nesting = 1 + state.levels + depthAt(state, src, si + 1)
    return nesting > MAX_NESTING ? bad('too_deep', si) : PASS
  }
  if (')' === first || ']' === first) {
    state.seen = true
    return 0 === depthAt(state, src, si) ? bad('unbalanced', si) : PASS
  }
  if (':' === first) {
    state.seen = true
    const nameLen = symbolRun(src, si + 1)
    // A bare colon is nothing in this language; the engine reports it as
    // `unexpected` when no band claims it.
    if (0 === nameLen) return PASS
    return { kind: 'word', name: KW, value: src.substring(si + 1, si + 1 + nameLen), len: 1 + nameLen }
  }
  if (isSymbolChar(first)) {
    state.seen = true
    const len = symbolRun(src, si)
    const word = src.substring(si, si + len)
    if (isJsonNumber(word)) {
      // The AST keeps the lexeme rather than this value.
      return { kind: 'word', name: '#NR', value: Number(word), len }
    }
    if ('true' === word || 'false' === word) {
      return { kind: 'word', name: '#VL', value: 'true' === word, len }
    }
    if ('null' === word) return { kind: 'word', name: '#VL', value: null, len }
    return { kind: 'word', name: '#TX', value: word, len }
  }
  // A quote, or something no band will claim: the engine's matchers
  // decide, but a line has begun either way.
  state.seen = true
  return PASS
}

// A line end at `si`: a layout decision outside delimiters, whitespace
// inside them.
function lineEnd(src: string, si: number, state: LayoutState): Decision {
  return depthAt(state, src, si) > 0 ? PASS : lineStart(src, si, true, state)
}

// The length of the symbol-alphabet run at `from`.
function symbolRun(src: string, from: number): number {
  let i = from
  while (i < src.length && isSymbolChar(src[i])) i++
  return i - from
}

// The index after the line terminator at `pos` (`\n`, or `\r\n`), or
// `pos` when there is none.
function afterTerminator(src: string, pos: number): number {
  if ('\n' === src[pos]) return pos + 1
  if ('\r' === src[pos] && '\n' === src[pos + 1]) return pos + 2
  return pos
}

// From a line start at `from` (just before its terminator when
// `afterNewline`), skip blank and comment-only lines to the next content
// line and decide the layout token its indentation calls for.
function lineStart(src: string, from: number, afterNewline: boolean, state: LayoutState): Decision {
  scans.count++
  let pos = afterNewline ? afterTerminator(src, from) : from
  for (;;) {
    let spaces = 0
    let tabAt: number | undefined = undefined
    let firstCr: number | undefined = undefined
    let lastCr: number | undefined = undefined
    for (;;) {
      while (pos < src.length) {
        const c = src[pos]
        if (' ' === c) spaces++
        else if ('\t' === c) {
          if (undefined === tabAt) tabAt = pos
        } else break
        pos++
      }
      // A comment runs to the line end and counts for nothing.
      if (';' === src[pos]) {
        while (pos < src.length && '\n' !== src[pos] && '\r' !== src[pos]) pos++
      }
      // A lone carriage return restarts the indentation.
      if ('\r' === src[pos] && '\n' !== src[pos + 1]) {
        if (undefined === firstCr) firstCr = pos
        lastCr = pos
        pos++
        spaces = 0
        tabAt = undefined
        continue
      }
      break
    }
    if (pos >= src.length) {
      // Only trivia to the end: the engine's own matchers eat it, and
      // `#ZZ` closes what is open.
      state.checked = Math.max(state.checked, src.length)
      return PASS
    }
    if ('\n' === src[pos]) {
      pos += 1
    } else if ('\r' === src[pos] && '\n' === src[pos + 1]) {
      pos += 2
    } else {
      if (undefined !== tabAt) return bad('tab_indent', tabAt, lastCr)
      return layoutToken(spaces, pos, from, firstCr, lastCr, state)
    }
  }
}

// The layout token for a content line indented by `indent` spaces whose
// first character is at `content`, when the matcher was called at `from`.
function layoutToken(
  indent: number,
  content: number,
  from: number,
  firstCr: number | undefined,
  lastCr: number | undefined,
  state: LayoutState,
): Decision {
  const rowCr = lastCr
  if (!state.seen) {
    if (0 === indent) {
      // The first form, found: nothing before it needs a scan again.
      state.checked = Math.max(state.checked, content)
      return PASS
    }
    return bad('bad_indent', content, rowCr)
  }
  const len = (undefined === firstCr ? content : firstCr) - from
  const t = top(state)
  if (indent === t) return { kind: 'layout', name: NL, len, pending: 0 }
  if (indent === t + 2) {
    // The new level, with the layout line it opens, is the nesting.
    if (1 + state.levels + 1 > MAX_NESTING) return bad('too_deep', content, rowCr)
    state.levels++
    return { kind: 'layout', name: IN, len, pending: 0 }
  }
  if (indent > t) return bad('bad_indent', content, rowCr)
  // A dedent returns to an open level: an even indentation below the top.
  if (0 !== indent % 2) return bad('bad_dedent', content, rowCr)
  const pops = (t - indent) / 2
  state.levels -= pops
  // The first `#DE` goes out now with the consumed trivia; the rest are
  // owed and issued on the following calls, which stand at `content`.
  state.pending = pops - 1
  return { kind: 'layout', name: DE, len, pending: state.pending }
}

// The state of the parse `lex` belongs to, created on its first call.
export function stateOf(lex: Lex): LayoutState {
  const u: Record<string, any> = (lex.ctx as any).u
  let state: LayoutState | undefined = u[K_STATE]
  if (undefined === state) {
    state = u[K_STATE] = initialState()
  }
  return state
}

// Move the cursor over `len` characters: a line feed starts a row, and
// everything else, a lone `\r` included, moves the column on. (The
// engine's own line matcher restarts the column at a lone `\r` too, which
// is why a layout token ends at the first one on its row.)
function advance(lex: Lex, len: number) {
  const pnt = lex.pnt
  const src = lex.src
  const end = pnt.sI + len
  for (let i = pnt.sI; i < end; i++) {
    if ('\n' === src[i]) {
      pnt.rI++
      pnt.cI = 1
    } else {
      pnt.cI++
    }
  }
  pnt.sI = end
}

// The matcher itself: decide against the source, then build the token at
// the point before the move and move the cursor.
function layoutMatcher(lex: Lex, _rule: Rule): Token | undefined {
  const pnt = lex.pnt
  const si = pnt.sI
  const src = lex.src
  const decision = decide(src, si, stateOf(lex))
  switch (decision.kind) {
    case 'pass':
      return undefined
    case 'word': {
      const text = src.substring(si, si + decision.len)
      const tkn = lex.token(decision.name, decision.value, text, pnt)
      advance(lex, decision.len)
      return tkn
    }
    case 'layout': {
      const text = src.substring(si, si + decision.len)
      const tkn = lex.token(decision.name, undefined, text, pnt)
      advance(lex, decision.len)
      return tkn
    }
    case 'bad': {
      advance(lex, decision.at - si)
      // The cursor's column counts the lone `\r` it moved over as a
      // character; the engine's line matcher restarts it there.
      if (undefined !== decision.rowCr) {
        pnt.cI = codePoints(src, decision.rowCr + 1, decision.at) + 1
      }
      return lex.bad(decision.code, -1, -1)
    }
  }
}

// The matcher's factory, as `options.lex.match` takes it: the document
// names it `@alchemy-layout`.
export function makeLayoutMatcher(_cfg: Config, _opts: TabnasOptions) {
  return layoutMatcher
}
