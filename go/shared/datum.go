// Copyright (c) 2026 tabnas, MIT License

package shared

import (
	"encoding/json"
	"fmt"
	"io"
	"math"
	"strconv"
	"strings"
)

// DatumKind names one kind of retained value.
type DatumKind uint8

// The kinds of Datum.
const (
	DatumNull DatumKind = iota
	DatumBool
	DatumNumber
	DatumString
	DatumArray
	DatumObject
)

// Member is one member of an object Datum.
type Member struct {
	Key   string
	Value Datum
}

// Datum is a retained JSON-like value: what a capture materializes and
// what a projected cell holds. Distinct from the engine's values on
// purpose: a datum keeps a number's lexeme, measures its own size
// against limits, and is owned by the transducer rather than shared with
// a parse.
//
// Bool is a DatumBool's value; Value, and Lexeme when HasLexeme is set, a
// DatumNumber's (as on Event: HasLexeme false is no lexeme, and an empty
// Lexeme with it set is the empty one); Text a DatumString's; Items a
// DatumArray's; Members a DatumObject's, in source order. A repeated
// member replaces the earlier one in its place (last value wins) unless
// the builder's policy rejected it first.
type Datum struct {
	Kind      DatumKind
	Bool      bool
	HasLexeme bool
	Value     float64
	Lexeme    string
	Text      string
	Items     []Datum
	Members   []Member
}

// Datum constructors.

func NullDatum() Datum            { return Datum{Kind: DatumNull} }
func BoolDatum(b bool) Datum      { return Datum{Kind: DatumBool, Bool: b} }
func NumberDatum(v float64) Datum { return Datum{Kind: DatumNumber, Value: v} }
func StringDatum(s string) Datum  { return Datum{Kind: DatumString, Text: s} }

// NumberDatumLexeme is a number with the source text it was read from;
// the empty text is a lexeme too. NumberDatum is a number without one.
func NumberDatumLexeme(v float64, lexeme string) Datum {
	return Datum{Kind: DatumNumber, HasLexeme: true, Value: v, Lexeme: lexeme}
}

// ArrayDatum is an array of items.
func ArrayDatum(items ...Datum) Datum { return Datum{Kind: DatumArray, Items: items} }

// ObjectDatum is an object of members, in the order given.
func ObjectDatum(members ...Member) Datum { return Datum{Kind: DatumObject, Members: members} }

// ByteSize is payload bytes plus NodeBytes per node: the measure limits
// use.
func (d *Datum) ByteSize() int {
	switch d.Kind {
	case DatumNumber:
		if !d.HasLexeme {
			return NodeBytes + 8
		}
		return NodeBytes + len(d.Lexeme)
	case DatumString:
		return NodeBytes + len(d.Text)
	case DatumArray:
		n := NodeBytes
		for i := range d.Items {
			n += d.Items[i].ByteSize()
		}
		return n
	case DatumObject:
		n := NodeBytes
		for i := range d.Members {
			n += len(d.Members[i].Key) + d.Members[i].Value.ByteSize()
		}
		return n
	}
	return NodeBytes
}

// Get is the member under key, for an object.
func (d *Datum) Get(key string) (*Datum, bool) {
	if d.Kind != DatumObject {
		return nil, false
	}
	for i := range d.Members {
		if d.Members[i].Key == key {
			return &d.Members[i].Value, true
		}
	}
	return nil, false
}

// GetPath is the value at a concrete path below this one.
func (d *Datum) GetPath(path []Segment) (*Datum, bool) {
	here := d
	for _, seg := range path {
		switch {
		case seg.IsIndex && here.Kind == DatumArray:
			if seg.Index < 0 || seg.Index >= len(here.Items) {
				return nil, false
			}
			here = &here.Items[seg.Index]
		case !seg.IsIndex && here.Kind == DatumObject:
			next, ok := here.Get(seg.Key)
			if !ok {
				return nil, false
			}
			here = next
		default:
			return nil, false
		}
	}
	return here, true
}

