// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"fmt"
	"math"
	"strconv"
	"strings"
	"unicode/utf8"

	"github.com/tabnas/alchemy/go/shared"
)

// lower.go: lowering (rs/src/lower.rs): a Plan becomes a chain of
// transduce and render sinks, built through the Routers and Renderers the
// host handed in (Runtime.Routers, Runtime.Renderers).
//
// The evaluator answers a plan; the host has a JsonEvents/1 producer and a
// text output. This file joins them. The chain is push-based and
// synchronous, as the two packages are: the host calls the returned Sink
// once per event and once with End, each stage does its work and calls the
// next, and the output is flushed once, at the end, by the stage that owns
// it. Four kinds of stage exist, one per protocol boundary:
//
//   - events (JsonEvents/1): the host's input, a `records` stage
//     (RecordsToJSON) over table events, or an adapter (taggedToJSON)
//     reading the tagged events an interpreted stream yields, the reverse
//     of `events`;
//   - table (TableRows/1): an adapter reading the tagged `schema`, `row`
//     and `table-end` values a stream yields;
//   - items (a stream of values): a Router over events for `route` and
//     `select`, eventsToItems for `events`, scanStage for `scan-emit`,
//     and per-item stages for `map` and `filter`;
//   - text: JSONRenderer over events for `json`, the host's CSVRenderer or
//     RecordsToJSON over a table result (SinkAs), `concat-map` and `join`
//     over a stream of items, `replace-text` as ReplaceText, and `concat`
//     around one live text as a frame that writes its prefix before the
//     first fragment and its suffix at the flush.
//
// A text that does not reach the input is finite and is written whole
// wherever it lands (writeFinite): a row of the library's CSV is rendered
// into a scratch string and written as one fragment, so a failure in the
// middle of a row leaves no half row behind, as the host's CSV renderer
// promises. Errors are transduce failures with the transduce and render
// codes unchanged; a stream given to a stage of another protocol is
// DSL_TYPE_ERROR with the `protocol_mismatch` finer code, the same word the
// checker uses when it can see it first.

// ItemSink consumes a stream's items.
type ItemSink interface {
	Item(v Val) (shared.Flow, *Fail)
	// End is the stream's validated end: exactly once, after the last
	// item.
	End() (shared.Flow, *Fail)
}

// Renderer is how the host renders a stream result: CSV for table events
// (the default), JSON as records; JSON for a JSON-events result.
// RenderDefault is the host's default, and the only choice for a program
// that renders its own text.
type Renderer uint8

// The renderers.
const (
	RenderDefault Renderer = iota
	RenderCSV
	RenderJSON
)

// RendererNamed is the renderer by the name the command line uses.
func RendererNamed(name string) (Renderer, bool) {
	switch name {
	case "csv":
		return RenderCSV, true
	case "json":
		return RenderJSON, true
	}
	return RenderDefault, false
}

// String is the renderer's name, "" for the default.
func (r Renderer) String() string {
	switch r {
	case RenderCSV:
		return "csv"
	case RenderJSON:
		return "json"
	}
	return ""
}

// jsonOptions are the options the `json` renderer and the `--render json`
// echo use: compact, one document, a final newline.
func jsonOptions() shared.JSONOptions { return shared.JSONOptions{Indent: 0, TrailingNewline: true} }

func protocolMismatch(message string) *Fail {
	return NewFail(CodeDSLTypeError, "protocol_mismatch: "+message)
}

// nullNonFinite is the json renderer's adapter for :non-finite :null: a
// number that is not finite goes on as null, everything else as it came.
type nullNonFinite struct {
	next shared.Sink
}

func (n *nullNonFinite) Event(ev shared.Event) (shared.Flow, *Fail) {
	if ev.Kind == shared.Number && (math.IsNaN(ev.Value) || math.IsInf(ev.Value, 0)) {
		return n.next.Event(shared.EvNull())
	}
	return n.next.Event(ev)
}

// cell is a cell as the table protocol carries it, from the value a
// program produced: the scalars as themselves, `missing` as missing, a
// vector or a record as its compact JSON text, as `scalar-text` writes it,
// under the same max_scalar_bytes.
func cell(rt *Runtime, v Val) (shared.Cell, *Fail) {
	switch x := v.(type) {
	case NullVal:
		return shared.Cell{Kind: shared.CellNull}, nil
	case BoolVal:
		return shared.Cell{Kind: shared.CellBool, Bool: bool(x)}, nil
	case NumVal:
		return shared.Cell{Kind: shared.CellNumber, Value: x.Value, Lexeme: x.Lexeme, HasLexeme: x.HasLexeme}, nil
	case StrVal:
		return shared.Cell{Kind: shared.CellString, Text: string(x)}, nil
	}
	if IsMissing(v) {
		return shared.Cell{Kind: shared.CellMissing}, nil
	}
	switch v.(type) {
	case *VectorVal, *RecordVal:
		text, f := rt.jsonText(v)
		if f != nil {
			return shared.Cell{}, f
		}
		return shared.Cell{Kind: shared.CellString, Text: text}, nil
	}
	return shared.Cell{}, typeError("a cell must be a scalar, a vector or a record, not " + KindOf(v))
}

// labelText is the text of a column's label, one policy for every table:
// a string as it is, a number by its lexeme, a boolean by its name (as
// `scalar-text` would write the same header cell); `missing` is
// MISSING_VALUE, and anything else INPUT_INVALID. The adapter reading a
// program's schema values takes it, and csv-table validates the library's
// schema by it, so a label is accepted or refused alike whichever renders
// it.
func labelText(label Val) (string, *Fail) {
	switch x := label.(type) {
	case StrVal:
		return string(x), nil
	case NumVal:
		return numberText(x.Value, x.Lexeme, x.HasLexeme)
	case BoolVal:
		return strconv.FormatBool(bool(x)), nil
	}
	if IsMissing(label) {
		return "", NewFail(CodeMissingValue, "a column has no label")
	}
	return "", shared.InputFail("a column's label must be a string, a number or a boolean, not " + KindOf(label))
}

// label is the label of a schema value's column: a record's :label.
func label(column Val) (string, *Fail) {
	l, ok := Field(column, "label")
	if !ok {
		return "", shared.ProtocolFail("a schema column is not a record")
	}
	return labelText(l)
}

