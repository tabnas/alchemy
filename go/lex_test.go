// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// lex_test.go: the layout matcher's decisions (rs/src/lex.rs's tests).

import (
	"fmt"
	"strings"
	"testing"

	tabnas "github.com/tabnas/parser/go"
)

type word struct{ name, text string }

// words drives decide the way the lexer would over a source with no
// strings or comments: this matcher's tokens by name and source, and `_`
// for a position it passed on (advanced by one byte).
func words(src string, state *layoutState) []word {
	var out []word
	for si := 0; si < len(src); {
		d := decide(src, si, state)
		switch d.kind {
		case decidePass:
			out = append(out, word{"_", src[si : si+1]})
			si++
		case decideWord, decideLayout:
			out = append(out, word{d.name, src[si : si+d.n]})
			si += d.n
		case decideBad:
			return append(out, word{"BAD", d.code})
		}
	}
	return out
}

func names(src string) string {
	var b strings.Builder
	for _, w := range words(src, &layoutState{}) {
		switch w.name {
		case "_":
			b.WriteString(w.text)
		case tokenIN, tokenDE, tokenNL:
			b.WriteString(" " + w.name + " ")
		case "BAD":
			b.WriteString(" BAD:" + w.text + " ")
		default:
			b.WriteString(w.name + "(" + w.text + ")")
		}
	}
	return b.String()
}

// staircase is levels+1 lines, each two spaces deeper than the one before.
func staircase(levels int) string {
	lines := make([]string, levels+1)
	for i := range lines {
		lines[i] = strings.Repeat("  ", i) + "x"
	}
	return strings.Join(lines, "\n")
}

func TestJSONNumbersAreRecognisedExactly(t *testing.T) {
	for _, ok := range []string{"0", "-0", "12", "1.5", "-1.5e10", "2E-3", "0.0"} {
		if !IsJSONNumber(ok) {
			t.Errorf("%s is a JSON number", ok)
		}
	}
	for _, bad := range []string{"01", "1.", ".5", "+1", "1e", "-", "1_000", "0x1f", "1a"} {
		if IsJSONNumber(bad) {
			t.Errorf("%s is not a JSON number", bad)
		}
	}
}

func eqNames(t *testing.T, src, want string) {
	t.Helper()
	if got := names(src); got != want {
		t.Errorf("%q:\n got %q\nwant %q", src, got, want)
	}
}

func TestWordsSplitIntoNumbersValuesAndSymbols(t *testing.T) {
	eqNames(t, "def a-b? -1 1.5e3 true null :key x",
		"#TX(def) #TX(a-b?) #NR(-1) #NR(1.5e3) #VL(true) #VL(null) #KW(:key) #TX(x)")
}

func TestIndentationBecomesStructuralTokens(t *testing.T) {
	eqNames(t, "a\n  b\n    c\n  d\ne", "#TX(a) #IN #TX(b) #IN #TX(c) #DE #TX(d) #DE #TX(e)")
}

func TestADedentOfTwoLevelsIsIssuedOneTokenPerCall(t *testing.T) {
	state := &layoutState{}
	src := "a\n  b\n    c\nd"
	at := strings.Index(src, "c\n") + 1
	for si := 0; si < at; si++ {
		decide(src, si, state)
	}
	if state.levels != 2 {
		t.Fatalf("levels %d", state.levels)
	}
	first := decide(src, at, state)
	if first.kind != decideLayout || first.name != tokenDE || first.n != 1 || first.pending != 1 {
		t.Errorf("first %+v", first)
	}
	second := decide(src, at+1, state)
	if second.kind != decideLayout || second.name != tokenDE || second.n != 0 || second.pending != 0 {
		t.Errorf("second %+v", second)
	}
	if state.levels != 0 {
		t.Errorf("levels %d", state.levels)
	}
	if d := decide(src, at+1, state); d.kind != decideWord || d.name != "#TX" {
		t.Errorf("then %+v", d)
	}
}

func TestBlankAndCommentLinesDoNotCount(t *testing.T) {
	eqNames(t, "a\n\n  ; note\n   \n  b\n\n", "#TX(a) #IN #TX(b)\n\n")
}

