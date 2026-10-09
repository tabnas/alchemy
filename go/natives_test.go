// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// natives_test.go: the natives' implementations (rs/src/stdlib/registry.rs's
// tests), and the two halves of the natives held to the same names.

import (
	"math"
	"sort"
	"strings"
	"testing"

	"github.com/tabnas/alchemy/go/shared"
)

// TestEveryNativeHasAnImplementation: the registry's table (registry.go)
// and the implementations (natives.go) name the same natives, so neither
// can gain or lose one alone.
func TestEveryNativeHasAnImplementation(t *testing.T) {
	var registered, implemented []string
	for _, n := range Natives() {
		registered = append(registered, n.Name)
	}
	for name := range nativeImpls {
		implemented = append(implemented, name)
	}
	sort.Strings(registered)
	sort.Strings(implemented)
	if strings.Join(registered, " ") != strings.Join(implemented, " ") {
		t.Errorf("registered %v\nimplemented %v", registered, implemented)
	}
}

// The JSON string form, with the C1 controls escaped as well, in the
// render package's lowercase hex; everything else as itself.
func TestQuotedIsTheJSONStringFormWithTheC1ControlsEscaped(t *testing.T) {
	for _, c := range []struct{ in, want string }{
		{"", `""`},
		{"plain", `"plain"`},
		{"q\" b\\ n\n r\r t\t bs\b ff\f nul\x00 c1\x01 us\x1f", `"q\" b\\ n\n r\r t\t bs\b ff\f nul\u0000 c1\u0001 us\u001f"`},
		{"del\u007f pad\u0080 apc\u009f nbsp  é 日本 🚀 /", "\"del\\u007f pad\\u0080 apc\\u009f nbsp  é 日本 🚀 /\""},
	} {
		if got := quote(c.in); got != c.want {
			t.Errorf("%q: %q, want %q", c.in, got, c.want)
		}
	}
	// What a retained value writes for the same string (the shared
	// Datum, which transduce's captures hold), where both escape: the two
	// agree on JSON's own escapes.
	s := "q\" \\ \n \x1f é"
	if json := shared.StringDatum(s).String(); quote(s) != json {
		t.Errorf("%q, the shared Datum %q", quote(s), json)
	}
	// The length counted before building is the length built.
	for _, s := range []string{"", "plain", "q\" \\ \n \x1f é", "del\u007f pad\u0080 日本 🚀", "nonchar\ufffe\uffff\ufffd"} {
		if quotedLen(s) != len(quote(s)) {
			t.Errorf("%q: %d, built %d", s, quotedLen(s), len(quote(s)))
		}
	}
	// The two noncharacters YAML's printable set and XML's characters
	// exclude are escaped; U+FFFD, a character, is not.
	if got := quote("\ufffe\uffff\ufffd"); got != "\"\\ufffe\\uffff\ufffd\"" {
		t.Errorf("%q", got)
	}
}

func TestUnquoteReadsBackWhatQuoteWrites(t *testing.T) {
	for _, s := range []string{
		"",
		"plain",
		"q\" b\\ n\n r\r t\t bs\b ff\f nul\x00 us\x1f",
		"del\u007f pad\u0080 apc\u009f nbsp\u00a0 é 日本 🚀 /",
		"\ufffe\uffff",
	} {
		if got, ok := unquote(quote(s)); !ok || got != s {
			t.Errorf("%q: %q %v", s, got, ok)
		}
	}
	// JSON's other spellings: the solidus, uppercase hex, a surrogate pair.
	if got, ok := unquote(`"\/\u00E9\ud83d\ude80"`); !ok || got != "/é🚀" {
		t.Errorf("%q %v", got, ok)
	}
	// Not a double-quoted form.
	for _, text := range []string{
		"",
		"\"",
		"plain",
		"\"open",
		"\"in\"side\"",
		"\"raw\nline\"",
		"\"bad \\x escape\"",
		"\"short \\u12\"",
		"\"lone \\ud800\"",
		"\"low \\udc00 first\"",
		"\"pair \\ud800\\u0041\"",
	} {
		if got, ok := unquote(text); ok {
			t.Errorf("%q: %q", text, got)
		}
	}
}