// schemaColumns are the columns of a schema value, as the table protocol
// carries them: a vector of column records, at most max_columns of them.
func schemaColumns(fields []Val, limits shared.Limits) ([]shared.PublicColumn, *Fail) {
	columns, ok := fields[0].(*VectorVal)
	if !ok {
		return nil, shared.ProtocolFail("schema takes a vector of columns")
	}
	if len(columns.Items) > limits.MaxColumns {
		return nil, shared.LimitFail("max_columns", uint64(limits.MaxColumns),
			fmt.Sprintf("the schema declares %d columns, more than %d", len(columns.Items), limits.MaxColumns))
	}
	out := make([]shared.PublicColumn, 0, len(columns.Items))
	for _, c := range columns.Items {
		l, f := label(c)
		if f != nil {
			return nil, f
		}
		out = append(out, shared.PublicColumn{Label: l})
	}
	return out, nil
}

// rowCells are the cells of a row value, as the table protocol carries
// them, into cells.
func rowCells(rt *Runtime, fields []Val, cells []shared.Cell) ([]shared.Cell, *Fail) {
	values, ok := fields[0].(*VectorVal)
	if !ok {
		return cells, shared.ProtocolFail("row takes a vector of cells")
	}
	cells = cells[:0]
	for _, v := range values.Items {
		c, f := cell(rt, v)
		if f != nil {
			return cells, f
		}
		cells = append(cells, c)
	}
	return cells, nil
}

// briefMax is the most characters of a value's text a message quotes.
const briefMax = 60

// brief is a value for a message: its kind, and at most a short prefix of
// its text, so a failure over a large document does not carry the
// document.
func brief(v Val) string {
	text := DebugString(v)
	if utf8.RuneCountInString(text) <= briefMax {
		return text
	}
	cut, n := 0, 0
	for i := range text {
		if n == briefMax {
			cut = i
			break
		}
		n++
	}
	return KindOf(v) + " (" + text[:cut] + "...)"
}

// writeFinite writes a finite text or string to out, whole. Every node
// takes an evaluation step and a level (Runtime.Tick, Runtime.enter), so
// the host's abort flag stops a text of exponentially many fragments, and
// a concat-map whose function answers a text that applies it again is the
// `recursion` failure rather than an unbounded recursion. A join writes
// :record-end among its items as shared.RecordEnd, no item and no text.
func writeFinite(rt *Runtime, v Val, out shared.TextOut) *Fail {
	if f := rt.Tick(); f != nil {
		return f
	}
	var at *SourceSpan
	if t, ok := v.(TextVal); ok && t.Plan.Kind == PlanConcatMap {
		at = &t.Plan.At
	}
	if f := rt.enter(at); f != nil {
		return f
	}
	defer rt.leave()
	switch x := v.(type) {
	case StrVal:
		return out.WriteStr(string(x))
	case TextVal:
		p := x.Plan
		switch {
		case p.Kind == PlanLit:
			return out.WriteStr(p.Lit)
		case p.Kind == PlanConcat:
			for _, item := range p.Items {
				if f := writeFinite(rt, item, out); f != nil {
					return f
				}
			}
			return nil
		case p.Kind == PlanJoin && !p.Seq.IsStream():
			join := rt.Renderers().Join(out, p.Sep)
			for _, item := range p.Seq.Vector {
				if isRecordEnd(item) {
					if f := shared.RecordEnd(join); f != nil {
						return f
					}
					continue
				}
				if f := join.ItemStart(); f != nil {
					return f
				}
				if f := writeFinite(rt, item, unmarked{join}); f != nil {
					return f
				}
				if f := join.ItemEnd(); f != nil {
					return f
				}
			}
			return nil
		case p.Kind == PlanConcatMap && !p.Seq.IsStream():
			for _, item := range p.Seq.Vector {
				text, f := rt.Apply(p.F, []Val{item}, p.At)
				if f != nil {
					return f
				}
				if f := writeFinite(rt, text, unmarked{out}); f != nil {
					return f
				}
				if f := shared.RecordEnd(out); f != nil {
					return f
				}
			}
			return nil
		// Streamed through the replacer, as a live text is: it holds at
		// most the literal's length, never the text.
		case p.Kind == PlanReplace:
			replace := rt.Renderers().ReplaceText(held{out}, p.From, p.To)
			if f := writeFinite(rt, p.Text, replace); f != nil {
				return f
			}
			return replace.Flush()
		}
		return typeError(PlanName(p) + " is a live text and cannot be written as a value")
	}
	return typeError("a string or a text was expected, not " + KindOf(v))
}

// held is an output that passes fragments on and keeps its flush: a
// combinator written into the middle of a text (replace-text inside a
// concat) hands over what it holds at its own flush, and the text around
// it goes on.
type held struct{ out shared.TextOut }

func (h held) WriteStr(s string) *Fail { return h.out.WriteStr(s) }
func (h held) Flush() *Fail            { return nil }
func (h held) HasCommitted() bool      { return h.out.HasCommitted() }
func (h held) RecordEnd() *Fail        { return shared.RecordEnd(h.out) }

// unmarked is an item of a join or a concat-map over a vector, written
// through to the output beneath with its own record ends dropped (it is
// no shared.RecordEnder), as the stream stages' scratch drops them: an
// item is written whole, and the marks of the text are the ones between
// its items. Its flush is kept, as held's is.
type unmarked struct{ out shared.TextOut }

func (u unmarked) WriteStr(s string) *Fail { return u.out.WriteStr(s) }
func (u unmarked) Flush() *Fail            { return nil }
func (u unmarked) HasCommitted() bool      { return u.out.HasCommitted() }

// scratch is one item's text, assembled whole before it is written, so a
// failure half way through an item (a missing cell in a row) leaves
// nothing of it behind. It is bounded by max_output_bytes: an item longer
// than the whole output may be could never be written, and assembling it
// anyway is what would let a small program exhaust memory under an output
// limit. A record end inside an item marks nothing (it is no
// shared.RecordEnder): the item is written whole.
type scratch struct {
	text strings.Builder
	max  *uint64
}

func newScratch(limits shared.Limits) *scratch { return &scratch{max: limits.MaxOutputBytes} }

func (s *scratch) WriteStr(t string) *Fail {
	if s.max != nil {
		max := *s.max
		if n := uint64(s.text.Len() + len(t)); n > max {
			return shared.LimitFail("max_output_bytes", max,
				fmt.Sprintf("one item's text reached %d bytes; the output may not exceed %d", n, max))
		}
	}
	s.text.WriteString(t)
	return nil
}

