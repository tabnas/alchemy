// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// repeat_test.go: the reader repeats by replacement, never by a push chain
// (rs/tests/repeat_test.rs). Every repetition in the grammar is a replace
// loop (`r`), whose items all run in one frame, so rule depth follows a
// program's nesting and never its length. The engine's rule depth D is
// the observable, read with a rule subscriber on Make(): the maximum over
// ten thousand items of each repetition is what one item needs, while
// real nesting still grows it.

import (
	"fmt"
	"sort"
	"strings"
	"testing"
	"time"

	tabnas "github.com/tabnas/parser/go"
)

// maxDepth is the maximum rule depth any rule reaches while src parses.
func maxDepth(t *testing.T, src string) int {
	t.Helper()
	j := Make()
	max := 0
	j.Sub(nil, func(r *tabnas.Rule, _ *tabnas.Context) {
		if r.D > max {
			max = r.D
		}
	})
	if _, err := j.Parse(src); err != nil {
		n := len(src)
		if n > 40 {
			n = 40
		}
		t.Fatalf("%q does not parse: %v", src[:n], err)
	}
	return max
}

const repeatItems = 10_000

// Each repetition, as one item and as ten thousand, with the depth one
// item needs.
func TestEveryRepetitionStaysAtTheDepthOfOneItem(t *testing.T) {
	for _, c := range []struct {
		name, one, many string
		depth           int
	}{
		{"top-level lines", "x", strings.Repeat("x\n", repeatItems), 2},
		{"child lines", "p\n  x", "p\n" + strings.Repeat("  x\n", repeatItems), 4},
		{"inline forms", "f", "f" + strings.Repeat(" x", repeatItems), 2},
		{"items of ( )", "(x)", "(" + strings.Repeat("x ", repeatItems) + ")", 4},
		{"items of [ ]", "[x]", "[" + strings.Repeat("x ", repeatItems) + "]", 4},
	} {
		if got := maxDepth(t, c.one); got != c.depth {
			t.Errorf("%s: one item reaches %d, want %d", c.name, got, c.depth)
		}
		if got := maxDepth(t, c.many); got != c.depth {
			t.Errorf("%s: %d items reach %d, deeper than one item's %d", c.name, repeatItems, got, c.depth)
		}
	}
}

// Real recursion still nests: each `(` is a form and the list it opens,
// two frames deeper than the one around it.
func TestNestingStillGrowsTheDepth(t *testing.T) {
	nested := func(levels int) string { return strings.Repeat("(", levels) + strings.Repeat(")", levels) }
	for _, c := range [][2]int{{1, 3}, {10, 21}, {100, 201}} {
		if got := maxDepth(t, nested(c[0])); got != c[1] {
			t.Errorf("%d levels reach %d, want %d", c[0], got, c[1])
		}
	}
}

// The installed grammar says the same: the only close-phase push is a
// line's block (structure), and the two loops are the line and the form
// replacing themselves.
func TestTheRepetitionsAreReplaceLoops(t *testing.T) {
	j := Make()
	var pushes, replaces []string
	add := func(list *[]string, s string) {
		for _, x := range *list {
			if x == s {
				return
			}
		}
		*list = append(*list, s)
	}
	for _, spec := range j.Rules() {
		for _, phase := range []struct {
			name string
			alts []*tabnas.AltSpec
		}{{"open", spec.OpenAlts()}, {"close", spec.CloseAlts()}} {
			for _, alt := range phase.alts {
				if alt.P != "" {
					add(&pushes, fmt.Sprintf("%s %s p:%s", spec.Name, phase.name, alt.P))
				}
				if alt.R != "" {
					add(&replaces, fmt.Sprintf("%s %s r:%s", spec.Name, phase.name, alt.R))
				}
			}
		}
	}
	sort.Strings(pushes)
	sort.Strings(replaces)
	wantPushes := []string{
		"block open p:line",
		"bracket open p:form",
		"form open p:bracket",
		"form open p:paren",
		"line close p:block",
		"line open p:form",
		"paren open p:form",
		"program open p:line",
	}
	if strings.Join(pushes, "\n") != strings.Join(wantPushes, "\n") {
		t.Errorf("pushes:\n%s", strings.Join(pushes, "\n"))
	}
	if strings.Join(replaces, "\n") != "form close r:form\nline close r:line" {
		t.Errorf("replaces:\n%s", strings.Join(replaces, "\n"))
	}
}

// fastestParse is the fastest of a few parses of src, to keep a loaded
// machine's stalls and a coarse clock out of the comparison.
func fastestParse(t *testing.T, src string) time.Duration {
	j := Make()
	best := time.Duration(1 << 62)
	for i := 0; i < 3; i++ {
		start := time.Now()
		if _, err := j.Parse(src); err != nil {
			t.Fatal(err)
		}
		if d := time.Since(start); d < best {
			best = d
		}
	}
	return best
}

// Ten times the lines take about ten times as long. Quadratic work would
// take a hundred; the bound is generous so a busy machine cannot fail it.
func TestParseTimeGrowsLinearlyWithTheLines(t *testing.T) {
	small := fastestParse(t, strings.Repeat("f a 1\n", 1_000))
	large := fastestParse(t, strings.Repeat("f a 1\n", 10_000))
	s := small.Seconds()
	if s < 1e-6 {
		s = 1e-6
	}
	if ratio := large.Seconds() / s; ratio >= 30 {
		t.Errorf("1,000 lines took %v and 10,000 took %v: %.1fx", small, large, ratio)
	}
}
