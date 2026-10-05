// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// interp_test.go: the evaluator (rs/src/interp.rs's tests).

import (
	"fmt"
	"strings"
	"testing"

	tt "github.com/tabnas/transduce/go"
)

// runtimeOf is a runtime over src, named t.alc.
func runtimeOf(t testing.TB, src string) *Runtime {
	t.Helper()
	return runtimeNamed(t, src, "t.alc")
}

func runtimeNamed(t testing.TB, src, file string) *Runtime {
	t.Helper()
	forms, f := ParseFile(src, file)
	if f == nil {
		forms, f = Desugar(forms, src)
	}
	if f != nil {
		t.Fatalf("%q: %v", src, f)
	}
	sources := OneSource(file, src)
	resolved, f := Resolve(forms, sources, Outer)
	if f != nil {
		t.Fatalf("%q: %v", src, f)
	}
	return NewRuntime(resolved, sources).WithRouters(routers).WithRenderers(renderers)
}

// evalExpr evaluates the expression expr in the program src's scope.
func evalExpr(t testing.TB, src, expr string) (Val, *Fail) {
	t.Helper()
	return evalWith(t, runtimeOf(t, src), expr)
}

func evalWith(t testing.TB, rt *Runtime, expr string) (Val, *Fail) {
	t.Helper()
	forms, f := ParseFile(expr, "e.alc")
	if f == nil {
		forms, f = Desugar(forms, expr)
	}
	if f != nil {
		t.Fatalf("%q: %v", expr, f)
	}
	return rt.EvalProgramExpr(forms[0])
}

func mustEval(t testing.TB, src, expr string) Val {
	t.Helper()
	v, f := evalExpr(t, src, expr)
	if f != nil {
		t.Fatalf("%q: %v", expr, f)
	}
	return v
}

func mustFail(t testing.TB, src, expr string) *Fail {
	t.Helper()
	v, f := evalExpr(t, src, expr)
	if f == nil {
		t.Fatalf("%q answered %s", expr, DebugString(v))
	}
	return f
}

func TestValuesRecordsAndLookups(t *testing.T) {
	v := mustEval(t, "def opts\n  record\n    entry :delimiter \",\"\n    entry :header true", "get :delimiter opts")
	if !Equal(v, StrVal(",")) {
		t.Errorf("%s", DebugString(v))
	}
	if v := mustEval(t, "", "get :x (record)"); !IsMissing(v) {
		t.Errorf("%s", DebugString(v))
	}
	v = mustEval(t, "", `get-path (as-path ["a" 0]) (record (entry :a [7]))`)
	if n, ok := v.(NumVal); !ok || n.Value != 7 || n.Lexeme != "7" || !n.HasLexeme {
		t.Errorf("%#v", v)
	}
	v = mustEval(t, "", "map (fn [x] (get :a x)) [(record (entry :a 1)) (record (entry :a 2))]")
	if !Equal(v, Vector(Num(1), Num(2))) {
		t.Errorf("%s", DebugString(v))
	}
}

func TestLetIfAndMatchChoose(t *testing.T) {
	if v := mustEval(t, "", `let [x true] (if x "yes" "no")`); !Equal(v, StrVal("yes")) {
		t.Errorf("%s", DebugString(v))
	}
	src := "def tag [v]\n  match v\n    case (selected :columns raw) raw\n    case table-end \"end\"\n    case [a b] b\n    case 1 \"one\"\n    case _ \"other\""
	for _, c := range []struct {
		expr string
		want Val
	}{
		{"tag (selected :columns 5)", Num(5)},
		{"tag table-end", StrVal("end")},
		{"tag [1 2]", Num(2)},
		{"tag 1", StrVal("one")},
		{"tag no-schema", StrVal("other")},
	} {
		if v := mustEval(t, src, c.expr); !Equal(v, c.want) {
			t.Errorf("%s: %s", c.expr, DebugString(v))
		}
	}
	if f := mustFail(t, "", "match 3 (case 1 1)"); !strings.HasPrefix(f.Message, "no_match: ") {
		t.Errorf("%v", f)
	}
	// A value no case takes is named by its kind and a short prefix, so a
	// failure over a large document does not carry the document.
	big := "match [" + strings.Repeat("\"abcdefgh\" ", 10_000) + "] (case 1 1)"
	f := mustFail(t, "", big)
	if !strings.HasPrefix(f.Message, "no_match: no case matches a vector (") || len(f.Message) >= 200 {
		t.Errorf("%d: %s", len(f.Message), f.Message)
	}
}