func (s *scratch) Flush() *Fail       { return nil }
func (s *scratch) HasCommitted() bool { return false }

// ---------------------------------------------------------------------------
// Item stages
// ---------------------------------------------------------------------------

// routeToItems is `route` and `select`: what the router delivers, as
// items.
type routeToItems struct {
	down ItemSink
	// valuesOnly: `select` delivers the values; `route` wraps each in
	// `selected`.
	valuesOnly bool
}

func (r *routeToItems) Selected(s shared.Selected) (shared.Flow, *Fail) {
	value := Val(NullVal{})
	if s.Value != nil {
		value = FromDatum(s.Value)
	}
	if r.valuesOnly {
		return r.down.Item(value)
	}
	return r.down.Item(NewTagged("selected", KeywordVal(s.Tag), value))
}

func (r *routeToItems) End() (shared.Flow, *Fail) { return r.down.End() }

// eventsToItems is `events`: every event of JsonEvents/1 as the tagged
// value a program matches on, pushed down as it arrives, and End as the
// stream's end. Nothing is kept between events: a key is delivered as it
// is read, twice when the source repeats it, and a scalar with its lexeme.
// parserEventsToItems is the parser's rule passes as the items
// parser-events yields: (open rule) and (close rule), the rule a record of
// the pass's fields, a field the pass lacks left out (get answers missing
// for it). The parse's End is the stream's end. The head of a program's
// chain when the host declares parser events: the host pushes each pass
// into it.
type parserEventsToItems struct {
	down ItemSink
}

func (e *parserEventsToItems) ParserEvent(ev shared.ParserEvent) (shared.Flow, *Fail) {
	switch ev.Kind {
	case shared.ParserEnd:
		return e.down.End()
	case shared.ParserOpen, shared.ParserClose:
		// A host builds the event, and an open or a close without its
		// pass (the struct's zero value is one) is refused, not read.
		if ev.Pass == nil {
			return shared.Continue, shared.ProtocolFail("a parser event's " + ev.Kind.String() + " carries no rule pass")
		}
		return e.down.Item(NewTagged(ev.Kind.String(), ruleRecord(ev.Pass)))
	}
	return shared.Continue, shared.ProtocolFail("an unknown parser event kind")
}

// ruleRecord is a rule pass as the record (open rule) and (close rule)
// carry.
func ruleRecord(pass *shared.RulePass) Val {
	rec := NewRecord()
	rec.Set("name", StrVal(pass.Name))
	rec.Set("d", NumVal{Value: float64(pass.D)})
	rec.Set("i", NumVal{Value: float64(pass.I)})
	if pass.Alt != nil {
		alt := NewRecord()
		alt.Set("p", StrVal(pass.Alt.P))
		alt.Set("r", StrVal(pass.Alt.R))
		alt.Set("b", NumVal{Value: float64(pass.Alt.B)})
		groups := make([]Val, len(pass.Alt.G))
		for i, g := range pass.Alt.G {
			groups[i] = StrVal(g)
		}
		alt.Set("g", &VectorVal{Items: groups})
		rec.Set("alt", alt)
	}
	tokens := make([]Val, len(pass.Tokens))
	for i := range pass.Tokens {
		t := &pass.Tokens[i]
		token := NewRecord()
		token.Set("name", StrVal(t.Name))
		token.Set("val", FromDatum(&t.Val))
		token.Set("src", StrVal(t.Src))
		token.Set("row", NumVal{Value: float64(t.Row)})
		token.Set("col", NumVal{Value: float64(t.Col)})
		tokens[i] = token
	}
	rec.Set("tokens", &VectorVal{Items: tokens})
	rec.Set("forced", BoolVal(pass.Forced))
	n := NewRecord()
	for _, c := range pass.N {
		n.Set(c.Name, NumVal{Value: float64(c.Value)})
	}
	rec.Set("n", n)
	if pass.Value != nil {
		rec.Set("value", FromDatum(pass.Value))
	}
	return rec
}

// parserHead is what the host pushes parser events into: the chain's head
// (parserEventsToItems) when the program reads them, and otherwise (a text
// of the program's own, which reads nothing of its input) the parse's end
// alone, handed to the sink that writes the text once the input has
// validated.
type parserHead struct {
	head   shared.ParserSink
	events shared.Sink
}

func (h *parserHead) ParserEvent(ev shared.ParserEvent) (shared.Flow, *Fail) {
	if h.head != nil {
		return h.head.ParserEvent(ev)
	}
	if ev.Kind == shared.ParserEnd {
		return h.events.Event(shared.Event{Kind: shared.End})
	}
	return shared.Continue, nil
}

// parserEventsOnly is the JSON-events sink items answers where it lowers
// parser-events: the host pushes the rule passes into the head it keeps
// instead (Lowering.ParserHead), never JSON events into this.
type parserEventsOnly struct{}

func (parserEventsOnly) Event(shared.Event) (shared.Flow, *Fail) {
	return shared.Continue, protocolMismatch("the program reads parser events; push them into Program.ParserSink")
}

type eventsToItems struct {
	down ItemSink
}

func (e *eventsToItems) Event(ev shared.Event) (shared.Flow, *Fail) {
	var item Val
	switch ev.Kind {
	case shared.ObjectStart:
		item = NewTagged("object-start")
	case shared.ObjectEnd:
		item = NewTagged("object-end")
	case shared.ArrayStart:
		item = NewTagged("array-start")
	case shared.ArrayEnd:
		item = NewTagged("array-end")
	case shared.Key:
		item = NewTagged("key", StrVal(ev.Text))
	case shared.Null:
		item = NewTagged("scalar", NullVal{})
	case shared.Bool:
		item = NewTagged("scalar", BoolVal(ev.Bool))
	case shared.Number:
		item = NewTagged("scalar", NumVal{Value: ev.Value, Lexeme: ev.Lexeme, HasLexeme: ev.HasLexeme})
	case shared.String:
		item = NewTagged("scalar", StrVal(ev.Text))
	case shared.End:
		return e.down.End()
	default:
		return shared.Continue, shared.ProtocolFail("an unknown event kind")
	}
	return e.down.Item(item)
}

