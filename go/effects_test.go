// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// effects_test.go: the plan report, text and JSON, over the plans the
// programs build (rs/src/effects.rs's tests).

import (
	"strings"
	"testing"

	tt "github.com/tabnas/transduce/go"
)

// workedExample is the spec's program (sections 12.1 and 13.4).
const workedExample = "def column-from-meta [source]\n  record\n    entry :label (get \"title\" source)\n    entry :source\n      as-path\n        get \"path\" source\n\ndef api-binding\n  record\n    entry :columns\n      path \"response\" \"metadata\" \"fields\"\n    entry :rows\n      path \"response\" \"payload\" \"deep\" \"records\" each-index\n    entry :column column-from-meta\n\ndef api-table [input]\n  table-from-json api-binding input\n\ndef export [input]\n  pipe input\n    api-table\n    csv csv-options\n"

// mustCompile is src compiled, or the test's end.
func mustCompile(t testing.TB, src, file string) *Program {
	t.Helper()
	p, f := Compile(src, file, routers, renderers)
	if f != nil {
		t.Fatalf("%s: %v", file, f)
	}
	return p
}

// interpreted is p with the standard compositions run through the
// library's text.
func interpreted(t testing.TB, p *Program) *Program {
	t.Helper()
	q, f := p.WithNative(false)
	if f != nil {
		t.Fatal(f)
	}
	return q
}

func jsonMember(t testing.TB, o *JSONObject, path ...string) string {
	t.Helper()
	var v any = o
	for _, key := range path {
		obj, ok := v.(*JSONObject)
		if !ok {
			t.Fatalf("%v is not an object at %s", v, key)
		}
		if v, ok = obj.Get(key); !ok {
			t.Fatalf("no member %s", key)
		}
	}
	return EncodeJSON(v)
}

func TestTheWorkedExampleReportsInTheSpecLayout(t *testing.T) {
	program := mustCompile(t, workedExample, "export.alc")
	want := "export: api-table → csv\n" +
		"\n" +
		"Source reads:          1\n" +
		"Protocol:              JsonEvents/1 → TableRows/1 → Text\n" +
		"Selection:             shared prefix matcher, two capture routes\n" +
		"Duplicate members:     rejected in captured scopes (DUPLICATE_MEMBER)\n" +
		"Retained metadata:     .response.metadata.fields, capped at max_metadata_bytes\n" +
		"Row capture:           one .response.payload.deep.records[*], capped at max_record_bytes\n" +
		"Output order:          schema first; cells in schema order\n" +
		"Ordering contract:     metadata completes before first row begins\n" +
		"Contract verification: runtime\n" +
		"CSV quoting:           always\n" +
		"External storage:      disabled\n" +
		"\n" +
		"Guarantee:\n" +
		"  Memory is independent of the number of rows under the configured\n" +
		"  depth, metadata, record, scalar/key and output limits.\n" +
		"\n" +
		"Qualification:\n" +
		"  Valid JSON that violates the metadata-first contract is rejected.\n" +
		"  A later error can occur after earlier output has been written.\n"
	if got := program.Explain(); got != want {
		t.Errorf("got\n%s\nwant\n%s", got, want)
	}
	j := program.ExplainJSON()
	for _, c := range []struct {
		path []string
		want string
	}{
		{[]string{"chain"}, `["api-table","csv"]`},
		{[]string{"protocol"}, `["JsonEvents/1","TableRows/1","Text"]`},
		{[]string{"confidence"}, `"proven"`},
		{[]string{"readiness"}, `"record"`},
		{[]string{"renderer", "name"}, `"csv"`},
		{[]string{"output"}, `"Text"`},
	} {
		if got := jsonMember(t, j, c.path...); got != c.want {
			t.Errorf("%v: %s, want %s", c.path, got, c.want)
		}
	}
	text := EncodeJSON(j)
	for _, part := range []string{
		`"retention":[{"scope":"metadata","reason":"the column descriptors, bound once they complete","selector":".response.metadata.fields","limit":"max_metadata_bytes"},{"scope":"record","reason":"one row at a time, projected into schema order and released","selector":".response.payload.deep.records[*]","limit":"max_record_bytes"}]`,
		`"order_constraints":[{"before":"metadata completes","after":"first row begins","enforcement":"runtime"},{"before":"schema","after":"rows, then one end","enforcement":"runtime"}]`,
		`"renderer":{"name":"csv","quoting":"always","delimiter":",","newline":"\r\n","header":true,"null_text":"","missing":"error","missing_text":null,"host":false}`,
		`"guarantee":"Memory is independent of the number of rows under the configured depth, metadata, record, scalar/key and output limits."`,
	} {
		if !strings.Contains(text, part) {
			t.Errorf("%s\ndoes not hold\n%s", text, part)
		}
	}
}

