// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// spec_test.go: the shared fixtures in ../test/spec, run through
// tabnas/support's runner as every grammar repository runs its own: the
// loader, the escape codec, the ERROR:<code> contract and the row loop are
// the fleet's. What is specific to alchemy is what a row's input becomes:
// the canonical form of the parsed program (reader.tsv), or of the
// desugared program (pipe.tsv), as a JSON string in the expected column;
// the plan report or the failure (check.tsv).

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	support "github.com/tabnas/support/go"
)

func specDir(t *testing.T) string {
	t.Helper()
	dir, err := support.FindSpecDir("")
	if err != nil {
		t.Fatal(err)
	}
	return dir
}

func repoRoot(t *testing.T) string {
	t.Helper()
	return filepath.Dir(filepath.Dir(specDir(t)))
}

// failError is a *Fail as the runner sees it: an error whose code is the
// finer code (the first word of the message) and whose position is the
// failure's.
type failError struct{ f *Fail }

func (e failError) Error() string { return e.f.Error() }

func asErr(f *Fail) error {
	if f == nil {
		return nil
	}
	return failError{f}
}

func failOf(err error) (*Fail, bool) {
	var fe failError
	if errors.As(err, &fe) {
		return fe.f, true
	}
	return nil, false
}

// runner is the support runner over one input-to-text function.
func runner(parse func(input string) (string, *Fail)) support.Runner {
	return support.Runner{
		Parse: func(input string) (any, error) {
			text, f := parse(input)
			if f != nil {
				return nil, asErr(f)
			}
			return text, nil
		},
		ErrorCode: func(err error) string {
			if f, ok := failOf(err); ok {
				return f.FinerCode()
			}
			return err.Error()
		},
		ErrorPos: func(err error) (int, int, bool) {
			if f, ok := failOf(err); ok && f.Row > 0 {
				return int(f.Row), int(f.Column), true
			}
			return 0, 0, false
		},
	}
}

func readerRow(input string) (string, *Fail) {
	program, f := Parse(input)
	if f != nil {
		return "", f
	}
	return Canonical(program), nil
}

func pipeRow(input string) (string, *Fail) {
	program, f := Parse(input)
	if f != nil {
		return "", f
	}
	core, f := Desugar(program, input)
	if f != nil {
		return "", f
	}
	return Canonical(core), nil
}

// The fixtures this package runs, and the one it does not yet: run.tsv is
// the interpreter's, the second half of the port.
var specRunners = map[string]bool{
	"reader.tsv": true,
	"pipe.tsv":   true,
	"check.tsv":  true,
	"run.tsv":    false,
}

// TestEveryFixtureHasARunner fails when a fixture is added without a
// runner here, rather than passing silently.
func TestEveryFixtureHasARunner(t *testing.T) {
	files, err := filepath.Glob(filepath.Join(specDir(t), "*.tsv"))
	if err != nil {
		t.Fatal(err)
	}
	var names, known []string
	for _, f := range files {
		names = append(names, filepath.Base(f))
	}
	for name := range specRunners {
		known = append(known, name)
	}
	sort.Strings(names)
	sort.Strings(known)
	if len(names) != len(known) {
		t.Fatalf("fixtures %v, runners %v", names, known)
	}
	for i := range names {
		if names[i] != known[i] {
			t.Fatalf("fixtures %v, runners %v", names, known)
		}
	}
}

func TestSpecReader(t *testing.T) {
	runner(readerRow).File(t, filepath.Join(specDir(t), "reader.tsv"))
}

func TestSpecPipe(t *testing.T) {
	runner(pipeRow).File(t, filepath.Join(specDir(t), "pipe.tsv"))
}

func TestSpecRunIsTheInterpretersFixture(t *testing.T) {
	if _, err := os.Stat(filepath.Join(specDir(t), "run.tsv")); err != nil {
		t.Fatal(err)
	}
	t.Skip("run.tsv: needs interpreter (pass 2)")
}

