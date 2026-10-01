// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// differential_test.go: the differential test (rs/tests/stdlib_test.rs):
// the standard library's own text, interpreted, against the native
// compositions the runtime substitutes for it. Over every transduce
// fixture a grammar these tests can read, and the generated documents
// transduce's tests and benches share, the spec's worked-example program
// produces the same bytes both ways, or fails with the same code. The
// library text is the reference; the native path is the optimization, and
// this is what makes it one.

import (
	"bytes"
	"fmt"
	"os"
	"strings"
	"testing"

	csv "github.com/tabnas/csv/go"
	tabnasjson "github.com/tabnas/json/go"
	tabnasjsonl "github.com/tabnas/jsonl/go"
	tabnas "github.com/tabnas/parser/go"
	tt "github.com/tabnas/transduce/go"
	tabnasyaml "github.com/tabnas/yaml/go"
)

// replay replays events through program under the default limits: the
// output, or the failure.
func replay(program *Program, events []tt.Event, render Renderer) (string, *Fail) {
	return replayEvents(program, events, render, tt.DefaultLimits(), tt.NewMetrics())
}

// differential runs one document both ways: the same bytes, or the same
// code.
func differential(t testing.TB, name string, program *Program, events []tt.Event) string {
	t.Helper()
	slow := interpreted(t, program)
	if !program.Native() || slow.Native() {
		t.Fatalf("%s: native %v, interpreted %v", name, program.Native(), slow.Native())
	}
	a, fa := replay(program, events, RenderDefault)
	b, fb := replay(slow, events, RenderDefault)
	switch {
	case fa == nil && fb == nil && a == b:
		return ""
	case fa == nil && fb == nil:
		return fmt.Sprintf("%s: the bytes differ\n  native:      %q\n  interpreted: %q", name, a, b)
	case fa != nil && fb != nil && fa.Code == fb.Code:
		return ""
	case fa != nil && fb != nil:
		return fmt.Sprintf("%s: the codes differ\n  native:      %v\n  interpreted: %v", name, fa, fb)
	case fa == nil:
		return fmt.Sprintf("%s: native produced %q, interpreted failed: %v", name, a, fb)
	}
	return fmt.Sprintf("%s: interpreted produced %q, native failed: %v", name, b, fa)
}

func TestTheLibraryLoads(t *testing.T) {
	lib := StdlibLoaded()
	if lib.Get("table-from-json") == nil || lib.Get("csv") == nil {
		t.Error("the library lacks table-from-json or csv")
	}
}

// testDocument is a document to compare: its name, the grammar that reads it,
// its text.
type testDocument struct {
	name string
	make func() *tabnas.Tabnas
	text string
}

func jsonGrammar() *tabnas.Tabnas  { return tabnasjson.Make() }
func jsonlGrammar() *tabnas.Tabnas { return tabnasjsonl.Make() }
func yamlGrammar() *tabnas.Tabnas  { return tabnasyaml.MakeJsonic() }
func csvGrammar() *tabnas.Tabnas {
	j, err := csv.Make()
	if err != nil {
		panic(err)
	}
	return j
}