// TakePath is the value at a concrete path below this one, moved out and
// replaced by null. For a consumer that owns the datum and reads each
// path once: a later read of the same path, or of one below it, finds
// the null, so the caller checks its paths are disjoint first.
func (d *Datum) TakePath(path []Segment) (Datum, bool) {
	here, ok := d.GetPath(path)
	if !ok {
		return Datum{}, false
	}
	out := *here
	*here = NullDatum()
	return out, true
}

// IsContainer reports whether the datum is an array or an object.
func (d *Datum) IsContainer() bool { return d.Kind == DatumArray || d.Kind == DatumObject }

// Equal reports whether two datums are the same value: kinds, scalars,
// lexemes, items and members in order. An empty container equals an
// empty container whatever its slice is.
func (d Datum) Equal(o Datum) bool {
	if d.Kind != o.Kind {
		return false
	}
	switch d.Kind {
	case DatumBool:
		return d.Bool == o.Bool
	case DatumNumber:
		return math.Float64bits(d.Value) == math.Float64bits(o.Value) &&
			d.HasLexeme == o.HasLexeme && d.Lexeme == o.Lexeme
	case DatumString:
		return d.Text == o.Text
	case DatumArray:
		if len(d.Items) != len(o.Items) {
			return false
		}
		for i := range d.Items {
			if !d.Items[i].Equal(o.Items[i]) {
				return false
			}
		}
	case DatumObject:
		if len(d.Members) != len(o.Members) {
			return false
		}
		for i := range d.Members {
			if d.Members[i].Key != o.Members[i].Key || !d.Members[i].Value.Equal(o.Members[i].Value) {
				return false
			}
		}
	}
	return true
}

// String is the datum as compact JSON, keeping number lexemes.
func (d Datum) String() string {
	var b strings.Builder
	WriteJSON(&d, &b)
	return b.String()
}

// DatumFromJSON reads one JSON text into a datum, keeping member order
// and each number's text as its lexeme. For tests and oracles.
func DatumFromJSON(text string) (Datum, error) {
	dec := json.NewDecoder(strings.NewReader(text))
	dec.UseNumber()
	d, err := decodeDatum(dec)
	if err != nil {
		return Datum{}, err
	}
	if _, err := dec.Token(); err != io.EOF {
		return Datum{}, fmt.Errorf("trailing content after a JSON value")
	}
	return d, nil
}

func decodeDatum(dec *json.Decoder) (Datum, error) {
	tok, err := dec.Token()
	if err != nil {
		return Datum{}, err
	}
	switch t := tok.(type) {
	case nil:
		return NullDatum(), nil
	case bool:
		return BoolDatum(t), nil
	case json.Number:
		v, err := strconv.ParseFloat(t.String(), 64)
		if err != nil {
			v = math.NaN()
		}
		return NumberDatumLexeme(v, t.String()), nil
	case string:
		return StringDatum(t), nil
	case json.Delim:
		switch t {
		case '[':
			items := []Datum{}
			for dec.More() {
				item, err := decodeDatum(dec)
				if err != nil {
					return Datum{}, err
				}
				items = append(items, item)
			}
			_, err := dec.Token()
			return ArrayDatum(items...), err
		case '{':
			members := []Member{}
			for dec.More() {
				ktok, err := dec.Token()
				if err != nil {
					return Datum{}, err
				}
				key, _ := ktok.(string)
				value, err := decodeDatum(dec)
				if err != nil {
					return Datum{}, err
				}
				replaced := false
				for i := range members {
					if members[i].Key == key {
						members[i].Value = value
						replaced = true
						break
					}
				}
				if !replaced {
					members = append(members, Member{Key: key, Value: value})
				}
			}
			_, err := dec.Token()
			return ObjectDatum(members...), err
		}
	}
	return Datum{}, fmt.Errorf("unexpected JSON token %v", tok)
}

