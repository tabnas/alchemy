// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"math"
	"strconv"
	"unicode/utf8"

	tabnas "github.com/tabnas/parser/go"
)

// lex.go: the layout lex matcher, indentation as tokens, and the word
// tokens of the language (rs/src/lex.rs, whose module documentation is
// the full account).
//
// The engine's lexer emits whole tokens and drops the IGNORE set between
// rules, so a rule never sees where a line begins. This matcher runs ahead
// of the engine's built-in bands (order 1e5) and, at each line end outside
// explicit delimiters, reads the indentation of the next non-blank,
// non-comment line and turns it into `#IN` (two spaces deeper), `#NL` (the
// same level) or `#DE` (shallower, one token per level closed, the rest
// owed and issued one per call with an empty source).
//
// A line ends at a line feed, alone or after a carriage return; a lone
// carriage return is whitespace that restarts the indentation, as it
// restarts the engine's column. Layout is suspended while the explicit
// delimiter depth is above zero. Nesting is bounded here, before anything
// is built: at most MaxNesting levels, counting a layout line, each
// indentation level and each open delimiter as one (`too_deep`).
//
// The same matcher owns the word tokens: a maximal run of the symbol
// alphabet is a JSON number (`#NR`), `true`, `false` or `null` (`#VL`), or
// a symbol (`#TX`); `:name` is a keyword (`#KW`).

// layoutMatcherRef is the reference name the grammar document's
// options.lex.match uses.
const layoutMatcherRef = "@alchemy-layout"

// The structural tokens, registered by name before the grammar names them.
const (
	tokenIN = "#IN"
	tokenDE = "#DE"
	tokenNL = "#NL"
	tokenKW = "#KW"
)

// layoutStateKey is the one key this matcher owns in the context bag.
const layoutStateKey = "alchemyLayout"

// IsSymbolChar is whether c belongs to the symbol alphabet
// `[A-Za-z0-9_\-?!*+/<>=.$%&|^~@]`.
func IsSymbolChar(c rune) bool {
	switch {
	case c >= 'a' && c <= 'z', c >= 'A' && c <= 'Z', c >= '0' && c <= '9':
		return true
	}
	switch c {
	case '_', '-', '?', '!', '*', '+', '/', '<', '>', '=', '.', '$', '%', '&', '|', '^', '~', '@':
		return true
	}
	return false
}

// IsJSONNumber is whether text is exactly a JSON number:
// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
func IsJSONNumber(text string) bool {
	i, n := 0, len(text)
	digit := func(i int) bool { return i < n && text[i] >= '0' && text[i] <= '9' }
	if i < n && text[i] == '-' {
		i++
	}
	switch {
	case i < n && text[i] == '0':
		i++
	case digit(i):
		for digit(i) {
			i++
		}
	default:
		return false
	}
	if i < n && text[i] == '.' {
		i++
		start := i
		for digit(i) {
			i++
		}
		if i == start {
			return false
		}
	}
	if i < n && (text[i] == 'e' || text[i] == 'E') {
		i++
		if i < n && (text[i] == '+' || text[i] == '-') {
			i++
		}
		start := i
		for digit(i) {
			i++
		}
		if i == start {
			return false
		}
	}
	return i == n
}

// layoutState is the layout state of one parse, kept in the context bag
// between calls (Context.U, since the instance is shared between parses).
// The initial state, before any token, is every field at zero.
type layoutState struct {
	// seen: whether any content token has been lexed yet.
	seen bool
	// levels: the open indentation levels below the top one; the open
	// levels are always 0, 2, ..., 2*levels.
	levels int
	// pending: `#DE` tokens still owed for the dedent in progress.
	pending int
	// The delimiter-depth scan: the depth at depthPos, and whether that
	// position is inside a string or a comment.
	depth     int
	depthPos  int
	inString  bool
	inComment bool
	// checked: before the first form, the byte up to which the source has
	// been scanned and found to hold trivia only.
	checked int
	// scans counts lineStart's scans ahead, for the test that holds the
	// leading trivia to one scan.
	scans int
}

func (s *layoutState) top() int { return 2 * s.levels }

