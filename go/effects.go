// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"fmt"
	"strconv"
	"strings"
)

// effects.go: the effect summary and the plan report (rs/src/effects.rs).
//
// The summary is computed from the plan the program built, not from its
// text: the plan knows which stages run (the native table transducer or a
// route and a scan-emit, a renderer or the text algebra) and what each
// retains. Explain prints the report in the layout of spec 15.5;
// EffectSummary.JSON is the same information as one object.
//
// The plan is the interpreter's (the second half of the port), so this
// file reads it through a view: PlanView, the facts the summary needs,
// with the plan's stages from the input outward as Stage values. The
// interpreter builds the view from its plan (the Rust `stages` walk, the
// library-table test, the CSV dialect and the inferred-binding test); the
// summary, the text and the JSON are all here.

// StageKind is the kind of one stage of a plan.
type StageKind uint8

// The stages of a plan, as the Rust Plan enum names them.
const (
	StageInput StageKind = iota
	StageLit
	StageRoute
	StageSelect
	StageEvents
	StageScanEmit
	StageMap
	StageFilter
	StageTableFromJSON
	StageRecords
	StageCsvTable
	StageCsv
	StageJSON
	StageConcatMap
	StageJoin
	StageConcat
	StageReplace
)

// Selector is a selector as the report prints it: transduce's Selector
// (`.response.payload.deep.records[*]`) satisfies it.
type Selector interface {
	String() string
	IsMulti() bool
}

// Capture is one capture of a route: its selector, and the Limits field
// that caps it ("" for max_capture_bytes).
type Capture struct {
	Selector Selector
	Budget   string
}

// CsvProfile is the CSV renderer's dialect as the report prints it: the
// program's own options for its csv, or the defaults when the host renders
// a table.
type CsvProfile struct {
	Delimiter      rune
	Newline        string
	Header         bool
	NullText       string
	MissingIsError bool
	MissingText    string
}

// DefaultCsvProfile is the renderer's default dialect: a comma, CRLF, a
// header, an empty null text, a missing cell an error.
func DefaultCsvProfile() CsvProfile {
	return CsvProfile{Delimiter: ',', Newline: "\r\n", Header: true, MissingIsError: true}
}

// Stage is one stage of a plan, with what the summary reads from it.
type Stage struct {
	Kind StageKind
	// StageTableFromJSON: the binding's `columns` and `rows` selectors (nil
	// when the field is not one) and whether its columns are inferred.
	Columns  Selector
	Rows     Selector
	Inferred bool
	// StageRoute: its captures.
	Captures []Capture
	// StageSelect: its selector.
	Selector Selector
	// StageScanEmit: whether the scan is the standard library's table (its
	// table-step, bare or partially applied, from no-schema).
	LibraryTable bool
	// StageCsv: the dialect lowering builds the renderer with.
	Csv CsvProfile
}

// Duplicates is the policy for a member name an object of the source
// repeats.
type Duplicates uint8

// The policies.
const (
	DuplicatesReject Duplicates = iota
	DuplicatesLastWins
	DuplicatesFirstWins
)

// PlanView is what the summary reads from a compiled program: the output
// the checker decided, whether the result reaches the input (a live
// stream or text), the plan's stages from the input outward when it does,
// and the duplicate-member policy.
type PlanView struct {
	Output     Output
	Live       bool
	Stages     []Stage
	Duplicates Duplicates
}

// RetentionScope is what a stage keeps alive.
type RetentionScope uint8

// The scopes.
const (
	RetainMetadata RetentionScope = iota
	RetainRecord
	RetainSubtree
	RetainState
)

func (s RetentionScope) String() string {
	switch s {
	case RetainRecord:
		return "record"
	case RetainSubtree:
		return "subtree"
	case RetainState:
		return "state"
	}
	return "metadata"
}

// Retention is one live retention requirement.
type Retention struct {
	Scope RetentionScope
	// Label is the label the report prints it under.
	Label  string
	Reason string
	// Selector is the selector whose matches are retained, when one
	// applies.
	Selector Selector
	// Limit is the Limits field that caps it, "" when none does.
	Limit string
}