// writeJSONString appends s as a JSON string literal, escaped as RFC
// 8259 requires: `"` and `\` escaped, control characters as \u00XX
// (lowercase hex) with the short forms for \b \f \n \r \t, everything
// else as itself.
func writeJSONString(s string, out *strings.Builder) {
	out.WriteByte('"')
	start := 0
	for i := 0; i < len(s); i++ {
		c := s[i]
		if c >= 0x20 && c != '"' && c != '\\' {
			continue
		}
		out.WriteString(s[start:i])
		switch c {
		case '"':
			out.WriteString(`\"`)
		case '\\':
			out.WriteString(`\\`)
		case '\b':
			out.WriteString(`\b`)
		case '\f':
			out.WriteString(`\f`)
		case '\n':
			out.WriteString(`\n`)
		case '\r':
			out.WriteString(`\r`)
		case '\t':
			out.WriteString(`\t`)
		default:
			fmt.Fprintf(out, `\u%04x`, c)
		}
		start = i + 1
	}
	out.WriteString(s[start:])
	out.WriteByte('"')
}

// WriteJSONString appends s as an RFC 8259 JSON string literal.
func WriteJSONString(s string, out *strings.Builder) { writeJSONString(s, out) }

// WriteJSONNumber appends a number: its lexeme as it stands when it has
// one (hasLexeme), else the shortest text that reads back as the same
// float64, written without an exponent as Rust's f64 Display writes it. A
// non-finite value has no JSON form and is written as null; renderers
// reject it before it gets here.
func WriteJSONNumber(value float64, lexeme string, hasLexeme bool, out *strings.Builder) {
	switch {
	case hasLexeme:
		out.WriteString(lexeme)
	case math.IsInf(value, 0) || math.IsNaN(value):
		out.WriteString("null")
	default:
		out.WriteString(formatFloat(value))
	}
}

// WriteJSON appends a datum as compact JSON.
func WriteJSON(d *Datum, out *strings.Builder) {
	switch d.Kind {
	case DatumNull:
		out.WriteString("null")
	case DatumBool:
		if d.Bool {
			out.WriteString("true")
		} else {
			out.WriteString("false")
		}
	case DatumNumber:
		WriteJSONNumber(d.Value, d.Lexeme, d.HasLexeme, out)
	case DatumString:
		writeJSONString(d.Text, out)
	case DatumArray:
		out.WriteByte('[')
		for i := range d.Items {
			if i > 0 {
				out.WriteByte(',')
			}
			WriteJSON(&d.Items[i], out)
		}
		out.WriteByte(']')
	case DatumObject:
		out.WriteByte('{')
		for i := range d.Members {
			if i > 0 {
				out.WriteByte(',')
			}
			writeJSONString(d.Members[i].Key, out)
			out.WriteByte(':')
			WriteJSON(&d.Members[i].Value, out)
		}
		out.WriteByte('}')
	}
}

// WalkDatum emits a datum as JsonEvents/1, without the final End, so a
// datum can stand in for any part of a document.
func WalkDatum(d *Datum, sink Sink) (Flow, *Fail) {
	send := func(ev Event) (bool, *Fail) {
		flow, f := sink.Event(ev)
		return f == nil && flow == Continue, f
	}
	var ok bool
	var f *Fail
	switch d.Kind {
	case DatumNull:
		ok, f = send(EvNull())
	case DatumBool:
		ok, f = send(EvBool(d.Bool))
	case DatumNumber:
		ok, f = send(Event{Kind: Number, HasLexeme: d.HasLexeme, Value: d.Value, Lexeme: d.Lexeme})
	case DatumString:
		ok, f = send(EvString(d.Text))
	case DatumArray:
		if ok, f = send(EvArrayStart()); !ok {
			break
		}
		for i := range d.Items {
			flow, f := WalkDatum(&d.Items[i], sink)
			if f != nil || flow == Stop {
				return flow, f
			}
		}
		ok, f = send(EvArrayEnd())
	case DatumObject:
		if ok, f = send(EvObjectStart()); !ok {
			break
		}
		for i := range d.Members {
			if ok, f = send(EvKey(d.Members[i].Key)); !ok {
				break
			}
			flow, f := WalkDatum(&d.Members[i].Value, sink)
			if f != nil || flow == Stop {
				return flow, f
			}
		}
		if ok {
			ok, f = send(EvObjectEnd())
		}
	}
	if f != nil {
		return Continue, f
	}
	if !ok {
		return Stop, nil
	}
	return Continue, nil
}

