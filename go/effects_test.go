// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// effects_test.go: the plan report, text and JSON, held to check.tsv's
// report rows and to rs/src/effects.rs's tests through the views of the
// plans the interpreter builds for them. The interpreter's own test (pass
// 2) replaces each hand-built view with the one its plan answers, and the
// fixture rows run end to end.

import (
	"path/filepath"
	"strings"
	"testing"

	support "github.com/tabnas/support/go"
)

// sel is a selector as the report prints it.
type sel struct {
	text  string
	multi bool
}

func (s sel) String() string { return s.text }
func (s sel) IsMulti() bool  { return s.multi }

var (
	metadataFields = sel{".response.metadata.fields", false}
	deepRecords    = sel{".response.payload.deep.records[*]", true}
)

// reportCase is a check.tsv report row and the view of the plan the
// interpreter builds for it.
type reportCase struct {
	key  string
	view PlanView
}

var reportCases = []reportCase{
	// The worked example, natively: the table transducer and the CSV
	// renderer with the program's own (default) options.
	{"report: worked example", PlanView{Output: OutputText, Live: true, Stages: []Stage{
		{Kind: StageInput},
		{Kind: StageTableFromJSON, Columns: metadataFields, Rows: deepRecords},
		{Kind: StageCsv, Csv: DefaultCsvProfile()},
	}}},
	// The JSON echo.
	{"def export [input]\n  json input", PlanView{Output: OutputText, Live: true, Stages: []Stage{
		{Kind: StageInput}, {Kind: StageJSON},
	}}},
	// The identity: JSON events the host renders.
	{"def export [input] input", PlanView{Output: OutputJsonEvents, Live: true, Stages: []Stage{
		{Kind: StageInput},
	}}},
	// A text of its own.
	{`def export [input] "done"`, PlanView{Output: OutputText}},
	// The keys of every object, one per line: events, a scan, a join.
	{"report: events", PlanView{Output: OutputText, Live: true, Stages: []Stage{
		{Kind: StageInput}, {Kind: StageEvents}, {Kind: StageScanEmit}, {Kind: StageJoin},
	}}},
	// An inferred binding.
	{"report: inferred binding", PlanView{Output: OutputText, Live: true, Stages: []Stage{
		{Kind: StageInput},
		{Kind: StageTableFromJSON, Rows: sel{"[*]", true}, Inferred: true},
		{Kind: StageCsv, Csv: DefaultCsvProfile()},
	}}},
}

// TestTheReportRowsOfCheckTsv: every report row of check.tsv, through
// the view of its plan, prints exactly the report the fixture pins; and
// every report row has a case here.
func TestTheReportRowsOfCheckTsv(t *testing.T) {
	spec, err := support.LoadSpec(filepath.Join(specDir(t), "check.tsv"), nil)
	if err != nil {
		t.Fatal(err)
	}
	views := map[string]PlanView{}
	for _, c := range reportCases {
		views[c.key] = c.view
	}
	reports := 0
	for _, row := range spec.Rows {
		expected := row.Col(1)
		if support.IsErrorExpect(expected) {
			continue
		}
		reports++
		input := row.Unesc(0)
		key := checkRowKey(input)
		view, ok := views[key]
		if !ok {
			t.Errorf("%s: a report row with no view here", row.Where())
			continue
		}
		want, err := support.ParseExpect(expected)
		if err != nil {
			t.Fatal(err)
		}
		a, f := Analyze(input, "check")
		if f != nil {
			t.Fatalf("%s: %v", row.Where(), f)
		}
		if view.Output != a.Checked.Output {
			t.Errorf("%s: the view's output %s is not the checker's %s", row.Where(), view.Output, a.Checked.Output)
		}
		if got := Explain(a.Resolved, view); got != want {
			t.Errorf("%s:\n got %q\nwant %q", row.Where(), got, want)
		}
	}
	if reports != len(reportCases) {
		t.Errorf("%d report rows, %d views", reports, len(reportCases))
	}
}

