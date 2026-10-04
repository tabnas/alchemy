// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// spec_test.go: the shared fixtures in ../test/spec, run through
// tabnas/support's runner as every grammar repository runs its own: the
// loader, the escape codec, the ERROR:<code> contract and the row loop are
// the fleet's. What is specific to alchemy is what a row's input becomes:
// the canonical form of the parsed program (reader.tsv), or of the
// desugared program (pipe.tsv), as a JSON string in the expected column;
// the plan report or the failure (check.tsv); or the bytes a run writes
// (run.tsv).

import (
	"bytes"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	tabnasjson "github.com/tabnas/json/go"
	support "github.com/tabnas/support/go"
	tt "github.com/tabnas/transduce/go"
)

func specDir(t testing.TB) string {
	t.Helper()
	dir, err := support.FindSpecDir("")
	if err != nil {
		t.Fatal(err)
	}
	return dir
}

func repoRoot(t testing.TB) string {
	t.Helper()
	return filepath.Dir(filepath.Dir(specDir(t)))
}

// failError is a *Fail as the runner sees it: an error whose code is the
// one a row pins (runCode: the finer code for this package's own codes,
// the transduce code for any other, in every file, as test/AGENTS.md asks
// of a port's runners) and whose position is the failure's.
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
				return runCode(f)
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

// specRunners are the fixtures this package runs, each by a test below.
var specRunners = map[string]bool{
	"reader.tsv": true,
	"pipe.tsv":   true,
	"check.tsv":  true,
	"run.tsv":    true,
}

// TestEveryFixtureHasARunner fails when a fixture is added without a
// runner here, rather than passing silently; and test/AGENTS.md, the
// fixtures' guide, describes each one.
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
	if strings.Join(names, " ") != strings.Join(known, " ") {
		t.Fatalf("fixtures %v, runners %v", names, known)
	}
	guide, err := os.ReadFile(filepath.Join(repoRoot(t), "test", "AGENTS.md"))
	if err != nil {
		t.Fatal(err)
	}
	for _, name := range names {
		if !strings.Contains(string(guide), "[`"+name+"`](spec/"+name+")") {
			t.Errorf("test/AGENTS.md does not describe %s", name)
		}
	}
}

func TestSpecReader(t *testing.T) {
	runner(readerRow).File(t, filepath.Join(specDir(t), "reader.tsv"))
}

func TestSpecPipe(t *testing.T) {
	runner(pipeRow).File(t, filepath.Join(specDir(t), "pipe.tsv"))
}

// checkRow is a program that checks as its plan report; one that does not
// fails with the resolver's, the checker's or the plan evaluation's code,
// at the position it names.
func checkRow(input string) (string, *Fail) {
	program, f := Compile(input, "check", routers, renderers)
	if f != nil {
		return "", f
	}
	return program.Explain(), nil
}

// TestSpecCheck runs check.tsv: compile builds the plan, so a failure
// only the plan's evaluation meets has a row here too, and every plan
// report is the plan's.
func TestSpecCheck(t *testing.T) {
	runner(checkRow).File(t, filepath.Join(specDir(t), "check.tsv"))
}

// runCode is the code a run row pins: the finer code (the first word of
// the message) for this package's own codes, which carry one, and the
// transduce or render code itself for every other failure, whose message
// is free text (`fail "a: b"` must not pin `a`).
func runCode(f *Fail) string {
	switch f.Code {
	case CodeDSLParseError, CodeDSLTypeError, CodeStreamReused, CodeStreamabilityUnknown:
		return FinerCode(f)
	}
	return f.Code.String()
}

// runRender is the renderer a run row names.
func runRender(t testing.TB, row *support.Row) Renderer {
	name := row.Named("render")
	if name == "" {
		return RenderDefault
	}
	render, ok := RendererNamed(name)
	if !ok {
		t.Fatalf("%s: render is csv, json or empty, not %q", row.Where(), name)
	}
	return render
}

// runDoc is the document a run row reads: an empty cell is null.
func runDoc(row *support.Row) string {
	if doc := row.UnescNamed("doc"); doc != "" {
		return doc
	}
	return "null"
}

// runOutcome is one run of a row's program: the bytes it wrote, or the
// failure.
type runOutcome struct {
	out  string
	fail *Fail
}

func (o runOutcome) String() string {
	if o.fail != nil {
		return "Err(" + o.fail.Error() + ")"
	}
	return fmt.Sprintf("Ok(%q)", o.out)
}

func (o runOutcome) agrees(other runOutcome) bool {
	switch {
	case o.fail == nil && other.fail == nil:
		return o.out == other.out
	case o.fail != nil && other.fail != nil:
		return runCode(o.fail) == runCode(other.fail) &&
			o.fail.Row == other.fail.Row && o.fail.Column == other.fail.Column
	}
	return false
}