// taggedToJSON reads the tagged events an interpreted stream yields as the
// JSON events a taker of JsonEvents reads (`json`, `table-from-json`,
// `select`, `route`, `events` again): the reverse of eventsToItems, so a
// program can rewrite a document event by event (`events` through a
// `scan-emit`), or build events with their constructors, and hand them on.
// Each item must be an event of the shape the constructors make
// (`object-start`, `object-end`, `array-start`, `array-end`, `(key
// name)`, `(scalar value)`); anything else is PROTOCOL_ORDER_ERROR naming
// it. The sequence is the taker's to validate, as the source's own events
// are, and the stream's end is the events' End. The sink beneath is the
// source's guard (Guarded), so the source's limits hold on these events as
// on its own.
type taggedToJSON struct {
	down shared.Sink
}

func (t *taggedToJSON) Item(v Val) (shared.Flow, *Fail) {
	tagged, ok := v.(*TaggedVal)
	if !ok {
		return shared.Continue, shared.ProtocolFail("an event was expected, not " + KindOf(v))
	}
	var ev shared.Event
	bad := false
	switch {
	case tagged.Tag == "object-start" && len(tagged.Fields) == 0:
		ev = shared.EvObjectStart()
	case tagged.Tag == "object-end" && len(tagged.Fields) == 0:
		ev = shared.EvObjectEnd()
	case tagged.Tag == "array-start" && len(tagged.Fields) == 0:
		ev = shared.EvArrayStart()
	case tagged.Tag == "array-end" && len(tagged.Fields) == 0:
		ev = shared.EvArrayEnd()
	case tagged.Tag == "key" && len(tagged.Fields) == 1:
		name, ok := tagged.Fields[0].(StrVal)
		ev, bad = shared.EvKey(string(name)), !ok
	case tagged.Tag == "scalar" && len(tagged.Fields) == 1:
		switch x := tagged.Fields[0].(type) {
		case NullVal:
			ev = shared.EvNull()
		case BoolVal:
			ev = shared.EvBool(bool(x))
		case NumVal:
			ev = shared.Event{Kind: shared.Number, Value: x.Value, Lexeme: x.Lexeme, HasLexeme: x.HasLexeme}
		case StrVal:
			ev = shared.EvString(string(x))
		default:
			bad = true
		}
	default:
		bad = true
	}
	if bad {
		return shared.Continue, shared.ProtocolFail("an event was expected, not " + brief(v))
	}
	return t.down.Event(ev)
}

func (t *taggedToJSON) End() (shared.Flow, *Fail) { return t.down.Event(shared.EvEnd()) }

// stateBounds are what a scan-emit state may hold: what a stage keeps from
// row to row, as the library's table keeps its bound columns, so
// max_metadata_bytes in the transduce measure; and no deeper than a
// captured value may be, max_depth, since a state that wraps itself once
// per item would otherwise nest without bound.
func stateBounds(limits shared.Limits) Bounds {
	return Bounds{
		NodeBytes:  shared.NodeBytes,
		MaxBytes:   uint64(limits.MaxMetadataBytes),
		BytesLimit: "max_metadata_bytes",
		MaxDepth:   limits.MaxDepth,
		What:       "the scan-emit state",
	}
}

// raiseHigh lifts a high-water mark to value if it is higher.
func raiseHigh(high interface {
	Load() uint64
	CompareAndSwap(old, new uint64) bool
}, value uint64) {
	for {
		old := high.Load()
		if value <= old || high.CompareAndSwap(old, value) {
			return
		}
	}
}

// scanStage is `scan-emit`: transduce's operator over the program's step
// and finish. The operator takes values of any type (Routers.ScanEmit), so
// the state, the items and the outputs cross it as any and are read back
// as the values they are.
type scanStage struct {
	scan shared.ScanEmitter
	down ItemSink
}

// asVal is a value that crossed the scan operator as any.
func asVal(v any) Val {
	x, _ := v.(Val)
	return x
}

// asAnys is values as the scan operator takes them: a fresh slice, nil
// for none.
func asAnys(items []Val) []any {
	if len(items) == 0 {
		return nil
	}
	out := make([]any, len(items))
	for i, v := range items {
		out[i] = v
	}
	return out
}

func newScanStage(rt *Runtime, init Val, step, finish Fn, at SourceSpan, metrics *shared.Metrics, down ItemSink) (*scanStage, *Fail) {
	// The initial state is retained like any the step returns, and the
	// step measures only a state that changed: a step that hands the same
	// one back, or a source with no items, would never measure it, so it
	// is measured here, before anything is read.
	size, f := rt.Measure(init, stateBounds(rt.Limits()))
	if f != nil {
		return nil, rt.FailAt(f, at)
	}
	raiseHigh(&metrics.RetainedBytesHigh, metrics.CapturedBytes.Load()+size.Bytes)
	stepFn := func(stateAny, itemAny any) (shared.Transition[any, any], *Fail) {
		state, item := asVal(stateAny), asVal(itemAny)
		before := state
		t, f := rt.Apply(step, []Val{state, item}, at)
		if f != nil {
			return shared.Transition[any, any]{}, f
		}
		tagged, ok := t.(*TaggedVal)
		if !ok || tagged.Tag != "transition" || len(tagged.Fields) != 2 {
			return shared.Transition[any, any]{}, rt.FailAt(typeError("scan-emit: the step must answer a transition, not "+KindOf(t)), at)
		}
		outputs, ok := tagged.Fields[1].(*VectorVal)
		if !ok {
			return shared.Transition[any, any]{}, rt.FailAt(typeError("scan-emit: a transition's outputs must be a vector"), at)
		}
		// The state is what this stage retains across items: measured
		// when it changes, against the limits that bound what a stage
		// keeps from row to row, and reported with the captures in
		// retained_bytes_high.
		if !Same(tagged.Fields[0], before) {
			size, f := rt.Measure(tagged.Fields[0], stateBounds(rt.Limits()))
			if f != nil {
				return shared.Transition[any, any]{}, rt.FailAt(f, at)
			}
			raiseHigh(&metrics.RetainedBytesHigh, metrics.CapturedBytes.Load()+size.Bytes)
		}
		return shared.Transition[any, any]{State: tagged.Fields[0], Outputs: asAnys(outputs.Items)}, nil
	}
	finishFn := func(stateAny any) ([]any, *Fail) {
		v, f := rt.Apply(finish, []Val{asVal(stateAny)}, at)
		if f != nil {
			return nil, f
		}
		outputs, ok := v.(*VectorVal)
		if !ok {
			return nil, rt.FailAt(typeError("scan-emit: finish must answer a vector of outputs, not "+KindOf(v)), at)
		}
		return asAnys(outputs.Items), nil
	}
	outFn := func(o any) (shared.Flow, *Fail) { return down.Item(asVal(o)) }
	return &scanStage{scan: rt.Routers().ScanEmit(init, stepFn, finishFn, outFn), down: down}, nil
}

