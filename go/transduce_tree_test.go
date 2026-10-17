// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"fmt"
	"reflect"
	"strings"
	"testing"

	"github.com/tabnas/alchemy/go/shared"
)

// jsonSpec is the json grammar's spec, as the plan writes it.
const jsonSpec = "def json-spec\n  record\n    entry :map (record (entry :container :object))\n    entry :list (record (entry :container :array))\n    entry :pair (record (entry :key 0))\n    entry :val (record (entry :scalar 0))\n"

// A spec of the standard shape is read as one; a spec of any other shape
// is not, and runs the library's text (rs/src/lower.rs's test).
func TestATransduceTreeSpecOfTheStandardShapeIsRead(t *testing.T) {
	rt := runtimeOf(t, jsonSpec+
		"def named (record (entry :doc (record (entry :key \"k\") (entry :scalar :value) (entry :note 1))))\n"+
		"def odd-container (record (entry :map (record (entry :container :set))))\n"+
		"def odd-key (record (entry :pair (record (entry :key 1.5))))\n"+
		"def odd-scalar (record (entry :val (record (entry :scalar \"x\"))))\n"+
		"def odd-entry (record (entry :val 1))\n"+
		"def texts (record (entry :row (record (entry :container :object) (entry :member \"a\") (entry :text true))) (entry :cell (record (entry :member 1) (entry :text false))))\n"+
		"def odd-member (record (entry :row (record (entry :member -1))))\n"+
		"def odd-text (record (entry :row (record (entry :text :yes))))\n")
	value := func(name string) Val {
		v, f := rt.DefValue(ScopeProgram, name)
		if f != nil {
			t.Fatal(f)
		}
		return v
	}
	spec, ok := treeSpecOf(value("json-spec"))
	want := shared.TreeSpec{Rules: map[string]shared.TreeRule{
		"map":  {Container: shared.TreeObject},
		"list": {Container: shared.TreeArray},
		"pair": {Key: shared.TreeKey{Set: true, IsToken: true, Token: 0}},
		"val":  {Scalar: shared.TreeScalar{Set: true, Token: 0}},
	}}
	if !ok || !reflect.DeepEqual(spec, want) {
		t.Errorf("json-spec: %v %+v", ok, spec)
	}
	// A field the library never reads is not read here either.
	spec, ok = treeSpecOf(value("named"))
	want = shared.TreeSpec{Rules: map[string]shared.TreeRule{
		"doc": {Key: shared.TreeKey{Set: true, Name: "k"}, Scalar: shared.TreeScalar{Set: true, Value: true}},
	}}
	if !ok || !reflect.DeepEqual(spec, want) {
		t.Errorf("named: %v %+v", ok, spec)
	}
	// A member is read as a key is, and a text as a boolean.
	spec, ok = treeSpecOf(value("texts"))
	want = shared.TreeSpec{Rules: map[string]shared.TreeRule{
		"row":  {Container: shared.TreeObject, Member: shared.TreeKey{Set: true, Name: "a"}, Text: true},
		"cell": {Member: shared.TreeKey{Set: true, IsToken: true, Token: 1}},
	}}
	if !ok || !reflect.DeepEqual(spec, want) {
		t.Errorf("texts: %v %+v", ok, spec)
	}
	for _, odd := range []string{"odd-container", "odd-key", "odd-scalar", "odd-entry", "odd-member", "odd-text"} {
		if _, ok := treeSpecOf(value(odd)); ok {
			t.Errorf("%s is read as a spec", odd)
		}
	}
	if _, ok := treeSpecOf(NullVal{}); ok {
		t.Error("null is read as a spec")
	}
}

// logRouters has a guard that passes the events through and a scan that
// does nothing, each call logged, and no native transduce-tree.
type logRouters struct {
	inertRouters
	log *[]string
}

func (r logRouters) ScanEmit(any, func(any, any) (shared.Transition[any, any], *Fail), func(any) ([]any, *Fail), func(any) (shared.Flow, *Fail)) shared.ScanEmitter {
	*r.log = append(*r.log, "scan-emit")
	return inertScan{}
}

func (r logRouters) Guarded(inner shared.Sink, _ shared.Limits, _ *shared.AbortFlag, _ *shared.Metrics) shared.Sink {
	*r.log = append(*r.log, "guarded")
	return inner
}