// Every transduce fixture a grammar here reads, plus the generated
// documents in every shape: identical bytes or identical codes.
func TestInterpretedAndNativeAgreeOnEveryFixtureAndGeneratedDocument(t *testing.T) {
	program := mustCompile(t, workedExample, "export.alc")
	var documents []testDocument
	for _, path := range fixtures(t) {
		make := fixtureGrammar(path)
		if make == nil {
			continue
		}
		text, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		documents = append(documents, testDocument{path, make, string(text)})
	}
	for _, n := range []int{0, 1, 2, 3, 50} {
		documents = append(documents, testDocument{fmt.Sprintf("records_json(%d)", n), jsonGrammar, recordsJSON(n)})
	}
	documents = append(documents,
		testDocument{"records_yaml(3)", yamlGrammar, recordsYAML(3)},
		testDocument{"records_jsonl(3)", jsonlGrammar, recordsJSONL(3)},
		testDocument{"records_csv(3)", csvGrammar, recordsCSV(3)},
	)
	// Cells of every kind, in the worked example's shape: null, missing
	// members (a failure under the standard options), nested containers,
	// booleans, quotes, delimiters, line breaks and non-ASCII text.
	for _, c := range []struct{ name, records string }{
		{"nulls", `{"id":null,"person":{"name":null},"account":{"balance":null}}`},
		{"missing", `{"id":1,"person":{},"account":{"balance":2}}`},
		{"containers", `{"id":[1,2.50],"person":{"name":{"first":"A","last":"B"}},"account":{"balance":{}}}`},
		{"booleans", `{"id":true,"person":{"name":false},"account":{"balance":0}}`},
		{"quotes", `{"id":"say \"hi\"","person":{"name":"a,b"},"account":{"balance":"line\r\nbreak"}}`},
		{"unicode", `{"id":"caf\u00e9","person":{"name":"日本"},"account":{"balance":"\ud83d\ude00"}}`},
		{"lexemes", `{"id":1e2,"person":{"name":"x"},"account":{"balance":-0.0}}`},
		{"big", `{"id":12345678901234567890123,"person":{"name":"x"},"account":{"balance":1E+2}}`},
		{"two rows out of order", `{"account":{"balance":1},"id":2,"person":{"name":"z"}},{"id":3,"person":{"name":"y"},"account":{"balance":4}}`},
	} {
		documents = append(documents, testDocument{c.name, jsonGrammar, shaped(c.records)})
	}
	// The order contract and the absent shapes.
	documents = append(documents,
		testDocument{"metadata after rows", jsonGrammar, `{"response":{"payload":{"deep":{"records":[` + record(1) + `]}},"metadata":` + metadata + `}}`},
		testDocument{"no metadata", jsonGrammar, `{"response":{"payload":{"deep":{"records":[` + record(1) + `]}}}}`},
		testDocument{"no records", jsonGrammar, `{"response":{"metadata":` + metadata + `}}`},
		testDocument{"metadata not an array", jsonGrammar, `{"response":{"metadata":{"fields":{}}}}`},
		testDocument{"descriptor without title", jsonGrammar, `{"response":{"metadata":{"fields":[{"path":["a"]}]}}}`},
		testDocument{"descriptor with a bad segment", jsonGrammar, `{"response":{"metadata":{"fields":[{"title":"a","path":[true]}]}}}`},
		testDocument{"row that is not an object", jsonGrammar, shaped("1")},
	)
	total := len(documents)
	var failures []string
	skipped := 0
	for i, doc := range documents {
		if i%10 == 0 {
			t.Logf("differential: %d of %d (%d%%)", i, total, i*100/total)
		}
		events, f := recordEvents(doc.make(), doc.text)
		if f != nil {
			// The grammar refused the document: nothing reached a
			// program, so there is nothing to compare.
			skipped++
			continue
		}
		if report := differential(t, doc.name, program, events); report != "" {
			failures = append(failures, report)
		}
	}
	t.Logf("differential: %d of %d (100%%), %d unreadable", total, total, skipped)
	if total-skipped <= 20 {
		t.Errorf("only %d documents were read", total-skipped)
	}
	if len(failures) > 0 {
		t.Errorf("%d document(s) differ:\n%s", len(failures), strings.Join(failures, "\n"))
	}
}

// The same, with the JSON renderer over the table both ways (records then
// json), and with a program whose options record changes the dialect the
// native renderer takes.
func TestInterpretedAndNativeAgreeOnOtherRenderings(t *testing.T) {
	program := mustCompile(t, strings.Replace(workedExample, "    csv csv-options\n", "    records\n    json\n", 1), "records.alc")
	dialect := mustCompile(t, strings.Replace(workedExample, "    csv csv-options\n",
		"    csv options\n\ndef options\n  record\n    entry :delimiter \";\"\n    entry :newline \"\\n\"\n    entry :header false\n    entry :null-text \"NULL\"\n    entry :missing \"-\"\n", 1), "dialect.alc")
	docs := []string{
		recordsJSON(3),
		`{"response":{"metadata":` + metadata + `,"payload":{"deep":{"records":[{"id":null,"person":{},"account":{"balance":"a;b"}}]}}}}`,
	}
	for i, doc := range docs {
		events := mustEvents(t, doc)
		for _, c := range []struct {
			name    string
			program *Program
		}{{"records/json", program}, {"dialect", dialect}} {
			if report := differential(t, fmt.Sprintf("%s %d", c.name, i), c.program, events); report != "" {
				t.Error(report)
			}
		}
	}
	if out, f := replay(dialect, mustEvents(t, docs[1]), RenderDefault); f != nil || out != "\"NULL\";\"-\";\"a;b\"\n" {
		t.Errorf("%q %v", out, f)
	}
}