// unquoted reads a double-quoted form back, and refuses any other text as
// data the program was given, naming it.
func TestUnquotedReadsADoubleQuotedFormOrRefusesTheText(t *testing.T) {
	at := SourceSpan{File: NewFile("t")}
	if v, f := nUnquoted(runtimeOf(t, ""), []Val{StrVal(`"a\nb \ud83d\ude00"`)}, at); f != nil || !Equal(v, StrVal("a\nb 😀")) {
		t.Errorf("%v %v", v, f)
	}
	if _, f := nUnquoted(runtimeOf(t, ""), []Val{StrVal("x")}, at); f == nil || f.Code != CodeInputInvalid ||
		f.Message != `unquoted: "x" is not a double-quoted string` {
		t.Errorf("%v", f)
	}
	if _, f := nUnquoted(runtimeOf(t, ""), []Val{StrVal(`"\ud800"`)}, at); f == nil || f.Code != CodeInputInvalid {
		t.Errorf("%v", f)
	}
	if f := mustFail(t, "", "unquoted 1"); f.Message != "type_mismatch: unquoted: the string must be a string, not a number" {
		t.Errorf("%v", f)
	}
}

// chars-within tests every character, a code point at a time, against
// ranges it validates first, and takes an evaluation step every 4096
// characters, so the host's abort flag stops a long string's test.
func TestCharsWithinTestsEveryCharacterInStepsTheAbortFlagReads(t *testing.T) {
	const xml = "[[9 10] [13 13] [32 55295] [57344 65533] [65536 1114111]]"
	for _, c := range []struct {
		expr string
		want bool
	}{
		{"chars-within " + xml + ` "tab\there 😀"`, true},
		{"chars-within " + xml + ` "nul\u0000"`, false},
		{"chars-within " + xml + ` "\uffff"`, false},
		{"chars-within " + xml + ` ""`, true},
		{`chars-within [] ""`, true},
		{`chars-within [] "a"`, false},
		// A character above U+FFFF is one code point.
		{`chars-within [[128512 128512]] "😀"`, true},
		{`chars-within [[0 65535]] "😀"`, false},
	} {
		if v := mustEval(t, "", c.expr); !Equal(v, BoolVal(c.want)) {
			t.Errorf("%s: %s", c.expr, DebugString(v))
		}
	}
	for _, c := range []struct{ expr, message string }{
		{`chars-within [[5 3]] "a"`, "a range must be a vector [low high] with low at most high, not [5 3]"},
		{`chars-within [[0 1.5]] "a"`, "a range's bound must be a code point from 0 to 1114111, not 1.5"},
		{`chars-within [[-1 2]] "a"`, "a range's bound must be a code point from 0 to 1114111, not -1"},
		{`chars-within [[0 1114112]] "a"`, "a range's bound must be a code point from 0 to 1114111, not 1114112"},
		{`chars-within [[0 (number "Infinity")]] "a"`, "a range's bound must be a code point from 0 to 1114111, not inf"},
		{`chars-within [["a" 1]] "a"`, "a range's bound must be a number, not a string"},
		{`chars-within [[0]] "a"`, "a range must be a vector [low high], not a vector of 1 items"},
		{`chars-within [1] "a"`, "a range must be a vector [low high], not a number"},
		{`chars-within 1 "a"`, "the data must be a vector, not a number"},
		{`chars-within [[5 3]] 1`, "a range must be a vector [low high] with low at most high, not [5 3]"},
		{`chars-within [[0 3]] 1`, "the string must be a string, not a number"},
	} {
		if f := mustFail(t, "", c.expr); f.Code != CodeDSLTypeError || f.Message != "type_mismatch: chars-within: "+c.message {
			t.Errorf("%s: %v", c.expr, f)
		}
	}
	at := SourceSpan{File: NewFile("t")}
	all := Vector(Vector(Num(0), Num(1114111)))
	ascii := Vector(Vector(Num(0), Num(127)))
	long := strings.Repeat("k", 4096*64)
	if v, f := nCharsWithin(runtimeOf(t, ""), []Val{all, StrVal(long)}, at); f != nil || !Equal(v, BoolVal(true)) {
		t.Errorf("%v %v", v, f)
	}
	flag := shared.NewAbortFlag()
	rt := runtimeOf(t, "").WithAbort(flag)
	flag.Abort()
	if _, f := nCharsWithin(rt, []Val{all, StrVal(long)}, at); f == nil || f.Code != CodeAborted {
		t.Errorf("%v", f)
	}
	// The first character outside the ranges answers at once.
	if v, f := nCharsWithin(rt, []Val{ascii, StrVal("é" + long)}, at); f != nil || !Equal(v, BoolVal(false)) {
		t.Errorf("%v %v", v, f)
	}
}