func explainOf(t *testing.T, src string, view PlanView) *EffectSummary {
	t.Helper()
	a, f := Analyze(src, "t.alc")
	if f != nil {
		t.Fatalf("%q: %v", src, f)
	}
	return Summarize(a.Resolved, view)
}

const workedExample = binding + "def api-table [input]\n  table-from-json api-binding input\ndef export [input]\n  pipe input\n    api-table\n    csv csv-options\n"

func TestTheWorkedExampleReportsAsJSON(t *testing.T) {
	s := explainOf(t, workedExample, reportCases[0].view)
	j := EncodeJSON(s.JSON())
	for _, part := range []string{
		`{"entry":"export","finite":false,"chain":["api-table","csv"],"output":"Text","passes":1,`,
		`"protocol":["JsonEvents/1","TableRows/1","Text"]`,
		`"retention":[{"scope":"metadata","reason":"the column descriptors, bound once they complete","selector":".response.metadata.fields","limit":"max_metadata_bytes"},{"scope":"record",`,
		`"readiness":"record"`,
		`"order_constraints":[{"before":"metadata completes","after":"first row begins","enforcement":"runtime"},{"before":"schema","after":"rows, then one end","enforcement":"runtime"}]`,
		`"renderer":{"name":"csv","quoting":"always","delimiter":",","newline":"\r\n","header":true,"null_text":"","missing":"error","missing_text":null,"host":false}`,
		`"confidence":"proven"`,
		`"guarantee":"Memory is independent of the number of rows under the configured depth, metadata, record, scalar/key and output limits."`,
	} {
		if !strings.Contains(j, part) {
			t.Errorf("%s\ndoes not hold\n%s", j, part)
		}
	}
}

// The library's twin of the table: a route of two captures, the scan, and
// the library's csv validating the table events it renders.
func TestTheInterpretedLibraryReportsItsRouteAndState(t *testing.T) {
	s := explainOf(t, workedExample, PlanView{Output: OutputText, Live: true, Stages: []Stage{
		{Kind: StageInput},
		{Kind: StageRoute, Captures: []Capture{
			{Selector: metadataFields, Budget: "max_metadata_bytes"},
			{Selector: deepRecords, Budget: "max_record_bytes"},
		}},
		{Kind: StageScanEmit, LibraryTable: true},
		{Kind: StageCsvTable},
		{Kind: StageConcatMap},
	}})
	if strings.Join(s.Protocol, " ") != "JsonEvents/1 Stream<Selected> Stream<Value> TableRows/1 Text" {
		t.Errorf("%v", s.Protocol)
	}
	if s.Selection != "shared prefix matcher, two capture routes" || s.Confidence != Conditional {
		t.Errorf("%s %s", s.Selection, s.Confidence)
	}
	text := s.Text()
	for _, part := range []string{
		"Capture:               .response.metadata.fields, capped at max_metadata_bytes\n",
		"Row capture:           one .response.payload.deep.records[*], capped at max_record_bytes\n",
		"Retained state:        the table's columns once bound, at most max_columns of them, no deeper than max_depth, capped at max_metadata_bytes\n",
		"Contract verification: runtime\n",
		"Ordering contract:     each item before its outputs\n",
	} {
		if !strings.Contains(text, part) {
			t.Errorf("%s\ndoes not hold %q", text, part)
		}
	}
}

func TestTheHostRendersATableAndEvents(t *testing.T) {
	table := strings.Replace(workedExample, "    csv csv-options\n", "", 1)
	s := explainOf(t, table, PlanView{Output: OutputTableRows, Live: true, Stages: []Stage{
		{Kind: StageInput},
		{Kind: StageTableFromJSON, Columns: metadataFields, Rows: deepRecords},
	}})
	text := s.Text()
	for _, part := range []string{
		"export: api-table\n",
		"Protocol:              JsonEvents/1 → TableRows/1 → Text\n",
		"CSV quoting:           always (the host's renderer)\n",
	} {
		if !strings.Contains(text, part) {
			t.Errorf("%s\ndoes not hold %q", text, part)
		}
	}
	if host, _ := s.JSON().Get("renderer"); !strings.Contains(EncodeJSON(host), `"host":true`) {
		t.Errorf("%s", EncodeJSON(host))
	}
	identity := explainOf(t, "def export [input] input", reportCases[2].view).Text()
	if !strings.Contains(identity, "JSON profile:          compact, one document, trailing newline (the host's renderer)\n") {
		t.Errorf("%s", identity)
	}
}