// An inferred binding reads one capture route, retains the columns it
// took from the first row under max_columns, and orders nothing before the
// rows, since there is no metadata; the library's twin routes one capture
// too.
func TestAnInferredBindingReportsOneRoute(t *testing.T) {
	src := "def b (record (entry :columns :infer) (entry :rows (path each-index)))\ndef export [input] (csv csv-options (table-from-json b input))"
	program := mustCompile(t, src, "inferred.alc")
	s := program.Summary()
	if s.Selection != "shared prefix matcher, one capture route" {
		t.Errorf("%s", s.Selection)
	}
	if len(s.Retention) != 2 ||
		s.Retention[0].Scope != RetainMetadata || s.Retention[0].Label != "Inferred columns:" || s.Retention[0].Limit != "max_metadata_bytes" ||
		s.Retention[1].Scope != RetainRecord || s.Retention[1].Label != "Row capture:" || s.Retention[1].Limit != "max_record_bytes" {
		t.Errorf("%+v", s.Retention)
	}
	for _, o := range s.OrderConstraints {
		if o.Before == "metadata completes" {
			t.Errorf("%+v", s.OrderConstraints)
		}
	}
	if !strings.Contains(program.Explain(), "Inferred columns:") {
		t.Error(program.Explain())
	}
	twin := interpreted(t, program).Summary()
	if twin.Selection != "shared prefix matcher, one capture route" {
		t.Errorf("%s", twin.Selection)
	}
	var state *Retention
	for i := range twin.Retention {
		if twin.Retention[i].Scope == RetainState {
			state = &twin.Retention[i]
		}
	}
	if state == nil || !strings.Contains(state.Reason, "at most max_columns") || state.Limit != "max_metadata_bytes" {
		t.Errorf("%+v", state)
	}
}

// A program may run the library's table-step itself. Seeded with columns
// already bound, the scan builds no schema, so nothing holds the columns
// to max_columns and the report does not claim it; seeded with no-schema,
// it is the library's table and does.
func TestATableStepSeededWithColumnsClaimsNoColumnCap(t *testing.T) {
	state := func(init string) Retention {
		src := "def b (record (entry :columns :infer) (entry :rows (path each-index)))\n" +
			"def export [input]\n  join \"\"\n    map (fn [e] \"x\")\n      " +
			"scan-emit " + init + " (partial table-step b) (fn [s] [])\n        " +
			"route (table-captures b) input"
		for _, r := range mustCompile(t, src, "step.alc").Summary().Retention {
			if r.Scope == RetainState {
				return r
			}
		}
		t.Fatalf("%s: no state retained", init)
		return Retention{}
	}
	seeded := state("(ready [])")
	if seeded.Reason != "what the step returns, no deeper than max_depth" || seeded.Limit != "max_metadata_bytes" {
		t.Errorf("%+v", seeded)
	}
	if unbound := state("no-schema"); !strings.Contains(unbound.Reason, "at most max_columns") {
		t.Errorf("%+v", unbound)
	}
}

// events after a stage that selected keeps that stage's selection in the
// report: the stages are read from the input outward, and the events of a
// table's records are still the table's captures.
func TestEventsAfterASelectingStageKeepItsSelection(t *testing.T) {
	src := strings.Replace(workedExample, "    csv csv-options\n", "    records\n    events\n    map (fn [e] \"x\")\n    join \"\"\n", 1)
	s := mustCompile(t, src, "events.alc").Summary()
	if s.Selection != "shared prefix matcher, two capture routes" {
		t.Errorf("%s", s.Selection)
	}
	if !strings.Contains(strings.Join(s.Protocol, " "), "Stream<Event>") {
		t.Errorf("%v", s.Protocol)
	}
	if len(s.Retention) != 2 || s.Retention[0].Scope != RetainMetadata || s.Retention[1].Scope != RetainRecord {
		t.Errorf("%+v", s.Retention)
	}
	plain := mustCompile(t, "def export [input] (join \"\" (map (fn [e] \"x\") (events input)))", "plain.alc")
	if got := plain.Summary().Selection; got != "none; every event is delivered as an item" {
		t.Errorf("%s", got)
	}
}