// needsInterpreter are the check.tsv rows the front end cannot finish, by
// their input (the long report rows by checkRowKey) and the reason: check builds the
// plan, so a failure only the plan's evaluation meets, and every plan
// report, is the interpreter's. Each one must pass the front end (the
// test says so when one does not), so the list cannot hide a front-end
// failure; the interpreter removes it.
var needsInterpreter = map[string]string{
	// The evaluator's recursion: a function applied to itself.
	"def w [f] (f f)\ndef export [input]\n  let [x (w w)]\n    json input": "recursion",
	// A vector refuses a live text where it is built.
	`def export [input] (join "," [(json input)])`: "type_mismatch",
	// A record with a key twice; a match no case takes.
	`def export [input] (concat (get :a (record (entry :a "1") (entry :a "2"))) (json input))`: "duplicate_key",
	`def export [input] (concat (match 5 (case 1 "one")) (json input))`:                        "no_match",
	// The stack operators and repeat refuse their values where the plan's
	// evaluation reaches them.
	"def export [input] (let [x (pop [])] (json input))":                  "type_mismatch",
	"def export [input] (let [x (top [])] (json input))":                  "type_mismatch",
	`def export [input] (let [x (repeat -1 "ab")] (json input))`:          "type_mismatch",
	`def export [input] (let [x (string-join "," ["a" 1])] (json input))`: "type_mismatch",
	// The plan reports: the summary reads the plan the evaluator builds
	// (effects_test.go holds the report text to these rows through the
	// views the interpreter will build).
	"def export [input]\n  json input": "report",
	"def export [input] input":         "report",
	`def export [input] "done"`:        "report",
	"report: worked example":           "report",
	"report: events":                   "report",
	"report: inferred binding":         "report",
}

// reportRowKey names the long report rows by what they are, so the list
// above reads; the rest are named by their input.
func checkRowKey(input string) string {
	switch {
	case len(input) > 30 && input[:30] == "def column-from-meta [source]\n":
		return "report: worked example"
	case len(input) > 26 && input[:26] == "def key-lines [s event]\n  ":
		return "report: events"
	case len(input) > 16 && input[:16] == "def rows-binding":
		return "report: inferred binding"
	}
	return input
}

func checkRow(input string) (string, *Fail) {
	if _, f := Analyze(input, "check"); f != nil {
		return "", f
	}
	return "", nil
}

// TestSpecCheck runs check.tsv: a program that fails the front end fails
// with the resolver's or the checker's code, at the position it names. The
// rows only the interpreter can finish are skipped, each by name, after
// the front end has passed them.
func TestSpecCheck(t *testing.T) {
	spec, err := support.LoadSpec(filepath.Join(specDir(t), "check.tsv"), nil)
	if err != nil {
		t.Fatal(err)
	}
	r := runner(checkRow)
	seen := map[string]bool{}
	ran, skipped := 0, 0
	for _, row := range spec.Rows {
		input := row.Unesc(0)
		expected := row.Col(1)
		key := checkRowKey(input)
		t.Run(fmt.Sprintf("row %d", row.Line), func(t *testing.T) {
			if want, ok := needsInterpreter[key]; ok {
				seen[key] = true
				skipped++
				if _, f := Analyze(input, "check"); f != nil {
					t.Fatalf("%s: listed as needing the interpreter (%s), but the front end refuses it: %v", row.Where(), want, f)
				}
				if want != "report" && expected != "ERROR:"+want && !strings.HasPrefix(expected, "ERROR:"+want+"@") {
					t.Fatalf("%s: listed as %s, but the fixture expects %s", row.Where(), want, expected)
				}
				t.Skip("needs interpreter (pass 2): " + want)
			}
			ran++
			if err := r.CheckRow(row, input, expected); err != nil {
				t.Error(err)
			}
		})
	}
	for key := range needsInterpreter {
		if !seen[key] {
			t.Errorf("needsInterpreter lists %q, which no row of check.tsv is", key)
		}
	}
	t.Logf("check.tsv: %d rows run, %d need the interpreter", ran, skipped)
}

