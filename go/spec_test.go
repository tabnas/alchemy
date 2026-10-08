// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// spec_test.go: the shared fixtures in ../test/spec, run through
// tabnas/support's runner as every grammar repository runs its own: the
// loader, the escape codec, the ERROR:<code> contract and the row loop are
// the fleet's. What is specific to alchemy is what a row's input becomes:
// the canonical form of the parsed program (reader.tsv), or of the
// desugared program (pipe.tsv), as a JSON string in the expected column;
// or the plan report or the failure (check.tsv). The bytes a run writes
// (run.tsv) need transduce's routers and render's renderers, which this
// package does not depend on: alchemy-cli runs those rows (go/e2e).

import (
	"errors"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"testing"

	support "github.com/tabnas/support/go"
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

// specRunners are the fixtures, each named by the test that runs it. Every
// one is this package's but run.tsv: its rows run a program over a
// document, which needs transduce's routers and render's renderers, and
// this package depends on neither, so alchemy-cli runs it, the composition
// root that builds all three.
var specRunners = map[string]string{
	"reader.tsv": "TestSpecReader",
	"pipe.tsv":   "TestSpecPipe",
	"check.tsv":  "TestSpecCheck",
	"run.tsv":    runByCLI,
}

// runByCLI is the runner of a fixture this package does not run.
const runByCLI = "alchemy-cli's go/e2e TestSpecRun"

// TestEveryFixtureHasARunner fails when a fixture is added without a
// runner, here or in alchemy-cli, rather than passing silently; and
// test/AGENTS.md, the fixtures' guide, describes each one.
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

// rowFailures is what a row's program meets, as the runners above meet it:
// the reader's (reader.tsv), the desugarer's (pipe.tsv) or compile's
// (check.tsv).
func rowFailures(file string, row *support.Row) []*Fail {
	input := row.Unesc(0)
	var f *Fail
	switch file {
	case "reader.tsv":
		_, f = readerRow(input)
	case "pipe.tsv":
		_, f = pipeRow(input)
	case "check.tsv":
		_, f = checkRow(input)
	}
	if f == nil {
		return nil
	}
	return []*Fail{f}
}

// placeholder is a template's `{name}`; finalPlaceholder one that ends it,
// with the space before it.
var (
	placeholder      = regexp.MustCompile(`\{[a-z_]+\}`)
	finalPlaceholder = regexp.MustCompile(`\s*\{[a-z_]+\}$`)
)

// fills reports whether text fills the template line: its fixed parts in
// order, each `{name}` any text, none included.
func fills(line, text string) bool {
	parts := placeholder.Split(line, -1)
	if len(parts) == 1 {
		return text == line
	}
	first, last := parts[0], parts[len(parts)-1]
	if len(text) < len(first)+len(last) || !strings.HasPrefix(text, first) || !strings.HasSuffix(text, last) {
		return false
	}
	end := len(text) - len(last)
	at := len(first)
	for _, part := range parts[1 : len(parts)-1] {
		found := strings.Index(text[at:end], part)
		if found < 0 {
			return false
		}
		at += found + len(part)
	}
	return true
}

// instanceOf reports whether text is an instance of the template line. The
// engine trims the messages it writes from a template, so a `{name}` that
// ends a line may take the space before it with it.
func instanceOf(line, text string) bool {
	if fills(line, text) {
		return true
	}
	bare := finalPlaceholder.ReplaceAllString(line, "")
	return bare != line && fills(bare, text)
}

// TestTheRaisedMessagesMatchTheDocument: every failure the fixtures this
// package runs meet is declared in the grammar document
// (rs/tests/spec_test.rs, the_raised_messages_match_the_document). The
// finer code that leads the message is a key of the installed error
// messages, the engine's and this grammar's, and the text after it is a
// line of that entry, each `{name}` standing for what the raising site
// fills in (a failure raised inside the standard library ends with its
// position there, ` (at stdlib/...)`, which is not part of the text). Each
// of the document's codes has a hint. A finer code comes with the same
// code wherever it is raised (bad_let is a DSL_PARSE_ERROR from the
// desugarer and from the resolver alike). A raising site whose code or
// text drifts from the document fails here.
//
// The rows are reader.tsv's, pipe.tsv's and check.tsv's. run.tsv's run
// programs over documents, on transduce's routers and render's renderers,
// which this package does not depend on: alchemy-cli's copy of this test
// (go/e2e/spec_test.go) reads every error row of all four files, and holds
// the document to the clause that needs them all: each line of each code
// the document declares is the most specific line some row meets, so the
// document holds no text nothing raises.
func TestTheRaisedMessagesMatchTheDocument(t *testing.T) {
	installed := Make().Config().ErrorMessages
	doc, err := documentValue()
	if err != nil {
		t.Fatal(err)
	}
	options := doc["options"].(map[string]any)
	declared := options["error"].(map[string]any)
	hints := options["hint"].(map[string]any)
	specs, err := support.LoadSpecDir(specDir(t), nil)
	if err != nil {
		t.Fatal(err)
	}
	// The codes each finer code is raised with: one, wherever it is raised.
	raisedAs := map[string]map[Code]bool{}
	failures := 0
	for _, spec := range specs {
		if specRunners[spec.Name] == runByCLI {
			continue
		}
		for _, row := range spec.Rows {
			if !support.IsErrorExpect(row.Col(1)) {
				continue
			}
			for _, f := range rowFailures(spec.Name, row) {
				switch f.Code {
				case CodeDSLParseError, CodeDSLTypeError, CodeStreamReused, CodeStreamabilityUnknown:
				default:
					continue
				}
				failures++
				code, text, ok := strings.Cut(f.Message, ": ")
				if !ok {
					t.Errorf("%s: %s has no finer code: %s", row.Where(), f.Code, f.Message)
					continue
				}
				if raisedAs[code] == nil {
					raisedAs[code] = map[Code]bool{}
				}
				raisedAs[code][f.Code] = true
				if library := strings.LastIndex(text, " (at stdlib/"); library >= 0 && strings.HasSuffix(text, ")") {
					text = text[:library]
				}
				entry, ok := installed[code]
				if !ok {
					t.Errorf("%s: %s is not declared in options.error", row.Where(), code)
					continue
				}
				found := false
				for _, line := range strings.Split(entry, "\n") {
					if instanceOf(line, text) {
						found = true
						break
					}
				}
				if !found {
					t.Errorf("%s: no line of options.error.%s is %q", row.Where(), code, text)
				}
			}
		}
	}
	if failures == 0 {
		t.Fatal("the fixtures meet no failures")
	}
	for code, uppers := range raisedAs {
		if len(uppers) > 1 {
			names := make([]string, 0, len(uppers))
			for upper := range uppers {
				names = append(names, upper.String())
			}
			sort.Strings(names)
			t.Errorf("%s is raised as %s; a finer code has one code", code, strings.Join(names, " and "))
		}
	}
	for code := range declared {
		if _, ok := hints[code].(string); !ok {
			t.Errorf("options.hint.%s is not declared", code)
		}
	}
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
