// Copyright (c) 2026 tabnas, MIT License

package shared

import (
	"encoding/json"
	"fmt"
	"math"
	"strconv"
	"strings"
)

// parser_event.go: ParserEvents/1, the parser's rule passes, as a
// transducer part reads them (rs/src/shared/parser_event.rs).
//
// A source over a tabnas parse (transduce's parser-event source) relays
// each rule pass the engine reports to its rule-done subscribers as one
// ParserEvent: an Open when a rule's open pass has run, a Close when its
// close pass has, and one End after the last pass, only once the engine
// has returned the parse's value, so a document is complete only when it
// validated. A pass made inside a probe (the engine's RuleDone.Probe) is
// speculative, and the source drops it.
//
// Containers never travel in parser events: a rule's node is handed on
// only as a scalar (RulePass.Value), so a transducer that wants a
// container emits its start and end from the passes that open and close
// it, and nothing in the stream grows with the document.
//
// The fixture encoding (ParserEvent.JSON) follows the JSON events'
// encoding: an array whose first element names the event, `["open",
// {"name": "map", "d": 1, "i": 3, "alt": {"p": "pair", "r": "", "b": 0,
// "g": ["map", "json"]}, "tokens": [["#OB", "{", "{", 1, 1]], "forced":
// false, "n": {"pk": 0}}]`, and `["end"]`. A field that is absent (Alt when
// the state declared no alternates, Value when the node is not a scalar or
// the pass is an open) is left out, so `"value": null` is always a null
// scalar.

// ParserEventKind is which item of ParserEvents/1 an event is.
type ParserEventKind uint8

// The kinds.
const (
	// ParserOpen: a rule's open pass completed.
	ParserOpen ParserEventKind = iota
	// ParserClose: a rule's close pass completed.
	ParserClose
	// ParserEnd: the parse returned its value. Exactly one, after the last
	// pass.
	ParserEnd
)

// String is the event's name in the encoding: open, close or end.
func (k ParserEventKind) String() string {
	switch k {
	case ParserOpen:
		return "open"
	case ParserClose:
		return "close"
	}
	return "end"
}

// ParserEvent is one item of ParserEvents/1; Pass is an Open's or a
// Close's.
type ParserEvent struct {
	Kind ParserEventKind
	Pass *RulePass
}

// RulePass is what one rule pass reports.
type RulePass struct {
	// Name is the rule's name (map, pair, val).
	Name string
	// D is the rule's depth on the stack: one more than the rule that
	// pushed it, the same as the rule it replaced.
	D uint64
	// I is the rule's instance number in the parse.
	I uint64
	// Alt is the alternate that matched; nil when the state declared none.
	Alt *PassAlt
	// Tokens are the tokens the pass consumed.
	Tokens []PassToken
	// Forced is a close synthesized by recovery.
	Forced bool
	// N is the rule's named counters, by name.
	N []PassCounter
	// Value is the node's value, when it is a scalar at a close pass; nil
	// otherwise.
	Value *Datum
}

// PassAlt is the alternate a pass matched.
type PassAlt struct {
	// P is the rule it pushed, or empty; R the rule it replaced the
	// current one with, or empty; B how many tokens it gave back; G its
	// groups.
	P, R string
	B    uint64
	G    []string
}

// PassToken is a token a pass consumed.
type PassToken struct {
	// Name is the token's name (#OB, #ST, #NR); Val its value (a string's
	// contents, a number's value); Src the source text it was lexed from.
	Name     string
	Val      Datum
	Src      string
	Row, Col uint64
}

// PassCounter is one of a rule's named counters.
type PassCounter struct {
	Name  string
	Value int64
}

// ParserSink consumes ParserEvents/1: what a transducer part's head is,
// and what a source pushes each rule pass into, synchronously, on the
// goroutine that parses. As with Sink, a non-nil *Fail aborts the run and
// Stop asks the source to stop the parse.
type ParserSink interface {
	ParserEvent(ev ParserEvent) (Flow, *Fail)
}

// JSON is the fixture encoding: `["open", {...}]`, `["close", {...}]` or
// `["end"]`.
func (e ParserEvent) JSON() string {
	var b strings.Builder
	b.WriteString(`["`)
	b.WriteString(e.Kind.String())
	b.WriteByte('"')
	if e.Kind == ParserEnd || e.Pass == nil {
		b.WriteByte(']')
		return b.String()
	}
	p := e.Pass
	b.WriteString(`,{"name":`)
	WriteJSONString(p.Name, &b)
	b.WriteString(`,"d":` + strconv.FormatUint(p.D, 10))
	b.WriteString(`,"i":` + strconv.FormatUint(p.I, 10))
	if p.Alt != nil {
		b.WriteString(`,"alt":{"p":`)
		WriteJSONString(p.Alt.P, &b)
		b.WriteString(`,"r":`)
		WriteJSONString(p.Alt.R, &b)
		b.WriteString(`,"b":` + strconv.FormatUint(p.Alt.B, 10) + `,"g":[`)
		for i, g := range p.Alt.G {
			if i > 0 {
				b.WriteByte(',')
			}
			WriteJSONString(g, &b)
		}
		b.WriteString("]}")
	}
	b.WriteString(`,"tokens":[`)
	for i := range p.Tokens {
		t := &p.Tokens[i]
		if i > 0 {
			b.WriteByte(',')
		}
		b.WriteByte('[')
		WriteJSONString(t.Name, &b)
		b.WriteByte(',')
		WriteJSON(&t.Val, &b)
		b.WriteByte(',')
		WriteJSONString(t.Src, &b)
		b.WriteString("," + strconv.FormatUint(t.Row, 10) + "," + strconv.FormatUint(t.Col, 10) + "]")
	}
	b.WriteString(`],"forced":` + strconv.FormatBool(p.Forced) + `,"n":{`)
	for i, c := range p.N {
		if i > 0 {
			b.WriteByte(',')
		}
		WriteJSONString(c.Name, &b)
		b.WriteString(":" + strconv.FormatInt(c.Value, 10))
	}
	b.WriteByte('}')
	if p.Value != nil {
		b.WriteString(`,"value":`)
		WriteJSON(p.Value, &b)
	}
	b.WriteString("}]")
	return b.String()
}