// outcome is what a run both ways must come to: the bytes, or the code.
type outcome struct {
	ok   bool
	out  string
	code Code
}

func okOut(out string) outcome  { return outcome{ok: true, out: out} }
func errCode(code Code) outcome { return outcome{code: code} }

// agree runs both ways to one outcome: the same bytes, or the same code and
// the same limit named; expected is that outcome, and both ways count the
// same rows.
func agree(t testing.TB, name string, program *Program, events []tt.Event, limits tt.Limits, expected outcome) {
	t.Helper()
	slow := interpreted(t, program)
	nativeRows, interpretedRows := tt.NewMetrics(), tt.NewMetrics()
	a, fa := replayEvents(program, events, RenderDefault, limits, nativeRows)
	b, fb := replayEvents(slow, events, RenderDefault, limits, interpretedRows)
	switch {
	case fa == nil && fb == nil && expected.ok:
		if a != expected.out || b != expected.out {
			t.Errorf("%s:\n  native:      %q\n  interpreted: %q\n  want:        %q", name, a, b, expected.out)
		}
		if nativeRows.Rows.Load() != interpretedRows.Rows.Load() {
			t.Errorf("%s: rows counted %d native, %d interpreted", name, nativeRows.Rows.Load(), interpretedRows.Rows.Load())
		}
	case fa != nil && fb != nil && !expected.ok:
		if fa.Code != expected.code || fb.Code != expected.code {
			t.Errorf("%s: want %s\n  native:      %v\n  interpreted: %v", name, expected.code, fa, fb)
		}
		la, lb := "", ""
		if fa.Limit != nil {
			la = fa.Limit.Name
		}
		if fb.Limit != nil {
			lb = fb.Limit.Name
		}
		if la != lb {
			t.Errorf("%s: the limit named: native %q, interpreted %q", name, la, lb)
		}
	default:
		t.Errorf("%s: expected %+v\n  native:      %q %v\n  interpreted: %q %v", name, expected, a, fa, b, fb)
	}
}