func TestTheInterpretedLibraryReportsItsRouteAndState(t *testing.T) {
	s := interpreted(t, mustCompile(t, workedExample, "export.alc")).Summary()
	// The library's csv validates the table events it renders
	// (csv-table), so the chain passes through TableRows/1 as the native
	// one does.
	if got := strings.Join(s.Protocol, " "); got != "JsonEvents/1 Stream<Selected> Stream<Value> TableRows/1 Text" {
		t.Errorf("%s", got)
	}
	if s.Selection != "shared prefix matcher, two capture routes" || s.Confidence != Conditional {
		t.Errorf("%s %s", s.Selection, s.Confidence)
	}
	if len(s.Retention) != 3 || s.Retention[0].Scope != RetainSubtree || s.Retention[1].Scope != RetainRecord || s.Retention[2].Scope != RetainState {
		t.Errorf("%+v", s.Retention)
	}
	text := s.Text()
	for _, part := range []string{
		// The library's captures name the limits the native table holds the
		// same scopes to, and the state is capped.
		"Capture:               .response.metadata.fields, capped at max_metadata_bytes\n",
		"Row capture:           one .response.payload.deep.records[*], capped at max_record_bytes\n",
		// Its state is the table's columns, whose count the schema it
		// builds holds to max_columns, as the native table's is.
		"Retained state:        the table's columns once bound, at most max_columns of them, no deeper than max_depth, capped at max_metadata_bytes\n",
		"Contract verification: runtime\n",
		"Ordering contract:     each item before its outputs\n",
	} {
		if !strings.Contains(text, part) {
			t.Errorf("%s\ndoes not hold %q", text, part)
		}
	}
}

func TestTheJSONEchoAndAHostRenderedTable(t *testing.T) {
	echo := mustCompile(t, "def export [input] (json input)", "echo.alc")
	want := "export: json\n" +
		"\n" +
		"Source reads:          1\n" +
		"Protocol:              JsonEvents/1 → Text\n" +
		"Selection:             none; every event passes through\n" +
		"Duplicate members:     preserved; the events are copied as they arrive, not mapped by key\n" +
		"Output order:          source order\n" +
		"Ordering contract:     none\n" +
		"Contract verification: static\n" +
		"JSON profile:          compact, one document, trailing newline\n" +
		"External storage:      disabled\n" +
		"\n" +
		"Guarantee:\n" +
		"  Memory is independent of the document's size under the configured\n" +
		"  depth, scalar/key and output limits: nothing is retained beyond the\n" +
		"  renderer's nesting stack.\n" +
		"\n" +
		"Qualification:\n" +
		"  A later error can occur after earlier output has been written.\n"
	if got := echo.Explain(); got != want {
		t.Errorf("got\n%s\nwant\n%s", got, want)
	}
	identity := mustCompile(t, "def export [input] input", "id.alc").Explain()
	if !strings.HasPrefix(identity, "export: input\n") ||
		!strings.Contains(identity, "JSON profile:          compact, one document, trailing newline (the host's renderer)\n") {
		t.Errorf("%s", identity)
	}
	table := mustCompile(t, strings.Replace(workedExample, "    csv csv-options\n", "", 1), "table.alc")
	text := table.Explain()
	for _, part := range []string{
		"export: api-table\n",
		"Protocol:              JsonEvents/1 → TableRows/1 → Text\n",
		"CSV quoting:           always (the host's renderer)\n",
	} {
		if !strings.Contains(text, part) {
			t.Errorf("%s\ndoes not hold %q", text, part)
		}
	}
	if got := jsonMember(t, table.ExplainJSON(), "renderer", "host"); got != "true" {
		t.Errorf("%s", got)
	}
}

func TestATextOfItsOwnReadsTheInputOnlyToValidateIt(t *testing.T) {
	done := mustCompile(t, `def export [input] "done"`, "done.alc")
	want := "export: a text of its own\n" +
		"\n" +
		"Source reads:          1\n" +
		"Protocol:              Text\n" +
		"Selection:             none; the input is read and validated, and nothing of it is used\n" +
		"Duplicate members:     not examined; the input is only validated\n" +
		"Output order:          the program's own text, written once the input has validated\n" +
		"Ordering contract:     none\n" +
		"Contract verification: static\n" +
		"External storage:      disabled\n" +
		"\n" +
		"Guarantee:\n" +
		"  Memory is independent of the document's size under the configured\n" +
		"  depth and scalar/key limits: nothing of the input is kept. The text\n" +
		"  is the program's own, written under the output limit.\n" +
		"\n" +
		"Qualification:\n" +
		"  The text is written only after the whole input has validated; an\n" +
		"  invalid input writes nothing.\n"
	if got := done.Explain(); got != want {
		t.Errorf("got\n%s\nwant\n%s", got, want)
	}
	j := done.ExplainJSON()
	for _, c := range []struct {
		path []string
		want string
	}{
		{[]string{"finite"}, "true"},
		{[]string{"protocol"}, `["Text"]`},
		{[]string{"readiness"}, `"scope-end"`},
		{[]string{"qualification"}, `["The text is written only after the whole input has validated; an invalid input writes nothing."]`},
	} {
		if got := jsonMember(t, j, c.path...); got != c.want {
			t.Errorf("%v: %s, want %s", c.path, got, c.want)
		}
	}
	// A finite text built of calls names them.
	built := mustCompile(t, `def export [input] (replace-text "a" "b" (concat "x" "a"))`, "built.alc").Explain()
	if !strings.HasPrefix(built, "export: concat → replace-text\n") || !strings.Contains(built, "Protocol:              Text\n") {
		t.Errorf("%s", built)
	}
}

