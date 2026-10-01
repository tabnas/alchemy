// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// run_test.go: the public API end to end (rs/tests/run_test.rs): the
// spec's worked example (section 5) run through the spec's program
// (sections 12.1 and 13.4), natively and interpreted, byte for byte; the
// streaming behaviours of the spec's section 19.5 that apply to a run over
// one document; the json echo of every fixture against the walk's own
// rendering; and records of a table round-tripping through JSON.

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"
	"time"

	csv "github.com/tabnas/csv/go"
	tabnasjson "github.com/tabnas/json/go"
	tabnasjsonl "github.com/tabnas/jsonl/go"
	tabnas "github.com/tabnas/parser/go"
	tr "github.com/tabnas/render/go"
	tt "github.com/tabnas/transduce/go"
	tabnasyaml "github.com/tabnas/yaml/go"
)

// hostRun is what a host does: read text with the json grammar,
// incrementally (readRun: through the walk in a build without the
// adapter), pruned under the program's row selector when it has one, push
// the events into the program's sink, and mark a failure as leaving
// partial output when OutputBytes says bytes reached the writer.
func hostRun(t testing.TB, program *Program, text string, render Renderer, limits tt.Limits) (string, *Fail) {
	t.Helper()
	metrics := tt.NewMetrics()
	out, f := readRun(t, program, text, render, limits, metrics)
	if f != nil && metrics.OutputBytes.Load() > 0 && !f.CommittedOutput {
		f.Committed()
	}
	return out, f
}

func hostOK(t testing.TB, program *Program, text string, render Renderer) string {
	t.Helper()
	out, f := hostRun(t, program, text, render, tt.DefaultLimits())
	if f != nil {
		t.Fatalf("native %v: %v", program.Native(), f)
	}
	return out
}

func hostErr(t testing.TB, program *Program, text string, limits tt.Limits) (*Fail, string) {
	t.Helper()
	out, f := hostRun(t, program, text, RenderDefault, limits)
	if f == nil {
		t.Fatalf("native %v: the run succeeded: %q", program.Native(), out)
	}
	return f, out
}

// both is the spec's program, native and interpreted.
func both(t testing.TB) []*Program {
	t.Helper()
	native := mustCompile(t, workedExample, "export.alc")
	return []*Program{native, interpreted(t, native)}
}

// Acceptance 3: the worked example prints the spec's bytes, both ways.
func TestTheWorkedExamplePrintsTheSpecCsvBothWays(t *testing.T) {
	for _, program := range both(t) {
		if program.Output() != OutputText || rowSelectorText(program) != ".response.payload.deep.records[*]" {
			t.Errorf("%s %s", program.Output(), rowSelectorText(program))
		}
		if got := hostOK(t, program, records, RenderDefault); got != expectedCSV {
			t.Errorf("native %v: %q", program.Native(), got)
		}
	}
}

// Spec 19.5: metadata after rows is rejected under the metadata-first
// policy, before any row is retained, and no CSV is written.
func TestMetadataAfterRowsIsAnInputOrderViolation(t *testing.T) {
	doc := `{"response":{"payload":{"deep":{"records":[` + record(1) + `]}},"metadata":` + metadata + `}}`
	for _, program := range both(t) {
		f, out := hostErr(t, program, doc, tt.DefaultLimits())
		if f.Code != CodeInputOrderViolation || f.CommittedOutput || out != "" {
			t.Errorf("native %v: %v %q", program.Native(), f, out)
		}
	}
}

// Spec 19.5: cells follow the schema's order whatever the row's.
func TestCellsFollowSchemaOrderNotMemberOrder(t *testing.T) {
	doc := shaped(`{"account":{"balance":1},"person":{"name":"z"},"id":2}`)
	for _, program := range both(t) {
		if got := hostOK(t, program, doc, RenderDefault); got != "\"Identifier\",\"Full name\",\"Balance\"\r\n\"2\",\"z\",\"1\"\r\n" {
			t.Errorf("native %v: %q", program.Native(), got)
		}
	}
}