// treeRouters is logRouters with a native transduce-tree offered or
// declined.
type treeRouters struct {
	logRouters
	offer bool
}

func (r treeRouters) TransduceTree(spec shared.TreeSpec, downstream shared.Sink) shared.ParserSink {
	*r.log = append(*r.log, fmt.Sprintf("transduce-tree of %d rules", len(spec.Rules)))
	if !r.offer {
		return nil
	}
	return nativeTree{downstream}
}

// nativeTree is the native treeRouters offers: it hands the parse's end
// on.
type nativeTree struct{ down shared.Sink }

func (n nativeTree) ParserEvent(ev shared.ParserEvent) (shared.Flow, *Fail) {
	if ev.Kind == shared.ParserEnd {
		return n.down.Event(shared.Event{Kind: shared.End})
	}
	return shared.Continue, nil
}

// inertScan is the scan treeRouters makes, which does nothing.
type inertScan struct{}

func (inertScan) Item(any) (shared.Flow, *Fail) { return shared.Continue, nil }
func (inertScan) Finish() (shared.Flow, *Fail)  { return shared.Continue, nil }

// The library's transduce-tree over the host's parser events runs natively
// where the routers offer it, the source's guard in front of what it
// writes, and through the library's scan where they decline, where the
// natives are off, and where the spec is not of the standard shape. A
// program that writes the library's scan itself is run the same way; one
// whose step is its own is not (rs/src/lower.rs's test).
func TestTransduceTreeRunsNativelyWhereTheRoutersOfferIt(t *testing.T) {
	lower := func(export string, native bool, offer string) []string {
		t.Helper()
		var log []string
		plain := logRouters{log: &log}
		var r shared.Routers = treeRouters{logRouters: plain, offer: offer == "offer"}
		if offer == "absent" {
			r = plain
		}
		rt := runtimeOf(t, jsonSpec+export).WithRouters(r).WithNative(native)
		result, f := rt.ExportWith(InputParserEvents)
		if f != nil {
			t.Fatal(f)
		}
		lowering := NewLowering(rt, shared.DefaultLimits(), nil)
		ended := &eventLog{}
		events, f := lowering.events(result.(StreamVal).Plan, ended)
		if f != nil {
			t.Fatal(f)
		}
		if _, f := lowering.ParserHead(events).ParserEvent(shared.ParserEvent{Kind: shared.ParserEnd}); f != nil {
			t.Fatal(f)
		}
		return append(log, ended.seen...)
	}
	check := func(got []string, want ...string) {
		t.Helper()
		if strings.Join(got, ", ") != strings.Join(want, ", ") {
			t.Errorf("got %v, want %v", got, want)
		}
	}
	tree := "def export [input] (transduce-tree json-spec input)\n"
	native := []string{"guarded", "transduce-tree of 4 rules", "end"}
	scan := []string{"guarded", "scan-emit", "end"}
	check(lower(tree, true, "offer"), native...)
	check(lower(tree, true, "decline"), "guarded", "transduce-tree of 4 rules", "scan-emit", "end")
	check(lower(tree, true, "absent"), scan...)
	check(lower(tree, false, "offer"), scan...)
	check(lower("def export [input] (transduce-tree (record (entry :map (record (entry :container :set)))) input)\n", true, "offer"), scan...)
	check(lower("def export [input] (as-events (scan-emit [[] false] (partial transduce-step json-spec) transduce-finish (parser-events input)))\n", true, "offer"), native...)
	check(lower("def export [input] (as-events (scan-emit [[[] false] []] (partial transduce-pass json-spec) transduce-done (parser-events input)))\n", true, "offer"), native...)
	// Each step from the other's state is a scan of the program's own.
	check(lower("def export [input] (as-events (scan-emit [[[] false] []] (partial transduce-step json-spec) transduce-done (parser-events input)))\n", true, "offer"), scan...)
	check(lower("def export [input] (as-events (scan-emit [[] false] (partial transduce-pass json-spec) transduce-finish (parser-events input)))\n", true, "offer"), scan...)
	check(lower("def my-step [spec state pass] (transduce-step spec state pass)\ndef export [input] (as-events (scan-emit [[] false] (partial my-step json-spec) transduce-finish (parser-events input)))\n", true, "offer"), scan...)
}
