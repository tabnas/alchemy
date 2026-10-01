// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// check_test.go: the resolver, the types and the checker
// (rs/src/resolve.rs's, rs/src/types.rs's and rs/src/check.rs's tests).

import (
	"fmt"
	"strings"
	"testing"
)

// ---------------------------------------------------------------------------
// Resolution
// ---------------------------------------------------------------------------

func testOuter(name string) NameKind {
	switch name {
	case "table-end", "no-schema", "missing":
		return NameConstant
	case "selected", "schema", "row":
		return NameConstructor
	case "map", "get", "csv", "table-from-json":
		return NameFunction
	case "csv-options":
		return NameValue
	}
	return NameNone
}

func resolved(t *testing.T, src string) (*Resolved, *Fail) {
	t.Helper()
	forms, f := Desugar(mustParse(t, src), src)
	if f != nil {
		t.Fatalf("%q: %v", src, f)
	}
	return Resolve(forms, OneSource("t", src), testOuter)
}

type coded struct {
	code     Code
	finer    string
	row, col uint64
}

func resolveCode(t *testing.T, src string) coded {
	t.Helper()
	_, f := resolved(t, src)
	if f == nil {
		t.Fatalf("%q resolved", src)
	}
	return coded{f.Code, f.FinerCode(), f.Row, f.Column}
}

func TestDefinitionsSeeEachOtherInAnyOrder(t *testing.T) {
	r, f := resolved(t, "def export [input]\n  csv csv-options (api-table input)\ndef api-table [input]\n  table-from-json binding input\ndef binding 1")
	if f != nil {
		t.Fatal(f)
	}
	if len(r.Defs) != 3 {
		t.Errorf("%d defs", len(r.Defs))
	}
	if fmt.Sprint(r.References["export"]) != "[api-table]" || fmt.Sprint(r.References["api-table"]) != "[binding]" {
		t.Errorf("%v", r.References)
	}
	if fmt.Sprint(r.Reachable("export")) != "[api-table binding]" {
		t.Errorf("%v", r.Reachable("export"))
	}
	if _, ok := r.Get("binding").Params(); ok {
		t.Error("binding has params")
	}
	if p, ok := r.Get("export").Params(); !ok || fmt.Sprint(p) != "[input]" {
		t.Errorf("%v", p)
	}
	if fmt.Sprint(r.DependencyOrder()) != "[binding api-table export]" {
		t.Errorf("%v", r.DependencyOrder())
	}
}

func TestLocalsShadowAndPatternsBind(t *testing.T) {
	if _, f := resolved(t, "def f [x]\n  let [y (map x x)]\n    match y\n      case (selected :columns raw) raw\n      case [a b] (get a b)\n      case table-end y\n      case _ x"); f != nil {
		t.Fatal(f)
	}
	// raw is only bound inside its clause.
	c := resolveCode(t, "def f [x]\n  match x\n    case (selected :a raw) raw\n    case _ raw")
	if c != (coded{CodeDSLTypeError, "unknown_name", 4, 12}) {
		t.Errorf("%+v", c)
	}
}

func TestTheResolverFinerCodes(t *testing.T) {
	for _, c := range [][2]string{
		{"def f [x] (nope x)", "unknown_name"},
		{"csv 1", "not_def"},
		{"def x 1\ndef x 2", "duplicate_def"},
		{"def if 1", "reserved"},
		{"def f (fn x x)", "bad_fn"},
		{"def f (fn [1] x)", "bad_fn"},
		{"def f [x] (def y x)", "misplaced_def"},
		{"def f [x] (match x (case (map a) a))", "bad_pattern"},
		{"def f [x] (match x (case (1 a) a))", "bad_pattern"},
		{"def a [x] (a x)", "recursion"},
		{"def a (fn [x] (b x))\ndef b [y] (a y)", "recursion"},
	} {
		if got := resolveCode(t, c[0]).finer; got != c[1] {
			t.Errorf("%q: %s, want %s", c[0], got, c[1])
		}
	}
	c := resolveCode(t, "def a [x] (b x)\ndef b [x] (c x)\ndef c [x] (a x)")
	if c.code != CodeStreamabilityUnknown || c.finer != "recursion" || c.row != 1 {
		t.Errorf("%+v", c)
	}
	_, f := resolved(t, "def a [x] (b x)\ndef b [x] (c x)\ndef c [x] (a x)")
	if !strings.Contains(f.Message, "a reaches itself through b, then c") {
		t.Errorf("%s", f.Message)
	}
}

