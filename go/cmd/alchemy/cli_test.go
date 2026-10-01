// Copyright (c) 2026 tabnas, MIT License

package main

// cli_test.go: the alchemy binary, built and run the way a script runs it
// (rs/tests/cli_test.rs): the five commands, standard input as -, the
// statuses, and that nothing but the answer reaches standard output.
//
// The binary is built once, with the build tags this test binary has: the
// incremental adapter (tabnas_nodecell) or not. Without it, run refuses
// with STREAMABILITY_UNKNOWN before the document is read, so the cases
// that run a document are skipped there, by name, and
// TestRunRefusesWithoutTheIncrementalAdapter holds the refusal.

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	alchemy "github.com/tabnas/alchemy/go"
	tt "github.com/tabnas/transduce/go"
)

var binary string

func TestMain(m *testing.M) {
	dir, err := os.MkdirTemp("", "alchemy-cli-")
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	binary = filepath.Join(dir, "alchemy")
	args := []string{"build", "-o", binary}
	if tt.AdapterBuilt() {
		args = append(args, "-tags", "tabnas_nodecell")
	}
	build := exec.Command("go", append(args, ".")...)
	build.Stdout, build.Stderr = os.Stderr, os.Stderr
	if err := build.Run(); err != nil {
		fmt.Fprintln(os.Stderr, "the alchemy binary does not build:", err)
		os.Exit(1)
	}
	code := m.Run()
	os.RemoveAll(dir)
	os.Exit(code)
}

// output is what one run of the binary left.
type output struct {
	status int
	stdout string
	stderr string
}

func runIn(t *testing.T, dir string, args []string, stdin *string) output {
	t.Helper()
	cmd := exec.Command(binary, args...)
	cmd.Dir = dir
	if stdin != nil {
		// A command refused before standard input is read may have exited
		// already, closing the pipe: that is the behaviour under test, and
		// os/exec does not count the closed pipe as a failure.
		cmd.Stdin = strings.NewReader(*stdin)
	}
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	err := cmd.Run()
	status := 0
	var exit *exec.ExitError
	switch {
	case errors.As(err, &exit):
		status = exit.ExitCode()
	case err != nil:
		t.Fatalf("the binary does not run: %v", err)
	}
	return output{status: status, stdout: stdout.String(), stderr: stderr.String()}
}

func invoke(t *testing.T, args []string, stdin *string) output {
	t.Helper()
	return runIn(t, "", args, stdin)
}

func text(s string) *string { return &s }

// failJSON is the one JSON object on standard error.
func failJSON(t *testing.T, o output) map[string]any {
	t.Helper()
	var f map[string]any
	if err := json.Unmarshal([]byte(strings.TrimSpace(o.stderr)), &f); err != nil {
		t.Fatalf("one JSON object on stderr, not %q", o.stderr)
	}
	return f
}

func tempFile(t *testing.T, name, content string) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), name)
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
	return path
}

// needsRun skips a case that runs a document in a build without the
// incremental adapter.
func needsRun(t *testing.T) {
	t.Helper()
	if !tt.AdapterBuilt() {
		t.Skip("runs a document incrementally, as alchemy run does, and this build has no incremental adapter (build with -tags tabnas_nodecell)")
	}
}

// records is the spec's worked example: aless's
// tests/fixtures/records.json.
const records = `{"response":{"metadata":{"fields":[{"title":"Identifier","path":["id"]},{"title":"Full name","path":["person","name"]},{"title":"Balance","path":["account","balance"]}]},"payload":{"deep":{"records":[{"id":123,"person":{"name":"Alice"},"account":{"balance":50.25}},{"account":{"balance":72},"person":{"name":"Bob"},"id":456}]}}}}`

const expectedCSV = "\"Identifier\",\"Full name\",\"Balance\"\r\n\"123\",\"Alice\",\"50.25\"\r\n\"456\",\"Bob\",\"72\"\r\n"