// Duplicates is how a builder treats a repeated member name.
type Duplicates uint8

const (
	// Reject fails with DUPLICATE_MEMBER.
	Reject Duplicates = iota
	// LastWins: the later value replaces the earlier one, in its place.
	LastWins
	// FirstWins: the earlier value stays.
	FirstWins
)

// String is the policy's name as the shared fixtures spell it.
func (d Duplicates) String() string {
	switch d {
	case LastWins:
		return "last_wins"
	case FirstWins:
		return "first_wins"
	}
	return "reject"
}

// DatumBuilder builds one Datum from the events of one value, under a
// byte limit.
//
// Feed it every event from the value's first to its last; Finished says
// when the value is complete. Bytes are counted as they arrive, and the
// limit fails at the first byte over it rather than after the value is
// whole. The builder knows nothing of where in a document its value
// sits: a failure leaves the path unset and the stage that placed the
// builder adds it.
type DatumBuilder struct {
	stack      []builderFrame
	done       *Datum
	bytes      int
	limit      int
	limitName  string
	duplicates Duplicates
}

type builderFrame struct {
	array   bool
	items   []Datum
	members []Member
	// index finds a member by name once the object is large enough that
	// a scan per key would cost more than the map.
	index  map[string]int
	key    string
	hasKey bool
}

// indexAt is how many members an object holds before the builder keeps
// an index of their names.
const indexAt = 16

func (fr *builderFrame) find(key string) int {
	if fr.index != nil {
		if i, ok := fr.index[key]; ok {
			return i
		}
		return -1
	}
	for i := range fr.members {
		if fr.members[i].Key == key {
			return i
		}
	}
	return -1
}

func (fr *builderFrame) add(key string, value Datum) {
	fr.members = append(fr.members, Member{Key: key, Value: value})
	switch {
	case fr.index != nil:
		fr.index[key] = len(fr.members) - 1
	case len(fr.members) >= indexAt:
		fr.index = make(map[string]int, len(fr.members)*2)
		for i := range fr.members {
			fr.index[fr.members[i].Key] = i
		}
	}
}

// NewDatumBuilder is a builder whose limit failure names limitName (a
// Limits field, as max_capture_bytes).
func NewDatumBuilder(limit int, limitName string, duplicates Duplicates) *DatumBuilder {
	return &DatumBuilder{limit: limit, limitName: limitName, duplicates: duplicates}
}

// Bytes is what the builder holds now, on the limits' measure.
func (b *DatumBuilder) Bytes() int { return b.bytes }

// Finished reports whether the value is complete.
func (b *DatumBuilder) Finished() bool { return b.done != nil }

// Take is the value, once Finished. Probing an unfinished builder
// returns false and leaves the charge for the partial value in place:
// the bytes it holds are still held.
func (b *DatumBuilder) Take() (Datum, bool) {
	if b.done == nil {
		return Datum{}, false
	}
	d := *b.done
	b.done = nil
	b.bytes = 0
	return d, true
}

func (b *DatumBuilder) charge(n int) *Fail {
	b.bytes += n
	if b.bytes > b.limit {
		return LimitFail(b.limitName, uint64(b.limit), fmt.Sprintf("a value is larger than %d bytes", b.limit))
	}
	return nil
}