func (s *scanStage) Item(v Val) (shared.Flow, *Fail) { return s.scan.Item(v) }

func (s *scanStage) End() (shared.Flow, *Fail) {
	flow, f := s.scan.Finish()
	if f != nil {
		return shared.Continue, f
	}
	if flow == shared.Stop {
		return shared.Stop, nil
	}
	return s.down.End()
}

// mapStage is `map` over a stream.
type mapStage struct {
	rt   *Runtime
	f    Fn
	at   SourceSpan
	down ItemSink
}

func (m *mapStage) Item(v Val) (shared.Flow, *Fail) {
	mapped, f := m.rt.Apply(m.f, []Val{v}, m.at)
	if f != nil {
		return shared.Continue, f
	}
	if IsLive(mapped) {
		return shared.Continue, m.rt.FailAt(typeError("map: the function must answer a value per item, not a live stream"), m.at)
	}
	return m.down.Item(mapped)
}

func (m *mapStage) End() (shared.Flow, *Fail) { return m.down.End() }

// filterStage is `filter` over a stream.
type filterStage struct {
	rt   *Runtime
	f    Fn
	at   SourceSpan
	down ItemSink
}

func (s *filterStage) Item(v Val) (shared.Flow, *Fail) {
	keep, f := s.rt.Apply(s.f, []Val{v}, s.at)
	if f != nil {
		return shared.Continue, f
	}
	yes, f := truth("filter", keep)
	if f != nil {
		return shared.Continue, s.rt.FailAt(f, s.at)
	}
	if yes {
		return s.down.Item(v)
	}
	return shared.Continue, nil
}

func (s *filterStage) End() (shared.Flow, *Fail) { return s.down.End() }

// taggedToTable reads the tagged values a stream yields as table events.
// One schema first, rows, one table-end: the renderer
// beneath validates the sequence; what is validated here is that each item
// is a table event at all, of the table protocol's shape (at most
// max_columns columns, labels by labelText), and that the stream did end
// with table-end. It is the last table stage before a renderer, so it
// counts each row in metrics.Rows, unless countRows says a later stage does
// (Lowering.items).
type taggedToTable struct {
	rt        *Runtime
	metrics   *shared.Metrics
	countRows bool
	table     shared.TableSink
	columns   []shared.PublicColumn
	cells     []shared.Cell
	ended     bool
}

func (t *taggedToTable) Item(v Val) (shared.Flow, *Fail) {
	tagged, ok := v.(*TaggedVal)
	if !ok {
		return shared.Continue, shared.ProtocolFail("a table event was expected, not " + KindOf(v))
	}
	switch {
	case tagged.Tag == "schema" && len(tagged.Fields) == 1:
		columns, f := schemaColumns(tagged.Fields, t.rt.Limits())
		if f != nil {
			return shared.Continue, f
		}
		t.columns = columns
		return t.table.TableEvent(shared.TableEvent{Kind: shared.TableSchema, Columns: t.columns})
	case tagged.Tag == "row" && len(tagged.Fields) == 1:
		cells, f := rowCells(t.rt, tagged.Fields, t.cells)
		t.cells = cells
		if f != nil {
			return shared.Continue, f
		}
		if t.countRows {
			t.metrics.Rows.Add(1)
		}
		return t.table.TableEvent(shared.TableEvent{Kind: shared.TableRow, Cells: t.cells})
	case tagged.Tag == "table-end" && len(tagged.Fields) == 0:
		t.ended = true
		return t.table.TableEvent(shared.TableEvent{Kind: shared.TableEnd})
	}
	return shared.Continue, shared.ProtocolFail("a table event was expected, not " + brief(v))
}

func (t *taggedToTable) End() (shared.Flow, *Fail) {
	if t.ended {
		return shared.Continue, nil
	}
	return shared.Continue, shared.ProtocolFail("the table events ended without table-end")
}

type phase uint8

const (
	phaseBeforeSchema phase = iota
	phaseRows
	phaseDone
)

// csvTableStage is `csv-table options events`: the protocol validator of
// spec section 13.2 in the library's own csv. It checks what the host's
// CSV renderer checks of a table result, in the same order and with the
// same codes (the adapter above, then the renderer: the shape of each
// event, max_columns and the labels, then one schema first, at least one
// column, rows as wide as the schema, one table-end), and passes each
// event on unchanged for the text to render; the cells' own checks (a
// number's lexeme, a missing value) are the text's `scalar-text`, as they
// are the renderer's.
type csvTableStage struct {
	rt      *Runtime
	metrics *shared.Metrics
	// noColumns is whether the options say (entry :no-columns :empty): a
	// table of no columns passes, for the text to write as the empty
	// document.
	noColumns bool
	// countRows is whether this stage counts the rows in metrics.Rows:
	// when no later table stage does (Lowering.items).
	countRows bool
	down      ItemSink
	phase     phase
	width     int
	rows      uint64
	cells     []shared.Cell
}