// program is the spec's program (sections 12.1 and 13.4).
const program = "def column-from-meta [source]\n  record\n    entry :label (get \"title\" source)\n    entry :source\n      as-path\n        get \"path\" source\n\ndef api-binding\n  record\n    entry :columns\n      path \"response\" \"metadata\" \"fields\"\n    entry :rows\n      path \"response\" \"payload\" \"deep\" \"records\" each-index\n    entry :column column-from-meta\n\ndef api-table [input]\n  table-from-json api-binding input\n\ndef export [input]\n  pipe input\n    api-table\n    csv csv-options\n"

const export = "def export [input]\n  pipe input\n    table-from-json api-binding\n    csv csv-options\n"

func TestCanonPrintsTheCanonicalForm(t *testing.T) {
	o := invoke(t, []string{"canon", tempFile(t, "canon.alc", export)}, nil)
	if o.status != 0 || o.stdout != "(def export [input] (pipe input (table-from-json api-binding) (csv csv-options)))\n" || o.stderr != "" {
		t.Errorf("%+v", o)
	}
}

func TestFormatPrintsTheLayoutFormAndReadsStandardInput(t *testing.T) {
	o := invoke(t, []string{"format", "-"}, text("(def export [input] (pipe input (table-from-json api-binding) (csv csv-options)))"))
	if o.status != 0 || o.stdout != export {
		t.Errorf("%+v", o)
	}
}

func TestCheckIsSilentOnAProgramThatChecks(t *testing.T) {
	o := invoke(t, []string{"check", "-"}, text(program))
	if o.status != 0 || o.stdout != "" || o.stderr != "" {
		t.Errorf("%+v", o)
	}
}

// A library definition over a selection's items, which are typed Value:
// check passes it (the items may be records, or vectors), and run checks
// each one and prints the result.
func TestADefinitionOverSelectedItemsChecksAndRuns(t *testing.T) {
	for _, c := range []struct{ name, program, input, want string }{
		{"cols.alc", "def cols [input] (map public-column (select (path \"cols\" each-index) input))\ndef export [input] (join \",\" (map (fn [c] (get :label c)) (cols input)))\n",
			`{"cols":[{"label":"a","source":1},{"label":"b"}]}`, "a,b"},
		{"rows.alc", "def export [input]\n  concat-map (partial csv-row csv-options) (select (path \"rows\" each-index) input)\n",
			`{"rows":[[1,"x"],[2,"y"]]}`, "\"1\",\"x\"\r\n\"2\",\"y\"\r\n"},
	} {
		path := tempFile(t, c.name, c.program)
		if o := invoke(t, []string{"check", path}, nil); o.status != 0 {
			t.Errorf("%s: %+v", c.name, o)
		}
		if !tt.AdapterBuilt() {
			continue
		}
		if o := invoke(t, []string{"run", path, "-"}, text(c.input)); o.status != 0 || o.stdout != c.want {
			t.Errorf("%s: %+v", c.name, o)
		}
	}
	needsRun(t)
}

func TestCheckReportsAReaderFailureAsJSONOnStderrWithStatus2(t *testing.T) {
	o := invoke(t, []string{"check", "-"}, text("pipe x\n  f\n  []\n"))
	if o.status != 2 || o.stdout != "" {
		t.Errorf("%+v", o)
	}
	f := failJSON(t, o)
	if f["code"] != "DSL_PARSE_ERROR" || !strings.HasPrefix(f["message"].(string), "empty_step: ") || f["row"] != 3.0 || f["col"] != 3.0 || f["output"] != "none" {
		t.Errorf("%v", f)
	}
}

// check reaches the resolver and the checker: an unknown name, a reused
// stream and a recursive definition each fail with their code, at the
// position they name, with status 2.
func TestCheckReportsTheResolverAndCheckerCodes(t *testing.T) {
	// The composed program of the README names a binding it does not
	// define.
	o := invoke(t, []string{"check", "-"}, text(export))
	f := failJSON(t, o)
	if o.status != 2 || f["code"] != "DSL_TYPE_ERROR" || !strings.HasPrefix(f["message"].(string), "unknown_name: api-binding") || f["row"] != 3.0 || f["col"] != 21.0 {
		t.Errorf("%d %v", o.status, f)
	}
	o = invoke(t, []string{"check", "-"}, text("def export [input] (concat (json input) (json input))"))
	f = failJSON(t, o)
	if o.status != 2 || f["code"] != "STREAM_REUSED" || !strings.HasPrefix(f["message"].(string), "reused: ") {
		t.Errorf("%d %v", o.status, f)
	}
	o = invoke(t, []string{"check", "-"}, text("def a [x] (b x)\ndef b [x] (a x)\ndef export [input] (a input)"))
	if o.status != 2 || failJSON(t, o)["code"] != "STREAMABILITY_UNKNOWN" {
		t.Errorf("%+v", o)
	}
}