// ParserEventFromJSON reads an event from its fixture encoding, or says
// what is wrong with it.
func ParserEventFromJSON(text string) (ParserEvent, error) {
	wrong := func(what string) (ParserEvent, error) {
		return ParserEvent{}, fmt.Errorf("%s: %s", what, text)
	}
	var items []json.RawMessage
	if err := json.Unmarshal([]byte(text), &items); err != nil {
		return wrong("a parser event is an array")
	}
	var name string
	if len(items) == 0 || json.Unmarshal(items[0], &name) != nil {
		return wrong("a parser event starts with its name")
	}
	var kind ParserEventKind
	switch {
	case name == "end" && len(items) == 1:
		return ParserEvent{Kind: ParserEnd}, nil
	case name == "open" && len(items) == 2:
		kind = ParserOpen
	case name == "close" && len(items) == 2:
		kind = ParserClose
	default:
		return wrong("not a parser event")
	}
	var rule map[string]json.RawMessage
	if err := json.Unmarshal(items[1], &rule); err != nil || rule == nil {
		return wrong("a rule pass is an object")
	}
	text1 := func(raw json.RawMessage, key string) (string, error) {
		var s string
		if raw == nil || json.Unmarshal(raw, &s) != nil {
			return "", fmt.Errorf("a rule pass's :%s is a string: %s", key, text)
		}
		return s, nil
	}
	count := func(raw json.RawMessage, key string) (uint64, error) {
		var n uint64
		if raw == nil || json.Unmarshal(raw, &n) != nil {
			return 0, fmt.Errorf("a rule pass's :%s is a count: %s", key, text)
		}
		return n, nil
	}
	pass := &RulePass{}
	var err error
	if pass.Name, err = text1(rule["name"], "name"); err != nil {
		return ParserEvent{}, err
	}
	if pass.D, err = count(rule["d"], "d"); err != nil {
		return ParserEvent{}, err
	}
	if pass.I, err = count(rule["i"], "i"); err != nil {
		return ParserEvent{}, err
	}
	if raw, ok := rule["alt"]; ok {
		var alt map[string]json.RawMessage
		if json.Unmarshal(raw, &alt) != nil || alt == nil {
			return wrong("an alternate is an object")
		}
		a := &PassAlt{}
		if a.P, err = text1(alt["p"], "p"); err != nil {
			return ParserEvent{}, err
		}
		if a.R, err = text1(alt["r"], "r"); err != nil {
			return ParserEvent{}, err
		}
		if a.B, err = count(alt["b"], "b"); err != nil {
			return ParserEvent{}, err
		}
		if json.Unmarshal(alt["g"], &a.G) != nil || a.G == nil {
			return wrong("an alternate's :g is a vector of strings")
		}
		pass.Alt = a
	}
	var tokens []json.RawMessage
	if json.Unmarshal(rule["tokens"], &tokens) != nil || tokens == nil {
		return wrong("a rule pass's :tokens is a vector")
	}
	for _, raw := range tokens {
		var parts []json.RawMessage
		if json.Unmarshal(raw, &parts) != nil || len(parts) != 5 {
			return wrong("a token is [name, val, src, row, col]")
		}
		var t PassToken
		val, verr := DatumFromJSON(string(parts[1]))
		if json.Unmarshal(parts[0], &t.Name) != nil || verr != nil ||
			json.Unmarshal(parts[2], &t.Src) != nil ||
			json.Unmarshal(parts[3], &t.Row) != nil || json.Unmarshal(parts[4], &t.Col) != nil {
			return wrong("a token is [name, val, src, row, col]")
		}
		t.Val = val
		pass.Tokens = append(pass.Tokens, t)
	}
	if pass.Tokens == nil {
		pass.Tokens = []PassToken{}
	}
	n, nerr := DatumFromJSON(string(rule["n"]))
	if rule["n"] == nil || nerr != nil || n.Kind != DatumObject {
		return wrong("a rule pass's :n is a record")
	}
	for _, m := range n.Members {
		v := m.Value
		if v.Kind != DatumNumber || v.Value != math.Trunc(v.Value) || strings.ContainsAny(v.Lexeme, ".eE") {
			return wrong("a rule pass's counters are integers")
		}
		pass.N = append(pass.N, PassCounter{Name: m.Key, Value: int64(v.Value)})
	}
	if json.Unmarshal(rule["forced"], &pass.Forced) != nil {
		return wrong("a rule pass's :forced is a boolean")
	}
	if raw, ok := rule["value"]; ok {
		v, verr := DatumFromJSON(string(raw))
		if verr != nil {
			return wrong("a rule pass's :value is a scalar")
		}
		pass.Value = &v
	}
	return ParserEvent{Kind: kind, Pass: pass}, nil
}