// Readiness is when output is ready.
type Readiness uint8

// The readiness levels.
const (
	ReadyEvent Readiness = iota
	ReadyRecord
	ReadyScopeEnd
)

func (r Readiness) String() string {
	switch r {
	case ReadyRecord:
		return "record"
	case ReadyScopeEnd:
		return "scope-end"
	}
	return "event"
}

// OrderConstraint is an ordering the plan relies on, and who enforces it:
// "static" when the plan cannot violate it, "runtime" when the run checks
// it and fails.
type OrderConstraint struct {
	Before      string
	After       string
	Enforcement string
}

// Confidence is how sure the analysis is of its guarantee.
type Confidence uint8

// The confidences.
const (
	// Proven: every stage is a native with fixed retention under the limits.
	Proven Confidence = iota
	// Conditional: a stage's retention is the program's (a scan-emit state).
	Conditional
)

func (c Confidence) String() string {
	if c == Conditional {
		return "conditional"
	}
	return "proven"
}

// RendererKind is the renderer that writes the text.
type RendererKind uint8

// The renderers.
const (
	RendererCsv RendererKind = iota
	RendererJSON
	RendererText
)

// RendererProfile is the renderer that writes the text: Host says the host
// chose it for a table or events result.
type RendererProfile struct {
	Kind RendererKind
	Host bool
	Csv  CsvProfile
}

// EffectSummary is the effect summary of spec 15.2 with the report's
// other facts.
type EffectSummary struct {
	Entry            string
	Finite           bool
	Chain            []string
	Passes           int
	Protocol         []string
	Selection        string
	Duplicates       string
	Retention        []Retention
	Readiness        Readiness
	OutputOrder      string
	OrderConstraints []OrderConstraint
	Renderer         RendererProfile
	ExternalStorage  string
	Confidence       Confidence
	Guarantee        string
	Qualification    []string
	Output           Output
}

// protocolOf is the protocol a stage produces, as the report names it.
func protocolOf(s StageKind) string {
	switch s {
	case StageInput, StageRecords:
		return "JsonEvents/1"
	case StageTableFromJSON, StageCsvTable:
		return "TableRows/1"
	case StageRoute:
		return "Stream<Selected>"
	case StageSelect:
		return "Stream<Value>"
	case StageEvents:
		return "Stream<Event>"
	case StageScanEmit, StageMap, StageFilter:
		return "Stream<Value>"
	}
	return "Text"
}

// chainOf is the calls on the data-last spine of an export body, innermost
// first, ending where the spine reaches the parameter or a special form.
func chainOf(body *Expr) []string {
	var chain []string
	for here := body; here.Kind == ExprList; {
		head, ok := "", false
		if len(here.Items) > 0 {
			head, ok = here.Items[0].Symbol()
		}
		if !ok {
			break
		}
		switch head {
		case "fn", "let", "if", "match", "def":
			ok = false
		}
		if !ok || len(here.Items) < 2 {
			break
		}
		chain = append(chain, head)
		here = here.Items[len(here.Items)-1]
	}
	for i, j := 0, len(chain)-1; i < j; i, j = i+1, j-1 {
		chain[i], chain[j] = chain[j], chain[i]
	}
	return chain
}

// tabular is whether the plan produces table events, natively or as
// tagged values the host's adapter reads.
func tabular(stages []Stage, output Output) bool {
	for _, s := range stages {
		switch s.Kind {
		case StageTableFromJSON, StageCsvTable, StageCsv, StageRecords:
			return true
		}
	}
	return output == OutputTableRows
}

// duplicatesOf is the duplicate-member line.
func duplicatesOf(stages []Stage, policy Duplicates, finite bool) string {
	if finite {
		return "not examined; the input is only validated"
	}
	for _, s := range stages {
		switch s.Kind {
		case StageRoute, StageSelect, StageTableFromJSON:
			switch policy {
			case DuplicatesLastWins:
				return "the last value wins in captured scopes"
			case DuplicatesFirstWins:
				return "the first value wins in captured scopes"
			}
			return "rejected in captured scopes (DUPLICATE_MEMBER)"
		}
	}
	return "preserved; the events are copied as they arrive, not mapped by key"
}