func TestExplainPrintsThePlanReport(t *testing.T) {
	o := invoke(t, []string{"explain", "-"}, text(program))
	if o.status != 0 || o.stderr != "" ||
		!strings.HasPrefix(o.stdout, "export: api-table → csv\n\nSource reads:          1\n") ||
		!strings.Contains(o.stdout, "Protocol:              JsonEvents/1 → TableRows/1 → Text\n") ||
		!strings.Contains(o.stdout, "Row capture:           one .response.payload.deep.records[*], capped at max_record_bytes\n") ||
		!strings.HasSuffix(o.stdout, "A later error can occur after earlier output has been written.\n") {
		t.Errorf("%+v", o)
	}
	o = invoke(t, []string{"explain", "-"}, text("def x 1"))
	if o.status != 2 || !strings.HasPrefix(failJSON(t, o)["message"].(string), "no_export: ") {
		t.Errorf("%+v", o)
	}
}

func TestRunPrintsTheWorkedExampleCsvBothWays(t *testing.T) {
	needsRun(t)
	programPath := tempFile(t, "export.alc", program)
	input := tempFile(t, "records.json", records)
	for _, c := range []struct {
		args  []string
		stdin *string
	}{
		{[]string{"run", programPath, input}, nil},
		{[]string{"run", "--no-native", programPath, input}, nil},
		{[]string{"run", programPath, "-"}, text(records)},
		{[]string{"run", "--no-native", "-", input}, text(program)},
	} {
		if o := invoke(t, c.args, c.stdin); o.status != 0 || o.stdout != expectedCSV || o.stderr != "" {
			t.Errorf("%v: %+v", c.args, o)
		}
	}
}

func TestRunRendersATableResultAsCsvOrJSON(t *testing.T) {
	needsRun(t)
	table := tempFile(t, "table.alc", strings.Replace(program, "    csv csv-options\n", "", 1))
	if o := invoke(t, []string{"run", table, "-"}, text(records)); o.status != 0 || o.stdout != expectedCSV {
		t.Errorf("%+v", o)
	}
	if o := invoke(t, []string{"run", "--render", "json", table, "-"}, text(records)); o.status != 0 ||
		o.stdout != "[{\"Identifier\":123,\"Full name\":\"Alice\",\"Balance\":50.25},{\"Identifier\":456,\"Full name\":\"Bob\",\"Balance\":72}]\n" {
		t.Errorf("%+v", o)
	}
	// The echo.
	echo := tempFile(t, "echo.alc", "def export [input] input\n")
	if o := invoke(t, []string{"run", echo, "-"}, text(`[1.50, {"a": null}]`)); o.status != 0 || o.stdout != "[1.50,{\"a\":null}]\n" {
		t.Errorf("%+v", o)
	}
}

// A renderer refused for a program, before any document is read: CSV for
// a result of JSON events, any renderer for a program that renders its
// own text.
func TestRunRefusesARendererBeforeReading(t *testing.T) {
	echo := tempFile(t, "echo.alc", "def export [input] input\n")
	o := invoke(t, []string{"run", "--render", "csv", echo, "-"}, text("[1]"))
	if o.status != 2 || !strings.HasPrefix(failJSON(t, o)["message"].(string), "protocol_mismatch: ") {
		t.Errorf("%+v", o)
	}
	own := tempFile(t, "text.alc", program)
	o = invoke(t, []string{"run", "--render", "json", own, "-"}, text(records))
	if o.status != 2 || !strings.HasPrefix(failJSON(t, o)["message"].(string), "render_of_text: ") {
		t.Errorf("%+v", o)
	}
}

