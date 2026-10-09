// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// program_test.go: the API a host embeds (rs/src/program.rs's tests). The
// sink a program writes through is alchemy-cli's to test
// (go/e2e/program_test.go): it runs on transduce's and render's stages.

import (
	"strings"
	"testing"

	"github.com/tabnas/alchemy/go/shared"
)

func rowSelectorText(p *Program) string {
	s, ok := p.RowSelector()
	if !ok {
		return "<none>"
	}
	return s.String()
}

func TestAProgramKnowsItsOutputAndRowSelector(t *testing.T) {
	p := mustCompile(t, workedExample, "t.alc")
	if p.Output() != OutputText || rowSelectorText(p) != ".response.payload.deep.records[*]" || !p.Native() {
		t.Errorf("%s %s %v", p.Output(), rowSelectorText(p), p.Native())
	}
	slow := interpreted(t, p)
	if slow.Native() || rowSelectorText(slow) != ".response.payload.deep.records[*]" {
		t.Errorf("%v %s", slow.Native(), rowSelectorText(slow))
	}
	if table := mustCompile(t, strings.Replace(workedExample, "    csv csv-options\n", "", 1), "t.alc"); table.Output() != OutputTableRows {
		t.Errorf("%s", table.Output())
	}
	echo := mustCompile(t, "def export [input] input", "t.alc")
	if echo.Output() != OutputJsonEvents || rowSelectorText(echo) != "<none>" {
		t.Errorf("%s %s", echo.Output(), rowSelectorText(echo))
	}
	sel := mustCompile(t, `def export [input] (join "," (select (path "a" each-index) input))`, "t.alc")
	if sel.Output() != OutputText || rowSelectorText(sel) != ".a[*]" {
		t.Errorf("%s %s", sel.Output(), rowSelectorText(sel))
	}
	// A rewritten tree the program says is events: the rows are still the
	// select's, behind as-events.
	rewritten := mustCompile(t, `def export [input] (as-events (map (fn [x] (scalar x)) (select (path "a" each-index) input)))`, "t.alc")
	if rewritten.Output() != OutputJsonEvents || rowSelectorText(rewritten) != ".a[*]" {
		t.Errorf("%s %s", rewritten.Output(), rowSelectorText(rewritten))
	}
}

func TestCompileReportsReaderResolverAndExportFailures(t *testing.T) {
	if _, f := Compile("(a b", "t.alc", routers, renderers); f == nil || f.Code != CodeDSLParseError {
		t.Errorf("%v", f)
	}
	if _, f := Compile("def export [input] (nope input)", "t.alc", routers, renderers); f == nil || !strings.HasPrefix(f.Message, "unknown_name: ") {
		t.Errorf("%v", f)
	}
	if _, f := Compile("def x 1", "t.alc", routers, renderers); f == nil || !strings.HasPrefix(f.Message, "no_export: ") {
		t.Errorf("%v", f)
	}
}

// WithAbort keeps the plan and changes only the flag the sinks read.
func TestWithAbortKeepsThePlan(t *testing.T) {
	p := mustCompile(t, workedExample, "t.alc")
	flag := shared.NewAbortFlag()
	q := p.WithAbort(flag)
	if q.Result() != p.Result() || q.abort != flag || p.abort == flag {
		t.Error("WithAbort rebuilt the plan or changed the program it was asked of")
	}
}