// passThrough is the selection summary of a plan that selects nothing.
const passThrough = "none; every event passes through"

// count is a small count in words.
func count(n int) string {
	switch n {
	case 0:
		return "no"
	case 1:
		return "one"
	case 2:
		return "two"
	case 3:
		return "three"
	}
	return strconv.Itoa(n)
}

// Summarize is the summary of a program: its resolved definitions (for
// export's chain of calls) and the view of its plan.
func Summarize(resolved *Resolved, view PlanView) *EffectSummary {
	output := view.Output
	// A result that never reaches the input (a string, or a text of the
	// program's own) is written at the end; the source is only validated.
	finite := !view.Live
	var stages []Stage
	if !finite {
		stages = view.Stages
	}
	var chain []string
	if def := resolved.Get("export"); def != nil && def.Value.Kind == ExprList && len(def.Value.Items) == 3 {
		chain = chainOf(def.Value.Items[2])
	}

	if finite {
		return &EffectSummary{
			Entry:           "export",
			Finite:          true,
			Chain:           chain,
			Passes:          1,
			Protocol:        []string{"Text"},
			Selection:       "none; the input is read and validated, and nothing of it is used",
			Duplicates:      duplicatesOf(stages, view.Duplicates, finite),
			Readiness:       ReadyScopeEnd,
			OutputOrder:     "the program's own text, written once the input has validated",
			Renderer:        RendererProfile{Kind: RendererText},
			ExternalStorage: "disabled",
			Confidence:      Proven,
			Guarantee:       "Memory is independent of the document's size under the configured\n  depth and scalar/key limits: nothing of the input is kept. The text\n  is the program's own, written under the output limit.",
			Qualification: []string{
				"The text is written only after the whole input has validated; an\n  invalid input writes nothing.",
			},
			Output: output,
		}
	}

	protocol := []string{"JsonEvents/1"}
	for i, stage := range stages {
		if i == 0 {
			continue
		}
		if p := protocolOf(stage.Kind); protocol[len(protocol)-1] != p {
			protocol = append(protocol, p)
		}
	}
	var renderer RendererProfile
	var last *Stage
	if len(stages) > 0 {
		last = &stages[len(stages)-1]
	}
	switch {
	case last != nil && last.Kind == StageCsv:
		renderer = RendererProfile{Kind: RendererCsv, Csv: last.Csv}
	case last != nil && last.Kind == StageJSON:
		renderer = RendererProfile{Kind: RendererJSON}
	case output == OutputTableRows:
		protocol = append(protocol, "Text")
		renderer = RendererProfile{Kind: RendererCsv, Host: true, Csv: DefaultCsvProfile()}
	case output == OutputJsonEvents:
		protocol = append(protocol, "Text")
		renderer = RendererProfile{Kind: RendererJSON, Host: true}
	default:
		renderer = RendererProfile{Kind: RendererText}
	}

	selection := passThrough
	var retention []Retention
	var orderConstraints []OrderConstraint
	readiness := ReadyEvent
	confidence := Proven
	outputOrder := "source order"
	for _, stage := range stages {
		switch stage.Kind {
		case StageTableFromJSON:
			readiness = ReadyRecord
			outputOrder = "schema first; cells in schema order"
			if stage.Inferred {
				selection = "shared prefix matcher, one capture route"
				retention = append(retention, Retention{
					Scope:  RetainMetadata,
					Label:  "Inferred columns:",
					Reason: "the first row's keys, taken as the columns once it completes, at most max_columns of them",
					Limit:  "max_metadata_bytes",
				})
			} else {
				selection = "shared prefix matcher, two capture routes"
				retention = append(retention, Retention{
					Scope:    RetainMetadata,
					Label:    "Retained metadata:",
					Reason:   "the column descriptors, bound once they complete",
					Selector: stage.Columns,
					Limit:    "max_metadata_bytes",
				})
			}
			retention = append(retention, Retention{
				Scope:    RetainRecord,
				Label:    "Row capture:",
				Reason:   "one row at a time, projected into schema order and released",
				Selector: stage.Rows,
				Limit:    "max_record_bytes",
			})
			if !stage.Inferred {
				orderConstraints = append(orderConstraints, OrderConstraint{
					Before: "metadata completes", After: "first row begins", Enforcement: "runtime",
				})
			}
		case StageRoute:
			plural := "s"
			if len(stage.Captures) == 1 {
				plural = ""
			}
			selection = fmt.Sprintf("shared prefix matcher, %s capture route%s", count(len(stage.Captures)), plural)
			readiness = ReadyRecord
			outputOrder = "selection order: completed matches, in source order"
			for _, spec := range stage.Captures {
				multi := spec.Selector.IsMulti()
				r := Retention{
					Scope:    RetainSubtree,
					Label:    "Capture:",
					Reason:   "one selected scope, materialized whole and released after delivery",
					Selector: spec.Selector,
					Limit:    "max_capture_bytes",
				}
				if multi {
					r.Scope = RetainRecord
					r.Label = "Row capture:"
					r.Reason = "one at a time, one selected scope, materialized whole and released after delivery"
				}
				if spec.Budget != "" {
					r.Limit = spec.Budget
				}
				retention = append(retention, r)
			}
		case StageSelect:
			selection = "shared prefix matcher, one capture route"
			readiness = ReadyRecord
			outputOrder = "selection order: completed matches, in source order"
			retention = append(retention, Retention{
				Scope:    RetainRecord,
				Label:    "Row capture:",
				Reason:   "one selected value at a time, released after delivery",
				Selector: stage.Selector,
				Limit:    "max_capture_bytes",
			})
		case StageEvents:
			// Every event becomes one item as it arrives. A stage before it
			// that selected keeps its own summary.
			if selection == passThrough {
				selection = "none; every event is delivered as an item"
			}
		case StageScanEmit:
			confidence = Conditional
			reason := "what the step returns, no deeper than max_depth"
			if stage.LibraryTable {
				reason = "the table's columns once bound, at most max_columns of them, no deeper than max_depth"
			}
			retention = append(retention, Retention{
				Scope:  RetainState,
				Label:  "Retained state:",
				Reason: reason,
				Limit:  "max_metadata_bytes",
			})
			orderConstraints = append(orderConstraints, OrderConstraint{
				Before: "each item", After: "its outputs", Enforcement: "static",
			})
		}
	}
	if tabular(stages, output) && !(len(stages) > 0 && stages[0].Kind == StageTableFromJSON) {
		// Table events reach a renderer: it validates the protocol.
		has := false
		for _, c := range orderConstraints {
			if c.Before == "schema" {
				has = true
			}
		}
		if !has {
			orderConstraints = append(orderConstraints, OrderConstraint{
				Before: "schema", After: "rows, then one end", Enforcement: "runtime",
			})
		}
	}
	if renderer.Kind == RendererCsv && outputOrder == "source order" {
		outputOrder = "schema first; cells in schema order"
	}

	var guarantee string
	switch {
	case len(retention) == 0:
		guarantee = "Memory is independent of the document's size under the configured\n  depth, scalar/key and output limits: nothing is retained beyond the\n  renderer's nesting stack."
	case confidence == Proven:
		guarantee = "Memory is independent of the number of rows under the configured\n  depth, metadata, record, scalar/key and output limits."
	default:
		guarantee = "Memory is independent of the number of rows under the configured\n  depth, capture, metadata, scalar/key and output limits: the state the\n  step returns is capped at max_metadata_bytes. What one step computes\n  is the program's, bounded by the host's abort flag."
	}
	var qualification []string
	for _, c := range orderConstraints {
		if strings.HasPrefix(c.Before, "metadata") {
			qualification = append(qualification, "Valid JSON that violates the metadata-first contract is rejected.")
			break
		}
	}
	qualification = append(qualification, "A later error can occur after earlier output has been written.")

	return &EffectSummary{
		Entry:            "export",
		Finite:           false,
		Chain:            chain,
		Passes:           1,
		Protocol:         protocol,
		Selection:        selection,
		Duplicates:       duplicatesOf(stages, view.Duplicates, finite),
		Retention:        retention,
		Readiness:        readiness,
		OutputOrder:      outputOrder,
		OrderConstraints: orderConstraints,
		Renderer:         renderer,
		ExternalStorage:  "disabled",
		Confidence:       confidence,
		Guarantee:        guarantee,
		Qualification:    qualification,
		Output:           output,
	}
}