func TestALineEndsAtALineFeedAndALoneCarriageReturnIsWhitespace(t *testing.T) {
	for _, c := range [][2]string{
		{"a\r\n  b\r\nc", "#TX(a) #IN #TX(b) #DE #TX(c)"},
		{"a\r  b\r c", "#TX(a)\r  #TX(b)\r #TX(c)"},
		{"a \r b", "#TX(a) \r #TX(b)"},
		{"a\n\r  b", "#TX(a) #IN \r  #TX(b)"},
		{"a\n  \r  b", "#TX(a) #IN \r  #TX(b)"},
		{"a\n  b\n  \rc", "#TX(a) #IN #TX(b) #DE \r#TX(c)"},
		{"a\n  b\n  \r  \r  c", "#TX(a) #IN #TX(b) #NL \r  \r  #TX(c)"},
		{"a\n\t\r  b", "#TX(a) #IN \r  #TX(b)"},
		{"\r  a", " BAD:bad_indent "},
		{"\ra", "\r#TX(a)"},
		{"a\r\n\r  b", "#TX(a) #IN \r  #TX(b)"},
	} {
		eqNames(t, c[0], c[1])
	}
}

// Comment fragments ahead of the first form cost one scan, not one per
// fragment.
func TestTheTriviaBeforeTheFirstFormIsScannedOnce(t *testing.T) {
	for _, sep := range []string{"\r", "\n", "\r\n"} {
		src := strings.Repeat(sep+";comment", 10_000) + "\na b"
		j := Make()
		var state *layoutState
		j.Sub(func(_ *tabnas.Token, _ *tabnas.Rule, ctx *tabnas.Context) {
			if s, ok := ctx.U[layoutStateKey].(*layoutState); ok {
				state = s
			}
		}, nil)
		v, err := j.Parse(src)
		if err != nil {
			t.Fatalf("%q: %v", sep, err)
		}
		if got := strings.Count(valueJSON(plain(v)), `"a"`); got != 1 {
			t.Errorf("%q: %d", sep, got)
		}
		if state == nil || state.scans != 1 {
			t.Errorf("%q: scans %v", sep, state)
		}
	}
	state := &layoutState{}
	src := "\r;x\r;y\n  \nab"
	if decide(src, 0, state).kind != decidePass || state.checked != len(src)-2 {
		t.Errorf("checked %d", state.checked)
	}
	if decide(src, 3, state).kind != decidePass || decide(src, 6, state).kind != decidePass || state.checked != len(src)-2 {
		t.Errorf("checked %d", state.checked)
	}
	state = &layoutState{}
	if decide("\r;x\n;y", 0, state).kind != decidePass || state.checked != 6 {
		t.Errorf("checked %d", state.checked)
	}
	// Past the first form the count is the layout's own: one scan per line
	// end outside delimiters, none inside them.
	state = &layoutState{}
	words("a\n  b\n  c (d\n e)\nf", state)
	if state.scans != 3 {
		t.Errorf("scans %d", state.scans)
	}
}

func TestALineHoldingOnlyTriviaAfterALoneCarriageReturnIsBlank(t *testing.T) {
	for _, c := range [][2]string{
		{"f x\n\r", "#TX(f) #TX(x)\n\r"},
		{"f x\n  \r  ", "#TX(f) #TX(x)\n  \r  "},
		{"a\n  b\n  \r;c\nd", "#TX(a) #IN #TX(b) #DE #TX(d)"},
		{"a\n  b\n\r;x\n  c", "#TX(a) #IN #TX(b) #NL #TX(c)"},
		{"a\n;x\r  c", "#TX(a) #IN \r  #TX(c)"},
		{"\r;x\n  a", " BAD:bad_indent "},
	} {
		eqNames(t, c[0], c[1])
	}
}

func TestAnErrorAfterALoneCarriageReturnCountsItsColumnFromTheLast(t *testing.T) {
	for _, c := range []struct {
		src       string
		si        int
		seen      bool
		code      string
		at, rowCR int
	}{
		{"  \r  a", 0, false, "bad_indent", 5, 2},
		{"\r \r   a", 0, false, "bad_indent", 6, 2},
		{"a\n\r \tb", 1, true, "tab_indent", 4, 2},
		{"a\n  \r   b", 1, true, "bad_indent", 8, 4},
	} {
		d := decide(c.src, c.si, &layoutState{seen: c.seen})
		if d.kind != decideBad || d.code != c.code || d.at != c.at || d.rowCR != c.rowCR {
			t.Errorf("%q: %+v", c.src, d)
		}
	}
}