// A number that is not finite is refused by scalar-text, as the renderers
// refuse it, unless the options' :non-finite names what to write instead;
// a policy that is none is a type error where a number needs it, and only
// there.
func TestScalarTextWritesANumberThatIsNotFiniteByTheOptionsPolicy(t *testing.T) {
	options := func(policy string) string {
		o := `(record (entry :null-text "-") (entry :missing :error)`
		if policy != "" {
			o += " (entry :non-finite " + policy + ")"
		}
		return o + ")"
	}
	text := func(policy, x string) string { return "scalar-text " + options(policy) + ` (number "` + x + `")` }
	for _, c := range []struct{ policy, x, want string }{
		{":null", "NaN", "-"},
		{":literal", "NaN", "NaN"},
		{":literal", "Infinity", "Infinity"},
		{":literal", "-Infinity", "-Infinity"},
		{":reject", "2.50", "2.50"},
		{":nope", "2.50", "2.50"},
	} {
		if v := mustEval(t, "", text(c.policy, c.x)); !Equal(v, StrVal(c.want)) {
			t.Errorf("%s %s: %s", c.policy, c.x, DebugString(v))
		}
	}
	for _, policy := range []string{"", ":reject"} {
		if f := mustFail(t, "", text(policy, "Infinity")); f.Code != CodeTargetValueUnrepresentable {
			t.Errorf("%q: %v", policy, f)
		}
	}
	if f := mustFail(t, "", text(":nope", "NaN")); f.Code != CodeDSLTypeError ||
		f.Message != "type_mismatch: :non-finite must be :reject, :null or :literal, not :nope" {
		t.Errorf("%v", f)
	}
	if f := mustFail(t, "", text("1", "NaN")); f.Message != "type_mismatch: :non-finite must be :reject, :null or :literal, not a number" {
		t.Errorf("%v", f)
	}
	// The policy of a record, and of what is no record: none.
	for _, c := range []struct {
		v    Val
		want NonFinite
	}{
		{NullVal{}, NonFiniteReject},
		{mustEval(t, "", "(record)"), NonFiniteReject},
		{mustEval(t, "", "(record (entry :non-finite missing))"), NonFiniteReject},
		{mustEval(t, "", "(record (entry :non-finite :literal))"), NonFiniteLiteral},
	} {
		if got, f := NonFinitePolicy(c.v); f != nil || got != c.want {
			t.Errorf("%s: %v %v", DebugString(c.v), got, f)
		}
	}
	if NonFiniteWord(math.Inf(-1)) != "-Infinity" {
		t.Error(NonFiniteWord(math.Inf(-1)))
	}
	if _, ok := NonFiniteNamed("nope"); ok {
		t.Error("nope names a policy")
	}
}

// json takes an options record first, which holds :non-finite and nothing
// else, :reject or :null; the plan carries the policy.
func TestJSONTakesAnOptionsRecordBeforeItsEvents(t *testing.T) {
	policy := func(options string) NonFinite {
		text, ok := mustCompile(t, "def export [input] (json "+options+"input)", "t.alc").Result().(TextVal)
		if !ok || text.Plan.Kind != PlanJSON {
			t.Fatalf("%s: not a json plan", options)
		}
		return text.Plan.NonFinite
	}
	for _, c := range []struct {
		options string
		want    NonFinite
	}{
		{"", NonFiniteReject},
		{"(record) ", NonFiniteReject},
		{"(record (entry :non-finite :reject)) ", NonFiniteReject},
		{"(record (entry :non-finite :null)) ", NonFiniteNull},
	} {
		if got := policy(c.options); got != c.want {
			t.Errorf("%q: %v", c.options, got)
		}
	}
	for _, c := range []struct{ options, message string }{
		{"(record (entry :non-finite :literal))", "json: :non-finite must be :reject or :null, not :literal"},
		{"(record (entry :non-finite :nope))", ":non-finite must be :reject, :null or :literal, not :nope"},
		{"(record (entry :non-finite :null) (entry :indent 2))", "json: an option must be :non-finite, not :indent"},
	} {
		_, f := Compile("def export [input] (json "+c.options+" input)", "t.alc", routers, renderers)
		if f == nil || f.Code != CodeDSLTypeError || f.Message != "type_mismatch: "+c.message || f.Row != 1 || f.Column != 20 {
			t.Errorf("%s: %v", c.options, f)
		}
	}
}