// Text is the report, in the layout of spec 15.5.
func (s *EffectSummary) Text() string {
	type line struct{ label, value string }
	lines := []line{
		{"Source reads:", strconv.Itoa(s.Passes)},
		{"Protocol:", strings.Join(s.Protocol, " → ")},
		{"Selection:", s.Selection},
		{"Duplicate members:", s.Duplicates},
	}
	for _, r := range s.Retention {
		var value strings.Builder
		if r.Selector != nil {
			if r.Scope == RetainRecord {
				value.WriteString("one ")
			}
			value.WriteString(r.Selector.String())
		} else {
			value.WriteString(r.Reason)
		}
		if r.Limit != "" {
			value.WriteString(", capped at " + r.Limit)
		}
		lines = append(lines, line{r.Label, value.String()})
	}
	lines = append(lines, line{"Output order:", s.OutputOrder})
	contract, verification := "none", "static"
	if len(s.OrderConstraints) > 0 {
		c := s.OrderConstraints[0]
		contract = c.Before + " before " + c.After
		for _, o := range s.OrderConstraints {
			if o.Enforcement == "runtime" {
				verification = "runtime"
				break
			}
		}
	}
	lines = append(lines, line{"Ordering contract:", contract}, line{"Contract verification:", verification})
	switch s.Renderer.Kind {
	case RendererCsv:
		v := "always"
		if s.Renderer.Host {
			v = "always (the host's renderer)"
		}
		lines = append(lines, line{"CSV quoting:", v})
	case RendererJSON:
		v := "compact, one document, trailing newline"
		if s.Renderer.Host {
			v += " (the host's renderer)"
		}
		lines = append(lines, line{"JSON profile:", v})
	}
	lines = append(lines, line{"External storage:", s.ExternalStorage})

	var out strings.Builder
	out.WriteString(s.Entry)
	out.WriteString(": ")
	switch {
	case len(s.Chain) == 0 && s.Finite:
		out.WriteString("a text of its own")
	case len(s.Chain) == 0:
		out.WriteString("input")
	}
	out.WriteString(strings.Join(s.Chain, " → "))
	out.WriteString("\n\n")
	for _, l := range lines {
		// `{label:<23}` pads by characters; the labels are ASCII.
		out.WriteString(l.label)
		if pad := 23 - len([]rune(l.label)); pad > 0 {
			out.WriteString(strings.Repeat(" ", pad))
		}
		out.WriteString(l.value)
		out.WriteByte('\n')
	}
	out.WriteString("\nGuarantee:\n  ")
	out.WriteString(s.Guarantee)
	out.WriteString("\n\nQualification:\n")
	for _, q := range s.Qualification {
		out.WriteString("  ")
		out.WriteString(q)
		out.WriteByte('\n')
	}
	return out.String()
}