// The library's csv validates the table events it renders as the native
// renderer does (spec 13.2: the protocol validator is not omitted), so a
// program that produces its own table events, or a document or an options
// record the renderer refuses, fails with the same code both ways, and a
// sound one prints the same bytes.
func TestTheLibraryCsvValidatesWhatTheRendererValidates(t *testing.T) {
	items := mustEvents(t, `{"items":[{"n":"a","v":1},{"n":"b\"q","v":-2},{"n":"c","v":null}],"tail":"t"}`)
	limits := tt.DefaultLimits()
	label := func(l string) string { return `(record (entry :label "` + l + `"))` }
	table := func(step, finish string) string {
		return "def export [input] (csv csv-options (scan-emit no-schema " + step + " " + finish + " (select (path \"items\" each-index) input)))"
	}
	// One schema on the first item, a row per item: the sound shape.
	first := func(schema, row string) string {
		return "(fn [s x] (transition (ready []) (if (is-ready s) [(row " + row + ")] [(schema " + schema + ") (row " + row + ")])))"
	}
	n, v := label("N"), label("V")
	for _, c := range []struct {
		name, src string
		want      outcome
	}{
		{"sound", table(first("["+n+" "+v+"]", "[(get :n x) (get :v x)]"), "(fn [s] [table-end])"),
			okOut("\"N\",\"V\"\r\n\"a\",\"1\"\r\n\"b\"\"q\",\"-2\"\r\n\"c\",\"\"\r\n")},
		{"rows without a schema", table("(fn [s x] (transition s [(row [(get :n x)])]))", "(fn [s] [table-end])"), errCode(CodeProtocolOrderError)},
		{"a schema per item", table("(fn [s x] (transition s [(schema ["+n+"])]))", "(fn [s] [table-end])"), errCode(CodeProtocolOrderError)},
		{"a row wider than the schema", table(first("["+n+"]", "[(get :n x) (get :v x)]"), "(fn [s] [table-end])"), errCode(CodeProtocolOrderError)},
		{"no table-end", table(first("["+n+"]", "[(get :n x)]"), "(fn [s] [])"), errCode(CodeProtocolOrderError)},
		{"two table-ends", table(first("["+n+"]", "[(get :n x)]"), "(fn [s] [table-end table-end])"), errCode(CodeProtocolOrderError)},
		{"an item that is not a table event", "def export [input] (csv csv-options (map (fn [x] (get :n x)) (select (path \"items\" each-index) input)))", errCode(CodeProtocolOrderError)},
		{"no items at all", "def export [input] (csv csv-options (scan-emit no-schema (fn [s x] (transition s [x])) (fn [s] []) (select (path \"none\" each-index) input)))", errCode(CodeProtocolOrderError)},
		{"a schema of no columns", table(first("[]", "[]"), "(fn [s] [table-end])"), errCode(CodeTargetValueUnrepresentable)},
		{"a column without a label", table(first(`[(record (entry :x "N"))]`, "[(get :n x)]"), "(fn [s] [table-end])"), errCode(CodeMissingValue)},
		{"a label that is a record", table(first("[(record (entry :label (record)))]", "[(get :n x)]"), "(fn [s] [table-end])"), errCode(CodeInputInvalid)},
	} {
		program, f := Compile(c.src, "table.alc")
		if f != nil {
			t.Fatalf("%s: %v", c.name, f)
		}
		agree(t, c.name, program, items, limits, c.want)
	}

	// The worked example over a document whose metadata is empty: a table
	// of no columns has no CSV form, whichever path renders it.
	program := mustCompile(t, workedExample, "export.alc")
	empty := mustEvents(t, `{"response":{"metadata":{"fields":[]},"payload":{"deep":{"records":[{"id":1}]}}}}`)
	agree(t, "no columns", program, empty, limits, errCode(CodeTargetValueUnrepresentable))

	// A delimiter no CSV reader could take is refused before anything runs,
	// natively (the renderer) and interpreted (csv-table).
	recordEventsList := mustEvents(t, records)
	for _, delimiter := range []string{`\"`, `\n`, `\r`} {
		src := strings.Replace(workedExample, "    csv csv-options\n",
			"    csv (record (entry :delimiter \""+delimiter+"\") (entry :newline \"\\r\\n\") (entry :header true) (entry :null-text \"\") (entry :missing :error))\n", 1)
		agree(t, "delimiter "+delimiter, mustCompile(t, src, "delimiter.alc"), recordEventsList, limits, errCode(CodeTargetValueUnrepresentable))
	}

	// A column function that answers something other than a record fails
	// as get does, both ways.
	keyword := mustCompile(t, "def b (record (entry :columns (path \"m\")) (entry :rows (path \"r\" each-index)) (entry :column (fn [d] :oops)))\ndef export [input] (csv csv-options (table-from-json b input))", "keyword.alc")
	doc := mustEvents(t, `{"m":[{"title":"t","path":["a"]}],"r":[{"a":"x"}]}`)
	agree(t, "a column that is a keyword", keyword, doc, limits, errCode(CodeDSLTypeError))
}

// The library's table holds the scopes it captures to the limits the
// native table does (the metadata under max_metadata_bytes, each row under
// max_record_bytes, at most max_columns columns), so under the host's own
// limits both ways fail alike or print alike, and count the same rows.
func TestTheLimitsHoldAlikeBothWays(t *testing.T) {
	program := mustCompile(t, workedExample, "export.alc")
	events := mustEvents(t, records)
	with := func(change func(*tt.Limits)) tt.Limits {
		l := tt.DefaultLimits()
		change(&l)
		return l
	}
	for _, c := range []struct {
		name   string
		limits tt.Limits
		want   outcome
	}{
		{"defaults", tt.DefaultLimits(), okOut(expectedCSV)},
		{"max_record_bytes", with(func(l *tt.Limits) { l.MaxRecordBytes = 100 }), errCode(CodeResourceLimitExceeded)},
		{"max_metadata_bytes", with(func(l *tt.Limits) { l.MaxMetadataBytes = 50 }), errCode(CodeResourceLimitExceeded)},
		{"max_columns", with(func(l *tt.Limits) { l.MaxColumns = 2 }), errCode(CodeResourceLimitExceeded)},
		{"max_capture_bytes decides neither table", with(func(l *tt.Limits) { l.MaxCaptureBytes = 100 }), okOut(expectedCSV)},
	} {
		agree(t, c.name, program, events, c.limits, c.want)
	}
	// A cell that is a vector is its compact JSON text, one scalar of the
	// output, held to max_scalar_bytes both ways.
	containers := mustEvents(t, strings.Replace(records, `"id":123`, `"id":[1,2,3,4,5,6]`, 1))
	agree(t, "a container cell under max_scalar_bytes", program, containers,
		with(func(l *tt.Limits) { l.MaxScalarBytes = 12 }), errCode(CodeResourceLimitExceeded))
	agree(t, "a container cell within it", program, containers,
		with(func(l *tt.Limits) { l.MaxScalarBytes = 13 }),
		okOut("\"Identifier\",\"Full name\",\"Balance\"\r\n\"[1,2,3,4,5,6]\",\"Alice\",\"50.25\"\r\n\"456\",\"Bob\",\"72\"\r\n"))
	metrics := tt.NewMetrics()
	if _, f := replayEvents(interpreted(t, program), events, RenderDefault, tt.DefaultLimits(), metrics); f != nil {
		t.Fatal(f)
	}
	if metrics.Rows.Load() != 2 {
		t.Errorf("%d rows", metrics.Rows.Load())
	}
}