// What a render needs beyond the stack: length and compare see a key past
// a length, and number-class names a number's class.
func TestTheRenderOperators(t *testing.T) {
	for _, c := range []struct{ expr, message string }{
		{"length 1", "length: the string must be a string"},
		{"compare 1 \"2\"", "compare: the second must be a number, not a string"},
		{"number-class \"1\"", "number-class: the number must be a number, not a string"},
	} {
		f := mustFail(t, "", c.expr)
		if f.Code != CodeDSLTypeError || !strings.HasPrefix(f.Message, "type_mismatch: "+c.message) {
			t.Errorf("%s: %v", c.expr, f)
		}
	}
	// length counts characters, not bytes.
	if v := mustEval(t, "", `length "héllo 日本"`); !Equal(v, Num(8)) {
		t.Errorf("%s", DebugString(v))
	}
	if v := mustEval(t, "", `length ""`); !Equal(v, Num(0)) {
		t.Errorf("%s", DebugString(v))
	}
	for _, c := range []struct{ expr, want string }{
		{"compare 1 2", "less"},
		{"compare 2 2", "equal"},
		{"compare 3 2.5", "greater"},
		{"compare (length \"abc\") 3", "equal"},
		{"number-class 1e3", "finite"},
		{"number-class 1e400", "infinity"},
		{"number-class -1e400", "negative-infinity"},
	} {
		if v := mustEval(t, "", c.expr); !Equal(v, KeywordVal(c.want)) {
			t.Errorf("%s: %s", c.expr, DebugString(v))
		}
	}
}