// Spec 19.5: no matching rows is a valid empty table: the header alone.
func TestNoMatchingRowsPrintsTheHeaderOnly(t *testing.T) {
	for _, doc := range []string{shaped(""), `{"response":{"metadata":` + metadata + `}}`} {
		for _, program := range both(t) {
			if got := hostOK(t, program, doc, RenderDefault); got != "\"Identifier\",\"Full name\",\"Balance\"\r\n" {
				t.Errorf("native %v: %q", program.Native(), got)
			}
		}
	}
}

// Spec 19.5: invalid trailing input fails the run after rows were
// exported; with enough rows to pass the writer's budget, the failure says
// the output is partial, and what was committed is whole records.
func TestInvalidTrailingInputFailsAfterRowsWereExported(t *testing.T) {
	requireIncremental(t)
	whole := recordsJSON(2000)
	doc := whole + " x"
	for _, program := range both(t) {
		full := hostOK(t, program, whole, RenderDefault)
		f, out := hostErr(t, program, doc, tt.DefaultLimits())
		if f.Code != CodeInputInvalid || !f.CommittedOutput {
			t.Errorf("native %v: %v", program.Native(), f)
		}
		if !strings.HasPrefix(out, "\"Identifier\",\"Full name\",\"Balance\"\r\n") || len(out) <= 32*1024 {
			t.Errorf("native %v: %d bytes", program.Native(), len(out))
		}
		// The writer coalesces by fragment, never holding a row back to
		// end on a record boundary (spec 17.4): what was committed is a
		// prefix of the whole output, and may end inside a record.
		if !strings.HasPrefix(full, out) || len(out) >= len(full) {
			t.Errorf("native %v: %d of %d bytes", program.Native(), len(out), len(full))
		}
	}
	// A small document: the rows were buffered, not committed, so nothing
	// reached the writer and the failure says so.
	f, out := hostErr(t, both(t)[0], records+" x", tt.DefaultLimits())
	if f.Code != CodeInputInvalid || f.CommittedOutput || out != "" {
		t.Errorf("%v %q", f, out)
	}
}

// Spec 19.5: a very large selected row fails clearly under the limit that
// bounds it, the same one both ways: the native transducer materializes
// rows under max_record_bytes, and the library's table-from-json captures
// its rows under the same limit, so the generic max_capture_bytes does
// not decide either.
func TestAVeryLargeSelectedRowNamesTheLimit(t *testing.T) {
	big := fmt.Sprintf(`{"id":1,"person":{"name":"%s"},"account":{"balance":2}}`, strings.Repeat("x", 4096))
	doc := shaped(record(0) + "," + big)
	limits := tt.DefaultLimits()
	limits.MaxRecordBytes = 1024
	for _, program := range both(t) {
		f, _ := hostErr(t, program, doc, limits)
		if f.Code != CodeResourceLimitExceeded || f.Limit.Name != "max_record_bytes" || f.Path != ".response.payload.deep.records[1]" {
			t.Errorf("native %v: %v", program.Native(), f)
		}
	}
	// A small generic capture limit decides neither: both print the rows.
	capture := tt.DefaultLimits()
	capture.MaxCaptureBytes = 64
	for _, program := range both(t) {
		if out, f := hostRun(t, program, records, RenderDefault, capture); f != nil || out != expectedCSV {
			t.Errorf("native %v: %q %v", program.Native(), out, f)
		}
	}
}

