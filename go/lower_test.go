// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// lower_test.go: plans to sinks (rs/src/lower.rs's tests): how a csv
// plan's options map onto the renderer's dialect. The tests that lower a
// program to a sink and run it are alchemy-cli's (go/e2e/lower_test.go):
// they need transduce's routers and render's renderers.

import (
	"fmt"
	"strings"
	"testing"

	"github.com/tabnas/alchemy/go/shared"
)

func TestTheCsvOptionsMapToTheDialectOrNot(t *testing.T) {
	rt := runtimeOf(t, "def o csv-options\ndef lf (record (entry :delimiter \";\") (entry :newline \"\\n\") (entry :header false) (entry :null-text \"NULL\") (entry :missing \"-\"))\ndef bad (record (entry :delimiter \"ab\"))")
	value := func(name string) Val {
		v, f := rt.DefValue(ScopeProgram, name)
		if f != nil || v == nil {
			t.Fatalf("%s: %v", name, f)
		}
		return v
	}
	if o, ok := csvOptions(value("o")); !ok || o.Delimiter != ',' || o.Newline != shared.NewlineCRLF || !o.Header || o.NullText != "" || o.Missing != nil || o.Quoting != shared.QuotingAlways {
		t.Errorf("%+v %v", o, ok)
	}
	lf, ok := csvOptions(value("lf"))
	if !ok || lf.Delimiter != ';' || lf.Newline != shared.NewlineLF || lf.Header || lf.NullText != "NULL" || lf.Missing == nil || *lf.Missing != "-" {
		t.Errorf("%+v %v", lf, ok)
	}
	if _, ok := csvOptions(value("bad")); ok {
		t.Error("a two-character delimiter maps")
	}
	if _, ok := csvOptions(NullVal{}); ok {
		t.Error("null maps")
	}
}

// :non-finite and :no-columns, when the record has them, are policies,
// which the lowering applies around the renderer; one that is none maps to
// no dialect, and the library's csv runs, which refuses it where it is
// read.
func TestACsvOptionsRecordMapsWithANonFinitePolicyAndNotWithAnotherValue(t *testing.T) {
	const base = `(entry :delimiter ",") (entry :newline "\r\n") (entry :header true) (entry :null-text "") (entry :missing :error)`
	record := func(policy string) string { return "(record " + base + " " + policy + ")" }
	for _, c := range []struct {
		policy string
		maps   bool
	}{
		{"(entry :non-finite :literal)", true}, {"(entry :non-finite :null)", true}, {"(entry :non-finite :reject)", true},
		{"(entry :non-finite :nope)", false}, {"(entry :non-finite 1)", false},
		{"(entry :no-columns :empty)", true}, {"(entry :no-columns :refuse)", true}, {"(entry :no-columns :nope)", false},
	} {
		o, ok := csvOptions(mustEval(t, "", record(c.policy)))
		if ok != c.maps || (ok && o != shared.DefaultCSVOptions()) {
			t.Errorf("%s: %+v %v", c.policy, o, ok)
		}
		// The fast path takes what maps, and declines the rest for the
		// library's text.
		src := "def b\n  record\n    entry :columns (path \"m\")\n    entry :rows (path \"r\" each-index)\n    entry :column (fn [d] d)\n" +
			"def export [input] (csv " + record(c.policy) + " (table-from-json b input))"
		text, ok := mustCompile(t, src, "t.alc").Result().(TextVal)
		if !ok {
			t.Errorf("%s: not a text", c.policy)
		} else if (text.Plan.Kind == PlanCsv) != c.maps {
			t.Errorf("%s: %s", c.policy, PlanName(text.Plan))
		}
	}
}

// tableLog is what reaches a renderer, an event to a line.
type tableLog struct{ lines []string }

func (l *tableLog) TableEvent(ev shared.TableEvent) (shared.Flow, *Fail) {
	switch ev.Kind {
	case shared.TableSchema:
		l.lines = append(l.lines, fmt.Sprintf("schema %d", len(ev.Columns)))
	case shared.TableRow:
		l.lines = append(l.lines, fmt.Sprintf("row %d", len(ev.Cells)))
	default:
		l.lines = append(l.lines, "end")
	}
	return shared.Continue, nil
}