// depthAt is the explicit-delimiter depth just before byte target of src:
// `(` and `[` opened minus `)` and `]` closed, ignoring delimiters inside a
// string (JSON escapes honoured) or a `;` comment. The scan continues from
// where it last stopped; asking about an earlier position restarts it.
func (s *layoutState) depthAt(src string, target int) int {
	if target > len(src) {
		target = len(src)
	}
	if target < s.depthPos {
		s.depth, s.depthPos, s.inString, s.inComment = 0, 0, false, false
	}
	for i := s.depthPos; i < target; i++ {
		c := src[i]
		switch {
		case s.inString:
			switch c {
			case '\\':
				i++
			case '"', '\n', '\r':
				s.inString = false
			}
		case s.inComment:
			// The engine ends a line comment at either character.
			if c == '\n' || c == '\r' {
				s.inComment = false
			}
		default:
			switch c {
			case '(', '[':
				s.depth++
			case ')', ']':
				if s.depth > 0 {
					s.depth--
				}
			case '"':
				s.inString = true
			case ';':
				s.inComment = true
			}
		}
	}
	s.depthPos = target
	return s.depth
}

// decisionKind is what one matcher call decided.
type decisionKind uint8

const (
	// Not this matcher's: leave the position to the engine's bands.
	decidePass decisionKind = iota
	// A word or keyword token over n bytes of source.
	decideWord
	// A layout token whose source is the n bytes of line end and
	// indentation it consumed.
	decideLayout
	// An error with this matcher's code, reported at byte at; rowCR is
	// the last lone `\r` before at on its row, or -1.
	decideBad
)

type decision struct {
	kind  decisionKind
	name  string
	tin   tabnas.Tin
	value any
	n     int
	// Layout: further `#DE` owed.
	pending int
	// Bad.
	code  string
	at    int
	rowCR int
}

var pass = decision{kind: decidePass}

func bad(code string, at, rowCR int) decision {
	return decision{kind: decideBad, code: code, at: at, rowCR: rowCR}
}

// decide decides what src holds at byte si, updating state.
func decide(src string, si int, state *layoutState) decision {
	if state.pending > 0 {
		state.pending--
		return decision{kind: decideLayout, name: tokenDE, n: 0, pending: state.pending}
	}
	if si >= len(src) {
		return pass
	}
	first, _ := utf8.DecodeRuneInString(src[si:])

	// The start of the source is a line start with no line end before it.
	if si == 0 && !state.seen && (first == ' ' || first == '\t') {
		return lineStart(src, 0, false, state)
	}
	// Before the first form, a line end or a lone `\r` within the stretch
	// a scan has covered is trivia the engine eats.
	if !state.seen && si < state.checked && (first == '\n' || first == '\r') {
		return pass
	}

	switch {
	case first == '\n':
		return lineEnd(src, si, state)
	case first == '\r' && si+1 < len(src) && src[si+1] == '\n':
		return lineEnd(src, si, state)
	case first == '\r' && !state.seen:
		// A carriage return with no line feed after it is whitespace;
		// before the first line it restarts the indentation the first
		// line must not have, so it is read as a line start there.
		return lineStart(src, si, false, state)
	case first == ' ' || first == '\t' || first == '\r' || first == ';':
		return pass
	case first == '(' || first == '[':
		state.seen = true
		// The depth just after this opener, with the layout line and the
		// indentation levels around it, is the nesting it opens.
		if 1+state.levels+state.depthAt(src, si+1) > MaxNesting {
			return bad("too_deep", si, -1)
		}
		return pass
	case first == ')' || first == ']':
		state.seen = true
		if state.depthAt(src, si) == 0 {
			return bad("unbalanced", si, -1)
		}
		return pass
	case first == ':':
		state.seen = true
		nameLen := symbolRun(src[si+1:])
		if nameLen == 0 {
			// A bare colon is nothing in this language; the engine
			// reports it as `unexpected` when no band claims it.
			return pass
		}
		return decision{kind: decideWord, name: tokenKW, tin: -1, value: src[si+1 : si+1+nameLen], n: 1 + nameLen}
	case IsSymbolChar(first):
		state.seen = true
		n := symbolRun(src[si:])
		word := src[si : si+n]
		if IsJSONNumber(word) {
			// Every JSON number parses; an exponent out of range
			// saturates to an infinity, and the AST keeps the lexeme.
			value, err := strconv.ParseFloat(word, 64)
			if err != nil {
				if ne, ok := err.(*strconv.NumError); !ok || ne.Err != strconv.ErrRange {
					value = math.NaN()
				}
			}
			return decision{kind: decideWord, name: "#NR", tin: tabnas.TinNR, value: value, n: n}
		}
		switch word {
		case "true", "false":
			return decision{kind: decideWord, name: "#VL", tin: tabnas.TinVL, value: word == "true", n: n}
		case "null":
			return decision{kind: decideWord, name: "#VL", tin: tabnas.TinVL, value: nil, n: n}
		}
		return decision{kind: decideWord, name: "#TX", tin: tabnas.TinTX, value: word, n: n}
	default:
		// A quote, or something no band will claim: the engine's
		// matchers decide, but a line has begun either way.
		state.seen = true
		return pass
	}
}