func (c *csvTableStage) Item(v Val) (shared.Flow, *Fail) {
	tagged, ok := v.(*TaggedVal)
	if !ok {
		return shared.Continue, shared.ProtocolFail("a table event was expected, not " + KindOf(v))
	}
	switch {
	case tagged.Tag == "schema" && len(tagged.Fields) == 1:
		columns, f := schemaColumns(tagged.Fields, c.rt.Limits())
		if f != nil {
			return shared.Continue, f
		}
		switch c.phase {
		case phaseRows:
			return shared.Continue, shared.ProtocolFail("a second schema")
		case phaseDone:
			return shared.Continue, shared.ProtocolFail("a schema after the end")
		}
		if len(columns) == 0 && !c.noColumns {
			return shared.Continue, NewFail(CodeTargetValueUnrepresentable, "a table with no columns has no CSV form")
		}
		c.width = len(columns)
		c.phase = phaseRows
	case tagged.Tag == "row" && len(tagged.Fields) == 1:
		cells, f := rowCells(c.rt, tagged.Fields, c.cells)
		c.cells = cells
		if f != nil {
			return shared.Continue, f
		}
		switch c.phase {
		case phaseBeforeSchema:
			return shared.Continue, shared.ProtocolFail("a row before the schema")
		case phaseDone:
			return shared.Continue, shared.ProtocolFail("a row after the end")
		}
		if len(c.cells) != c.width {
			return shared.Continue, shared.ProtocolFail(fmt.Sprintf("row %d has %d cells; the schema has %d columns",
				c.rows+1, len(c.cells), c.width))
		}
		c.rows++
		if c.countRows {
			c.metrics.Rows.Add(1)
		}
	case tagged.Tag == "table-end" && len(tagged.Fields) == 0:
		switch c.phase {
		case phaseRows:
			c.phase = phaseDone
		case phaseBeforeSchema:
			return shared.Continue, shared.ProtocolFail("the end before the schema")
		default:
			return shared.Continue, shared.ProtocolFail("a second end")
		}
	default:
		return shared.Continue, shared.ProtocolFail("a table event was expected, not " + brief(v))
	}
	return c.down.Item(v)
}

func (c *csvTableStage) End() (shared.Flow, *Fail) {
	if c.phase != phaseDone {
		return shared.Continue, shared.ProtocolFail("the table events ended without table-end")
	}
	return c.down.End()
}

// checkDelimiter refuses the delimiter a csv-table's options name when no
// CSV reader could take it: the quote, a line break or NUL, as the
// renderer refuses one when it is built. A delimiter that is not a string
// is the text's to refuse, where it joins the fields.
func checkDelimiter(options Val) *Fail {
	v, _ := Field(options, "delimiter")
	if d, ok := v.(StrVal); ok && strings.ContainsAny(string(d), "\"\r\n\x00") {
		return NewFail(CodeTargetValueUnrepresentable,
			strconv.Quote(string(d))+" cannot be a CSV delimiter: it holds the quote, a line break or NUL")
	}
	return nil
}

// ---------------------------------------------------------------------------
// Text stages
// ---------------------------------------------------------------------------

// concatMapStage is `concat-map` over a stream: each item's text, rendered
// whole, and the end of a record after it.
type concatMapStage struct {
	rt      *Runtime
	f       Fn
	at      SourceSpan
	out     shared.TextOut
	scratch *scratch
}

func (c *concatMapStage) Item(v Val) (shared.Flow, *Fail) {
	text, f := c.rt.Apply(c.f, []Val{v}, c.at)
	if f != nil {
		return shared.Continue, f
	}
	c.scratch.text.Reset()
	if f := writeFinite(c.rt, text, c.scratch); f != nil {
		return shared.Continue, c.rt.FailAt(f, c.at)
	}
	if f := c.out.WriteStr(c.scratch.text.String()); f != nil {
		return shared.Continue, f
	}
	if f := shared.RecordEnd(c.out); f != nil {
		return shared.Continue, f
	}
	return shared.Continue, nil
}

func (c *concatMapStage) End() (shared.Flow, *Fail) {
	if f := c.out.Flush(); f != nil {
		return shared.Continue, f
	}
	return shared.Continue, nil
}

// joinStage is `join` over a stream: each item is one logical item of the
// join; :record-end is none, and marks the end of a record.
type joinStage struct {
	rt      *Runtime
	join    shared.JoinOut
	scratch *scratch
}

func (j *joinStage) Item(v Val) (shared.Flow, *Fail) {
	if isRecordEnd(v) {
		return shared.Continue, shared.RecordEnd(j.join)
	}
	j.scratch.text.Reset()
	if f := writeFinite(j.rt, v, j.scratch); f != nil {
		return shared.Continue, f
	}
	if f := j.join.ItemStart(); f != nil {
		return shared.Continue, f
	}
	if f := j.join.WriteStr(j.scratch.text.String()); f != nil {
		return shared.Continue, f
	}
	if f := j.join.ItemEnd(); f != nil {
		return shared.Continue, f
	}
	return shared.Continue, nil
}

func (j *joinStage) End() (shared.Flow, *Fail) {
	if f := j.join.Flush(); f != nil {
		return shared.Continue, f
	}
	return shared.Continue, nil
}

// framed is `concat` around one live text: the finite items before it are
// written before its first fragment or record end (or at the flush, when
// it wrote nothing), the items after it at the flush, before the output
// beneath is flushed.
type framed struct {
	rt     *Runtime
	inner  shared.TextOut
	prefix []Val
	suffix []Val
	// started and finished say the prefix and the suffix are written.
	started  bool
	finished bool
}

func (fr *framed) start() *Fail {
	if fr.started {
		return nil
	}
	fr.started = true
	for _, item := range fr.prefix {
		if f := writeFinite(fr.rt, item, fr.inner); f != nil {
			return f
		}
	}
	return nil
}

func (fr *framed) WriteStr(s string) *Fail {
	if f := fr.start(); f != nil {
		return f
	}
	return fr.inner.WriteStr(s)
}

func (fr *framed) Flush() *Fail {
	if f := fr.start(); f != nil {
		return f
	}
	if !fr.finished {
		fr.finished = true
		for _, item := range fr.suffix {
			if f := writeFinite(fr.rt, item, fr.inner); f != nil {
				return f
			}
		}
	}
	return fr.inner.Flush()
}

func (fr *framed) HasCommitted() bool { return fr.inner.HasCommitted() }

func (fr *framed) RecordEnd() *Fail {
	if f := fr.start(); f != nil {
		return f
	}
	return shared.RecordEnd(fr.inner)
}

// finiteTextSink is a text that never reaches the input: the events are
// consumed and the text is written at the end.
type finiteTextSink struct {
	rt   *Runtime
	text Val
	out  shared.TextOut
}

func (s *finiteTextSink) Event(ev shared.Event) (shared.Flow, *Fail) {
	if ev.Kind == shared.End {
		if f := writeFinite(s.rt, s.text, s.out); f != nil {
			return shared.Continue, f
		}
		if f := s.out.Flush(); f != nil {
			return shared.Continue, f
		}
	}
	return shared.Continue, nil
}

// ---------------------------------------------------------------------------
// The lowering
// ---------------------------------------------------------------------------