// string-join builds one string from a vector of strings, the separator
// between them: a cell from the runs of a Markdown cell, a fail message
// that names a key. An item that is not a string is a type error where the
// plan is built, and the joined string is held to max_scalar_bytes before
// it is built.
func TestStringJoinBuildsOneStringFromSeveral(t *testing.T) {
	program := mustCompile(t, "def export [input]\n  concat\n    string-join \", \" [\"a\" \"b\" \"c\"]\n    \"|\"\n    string-join \"-\" []\n    \"|\"\n    string-join \"\" [\"x\"]\n    \"|\"\n    string-join \" \" [(quoted \"k\") \"holds\" (scalar-text csv-options 1.5)]\n    \"\\n\"\n", "join.alc")
	if program.Output() != OutputText {
		t.Errorf("%s", program.Output())
	}
	if out := hostOK(t, program, "null", RenderDefault); out != "a, b, c||x|\"k\" holds 1.5\n" {
		t.Errorf("%q", out)
	}
	// A failure that names a key, built from the parts.
	_, f := Compile("def export [input] (let [m (fail (string-join \" \" [\"no value under\" (quoted \"k\")]))] (json input))", "named.alc")
	if f == nil || f.Code != CodeInputInvalid || f.Message != "no value under \"k\"" {
		t.Errorf("%v", f)
	}
	// The joined string is one scalar, held to max_scalar_bytes: here one
	// built at the end of a scan over the events, under the run's limits.
	program = mustCompile(t, "def step [s e] (transition (push \"abcd\" s) [])\ndef fin [s] [(string-join \"\" s)]\ndef export [input]\n  join \"\" (scan-emit [] step fin (events input))\n", "big.alc")
	limits := tt.DefaultLimits()
	limits.MaxScalarBytes = 16
	f, out := hostErr(t, program, "[1,2,3,4,5,6,7,8]", limits)
	if f.Code != CodeResourceLimitExceeded || f.Limit.Name != "max_scalar_bytes" || !strings.Contains(f.Message, "string-join") || out != "" {
		t.Errorf("%v %q", f, out)
	}
}

// A missing cell under the standard options is MISSING_VALUE, and a null
// is the empty string, both ways.
func TestMissingAndNullCellsFollowTheOptions(t *testing.T) {
	for _, program := range both(t) {
		f, _ := hostErr(t, program, shaped(`{"id":1,"person":{},"account":{"balance":2}}`), tt.DefaultLimits())
		if f.Code != CodeMissingValue {
			t.Errorf("native %v: %v", program.Native(), f)
		}
		if out := hostOK(t, program, shaped(`{"id":null,"person":{"name":"n"},"account":{"balance":null}}`), RenderDefault); out != "\"Identifier\",\"Full name\",\"Balance\"\r\n\"\",\"n\",\"\"\r\n" {
			t.Errorf("native %v: %q", program.Native(), out)
		}
	}
}

// fixtureGrammar is the grammar for a transduce fixture, by extension,
// among those these tests take; nil skips the fixture.
func fixtureGrammar(path string) func() *tabnas.Tabnas {
	switch filepath.Ext(path) {
	case ".json":
		return func() *tabnas.Tabnas { return tabnasjson.Make() }
	case ".jsonl":
		return func() *tabnas.Tabnas { return tabnasjsonl.Make() }
	case ".yaml":
		return func() *tabnas.Tabnas { return tabnasyaml.MakeJsonic() }
	case ".csv", ".tsv":
		return func() *tabnas.Tabnas {
			j, err := csv.Make()
			if err != nil {
				panic(err)
			}
			return j
		}
	}
	return nil
}

// fixtures are transduce's fixtures, sorted.
func fixtures(t testing.TB) []string {
	t.Helper()
	dir := transduceFixtures(t)
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	var paths []string
	for _, e := range entries {
		paths = append(paths, filepath.Join(dir, e.Name()))
	}
	sort.Strings(paths)
	return paths
}

// json input echoes every fixture as the JSON renderer renders the walk's
// events: the plan passes the events through untouched.
func TestJSONEchoOfEveryFixtureEqualsTheWalksRendering(t *testing.T) {
	echo := mustCompile(t, "def export [input] (json input)", "echo.alc")
	identity := mustCompile(t, "def export [input] input", "id.alc")
	if echo.Output() != OutputText || identity.Output() != OutputJsonEvents {
		t.Fatalf("%s %s", echo.Output(), identity.Output())
	}
	compared := 0
	for _, path := range fixtures(t) {
		make := fixtureGrammar(path)
		if make == nil {
			continue
		}
		text, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		events, f := recordEvents(make(), string(text))
		if f != nil {
			continue
		}
		reference := tr.NewJSONRenderer[*tr.StringOut](tr.NewStringOut(), tr.JSONOptions{TrailingNewline: true})
		if _, f := tt.Replay(events, reference); f != nil {
			// A document the renderer refuses (a non-finite number, say)
			// is refused the same way through the program; not compared.
			continue
		}
		expected := reference.Inner().String()
		for _, program := range []*Program{echo, identity} {
			out, f := replayEvents(program, events, RenderDefault, tt.DefaultLimits(), nil)
			if f != nil || out != expected {
				t.Errorf("%s: %q %v, want %q", path, out, f, expected)
			}
		}
		compared++
	}
	if compared <= 10 {
		t.Errorf("%d fixtures compared", compared)
	}
}