// lineEnd is a line end at si: a layout decision outside delimiters,
// whitespace inside them.
func lineEnd(src string, si int, state *layoutState) decision {
	if state.depthAt(src, si) > 0 {
		return pass
	}
	return lineStart(src, si, true, state)
}

// symbolRun is the byte length of the symbol-alphabet run at the start of
// text (the alphabet is ASCII).
func symbolRun(text string) int {
	for i := 0; i < len(text); i++ {
		if text[i] >= utf8.RuneSelf || !IsSymbolChar(rune(text[i])) {
			return i
		}
	}
	return len(text)
}

// afterTerminator is the byte after the line terminator at pos (`\n`, or
// `\r\n`), or pos when there is none.
func afterTerminator(src string, pos int) int {
	switch {
	case pos < len(src) && src[pos] == '\n':
		return pos + 1
	case pos+1 < len(src) && src[pos] == '\r' && src[pos+1] == '\n':
		return pos + 2
	}
	return pos
}

// lineStart reads, from a line start at from (just before its terminator
// when afterNewline), past blank and comment-only lines to the next
// content line, and decides the layout token its indentation calls for.
//
// The indentation of a line is its run of leading spaces after the last
// lone carriage return before its first other character. The first lone
// `\r` on the content line's own row is where the layout token ends, and
// the last is where an error's column counts from.
func lineStart(src string, from int, afterNewline bool, state *layoutState) decision {
	state.scans++
	pos := from
	if afterNewline {
		pos = afterTerminator(src, from)
	}
	n := len(src)
	for {
		spaces := 0
		tabAt, firstCR, lastCR := -1, -1, -1
		for {
			for pos < n {
				c := src[pos]
				if c == ' ' {
					spaces++
				} else if c == '\t' {
					if tabAt < 0 {
						tabAt = pos
					}
				} else {
					break
				}
				pos++
			}
			// A comment runs to the line end and counts for nothing.
			if pos < n && src[pos] == ';' {
				for pos < n && src[pos] != '\n' && src[pos] != '\r' {
					pos++
				}
			}
			// A lone carriage return restarts the indentation.
			if pos < n && src[pos] == '\r' && !(pos+1 < n && src[pos+1] == '\n') {
				if firstCR < 0 {
					firstCR = pos
				}
				lastCR = pos
				pos++
				spaces = 0
				tabAt = -1
				continue
			}
			break
		}
		switch {
		case pos >= n:
			// Only trivia to the end: the engine's own matchers eat it,
			// and `#ZZ` closes what is open.
			if state.checked < n {
				state.checked = n
			}
			return pass
		case src[pos] == '\n':
			pos++
		case src[pos] == '\r' && pos+1 < n && src[pos+1] == '\n':
			pos += 2
		default:
			if tabAt >= 0 {
				return bad("tab_indent", tabAt, lastCR)
			}
			return layoutToken(spaces, pos, from, firstCR, lastCR, state)
		}
	}
}

// layoutToken is the layout token for a content line indented by indent
// spaces whose first character is at content, when the matcher was called
// at from.
func layoutToken(indent, content, from, firstCR, lastCR int, state *layoutState) decision {
	if !state.seen {
		if indent == 0 {
			// The first form, found: nothing before it needs a scan again.
			if state.checked < content {
				state.checked = content
			}
			return pass
		}
		return bad("bad_indent", content, lastCR)
	}
	end := content
	if firstCR >= 0 {
		end = firstCR
	}
	n := end - from
	top := state.top()
	if indent == top {
		return decision{kind: decideLayout, name: tokenNL, n: n}
	}
	if indent == top+2 {
		// The new level, with the layout line it opens, is the nesting.
		if 1+state.levels+1 > MaxNesting {
			return bad("too_deep", content, lastCR)
		}
		state.levels++
		return decision{kind: decideLayout, name: tokenIN, n: n}
	}
	if indent > top {
		return bad("bad_indent", content, lastCR)
	}
	// A dedent returns to an open level: an even indentation below the
	// top.
	if indent%2 != 0 {
		return bad("bad_dedent", content, lastCR)
	}
	pops := (top - indent) / 2
	state.levels -= pops
	// The first `#DE` goes out now with the consumed trivia; the rest are
	// owed and issued on the following calls, which stand at content.
	state.pending = pops - 1
	return decision{kind: decideLayout, name: tokenDE, n: n, pending: state.pending}
}