// Lowering lowers plans for one run: the runtime the stages call back
// into, and the limits and metrics the transduce stages take.
type Lowering struct {
	rt      *Runtime
	limits  shared.Limits
	metrics *shared.Metrics
	// head is the chain's head when the program reads parser events: set
	// where parser-events is lowered, taken by ParserHead.
	head shared.ParserSink
}

// keepParserHead keeps head for the host to push the parser events into:
// the one reader of them a program may have. The JSON-events side of the
// chain the caller answers is never driven.
func (l *Lowering) keepParserHead(head shared.ParserSink) (shared.Sink, *Fail) {
	if l.head != nil {
		return nil, NewFail(CodeStreamReused, "reused: the parser events are read twice")
	}
	l.head = head
	return parserEventsOnly{}, nil
}

// ParserHead is what the host pushes parser events into, once SinkAs has
// answered events, the chain's JSON-events head: the head parser-events
// was lowered to, or, when the program reads nothing of its input, the
// parse's end alone into events.
func (l *Lowering) ParserHead(events shared.Sink) shared.ParserSink {
	head := l.head
	l.head = nil
	return &parserHead{head: head, events: events}
}

// NewLowering is a lowering over rt, under limits, counting into metrics
// (a fresh set when nil).
func NewLowering(rt *Runtime, limits shared.Limits, metrics *shared.Metrics) *Lowering {
	if metrics == nil {
		metrics = shared.NewMetrics()
	}
	return &Lowering{rt: rt, limits: limits, metrics: metrics}
}

// Sink is the sink for a program's result over out. render is the host's
// choice for a stream result; a text result takes none (`render_of_text`).
// A stream of items is taken for a table's rows, as the runtime cannot
// tell an item's shape before it arrives; SinkAs takes the checker's word
// instead.
func (l *Lowering) Sink(result Val, out shared.TextOut, render Renderer) (shared.Sink, *Fail) {
	output := OutputText
	if x, ok := result.(StreamVal); ok {
		output = OutputTableRows
		if x.Plan.Protocol() == ProtocolJSONEvents {
			output = OutputJsonEvents
		}
	}
	return l.SinkAs(result, out, render, output)
}

// SinkAs is Sink with output, what the checker decided the result is: a
// stream of items the runtime cannot tell the protocol of is JSON events
// when the checker typed every item an event (a rewritten tree), and a
// table's rows otherwise.
func (l *Lowering) SinkAs(result Val, out shared.TextOut, render Renderer, output Output) (shared.Sink, *Fail) {
	switch x := result.(type) {
	// A string is a text where a text is expected (spec 10.4).
	case StrVal:
		return l.SinkAs(TextVal{Plan: litPlan(string(x))}, out, render, output)
	case TextVal:
		if render != RenderDefault {
			return nil, NewFail(CodeDSLTypeError,
				"render_of_text: the program renders its own text; --render applies to a table or JSON events result")
		}
		return l.text(x.Plan, out)
	case StreamVal:
		protocol := x.Plan.Protocol()
		// A rewritten tree: the checker saw every item an event.
		if protocol == ProtocolItems && output == OutputJsonEvents {
			protocol = ProtocolJSONEvents
		}
		if render == RenderDefault {
			render = RenderCSV
			if protocol == ProtocolJSONEvents {
				render = RenderJSON
			}
		}
		switch {
		case protocol == ProtocolJSONEvents && render == RenderJSON:
			return l.events(x.Plan, l.rt.Renderers().JSON(out, jsonOptions()))
		case protocol == ProtocolJSONEvents:
			return nil, protocolMismatch("csv renders table events; the program's result is JSON events (render it as json, or make a table of it with table-from-json)")
		case render == RenderCSV:
			renderer, f := l.rt.Renderers().CSV(out, shared.DefaultCSVOptions())
			if f != nil {
				return nil, f
			}
			return l.table(x.Plan, renderer, false)
		}
		json := l.rt.Renderers().JSON(out, jsonOptions())
		return l.table(x.Plan, l.rt.Renderers().RecordsToJSON(json), false)
	}
	return nil, typeError("export must answer a text or a stream, not " + KindOf(result))
}

// EventsInto is the sink for a program's JSON-events result whose events go
// into the host's own down rather than to a renderer: the host's stages
// take them as a source's (a renderer of its own, a scope over a path),
// and down sees the program's End after the root. output is the checker's
// word, as for SinkAs. A result that is a text or a table is refused
// (`protocol_mismatch`).
func (l *Lowering) EventsInto(result Val, output Output, down shared.Sink) (shared.Sink, *Fail) {
	x, ok := result.(StreamVal)
	if !ok {
		return nil, protocolMismatch("the program's result is " + KindOf(result) + ", and a host takes JSON events into its own sink")
	}
	protocol := x.Plan.Protocol()
	if protocol == ProtocolJSONEvents || (protocol == ProtocolItems && output == OutputJsonEvents) {
		return l.events(x.Plan, down)
	}
	return nil, protocolMismatch("the program's result is a table, and a host takes JSON events into its own sink")
}

func (l *Lowering) text(p *Plan, out shared.TextOut) (shared.Sink, *Fail) {
	if !p.IsLive() {
		return &finiteTextSink{rt: l.rt, text: TextVal{Plan: p}, out: out}, nil
	}
	switch {
	case p.Kind == PlanJSON:
		json := l.rt.Renderers().JSON(out, jsonOptions())
		if p.NonFinite == NonFiniteNull {
			return l.events(p.Source, &nullNonFinite{next: json})
		}
		return l.events(p.Source, json)
	case p.Kind == PlanConcatMap && p.Seq.IsStream():
		return l.items(p.Seq.Stream, &concatMapStage{rt: l.rt, f: p.F, at: p.At, out: out, scratch: newScratch(l.limits)}, false)
	case p.Kind == PlanJoin && p.Seq.IsStream():
		return l.items(p.Seq.Stream, &joinStage{rt: l.rt, join: l.rt.Renderers().Join(out, p.Sep), scratch: newScratch(l.limits)}, false)
	case p.Kind == PlanConcat:
		inner, ok := p.Items[p.Live].(TextVal)
		if !ok {
			return nil, typeError("concat: a stream is not a text")
		}
		fr := &framed{
			rt:     l.rt,
			inner:  out,
			prefix: append([]Val(nil), p.Items[:p.Live]...),
			suffix: append([]Val(nil), p.Items[p.Live+1:]...),
		}
		return l.text(inner.Plan, fr)
	case p.Kind == PlanReplace:
		if inner, ok := p.Text.(TextVal); ok {
			return l.text(inner.Plan, l.rt.Renderers().ReplaceText(out, p.From, p.To))
		}
	}
	return nil, typeError(PlanName(p) + " is not a text")
}