// kind names every retained value's kind by one keyword, the word the
// messages use.
func TestKindNamesAValueByOneKeyword(t *testing.T) {
	for _, c := range []struct {
		v    Val
		want string
	}{
		{NullVal{}, "null"},
		{BoolVal(true), "boolean"},
		{Num(1.5), "number"},
		{StrVal("s"), "string"},
		{KeywordVal("k"), "keyword"},
		{Vector(), "vector"},
		{Missing(), "missing"},
		{NewTagged("key", StrVal("a")), "tagged"},
	} {
		if got, ok := kindWord(c.v); !ok || got != c.want {
			t.Errorf("%s: %s", DebugString(c.v), got)
		}
	}
	if _, ok := kindWord(TextVal{Plan: litPlan("x")}); ok {
		t.Error("a text has a kind")
	}
}

// NaN and the infinities, which no literal spells: number-class names
// each, and compare orders the infinities and leaves NaN unordered.
func TestNumberClassAndCompareSeeTheNonFiniteNumbers(t *testing.T) {
	rt := runtimeOf(t, "")
	at := SourceSpan{File: NewFile("t")}
	word := func(v Val, f *Fail) string {
		if f != nil {
			t.Fatal(f)
		}
		return string(v.(KeywordVal))
	}
	class := func(v float64) string { return word(nNumberClass(rt, []Val{Num(v)}, at)) }
	for _, c := range []struct {
		v    float64
		want string
	}{
		{1.5, "finite"}, {math.Copysign(0, -1), "finite"}, {math.Inf(1), "infinity"},
		{math.Inf(-1), "negative-infinity"}, {math.NaN(), "nan"},
	} {
		if got := class(c.v); got != c.want {
			t.Errorf("%v: %s", c.v, got)
		}
	}
	order := func(x, y float64) string { return word(nCompare(rt, []Val{Num(x), Num(y)}, at)) }
	for _, c := range []struct {
		x, y float64
		want string
	}{
		{math.NaN(), 1, "unordered"}, {1, math.NaN(), "unordered"}, {math.Copysign(0, -1), 0, "equal"},
		{math.Inf(-1), -1e308, "less"}, {math.Inf(1), math.Inf(1), "equal"},
	} {
		if got := order(c.x, c.y); got != c.want {
			t.Errorf("%v %v: %s", c.x, c.y, got)
		}
	}
}

// length counts characters a chunk at a time, however a chunk cuts a
// character, and takes an evaluation step per chunk, so the host's abort
// flag stops the count of a long string.
func TestLengthCountsCharactersInStepsTheAbortFlagReads(t *testing.T) {
	at := SourceSpan{File: NewFile("t")}
	long := StrVal(strings.Repeat("héllo 日本", lengthChunk/3))
	v, f := nLength(runtimeOf(t, ""), []Val{long}, at)
	if f != nil || !Equal(v, Num(float64(8*(lengthChunk/3)))) {
		t.Errorf("%v %v", v, f)
	}
	flag := shared.NewAbortFlag()
	rt := runtimeOf(t, "").WithAbort(flag)
	flag.Abort()
	huge := StrVal(strings.Repeat("k", lengthChunk*64))
	if _, f := nLength(rt, []Val{huge}, at); f == nil || f.Code != CodeAborted {
		t.Errorf("%v", f)
	}
}

// indices gives the positions of a vector's items, the labels the
// inferred table gives an array row's cells, and takes an evaluation step
// per item, so the host's abort flag stops a long vector's count.
func TestIndicesGivesThePositionsInStepsTheAbortFlagReads(t *testing.T) {
	if v := mustEval(t, "", "indices [:a :b :c]"); !Equal(v, Vector(Num(0), Num(1), Num(2))) {
		t.Errorf("%s", DebugString(v))
	}
	if v := mustEval(t, "", "indices []"); !Equal(v, Vector()) {
		t.Errorf("%s", DebugString(v))
	}
	if f := mustFail(t, "", "indices (record)"); f.Code != CodeDSLTypeError || f.Message != "type_mismatch: indices: a vector was expected, not a record" {
		t.Errorf("%v", f)
	}
	at := SourceSpan{File: NewFile("t")}
	items := make([]Val, 1000)
	for i := range items {
		items[i] = NullVal{}
	}
	long := Vector(items...)
	if v, f := nIndices(runtimeOf(t, ""), []Val{long}, at); f != nil || len(v.(*VectorVal).Items) != 1000 {
		t.Errorf("%v %v", v, f)
	}
	flag := shared.NewAbortFlag()
	rt := runtimeOf(t, "").WithAbort(flag)
	flag.Abort()
	if _, f := nIndices(rt, []Val{long}, at); f == nil || f.Code != CodeAborted {
		t.Errorf("%v", f)
	}
}

