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