// The CSV renderer is reported with the dialect it is built with: the
// program's own options when its csv runs natively, the defaults when the
// host renders a table; and what it reports is what it writes.
func TestACustomCsvDialectIsReportedAsItRuns(t *testing.T) {
	lf := strings.Replace(workedExample, "    csv csv-options\n", "    csv lf\n", 1) +
		"\ndef lf (record (entry :delimiter \";\") (entry :newline \"\\n\") (entry :header false) (entry :null-text \"NULL\") (entry :missing \"-\"))\n"
	program := mustCompile(t, lf, "lf.alc")
	if !program.Native() {
		t.Fatal("not native")
	}
	if got := jsonMember(t, program.ExplainJSON(), "renderer"); got != `{"name":"csv","quoting":"always","delimiter":";","newline":"\n","header":false,"null_text":"NULL","missing":"text","missing_text":"-","host":false}` {
		t.Errorf("%s", got)
	}
	if out, f := replayRun(t, program, records, RenderDefault, tt.DefaultLimits(), nil); f != nil || out != "\"123\";\"Alice\";\"50.25\"\n\"456\";\"Bob\";\"72\"\n" {
		t.Errorf("%q %v", out, f)
	}
	// The null and missing texts reach the output as reported: a record
	// with a null name and no balance.
	sparse := strings.Replace(records, `{"account":{"balance":72},"person":{"name":"Bob"},"id":456}`, `{"person":{"name":null},"id":456}`, 1)
	if sparse == records {
		t.Fatal("the document did not change")
	}
	if out, f := replayRun(t, program, sparse, RenderDefault, tt.DefaultLimits(), nil); f != nil || out != "\"123\";\"Alice\";\"50.25\"\n\"456\";\"NULL\";\"-\"\n" {
		t.Errorf("%q %v", out, f)
	}
	// The default dialect, and the host's renderer, report the defaults.
	for _, c := range []struct {
		name string
		src  string
		host string
	}{
		{"default", workedExample, "false"},
		{"host", strings.Replace(workedExample, "    csv csv-options\n", "", 1), "true"},
	} {
		got := jsonMember(t, mustCompile(t, c.src, c.name+".alc").ExplainJSON(), "renderer")
		want := `{"name":"csv","quoting":"always","delimiter":",","newline":"\r\n","header":true,"null_text":"","missing":"error","missing_text":null,"host":` + c.host + `}`
		if got != want {
			t.Errorf("%s: %s", c.name, got)
		}
	}
}

func TestDuplicateMembersFollowThePolicyWhereScopesAreCaptured(t *testing.T) {
	program := mustCompile(t, workedExample, "export.alc")
	if got := program.Summary().Duplicates; got != "rejected in captured scopes (DUPLICATE_MEMBER)" {
		t.Errorf("%s", got)
	}
	for _, c := range []struct {
		policy Duplicates
		want   string
	}{
		{DuplicatesLastWins, "the last value wins in captured scopes"},
		{DuplicatesFirstWins, "the first value wins in captured scopes"},
	} {
		p, f := program.WithDuplicates(c.policy)
		if f != nil {
			t.Fatal(f)
		}
		if got := jsonMember(t, p.ExplainJSON(), "duplicates"); got != `"`+c.want+`"` {
			t.Errorf("%s", got)
		}
	}
	identity := mustCompile(t, "def export [input] input", "id.alc")
	if got := identity.Summary().Duplicates; got != "preserved; the events are copied as they arrive, not mapped by key" {
		t.Errorf("%s", got)
	}
}

// The checker's output and the plan's agree, for every report row of
// check.tsv's programs and the shapes the library makes.
func TestTheCheckersOutputIsThePlans(t *testing.T) {
	for _, src := range []string{
		workedExample,
		strings.Replace(workedExample, "    csv csv-options\n", "", 1),
		strings.Replace(workedExample, "    csv csv-options\n", "    records\n", 1),
		"def export [input] input",
		`def export [input] "done"`,
		"def export [input] (json input)",
	} {
		p := mustCompile(t, src, "t.alc")
		for _, q := range []*Program{p, interpreted(t, p)} {
			if q.Output() != q.PlanOutput() {
				t.Errorf("%q (native %v): the checker says %s, the plan %s", src, q.Native(), q.Output(), q.PlanOutput())
			}
		}
	}
}
