// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// ast_test.go: spans, the printers, the tagged-tree conversion and the
// desugarer (rs/src/ast.rs's and rs/src/desugar.rs's tests).

import (
	"strings"
	"testing"

	tabnas "github.com/tabnas/parser/go"
)

func mustParse(t *testing.T, src string) []*Expr {
	t.Helper()
	p, f := Parse(src)
	if f != nil {
		t.Fatalf("%q: %v", src, f)
	}
	return p
}

// A span is positioned in its own file's text, and a program of several
// sources names the file; one source, or a span of a file not among them,
// names none.
func TestSourcesPositionASpanInItsFileAndNameItWhenSeveral(t *testing.T) {
	a, b := NewFile("a.alc"), NewFile("b.alc")
	one := OneSource("a.alc", "x\ny")
	f := one.FailAt(NewFail(CodeInputInvalid, "m"), SourceSpan{a, 2, 3})
	if f.Row != 2 || f.Column != 1 || f.File != "" || f.Error() != "INPUT_INVALID: m (2:1)" {
		t.Errorf("%+v %s", f, f)
	}
	two := severalSources([]string{"a.alc", "b.alc"}, []string{"x\ny", "\n\nz"})
	f = two.FailAt(NewFail(CodeInputInvalid, "m"), SourceSpan{b, 2, 3})
	if f.Row != 3 || f.Column != 1 || f.File != "b.alc" || f.Error() != "INPUT_INVALID: m (b.alc:3:1)" {
		t.Errorf("%+v %s", f, f)
	}
	f = two.FailAt(NewFail(CodeInputInvalid, "m"), SourceSpan{a, 2, 3})
	if f.Row != 2 || f.Column != 1 || f.File != "a.alc" {
		t.Errorf("%+v", f)
	}
	f = two.FailAt(NewFail(CodeInputInvalid, "m"), SourceSpan{NewFile("stdlib/table.alc"), 2, 3})
	if f.Row != 2 || f.Column != 1 || f.File != "" {
		t.Errorf("%+v", f)
	}
}

func TestPositionsAreOneBasedRowsAndCharacterColumns(t *testing.T) {
	src := "ab\ncdé f\n"
	file := NewFile("t")
	for _, c := range []struct {
		src            string
		start, row, co int
	}{
		{src, 0, 1, 1},
		{src, 3, 2, 1},
		// `é` is two bytes and one column.
		{src, 8, 2, 5},
		{src, 99, 3, 1},
		// A lone carriage return restarts the column and not the row.
		{"a\r b", 3, 1, 2},
		{"a\r\nb", 3, 2, 1},
	} {
		row, col := SourceSpan{file, c.start, c.start + 1}.Position(c.src)
		if row != c.row || col != c.co {
			t.Errorf("%q@%d: %d:%d", c.src, c.start, row, col)
		}
	}
}

func TestSameShapeIgnoresSpansAndNothingElse(t *testing.T) {
	a := mustParse(t, "a (b 1) [\"x\"]")
	b := mustParse(t, "a\n  b 1\n  [\"x\"]")
	if !SameProgram(a, b) {
		t.Error("same shape")
	}
	if SameProgram(a, mustParse(t, "a (b 2) [\"x\"]")) {
		t.Error("different lexemes")
	}
	if Sym("a", SourceSpan{}).SameShape(&Expr{Kind: ExprKeyword, Text: "a"}) {
		t.Error("a symbol is not a keyword")
	}
}

func TestCanonicalPrintsEveryAtomKind(t *testing.T) {
	got := Canonical(mustParse(t, "s :k \"a\\\"b\\n\" -1.5e3 true false null [] ()"))
	if want := `(s :k "a\"b\n" -1.5e3 true false null [] ())`; got != want {
		t.Errorf("%s", got)
	}
	// Strings print as serde_json writes them: controls escaped in
	// lowercase hex, everything else as itself.
	if got := jsonString("\x01\x1f\x7f é \u2028 </>&"); got != "\"\\u0001\\u001f\x7f é \u2028 </>&\"" {
		t.Errorf("%q", got)
	}
}

func TestCanonicalAndFormatExamples(t *testing.T) {
	p := mustParse(t, "join \",\"\n  map csv-field values\n\n(newline)")
	if got := Canonical(p); got != "(join \",\" (map csv-field values))\n(newline)" {
		t.Errorf("%q", got)
	}
	p = mustParse(t, "(def csv-row [values] (concat (join \",\" (map csv-field values)) (newline)))")
	if got := Format(p); got != "def csv-row [values]\n  concat\n    join \",\"\n      map csv-field values\n    (newline)\n" {
		t.Errorf("%q", got)
	}
	p = mustParse(t, "def export [input]\n  csv input")
	if got := Canonical(p); got != "(def export [input] (csv input))" {
		t.Errorf("%q", got)
	}
}