// The stack operators: data last, a new vector each time, bounded by the
// vector's length; an empty vector refuses pop and top with a type error
// naming the operator.
func TestTheStackOperatorsAndTheStringForms(t *testing.T) {
	for _, c := range []struct {
		expr string
		want Val
	}{
		{"push :b [:a]", Vector(KeywordVal("a"), KeywordVal("b"))},
		{"pop [1 2 3]", Vector(Num(1), Num(2))},
		{"top [1 2 3]", Num(3)},
		{"count [1 2 3]", Num(3)},
		{"count []", Num(0)},
		{"top (push \"x\" (pop [1 2]))", StrVal("x")},
		{"pop [1]", Vector()},
		{`quoted "a\"b\\c\n"`, StrVal(`"a\"b\\c\n"`)},
		{`repeat 3 "ab"`, StrVal("ababab")},
		{`repeat 0 "ab"`, StrVal("")},
		{`repeat 2 ""`, StrVal("")},
		{`repeat 100000000000000000000 ""`, StrVal("")},
		// kind passed as a function meets what the checker refuses where it
		// is named: a finite text is refused as a live one is, and no :text
		// or :stream is ever answered.
		{`map kind [1 "s"]`, Vector(KeywordVal("number"), KeywordVal("string"))},
		// keys answers a record's keys in its order, as strings.
		{"keys (record (entry :b 1) (entry :a 2))", Vector(StrVal("b"), StrVal("a"))},
		{"keys (record)", Vector()},
		// The event values a program builds are the ones events delivers.
		{`key "k"`, NewTagged("key", StrVal("k"))},
		{"scalar null", NewTagged("scalar", NullVal{})},
		{"object-start", NewTagged("object-start")},
	} {
		if v := mustEval(t, "", c.expr); !Equal(v, c.want) {
			t.Errorf("%s: %s", c.expr, DebugString(v))
		}
	}
	for _, c := range []struct{ expr, message string }{
		{"pop []", "pop: the vector is empty"},
		{"top []", "top: the vector is empty"},
		{"count 1", "count: the data must be a vector"},
		{"push 1 :k", "push: the data must be a vector"},
		{"pop (record)", "pop: the data must be a vector"},
		{"quoted 1", "quoted: the string must be a string"},
		{`repeat 2.5 "a"`, "repeat: the count must be a whole number of at least 0"},
		{"repeat 2 1", "repeat: the string must be a string"},
		{"keys [1]", "keys: the record must be a record, not a vector"},
		{`map kind [(text "x")]`, "kind: a text cannot be asked"},
		{"key :k", "key: "},
		{"scalar [1]", "scalar: "},
	} {
		f := mustFail(t, "", c.expr)
		if f.Code != CodeDSLTypeError || !strings.HasPrefix(f.Message, "type_mismatch: "+c.message) {
			t.Errorf("%s: %v", c.expr, f)
		}
	}
	// The result is one scalar of the output: past max_scalar_bytes it is
	// refused before it is built, naming the limit. A count past what an
	// index holds is the limit's refusal too, judged before the count is
	// narrowed; a fraction or a negative count is a type error.
	for _, expr := range []string{`repeat 1000000000 "abcdefghij"`, `repeat 100000000000000000000 "a"`} {
		f := mustFail(t, "", expr)
		if f.Code != CodeResourceLimitExceeded || f.Limit == nil || f.Limit.Name != "max_scalar_bytes" {
			t.Errorf("%s: %v", expr, f)
		}
	}
	if f := mustFail(t, "", `repeat 1.5 "a"`); f.Code != CodeDSLTypeError || !strings.Contains(f.Message, "not 1.5") {
		t.Errorf("%v", f)
	}
	// An infinite count is no whole number, as Rust's fract() says.
	if f := mustFail(t, "", `repeat 1e400 "a"`); f.Code != CodeDSLTypeError || !strings.Contains(f.Message, "not 1e400") {
		t.Errorf("%v", f)
	}
	// The quoted form of a string can be six times the string: it is held
	// to the same limit, refused before it is built.
	f := mustFail(t, "", `quoted (repeat 3000000 "\u0001")`)
	if f.Code != CodeResourceLimitExceeded || f.Limit.Name != "max_scalar_bytes" || !strings.HasPrefix(f.Message, "quoted:") {
		t.Errorf("%v", f)
	}
	// Matched as the table events are: the constants compare, the
	// constructors bind their one field.
	src := "def tag [e]\n  match e\n    case object-start \"{\"\n    case array-end \"]\"\n    case (key name) name\n    case (scalar v) v\n    case _ \"?\""
	for _, c := range []struct {
		expr string
		want Val
	}{
		{"tag object-start", StrVal("{")},
		{"tag array-end", StrVal("]")},
		{"tag object-end", StrVal("?")},
		{`tag (key "k")`, StrVal("k")},
		{"tag (scalar 5)", Num(5)},
	} {
		if v := mustEval(t, src, c.expr); !Equal(v, c.want) {
			t.Errorf("%s: %s", c.expr, DebugString(v))
		}
	}
}

func TestClosuresPartialsAndArity(t *testing.T) {
	if v := mustEval(t, "def add [a b] [a b]", "(partial add 1) 2"); !Equal(v, Vector(Num(1), Num(2))) {
		t.Errorf("%s", DebugString(v))
	}
	f := mustFail(t, "def add [a b] [a b]", "add 1")
	if f.Code != CodeDSLTypeError || !strings.HasPrefix(f.Message, "arity: fn add takes 2") {
		t.Errorf("%v", f)
	}
	if f := mustFail(t, "", "get 1"); !strings.HasPrefix(f.Message, "arity: get takes 2") {
		t.Errorf("%v", f)
	}
	if f := mustFail(t, "", "1 2"); !strings.Contains(f.Message, "not a function") {
		t.Errorf("%v", f)
	}
}

func TestAProgramDefShadowsTheLibraryAndTheLibraryKeepsItsOwn(t *testing.T) {
	// The program's csv-row is not what the library's csv calls.
	rt := runtimeOf(t, "def csv-row [o c] 1\ndef export [input] (csv csv-options (table-from-json b input))\ndef b 1")
	lib, f := rt.DefValue(ScopeStdlib, "csv-row")
	if c, ok := lib.(*Closure); f != nil || !ok || c.Scope != ScopeStdlib {
		t.Errorf("%#v %v", lib, f)
	}
	mine, f := rt.DefValue(ScopeProgram, "csv-row")
	if c, ok := mine.(*Closure); f != nil || !ok || c.Scope != ScopeProgram {
		t.Errorf("%#v %v", mine, f)
	}
}