// runBoth compiles program and runs it over doc as `alchemy run` does: the
// document read by the JSON grammar incrementally, pruned under the
// program's row selector when it has one, with the default limits; the
// standard compositions native or through the library's text.
func runBoth(program, doc string, render Renderer, native bool) runOutcome {
	compiled, f := Compile(program, "run", routers, renderers)
	if f == nil && !native {
		compiled, f = compiled.WithNative(false)
	}
	if f != nil {
		return runOutcome{fail: f}
	}
	out, f := driveRun(compiled, doc, render, tt.DefaultLimits(), nil)
	return runOutcome{out: out, fail: f}
}

// driveRun parses doc with the json grammar incrementally, pruned under
// the program's row selector when it has one, and pushes its events
// through the program's sink: the output, or the failure. metrics may be
// nil.
func driveRun(program *Program, doc string, render Renderer, limits tt.Limits, metrics *tt.Metrics) (string, *Fail) {
	if metrics == nil {
		metrics = tt.NewMetrics()
	}
	var buffer bytes.Buffer
	sink, f := program.Sink(&buffer, render, limits, metrics)
	if f != nil {
		return "", f
	}
	prune := tt.Prune{}
	if selector, ok := program.RowSelector(); ok {
		prune = tt.PruneUnderSelector(selector)
	}
	_, f = tt.NewParserSource(tabnasjson.Make(), doc).
		Grammar("json").
		Mode(tt.IncrementalMode(prune)).
		Limits(limits).
		Metrics(metrics).
		Run(sink)
	return buffer.String(), f
}

// beforeTheSource is whether a row's run ends before its document is read:
// its program fails to compile, natively or interpreted, or its sink
// cannot be built (a renderer for a program that renders its own text).
// Such a row runs in every build; any other needs the incremental source.
func beforeTheSource(program string, render Renderer) bool {
	for _, native := range []bool{true, false} {
		compiled, f := Compile(program, "run", routers, renderers)
		if f == nil && !native {
			compiled, f = compiled.WithNative(false)
		}
		if f == nil {
			_, f = compiled.Sink(io.Discard, render, tt.DefaultLimits(), nil)
		}
		if f != nil {
			return true
		}
	}
	return false
}

// needsIncremental is the reason a test that reads a document as `alchemy
// run` does is skipped in a build without the incremental adapter.
const needsIncremental = "reads its document incrementally, as alchemy run does, and this build has no incremental adapter (build with -tags tabnas_nodecell)"

// TestSpecRun runs run.tsv: a program run over a JSON document, the bytes
// it writes or the failure. Every row runs twice, with the standard
// compositions native and through the library's text, and the two must
// agree to the byte, or on the failure's code and position: a row whose
// two paths differ fails whatever its expected cell says.
//
// A run reads its document incrementally. A build without the adapter
// (no tabnas_nodecell tag) runs the rows that end before the document is
// read, and skips the rest by name, saying why.
func TestSpecRun(t *testing.T) {
	spec, err := support.LoadSpec(filepath.Join(specDir(t), "run.tsv"), nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(spec.Rows) == 0 {
		t.Fatal("run.tsv holds no rows")
	}
	if got := strings.Join(spec.Rows[0].Header, " "); got != "input expected doc render" {
		t.Fatalf("run.tsv's columns are %s", got)
	}
	r := support.Runner{
		ParseRow: func(input string, row *support.Row) (any, error) {
			doc, render := runDoc(row), runRender(t, row)
			native := runBoth(input, doc, render, true)
			interpreted := runBoth(input, doc, render, false)
			if !native.agrees(interpreted) {
				return nil, fmt.Errorf("native and interpreted runs disagree:\n  native:      %s\n  interpreted: %s", native, interpreted)
			}
			if native.fail != nil {
				return nil, asErr(native.fail)
			}
			return native.out, nil
		},
		ErrorCode: func(err error) string {
			if f, ok := failOf(err); ok {
				return runCode(f)
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
	ran, skipped := 0, 0
	for _, row := range spec.Rows {
		input := row.Unesc(0)
		t.Run(fmt.Sprintf("row %d", row.Line), func(t *testing.T) {
			if !tt.AdapterBuilt() && !beforeTheSource(input, runRender(t, row)) {
				skipped++
				t.Skip(row.Where() + ": " + needsIncremental)
			}
			ran++
			if err := r.CheckRow(row, input, row.Col(1)); err != nil {
				t.Error(err)
			}
		})
	}
	t.Logf("run.tsv: %d rows run, %d skipped (no incremental adapter)", ran, skipped)
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