// A number without a lexeme is written as the renderers write it, by this
// package's own copy of render's formatter (shortestNumber): the shortest
// digits that read back, positional within [1e-6, 1e21), exponent form
// outside it with no '+' and no leading zeros. The first six are the Rust
// crate's cases (shortest_number, rs/src/stdlib/registry.rs); the rest
// hold the layout at its edges. alchemy-cli's differential test
// (go/e2e/differential_test.go) holds the copy to render's own on real
// documents.
func TestNumbersPrintAsTheRenderersPrintThem(t *testing.T) {
	for _, c := range []struct {
		v    float64
		want string
	}{
		{1, "1"}, {50.25, "50.25"}, {1e20, "100000000000000000000"}, {1e21, "1e21"},
		{1e-7, "1e-7"}, {math.Copysign(0, -1), "-0"},
		// Zero, and the negatives of the cases above.
		{0, "0"}, {-1, "-1"}, {-50.25, "-50.25"}, {-1e20, "-100000000000000000000"},
		{-1e21, "-1e21"}, {-1e-7, "-1e-7"},
		// Positional: fractions, integers, the shortest digits.
		{0.1, "0.1"}, {100, "100"}, {12345.678, "12345.678"}, {0.000123, "0.000123"},
		{1.2345678901234568e20, "123456789012345680000"}, {9.999999999999999e20, "999999999999999900000"},
		// The lower edge: 1e-6 is positional, anything smaller is not.
		{1e-6, "0.000001"}, {1.5e-6, "0.0000015"}, {-1e-6, "-0.000001"}, {9.99e-7, "9.99e-7"},
		// Exponent form: large and small exponents, with and without a
		// fraction, down to the subnormals.
		{1.5e21, "1.5e21"}, {1e100, "1e100"}, {-1e300, "-1e300"}, {1.7976931348623157e308, "1.7976931348623157e308"},
		{-1.25e-10, "-1.25e-10"}, {2.2250738585072014e-308, "2.2250738585072014e-308"}, {5e-324, "5e-324"},
		// No finite text: "", as render's FormatValue answers.
		{math.Inf(1), ""}, {math.Inf(-1), ""}, {math.NaN(), ""},
	} {
		if got := shortestNumber(c.v); got != c.want {
			t.Errorf("%v: %q, want %q", c.v, got, c.want)
		}
	}
	if s, f := numberText(1.5, "1.50", true); f != nil || s != "1.50" {
		t.Errorf("%q %v", s, f)
	}
	if _, f := numberText(1, "01", true); f == nil || f.Code != CodeInvalidNumber {
		t.Errorf("%v", f)
	}
	if _, f := numberText(math.Inf(1), "1e999", true); f == nil || f.Code != CodeTargetValueUnrepresentable {
		t.Errorf("%v", f)
	}
	if _, f := numberText(math.NaN(), "", false); f == nil || f.Code != CodeTargetValueUnrepresentable {
		t.Errorf("%v", f)
	}
	// Without a lexeme, numberText is shortestNumber's text.
	if s, f := numberText(-1e-7, "", false); f != nil || s != "-1e-7" {
		t.Errorf("%q %v", s, f)
	}
}

// fail takes a code before the message: one of three keywords, each its
// code; any other keyword, or a code that is not one, is a type error, as
// the checker's is for a literal keyword.
func TestFailNamesItsCode(t *testing.T) {
	src := `def boom [k] (fail k "no")`
	for _, c := range []struct {
		expr string
		code Code
	}{
		{"boom :unrepresentable", CodeTargetValueUnrepresentable},
		{"boom :protocol-order", CodeProtocolOrderError},
		{"boom :invalid", CodeInputInvalid},
	} {
		if f := mustFail(t, src, c.expr); f.Code != c.code || f.Message != "no" || f.Row != 1 || f.Column != 14 {
			t.Errorf("%s: %v", c.expr, f)
		}
	}
	for _, c := range []struct{ src, expr, message string }{
		{src, "boom :nope", "type_mismatch: the code of fail must be :invalid, :unrepresentable or :protocol-order, not :nope"},
		{src, "boom 1", "type_mismatch: fail: the code must be a keyword, not a number"},
		{`def boom [k] (fail k)`, "boom 1", "type_mismatch: fail: the message must be a string, not a number"},
	} {
		if f := mustFail(t, c.src, c.expr); f.Code != CodeDSLTypeError || f.Message != c.message {
			t.Errorf("%s: %v", c.expr, f)
		}
	}
}