// libraryFailAt is where the library's table writes its fail for a missing
// metadata.
func libraryFailAt(t testing.TB) (int, int) {
	t.Helper()
	lib, ok := StdlibSource("stdlib/table.alc")
	if !ok {
		t.Fatal("stdlib/table.alc")
	}
	for i, line := range strings.Split(lib, "\n") {
		if strings.Contains(line, `fail "Required metadata was not found"`) {
			return i + 1, strings.Index(line, "fail") + 1
		}
	}
	t.Fatal("no fail in stdlib/table.alc")
	return 0, 0
}

func TestFailNamesTheFormPositionInTheRightFile(t *testing.T) {
	f := mustFail(t, "def boom [x]\n  fail \"no\"", "boom 1")
	if f.Code != CodeInputInvalid || f.Message != "no" || f.Row != 2 || f.Column != 3 {
		t.Errorf("%v", f)
	}
	// A failure inside the library names the library's file and line in
	// its message, and no row of the program's.
	row, col := libraryFailAt(t)
	f = mustFail(t, "", "table-finish no-schema")
	if f.Code != CodeInputInvalid || f.Message != fmt.Sprintf("Required metadata was not found (at stdlib/table.alc:%d:%d)", row, col) || f.Row != 0 || f.Column != 0 {
		t.Errorf("%v", f)
	}
	// A program named like a library file is still its own text.
	rt := runtimeNamed(t, "def boom [x]\n  fail \"no\"", "stdlib/table.alc")
	if _, f := evalWith(t, rt, "boom 1"); f == nil || f.Message != "no" || f.Row != 2 || f.Column != 3 {
		t.Errorf("%v", f)
	}
	// A library failure under a native the program called takes the
	// program's position too.
	f = mustFail(t, "", "map (fn [s] (table-finish s)) [no-schema]")
	if !strings.HasSuffix(f.Message, fmt.Sprintf("(at stdlib/table.alc:%d:%d)", row, col)) || f.Row != 1 || f.Column != 1 {
		t.Errorf("%v", f)
	}
}

func TestStreamsArePlansAndExportAppliesToTheInput(t *testing.T) {
	out, f := runtimeOf(t, "def export [input] (json input)").Export()
	if text, ok := out.(TextVal); f != nil || !ok || text.Plan.Kind != PlanJSON {
		t.Errorf("%#v %v", out, f)
	}
	if _, f := runtimeOf(t, "def x 1").Export(); f == nil || !strings.HasPrefix(f.Message, "no_export: ") {
		t.Errorf("%v", f)
	}
	if _, f := runtimeOf(t, "def export 1").Export(); f == nil || !strings.HasPrefix(f.Message, "type_mismatch: ") {
		t.Errorf("%v", f)
	}
}

func stateBoundsOf(maxBytes uint64, maxDepth int) Bounds {
	return Bounds{NodeBytes: 16, MaxBytes: maxBytes, BytesLimit: "max_metadata_bytes", MaxDepth: maxDepth, What: "the state"}
}

const unbounded = int(^uint(0) >> 1)

// A partial whose function is itself a partial holds the one before it,
// so a chain of them (a scan-emit state that wraps itself once per item)
// is measured link by link: each link one node and one level, and every
// link's arguments counted.
func TestAChainOfPartialsIsMeasured(t *testing.T) {
	rt := runtimeOf(t, "")
	arg := StrVal(strings.Repeat("a", 100))
	chain := func(links int) Val {
		var fn Fn = LookupNative("vector")
		for i := 0; i < links; i++ {
			fn = &Partial{F: fn, Args: []Val{arg}}
		}
		return fn
	}
	three, f := rt.Measure(chain(3), stateBoundsOf(^uint64(0), unbounded))
	if f != nil || three != (Measure{Bytes: 6*16 + 300, Depth: 4}) {
		t.Errorf("%+v %v", three, f)
	}
	long := chain(1000)
	if _, f := rt.Measure(long, stateBoundsOf(1<<30, 256)); f == nil || f.Limit.Name != "max_depth" {
		t.Errorf("%v", f)
	}
	if _, f := rt.Measure(long, stateBoundsOf(4096, unbounded)); f == nil || f.Limit.Name != "max_metadata_bytes" {
		t.Errorf("%v", f)
	}
}