// TestFormatRoundTripsEveryFixtureRow: Format prints a program the reader
// reads back to the same forms, spans aside, for every program the
// fixtures parse.
func TestFormatRoundTripsEveryFixtureRow(t *testing.T) {
	specs, err := support.LoadSpecDir(specDir(t), nil)
	if err != nil {
		t.Fatal(err)
	}
	rows := 0
	for _, spec := range specs {
		for _, row := range spec.Rows {
			if support.IsErrorExpect(row.Col(1)) {
				continue
			}
			rows++
			input := row.Unesc(0)
			program, f := Parse(input)
			if f != nil {
				continue // the fixture's own runner reports it
			}
			layout := Format(program)
			again, f := Parse(layout)
			if f != nil {
				t.Errorf("%s: the layout form does not parse: %v\n  layout: %q", row.Where(), f, layout)
				continue
			}
			if !SameProgram(program, again) {
				t.Errorf("%s: format changed the program\n  input:  %q\n  layout: %q", row.Where(), input, layout)
			}
		}
	}
	if rows == 0 {
		t.Fatal("the fixtures hold no value rows")
	}
}

// TestEveryLanguageReferenceExampleIsAFixtureRow: each `alchemy` block of
// docs/language.md with the canonical, core or check block after it is a
// row of reader.tsv, pipe.tsv or check.tsv, and the result shown is that
// row's expected value.
func TestEveryLanguageReferenceExampleIsAFixtureRow(t *testing.T) {
	doc, err := os.ReadFile(filepath.Join(repoRoot(t), "docs", "language.md"))
	if err != nil {
		t.Fatal(err)
	}
	rows := func(file string) map[string]string {
		spec, err := support.LoadSpec(filepath.Join(specDir(t), file), nil)
		if err != nil {
			t.Fatal(err)
		}
		out := map[string]string{}
		for _, row := range spec.Rows {
			out[strings.TrimRight(row.Unesc(0), "\n")] = row.Col(1)
		}
		return out
	}
	fixtures := map[string]map[string]string{
		"canonical": rows("reader.tsv"),
		"core":      rows("pipe.tsv"),
		"check":     rows("check.tsv"),
	}
	type block struct{ language, text string }
	var blocks []block
	open := false
	var language string
	var text []string
	for _, line := range strings.Split(string(doc), "\n") {
		rest, fence := strings.CutPrefix(line, "```")
		switch {
		case !open && fence && rest != "":
			open, language, text = true, rest, nil
		case open && fence && rest == "":
			blocks = append(blocks, block{language, strings.Join(text, "\n")})
			open = false
		case open:
			text = append(text, line)
		}
	}
	examples := 0
	for i, b := range blocks {
		if b.language != "alchemy" {
			continue
		}
		examples++
		if i+1 >= len(blocks) || fixtures[blocks[i+1].language] == nil {
			t.Errorf("%q: an alchemy block is followed by a canonical, core or check block", b.text)
			continue
		}
		cell, ok := fixtures[blocks[i+1].language][strings.TrimRight(b.text, "\n")]
		if !ok {
			t.Errorf("%q: not a fixture row", b.text)
			continue
		}
		shown := cell
		if !support.IsErrorExpect(cell) {
			v, err := support.ParseExpect(cell)
			s, isString := v.(string)
			if err != nil || !isString {
				t.Errorf("%q: the fixture expects %v", b.text, v)
				continue
			}
			shown = s
		}
		if strings.TrimRight(shown, "\n") != blocks[i+1].text {
			t.Errorf("%q: the page shows %q, the fixture pins %q", b.text, blocks[i+1].text, shown)
		}
	}
	if examples == 0 {
		t.Fatal("the reference holds no examples")
	}
}