// records of a table is JSON events: rendered as JSON they read back as
// the rows, keyed by label, with the lexemes kept.
func TestRecordsOfATableRoundTrips(t *testing.T) {
	program := mustCompile(t, strings.Replace(workedExample, "    csv csv-options\n", "    records\n    json\n", 1), "records.alc")
	if program.Output() != OutputText {
		t.Errorf("%s", program.Output())
	}
	for _, p := range []*Program{program, interpreted(t, program)} {
		out := hostOK(t, p, records, RenderDefault)
		if out != recordsJSONOut+"\n" {
			t.Errorf("native %v: %q", p.Native(), out)
		}
		var parsed []map[string]any
		if err := json.Unmarshal([]byte(out), &parsed); err != nil || parsed[1]["Full name"] != "Bob" || parsed[0]["Balance"] != 50.25 {
			t.Errorf("%v %v", parsed, err)
		}
	}
	// The table result rendered by the host as JSON is the same document,
	// and as CSV the spec's bytes.
	table := mustCompile(t, strings.Replace(workedExample, "    csv csv-options\n", "", 1), "table.alc")
	if table.Output() != OutputTableRows {
		t.Errorf("%s", table.Output())
	}
	for _, c := range []struct {
		render Renderer
		want   string
	}{{RenderJSON, recordsJSONOut + "\n"}, {RenderCSV, expectedCSV}, {RenderDefault, expectedCSV}} {
		if out := hostOK(t, table, records, c.render); out != c.want {
			t.Errorf("%s: %q", c.render, out)
		}
	}
	// A renderer for a program that renders its own text is refused.
	_, f := hostRun(t, program, records, RenderJSON, tt.DefaultLimits())
	if f == nil || f.Code != CodeDSLTypeError || !strings.HasPrefix(f.Message, "render_of_text: ") {
		t.Errorf("%v", f)
	}
}

// The output limit is the writer's: a run that would exceed it fails with
// the limit named, and nothing past it is written.
func TestTheOutputLimitIsEnforcedByTheWriter(t *testing.T) {
	limits := tt.DefaultLimits()
	max := uint64(200)
	limits.MaxOutputBytes = &max
	f, _ := hostErr(t, both(t)[0], recordsJSON(50), limits)
	if f.Code != CodeResourceLimitExceeded || f.Limit.Name != "max_output_bytes" {
		t.Errorf("%v", f)
	}
}

// SinkOut takes any text output: a writer with no budget commits every
// fragment as it is written.
func TestSinkOutTakesTheHostsOwnTextOutput(t *testing.T) {
	program := mustCompile(t, workedExample, "export.alc")
	var buffer bytes.Buffer
	sink, f := program.SinkOut(tr.NewWriteOut(&buffer).WithBudget(0), RenderDefault, tt.DefaultLimits(), tt.NewMetrics())
	if f != nil {
		t.Fatal(f)
	}
	if _, f := tt.Replay(mustEvents(t, records), sink); f != nil {
		t.Fatal(f)
	}
	if buffer.String() != expectedCSV {
		t.Errorf("%q", buffer.String())
	}
}

// doubling is definitions a0 to a<levels>, each the concatenation of the
// one before it with itself: a text of 10 * 2^levels bytes, built of
// shared parts in linear time.
func doubling(levels int) string {
	var b strings.Builder
	b.WriteString("def a0 \"0123456789\"\n")
	for i := 1; i <= levels; i++ {
		fmt.Fprintf(&b, "def a%d (concat a%d a%d)\n", i, i-1, i-1)
	}
	return b.String()
}

