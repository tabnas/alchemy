// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// events_test.go: `events` (rs/tests/events_test.rs): the affine rule over
// events, which the checker holds before anything runs. The tests that run
// programs over events (the YAML-like block renderer, the explicit key
// form, YAML's non-finite numbers, the plan report held to its run, and
// events a program builds fed back to a taker of JSON events) are
// alchemy-cli's (go/e2e/events_test.go): they need transduce's routers and
// render's renderers.

import (
	"strings"
	"testing"

	"github.com/tabnas/alchemy/go/shared"
)

// events consumes the input, so a program that reads it twice, or captures
// it in a function, is refused as any affine stream is.
func TestEventsOverTheInputIsAffine(t *testing.T) {
	twice := "def export [input]\n  concat\n    join \"\" (map (fn [e] \"a\") (events input))\n    join \"\" (map (fn [e] \"b\") (events input))\n"
	if _, f := Compile(twice, "twice.alc", routers, renderers); f == nil || f.Code != CodeStreamReused || !strings.HasPrefix(f.Message, "reused: ") || f.Row != 4 || f.Column != 39 {
		t.Errorf("%v", f)
	}
	withJSON := "def export [input] (concat (json input) (join \"\" (map (fn [e] \"\") (events input))))"
	if _, f := Compile(withJSON, "json.alc", routers, renderers); f == nil || f.Code != CodeStreamReused || !strings.HasPrefix(f.Message, "reused: ") {
		t.Errorf("%v", f)
	}
	captured := "def export [input]\n  concat-map (fn [x] (join \"\" (map (fn [e] \"\") (events input)))) [1]\n"
	if _, f := Compile(captured, "captured.alc", routers, renderers); f == nil || f.Code != CodeStreamReused || !strings.HasPrefix(f.Message, "captured: ") {
		t.Errorf("%v", f)
	}
	// The stream events yields is a stream of items: events, which json
	// takes back and which the host renders as JSON when it is the output
	// (a rewritten tree), but not table events for csv; a stream of values
	// is not events.
	for _, c := range []struct{ src, finer string }{
		{"def export [input] (json (select (path each-index) input))", "protocol_mismatch"},
		{"def export [input] (csv csv-options (events input))", "protocol_mismatch"},
		{"def export [input] (events (select (path each-index) input))", "protocol_mismatch"},
		{"def export [input] (as-events input)", "protocol_mismatch"},
	} {
		if _, f := Compile(c.src, "bad.alc", routers, renderers); f == nil || f.Code != CodeDSLTypeError || !strings.HasPrefix(f.Message, c.finer+": ") {
			t.Errorf("%s: %v", c.src, f)
		}
	}
	if tree, f := Compile("def export [input] (events input)", "tree.alc", routers, renderers); f != nil || tree.Output() != OutputJsonEvents {
		t.Errorf("%v %v", tree, f)
	}
}

// parser-events: a host that declares a parser-events input hands the
// program the parser's rule passes, which parser-events reads as items and
// nothing else reads. The stream is affine as events's is, and a
// transducer's outputs are a rewritten tree, handed on as JSON events.
func TestParserEventsOverADeclaredInput(t *testing.T) {
	transducer := "def step [s pass]\n  match pass\n    case (close rule) (transition s [(scalar (get :name rule))])\n    case _ (transition s [])\n\ndef export [input]\n  as-events (scan-emit [] step (fn [s] []) (parser-events input))\n"
	program, f := CompileWith(transducer, "t.alc", InputParserEvents, routers, renderers)
	if f != nil {
		t.Fatal(f)
	}
	if program.Input() != InputParserEvents || program.Output() != OutputJsonEvents {
		t.Errorf("%v %v", program.Input(), program.Output())
	}
	if !strings.Contains(program.Explain(), "Protocol:              ParserEvents/1 → Stream<ParserEvent>") {
		t.Error(program.Explain())
	}
	// Compiled over the default input, JSON events, it reads the wrong
	// protocol.
	if _, f := Compile(transducer, "t.alc", routers, renderers); f == nil || !strings.HasPrefix(f.Message, "protocol_mismatch: ") {
		t.Errorf("%v", f)
	}
	if p, f := Compile("def export [input] input", "j.alc", routers, renderers); f != nil || p.Input() != InputJSONEvents {
		t.Errorf("%v", f)
	}

	twice := "def export [input]\n  concat\n    join \"\" (map (fn [p] \"a\") (parser-events input))\n    join \"\" (map (fn [p] \"b\") (parser-events input))\n"
	if _, f := CompileWith(twice, "twice.alc", InputParserEvents, routers, renderers); f == nil || f.Code != CodeStreamReused {
		t.Errorf("%v", f)
	}
	captured := "def export [input]\n  concat-map (fn [x] (join \"\" (map (fn [p] \"\") (parser-events input)))) [1]\n"
	if _, f := CompileWith(captured, "captured.alc", InputParserEvents, routers, renderers); f == nil || !strings.HasPrefix(f.Message, "captured: ") {
		t.Errorf("%v", f)
	}

	// The host drives the sink its declared input names, and is told so
	// before anything runs when it does not.
	refused := func(f *Fail) {
		t.Helper()
		if f == nil || f.Code != CodeDSLTypeError || !strings.HasPrefix(f.Message, "protocol_mismatch: ") {
			t.Errorf("%v", f)
		}
	}
	_, f = program.Sink(nil, RenderDefault, shared.DefaultLimits(), nil)
	refused(f)
	json, _ := Compile("def export [input] input", "j.alc", routers, renderers)
	_, f = json.ParserSink(nil, RenderDefault, shared.DefaultLimits(), nil)
	refused(f)

	// A host that takes a transducer's events into its own sink drives
	// ParserSinkInto, which holds the program to JSON events: one compiled
	// over JSON events, or answering a text or a table, is refused before
	// any stage is built. Running one is tabnas-alchemy-cli's.
	_, f = json.ParserSinkInto(nil, shared.DefaultLimits(), nil)
	refused(f)
	text, f := CompileWith("def export [input] (join \"\" (map (fn [p] \"a\") (parser-events input)))", "text.alc", InputParserEvents, routers, renderers)
	if f != nil || text.Output() != OutputText {
		t.Fatalf("%v", f)
	}
	if _, f := text.ParserSinkInto(nil, shared.DefaultLimits(), nil); f == nil || !strings.HasPrefix(f.Message, "protocol_mismatch: the program's result is a text") {
		t.Errorf("%v", f)
	}
	rows, f := CompileWith("def step [s pass]\n  match pass\n    case (close rule) (transition s [(scalar (get :name rule))])\n    case _ (transition s [])\n\ndef export [input]\n  table-from-json (record (entry :columns :infer) (entry :rows (path each-index))) (as-events (scan-emit [] step (fn [s] []) (parser-events input)))\n", "rows.alc", InputParserEvents, routers, renderers)
	if f != nil || rows.Output() != OutputTableRows {
		t.Fatalf("%v", f)
	}
	if _, f := rows.ParserSinkInto(nil, shared.DefaultLimits(), nil); f == nil || !strings.HasPrefix(f.Message, "protocol_mismatch: the program's result is a table") {
		t.Errorf("%v", f)
	}
}