func TestAProgramMayShadowALibraryName(t *testing.T) {
	r, f := resolved(t, "def csv [x] x\ndef export [input] (csv input)")
	if f != nil {
		t.Fatal(f)
	}
	if fmt.Sprint(r.References["export"]) != "[csv]" {
		t.Errorf("%v", r.References)
	}
}

// Across several sources a name defined twice names where it was first.
func TestADuplicateAcrossSourcesNamesTheFirst(t *testing.T) {
	_, f := AnalyzeSources([]Source{
		{File: "a.alc", Text: "def x 1\n"},
		{File: "b.alc", Text: "\ndef x 2\ndef export [input] input"},
	})
	if f == nil || f.FinerCode() != "duplicate_def" || f.File != "b.alc" || f.Row != 2 ||
		!strings.Contains(f.Message, "first at a.alc:1:1") {
		t.Errorf("%v", f)
	}
	_, f = AnalyzeSources([]Source{{File: "a.alc", Text: "x"}, {File: "a.alc", Text: "y"}})
	if f == nil || f.FinerCode() != "duplicate_file" {
		t.Errorf("%v", f)
	}
}

// A source linked under another name defines its export under it, and the
// program's export may call it.
func TestASourceLinkedUnderAnotherName(t *testing.T) {
	a, f := AnalyzeSources([]Source{
		{File: "program.alc", Text: "def export [input] (json input)", ExportAs: "program-export"},
		{File: "render.alc", Text: "def export [input] (program-export input)"},
	})
	if f != nil {
		t.Fatal(f)
	}
	if a.Resolved.Get("program-export") == nil || a.Checked.Output != OutputText {
		t.Errorf("%+v", a.Checked)
	}
	_, f = AnalyzeSources([]Source{
		{File: "lib.alc", Text: "def x 1", ExportAs: "program-export"},
		{File: "render.alc", Text: "def export [input] input"},
	})
	if f == nil || f.FinerCode() != "no_export" || f.File != "lib.alc" {
		t.Errorf("%v", f)
	}
	_, f = AnalyzeSources([]Source{
		{File: "p.alc", Text: "def export [program-export] program-export", ExportAs: "program-export"},
		{File: "render.alc", Text: "def export [input] input"},
	})
	if f == nil || f.FinerCode() != "duplicate_def" || f.Row != 1 || f.Column != 13 {
		t.Errorf("%v", f)
	}
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

func TestUnknownAndNeverAreAcceptedEverywhere(t *testing.T) {
	for _, ty := range []Type{TextT, JsonEvents, RecordT, TableEvents()} {
		if !ty.Accepts(Unknown) || !ty.Accepts(Never) || !Unknown.Accepts(ty) {
			t.Errorf("%s", ty)
		}
	}
}

func TestValueAcceptsDataAndNothingElse(t *testing.T) {
	for _, ty := range []Type{NumberT, VectorOf(StringT), Tagged("missing")} {
		if !ValueT.Accepts(ty) {
			t.Errorf("Value refuses %s", ty)
		}
	}
	for _, ty := range []Type{SelectorT, TextT, FuncOf(1)} {
		if ValueT.Accepts(ty) {
			t.Errorf("Value accepts %s", ty)
		}
	}
}

func TestAValuePassesWhereParticularDataIsWanted(t *testing.T) {
	for _, ty := range []Type{RecordT, NumberT, StringT, NullT, VectorOf(ValueT), VectorOf(RecordT), Tagged("missing")} {
		if !ty.Accepts(ValueT) {
			t.Errorf("%s refuses Value", ty)
		}
	}
	if !VectorOf(RecordT).Accepts(VectorOf(ValueT)) {
		t.Error("Vector<Record> refuses Vector<Value>")
	}
	for _, ty := range []Type{TextT, SelectorT, KeywordT, TableEventT, JsonEvents, StreamOf(ValueT), FuncOf(1), Tagged("row")} {
		if ty.Accepts(ValueT) {
			t.Errorf("%s accepts Value", ty)
		}
	}
	if RecordT.Accepts(NumberT) || VectorOf(ValueT).Accepts(RecordT) || RecordT.Accepts(VectorOf(ValueT)) {
		t.Error("a type that can never be the data wanted passes")
	}
}

func TestStreamsAndTableEvents(t *testing.T) {
	table, events := TableEvents(), Events()
	checks := []struct {
		got  bool
		what string
	}{
		{table.Accepts(StreamOf(Unknown)), "table accepts Stream<Unknown>"},
		{table.Accepts(StreamOf(Tagged("row"))), "table accepts Stream<row>"},
		{!table.Accepts(StreamOf(Tagged("selected"))), "table refuses Stream<selected>"},
		{!table.Accepts(JsonEvents), "table refuses JsonEvents"},
		{!JsonEvents.Accepts(table), "JsonEvents refuses table"},
		{events.Accepts(StreamOf(Tagged("key"))), "events accepts key"},
		{events.Accepts(StreamOf(Tagged("object-end"))), "events accepts object-end"},
		{!events.Accepts(StreamOf(Tagged("row"))), "events refuses row"},
		{!table.Accepts(events) && !events.Accepts(table), "table and events are apart"},
		{!events.Accepts(JsonEvents), "events refuses JsonEvents"},
		{JsonEvents.Accepts(events), "JsonEvents accepts events"},
		{JsonEvents.Accepts(StreamOf(Unknown)), "JsonEvents accepts Stream<Unknown>"},
		{!JsonEvents.Accepts(StreamOf(ValueT)), "JsonEvents refuses Stream<Value>"},
		{events.IsAffine() && events.IsProtocol(), "events is affine"},
		{table.IsAffine() && TextT.IsAffine() && !StringT.IsAffine(), "affinity"},
	}
	for _, c := range checks {
		if !c.got {
			t.Error(c.what)
		}
	}
	for _, c := range [][2]string{
		{events.String(), "Stream<Event>"},
		{table.String(), "TableEvents"},
		{StreamOf(ValueT).String(), "Stream<Value>"},
		{Func([]Type{RecordT, JsonEvents}, table).String(), "Fn(Record JsonEvents -> TableEvents)"},
	} {
		if c[0] != c[1] {
			t.Errorf("%s, want %s", c[0], c[1])
		}
	}
}

func TestFunctionsByArityAndResult(t *testing.T) {
	f := Func([]Type{Unknown, Unknown}, TextT)
	if !f.Accepts(Func([]Type{RecordT, ValueT}, TextT)) || f.Accepts(FuncOf(1)) || !FuncOf(2).Accepts(f) {
		t.Error("function acceptance")
	}
}

func TestJoins(t *testing.T) {
	for _, c := range []struct{ a, b, want Type }{
		{Never, TextT, TextT},
		{TextT, StringT, TextT},
		{NumberT, StringT, ValueT},
		{TextT, RecordT, Unknown},
		{VectorOf(NumberT), VectorOf(NullT), VectorOf(ValueT)},
	} {
		if got := JoinTypes(c.a, c.b); !got.Equal(c.want) {
			t.Errorf("%s ∨ %s = %s", c.a, c.b, got)
		}
	}
}

// ---------------------------------------------------------------------------
// The checker
// ---------------------------------------------------------------------------

func checked(t *testing.T, src string) (*Checked, *Fail) {
	t.Helper()
	forms, f := ParseFile(src, "t.alc")
	if f != nil {
		t.Fatalf("%q: %v", src, f)
	}
	if forms, f = Desugar(forms, src); f != nil {
		t.Fatalf("%q: %v", src, f)
	}
	sources := OneSource("t.alc", src)
	r, f := Resolve(forms, sources, Outer)
	if f != nil {
		return nil, f
	}
	return CheckProgram(r, sources)
}

func mustCheck(t *testing.T, src string) *Checked {
	t.Helper()
	c, f := checked(t, src)
	if f != nil {
		t.Fatalf("%q: %v", src, f)
	}
	return c
}

func checkCode(t *testing.T, src string) coded {
	t.Helper()
	_, f := checked(t, src)
	if f == nil {
		t.Fatalf("%q checked", src)
	}
	return coded{f.Code, f.FinerCode(), f.Row, f.Column}
}

const binding = "def column-from-meta [source]\n  record\n    entry :label (get \"title\" source)\n    entry :source (as-path (get \"path\" source))\ndef api-binding\n  record\n    entry :columns (path \"response\" \"metadata\" \"fields\")\n    entry :rows (path \"response\" \"payload\" \"deep\" \"records\" each-index)\n    entry :column column-from-meta\n"

func TestTheStandardLibraryChecksCleanAgainstItsSignatures(t *testing.T) {
	lib := StdlibLoaded()
	for i, r := range lib.Files {
		src, _ := StdlibSource(StdlibFiles[i])
		if f := checkStdlibFile(r, src); f != nil {
			t.Error(f)
		}
	}
	for _, name := range lib.Names() {
		if !hasStdlibSignature(name) {
			t.Errorf("%s has no signature", name)
		}
	}
}

func TestTheWorkedExampleIsATextOverTableEvents(t *testing.T) {
	c := mustCheck(t, binding+"def api-table [input]\n  table-from-json api-binding input\ndef export [input]\n  pipe input\n    api-table\n    csv csv-options\n")
	if c.Output != OutputText || !c.Export.Equal(TextT) || !c.Defs["api-binding"].Equal(RecordT) ||
		!c.Defs["api-table"].Equal(Func([]Type{Unknown}, TableEvents())) ||
		!c.Defs["column-from-meta"].Equal(Func([]Type{Unknown}, RecordT)) {
		t.Errorf("%+v", c)
	}
}

func TestOutputs(t *testing.T) {
	for _, c := range []struct {
		src  string
		want Output
	}{
		{"def export [input] input", OutputJsonEvents},
		{"def export [input] (json input)", OutputText},
		{"def export [input] \"x\"", OutputText},
		{binding + "def export [input] (table-from-json api-binding input)", OutputTableRows},
		{binding + "def export [input] (records (table-from-json api-binding input))", OutputJsonEvents},
		// A user's own scan-emit is a stream of unknown items: rendered as
		// a table, checked at run time.
		{"def step [s x] (transition s [(row [x])])\ndef fin [s] [table-end]\ndef export [input] (scan-emit null step fin (select (path each-index) input))", OutputTableRows},
	} {
		if got := mustCheck(t, c.src).Output; got != c.want {
			t.Errorf("%q: %s", c.src, got)
		}
	}
	for _, c := range [][2]string{
		{"def export [input] (select (path each-index) input)", "bad_output"},
		{"def export [input] 1", "bad_output"},
		{"def export [input] csv-options", "bad_output"},
		{"def x 1", "no_export"},
		{"def export 1", "type_mismatch"},
		{"def export [a b] a", "arity"},
	} {
		if got := checkCode(t, c[0]).finer; got != c[1] {
			t.Errorf("%q: %s, want %s", c[0], got, c[1])
		}
	}
	if c := checkCode(t, "def export [input] (get :x csv-options)"); c.code != CodeStreamabilityUnknown || c.finer != "unknown_output" {
		t.Errorf("%+v", c)
	}
}

func TestArityTypeAndProtocolMismatches(t *testing.T) {
	for _, c := range [][2]string{
		{"def export [input] (json input 1)", "arity"},
		{"def export [input] (csv csv-options)", "arity"},
		{"def f [a b] a\ndef export [input] (json (f input))", "arity"},
		{"def export [input] (text 1 (json input))", "arity"},
		{"def export [input] (if 1 (json input) (json input))", "type_mismatch"},
		{"def export [input] (csv csv-options input)", "protocol_mismatch"},
		{"def export [input] (table-from-json csv-options (json input))", "protocol_mismatch"},
		{"def export [input] (json (select (path each-index) input))", "protocol_mismatch"},
		{"def export [input] (map (fn [x] x) input)", "protocol_mismatch"},
		{"def export [input] (records input)", "protocol_mismatch"},
		{"def export [input] (json [input])", "type_mismatch"},
		{"def export [input] (json (vector input))", "type_mismatch"},
		{"def export [input] (concat (record (entry :x input)))", "type_mismatch"},
		{"def export [input] (concat 1 (json input))", "type_mismatch"},
		{"def export [input] (1 input)", "type_mismatch"},
		{"def export [input] (csv-options input)", "type_mismatch"},
		{"def export [input] (concat-map (fn [x] 1) (select (path each-index) input))", "type_mismatch"},
		{"def export [input] (join 1 (select (path each-index) input))", "type_mismatch"},
		{"def export [input] (match input (case (selected :a) \"x\"))", "arity"},
	} {
		if got := checkCode(t, c[0]).finer; got != c[1] {
			t.Errorf("%q: %s, want %s", c[0], got, c[1])
		}
	}
	if c := checkCode(t, "def export [input] (json (text 1))"); c != (coded{CodeDSLTypeError, "type_mismatch", 1, 32}) {
		t.Errorf("%+v", c)
	}
}

func TestStreamsAreAffine(t *testing.T) {
	if c := checkCode(t, "def export [input] (concat (json input) (json input))"); c != (coded{CodeStreamReused, "reused", 1, 47}) {
		t.Errorf("%+v", c)
	}
	if c := checkCode(t, "def export [input]\n  concat-map (fn [x] (json input)) (select (path each-index) input)"); c.code != CodeStreamReused || c.finer != "captured" {
		t.Errorf("%+v", c)
	}
	if c := checkCode(t, "def export [input]\n  let [s (select (path each-index) input)]\n    concat (join \",\" s) (join \";\" s)"); c.code != CodeStreamReused || c.finer != "reused" {
		t.Errorf("%+v", c)
	}
	for _, src := range []string{
		// Alternatives are not two uses.
		"def export [input] (if true (json input) (json input))",
		"def export [input]\n  match 1\n    case 1 (json input)\n    case _ (json input)",
		// A let that consumes it once, then uses the binding once.
		"def export [input]\n  let [s (select (path each-index) input)]\n    join \",\" s",
		// Shadowing ends the scope.
		"def export [input] (concat (json input) (concat-map (fn [input] input) [\"a\"]))",
	} {
		mustCheck(t, src)
	}
}

// colOf is the column of the nth (from 1) occurrence of needle in the
// line of src holding it, 1-based.
func colOf(src, needle string, n int) uint64 {
	for _, line := range strings.Split(src, "\n") {
		if !strings.Contains(line, needle) {
			continue
		}
		at := -1
		for i := 0; i < n; i++ {
			next := strings.Index(line[at+1:], needle)
			at += next + 1
		}
		return uint64(len([]rune(line[:at])) + 1)
	}
	return 0
}

func TestAStreamPassedToADefinitionIsAffineInItsBody(t *testing.T) {
	src := "def g [s] (concat-map (fn [x] (json s)) [1])\ndef export [input] (g input)"
	if c := checkCode(t, src); c != (coded{CodeStreamReused, "captured", 1, colOf(src, "s)", 1)}) {
		t.Errorf("%+v", c)
	}
	if c := checkCode(t, "def g [s] (join \",\" (map (fn [x] (json s)) [1 2]))\ndef export [input] (g input)"); c.finer != "captured" {
		t.Errorf("%+v", c)
	}
	src = "def mk [s] (fn [] s)\ndef export [input]\n  let [g (mk input)]\n    concat (json (g)) \"x\""
	if c := checkCode(t, src); c != (coded{CodeStreamReused, "captured", 1, colOf(src, "s)", 1)}) {
		t.Errorf("%+v", c)
	}
	src = "def twice [s] (concat (json s) (json s))\ndef export [input] (twice input)"
	if c := checkCode(t, src); c != (coded{CodeStreamReused, "reused", 1, colOf(src, "s)", 2)}) {
		t.Errorf("%+v", c)
	}
	for _, c := range [][2]string{
		{"def mk [s] (partial json s)\ndef export [input]\n  let [g (mk input)]\n    concat (g) (g)", "type_mismatch"},
		{"def mk [s] (record (entry :s s))\ndef export [input]\n  let [r (mk input)]\n    concat (json (get :s r)) (json (get :s r))", "type_mismatch"},
		{"def twice [t] (concat t t)\ndef export [input] (twice (json input))", "reused"},
		{"def export [input] ((fn [s] (concat (json s) (json s))) input)", "reused"},
	} {
		if got := checkCode(t, c[0]).finer; got != c[1] {
			t.Errorf("%q: %s, want %s", c[0], got, c[1])
		}
	}
	if c := checkCode(t, "def h [t] (concat-map (fn [x] (json t)) [1])\ndef g [s] (h s)\ndef export [input] (g input)"); c.code != CodeStreamReused || c.finer != "captured" || c.row != 1 {
		t.Errorf("%+v", c)
	}
	if got := mustCheck(t, "def g [s] s\ndef export [input] (g input)").Output; got != OutputJsonEvents {
		t.Errorf("%s", got)
	}
	if got := mustCheck(t, "def g [s] (json s)\ndef export [input] (g input)").Export; !got.Equal(TextT) {
		t.Errorf("%s", got)
	}
	mustCheck(t, "def g [s] (if true (json s) (json s))\ndef export [input] (g input)")
}

func TestAVectorMayHoldAFiniteTextAndNeverAStream(t *testing.T) {
	for _, src := range []string{
		"def export [input] (concat (join \",\" [(text \"i\")]) (json input))",
		"def export [input] (concat (join \",\" (vector (text \"i\") \"j\")) (json input))",
		"def export [input] (concat (join \",\" (map text [\"a\" \"b\"])) (json input))",
		// A live text in a vector passes the checker; building the plan
		// refuses it, which is the interpreter's (check.tsv pins it).
		"def export [input] (join \",\" [(json input)])",
	} {
		mustCheck(t, src)
	}
	for _, src := range []string{
		"def export [input] (json [input])",
		"def export [input] (join \",\" [(select (path each-index) input)])",
		"def export [input] (join \",\" (vector (select (path each-index) input)))",
	} {
		if got := checkCode(t, src).finer; got != "type_mismatch" {
			t.Errorf("%q: %s", src, got)
		}
	}
}

func TestAChainOfDefinitionsIsTypedWithoutNestingTheChecker(t *testing.T) {
	var b strings.Builder
	b.WriteString("def a0 1\n")
	for i := 1; i < 10_000; i++ {
		fmt.Fprintf(&b, "def a%d a%d\n", i, i-1)
	}
	b.WriteString("def export [input] (if false (let [y a9999] (json input)) (json input))\n")
	if c := mustCheck(t, b.String()); !c.Defs["a9999"].Equal(NumberT) {
		t.Errorf("%s", c.Defs["a9999"])
	}
	// A stream passed down five hundred definitions is followed to
	// MaxApplied of them and typed from there on as the body inferred.
	b.Reset()
	b.WriteString("def f0 [s] (json s)\n")
	for i := 1; i < 500; i++ {
		fmt.Fprintf(&b, "def f%d [s] (f%d s)\n", i, i-1)
	}
	b.WriteString("def export [input] (f499 input)\n")
	mustCheck(t, b.String())
}

func TestStrictModeWantsStaticFunctionsOverStreams(t *testing.T) {
	if c := checkCode(t, "def go [f input] (concat-map f (select (path each-index) input))\ndef export [input] (go text input)"); c.code != CodeStreamabilityUnknown || c.finer != "dynamic" {
		t.Errorf("%+v", c)
	}
	if c := checkCode(t, "def export [input] (scan-emit null (get :f csv-options) (fn [s] []) (select (path each-index) input))"); c.finer != "dynamic" {
		t.Errorf("%+v", c)
	}
	for _, src := range []string{
		"def step [b s x] (transition s [x])\ndef fin [s] []\ndef export [input] (join \",\" (scan-emit null (partial step 1) fin (select (path each-index) input)))",
		"def export [input] (concat-map text (select (path each-index) input))",
		"def export [input] (concat-map (get :f csv-options) [\"a\"])",
	} {
		mustCheck(t, src)
	}
	if c := checkCode(t, "def step [s x] [x]\ndef fin [s] []\ndef export [input] (join \",\" (scan-emit null step fin (select (path each-index) input)))"); c.finer != "type_mismatch" {
		t.Errorf("%+v", c)
	}
	if c := checkCode(t, "def step [s] s\ndef fin [s] []\ndef export [input] (join \",\" (scan-emit null step fin (select (path each-index) input)))"); c.finer != "arity" {
		t.Errorf("%+v", c)
	}
}

func TestADefinitionOverItemsIsCheckedAsItsEtaExpansion(t *testing.T) {
	if c := checkCode(t, "def bad (map public-column [1])\ndef export [input] (json input)"); c != (coded{CodeDSLTypeError, "type_mismatch", 1, 28}) {
		t.Errorf("%+v", c)
	}
	_, f := checked(t, "def bad (filter public-column [1])\ndef export [input] (json input)")
	if f == nil || !strings.HasPrefix(f.Message, "type_mismatch: an item given to the function of filter must be Record, not Number") || f.Row != 1 || f.Column != 31 {
		t.Errorf("%v", f)
	}
	for _, src := range []string{
		"def export [input]\n  concat-map (partial csv-row csv-options) (map (fn [v] 1) (select (path each-index) input))",
		"def export [input]\n  concat-map (fn [r] (scalar-text csv-options (get :label r))) (map public-column (map (fn [v] \"s\") (select (path each-index) input)))",
		"def bad (map (fn [x] (get :a x)) [1])\ndef export [input] (json input)",
	} {
		if got := checkCode(t, src).finer; got != "type_mismatch" {
			t.Errorf("%q: %s", src, got)
		}
	}
	for _, f := range []string{"public-column", "(fn [x] (public-column x))"} {
		if got := checkCode(t, "def bad (map "+f+" [1])\ndef export [input] (json input)").finer; got != "type_mismatch" {
			t.Errorf("%s: %s", f, got)
		}
		mustCheck(t, "def ok (map "+f+" [(record (entry :label \"x\"))])\ndef export [input] (json input)")
		mustCheck(t, "def ok [xs] (map "+f+" xs)\ndef export [input] (json input)")
		mustCheck(t, "def cols [input] (map "+f+" (select (path \"cols\" each-index) input))\ndef export [input] (join \",\" (map (fn [c] (get :label c)) (cols input)))")
	}
	for _, f := range []string{"(partial csv-row csv-options)", "(fn [cells] (csv-row csv-options cells))"} {
		mustCheck(t, "def export [input]\n  concat-map "+f+" (select (path \"rows\" each-index) input)")
	}
	mustCheck(t, "def ok (map (partial get :a) [1])\ndef export [input] (json input)")
}

func TestEventsAndTheStackOperators(t *testing.T) {
	render := "def export [input]\n  join \"\"\n    map\n      fn [e]\n        match e\n          case (key n) (quoted n)\n          case (scalar v) (scalar-text csv-options v)\n          case object-start \"{\"\n          case _ \"\"\n      events input"
	if got := mustCheck(t, render).Export; !got.Equal(TextT) {
		t.Errorf("%s", got)
	}
	keep := mustCheck(t, "def keep [s e] (transition (push e s) [])\ndef fin [s] [(repeat (count s) \"  \") (quoted (top s))]\ndef export [input] (join \"\" (scan-emit [] keep fin (events input)))")
	if keep.Output != OutputText || !keep.Defs["fin"].Equal(Func([]Type{Unknown}, VectorOf(StringT))) {
		t.Errorf("%+v", keep)
	}
	defs := mustCheck(t, "def s (push :b [:a])\ndef n (count s)\ndef t (top s)\ndef p (pop s)\ndef export [input] (json input)")
	if fmt.Sprint(defs.Order) != "[s n t p]" || !defs.Defs["s"].Equal(VectorOf(KeywordT)) || !defs.Defs["n"].Equal(NumberT) ||
		!defs.Defs["t"].Equal(KeywordT) || !defs.Defs["p"].Equal(VectorOf(KeywordT)) {
		t.Errorf("%+v", defs)
	}
	if got := mustCheck(t, "def export [input] (json (events input))").Export; !got.Equal(TextT) {
		t.Errorf("%s", got)
	}
	for _, c := range [][2]string{
		{"def export [input] (events input)", "bad_output"},
		{"def export [input] (json (select (path each-index) input))", "protocol_mismatch"},
		{"def export [input] (csv csv-options (events input))", "protocol_mismatch"},
		{"def export [input] (events (select (path each-index) input))", "protocol_mismatch"},
		{"def export [input] (concat (json input) (join \"\" (map (fn [e] \"\") (events input))))", "reused"},
		{"def export [input] (let [x (push (events input) [])] \"done\")", "type_mismatch"},
		{"def export [input] (let [x (count 1)] (json input))", "type_mismatch"},
		{"def export [input] (let [x (top csv-options)] (json input))", "type_mismatch"},
		{"def export [input] (let [x (quoted 1)] (json input))", "type_mismatch"},
		{"def export [input] (let [x (repeat \"a\" 1)] (json input))", "type_mismatch"},
		{"def export [input] (let [x (key :a)] (json input))", "type_mismatch"},
		{"def export [input] (let [x (scalar csv-options)] (json input))", "type_mismatch"},
		{"def export [input] (join \"\" (map (fn [e] (match e (case (key a b) a) (case _ \"\"))) (events input)))", "arity"},
	} {
		if got := checkCode(t, c[0]).finer; got != c[1] {
			t.Errorf("%q: %s, want %s", c[0], got, c[1])
		}
	}
}

func TestPatternsTypeTheirBindings(t *testing.T) {
	mustCheck(t, "def export [input]\n  concat-map\n    fn [e]\n      match e\n        case (row cells) (join \",\" (map (fn [c] (scalar-text csv-options c)) cells))\n        case _ \"\"\n    select (path each-index) input")
	mustCheck(t, "def export [input]\n  concat-map\n    fn [e]\n      match e\n        case table-end \"end\"\n        case other (scalar-text csv-options other)\n    select (path each-index) input")
}
