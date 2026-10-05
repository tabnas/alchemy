// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// program_test.go: the API a host embeds (rs/src/program.rs's tests).

import (
	"bytes"
	"strings"
	"testing"

	tt "github.com/tabnas/transduce/go"
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
}

func TestTheSinkWritesThroughAWriter(t *testing.T) {
	p := mustCompile(t, workedExample, "t.alc")
	var buffer bytes.Buffer
	sink, f := p.Sink(&buffer, RenderDefault, tt.DefaultLimits(), tt.NewMetrics())
	if f != nil {
		t.Fatal(f)
	}
	d, err := tt.DatumFromJSON(records)
	if err != nil {
		t.Fatal(err)
	}
	if _, f := tt.WalkDatum(&d, sink); f != nil {
		t.Fatal(f)
	}
	if _, f := sink.Event(tt.EvEnd()); f != nil {
		t.Fatal(f)
	}
	if buffer.String() != expectedCSV {
		t.Errorf("%q", buffer.String())
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
	flag := tt.NewAbortFlag()
	q := p.WithAbort(flag)
	if q.Result() != p.Result() || q.abort != flag || p.abort == flag {
		t.Error("WithAbort rebuilt the plan or changed the program it was asked of")
	}
}