// JSONObject is a JSON object whose members keep their order, as the Rust
// crate's (serde_json with preserve_order) do; EncodeJSON writes it.
type JSONObject struct {
	Keys   []string
	Values []any
}

func (o *JSONObject) set(key string, value any) *JSONObject {
	o.Keys = append(o.Keys, key)
	o.Values = append(o.Values, value)
	return o
}

// Get is the member key, and whether there is one.
func (o *JSONObject) Get(key string) (any, bool) {
	for i, k := range o.Keys {
		if k == key {
			return o.Values[i], true
		}
	}
	return nil, false
}

// JSON is the same facts as Text, as one object, in the member order the
// Rust crate writes them.
func (s *EffectSummary) JSON() *JSONObject {
	var renderer *JSONObject
	switch s.Renderer.Kind {
	case RendererCsv:
		c := s.Renderer.Csv
		missing, missingText := "text", any(c.MissingText)
		if c.MissingIsError {
			missing, missingText = "error", nil
		}
		renderer = (&JSONObject{}).
			set("name", "csv").
			set("quoting", "always").
			set("delimiter", string(c.Delimiter)).
			set("newline", c.Newline).
			set("header", c.Header).
			set("null_text", c.NullText).
			set("missing", missing).
			set("missing_text", missingText).
			set("host", s.Renderer.Host)
	case RendererJSON:
		renderer = (&JSONObject{}).set("name", "json").set("indent", nil).set("trailing_newline", true).set("host", s.Renderer.Host)
	default:
		renderer = (&JSONObject{}).set("name", "text")
	}
	strs := func(items []string) []any {
		out := make([]any, len(items))
		for i, item := range items {
			out[i] = item
		}
		return out
	}
	retention := make([]any, len(s.Retention))
	for i, r := range s.Retention {
		var selector, limit any
		if r.Selector != nil {
			selector = r.Selector.String()
		}
		if r.Limit != "" {
			limit = r.Limit
		}
		retention[i] = (&JSONObject{}).set("scope", r.Scope.String()).set("reason", r.Reason).set("selector", selector).set("limit", limit)
	}
	constraints := make([]any, len(s.OrderConstraints))
	for i, c := range s.OrderConstraints {
		constraints[i] = (&JSONObject{}).set("before", c.Before).set("after", c.After).set("enforcement", c.Enforcement)
	}
	qualification := make([]any, len(s.Qualification))
	for i, q := range s.Qualification {
		qualification[i] = strings.ReplaceAll(q, "\n  ", " ")
	}
	return (&JSONObject{}).
		set("entry", s.Entry).
		set("finite", s.Finite).
		set("chain", strs(s.Chain)).
		set("output", s.Output.String()).
		set("passes", s.Passes).
		set("protocol", strs(s.Protocol)).
		set("selection", s.Selection).
		set("duplicates", s.Duplicates).
		set("retention", retention).
		set("readiness", s.Readiness.String()).
		set("output_order", s.OutputOrder).
		set("order_constraints", constraints).
		set("renderer", renderer).
		set("external_storage", s.ExternalStorage).
		set("confidence", s.Confidence.String()).
		set("guarantee", strings.ReplaceAll(s.Guarantee, "\n  ", " ")).
		set("qualification", qualification)
}

