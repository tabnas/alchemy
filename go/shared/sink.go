// Copyright (c) 2026 tabnas, MIT License

package shared

import (
	"fmt"
	"strconv"
)

// Flow is what a stage wants next.
type Flow uint8

const (
	// Continue: keep sending.
	Continue Flow = iota
	// Stop: the stage has all it needs; the source should stop. Not an
	// error: the source stops the parse, releases what it holds, and
	// reports nothing further.
	Stop
)

// String is "continue" or "stop".
func (f Flow) String() string {
	if f == Stop {
		return "stop"
	}
	return "continue"
}

// Sink consumes JsonEvents/1. A pipeline is a chain of sinks: the source
// calls the first once per event, synchronously, on the goroutine that
// parses; each stage does its work and calls the next. Nothing is queued
// between stages, so a slow writer at the end slows the parser at the
// start: that is the backpressure, and it costs no buffer.
//
// A non-nil *Fail aborts the run; the source stops the parse and the
// failure reaches the caller unchanged.
type Sink interface {
	Event(ev Event) (Flow, *Fail)
}

// Recorder is a sink that keeps every event.
type Recorder struct {
	Events []Event
}

// Event records ev.
func (r *Recorder) Event(ev Event) (Flow, *Fail) {
	r.Events = append(r.Events, ev)
	return Continue, nil
}

// FnSink is a sink made of a function.
type FnSink func(ev Event) (Flow, *Fail)

// Event calls the function.
func (f FnSink) Event(ev Event) (Flow, *Fail) { return f(ev) }

// CountSink counts events and drops them: the cheapest consumer, for
// measuring a source on its own.
type CountSink struct {
	Events uint64
}

// Event counts ev.
func (c *CountSink) Event(Event) (Flow, *Fail) {
	c.Events++
	return Continue, nil
}

// Replay feeds a recording back into a sink, stopping where the sink
// stops.
func Replay(events []Event, sink Sink) (Flow, *Fail) {
	for _, ev := range events {
		flow, f := sink.Event(ev)
		if f != nil {
			return Continue, f
		}
		if flow == Stop {
			return Stop, nil
		}
	}
	return Continue, nil
}

// TreeContract holds a stream to a tree's events in front of a sink that
// takes them as one (a render that writes a document from them): one
// root value, and in each object a key and then its value, each key
// once. A value walked from a parsed tree keeps it by construction. A
// parse streamed as it proceeds may not: it hands on a member its grammar
// reads twice (JSON's {"a":1,"a":2}) where a tree has one, and the
// rule-event adapter refuses most shapes it cannot follow but not every
// one a grammar can produce. A repeated key in one object is refused with
// DUPLICATE_MEMBER, and events no tree has (a value where a key is due, a
// key outside an object, a close with nothing open, a second root) with
// STREAMABILITY_UNKNOWN, each at the path of the object concerned; the
// event is not passed on.
type TreeContract struct {
	open     []treeOpen
	inner    Sink
	rootDone bool
}

type treeOpen struct {
	object bool
	// keys the object has had, in order (keyList) and as a set.
	keyList []string
	keys    map[string]struct{}
	keyDue  bool
	// next is the index of the element due next, for an array.
	next int
}

// NewTreeContract wraps inner.
func NewTreeContract(inner Sink) *TreeContract { return &TreeContract{inner: inner} }

// Inner is the wrapped sink.
func (t *TreeContract) Inner() Sink { return t.inner }

// path is the path of the value due next: the open containers, each by
// the member or element open in it, and in the innermost object its last
// key when that member's value is due.
func (t *TreeContract) path() Path {
	p := make(Path, 0, len(t.open))
	last := len(t.open) - 1
	for i, o := range t.open {
		if o.object {
			// A container open inside this object is its last key's
			// value, whatever keyDue says: the member was counted as
			// taken when its value opened.
			if (i != last || !o.keyDue) && len(o.keyList) > 0 {
				p = append(p, KeySegment(o.keyList[len(o.keyList)-1]))
			}
		} else {
			p = append(p, IndexSegment(o.next))
		}
	}
	return p
}

func (t *TreeContract) notATree(what string) *Fail {
	return NewFail(CodeStreamabilityUnknown,
		fmt.Sprintf("the stream holds %s, which a tree's events never do, so it is not a tree's", what)).
		AtPath(t.path().String())
}

func (t *TreeContract) closed() {
	if n := len(t.open); n > 0 {
		if !t.open[n-1].object {
			t.open[n-1].next++
		}
		return
	}
	t.rootDone = true
}

// Event checks ev against the tree contract, then passes it on.
func (t *TreeContract) Event(ev Event) (Flow, *Fail) {
	n := len(t.open)
	var top *treeOpen
	if n > 0 {
		top = &t.open[n-1]
	}
	switch ev.Kind {
	case Key:
		switch {
		case top != nil && top.object && top.keyDue:
			if _, seen := top.keys[ev.Text]; seen {
				p := append(t.path(), KeySegment(ev.Text))
				return Continue, NewFail(CodeDuplicateMember, fmt.Sprintf(
					"member %s appears twice in one object, and a tree's events hold each key once",
					strconv.Quote(ev.Text))).AtPath(p.String())
			}
			top.keys[ev.Text] = struct{}{}
			top.keyList = append(top.keyList, ev.Text)
			top.keyDue = false
		case top != nil && top.object:
			return Continue, t.notATree("a key where a value is due")
		default:
			return Continue, t.notATree("a key outside an object")
		}
	case ObjectEnd:
		switch {
		case top != nil && top.object && top.keyDue:
			t.open = t.open[:n-1]
			t.closed()
		case top != nil && top.object:
			return Continue, t.notATree("an object's end where a value is due")
		default:
			return Continue, t.notATree("an object's end where none is due")
		}
	case ArrayEnd:
		if top != nil && !top.object {
			t.open = t.open[:n-1]
			t.closed()
		} else {
			return Continue, t.notATree("an array's end where none is due")
		}
	case End:
		if n > 0 {
			return Continue, t.notATree("its end inside an open container")
		}
	default:
		// A value: in an object, only once its key is in; at the root,
		// only once.
		switch {
		case top != nil && top.object:
			if top.keyDue {
				return Continue, t.notATree("a value where a key is due")
			}
			top.keyDue = true
		case top == nil && t.rootDone:
			return Continue, t.notATree("a second root value")
		}
		switch ev.Kind {
		case ObjectStart:
			t.open = append(t.open, treeOpen{object: true, keys: map[string]struct{}{}, keyDue: true})
		case ArrayStart:
			t.open = append(t.open, treeOpen{})
		default:
			t.closed()
		}
	}
	return t.inner.Event(ev)
}
