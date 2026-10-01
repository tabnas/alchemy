// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// grammar_test.go: the reader's tagged tree, its error codes, and the
// grammar document held to the code that installs it (rs/src/grammar.rs's
// tests).

import (
	"errors"
	"sort"
	"strings"
	"testing"

	tabnas "github.com/tabnas/parser/go"
)

// valueJSON writes a ParseValue tree as compact JSON, its keys in order.
func valueJSON(v any) string {
	var conv func(any) any
	conv = func(v any) any {
		switch x := v.(type) {
		case *tabnas.OrderedMap:
			o := &JSONObject{}
			for _, k := range x.Keys {
				item, _ := x.Get(k)
				o.set(k, conv(item))
			}
			return o
		case []any:
			out := make([]any, len(x))
			for i, item := range x {
				out[i] = conv(item)
			}
			return out
		}
		return v
	}
	return EncodeJSON(conv(v))
}

func parsedJSON(t *testing.T, src string) string {
	t.Helper()
	v, err := ParseValue(src)
	if err != nil {
		t.Fatalf("%q: %v", src, err)
	}
	return valueJSON(v)
}

func engineCode(t *testing.T, src string) string {
	t.Helper()
	v, err := ParseValue(src)
	if err == nil {
		t.Fatalf("%q parsed: %s", src, valueJSON(v))
	}
	var te *tabnas.TabnasError
	if !errors.As(err, &te) {
		t.Fatalf("%q: not an engine error: %v", src, err)
	}
	return te.Code
}

func TestAnEmptyProgramIsAnEmptyArray(t *testing.T) {
	for _, src := range []string{"", "\n\n; only a comment\n  \n"} {
		if got := parsedJSON(t, src); got != "[]" {
			t.Errorf("%q: %s", src, got)
		}
	}
}

func TestALineWithOneFormIsThatForm(t *testing.T) {
	for _, c := range [][2]string{
		{"x", `[{"$":"sym","name":"x","span":[0,1]}]`},
		{"\"a\\nb\"", `[{"$":"str","value":"a\nb","span":[0,6]}]`},
		{"-1.5e3", `[{"$":"num","lexeme":"-1.5e3","span":[0,6]}]`},
		{"true", `[{"$":"bool","value":true,"span":[0,4]}]`},
		{"null", `[{"$":"null","span":[0,4]}]`},
		{":k", `[{"$":"kw","name":"k","span":[0,2]}]`},
	} {
		if got := parsedJSON(t, c[0]); got != c[1] {
			t.Errorf("%q:\n got %s\nwant %s", c[0], got, c[1])
		}
	}
}

func TestSeveralInlineFormsMakeAList(t *testing.T) {
	want := `[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[2,3]}],"span":[0,3]}]`
	if got := parsedJSON(t, "a b"); got != want {
		t.Errorf("got %s", got)
	}
}

func TestExplicitDelimitersCarryTheirOwnSpans(t *testing.T) {
	for _, c := range [][2]string{
		{"( a )", `[{"$":"list","items":[{"$":"sym","name":"a","span":[2,3]}],"span":[0,5]}]`},
		{"()", `[{"$":"list","items":[],"span":[0,2]}]`},
		{"[]", `[{"$":"vector","items":[],"span":[0,2]}]`},
		{"[1 [2]]", `[{"$":"vector","items":[{"$":"num","lexeme":"1","span":[1,2]},{"$":"vector","items":[{"$":"num","lexeme":"2","span":[4,5]}],"span":[3,6]}],"span":[0,7]}]`},
	} {
		if got := parsedJSON(t, c[0]); got != c[1] {
			t.Errorf("%q:\n got %s\nwant %s", c[0], got, c[1])
		}
	}
}

func TestChildrenFollowTheInlineForms(t *testing.T) {
	want := `[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[4,5]},{"$":"list","items":[{"$":"sym","name":"c","span":[8,9]},{"$":"sym","name":"d","span":[10,11]}],"span":[8,11]}],"span":[0,11]},{"$":"sym","name":"e","span":[12,13]}]`
	if got := parsedJSON(t, "a\n  b\n  c d\ne"); got != want {
		t.Errorf("got %s", got)
	}
}