// A program is code, and code can ask for too much before it reads a byte.
// Building the plan is bounded: its steps by max_plan_steps, its nesting
// by the evaluation depth (a function applied to itself is recursion),
// and a concat over shared parts knows which item is live without walking
// them again.
func TestBuildingThePlanIsBounded(t *testing.T) {
	d := strings.Repeat("(d ", 40) + "id" + strings.Repeat(")", 40)
	expo := "def id [x] x\ndef d [g] (fn [x] (g (g x)))\ndef export [input]\n  let [y (" + d + " 1)]\n    json input\n"
	if _, f := Compile(expo, "expo.alc"); f == nil || f.Code != CodeResourceLimitExceeded || f.Limit.Name != "max_plan_steps" {
		t.Errorf("%v", f)
	}
	var wide strings.Builder
	wide.WriteString("def v0 [1 2 3]\n")
	for i := 1; i <= 40; i++ {
		fmt.Fprintf(&wide, "def v%d (vector v%d v%d)\n", i, i-1, i-1)
	}
	wide.WriteString("def export [input] (concat (scalar-text csv-options v40) (json input))\n")
	if _, f := Compile(wide.String(), "wide.alc"); f == nil || f.Code != CodeResourceLimitExceeded {
		t.Errorf("%v", f)
	}
	omega := "def w [f] (f f)\ndef export [input]\n  let [x (w w)]\n    json input\n"
	if _, f := Compile(omega, "omega.alc"); f == nil || f.Code != CodeStreamabilityUnknown || !strings.HasPrefix(f.Message, "recursion: ") || f.Row != 1 || f.Column != 12 {
		t.Errorf("%v", f)
	}
	start := time.Now()
	program := mustCompile(t, doubling(40)+"def export [input] (concat a40 (json input))\n", "shared.alc")
	if elapsed := time.Since(start); elapsed > 20*time.Second {
		t.Errorf("%v", elapsed)
	}
	// Its prefix is 10 TB; the output limit ends it.
	limits := tt.DefaultLimits()
	max := uint64(1000)
	limits.MaxOutputBytes = &max
	f, out := hostErr(t, program, "1", limits)
	if f.Limit == nil || f.Limit.Name != "max_output_bytes" || len(out) > 1000 {
		t.Errorf("%v %d", f, len(out))
	}
}

// A function applied to itself per item fails with recursion at the item,
// rather than taking the host down.
func TestSelfApplicationPerItemIsRecursionAtRunTime(t *testing.T) {
	program := mustCompile(t, "def w [f] (f f)\ndef export [input]\n  pipe input\n    select (path each-index)\n    map (fn [x] (w w))\n    join \",\"\n", "omega.alc")
	f, out := hostErr(t, program, "[1]", tt.DefaultLimits())
	if f.Code != CodeStreamabilityUnknown || !strings.HasPrefix(f.Message, "recursion: ") || out != "" {
		t.Errorf("%v %q", f, out)
	}
}

// The host's abort flag reaches the program's own functions: a long
// computation on one item stops with ABORTED at the next evaluation step,
// not when the item is done. The source is not given the flag here, so
// only the program can have seen it.
func TestTheAbortFlagStopsALongComputationOnOneItem(t *testing.T) {
	d := strings.Repeat("(d ", 30) + "id" + strings.Repeat(")", 30)
	src := "def id [x] x\ndef d [g] (fn [x] (g (g x)))\ndef export [input]\n  join \",\"\n    map (fn [x] (" + d + " x)) (select (path each-index) input)\n"
	flag := tt.NewAbortFlag()
	program := mustCompile(t, src, "long.alc").WithAbort(flag)
	events := mustEvents(t, "[1,2,3]")
	var buffer bytes.Buffer
	sink, f := program.Sink(&buffer, RenderDefault, tt.DefaultLimits(), tt.NewMetrics())
	if f != nil {
		t.Fatal(f)
	}
	flag.Abort()
	start := time.Now()
	if _, f := tt.Replay(events, sink); f == nil || f.Code != CodeAborted {
		t.Errorf("%v", f)
	}
	if elapsed := time.Since(start); elapsed > 20*time.Second {
		t.Errorf("%v", elapsed)
	}
}

