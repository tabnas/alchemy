// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"fmt"
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
//   - table (TableRows/1): the native table transducer (TableFromJSON)
//     over events, or an adapter reading the tagged `schema`, `row` and
//     `table-end` values an interpreted stream yields;
//   - items (a stream of values): a Router over events for `route` and
//     `select`, eventsToItems for `events`, scanStage for `scan-emit`,
//     per-item stages for `map` and `filter`, and an adapter turning
//     native table events into the tagged values a program
//     pattern-matches;
//   - text: CSVRenderer and JSONRenderer over their protocols, `concat-map`
//     and `join` over a stream of items, `replace-text` as ReplaceText,
//     and `concat` around one live text as a frame that writes its prefix
//     before the first fragment and its suffix at the flush.
//
// A text that does not reach the input is finite and is written whole
// wherever it lands (writeFinite): a row of an interpreted CSV is rendered
// into a scratch string and written as one fragment, so a failure in the
// middle of a row leaves no half row behind, as the native renderer
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

// isTableBinding is whether a value has the shape of the standard table
// binding: a record with :columns and :rows selectors and a :column
// function, or with :columns the keyword :infer (the columns are the first
// row's keys, and :column is not read) and a :rows selector.
func isTableBinding(v Val) bool {
	r, ok := v.(*RecordVal)
	if !ok {
		return false
	}
	rowsV, _ := r.Get("rows")
	_, rows := rowsV.(*SelectorVal)
	columns, _ := r.Get("columns")
	switch c := columns.(type) {
	case *SelectorVal:
		column, _ := r.Get("column")
		_, fn := column.(Fn)
		return rows && fn
	case KeywordVal:
		return rows && c == "infer"
	}
	return false
}

// isInferred is whether a table binding takes its columns from the first
// row: :columns is the keyword :infer.
func isInferred(binding Val) bool {
	v, _ := Field(binding, "columns")
	k, ok := v.(KeywordVal)
	return ok && k == "infer"
}

// csvOptions is the renderer's dialect for a `csv-options` record, when
// every field has a value the renderer accepts: :delimiter one character,
// :newline CRLF or LF, :header a boolean, :null-text a string, :missing
// :error or a string. Any other record runs the library's own csv.
func csvOptions(v Val) (shared.CSVOptions, bool) {
	r, ok := v.(*RecordVal)
	if !ok {
		return shared.CSVOptions{}, false
	}
	options := shared.DefaultCSVOptions()
	d, _ := r.Get("delimiter")
	delimiter, ok := d.(StrVal)
	if !ok || utf8.RuneCountInString(string(delimiter)) != 1 {
		return shared.CSVOptions{}, false
	}
	options.Delimiter, _ = utf8.DecodeRuneInString(string(delimiter))
	n, _ := r.Get("newline")
	switch n {
	case StrVal("\r\n"):
		options.Newline = shared.NewlineCRLF
	case StrVal("\n"):
		options.Newline = shared.NewlineLF
	default:
		return shared.CSVOptions{}, false
	}
	h, _ := r.Get("header")
	header, ok := h.(BoolVal)
	if !ok {
		return shared.CSVOptions{}, false
	}
	options.Header = bool(header)
	t, _ := r.Get("null-text")
	nullText, ok := t.(StrVal)
	if !ok {
		return shared.CSVOptions{}, false
	}
	options.NullText = string(nullText)
	m, _ := r.Get("missing")
	switch missing := m.(type) {
	case KeywordVal:
		if missing != "error" {
			return shared.CSVOptions{}, false
		}
		options.Missing = nil
	case StrVal:
		options.Missing = shared.MissingAs(string(missing))
	default:
		return shared.CSVOptions{}, false
	}
	return options, true
}