// The statuses follow the code: 1 for an input or protocol failure, 5 for
// a limit, 2 for the program; the failure is one JSON object on standard
// error and standard output carries nothing but the answer.
func TestRunStatusesFollowTheFailureCode(t *testing.T) {
	// A program that does not check: status 2, nothing read from the input.
	bad := tempFile(t, "bad.alc", "def export [input] (nope input)\n")
	if o := invoke(t, []string{"run", bad, "-"}, text("not json")); o.status != 2 || failJSON(t, o)["code"] != "DSL_TYPE_ERROR" {
		t.Errorf("%+v", o)
	}
	needsRun(t)
	programPath := tempFile(t, "export2.alc", program)
	// Metadata after the rows: INPUT_ORDER_VIOLATION, status 1, no output.
	reordered := `{"response":{"payload":{"deep":{"records":[{"id":1,"person":{"name":"a"},"account":{"balance":2}}]}},"metadata":{"fields":[{"title":"Identifier","path":["id"]}]}}}`
	o := invoke(t, []string{"run", programPath, "-"}, text(reordered))
	f := failJSON(t, o)
	if o.status != 1 || o.stdout != "" || f["code"] != "INPUT_ORDER_VIOLATION" || f["output"] != "none" || !strings.HasPrefix(fmt.Sprint(f["path"]), ".response.payload") {
		t.Errorf("%d %q %v", o.status, o.stdout, f)
	}
	// Invalid JSON after the rows: INPUT_INVALID, status 1; the rows were
	// buffered, not committed, so nothing reached standard output.
	o = invoke(t, []string{"run", programPath, "-"}, text(records+" x"))
	f = failJSON(t, o)
	if o.status != 1 || o.stdout != "" || f["code"] != "INPUT_INVALID" || f["output"] != "none" {
		t.Errorf("%d %q %v", o.status, o.stdout, f)
	}
	// A missing cell under the standard options: MISSING_VALUE, status 1.
	missing := `{"response":{"metadata":{"fields":[{"title":"Identifier","path":["id"]},{"title":"Full name","path":["person","name"]}]},"payload":{"deep":{"records":[{"id":1}]}}}}`
	if o := invoke(t, []string{"run", programPath, "-"}, text(missing)); o.status != 1 || failJSON(t, o)["code"] != "MISSING_VALUE" {
		t.Errorf("%+v", o)
	}
}

func TestAReaderErrorCarriesTheEngineCodeAndPosition(t *testing.T) {
	o := invoke(t, []string{"canon", "-"}, text("a\n  b\n c\n"))
	f := failJSON(t, o)
	if o.status != 2 || f["code"] != "DSL_PARSE_ERROR" || !strings.HasPrefix(f["message"].(string), "bad_dedent: ") || f["row"] != 3.0 || f["col"] != 2.0 {
		t.Errorf("%d %v", o.status, f)
	}
}

// The reader bounds nesting, so a program nested far past the bound is a
// too_deep failure with status 2 from every command.
func TestAProgramNestedBeyondTheBoundIsAFailureNotAnAbort(t *testing.T) {
	depth := 100_000
	parens := strings.Repeat("(", depth) + "x" + strings.Repeat(")", depth)
	var indented strings.Builder
	for level := 0; level < 600; level++ {
		indented.WriteString(strings.Repeat("  ", level) + "x\n")
	}
	// Flat to the reader, one level per step to the desugarer.
	pipe := "pipe x" + strings.Repeat(" f", depth) + "\n"
	// The failure names the opener, the line or the form that passed the
	// bound: the 256th paren, the line that would open the 256th level,
	// the pipe.
	for _, c := range []struct {
		command, program string
		row              float64
	}{
		{"canon", parens, 1}, {"format", parens, 1}, {"check", parens, 1}, {"explain", parens, 1},
		{"canon", indented.String(), 257}, {"check", pipe, 1},
	} {
		o := invoke(t, []string{c.command, "-"}, text(c.program))
		if o.status != 2 || o.stdout != "" {
			t.Errorf("%s: %d %q", c.command, o.status, o.stdout)
			continue
		}
		f := failJSON(t, o)
		if f["code"] != "DSL_PARSE_ERROR" || !strings.HasPrefix(f["message"].(string), "too_deep: ") || f["row"] != c.row {
			t.Errorf("%s: %v", c.command, f)
		}
	}
}