// The output limit bounds what a finite text holds, not only what is
// written: replace-text over a finite text streams through the replacer
// (it holds at most the literal), so the limit stops it at its first byte
// past; a join or concat-map item is assembled whole to write it
// atomically, and the assembly fails as soon as it passes the limit,
// before anything of it is written.
func TestTheOutputLimitBoundsAFiniteText(t *testing.T) {
	limits := tt.DefaultLimits()
	max := uint64(100_000)
	limits.MaxOutputBytes = &max
	program := mustCompile(t, doubling(25)+"def export [input] (replace-text \"0\" \"x\" a25)\n", "replace.alc")
	f, out := hostErr(t, program, "1", limits)
	// The writer refuses the fragment that would cross the limit, so what
	// reached it is the text up to there.
	if f.Limit == nil || f.Limit.Name != "max_output_bytes" || !f.CommittedOutput {
		t.Errorf("%v", f)
	}
	if len(out) <= 50_000 || len(out) > 100_000 || !strings.HasPrefix(out, "x123456789x123456789") {
		t.Errorf("%d bytes", len(out))
	}
	for _, step := range []string{`join ","`, "concat-map (fn [t] t)"} {
		program := mustCompile(t, doubling(25)+"def export [input] ("+step+" (map (fn [x] a25) (select (path each-index) input)))\n", "items.alc")
		f, out := hostErr(t, program, "[1,2]", limits)
		if f.Limit == nil || f.Limit.Name != "max_output_bytes" || out != "" {
			t.Errorf("%s: %v %q", step, f, out)
		}
	}
}

// A scan-emit state is what a stage retains from item to item, so it is
// measured as it changes: no deeper than max_depth (a state that wraps
// itself once per item fails at the item that passes it), no larger than
// max_metadata_bytes, and reported in retained_bytes_high.
func TestAScanEmitStateIsMeasuredAndCapped(t *testing.T) {
	grow := mustCompile(t, "def step [s x] (transition [s x] [])\ndef fin [s] [\"done\"]\ndef export [input]\n  join \",\"\n    scan-emit null step fin (select (path each-index) input)\n", "grow.alc")
	numbers := "[" + strings.TrimSuffix(strings.Repeat("1,", 300), ",") + "]"
	f, _ := hostErr(t, grow, numbers, tt.DefaultLimits())
	if f.Limit == nil || f.Limit.Name != "max_depth" || f.Row != 5 || f.Column != 5 {
		t.Errorf("%v", f)
	}
	// The same state under a small byte limit.
	small := tt.DefaultLimits()
	small.MaxMetadataBytes = 256
	if f, _ := hostErr(t, grow, numbers, small); f.Limit == nil || f.Limit.Name != "max_metadata_bytes" {
		t.Errorf("%v", f)
	}
	// A state that wraps itself in a partial once per item holds the one
	// before it inside the function, not the arguments: measured link by
	// link, it fails as the vector does.
	chain := mustCompile(t, "def step [s x] (transition (partial s x) [])\ndef fin [s] [\"done\"]\ndef export [input]\n  join \",\"\n    scan-emit (fn [a] a) step fin (select (path each-index) input)\n", "chain.alc")
	if f, _ := hostErr(t, chain, numbers, tt.DefaultLimits()); f.Limit == nil || f.Limit.Name != "max_depth" || f.Row != 5 || f.Column != 5 {
		t.Errorf("%v", f)
	}
	strs := "[" + strings.TrimSuffix(strings.Repeat(`"`+strings.Repeat("a", 100)+`",`, 200), ",") + "]"
	if f, _ := hostErr(t, chain, strs, small); f.Limit == nil || f.Limit.Name != "max_metadata_bytes" {
		t.Errorf("%v", f)
	}
	// A state that keeps the one before inside the partial a finite text's
	// concat-map applies is walked through that function: it fails as the
	// vector does, rather than growing a level per item.
	text := mustCompile(t, "def g [prev y] y\ndef step [s x] (transition [(concat-map (partial g s) [\"x\"])] [])\ndef fin [s] [\"done\"]\ndef export [input]\n  join \",\"\n    scan-emit [] step fin (select (path each-index) input)\n", "text.alc")
	if f, _ := hostErr(t, text, numbers, tt.DefaultLimits()); f.Limit == nil || f.Limit.Name != "max_depth" || f.Row != 6 || f.Column != 5 {
		t.Errorf("%v", f)
	}
	if f, _ := hostErr(t, text, numbers, small); f.Limit == nil || f.Limit.Name != "max_metadata_bytes" {
		t.Errorf("%v", f)
	}
	// A state that keeps the last item is measured and reported.
	last := mustCompile(t, "def step [s x] (transition x [])\ndef fin [s] [\"done\"]\ndef export [input]\n  join \",\"\n    scan-emit null step fin (select (path each-index) input)\n", "last.alc")
	metrics := tt.NewMetrics()
	var buffer bytes.Buffer
	sink, f := last.Sink(&buffer, RenderDefault, tt.DefaultLimits(), metrics)
	if f != nil {
		t.Fatal(f)
	}
	doc := `[{"k":"` + strings.Repeat("a", 500) + `"},{"k":"b"}]`
	if _, f := tt.NewParserSource(tabnasjson.Make(), doc).Grammar("json").Metrics(metrics).Run(sink); f != nil {
		t.Fatal(f)
	}
	if buffer.String() != "done" || metrics.RetainedBytesHigh.Load() < 500 {
		t.Errorf("%q %d", buffer.String(), metrics.RetainedBytesHigh.Load())
	}
}