// csvProfile is a CSV dialect as the report prints it.
func csvProfile(o shared.CSVOptions) CsvProfile {
	p := CsvProfile{Delimiter: o.Delimiter, Newline: o.Newline.String(), Header: o.Header, NullText: o.NullText, MissingIsError: o.Missing == nil}
	if o.Missing != nil {
		p.MissingText = *o.Missing
	}
	return p
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

// cellVal is a cell as a program sees it.
func cellVal(c *shared.Cell) Val {
	switch c.Kind {
	case shared.CellBool:
		return BoolVal(c.Bool)
	case shared.CellNumber:
		return NumVal{Value: c.Value, Lexeme: c.Lexeme, HasLexeme: c.HasLexeme}
	case shared.CellString:
		return StrVal(c.Text)
	case shared.CellMissing:
		return Missing()
	}
	return NullVal{}
}

// labelText is the text of a column's label, one policy for every table:
// a string as it is, a number by its lexeme, a boolean by its name (as
// `scalar-text` would write the same header cell); `missing` is
// MISSING_VALUE, and anything else INPUT_INVALID. The native table binds
// its columns by it, the adapter reading a program's schema values by it,
// and csv-table validates the library's schema by it, so a label is
// accepted or refused alike whichever path renders it.
func labelText(renderers shared.Renderers, label Val) (string, *Fail) {
	switch x := label.(type) {
	case StrVal:
		return string(x), nil
	case NumVal:
		return numberText(renderers, x.Value, x.Lexeme, x.HasLexeme)
	case BoolVal:
		return strconv.FormatBool(bool(x)), nil
	}
	if IsMissing(label) {
		return "", NewFail(CodeMissingValue, "a column has no label")
	}
	return "", shared.InputFail("a column's label must be a string, a number or a boolean, not " + KindOf(label))
}

// label is the label of a schema value's column: a record's :label.
func label(renderers shared.Renderers, column Val) (string, *Fail) {
	l, ok := Field(column, "label")
	if !ok {
		return "", shared.ProtocolFail("a schema column is not a record")
	}
	return labelText(renderers, l)
}

// schemaColumns are the columns of a schema value, as the table protocol
// carries them: a vector of column records, at most max_columns of them.
func schemaColumns(renderers shared.Renderers, fields []Val, limits shared.Limits) ([]shared.PublicColumn, *Fail) {
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
		l, f := label(renderers, c)
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
// `recursion` failure rather than an unbounded recursion.
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
				if f := join.ItemStart(); f != nil {
					return f
				}
				if f := writeFinite(rt, item, join); f != nil {
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
				if f := writeFinite(rt, text, out); f != nil {
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

// scratch is one item's text, assembled whole before it is written, so a
// failure half way through an item (a missing cell in a row) leaves
// nothing of it behind. It is bounded by max_output_bytes: an item longer
// than the whole output may be could never be written, and assembling it
// anyway is what would let a small program exhaust memory under an output
// limit.
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
// row to row, as the native table keeps its bound columns, so
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

// tableToTagged turns native table events into the tagged values a
// program matches on.
type tableToTagged struct {
	down ItemSink
}

func (t *tableToTagged) TableEvent(ev shared.TableEvent) (shared.Flow, *Fail) {
	switch ev.Kind {
	case shared.TableSchema:
		columns := make([]Val, len(ev.Columns))
		for i, c := range ev.Columns {
			r := NewRecord()
			r.Set("label", StrVal(c.Label))
			columns[i] = r
		}
		return t.down.Item(NewTagged("schema", Vector(columns...)))
	case shared.TableRow:
		cells := make([]Val, len(ev.Cells))
		for i := range ev.Cells {
			cells[i] = cellVal(&ev.Cells[i])
		}
		return t.down.Item(NewTagged("row", Vector(cells...)))
	}
	flow, f := t.down.Item(NewTagged("table-end"))
	if f != nil || flow == shared.Stop {
		return flow, f
	}
	return t.down.End()
}

// cellBound holds the native table's string cells to max_scalar_bytes, as
// the interpreted path holds a cell's text to it. A document's own strings
// are within that bound already (the source refuses a longer one), so a
// longer string cell here is the compact JSON text of a vector or a
// record, which the library's `scalar-text` bounds the same way; so the
// two paths fail alike, naming the same limit.
//
// The native table adds each row to metrics.Rows as it hands it on. When
// the row is not the native table's to count (Lowering.items: its rows
// become items, which a later table stage counts if they reach one),
// uncount takes that back as the row arrives, before anything else sees
// it, so the count is the interpreted path's: the library's table is a
// scan-emit, which counts nothing.
type cellBound struct {
	max     int
	down    shared.TableSink
	metrics *shared.Metrics
	uncount bool
}

func (c *cellBound) TableEvent(ev shared.TableEvent) (shared.Flow, *Fail) {
	if ev.Kind == shared.TableRow {
		if c.uncount {
			c.metrics.Rows.Add(^uint64(0))
		}
		for i := range ev.Cells {
			if ev.Cells[i].Kind == shared.CellString && len(ev.Cells[i].Text) > c.max {
				return shared.Continue, shared.LimitFail("max_scalar_bytes", uint64(c.max),
					fmt.Sprintf("a cell's JSON text holds more than %d bytes", c.max))
			}
		}
	}
	return c.down.TableEvent(ev)
}

// taggedToTable reads the tagged values an interpreted stream yields as
// native table events. One schema first, rows, one table-end: the renderer
// beneath validates the sequence; what is validated here is that each item
// is a table event at all, of the table protocol's shape (at most
// max_columns columns, labels by labelText), and that the stream did end
// with table-end. It is the last table stage before a renderer, so it
// counts each row in metrics.Rows, as the native table counts the rows it
// hands a renderer, unless countRows says a later stage does
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
		columns, f := schemaColumns(t.rt.Renderers(), tagged.Fields, t.rt.Limits())
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
// spec section 13.2 in the library's own csv. It checks exactly what the
// native path checks, in the same order and with the same codes (the
// adapter above, then the CSV renderer: the shape of each event,
// max_columns and the labels, then one schema first, at least one column,
// rows as wide as the schema, one table-end), and passes each event on
// unchanged for the text to render; the cells' own checks (a number's
// lexeme, a missing value) are the text's `scalar-text`, as they are the
// renderer's.
type csvTableStage struct {
	rt      *Runtime
	metrics *shared.Metrics
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
		columns, f := schemaColumns(c.rt.Renderers(), tagged.Fields, c.rt.Limits())
		if f != nil {
			return shared.Continue, f
		}
		switch c.phase {
		case phaseRows:
			return shared.Continue, shared.ProtocolFail("a second schema")
		case phaseDone:
			return shared.Continue, shared.ProtocolFail("a schema after the end")
		}
		if len(columns) == 0 {
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
// whole.
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
	return shared.Continue, nil
}

func (c *concatMapStage) End() (shared.Flow, *Fail) {
	if f := c.out.Flush(); f != nil {
		return shared.Continue, f
	}
	return shared.Continue, nil
}

// joinStage is `join` over a stream: each item is one logical item of the
// join.
type joinStage struct {
	rt      *Runtime
	join    shared.JoinOut
	scratch *scratch
}

func (j *joinStage) Item(v Val) (shared.Flow, *Fail) {
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
// written before its first fragment (or at the flush, when it wrote
// nothing), the items after it at the flush, before the output beneath is
// flushed.
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
func (l *Lowering) Sink(result Val, out shared.TextOut, render Renderer) (shared.Sink, *Fail) {
	switch x := result.(type) {
	// A string is a text where a text is expected (spec 10.4).
	case StrVal:
		return l.Sink(TextVal{Plan: litPlan(string(x))}, out, render)
	case TextVal:
		if render != RenderDefault {
			return nil, NewFail(CodeDSLTypeError,
				"render_of_text: the program renders its own text; --render applies to a table or JSON events result")
		}
		return l.text(x.Plan, out)
	case StreamVal:
		protocol := x.Plan.Protocol()
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

func (l *Lowering) text(p *Plan, out shared.TextOut) (shared.Sink, *Fail) {
	if !p.IsLive() {
		return &finiteTextSink{rt: l.rt, text: TextVal{Plan: p}, out: out}, nil
	}
	switch {
	case p.Kind == PlanCsv:
		options, ok := csvOptions(p.Options)
		if !ok {
			return nil, typeError("csv: the options record does not map to the renderer's dialect")
		}
		renderer, f := l.rt.Renderers().CSV(out, options)
		if f != nil {
			return nil, f
		}
		return l.table(p.Source, renderer, false)
	case p.Kind == PlanJSON:
		return l.events(p.Source, l.rt.Renderers().JSON(out, jsonOptions()))
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
	case PlanTableFromJSON:
		binding, f := l.tableBinding(p.Binding, p.At)
		if f != nil {
			return nil, f
		}
		bounded := &cellBound{max: l.limits.MaxScalarBytes, down: table, metrics: l.metrics, uncount: countedLater}
		transducer, f := l.rt.Routers().TableFromJSON(binding, l.limits, l.rt.Duplicates(), l.metrics, bounded)
		if f != nil {
			return nil, f
		}
		return l.events(p.Source, transducer)
	case PlanInput, PlanRecords:
		return nil, protocolMismatch("table events were expected, not JSON events (table-from-json makes a table of them)")
	}
	return l.items(p, &taggedToTable{rt: l.rt, metrics: l.metrics, countRows: !countedLater, table: table}, true)
}

// items is the sink for the stream of items p yields, handing them to down.
//
// countedLater says whether a table stage after down counts the rows these
// items carry. Each row is counted in metrics.Rows once, by the last table
// stage it passes, as the interpreted path counts it: the adapter to a
// renderer, or a csv-table whose rows reach no later table stage, or the
// native table when it hands its rows straight to a renderer or to
// `records`. The flag passes through a map, a filter and a scan-emit
// unchanged, so a row a filter drops is not counted and one a scan adds
// is; a table stage sets it for its own source; a text over items, and
// `records`, clear it. The native table's rows that become items are never
// its own to count: the library's table, a scan-emit, counts none.
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
	case PlanTableFromJSON:
		return l.table(p, &tableToTagged{down: down}, true)
	case PlanCsvTable:
		if f := checkDelimiter(p.Options); f != nil {
			return nil, f
		}
		return l.items(p.Source, &csvTableStage{rt: l.rt, metrics: l.metrics, countRows: !countedLater, down: down}, true)
	case PlanInput, PlanRecords:
		return nil, protocolMismatch("JSON events cannot be read item by item; select or route what the stream should yield, or read its events")
	}
	return nil, typeError(PlanName(p) + " is a text, not a stream")
}

// tableBinding is the native transducer's binding from the standard
// binding record. The column function is the program's, applied to each
// descriptor as the metadata completes; its record must carry a string
// :label and a :source selector naming one location. An inferred binding
// is the transducer's InferSchema: the first row's keys.
func (l *Lowering) tableBinding(binding Val, at SourceSpan) (shared.TableBinding, *Fail) {
	get := func(key string) Val {
		v, ok := Field(binding, key)
		if !ok {
			return Missing()
		}
		return v
	}
	if isInferred(binding) {
		rows, ok := get("rows").(*SelectorVal)
		if !ok {
			return shared.TableBinding{}, l.rt.FailAt(typeError("table-from-json: an inferred binding must carry a :rows selector"), at)
		}
		return shared.TableBinding{Schema: shared.InferSchema(), Rows: rows.Selector}, nil
	}
	columns, ok1 := get("columns").(*SelectorVal)
	rows, ok2 := get("rows").(*SelectorVal)
	column, ok3 := get("column").(Fn)
	if !ok1 || !ok2 || !ok3 {
		return shared.TableBinding{}, l.rt.FailAt(typeError(
			"table-from-json: the binding must carry :columns and :rows selectors and a :column function"), at)
	}
	rt := l.rt
	mapper := func(descriptor *shared.Datum) (shared.BoundColumn, *Fail) {
		v, f := rt.Apply(column, []Val{FromDatum(descriptor)}, at)
		if f != nil {
			return shared.BoundColumn{}, f
		}
		return boundColumn(rt.Renderers(), v)
	}
	return shared.TableBinding{Schema: shared.MetadataSchema(columns.Selector, mapper), Rows: rows.Selector}, nil
}

// boundColumn is a column record (:label, :source) as the transducer binds
// it. The label is read as the library's `public-column` reads it (`get
// :label`, so a column function that answers something other than a
// record fails as `get` does) and taken by labelText, the policy every
// table shares.
func boundColumn(renderers shared.Renderers, column Val) (shared.BoundColumn, *Fail) {
	l, f := getField("label", column)
	if f != nil {
		return shared.BoundColumn{}, f
	}
	text, f := labelText(renderers, l)
	if f != nil {
		return shared.BoundColumn{}, f
	}
	source, _ := Field(column, "source")
	s, ok := source.(*SelectorVal)
	if !ok {
		return shared.BoundColumn{}, typeError("table-from-json: a column record must carry a :source selector")
	}
	segments, ok := selectorSegments(s.Selector)
	if !ok {
		return shared.BoundColumn{}, typeError("table-from-json: a column's :source must name one location, not " + s.Selector.String())
	}
	return shared.NewBoundColumn(text, segments), nil
}