// EncodeJSON writes a value made of JSONObject, []any, string, bool, int
// and nil as compact JSON, exactly as serde_json::to_string writes the
// same value.
func EncodeJSON(value any) string {
	var b strings.Builder
	encodeJSON(value, &b)
	return b.String()
}

func encodeJSON(value any, b *strings.Builder) {
	switch v := value.(type) {
	case nil:
		b.WriteString("null")
	case bool:
		if v {
			b.WriteString("true")
		} else {
			b.WriteString("false")
		}
	case int:
		b.WriteString(strconv.Itoa(v))
	case string:
		writeJSONString(v, b)
	case []any:
		b.WriteByte('[')
		for i, item := range v {
			if i > 0 {
				b.WriteByte(',')
			}
			encodeJSON(item, b)
		}
		b.WriteByte(']')
	case *JSONObject:
		b.WriteByte('{')
		for i, key := range v.Keys {
			if i > 0 {
				b.WriteByte(',')
			}
			writeJSONString(key, b)
			b.WriteByte(':')
			encodeJSON(v.Values[i], b)
		}
		b.WriteByte('}')
	default:
		b.WriteString(fmt.Sprint(v))
	}
}

// Explain is the report of spec 15.5 for a program and the view of its
// plan.
func Explain(resolved *Resolved, view PlanView) string {
	return Summarize(resolved, view).Text()
}

// ExplainJSON is the report as one JSON object.
func ExplainJSON(resolved *Resolved, view PlanView) *JSONObject {
	return Summarize(resolved, view).JSON()
}