// inferred is a program over an inferred binding (:columns :infer): a root
// array of rows, written as CSV with a missing cell as an empty field.
const inferred = "def options\n  record\n    entry :delimiter \",\"\n    entry :newline \"\\r\\n\"\n    entry :header true\n    entry :null-text \"\"\n    entry :missing \"\"\n\ndef rows-binding\n  record\n    entry :columns :infer\n    entry :rows (path each-index)\n\ndef export [input]\n  pipe input\n    table-from-json rows-binding\n    csv options\n"

// The inferred binding: the columns are the first row's keys, in its
// order, each reading its own key. Natively it is transduce's InferSchema;
// the library's text infers them with keys. Both ways write the same
// bytes, fail with the same code, and hold the same limits, over every
// shape a first row and a later row can take.
func TestAnInferredBindingAgreesBothWays(t *testing.T) {
	program := mustCompile(t, inferred, "inferred.alc")
	ok := func(name, text, want string) {
		agree(t, name, program, mustEvents(t, text), tt.DefaultLimits(), okOut(want))
	}
	fails := func(name, text string, code Code) {
		agree(t, name, program, mustEvents(t, text), tt.DefaultLimits(), errCode(code))
	}
	ok("the first row's keys, in its order", `[{"b":1,"a":"x"},{"a":"y","b":2}]`, "\"b\",\"a\"\r\n\"1\",\"x\"\r\n\"2\",\"y\"\r\n")
	ok("a key a later row lacks is a missing cell", `[{"a":1,"b":2},{"a":3}]`, "\"a\",\"b\"\r\n\"1\",\"2\"\r\n\"3\",\"\"\r\n")
	ok("a key only a later row has is no column", `[{"a":1},{"c":3,"a":2}]`, "\"a\"\r\n\"1\"\r\n\"2\"\r\n")
	ok("cells of every kind", `[{"s":"q\"x","n":1.5,"t":true,"z":null,"o":{"k":[1,2]}}]`,
		"\"s\",\"n\",\"t\",\"z\",\"o\"\r\n\"q\"\"x\",\"1.5\",\"true\",\"\",\"{\"\"k\"\":[1,2]}\"\r\n")
	ok("a key that spells an index names a member", `[{"0":"zero","1":"one"}]`, "\"0\",\"1\"\r\n\"zero\",\"one\"\r\n")
	ok("a later row that is not an object has missing cells", `[{"a":1},2]`, "\"a\"\r\n\"1\"\r\n\"\"\r\n")
	fails("a first row that is a scalar", `[1,{"a":1}]`, CodeInputInvalid)
	fails("a first row that is an array", `[[1],{"a":1}]`, CodeInputInvalid)
	// A table of no columns, from no rows or from an empty first row, is
	// one the CSV renderer refuses, as it refuses any.
	fails("no rows", "[]", CodeTargetValueUnrepresentable)
	fails("an empty first row", `[{},{"a":1}]`, CodeTargetValueUnrepresentable)
	three := mustEvents(t, `[{"a":1,"b":2,"c":3}]`)
	with := func(change func(*tt.Limits)) tt.Limits {
		l := tt.DefaultLimits()
		change(&l)
		return l
	}
	agree(t, "max_columns holds the inferred columns", program, three, with(func(l *tt.Limits) { l.MaxColumns = 2 }), errCode(CodeResourceLimitExceeded))
	// Three one-byte names take 16 + 3 * (16 + 1) = 67 bytes natively; the
	// library's state holding them is larger still.
	agree(t, "max_metadata_bytes holds the inferred columns", program, three, with(func(l *tt.Limits) { l.MaxMetadataBytes = 40 }), errCode(CodeResourceLimitExceeded))
	agree(t, "max_record_bytes holds each row", program, three, with(func(l *tt.Limits) { l.MaxRecordBytes = 8 }), errCode(CodeResourceLimitExceeded))
	// The column count holds where the schema is built, so it holds when
	// the table events reach no renderer that would check them.
	for _, c := range []struct{ name, binding string }{
		{"an inferred table's events as items", "(record (entry :columns :infer) (entry :rows (path each-index)))"},
		{"a described table's events as items", "(record (entry :columns (path \"m\")) (entry :rows (path \"r\" each-index)) (entry :column (fn [d] (record (entry :label d) (entry :source (path d))))))"},
	} {
		items := mustCompile(t, "def b "+c.binding+"\ndef export [input] (join \"\" (map (fn [e] \"x\") (table-from-json b input)))", "items.alc")
		events := three
		if !strings.HasPrefix(c.name, "an inferred") {
			events = mustEvents(t, `{"m":["a","b","c"],"r":[{"a":1,"b":2,"c":3}]}`)
		}
		agree(t, c.name, items, events, with(func(l *tt.Limits) { l.MaxColumns = 2 }), errCode(CodeResourceLimitExceeded))
		agree(t, c.name, items, events, tt.DefaultLimits(), okOut("xxx"))
	}
	// The worked example's documents, bound by inference from their rows.
	api := mustCompile(t, strings.Replace(inferred, "(path each-index)", "(path \"response\" \"payload\" \"deep\" \"records\" each-index)", 1), "inferred-api.alc")
	for _, n := range []int{0, 1, 2, 3, 50} {
		if report := differential(t, fmt.Sprintf("records_json(%d)", n), api, mustEvents(t, recordsJSON(n))); report != "" {
			t.Error(report)
		}
	}
}