func TestLayoutIsSuspendedInsideDelimiters(t *testing.T) {
	eqNames(t, "a (b\n  c)\n  d", "#TX(a) (#TX(b)\n  #TX(c)) #IN #TX(d)")
	eqNames(t, "[1\n2]", "[#NR(1)\n#NR(2)]")
}

func TestAStrayCloserIsUnbalanced(t *testing.T) {
	eqNames(t, "a )", "#TX(a)  BAD:unbalanced ")
	eqNames(t, "(a))", "(#TX(a)) BAD:unbalanced ")
}

func TestIndentationErrorsHaveTheirOwnCodes(t *testing.T) {
	eqNames(t, "a\n\tb", "#TX(a) BAD:tab_indent ")
	eqNames(t, "a\n   b", "#TX(a) BAD:bad_indent ")
	eqNames(t, "a\n  b\n c", "#TX(a) #IN #TX(b) BAD:bad_dedent ")
	eqNames(t, "  a", " BAD:bad_indent ")
	eqNames(t, "\n\n  a", " BAD:bad_indent ")
}

func TestNestingPastTheBoundIsTooDeepAtTheOpenerOrTheLine(t *testing.T) {
	// A layout line and 255 open parens are 256 levels; the 256th paren is
	// one more.
	atLimit := strings.Repeat("(", MaxNesting-1)
	eqNames(t, atLimit, atLimit)
	eqNames(t, strings.Repeat("(", MaxNesting), atLimit+" BAD:too_deep ")
	// Indentation levels count the same way, together with delimiters.
	if strings.Contains(names(staircase(MaxNesting-1)), "BAD") {
		t.Error("the staircase at the bound is refused")
	}
	if !strings.HasSuffix(names(staircase(MaxNesting)), " BAD:too_deep ") {
		t.Error("the staircase past the bound is not refused")
	}
	mixed := staircase(2) + "\n" + strings.Repeat("  ", 3) + "["
	if !strings.HasSuffix(names(mixed+strings.Repeat("(", MaxNesting-5)), "(") {
		t.Error("the mixed nesting at the bound is refused")
	}
	if !strings.HasSuffix(names(mixed+strings.Repeat("(", MaxNesting-4)), " BAD:too_deep ") {
		t.Error("the mixed nesting past the bound is not refused")
	}
}

func TestLeadingTriviaBeforeTheFirstLineIssuesNothing(t *testing.T) {
	eqNames(t, "\n\n  \na", "\n\n  \n#TX(a)")
}

func TestTheDepthScanIgnoresStringsAndComments(t *testing.T) {
	state := &layoutState{}
	src := "(\"a)\" ; )\n)"
	if d := state.depthAt(src, len(src)-1); d != 1 {
		t.Errorf("%d", d)
	}
	if d := state.depthAt(src, len(src)); d != 0 {
		t.Errorf("%d", d)
	}
	// Asking about an earlier position restarts the scan.
	if d := state.depthAt(src, 1); d != 1 {
		t.Errorf("%d", d)
	}
}

// The state lives in the parse's context bag, one value under one key,
// throughout a parse, and a parse ends with every level closed.
func TestTheStateLivesInTheContextBag(t *testing.T) {
	j := Make()
	var seen []layoutState
	var ptrs []string
	j.Sub(func(_ *tabnas.Token, _ *tabnas.Rule, ctx *tabnas.Context) {
		if s, ok := ctx.U[layoutStateKey].(*layoutState); ok {
			seen = append(seen, *s)
			ptrs = append(ptrs, fmt.Sprintf("%p/%d", s, len(ctx.U)))
		}
	}, nil)
	if _, err := j.Parse("a\n  b\n    c\nd"); err != nil {
		t.Fatal(err)
	}
	deep := false
	for _, s := range seen {
		deep = deep || (s.levels == 2 && s.seen)
	}
	if !deep {
		t.Errorf("no token was lexed three levels deep: %+v", seen)
	}
	last := seen[len(seen)-1]
	if !last.seen || last.levels != 0 || last.pending != 0 || last.depth != 0 {
		t.Errorf("last %+v", last)
	}
	for _, p := range ptrs {
		if p != ptrs[0] {
			t.Errorf("one state, one key, throughout: %v", ptrs)
			break
		}
	}
	// A second parse on the instance starts afresh.
	seen = nil
	if _, err := j.Parse("x"); err != nil {
		t.Fatal(err)
	}
	if len(seen) == 0 || seen[0].levels != 0 {
		t.Errorf("%+v", seen)
	}
}