func (b *DatumBuilder) release(n int) {
	b.bytes -= n
	if b.bytes < 0 {
		b.bytes = 0
	}
}

func (b *DatumBuilder) place(value Datum) *Fail {
	if len(b.stack) == 0 {
		b.done = &value
		return nil
	}
	top := &b.stack[len(b.stack)-1]
	if top.array {
		top.items = append(top.items, value)
		return nil
	}
	if !top.hasKey {
		return ProtocolFail("a value arrived inside an object without a key")
	}
	key := top.key
	top.hasKey = false
	top.key = ""
	// A repeated member was charged in full as its events arrived, which
	// is right: for a moment both were held. Whichever one goes now gives
	// its bytes back, so an object that keeps repeating a small key does
	// not grow towards the limit while the value it holds stays the same
	// size. A completed value's ByteSize is exactly what its events were
	// charged.
	if i := top.find(key); i >= 0 {
		switch b.duplicates {
		case Reject:
			return NewFail(CodeDuplicateMember, fmt.Sprintf("member %s appears twice", strconv.Quote(key)))
		case FirstWins:
			b.release(len(key) + value.ByteSize())
			return nil
		default:
			released := len(key) + top.members[i].Value.ByteSize()
			top.members[i].Value = value
			b.release(released)
			return nil
		}
	}
	top.add(key, value)
	return nil
}

// Event takes one event of the value being built.
func (b *DatumBuilder) Event(ev Event) *Fail {
	if b.done != nil {
		return ProtocolFail("an event arrived after the value was complete")
	}
	switch ev.Kind {
	case ObjectStart:
		if f := b.charge(NodeBytes); f != nil {
			return f
		}
		b.stack = append(b.stack, builderFrame{members: []Member{}})
	case ArrayStart:
		if f := b.charge(NodeBytes); f != nil {
			return f
		}
		b.stack = append(b.stack, builderFrame{array: true, items: []Datum{}})
	case Key:
		if f := b.charge(len(ev.Text)); f != nil {
			return f
		}
		if n := len(b.stack); n > 0 && !b.stack[n-1].array && !b.stack[n-1].hasKey {
			b.stack[n-1].key = ev.Text
			b.stack[n-1].hasKey = true
		} else {
			return ProtocolFail("a key arrived where no member was expected")
		}
	case ObjectEnd:
		n := len(b.stack)
		if n == 0 || b.stack[n-1].array {
			return ProtocolFail("an object ended that had not started")
		}
		if b.stack[n-1].hasKey {
			return ProtocolFail("an object ended after a key without its value")
		}
		fr := b.stack[n-1]
		b.stack = b.stack[:n-1]
		return b.place(ObjectDatum(fr.members...))
	case ArrayEnd:
		n := len(b.stack)
		if n == 0 || !b.stack[n-1].array {
			return ProtocolFail("an array ended that had not started")
		}
		fr := b.stack[n-1]
		b.stack = b.stack[:n-1]
		return b.place(ArrayDatum(fr.items...))
	case Null:
		if f := b.charge(NodeBytes); f != nil {
			return f
		}
		return b.place(NullDatum())
	case Bool:
		if f := b.charge(NodeBytes); f != nil {
			return f
		}
		return b.place(BoolDatum(ev.Bool))
	case Number:
		n := 8
		if ev.HasLexeme {
			n = len(ev.Lexeme)
		}
		if f := b.charge(NodeBytes + n); f != nil {
			return f
		}
		return b.place(Datum{Kind: DatumNumber, HasLexeme: ev.HasLexeme, Value: ev.Value, Lexeme: ev.Lexeme})
	case String:
		if f := b.charge(NodeBytes + len(ev.Text)); f != nil {
			return f
		}
		return b.place(StringDatum(ev.Text))
	case End:
		return ProtocolFail("the document ended inside a value being captured")
	}
	return nil
}