func TestUsageErrorsAndUnreadableFilesExit2(t *testing.T) {
	o := invoke(t, nil, nil)
	if o.status != 2 || !strings.Contains(o.stderr, "usage:") || o.stdout != "" {
		t.Errorf("%+v", o)
	}
	// An unknown command is refused before standard input is read, so what
	// it holds does not matter: unparsable input is not a parse error here.
	o = invoke(t, []string{"bogus", "-"}, text("("))
	if o.status != 2 || failJSON(t, o)["code"] != "INPUT_INVALID" || o.stdout != "" {
		t.Errorf("%+v", o)
	}
	// A wrong argument count, a bad renderer, two standard inputs.
	for _, args := range [][]string{
		{"run", "-"},
		{"run", "--render", "xml", "a.alc", "b.json"},
		{"run", "--bogus", "a.alc", "b.json"},
		{"run", "-", "-"},
		{"explain"},
	} {
		o := invoke(t, args, nil)
		if o.status != 2 || failJSON(t, o)["code"] != "INPUT_INVALID" || o.stdout != "" {
			t.Errorf("%v: %+v", args, o)
		}
	}
	o = invoke(t, []string{"canon", "/nonexistent/program.alc"}, nil)
	if o.status != 2 || failJSON(t, o)["code"] != "INPUT_INVALID" || o.stdout != "" {
		t.Errorf("%+v", o)
	}
}

// A program inside the nesting bound checks, explains and runs: a pipe of
// 245 steps.
func TestAProgramInsideTheBoundRuns(t *testing.T) {
	path := tempFile(t, "pipe245.alc", "def export [input]\n  pipe input\n    select (path each-index)\n"+strings.Repeat("    map (fn [x] x)\n", 245)+"    join \",\"\n")
	for _, command := range []string{"check", "explain"} {
		if o := invoke(t, []string{command, path}, nil); o.status != 0 {
			t.Errorf("%s: %+v", command, o)
		}
	}
	needsRun(t)
	if o := invoke(t, []string{"run", path, "-"}, text(`["a","b"]`)); o.status != 0 || o.stdout != "a,b" {
		t.Errorf("%+v", o)
	}
}

// A function applied to itself is recursion with status 2 from check
// (which builds the plan) and from run.
func TestSelfApplicationIsAFailureNotAnAbort(t *testing.T) {
	omega := tempFile(t, "omega.alc", "def w [f] (f f)\ndef export [input]\n  let [x (w w)]\n    json input\n")
	o := invoke(t, []string{"check", omega}, nil)
	f := failJSON(t, o)
	if o.status != 2 || f["code"] != "STREAMABILITY_UNKNOWN" || !strings.HasPrefix(f["message"].(string), "recursion: ") {
		t.Errorf("%d %v", o.status, f)
	}
	needsRun(t)
	perItem := tempFile(t, "omega-item.alc", "def w [f] (f f)\ndef export [input]\n  pipe input\n    select (path each-index)\n    map (fn [x] (w w))\n    join \",\"\n")
	o = invoke(t, []string{"run", perItem, "-"}, text("[1]"))
	f = failJSON(t, o)
	if o.status != 2 || f["code"] != "STREAMABILITY_UNKNOWN" || !strings.HasPrefix(f["message"].(string), "recursion: ") {
		t.Errorf("%d %v", o.status, f)
	}
}

// check builds the plan, so a fail on export's own path is reported
// there, with its code (INPUT_INVALID, status 1) and the form's position,
// although no document was read.
func TestCheckReportsAFailThePlanReaches(t *testing.T) {
	o := invoke(t, []string{"check", tempFile(t, "fail.alc", "def export [input] (concat \"a\" (fail \"boom\"))\n")}, nil)
	f := failJSON(t, o)
	if o.status != 1 || f["code"] != "INPUT_INVALID" || f["message"] != "boom" || f["row"] != 1.0 || f["col"] != 32.0 {
		t.Errorf("%d %v", o.status, f)
	}
}

