// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"fmt"
	"math"
	"strconv"
	"strings"

	"github.com/tabnas/alchemy/go/shared"
)

// value.go: runtime values (rs/src/value.rs; design brief section 4.3,
// spec sections 10.2 and 10.3).
//
// A Val is cheap to copy: vectors, records, tagged values, selectors,
// captures, functions and plans are shared behind a pointer, and nothing
// here is changed after it is built, so a value is shared freely between
// the stages of a run.
//
// Streams and texts are plans. Evaluating `route`, `scan-emit`, `map` over
// a stream, `concat`, `join` or `csv` builds a Plan and consumes nothing;
// the runtime lowers the plan to a chain of transduce and render sinks
// once, when the host asks for the program's sink (lower.go). A plan is
// live when it reaches the host's input (PlanInput); a finite text (a
// literal, a join over a vector) is a plan too, written synchronously
// wherever it lands.
//
// A transduce Datum converts to a Val and back, so `get`, `get-path`,
// `as-vector` and `as-path` work on captured JSON values and on a
// program's own records alike. A value the source had no member for is the
// constant tagged value `missing` (Missing), distinct from `null`, as the
// spec asks (section 18.3); a policy maps it later.

// MissingTag is the tag of the constant tagged value a `get` or `get-path`
// answers for an absent member.
const MissingTag = "missing"

// Val is one runtime value: NullVal, BoolVal, NumVal, StrVal, KeywordVal,
// *VectorVal, *RecordVal, a function (Fn: *Closure, *Native, *Partial),
// *SelectorVal, *CaptureVal, *TaggedVal, StreamVal or TextVal.
type Val interface{ isVal() }

// NullVal is `null`.
type NullVal struct{}

// BoolVal is a boolean.
type BoolVal bool

// NumVal is a number as the source or the program spelled it, when known:
// a renderer writes the lexeme, so `50.25` stays `50.25`. HasLexeme says
// whether there is one, as on transduce's events.
type NumVal struct {
	Value     float64
	Lexeme    string
	HasLexeme bool
}

// StrVal is a string.
type StrVal string

// KeywordVal is a keyword, by its name.
type KeywordVal string

// VectorVal is a retained vector.
type VectorVal struct {
	Items []Val
}

// RecordVal is a retained record: its fields in construction order, the
// keys the keyword names.
type RecordVal struct {
	keys  []string
	vals  []Val
	index map[string]int
}

// SelectorVal is a selector.
type SelectorVal struct {
	Selector shared.Selector
}

// CaptureVal is a capture spec, what `capture` makes for a `route`.
type CaptureVal struct {
	Spec shared.CaptureSpec
}

// TaggedVal is a constructor's value: `schema`, `row`, `table-end`,
// `no-schema`, `ready`, `selected`, `transition`, `entry`, `missing`, and
// the events `events` delivers (`object-start`, `object-end`,
// `array-start`, `array-end`, `key`, `scalar`).
type TaggedVal struct {
	Tag    string
	Fields []Val
}

// StreamVal is a single-use stream: a plan producing items, table events
// or JSON events.
type StreamVal struct {
	Plan *Plan
}

// TextVal is a single-use text: a plan producing fragments.
type TextVal struct {
	Plan *Plan
}

func (NullVal) isVal()      {}
func (BoolVal) isVal()      {}
func (NumVal) isVal()       {}
func (StrVal) isVal()       {}
func (KeywordVal) isVal()   {}
func (*VectorVal) isVal()   {}
func (*RecordVal) isVal()   {}
func (*SelectorVal) isVal() {}
func (*CaptureVal) isVal()  {}
func (*TaggedVal) isVal()   {}
func (StreamVal) isVal()    {}
func (TextVal) isVal()      {}
func (*Closure) isVal()     {}
func (*Native) isVal()      {}
func (*Partial) isVal()     {}

// Num is a number without a lexeme.
func Num(value float64) NumVal { return NumVal{Value: value} }

// NumLexeme is a number with the text it was spelled with.
func NumLexeme(value float64, lexeme string) NumVal {
	return NumVal{Value: value, Lexeme: lexeme, HasLexeme: true}
}