// A finite text keeps the function its concat-map applies, so a state
// that hides the one before it in that function's partial (a state of
// [(concat-map (partial g s) ["x"])]) is walked through it: vector, text,
// plan, function, then the state before, each a level.
func TestTheFunctionAFiniteTextAppliesIsMeasured(t *testing.T) {
	rt := runtimeOf(t, "")
	at := SourceSpan{File: NewFile("t.alc")}
	link := func(prev Val) Val {
		fn := &Partial{F: LookupNative("vector"), Args: []Val{prev}}
		text := TextVal{Plan: &Plan{Kind: PlanConcatMap, F: fn, Seq: Seq{Vector: []Val{StrVal("x")}}, At: at}}
		return Vector(text)
	}
	chain := func(links int) Val {
		v := Val(Vector())
		for i := 0; i < links; i++ {
			v = link(v)
		}
		return v
	}
	// One link: the vector (1), its text (2), the plan (3), the function
	// and the item "x" (4), the state before (5).
	if one, f := rt.Measure(chain(1), stateBoundsOf(^uint64(0), unbounded)); f != nil || one != (Measure{Bytes: 6*16 + 1, Depth: 5}) {
		t.Errorf("%+v %v", one, f)
	}
	// Each further link is four more levels and five more nodes.
	if three, f := rt.Measure(chain(3), stateBoundsOf(^uint64(0), unbounded)); f != nil || three != (Measure{Bytes: 16*16 + 3, Depth: 13}) {
		t.Errorf("%+v %v", three, f)
	}
	long := chain(1000)
	if _, f := rt.Measure(long, stateBoundsOf(1<<30, 256)); f == nil || f.Limit.Name != "max_depth" {
		t.Errorf("%v", f)
	}
	if _, f := rt.Measure(long, stateBoundsOf(4096, unbounded)); f == nil || f.Limit.Name != "max_metadata_bytes" {
		t.Errorf("%v", f)
	}
}

func TestTheFastPathsSwitch(t *testing.T) {
	src := "def b\n  record\n    entry :columns (path \"m\")\n    entry :rows (path \"r\" each-index)\n    entry :column (fn [d] d)\ndef export [input] (csv csv-options (table-from-json b input))"
	fast, f := runtimeOf(t, src).Export()
	if text, ok := fast.(TextVal); f != nil || !ok || text.Plan.Kind != PlanCsv {
		t.Errorf("%#v %v", fast, f)
	}
	slow, f := runtimeOf(t, src).WithNative(false).Export()
	if text, ok := slow.(TextVal); f != nil || !ok || text.Plan.Kind != PlanConcatMap {
		t.Errorf("%#v %v", slow, f)
	}
}

// The host's abort flag is read every few evaluation steps, however the
// steps are taken.
func TestTheAbortFlagIsReadEveryFewSteps(t *testing.T) {
	flag := tt.NewAbortFlag()
	rt := runtimeOf(t, "").WithAbort(flag)
	for i := 0; i < 2*abortEvery; i++ {
		if f := rt.Tick(); f != nil {
			t.Fatalf("step %d: %v", i, f)
		}
	}
	flag.Abort()
	var f *Fail
	for i := 0; i < abortEvery && f == nil; i++ {
		f = rt.Tick()
	}
	if f == nil || f.Code != CodeAborted {
		t.Errorf("%v", f)
	}
}

// A local named like a special form shadows it: the form is a call of the
// local, as the evaluator reads a special form only while no local has
// its name.
func TestALocalShadowsASpecialForm(t *testing.T) {
	if v := mustEval(t, "", "((fn [if] (if 1 2 3)) vector)"); !Equal(v, Vector(Num(1), Num(2), Num(3))) {
		t.Errorf("%s", DebugString(v))
	}
	if v := mustEval(t, "", "let [fn vector] (fn [1] 2)"); !Equal(v, Vector(Vector(Num(1)), Num(2))) {
		t.Errorf("%s", DebugString(v))
	}
}