// The initial state is retained like any other: measured when the stage is
// built, so neither a step that hands the same state back nor a source
// with no items carries one past the limits, and the failure comes before
// anything is read or written.
func TestAScanEmitInitialStateIsMeasuredAndCapped(t *testing.T) {
	tail := "def fin [s] [\"done\"]\ndef export [input]\n  join \",\"\n    scan-emit init step fin (select (path each-index) input)\n"
	keep := "def step [s x] (transition s [])\n"
	big := fmt.Sprintf("def init [\"%s\"]\n%s%s", strings.Repeat("a", 1000), keep, tail)
	program := mustCompile(t, big, "big.alc")
	small := tt.DefaultLimits()
	small.MaxMetadataBytes = 256
	for _, input := range []string{"[1,2,3]", "[]"} {
		f, out := hostErr(t, program, input, small)
		if f.Limit == nil || f.Limit.Name != "max_metadata_bytes" || f.Row != 6 || f.Column != 5 || out != "" || f.CommittedOutput {
			t.Errorf("%s: %v %q", input, f, out)
		}
	}
	deep := fmt.Sprintf("def init %s1%s\n%s%s", strings.Repeat("[", 10), strings.Repeat("]", 10), keep, tail)
	program = mustCompile(t, deep, "deep.alc")
	shallow := tt.DefaultLimits()
	shallow.MaxDepth = 8
	for _, input := range []string{"[1,2,3]", "[]"} {
		f, out := hostErr(t, program, input, shallow)
		if f.Limit == nil || f.Limit.Name != "max_depth" || out != "" {
			t.Errorf("%s: %v %q", input, f, out)
		}
	}
	// Under the defaults it runs, and the state it retains is reported.
	program = mustCompile(t, big, "big.alc")
	for _, input := range []string{"[1,2,3]", "[]"} {
		metrics := tt.NewMetrics()
		var buffer bytes.Buffer
		sink, f := program.Sink(&buffer, RenderDefault, tt.DefaultLimits(), metrics)
		if f != nil {
			t.Fatal(f)
		}
		if _, f := tt.NewParserSource(tabnasjson.Make(), input).Grammar("json").Metrics(metrics).Run(sink); f != nil {
			t.Fatal(f)
		}
		if buffer.String() != "done" || metrics.RetainedBytesHigh.Load() < 1000 {
			t.Errorf("%s: %q %d", input, buffer.String(), metrics.RetainedBytesHigh.Load())
		}
	}
}