func TestLayoutKeepsExplicitParensOnlyWhereLayoutCannotExpressTheShape(t *testing.T) {
	p := mustParse(t, "(concat prefix (newline) suffix)\n((f x) y)\n(a (b c) d)")
	if got := Format(p); got != "concat prefix\n  (newline)\n  suffix\n\n((f x) y)\n\na\n  b c\n  d\n" {
		t.Errorf("%q", got)
	}
}

// nestedTree is the tagged tree for x nested in depth lists, as a layered
// grammar could hand over without the reader's own bound.
func nestedTree(depth int) any {
	node := func(tag string, key string, value any) any {
		m := tabnas.NewOrderedMap()
		m.Set("$", tag)
		m.Set(key, value)
		m.Set("span", []any{0, 1})
		return m
	}
	tree := node("sym", "name", "x")
	for i := 0; i < depth; i++ {
		tree = node("list", "items", []any{tree})
	}
	return tree
}

func TestATaggedTreeAtTheBoundConvertsAndOnePastItIsTooDeep(t *testing.T) {
	file := NewFile("t")
	at, f := ExprFromValue(nestedTree(MaxNesting), file)
	if f != nil {
		t.Fatal(f)
	}
	if got := CanonicalForm(at); got != strings.Repeat("(", MaxNesting)+"x"+strings.Repeat(")", MaxNesting) {
		t.Errorf("%s", got)
	}
	_, f = ExprFromValue(nestedTree(MaxNesting+1), file)
	if f == nil || f.Code != CodeDSLParseError || !strings.HasPrefix(f.Message, "too_deep: ") {
		t.Errorf("%v", f)
	}
	// Far deeper is refused without recursing into it.
	if _, f = ExprFromValue(nestedTree(1_000), file); f == nil || !strings.HasPrefix(f.Message, "too_deep: ") {
		t.Errorf("%v", f)
	}
}

func TestMalformedReaderOutputIsAParseErrorNotAPanic(t *testing.T) {
	file := NewFile("t")
	if _, f := ExprFromValue("nope", file); f == nil || f.Code != CodeDSLParseError {
		t.Errorf("%v", f)
	}
	m := tabnas.NewOrderedMap()
	m.Set("$", "sym")
	m.Set("span", []any{0})
	if _, f := ExprFromValue(m, file); f == nil {
		t.Error("a one-number span converted")
	}
	if _, f := ProgramFromValue(m, file); f == nil {
		t.Error("a program that is not an array converted")
	}
}

// ---------------------------------------------------------------------------
// Desugaring
// ---------------------------------------------------------------------------

func core(t *testing.T, src string) string {
	t.Helper()
	c, f := Desugar(mustParse(t, src), src)
	if f != nil {
		t.Fatalf("%q: %v", src, f)
	}
	return Canonical(c)
}

func desugarFailure(t *testing.T, src string) *Fail {
	t.Helper()
	_, f := Desugar(mustParse(t, src), src)
	if f == nil {
		t.Fatalf("%q desugared", src)
	}
	return f
}

func TestADefWithParametersBecomesADefOfAFn(t *testing.T) {
	if got := core(t, "def csv-field [value]\n  scalar-text value"); got != "(def csv-field (fn [value] (scalar-text value)))" {
		t.Errorf("%s", got)
	}
	if got := core(t, "def sep \",\""); got != "(def sep \",\")" {
		t.Errorf("%s", got)
	}
}

func TestTheGeneratedFnCarriesTheDefSpan(t *testing.T) {
	src := "def f [x]\n  x"
	c, f := Desugar(mustParse(t, src), src)
	if f != nil {
		t.Fatal(f)
	}
	def := c[0]
	fn := def.Items[2]
	if fn.Span != def.Span || fn.Items[0].Span != def.Span {
		t.Errorf("%v %v %v", fn.Span, fn.Items[0].Span, def.Span)
	}
	// The parameters and the body keep their own.
	if s := fn.Items[1].Span; s.Start != 6 || s.End != 9 {
		t.Errorf("%v", s)
	}
	if s := fn.Items[2].Span; s.Start != 12 || s.End != 13 {
		t.Errorf("%v", s)
	}
}