func TestADedentOfSeveralLevelsClosesEachBlock(t *testing.T) {
	want := `[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"list","items":[{"$":"sym","name":"b","span":[4,5]},{"$":"sym","name":"c","span":[10,11]}],"span":[4,11]}],"span":[0,11]},{"$":"sym","name":"d","span":[12,13]}]`
	if got := parsedJSON(t, "a\n  b\n    c\nd"); got != want {
		t.Errorf("got %s", got)
	}
}

func finerAt(t *testing.T, src string) (string, uint64, uint64) {
	t.Helper()
	_, f := Parse(src)
	if f == nil {
		t.Fatalf("%q parsed", src)
	}
	return f.FinerCode(), f.Row, f.Column
}

// A lone `\r` at the start of a line is whitespace there too: it restarts
// the indentation as it restarts the engine's column, a row holding
// nothing else after it is blank, and a block indented under such a row
// belongs to the line before it.
func TestALoneCarriageReturnAtALineStartIsWhitespace(t *testing.T) {
	for _, c := range [][2]string{
		{"f x\n\r", `[{"$":"list","items":[{"$":"sym","name":"f","span":[0,1]},{"$":"sym","name":"x","span":[2,3]}],"span":[0,3]}]`},
		{"a\n  b\n  \r;c\nd", `[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[4,5]}],"span":[0,5]},{"$":"sym","name":"d","span":[12,13]}]`},
		{"a\n  b\n\r;x\n  c", `[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[4,5]},{"$":"sym","name":"c","span":[12,13]}],"span":[0,13]}]`},
		{"a\n\r  b", `[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[5,6]}],"span":[0,6]}]`},
		{"a\n  \r  \rb", `[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[8,9]}]`},
		{"a\n;x\r  c\r;y\r  d", `[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"list","items":[{"$":"sym","name":"c","span":[7,8]},{"$":"sym","name":"d","span":[14,15]}],"span":[7,15]}],"span":[0,15]}]`},
	} {
		if got := parsedJSON(t, c[0]); got != c[1] {
			t.Errorf("%q:\n got %s\nwant %s", c[0], got, c[1])
		}
	}
	// The column a diagnostic names counts from the carriage return, as the
	// engine counts it: the indentation plus one; with several on the row,
	// from the last.
	for _, c := range []struct {
		src      string
		code     string
		row, col uint64
	}{
		{"\r  a", "bad_indent", 1, 3},
		{"a\n  b\n  \r   c", "bad_indent", 3, 4},
		{"a\n\r \tb", "tab_indent", 2, 2},
		{"\r \r   a", "bad_indent", 1, 4},
		{"a\n  \r \r   b", "bad_indent", 2, 4},
	} {
		code, row, col := finerAt(t, c.src)
		if code != c.code || row != c.row || col != c.col {
			t.Errorf("%q: %s@%d:%d, want %s@%d:%d", c.src, code, row, col, c.code, c.row, c.col)
		}
	}
}

func TestTheErrorsCarryTheGrammarCodes(t *testing.T) {
	for _, c := range [][2]string{
		{"a\n\tb", "tab_indent"},
		{"  a", "bad_indent"},
		{"a\n   b", "bad_indent"},
		{"a\n  b\n c", "bad_dedent"},
		{"a )", "unbalanced"},
		{"(a b", "unbalanced"},
		{"[a b", "unbalanced"},
		{"(a]", "unbalanced"},
		{"\"abc", "unterminated_string"},
		{"a { b", "unexpected"},
		{"\"\\q\"", "unexpected"},
		{strings.Repeat("(", MaxNesting), "too_deep"},
	} {
		if got := engineCode(t, c[0]); got != c[1] {
			t.Errorf("%q: %s, want %s", c[0], got, c[1])
		}
	}
}

