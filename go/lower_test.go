// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// lower_test.go: plans to sinks (rs/src/lower.rs's tests).

import (
	"bytes"
	"strings"
	"testing"

	tabnasjson "github.com/tabnas/json/go"
	tr "github.com/tabnas/render/go"
	tt "github.com/tabnas/transduce/go"
)

// lowerRun runs src over the JSON input as rs/src/lower.rs's tests do: a
// runtime over the resolved program (no checker, so the lowering's own
// refusals are seen), its export lowered to a sink over a writer, and the
// input read incrementally with no pruning; through the materialized walk
// in a build without the incremental adapter (see readRun).
func lowerRun(t testing.TB, src, input string, native bool, render Renderer) (string, *Fail) {
	t.Helper()
	rt := runtimeOf(t, src).WithNative(native)
	result, f := rt.Export()
	if f != nil {
		return "", f
	}
	limits, metrics := tt.DefaultLimits(), tt.NewMetrics()
	var buffer bytes.Buffer
	sink, f := NewLowering(rt, limits, metrics).Sink(result, tr.NewWriteOut(&buffer), render)
	if f != nil {
		return "", f
	}
	if tt.AdapterBuilt() {
		_, f = tt.NewParserSource(tabnasjson.Make(), input).
			Grammar("json").
			Mode(tt.IncrementalMode(tt.Prune{})).
			Limits(limits).
			Metrics(metrics).
			Run(sink)
	} else {
		_, f = tt.Replay(mustEvents(t, input), sink)
	}
	if f != nil {
		return "", f
	}
	return buffer.String(), nil
}

func mustLower(t testing.TB, src, input string, native bool, render Renderer) string {
	t.Helper()
	out, f := lowerRun(t, src, input, native, render)
	if f != nil {
		t.Fatalf("%q over %q (native %v): %v", src, input, native, f)
	}
	return out
}

func TestTheWorkedExampleThroughTheInterpretedLibrary(t *testing.T) {
	if got := mustLower(t, workedExample, records, false, RenderDefault); got != expectedCSV {
		t.Errorf("%q", got)
	}
}

func TestTheWorkedExampleThroughTheNativePath(t *testing.T) {
	if got := mustLower(t, workedExample, records, true, RenderDefault); got != expectedCSV {
		t.Errorf("%q", got)
	}
}

func TestJSONEchoesTheInputWithItsLexemes(t *testing.T) {
	requireIncremental(t)
	echo := "def export [input] (json input)"
	if got := mustLower(t, echo, records, true, RenderDefault); got != records+"\n" {
		t.Errorf("%q", got)
	}
	if got := mustLower(t, echo, "[1.50, 1e2]", false, RenderDefault); got != "[1.50,1e2]\n" {
		t.Errorf("%q", got)
	}
}

func TestSelectMapAndConcatMapStreamItems(t *testing.T) {
	requireIncremental(t)
	src := "def export [input]\n  pipe input\n    select (path \"a\" each-index)\n    map (fn [x] (get :n x))\n    concat-map (fn [n] (concat (scalar-text csv-options n) \";\"))"
	if got := mustLower(t, src, `{"a":[{"n":1},{"n":2.50}],"b":3}`, false, RenderDefault); got != "1;2.50;" {
		t.Errorf("%q", got)
	}
	joined := "def export [input]\n  join \",\"\n    map (fn [x] (get :n x)) (select (path \"a\" each-index) input)"
	if got := mustLower(t, joined, `{"a":[{"n":"x"},{"n":""},{"n":"y"}]}`, false, RenderDefault); got != "x,,y" {
		t.Errorf("%q", got)
	}
	framed := "def export [input]\n  concat\n    \"[\"\n    join \",\" (select (path each-index) input)\n    \"]\"\n    (text \"!\")"
	if got := mustLower(t, framed, `["a","b"]`, false, RenderDefault); got != "[a,b]!" {
		t.Errorf("%q", got)
	}
	if got := mustLower(t, framed, "[]", false, RenderDefault); got != "[]!" {
		t.Errorf("%q", got)
	}
	filtered := "def export [input]\n  concat-map (fn [x] (get :v x))\n    filter (fn [x] (get :keep x)) (select (path each-index) input)"
	if got := mustLower(t, filtered, `[{"keep":true,"v":"a"},{"keep":false,"v":"b"},{"keep":true,"v":"c"}]`, false, RenderDefault); got != "ac" {
		t.Errorf("%q", got)
	}
}

