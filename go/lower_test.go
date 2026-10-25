// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// lower_test.go: plans to sinks (rs/src/lower.rs's tests): the parser's
// rule passes pushed into the head. The tests that lower a program to a
// sink and run it are dmog's (go/e2e/lower_test.go): they need
// transduce's routers and render's renderers.

import (
	"strings"
	"testing"

	"github.com/tabnas/alchemy/go/shared"
)

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
	// A host's malformed event is refused, never read: an open or a close
	// without its pass (the zero value is one), and a kind that is none.
	for _, bad := range []shared.ParserEvent{{}, {Kind: shared.ParserClose}, {Kind: shared.ParserEventKind(9), Pass: &shared.RulePass{}}} {
		if _, f := (&parserEventsToItems{down: &itemLog{}}).ParserEvent(bad); f == nil || f.Code != shared.CodeProtocolOrderError {
			t.Errorf("%+v: %v", bad, f)
		}
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