// The bound is named in the message and the hint the document declares.
func TestTheTooDeepTextsNameTheBound(t *testing.T) {
	doc, err := documentValue()
	if err != nil {
		t.Fatal(err)
	}
	options := doc["options"].(map[string]any)
	bound := "256"
	if !strings.Contains(tooDeep, bound) || MaxNesting != 256 {
		t.Errorf("tooDeep %q, MaxNesting %d", tooDeep, MaxNesting)
	}
	if hint, _ := options["hint"].(map[string]any)["too_deep"].(string); !strings.Contains(hint, bound) {
		t.Errorf("hint %q", hint)
	}
	if msg, _ := options["error"].(map[string]any)["too_deep"].(string); msg != tooDeep {
		t.Errorf("the document's too_deep is %q, the code's %q", msg, tooDeep)
	}
}

// The desugaring codes are declared in the document so a fixture can pin
// them like the reader's, and raised in desugar.go with its own text; the
// two are one message.
func TestTheDesugaringMessagesMatchTheDocument(t *testing.T) {
	doc, err := documentValue()
	if err != nil {
		t.Fatal(err)
	}
	options := doc["options"].(map[string]any)
	errs := options["error"].(map[string]any)
	hints := options["hint"].(map[string]any)
	for _, m := range desugarMessages {
		if errs[m[0]] != m[1] {
			t.Errorf("options.error.%s is %v, desugar says %q", m[0], errs[m[0]], m[1])
		}
		if _, ok := hints[m[0]].(string); !ok {
			t.Errorf("options.hint.%s is not declared", m[0])
		}
	}
}

func TestTheFailCarriesTheCodeAndThePosition(t *testing.T) {
	_, f := Parse("a\n  b\n c")
	if f == nil || f.Code != CodeDSLParseError || !strings.HasPrefix(f.Message, "bad_dedent: ") {
		t.Fatalf("%v", f)
	}
	if f.Row != 3 || f.Column != 2 {
		t.Errorf("%d:%d", f.Row, f.Column)
	}
}

// Every @ reference the document names is one this package registers, and
// every one it registers is named.
func TestRefsTheDocumentNamesAreTheOnesRegistered(t *testing.T) {
	doc, err := documentValue()
	if err != nil {
		t.Fatal(err)
	}
	named := map[string]bool{}
	var walk func(any)
	walk = func(v any) {
		switch x := v.(type) {
		case string:
			if strings.HasPrefix(x, "@") {
				named[x] = true
			}
		case []any:
			for _, item := range x {
				walk(item)
			}
		case map[string]any:
			for _, item := range x {
				walk(item)
			}
		}
	}
	walk(doc)
	var a, b []string
	for name := range named {
		a = append(a, name)
	}
	b = append(b, registeredRefs...)
	sort.Strings(a)
	sort.Strings(b)
	if strings.Join(a, " ") != strings.Join(b, " ") {
		t.Errorf("the document names %v, the code registers %v", a, b)
	}
	if len(b) != 14 {
		t.Errorf("%d refs registered", len(b))
	}
	// And the installed grammar resolved every one: Make would have
	// failed otherwise.
	_ = Make()
}

// Whole numbers come back as integers after the read: `b` is an integer
// field.
func TestTheDocumentsWholeNumbersAreIntegers(t *testing.T) {
	if v := integralNumbers(float64(1)); v != int64(1) {
		t.Errorf("%T %v", v, v)
	}
	if v := integralNumbers(1.5); v != 1.5 {
		t.Errorf("%T %v", v, v)
	}
	data, err := document()
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), `"b":1,`) || strings.Contains(string(data), `"order":1e+05`) {
		t.Errorf("the document's numbers are not integers: %s", data)
	}
}

// Alchemy is a plugin like any other: installed on a bare engine, the
// instance reads alchemy.
func TestAlchemyInstallsOnABareEngine(t *testing.T) {
	j := tabnas.Make()
	if err := j.Use(Alchemy); err != nil {
		t.Fatal(err)
	}
	v, err := j.Parse("def x 1")
	if err != nil {
		t.Fatal(err)
	}
	if got := valueJSON(plain(v)); !strings.HasPrefix(got, `[{"$":"list"`) {
		t.Errorf("%s", got)
	}
}