// Each row counts once in metrics.Rows, by the last table stage it passes:
// the adapter to a renderer, a csv-table whose rows reach no later table
// stage, or the native table when it hands its rows straight to a
// renderer. A map, a filter or a scan-emit between two table stages hands
// the count on, so a row a filter drops is never counted, and rows that
// only ever become a text of items are not rows of any table; natively and
// interpreted alike, with the same bytes.
func TestEachRowCountsOnceAcrossComposedTableStages(t *testing.T) {
	tail := func(s string) string { return strings.Replace(workedExample, "    csv csv-options\n", s, 1) }
	identity := "    map (fn [e] e)\n"
	dropRows := "    filter (fn [e] (match e (case (row cells) false) (case _ true)))\n"
	passScan := "    scan-emit null (fn [s e] (transition s [e])) (fn [s] []) \n"
	asText := "    map (fn [e] \"x\")\n    join \",\"\n"
	events := mustEvents(t, records)
	for _, c := range []struct {
		name   string
		src    string
		render Renderer
		rows   uint64
	}{
		{"csv", tail("    csv csv-options\n"), RenderDefault, 2},
		{"csv-table as the result", tail("    csv-table csv-options\n"), RenderDefault, 2},
		{"csv-table as the result, as json", tail("    csv-table csv-options\n"), RenderJSON, 2},
		{"csv over csv-table", tail("    csv-table csv-options\n    csv csv-options\n"), RenderDefault, 2},
		{"records over csv-table", tail("    csv-table csv-options\n    records\n    json\n"), RenderDefault, 2},
		{"csv-table twice", tail("    csv-table csv-options\n    csv-table csv-options\n    csv csv-options\n"), RenderDefault, 2},
		{"the library csv over the native table", tail("    csv opts\n\ndef opts\n  record\n    entry :delimiter \"||\"\n    entry :newline \"\\r\\n\"\n    entry :header true\n    entry :null-text \"\"\n    entry :missing :error\n"), RenderDefault, 2},
		{"a map, then the program's csv", tail(identity + "    csv csv-options\n"), RenderDefault, 2},
		{"a map, csv-table, csv", tail(identity + "    csv-table csv-options\n    csv csv-options\n"), RenderDefault, 2},
		{"csv-table, a map, csv", tail("    csv-table csv-options\n" + identity + "    csv csv-options\n"), RenderDefault, 2},
		{"a map, the host's renderer", tail(identity), RenderCSV, 2},
		{"a map, the host's json", tail(identity), RenderJSON, 2},
		{"a filter that keeps every row, the host's renderer", tail("    filter (fn [e] true)\n"), RenderCSV, 2},
		{"a scan that passes each event on, then csv", tail(passScan + "    csv csv-options\n"), RenderDefault, 2},
		{"a filter that drops every row, then csv", tail(dropRows + "    csv csv-options\n"), RenderDefault, 0},
		{"a filter that drops every row, the host's renderer", tail(dropRows), RenderCSV, 0},
		{"the table's events as a text: no table stage", tail(asText), RenderDefault, 0},
		{"csv-table's events as a text", tail("    csv-table csv-options\n" + asText), RenderDefault, 2},
	} {
		program := mustCompile(t, c.src, "rows.alc")
		var outputs []string
		for _, native := range []bool{true, false} {
			p, f := program.WithNative(native)
			if f != nil {
				t.Fatal(f)
			}
			metrics := tt.NewMetrics()
			out, f := replayEvents(p, events, c.render, tt.DefaultLimits(), metrics)
			if f != nil {
				t.Fatalf("%s (native %v): %v", c.name, native, f)
			}
			if metrics.Rows.Load() != c.rows {
				t.Errorf("%s (native %v): %d rows counted, want %d", c.name, native, metrics.Rows.Load(), c.rows)
			}
			outputs = append(outputs, out)
		}
		if outputs[0] != outputs[1] {
			t.Errorf("%s: the bytes differ\n  %q\n  %q", c.name, outputs[0], outputs[1])
		}
	}
}