// Vector is a vector of items.
func Vector(items ...Val) *VectorVal {
	if items == nil {
		items = []Val{}
	}
	return &VectorVal{Items: items}
}

// NewTagged is a tagged value.
func NewTagged(tag string, fields ...Val) *TaggedVal {
	if fields == nil {
		fields = []Val{}
	}
	return &TaggedVal{Tag: tag, Fields: fields}
}

// Missing is the constant a lookup answers for an absent member.
func Missing() *TaggedVal { return NewTagged(MissingTag) }

// IsMissing reports whether v is `missing`.
func IsMissing(v Val) bool {
	t, ok := v.(*TaggedVal)
	return ok && t.Tag == MissingTag && len(t.Fields) == 0
}

// recordIndexAt is how many fields a record holds before it keeps an index
// of their keys, rather than finding a key by a scan.
const recordIndexAt = 16

// NewRecord is an empty record.
func NewRecord() *RecordVal { return &RecordVal{} }

// Len is how many fields the record holds.
func (r *RecordVal) Len() int { return len(r.keys) }

// Keys are the record's keys, in its order; the caller must not change
// them.
func (r *RecordVal) Keys() []string { return r.keys }

// Values are the record's values, in its order; the caller must not
// change them.
func (r *RecordVal) Values() []Val { return r.vals }

func (r *RecordVal) find(key string) int {
	if r.index != nil {
		if i, ok := r.index[key]; ok {
			return i
		}
		return -1
	}
	for i, k := range r.keys {
		if k == key {
			return i
		}
	}
	return -1
}

// Get is the field key, and whether the record has one.
func (r *RecordVal) Get(key string) (Val, bool) {
	if i := r.find(key); i >= 0 {
		return r.vals[i], true
	}
	return nil, false
}