// layoutStateOf is the state stored in the parse's context bag, created
// on the first call of a parse. A lexer driven without a context gets a
// state of its own per call, which is the most it can keep.
func layoutStateOf(ctx *tabnas.Context) *layoutState {
	if ctx == nil {
		return &layoutState{}
	}
	if ctx.U == nil {
		ctx.U = map[string]any{}
	}
	if s, ok := ctx.U[layoutStateKey].(*layoutState); ok {
		return s
	}
	s := &layoutState{}
	ctx.U[layoutStateKey] = s
	return s
}

// advanceTo moves the cursor from its byte to byte to, counting rows and
// columns as the engine's own cursor does for a matcher that moves it: a
// line feed starts a row, every other character is a column.
func advanceTo(pnt *tabnas.Point, src string, to int) {
	for i := pnt.SI; i < to; {
		r, size := utf8.DecodeRuneInString(src[i:])
		if r == '\n' {
			pnt.RI++
			pnt.CI = 1
		} else {
			pnt.CI++
		}
		i += size
	}
	pnt.SI = to
}

// layoutTins are the identities of the matcher's own tokens on one
// instance.
type layoutTins struct {
	in, de, nl, kw tabnas.Tin
}

func (t layoutTins) of(name string) tabnas.Tin {
	switch name {
	case tokenIN:
		return t.in
	case tokenDE:
		return t.de
	case tokenNL:
		return t.nl
	}
	return t.kw
}

// registerLayout registers the tokens on j, ahead of the grammar document
// that names them, and answers the matcher factory the document's
// `@alchemy-layout` reference resolves to.
func registerLayout(j *tabnas.Tabnas) tabnas.MakeLexMatcher {
	tins := layoutTins{
		in: j.Token(tokenIN),
		de: j.Token(tokenDE),
		nl: j.Token(tokenNL),
		kw: j.Token(tokenKW),
	}
	return func(_ *tabnas.LexConfig, _ *tabnas.Options) tabnas.LexMatcher {
		return func(lex *tabnas.Lex, _ *tabnas.Rule) *tabnas.Token {
			return layoutMatch(lex, tins)
		}
	}
}

// layoutMatch is the matcher itself: decide against the source, then move
// the cursor and build the token at the point captured before the move.
func layoutMatch(lex *tabnas.Lex, tins layoutTins) *tabnas.Token {
	pnt := lex.Cursor()
	si := pnt.SI
	src := lex.Src
	state := layoutStateOf(lex.Ctx)
	d := decide(src, si, state)
	switch d.kind {
	case decideWord:
		tin := d.tin
		if tin < 0 {
			tin = tins.of(d.name)
		}
		tkn := lex.Token(d.name, tin, d.value, src[si:si+d.n])
		advanceTo(pnt, src, si+d.n)
		return tkn
	case decideLayout:
		tkn := lex.Token(d.name, tins.of(d.name), tabnas.Undefined, src[si:si+d.n])
		advanceTo(pnt, src, si+d.n)
		return tkn
	case decideBad:
		advanceTo(pnt, src, d.at)
		next := ""
		if d.at < len(src) {
			r, _ := utf8.DecodeRuneInString(src[d.at:])
			next = string(r)
		}
		tkn := lex.Token("#BD", tabnas.TinBD, nil, next)
		tkn.Err = d.code
		tkn.Why = d.code
		// The cursor's column counts the lone `\r` it moved over as a
		// character; the engine's line matcher restarts it there.
		if d.rowCR >= 0 {
			tkn.CI = utf8.RuneCountInString(src[d.rowCR+1:d.at]) + 1
		}
		return tkn
	}
	return nil
}