// The two paths differ, knowingly, in one place the standard shapes never
// reach; pinned so a change to either is seen.
func TestTheKnownDifferencesArePinned(t *testing.T) {
	// Metadata selected twice: the native transducer calls it an order
	// violation (a table has one schema); the library text says fail,
	// which is INPUT_INVALID.
	twice := mustCompile(t, strings.Replace(workedExample, "      path \"response\" \"metadata\" \"fields\"\n", "      path \"response\" \"metadata\" each-index\n", 1), "twice.alc")
	doc := `{"response":{"metadata":[[{"title":"a","path":["id"]}],[{"title":"b","path":["id"]}]],"payload":{"deep":{"records":[` + record(1) + `]}}}}`
	events := mustEvents(t, doc)
	if _, f := replay(twice, events, RenderDefault); f == nil || f.Code != CodeInputOrderViolation {
		t.Errorf("native: %v", f)
	}
	if _, f := replay(interpreted(t, twice), events, RenderDefault); f == nil || f.Code != CodeInputInvalid {
		t.Errorf("interpreted: %v", f)
	}
	// A numeric title was a second difference; one label policy now serves
	// every table (a string as it is, a number by its lexeme, a boolean by
	// its name), so both ways render the lexeme.
	program := mustCompile(t, workedExample, "export.alc")
	numeric := mustEvents(t, `{"response":{"metadata":{"fields":[{"title":42,"path":["id"]}]},"payload":{"deep":{"records":[`+record(1)+`]}}}}`)
	agree(t, "a numeric title", program, numeric, tt.DefaultLimits(), okOut("\"42\"\r\n\"1\"\r\n"))
}

// The events end with End, as every source promises; a recording that
// does not is a source defect, and the sink says so rather than dropping
// the output silently: nothing is flushed.
func TestAStreamWithoutEndWritesNothing(t *testing.T) {
	program := mustCompile(t, workedExample, "export.alc")
	events := mustEvents(t, recordsJSON(2))
	if last := events[len(events)-1]; last.Kind != tt.End {
		t.Fatalf("the recording ends with %v", last)
	}
	events = events[:len(events)-1]
	for _, native := range []bool{true, false} {
		p, f := program.WithNative(native)
		if f != nil {
			t.Fatal(f)
		}
		var buffer bytes.Buffer
		sink, f := p.Sink(&buffer, RenderDefault, tt.DefaultLimits(), tt.NewMetrics())
		if f != nil {
			t.Fatal(f)
		}
		if _, f := tt.Replay(events, sink); f != nil {
			t.Fatal(f)
		}
		if buffer.Len() != 0 {
			t.Errorf("native %v: %q before the end", native, buffer.String())
		}
		// And the End then completes it.
		if _, f := sink.Event(tt.EvEnd()); f != nil {
			t.Fatal(f)
		}
		if buffer.Len() == 0 {
			t.Errorf("native %v: nothing after the end", native)
		}
	}
}