// The table :no-columns :empty keeps back from the renderer is held to
// what the library's csv-table holds it to, with its texts: a clean empty
// table reaches the renderer not at all, and a table of columns reaches it
// whole, a later schema too, for the renderer to refuse.
func TestNoColumnsHoldsTheTableItKeepsBack(t *testing.T) {
	// The events through a fresh adapter: what reached the renderer, or
	// the failure's text.
	run := func(events ...shared.TableEvent) ([]string, string) {
		log := &tableLog{}
		n := &noColumns{next: log}
		for _, ev := range events {
			if _, f := n.TableEvent(ev); f != nil {
				if f.Code != CodeProtocolOrderError {
					t.Errorf("%v", f)
				}
				return nil, f.Message
			}
		}
		return log.lines, ""
	}
	schema := func(labels ...string) shared.TableEvent {
		columns := []shared.PublicColumn{}
		for _, l := range labels {
			columns = append(columns, shared.PublicColumn{Label: l})
		}
		return shared.TableEvent{Kind: shared.TableSchema, Columns: columns}
	}
	row := func(cells ...shared.Cell) shared.TableEvent {
		return shared.TableEvent{Kind: shared.TableRow, Cells: cells}
	}
	end := shared.TableEvent{Kind: shared.TableEnd}
	one := shared.Cell{Kind: shared.CellNumber, Value: 1}
	// A clean empty table: nothing reaches the renderer.
	if seen, f := run(schema(), row(), row(), end); f != "" || len(seen) != 0 {
		t.Errorf("%v %q", seen, f)
	}
	// Each refusal, with the library's text.
	for _, c := range []struct {
		events []shared.TableEvent
		want   string
	}{
		{[]shared.TableEvent{schema(), row(), row(one)}, "row 2 has 1 cells; the schema has 0 columns"},
		{[]shared.TableEvent{schema(), schema("a")}, "a second schema"},
		{[]shared.TableEvent{schema(), end, schema()}, "a schema after the end"},
		{[]shared.TableEvent{schema(), end, row()}, "a row after the end"},
		{[]shared.TableEvent{schema(), end, end}, "a second end"},
	} {
		if _, f := run(c.events...); f != c.want {
			t.Errorf("%q, want %q", f, c.want)
		}
	}
	// A table of columns passes as it came, and so does a later empty
	// schema, for the renderer to refuse.
	seen, f := run(schema("a"), row(one), end, schema())
	if f != "" || strings.Join(seen, ", ") != "schema 1, row 1, end, schema 0" {
		t.Errorf("%v %q", seen, f)
	}
}

// itemLog records what reaches the items below a stage, and the end.
type itemLog struct{ seen []string }

func (l *itemLog) Item(v Val) (shared.Flow, *Fail) {
	l.seen = append(l.seen, DebugString(v))
	return shared.Continue, nil
}

func (l *itemLog) End() (shared.Flow, *Fail) {
	l.seen = append(l.seen, "end")
	return shared.Continue, nil
}

// eventLog records the JSON events a sink receives.
type eventLog struct{ seen []string }

func (l *eventLog) Event(ev shared.Event) (shared.Flow, *Fail) {
	l.seen = append(l.seen, ev.String())
	return shared.Continue, nil
}

// The host pushes the parser's rule passes into the head parser-events is
// lowered to: each pass reaches the items below as (open rule) or (close
// rule), a field the pass lacks left out of the rule, and the parse's end
// as the stream's end. A program that reads nothing of its input hands the
// end alone to the sink that writes its text.
func TestParserEventsAreItemsPushedIntoTheHead(t *testing.T) {
	var passes []shared.ParserEvent
	for _, text := range []string{
		`["open",{"name":"val","d":1,"i":1,"alt":{"p":"","r":"","b":0,"g":["json"]},"tokens":[["#NR",2.5,"2.50",1,1]],"forced":false,"n":{"dmap":1}}]`,
		`["close",{"name":"val","d":1,"i":1,"tokens":[],"forced":false,"n":{},"value":2.5}]`,
		`["end"]`,
	} {
		ev, err := shared.ParserEventFromJSON(text)
		if err != nil {
			t.Fatal(err)
		}
		passes = append(passes, ev)
	}
	plan := &Plan{Kind: PlanParserEvents, Source: &Plan{Kind: PlanParserInput}}
	log := &itemLog{}
	here := NewLowering(runtimeOf(t, "def export [input] (parser-events input)"), shared.DefaultLimits(), nil)
	events, f := here.items(plan, log, false)
	if f != nil {
		t.Fatal(f)
	}
	// The parser events are read once.
	if _, f := here.items(plan, log, false); f == nil || f.Code != CodeStreamReused {
		t.Errorf("%v", f)
	}
	head := here.ParserHead(events)
	for _, pass := range passes {
		if _, f := head.ParserEvent(pass); f != nil {
			t.Fatal(f)
		}
	}
	want := []string{
		`(open (record (entry :name "val") (entry :d 1) (entry :i 1) (entry :alt (record (entry :p "") (entry :r "") (entry :b 0) (entry :g ["json"]))) (entry :tokens [(record (entry :name "#NR") (entry :val 2.5) (entry :src "2.50") (entry :row 1) (entry :col 1))]) (entry :forced false) (entry :n (record (entry :dmap 1)))))`,
		`(close (record (entry :name "val") (entry :d 1) (entry :i 1) (entry :tokens []) (entry :forced false) (entry :n (record)) (entry :value 2.5)))`,
		"end",
	}
	if strings.Join(log.seen, "\n") != strings.Join(want, "\n") {
		t.Errorf("got\n%s\nwant\n%s", strings.Join(log.seen, "\n"), strings.Join(want, "\n"))
	}
	// The JSON-events side of a parser program is never driven.
	if _, f := (parserEventsOnly{}).Event(shared.Event{Kind: shared.Null}); f == nil || !strings.HasPrefix(f.Message, "protocol_mismatch: ") {
		t.Errorf("%v", f)
	}

	text := &eventLog{}
	nothing := NewLowering(runtimeOf(t, "def export [input] \"done\""), shared.DefaultLimits(), nil)
	finite := nothing.ParserHead(text)
	for _, pass := range passes {
		if _, f := finite.ParserEvent(pass); f != nil {
			t.Fatal(f)
		}
	}
	if len(text.seen) != 1 {
		t.Errorf("%v", text.seen)
	}
}