func TestPipeThreadsTheValueDataLast(t *testing.T) {
	for _, c := range [][2]string{
		{"pipe input (select (path \"payload\" \"records\" each-index)) (map normalize) (filter active?)",
			"(filter active? (map normalize (select (path \"payload\" \"records\" each-index) input)))"},
		{"pipe x f g", "(g (f x))"},
		{"pipe x", "x"},
		{"(pipe)", "(pipe)"},
		{"def export [input]\n  pipe input\n    table-from-json api-binding\n    csv csv-options",
			"(def export (fn [input] (csv csv-options (table-from-json api-binding input))))"},
		{"fn [x] (get :label x)\n(case 1 2)\ndef x (pipe y f)", "(fn [x] (get :label x))\n(case 1 2)\n(def x (f y))"},
	} {
		if got := core(t, c[0]); got != c[1] {
			t.Errorf("%q:\n got %s\nwant %s", c[0], got, c[1])
		}
	}
}

func TestAStepThatIsNotApplicableIsAnEmptyStepError(t *testing.T) {
	for _, c := range []struct {
		src string
		col uint64
	}{
		{"pipe x ()", 8}, {"pipe x 1", 8}, {"pipe x \"s\"", 8}, {"pipe x [f]", 8}, {"pipe x :k", 8},
		{"pipe x\n  f\n  []", 3},
		// A lone carriage return restarts the column, as the engine counts
		// it.
		{"pipe x\r ()", 2},
	} {
		f := desugarFailure(t, c.src)
		if f.Code != CodeDSLParseError || !strings.HasPrefix(f.Message, "empty_step: ") || f.Column != c.col {
			t.Errorf("%q: %v", c.src, f)
		}
	}
	if f := desugarFailure(t, "pipe x\n  f\n  []"); f.Row != 3 {
		t.Errorf("%v", f)
	}
}

func TestTheCoreFormShapesAreChecked(t *testing.T) {
	for _, c := range [][2]string{
		{"let [x 1] x", "(let [x 1] x)"},
		{"if a b c", "(if a b c)"},
		{"match v\n  case 1 \"one\"\n  case _ \"other\"", "(match v (case 1 \"one\") (case _ \"other\"))"},
		{"match v", "(match v)"},
	} {
		if got := core(t, c[0]); got != c[1] {
			t.Errorf("%q: %s", c[0], got)
		}
	}
	for _, c := range [][2]string{
		{"let [x] x", "bad_let"}, {"let [x 1 2] x", "bad_let"}, {"let [1 x] x", "bad_let"}, {"let [x 1]", "bad_let"},
		{"if a b", "bad_if"}, {"if a b c d", "bad_if"},
		{"(match)", "bad_match"}, {"match v (case 1)", "bad_match"}, {"match v (when 1 2)", "bad_match"}, {"match v 1", "bad_match"},
		{"(def)", "bad_def"}, {"def x", "bad_def"}, {"def 1 2", "bad_def"}, {"def x y z", "bad_def"}, {"def x [a] b c", "bad_def"},
	} {
		f := desugarFailure(t, c[0])
		if !strings.HasPrefix(f.Message, c[1]+": ") || f.Row != 1 || f.Column != 1 {
			t.Errorf("%q: %v", c[0], f)
		}
	}
}

func TestARewriteThatNestsPastTheBoundIsTooDeepAtTheForm(t *testing.T) {
	// A pipe nests one level per step: 256 steps reach the bound.
	atLimit := "pipe x" + strings.Repeat(" f", MaxNesting)
	if got := core(t, atLimit); got != strings.Repeat("(f ", MaxNesting)+"x"+strings.Repeat(")", MaxNesting) {
		t.Errorf("%s", got[:40])
	}
	f := desugarFailure(t, "pipe x"+strings.Repeat(" f", MaxNesting+1))
	if !strings.HasPrefix(f.Message, "too_deep: ") || f.Row != 1 || f.Column != 1 {
		t.Errorf("%v", f)
	}
	if f := desugarFailure(t, "pipe x"+strings.Repeat(" f", 100_000)); !strings.HasPrefix(f.Message, "too_deep: ") {
		t.Errorf("%v", f)
	}
	// def adds the fn level: a body two below the bound still fits under
	// the def, a body one below it does not, though the reader accepted
	// both.
	body := func(depth int) string { return strings.Repeat("(", depth) + "x" + strings.Repeat(")", depth) }
	if got := core(t, "def f [x] "+body(MaxNesting-2)); !strings.HasPrefix(got, "(def f (fn [x] ") {
		t.Errorf("%s", got[:30])
	}
	if f := desugarFailure(t, "def f [x] "+body(MaxNesting-1)); !strings.HasPrefix(f.Message, "too_deep: ") {
		t.Errorf("%v", f)
	}
}