func (l *Lowering) events(p *Plan, sink shared.Sink) (shared.Sink, *Fail) {
	switch p.Kind {
	case PlanInput:
		return sink, nil
	// `records` ends a table: after it the rows are JSON events, of which
	// a later table makes rows of its own.
	case PlanRecords:
		return l.table(p.Source, l.rt.Renderers().RecordsToJSON(sink), false)
	// `as-events` names what the arm below does for any items plan: the
	// program's items, each an event, as JSON events.
	case PlanAsEvents:
		return l.events(p.Source, sink)
	// The events as they are, held to a tree's contract in front of
	// whatever takes them.
	case PlanTreeContract:
		return l.events(p.Source, shared.NewTreeContract(sink))
	// A stream whose items may be events (`events` itself, or a
	// `scan-emit`, `map` or `filter` over anything): each item is turned
	// back into an event as the stream runs, the reverse of `events`, so a
	// program can rewrite a document event by event, or build events, and
	// hand them to any taker of JSON events.
	case PlanEvents, PlanScanEmit, PlanMap, PlanFilter:
		// The source's three limits hold on the events a program made as
		// on the source's own, through the source's guard: a document
		// built deeper than max_depth, a key longer than max_key_bytes or
		// a scalar past max_scalar_bytes is refused where it arrives,
		// whatever the input held. The guard counts into metrics of its
		// own: the source's count the source's events, and these are the
		// program's.
		guarded := l.rt.Routers().Guarded(sink, l.limits, l.rt.Abort(), shared.NewMetrics())
		return l.items(p, &taggedToJSON{down: guarded}, false)
	}
	// A stream that never yields events: `select`'s values, `route`'s
	// selections, a table's events as items.
	return nil, protocolMismatch(PlanName(p) + " yields a stream of items where JSON events were expected")
}

// table is the sink for the table plan p yields, handing its events to
// table. countedLater is as for Lowering.items: whether the rows table
// receives are counted after it rather than here.
func (l *Lowering) table(p *Plan, table shared.TableSink, countedLater bool) (shared.Sink, *Fail) {
	switch p.Kind {
	case PlanInput, PlanRecords, PlanAsEvents, PlanTreeContract:
		return nil, protocolMismatch("table events were expected, not JSON events (table-from-json makes a table of them)")
	}
	return l.items(p, &taggedToTable{rt: l.rt, metrics: l.metrics, countRows: !countedLater, table: table}, true)
}

// items is the sink for the stream of items p yields, handing them to down.
//
// countedLater says whether a table stage after down counts the rows these
// items carry. Each row is counted in metrics.Rows once, by the last table
// stage it passes: the adapter to a renderer, or a csv-table whose rows
// reach no later table stage. The flag passes through a map, a filter and
// a scan-emit unchanged, so a row a filter drops is not counted and one a
// scan adds is; a table stage sets it for its own source; a text over
// items, and `records`, clear it. The library's table, a scan-emit, counts
// none.
func (l *Lowering) items(p *Plan, down ItemSink, countedLater bool) (shared.Sink, *Fail) {
	switch p.Kind {
	case PlanRoute:
		// A capture bounded by a named limit takes the host's value for
		// it: the plan was built under the defaults.
		specs := make([]shared.CaptureSpec, len(p.Specs))
		for i, spec := range p.Specs {
			if spec.Budget != nil {
				budget := *spec.Budget
				if bytes, ok := captureBudget(l.limits, budget.Name); ok {
					budget.Bytes = bytes
				}
				spec.Budget = &budget
			}
			specs[i] = spec
		}
		router, f := l.rt.Routers().Router(specs, l.limits, l.rt.Duplicates(), l.metrics, &routeToItems{down: down})
		if f != nil {
			return nil, f
		}
		return l.events(p.Source, router)
	case PlanSelect:
		router, f := l.rt.Routers().Router([]shared.CaptureSpec{shared.MaterializeSpec("selected", p.Selector)},
			l.limits, l.rt.Duplicates(), l.metrics, &routeToItems{down: down, valuesOnly: true})
		if f != nil {
			return nil, f
		}
		return l.events(p.Source, router)
	case PlanEvents:
		return l.events(p.Source, &eventsToItems{down: down})
	case PlanParserEvents:
		// The host's parser events, the one source parser-events reads (its
		// native refuses any other): the head is kept for the host, which
		// pushes the rule passes into it, and the JSON events side of the
		// chain is never driven.
		if p.Source.Kind != PlanParserInput {
			return nil, protocolMismatch("parser-events reads the host's parser events alone")
		}
		return l.keepParserHead(&parserEventsToItems{down: down})
	case PlanParserInput:
		return nil, protocolMismatch("the parser's rule passes are read item by item through parser-events")
	case PlanScanEmit:
		stage, f := newScanStage(l.rt, p.Init, p.Step, p.Finish, p.At, l.metrics, down)
		if f != nil {
			return nil, f
		}
		return l.items(p.Source, stage, countedLater)
	case PlanMap:
		return l.items(p.Source, &mapStage{rt: l.rt, f: p.F, at: p.At, down: down}, countedLater)
	case PlanFilter:
		return l.items(p.Source, &filterStage{rt: l.rt, f: p.F, at: p.At, down: down}, countedLater)
	case PlanCsvTable:
		if f := checkDelimiter(p.Options); f != nil {
			return nil, f
		}
		empty, f := NoColumnsEmpty(p.Options)
		if f != nil {
			return nil, f
		}
		return l.items(p.Source, &csvTableStage{rt: l.rt, metrics: l.metrics, noColumns: empty, countRows: !countedLater, down: down}, true)
	case PlanInput, PlanRecords, PlanAsEvents, PlanTreeContract:
		return nil, protocolMismatch("JSON events cannot be read item by item; select or route what the stream should yield, or read its events")
	}
	return nil, typeError(PlanName(p) + " is a text, not a stream")
}