// Set sets the field key, in its place when the record has one already
// (the value replaced, as an IndexMap insert does), and reports whether it
// did. A record is built with Set and never changed after it is shared.
func (r *RecordVal) Set(key string, value Val) (replaced bool) {
	if i := r.find(key); i >= 0 {
		r.vals[i] = value
		return true
	}
	r.keys = append(r.keys, key)
	r.vals = append(r.vals, value)
	switch {
	case r.index != nil:
		r.index[key] = len(r.keys) - 1
	case len(r.keys) >= recordIndexAt:
		r.index = make(map[string]int, len(r.keys)*2)
		for i, k := range r.keys {
			r.index[k] = i
		}
	}
	return false
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

// Fn is a function value: a *Closure, a *Native or a *Partial.
type Fn interface {
	Val
	// Takes is the arguments the function still takes, when that is fixed.
	Takes() (int, bool)
	// Describe is a name for messages.
	Describe() string
}

// Scope is which global scope a closure's free names resolve in: a
// standard library definition sees the library and the natives, a
// program's definition sees the program first. Lexical, so a program's
// `csv-row` never hijacks the library's `csv`.
type Scope uint8

// The two scopes.
const (
	ScopeProgram Scope = iota
	ScopeStdlib
)

// Closure is a `fn` value: its parameters, its body and the environment it
// closed over. Name is the def it came from, for messages, "" for none.
type Closure struct {
	Name   string
	Params []string
	Body   *Expr
	Env    Env
	Scope  Scope
	Span   SourceSpan
}

// Partial is `partial f a b`: F with its first arguments supplied.
type Partial struct {
	F    Fn
	Args []Val
}

// Takes is the closure's parameter count.
func (c *Closure) Takes() (int, bool) { return len(c.Params), true }

// Describe is `fn name`, or `fn [params]` for an anonymous fn.
func (c *Closure) Describe() string {
	if c.Name != "" {
		return "fn " + c.Name
	}
	return "fn [" + strings.Join(c.Params, " ") + "]"
}

// Takes is the native's argument count, when it is exact.
func (n *Native) Takes() (int, bool) { return n.Arity.Exact() }

// Describe is the native's name.
func (n *Native) Describe() string { return n.Name }

// Takes is what the partial still takes.
func (p *Partial) Takes() (int, bool) {
	n, ok := p.F.Takes()
	if !ok {
		return 0, false
	}
	if n < len(p.Args) {
		return 0, true
	}
	return n - len(p.Args), true
}

// Describe is `partial f`.
func (p *Partial) Describe() string { return "partial " + p.F.Describe() }

// ---------------------------------------------------------------------------
// Environments
// ---------------------------------------------------------------------------

// Env is the local environment: a persistent chain of bindings, shared by
// the closures that captured it. The zero Env is empty.
type Env struct {
	frame *envFrame
}

type envFrame struct {
	name  string
	value Val
	next  Env
}

// Bind is this environment with one more binding, innermost.
func (e Env) Bind(name string, value Val) Env {
	return Env{frame: &envFrame{name: name, value: value, next: e}}
}

// Get is the innermost binding of name.
func (e Env) Get(name string) (Val, bool) {
	for here := e.frame; here != nil; here = here.next.frame {
		if here.name == name {
			return here.value, true
		}
	}
	return nil, false
}

// Has reports whether name is bound here.
func (e Env) Has(name string) bool {
	_, ok := e.Get(name)
	return ok
}

// Frame is the innermost binding's value and the environment outside it;
// ok is false for the empty environment.
func (e Env) Frame() (value Val, next Env, ok bool) {
	if e.frame == nil {
		return nil, Env{}, false
	}
	return e.frame.value, e.frame.next, true
}

// String names the bindings, innermost first.
func (e Env) String() string {
	var names []string
	for here := e.frame; here != nil; here = here.next.frame {
		names = append(names, here.name)
	}
	return "Env[" + strings.Join(names, " ") + "]"
}

// ---------------------------------------------------------------------------
// Plans
// ---------------------------------------------------------------------------

// Seq is a finite or single-use sequence an operator was given: a vector's
// items, or a stream (Stream non-nil).
type Seq struct {
	Vector []Val
	Stream *Plan
}

// IsStream reports whether the sequence is a stream.
func (s Seq) IsStream() bool { return s.Stream != nil }

// PlanKind names one kind of plan.
type PlanKind uint8

// The plans, as the Rust Plan enum names them.
const (
	// PlanInput is the host's JsonEvents/1.
	PlanInput PlanKind = iota
	// PlanRoute is `route specs input`: a stream of `selected` values.
	PlanRoute
	// PlanSelect is `select selector input`: a stream of the selected
	// values.
	PlanSelect
	// PlanEvents is `events input`: every event of JsonEvents/1 as one
	// tagged item; End is the stream's end, not an item.
	PlanEvents
	// PlanAsEvents is `as-events items`: a stream of items the program
	// built, each an event, as JsonEvents/1: the reverse of `events`,
	// which any taker of JSON events already applies to such a stream;
	// here the program says so, where the checker could not tell the
	// items' type.
	PlanAsEvents
	// PlanScanEmit is `scan-emit init step finish stream`.
	PlanScanEmit
	PlanMap
	PlanFilter
	// PlanTableFromJSON is the standard table transducer, run natively:
	// TableRows/1 from JsonEvents/1.
	PlanTableFromJSON
	// PlanRecords is `records table-events`: JsonEvents/1 from TableRows/1.
	PlanRecords
	// PlanCsvTable is `csv-table options events`: the tagged table events,
	// validated as the CSV renderer validates them, passed on unchanged.
	PlanCsvTable
	// PlanLit is a literal text.
	PlanLit
	// PlanConcat is `concat items...`: each a string or a text.
	PlanConcat
	// PlanJoin is `join separator items`.
	PlanJoin
	// PlanConcatMap is `concat-map f items`: f answers a string or a text
	// per item.
	PlanConcatMap
	// PlanReplace is `replace-text from to text`: the text is a string or a
	// text.
	PlanReplace
	// PlanCsv is the standard CSV renderer, run natively.
	PlanCsv
	// PlanJSON is `json events`, or `json options events`.
	PlanJSON
)

// NonFinite is what a renderer does with a number that is not finite
// (infinity, negative infinity or NaN), which JSON has no spelling for and
// CSV no type: the :non-finite option of `json` and of a CSV options
// record.
type NonFinite uint8

// The policies.
const (
	// NonFiniteReject is :reject, the default:
	// TARGET_VALUE_UNREPRESENTABLE.
	NonFiniteReject NonFinite = iota
	// NonFiniteNull is :null: written as null, `null` in JSON and the null
	// text in CSV.
	NonFiniteNull
	// NonFiniteLiteral is :literal (CSV only): the cell's text is the word
	// Infinity, -Infinity or NaN.
	NonFiniteLiteral
)

// NonFiniteNamed is the policy a keyword names.
func NonFiniteNamed(name string) (NonFinite, bool) {
	switch name {
	case "reject":
		return NonFiniteReject, true
	case "null":
		return NonFiniteNull, true
	case "literal":
		return NonFiniteLiteral, true
	}
	return NonFiniteReject, false
}

// NonFiniteWord is the word :literal writes for a number that is not
// finite.
func NonFiniteWord(value float64) string {
	switch {
	case math.IsNaN(value):
		return "NaN"
	case value > 0:
		return "Infinity"
	}
	return "-Infinity"
}

// Plan is a stream or a text, as a description of how to produce it. Kind
// says which fields apply:
//
//   - Source: the plan a stage reads (route, select, events, as-events,
//     scan-emit, map, filter, table-from-json, records, csv-table, csv,
//     json);
//   - Specs: a route's captures; Selector: a select's;
//   - Init, Step and Finish: a scan-emit's; F: a map's, a filter's or a
//     concat-map's function; At: the form a scan-emit, map, filter,
//     concat-map or native table was built at, for diagnostics;
//   - Binding: the native table's; Options: a csv-table's or a csv's;
//   - NonFinite: a json's policy for a number that is not finite;
//   - Lit: a literal's text;
//   - Items and Live: a concat's items and the position of the one item
//     that reaches the input (-1 for none), found once when the plan is
//     built (newConcat), so asking whether a concat is live never walks
//     its items again: a program that nests concat over shared
//     definitions builds a plan whose items, unshared, are exponentially
//     many;
//   - Sep and Seq: a join's; Seq: a concat-map's items;
//   - From, To and Text: a replace-text's literal, its replacement and
//     the string or text it reads.
type Plan struct {
	Kind      PlanKind
	Source    *Plan
	Specs     []shared.CaptureSpec
	Selector  shared.Selector
	Init      Val
	Step      Fn
	Finish    Fn
	F         Fn
	At        SourceSpan
	Binding   Val
	Options   Val
	NonFinite NonFinite
	Lit       string
	Items     []Val
	Live      int
	Sep       string
	Seq       Seq
	From      string
	To        string
	Text      Val
}

// inputPlan is the host's input.
func inputPlan() *Plan { return &Plan{Kind: PlanInput} }

// litPlan is a literal text.
func litPlan(s string) *Plan { return &Plan{Kind: PlanLit, Lit: s} }

// newConcat is a concat of items, with its live item found once.
func newConcat(items []Val) *Plan {
	live := -1
	for i, item := range items {
		if IsLive(item) {
			live = i
			break
		}
	}
	return &Plan{Kind: PlanConcat, Items: items, Live: live}
}

// IsLive reports whether the plan reaches the host's input: a live plan
// runs as the events arrive; a plan that does not is finite and is written
// whole.
func (p *Plan) IsLive() bool {
	switch p.Kind {
	case PlanInput:
		return true
	case PlanLit:
		return false
	case PlanConcat:
		return p.Live >= 0
	case PlanJoin, PlanConcatMap:
		return p.Seq.IsStream()
	case PlanReplace:
		return IsLive(p.Text)
	}
	return p.Source.IsLive()
}

// IsText reports whether the plan is a text rather than a stream.
func (p *Plan) IsText() bool {
	switch p.Kind {
	case PlanLit, PlanConcat, PlanJoin, PlanConcatMap, PlanReplace, PlanCsv, PlanJSON:
		return true
	}
	return false
}

// Protocol is what a plan produces.
type Protocol uint8

// The protocols.
const (
	ProtocolJSONEvents Protocol = iota
	ProtocolTableRows
	// ProtocolItems is a stream of values whose shape the runtime learns
	// item by item.
	ProtocolItems
	ProtocolText
)

// Protocol is the protocol the plan produces, when it is a stream:
// JsonEvents, TableRows (natively), or Items.
func (p *Plan) Protocol() Protocol {
	switch p.Kind {
	case PlanInput, PlanRecords, PlanAsEvents:
		return ProtocolJSONEvents
	case PlanTableFromJSON:
		return ProtocolTableRows
	case PlanRoute, PlanSelect, PlanEvents, PlanScanEmit, PlanMap, PlanFilter, PlanCsvTable:
		return ProtocolItems
	}
	return ProtocolText
}

// PlanName is the operator at the root of a plan, for messages.
func PlanName(p *Plan) string {
	switch p.Kind {
	case PlanInput:
		return "input"
	case PlanRoute:
		return "route"
	case PlanSelect:
		return "select"
	case PlanEvents:
		return "events"
	case PlanAsEvents:
		return "as-events"
	case PlanScanEmit:
		return "scan-emit"
	case PlanMap:
		return "map"
	case PlanFilter:
		return "filter"
	case PlanTableFromJSON:
		return "table-from-json"
	case PlanRecords:
		return "records"
	case PlanCsvTable:
		return "csv-table"
	case PlanLit:
		return "text"
	case PlanConcat:
		return "concat"
	case PlanJoin:
		return "join"
	case PlanConcatMap:
		return "concat-map"
	case PlanReplace:
		return "replace-text"
	case PlanCsv:
		return "csv"
	case PlanJSON:
		return "json"
	}
	return "plan"
}

// ---------------------------------------------------------------------------
// What a value is
// ---------------------------------------------------------------------------

// Same reports whether two values are the same retained value, not merely
// equal: one allocation behind both. A scalar is never the same as
// anything, which only costs a re-measure where this is asked.
func Same(a, b Val) bool {
	switch x := a.(type) {
	case *VectorVal:
		y, ok := b.(*VectorVal)
		return ok && x == y
	case *RecordVal:
		y, ok := b.(*RecordVal)
		return ok && x == y
	case *TaggedVal:
		y, ok := b.(*TaggedVal)
		return ok && x == y
	case *SelectorVal:
		y, ok := b.(*SelectorVal)
		return ok && x == y
	case StreamVal:
		y, ok := b.(StreamVal)
		return ok && x.Plan == y.Plan
	case TextVal:
		y, ok := b.(TextVal)
		return ok && x.Plan == y.Plan
	}
	return false
}

// IsLive reports whether this value holds a live stream or text: the
// one-shot resources a vector may not hide and a fn may not capture.
func IsLive(v Val) bool {
	switch x := v.(type) {
	case StreamVal:
		return x.Plan.IsLive()
	case TextVal:
		return x.Plan.IsLive()
	}
	return false
}

// KindOf is a one-word name of a value's kind, for messages.
func KindOf(v Val) string {
	switch v.(type) {
	case NullVal:
		return "null"
	case BoolVal:
		return "a boolean"
	case NumVal:
		return "a number"
	case StrVal:
		return "a string"
	case KeywordVal:
		return "a keyword"
	case *VectorVal:
		return "a vector"
	case *RecordVal:
		return "a record"
	case Fn:
		return "a function"
	case *SelectorVal:
		return "a selector"
	case *CaptureVal:
		return "a capture"
	case *TaggedVal:
		if IsMissing(v) {
			return "missing"
		}
		return "a tagged value"
	case StreamVal:
		return "a stream"
	case TextVal:
		return "a text"
	}
	return "nothing"
}

// liveKind is what a live value is, for a message that refuses to hold it:
// a text is named live, since a finite one could have been held.
func liveKind(v Val) string {
	if _, ok := v.(TextVal); ok {
		return "a live text"
	}
	return KindOf(v)
}

// FromDatum is the value of a retained transduce value. Numbers keep their
// lexemes.
func FromDatum(d *shared.Datum) Val {
	switch d.Kind {
	case shared.DatumBool:
		return BoolVal(d.Bool)
	case shared.DatumNumber:
		return NumVal{Value: d.Value, Lexeme: d.Lexeme, HasLexeme: d.HasLexeme}
	case shared.DatumString:
		return StrVal(d.Text)
	case shared.DatumArray:
		items := make([]Val, len(d.Items))
		for i := range d.Items {
			items[i] = FromDatum(&d.Items[i])
		}
		return &VectorVal{Items: items}
	case shared.DatumObject:
		r := &RecordVal{keys: make([]string, 0, len(d.Members)), vals: make([]Val, 0, len(d.Members))}
		for i := range d.Members {
			r.Set(d.Members[i].Key, FromDatum(&d.Members[i].Value))
		}
		return r
	}
	return NullVal{}
}

// ToDatum is the value as a retained transduce value: the data kinds
// convert, a keyword becomes its name, `missing` becomes `null`, and a
// function, a selector, a capture, a stream or a text has no data form.
func ToDatum(v Val) (shared.Datum, *Fail) {
	switch x := v.(type) {
	case NullVal:
		return shared.NullDatum(), nil
	case BoolVal:
		return shared.BoolDatum(bool(x)), nil
	case NumVal:
		return shared.Datum{Kind: shared.DatumNumber, Value: x.Value, Lexeme: x.Lexeme, HasLexeme: x.HasLexeme}, nil
	case StrVal:
		return shared.StringDatum(string(x)), nil
	case KeywordVal:
		return shared.StringDatum(string(x)), nil
	case *VectorVal:
		items := make([]shared.Datum, len(x.Items))
		for i, item := range x.Items {
			d, f := ToDatum(item)
			if f != nil {
				return shared.Datum{}, f
			}
			items[i] = d
		}
		return shared.ArrayDatum(items...), nil
	case *RecordVal:
		members := make([]shared.Member, len(x.keys))
		for i, k := range x.keys {
			d, f := ToDatum(x.vals[i])
			if f != nil {
				return shared.Datum{}, f
			}
			members[i] = shared.Member{Key: k, Value: d}
		}
		return shared.ObjectDatum(members...), nil
	}
	if IsMissing(v) {
		return shared.NullDatum(), nil
	}
	return shared.Datum{}, typeError(KindOf(v) + " has no data form")
}

// Field is the field key of a record, or `missing`; ok is false when v is
// not a record.
func Field(v Val, key string) (Val, bool) {
	r, ok := v.(*RecordVal)
	if !ok {
		return nil, false
	}
	if value, ok := r.Get(key); ok {
		return value, true
	}
	return Missing(), true
}

// GetPath is the value at a concrete path below v, walking records by key
// and vectors by index; `missing` where the path leaves the data.
func GetPath(v Val, segments []shared.Segment) Val {
	here := v
	for _, seg := range segments {
		switch x := here.(type) {
		case *RecordVal:
			if seg.IsIndex {
				return Missing()
			}
			next, ok := x.Get(seg.Key)
			if !ok {
				return Missing()
			}
			here = next
		case *VectorVal:
			if !seg.IsIndex || seg.Index < 0 || seg.Index >= len(x.Items) {
				return Missing()
			}
			here = x.Items[seg.Index]
		default:
			return Missing()
		}
	}
	return here
}

// JSONText is the compact JSON text of a data value, keeping number
// lexemes.
func JSONText(v Val) (string, *Fail) {
	d, f := ToDatum(v)
	if f != nil {
		return "", f
	}
	return d.String(), nil
}

// selectorSegments is a selector that names one location, as segments;
// false when it names many (each-index, each-member).
func selectorSegments(s shared.Selector) ([]shared.Segment, bool) {
	steps := s.Steps()
	out := make([]shared.Segment, 0, len(steps))
	for _, step := range steps {
		switch step.Kind {
		case shared.StepProperty:
			out = append(out, shared.KeySegment(step.Name))
		case shared.StepIndex:
			out = append(out, shared.IndexSegment(step.Index))
		default:
			return nil, false
		}
	}
	return out, true
}

// typeError is a DSL_TYPE_ERROR found while running, with the
// `type_mismatch` finer code the checker also uses.
func typeError(message string) *Fail {
	return NewFail(CodeDSLTypeError, "type_mismatch: "+message)
}

// Equal is structural equality of data; numbers by value; functions,
// streams and texts never equal. A record equals one with the same fields
// in any order.
func Equal(a, b Val) bool {
	switch x := a.(type) {
	case NullVal:
		_, ok := b.(NullVal)
		return ok
	case BoolVal:
		y, ok := b.(BoolVal)
		return ok && x == y
	case NumVal:
		y, ok := b.(NumVal)
		return ok && x.Value == y.Value
	case StrVal:
		y, ok := b.(StrVal)
		return ok && x == y
	case KeywordVal:
		y, ok := b.(KeywordVal)
		return ok && x == y
	case *VectorVal:
		y, ok := b.(*VectorVal)
		if !ok || len(x.Items) != len(y.Items) {
			return false
		}
		for i := range x.Items {
			if !Equal(x.Items[i], y.Items[i]) {
				return false
			}
		}
		return true
	case *RecordVal:
		y, ok := b.(*RecordVal)
		if !ok || len(x.keys) != len(y.keys) {
			return false
		}
		for i, k := range x.keys {
			other, ok := y.Get(k)
			if !ok || !Equal(x.vals[i], other) {
				return false
			}
		}
		return true
	case *SelectorVal:
		y, ok := b.(*SelectorVal)
		return ok && x.Selector.Equal(y.Selector)
	case *CaptureVal:
		y, ok := b.(*CaptureVal)
		return ok && captureEqual(x.Spec, y.Spec)
	case *TaggedVal:
		y, ok := b.(*TaggedVal)
		if !ok || x.Tag != y.Tag || len(x.Fields) != len(y.Fields) {
			return false
		}
		for i := range x.Fields {
			if !Equal(x.Fields[i], y.Fields[i]) {
				return false
			}
		}
		return true
	}
	return false
}

func captureEqual(a, b shared.CaptureSpec) bool {
	if a.Tag != b.Tag || a.Mode != b.Mode || !a.Selector.Equal(b.Selector) {
		return false
	}
	switch {
	case a.Budget == nil && b.Budget == nil:
		return true
	case a.Budget == nil || b.Budget == nil:
		return false
	}
	return *a.Budget == *b.Budget
}

// floatText is a number without a lexeme as Rust's f64 Display writes it,
// for messages: the shortest digits that read back, never an exponent.
func floatText(v float64) string {
	switch {
	case math.IsNaN(v):
		return "NaN"
	case math.IsInf(v, 1):
		return "inf"
	case math.IsInf(v, -1):
		return "-inf"
	}
	return strconv.FormatFloat(v, 'f', -1, 64)
}

// DebugString is a value as the Rust crate's Debug writes it, for
// messages: data in the language's own notation, the rest named.
func DebugString(v Val) string {
	var b strings.Builder
	writeDebug(v, &b)
	return b.String()
}

func writeDebug(v Val, b *strings.Builder) {
	switch x := v.(type) {
	case NullVal:
		b.WriteString("null")
	case BoolVal:
		b.WriteString(strconv.FormatBool(bool(x)))
	case NumVal:
		if x.HasLexeme {
			b.WriteString(x.Lexeme)
		} else {
			b.WriteString(floatText(x.Value))
		}
	case StrVal:
		b.WriteString(strconv.Quote(string(x)))
	case KeywordVal:
		b.WriteByte(':')
		b.WriteString(string(x))
	case *VectorVal:
		b.WriteByte('[')
		for i, item := range x.Items {
			if i > 0 {
				b.WriteByte(' ')
			}
			writeDebug(item, b)
		}
		b.WriteByte(']')
	case *RecordVal:
		b.WriteString("(record")
		for i, k := range x.keys {
			b.WriteString(" (entry :")
			b.WriteString(k)
			b.WriteByte(' ')
			writeDebug(x.vals[i], b)
			b.WriteByte(')')
		}
		b.WriteByte(')')
	case Fn:
		b.WriteString("<" + x.Describe() + ">")
	case *SelectorVal:
		b.WriteString("<selector " + x.Selector.String() + ">")
	case *CaptureVal:
		b.WriteString("<capture :" + x.Spec.Tag + " " + x.Spec.Selector.String() + ">")
	case *TaggedVal:
		if len(x.Fields) == 0 {
			b.WriteString(x.Tag)
			return
		}
		b.WriteString("(" + x.Tag)
		for _, field := range x.Fields {
			b.WriteByte(' ')
			writeDebug(field, b)
		}
		b.WriteByte(')')
	case StreamVal:
		b.WriteString("<stream " + PlanName(x.Plan) + ">")
	case TextVal:
		b.WriteString("<text " + PlanName(x.Plan) + ">")
	default:
		fmt.Fprintf(b, "%v", v)
	}
}