func TestATextOfItsOwnReadsTheInputOnlyToValidateIt(t *testing.T) {
	s := explainOf(t, `def export [input] "done"`, PlanView{Output: OutputText})
	j := s.JSON()
	if v, _ := j.Get("finite"); v != true {
		t.Errorf("finite %v", v)
	}
	if v, _ := j.Get("readiness"); v != "scope-end" {
		t.Errorf("%v", v)
	}
	if v, _ := j.Get("qualification"); EncodeJSON(v) != `["The text is written only after the whole input has validated; an invalid input writes nothing."]` {
		t.Errorf("%s", EncodeJSON(v))
	}
	// A finite text built of calls names them.
	built := explainOf(t, `def export [input] (replace-text "a" "b" (concat "x" "a"))`, PlanView{Output: OutputText}).Text()
	if !strings.HasPrefix(built, "export: concat → replace-text\n") || !strings.Contains(built, "Protocol:              Text\n") {
		t.Errorf("%s", built)
	}
}

func TestEventsAfterASelectingStageKeepItsSelection(t *testing.T) {
	s := explainOf(t, "def export [input] (json input)", PlanView{Output: OutputText, Live: true, Stages: []Stage{
		{Kind: StageInput},
		{Kind: StageTableFromJSON, Columns: metadataFields, Rows: deepRecords},
		{Kind: StageRecords}, {Kind: StageEvents}, {Kind: StageMap}, {Kind: StageJoin},
	}})
	if s.Selection != "shared prefix matcher, two capture routes" {
		t.Errorf("%s", s.Selection)
	}
	plain := explainOf(t, "def export [input] (json input)", PlanView{Output: OutputText, Live: true, Stages: []Stage{
		{Kind: StageInput}, {Kind: StageEvents}, {Kind: StageMap}, {Kind: StageJoin},
	}})
	if plain.Selection != "none; every event is delivered as an item" {
		t.Errorf("%s", plain.Selection)
	}
}

func TestACustomCsvDialectIsReportedAsItRuns(t *testing.T) {
	lf := CsvProfile{Delimiter: ';', Newline: "\n", Header: false, NullText: "NULL", MissingText: "-"}
	s := explainOf(t, workedExample, PlanView{Output: OutputText, Live: true, Stages: []Stage{
		{Kind: StageInput},
		{Kind: StageTableFromJSON, Columns: metadataFields, Rows: deepRecords},
		{Kind: StageCsv, Csv: lf},
	}})
	r, _ := s.JSON().Get("renderer")
	if got := EncodeJSON(r); got != `{"name":"csv","quoting":"always","delimiter":";","newline":"\n","header":false,"null_text":"NULL","missing":"text","missing_text":"-","host":false}` {
		t.Errorf("%s", got)
	}
}

func TestDuplicateMembersFollowThePolicyWhereScopesAreCaptured(t *testing.T) {
	view := reportCases[0].view
	for _, c := range []struct {
		policy Duplicates
		want   string
	}{
		{DuplicatesReject, "rejected in captured scopes (DUPLICATE_MEMBER)"},
		{DuplicatesLastWins, "the last value wins in captured scopes"},
		{DuplicatesFirstWins, "the first value wins in captured scopes"},
	} {
		view.Duplicates = c.policy
		if got := explainOf(t, workedExample, view).Duplicates; got != c.want {
			t.Errorf("%s", got)
		}
	}
	if got := explainOf(t, "def export [input] input", reportCases[2].view).Duplicates; got != "preserved; the events are copied as they arrive, not mapped by key" {
		t.Errorf("%s", got)
	}
}