func TestReplaceTextOverALiveTextCrossesFragments(t *testing.T) {
	src := "def export [input]\n  replace-text \"ab\" \"X\"\n    concat-map (fn [s] s) (select (path each-index) input)"
	if got := mustLower(t, src, `["a","b","zab","a"]`, false, RenderDefault); got != "XzXa" {
		t.Errorf("%q", got)
	}
}

func TestAFiniteTextResultIsWrittenAtTheEnd(t *testing.T) {
	if got := mustLower(t, `def export [input] "done"`, "1", false, RenderDefault); got != "done" {
		t.Errorf("%q", got)
	}
}

const recordsJSONOut = `[{"Identifier":123,"Full name":"Alice","Balance":50.25},{"Identifier":456,"Full name":"Bob","Balance":72}]`

func TestAStreamResultIsRenderedByTheHost(t *testing.T) {
	table := "def export [input] (table-from-json api-binding input)\n" +
		strings.Replace(workedExample, "def export [input]\n  pipe input\n    api-table\n    csv csv-options\n", "", 1)
	if got := mustLower(t, table, records, true, RenderDefault); got != expectedCSV {
		t.Errorf("%q", got)
	}
	if got := mustLower(t, table, records, false, RenderCSV); got != expectedCSV {
		t.Errorf("%q", got)
	}
	for _, native := range []bool{true, false} {
		if got := mustLower(t, table, records, native, RenderJSON); got != recordsJSONOut+"\n" {
			t.Errorf("native %v: %q", native, got)
		}
	}
	echo := "def export [input] input"
	if got := mustLower(t, echo, "[1]", true, RenderDefault); got != "[1]\n" {
		t.Errorf("%q", got)
	}
	if _, f := lowerRun(t, echo, "[1]", true, RenderCSV); f == nil || !strings.HasPrefix(f.Message, "protocol_mismatch: ") {
		t.Errorf("%v", f)
	}
	if _, f := lowerRun(t, "def export [input] (json input)", "1", true, RenderJSON); f == nil || !strings.HasPrefix(f.Message, "render_of_text: ") {
		t.Errorf("%v", f)
	}
}

func TestRecordsOfATableRoundTripsThroughJSON(t *testing.T) {
	src := strings.Replace(workedExample, "    csv csv-options\n", "    records\n    json\n", 1)
	for _, native := range []bool{true, false} {
		if got := mustLower(t, src, records, native, RenderDefault); got != recordsJSONOut+"\n" {
			t.Errorf("native %v: %q", native, got)
		}
	}
}

func TestProtocolMismatchesAreNamed(t *testing.T) {
	for _, c := range []struct{ src, input string }{
		{"def export [input] (csv csv-options input)", "1"},
		{"def export [input] (json (select (path each-index) input))", "[1]"},
		{"def export [input] (concat-map (fn [x] x) input)", "[1]"},
	} {
		_, f := lowerRun(t, c.src, c.input, true, RenderDefault)
		if f == nil || f.Code != CodeDSLTypeError || !strings.HasPrefix(f.Message, "protocol_mismatch: ") {
			t.Errorf("%s: %v", c.src, f)
		}
	}
}

func TestTheCsvOptionsMapToTheDialectOrNot(t *testing.T) {
	rt := runtimeOf(t, "def o csv-options\ndef lf (record (entry :delimiter \";\") (entry :newline \"\\n\") (entry :header false) (entry :null-text \"NULL\") (entry :missing \"-\"))\ndef bad (record (entry :delimiter \"ab\"))")
	value := func(name string) Val {
		v, f := rt.DefValue(ScopeProgram, name)
		if f != nil || v == nil {
			t.Fatalf("%s: %v", name, f)
		}
		return v
	}
	if o, ok := csvOptions(value("o")); !ok || o.Delimiter != ',' || o.Newline != tr.NewlineCRLF || !o.Header || o.NullText != "" || o.Missing != nil || o.Quoting != tr.QuotingAlways {
		t.Errorf("%+v %v", o, ok)
	}
	lf, ok := csvOptions(value("lf"))
	if !ok || lf.Delimiter != ';' || lf.Newline != tr.NewlineLF || lf.Header || lf.NullText != "NULL" || lf.Missing == nil || *lf.Missing != "-" {
		t.Errorf("%+v %v", lf, ok)
	}
	if _, ok := csvOptions(value("bad")); ok {
		t.Error("a two-character delimiter maps")
	}
	if _, ok := csvOptions(NullVal{}); ok {
		t.Error("null maps")
	}
}