// A failure inside the standard library names the library's file, row and
// column in its message, and gives no row or column of the user's file; a
// program that happens to share a library file's name is still read as
// its own text.
func TestALibraryFailureNamesTheLibraryFile(t *testing.T) {
	needsRun(t)
	lib, _ := alchemy.StdlibSource("stdlib/table.alc")
	row, col := 0, 0
	for i, line := range strings.Split(lib, "\n") {
		if strings.Contains(line, `fail "Required metadata was not found"`) {
			row, col = i+1, strings.Index(line, "fail")+1
		}
	}
	dir := t.TempDir()
	if err := os.MkdirAll(filepath.Join(dir, "stdlib"), 0o755); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"export.alc", "stdlib/table.alc"} {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(program), 0o644); err != nil {
			t.Fatal(err)
		}
		o := runIn(t, dir, []string{"run", "--no-native", name, "-"}, text("{}"))
		f := failJSON(t, o)
		if o.status != 1 || f["code"] != "INPUT_INVALID" || f["message"] != fmt.Sprintf("Required metadata was not found (at stdlib/table.alc:%d:%d)", row, col) {
			t.Errorf("%s: %d %v", name, o.status, f)
		}
		if _, has := f["row"]; has {
			t.Errorf("%s: %v", name, f)
		}
	}
}

// A standard error that cannot be written loses the report, not the
// status: the failure's own status comes back, never a signal's.
func TestAClosedStandardErrorKeepsTheStatus(t *testing.T) {
	bad := tempFile(t, "closed.alc", "def export [input] (nope input)\n")
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	r.Close()
	cmd := exec.Command(binary, "check", bad)
	cmd.Stderr = w
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	w.Close()
	err = cmd.Wait()
	var exit *exec.ExitError
	if !errors.As(err, &exit) || exit.ExitCode() != 2 {
		t.Errorf("%v", err)
	}
}

// --max-output-bytes bounds what run writes, and a text longer than it
// fails with the limit named, status 5.
func TestRunTakesAnOutputLimit(t *testing.T) {
	programPath := tempFile(t, "limit.alc", program)
	if o := invoke(t, []string{"run", "--max-output-bytes", "many", programPath, "-"}, text(records)); o.status != 2 || failJSON(t, o)["code"] != "INPUT_INVALID" {
		t.Errorf("%+v", o)
	}
	needsRun(t)
	o := invoke(t, []string{"run", "--max-output-bytes", "20", programPath, "-"}, text(records))
	f := failJSON(t, o)
	if limit, _ := f["limit"].(map[string]any); o.status != 5 || limit["name"] != "max_output_bytes" || len(o.stdout) > 20 {
		t.Errorf("%d %v %q", o.status, f, o.stdout)
	}
	if o := invoke(t, []string{"run", "--max-output-bytes", "1000", programPath, "-"}, text(records)); o.status != 0 || o.stdout != expectedCSV {
		t.Errorf("%+v", o)
	}
}

// In a build without the incremental adapter, run refuses with the
// source's STREAMABILITY_UNKNOWN, status 2, before the document is read
// and writing nothing, rather than read it through the materialized walk,
// which would not be the run the plan report promises (number lexemes
// lost, a repeated member collapsed, the whole document held).
func TestRunRefusesWithoutTheIncrementalAdapter(t *testing.T) {
	if tt.AdapterBuilt() {
		t.Skip("this build has the incremental adapter; the run cases above run instead")
	}
	echo := tempFile(t, "echo.alc", "def export [input] (json input)\n")
	o := invoke(t, []string{"run", echo, "-"}, text(`{"a":[1.50,1e2],"a":2}`))
	f := failJSON(t, o)
	if o.status != 2 || o.stdout != "" || f["code"] != "STREAMABILITY_UNKNOWN" || f["output"] != "none" || !strings.Contains(f["message"].(string), "tabnas_nodecell") {
		t.Errorf("%d %q %v", o.status, o.stdout, f)
	}
}
