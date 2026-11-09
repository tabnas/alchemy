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

// chars-within counts its work in comparisons, so ranges from data as many
// as they are take evaluation steps, and a bound on steps (the fuel, the
// abort flag) stops a test that has not yet read 4,096 characters.
func TestCharsWithinTakesAStepPerComparisons(t *testing.T) {
	at := SourceSpan{File: NewFile("t.alc")}
	ranges := make([]Val, 100_000)
	for i := range ranges {
		ranges[i] = Vector(Num(5), Num(5))
	}
	rt := runtimeOf(t, "def export [input] input").WithFuel(10)
	if _, f := nCharsWithin(rt, []Val{Vector(ranges...), StrVal("a")}, at); f == nil || f.Limit == nil || f.Limit.Name != "max_plan_steps" {
		t.Errorf("%v", f)
	}
	// A long string against a few ranges is counted the same way.
	few := Vector(Vector(Num(0), Num(1)), Vector(Num(97), Num(97)))
	rt = runtimeOf(t, "").WithFuel(10)
	if _, f := nCharsWithin(rt, []Val{few, StrVal(strings.Repeat("a", CharsWithinStep*6))}, at); f == nil || f.Limit == nil || f.Limit.Name != "max_plan_steps" {
		t.Errorf("%v", f)
	}
}

// unquoted reads its input an evaluation step per UnquotedStep characters
// as it goes, as length counts a long string in steps: ten steps of fuel
// read ten times that many characters and no more, whether or not the
// string turns out to be a quoted form, and a string refused at its start
// costs no more than its first step.
func TestUnquotedTakesAStepPerCharactersRead(t *testing.T) {
	at := SourceSpan{File: NewFile("t.alc")}
	run := func(text string) (Val, *Fail) {
		return nUnquoted(runtimeOf(t, "").WithFuel(10), []Val{StrVal(text)}, at)
	}
	// A quoted form n characters long, its quotes included.
	quoted := func(n int, c string) string { return `"` + strings.Repeat(c, n-2) + `"` }
	limit := func(f *Fail) bool { return f != nil && f.Limit != nil && f.Limit.Name == "max_plan_steps" }
	// A character is a rune: one of four bytes is one.
	for _, c := range []string{"a", "\U0001F680"} {
		if v, f := run(quoted(10*UnquotedStep, c)); f != nil || v != StrVal(strings.Repeat(c, 10*UnquotedStep-2)) {
			t.Errorf("%q: %v", c, f)
		}
	}
	// One character more takes an eleventh step.
	if _, f := run(quoted(10*UnquotedStep+1, "a")); !limit(f) {
		t.Errorf("%v", f)
	}
	// A string with no closing quote is read as far before it is refused.
	if _, f := run(`"` + strings.Repeat("a", 10*UnquotedStep)); !limit(f) {
		t.Errorf("%v", f)
	}
	if _, f := run("x" + strings.Repeat("a", 100*UnquotedStep)); f == nil || f.Code != CodeInputInvalid {
		t.Errorf("%v", f)
	}
}

// is-number answers whether number reads a string, and number fails
// exactly where it answers false; a string that is not one is the same type
// error to both.
func TestIsNumberAnswersWhetherNumberReadsTheString(t *testing.T) {
	at := SourceSpan{File: NewFile("t")}
	numbers := []string{"0", "-0", "12", "1.5", "-1.5e10", "2E-3", "1e+5", "0e0", "1e5", "1e999",
		"12345678901234567890", "Infinity", "-Infinity", "NaN"}
	others := []string{"", "-", "01", "-01", "1.", ".5", "+1", "1e", "1e+", "1-2", " 1", "1 ", "0x10",
		"1_000", "1.5e", "-NaN", "nan", "infinity", "Inf", "Infinityx", "NaNN", "١"}
	for _, c := range []struct {
		texts []string
		reads bool
	}{{numbers, true}, {others, false}} {
		for _, text := range c.texts {
			if v, f := nIsNumber(runtimeOf(t, ""), []Val{StrVal(text)}, at); f != nil || v != BoolVal(c.reads) {
				t.Errorf("is-number %q: %v %v", text, v, f)
			}
			_, f := nNumber(runtimeOf(t, ""), []Val{StrVal(text)}, at)
			if c.reads && f != nil || !c.reads && (f == nil || f.Code != CodeInputInvalid) {
				t.Errorf("number %q: %v", text, f)
			}
		}
	}
	// A finite number keeps its text as the lexeme; a non-finite one by
	// name has none.
	if v, f := nNumber(runtimeOf(t, ""), []Val{StrVal("1.50")}, at); f != nil || v != NumLexeme(1.5, "1.50") {
		t.Errorf("%v %v", v, f)
	}
	if v, f := nNumber(runtimeOf(t, ""), []Val{StrVal("-Infinity")}, at); f != nil || v != Num(math.Inf(-1)) {
		t.Errorf("%v %v", v, f)
	}
	for _, name := range []string{"is-number", "number"} {
		f := mustFail(t, "", name+" 1")
		if f.Code != CodeDSLTypeError || f.Message != "type_mismatch: "+name+": the string must be a string, not a number" {
			t.Errorf("%s: %v", name, f)
		}
	}
}

// number and is-number read their input an evaluation step per NumberStep
// characters as they go, as unquoted reads its own: ten steps of fuel read
// ten times that many characters and no more, whether or not the string
// turns out to spell a number, and a string refused at its start costs no
// more than its first step.
func TestNumberAndIsNumberTakeAStepPerCharactersRead(t *testing.T) {
	at := SourceSpan{File: NewFile("t.alc")}
	is := func(text string) (Val, *Fail) {
		return nIsNumber(runtimeOf(t, "").WithFuel(10), []Val{StrVal(text)}, at)
	}
	read := func(text string) (Val, *Fail) {
		return nNumber(runtimeOf(t, "").WithFuel(10), []Val{StrVal(text)}, at)
	}
	limit := func(f *Fail) bool { return f != nil && f.Limit != nil && f.Limit.Name == "max_plan_steps" }
	// A number n characters long.
	long := func(n int) string { return "0." + strings.Repeat("5", n-2) }
	if v, f := is(long(10 * NumberStep)); f != nil || v != BoolVal(true) {
		t.Errorf("%v %v", v, f)
	}
	if v, f := read(long(10 * NumberStep)); f != nil || v.(NumVal).Lexeme != long(10*NumberStep) {
		t.Errorf("%v", f)
	}
	// One character more takes an eleventh step, and so does a character
	// after as many that ends the spelling: the string is read as far
	// before it is refused.
	for _, text := range []string{long(10*NumberStep + 1), long(10*NumberStep) + "x"} {
		if _, f := is(text); !limit(f) {
			t.Errorf("is-number: %v", f)
		}
		if _, f := read(text); !limit(f) {
			t.Errorf("number: %v", f)
		}
	}
	early := "x" + long(100*NumberStep)
	if v, f := is(early); f != nil || v != BoolVal(false) {
		t.Errorf("%v %v", v, f)
	}
	if _, f := read(early); f == nil || f.Code != CodeInputInvalid {
		t.Errorf("%v", f)
	}
}

// split answers the strings between a separator's occurrences, found from
// the front, none overlapping another: one more than it occurs, an empty
// one where two meet or at either end. The separator is a literal that must
// not be empty.
func TestSplitAnswersTheStringsBetweenTheSeparators(t *testing.T) {
	at := SourceSpan{File: NewFile("t")}
	for _, c := range []struct {
		separator, s string
		want         []string
	}{
		{".", "a..b", []string{"a", "", "b"}},
		{".", "", []string{""}},
		{".", "abc", []string{"abc"}},
		{".", ".a.", []string{"", "a", ""}},
		{"::", "a::b::c", []string{"a", "b", "c"}},
		{"aa", "aaa", []string{"", "a"}},
		{"→", "α→β→γ", []string{"α", "β", "γ"}},
	} {
		v, f := nSplit(runtimeOf(t, ""), []Val{StrVal(c.separator), StrVal(c.s)}, at)
		want := make([]Val, len(c.want))
		for i, s := range c.want {
			want[i] = StrVal(s)
		}
		if f != nil || !Equal(v, Vector(want...)) {
			t.Errorf("split %q %q: %v %v", c.separator, c.s, DebugString(v), f)
		}
	}
	for _, c := range []struct{ expr, message string }{
		{`split "" "a"`, "the separator must be a non-empty string, not the empty string"},
		{`split 1 "a"`, "the separator must be a string, not a number"},
		{`split "." 1`, "the string must be a string, not a number"},
	} {
		if f := mustFail(t, "", c.expr); f.Code != CodeDSLTypeError || f.Message != "type_mismatch: split: "+c.message {
			t.Errorf("%s: %v", c.expr, f)
		}
	}
}

// split takes an evaluation step for each SplitStep characters it reads, or
// part of them, and one for each part: ten steps of fuel read nine times
// that many characters into one part, or split nine parts off eight
// separators, and no more.
func TestSplitTakesAStepPerCharactersReadAndPerPart(t *testing.T) {
	at := SourceSpan{File: NewFile("t.alc")}
	run := func(s string) (Val, *Fail) {
		return nSplit(runtimeOf(t, "").WithFuel(10), []Val{StrVal("."), StrVal(s)}, at)
	}
	limit := func(f *Fail) bool { return f != nil && f.Limit != nil && f.Limit.Name == "max_plan_steps" }
	// A character is a rune: one of four bytes is one.
	for _, c := range []string{"a", "\U0001F680"} {
		if _, f := run(strings.Repeat(c, 9*SplitStep)); f != nil {
			t.Errorf("%q: %v", c, f)
		}
		if _, f := run(strings.Repeat(c, 9*SplitStep+1)); !limit(f) {
			t.Errorf("%q: %v", c, f)
		}
	}
	if _, f := run(strings.Repeat(".", 8)); f != nil {
		t.Errorf("%v", f)
	}
	if _, f := run(strings.Repeat(".", 9)); !limit(f) {
		t.Errorf("%v", f)
	}
}

func numsVal(xs ...float64) *VectorVal {
	items := make([]Val, len(xs))
	for i, x := range xs {
		items[i] = Num(x)
	}
	return Vector(items...)
}

func countTo(n int) []float64 {
	xs := make([]float64, n)
	for i := range xs {
		xs[i] = float64(i)
	}
	return xs
}

// push-all joins two vectors, the vector's items first, and leaves both as
// they were; what is not a vector is a type error naming the argument.
func TestPushAllJoinsTwoVectors(t *testing.T) {
	for _, c := range []struct{ expr, want string }{
		{"push-all [3 4] [1 2]", "[1 2 3 4]"},
		{"push-all [] [1]", "[1]"},
		{"push-all [1] []", "[1]"},
		{"push-all [] []", "[]"},
	} {
		if got := mustEval(t, "", c.expr); !Equal(got, mustEval(t, "", c.want)) {
			t.Errorf("%s: %s", c.expr, DebugString(got))
		}
	}
	rt := runtimeOf(t, "def held [1]\ndef added [2]")
	for _, c := range []struct{ expr, want string }{
		{"push-all added held", "[1 2]"},
		{"held", "[1]"},
		{"added", "[2]"},
	} {
		if got, f := evalWith(t, rt, c.expr); f != nil || !Equal(got, mustEval(t, "", c.want)) {
			t.Errorf("%s: %s %v", c.expr, DebugString(got), f)
		}
	}
	for _, c := range []struct{ expr, message string }{
		{"push-all 1 []", "the items must be a vector, not a number"},
		{`push-all [] "a"`, "the data must be a vector, not a string"},
	} {
		if f := mustFail(t, "", c.expr); f.Code != CodeDSLTypeError || f.Message != "type_mismatch: push-all: "+c.message {
			t.Errorf("%s: %v", c.expr, f)
		}
	}
}

// push-all takes a step for each item it appends, and none for the vector
// it extends: ten steps of fuel append ten items and no more.
func TestPushAllTakesAStepPerItemAppended(t *testing.T) {
	at := SourceSpan{File: NewFile("t.alc")}
	run := func(n, held int) (Val, *Fail) {
		return nPushAll(runtimeOf(t, "").WithFuel(10), []Val{numsVal(countTo(n)...), numsVal(countTo(held)...)}, at)
	}
	if _, f := run(10, 1000); f != nil {
		t.Errorf("%v", f)
	}
	if _, f := run(11, 0); f == nil || f.Limit == nil || f.Limit.Name != "max_plan_steps" {
		t.Errorf("%v", f)
	}
}

// inc answers the count one more, exactly up to 2^53 - 1, and refuses a
// count that is not a whole number of at least 0 below it, as repeat
// refuses its count.
func TestIncCountsOneMoreExactly(t *testing.T) {
	for _, c := range []struct {
		expr string
		want float64
	}{
		{"inc 0", 1},
		{"inc -0", 1},
		{"inc 41", 42},
		{"inc 1e2", 101},
		{"inc 9007199254740990", IncMax},
	} {
		got, ok := mustEval(t, "", c.expr).(NumVal)
		if !ok || got.Value != c.want || got.HasLexeme {
			t.Errorf("%s: %#v", c.expr, got)
		}
	}
	at := SourceSpan{File: NewFile("t.alc")}
	for _, c := range []struct {
		arg     Val
		spelled string
	}{
		{Num(IncMax), "9007199254740991"},
		{Num(-1), "-1"},
		{Num(1.5), "1.5"},
		{NumLexeme(0.5, "5e-1"), "5e-1"},
		{Num(math.Inf(1)), "inf"},
		{Num(math.NaN()), "NaN"},
	} {
		_, f := nInc(runtimeOf(t, ""), []Val{c.arg}, at)
		want := "type_mismatch: inc: the count must be a whole number from 0 to 9007199254740990, not " + c.spelled
		if f == nil || f.Code != CodeDSLTypeError || f.Message != want {
			t.Errorf("%s: %v", c.spelled, f)
		}
	}
	if f := mustFail(t, "", `inc "1"`); f.Code != CodeDSLTypeError || f.Message != "type_mismatch: inc: the count must be a number, not a string" {
		t.Errorf("%v", f)
	}
}

// add and sub answer the sum and the difference exactly, without a lexeme
// and never -0, up to 2^53 - 1 either way; an operand that is not a whole
// number in that range, or a result past it, is a type error naming it as
// it is (rs/src/stdlib/registry.rs's test of the same name).
func TestAddAndSubAreExactOverWholeNumbers(t *testing.T) {
	at := SourceSpan{File: NewFile("t.alc")}
	ops := map[string]nativeCall{"add": nAdd, "sub": nSub}
	for _, c := range []struct {
		op   string
		x, y Val
		want float64
	}{
		{"add", Num(2), Num(3), 5},
		{"add", Num(-2), Num(3), 1},
		{"sub", Num(2), Num(3), -1},
		{"sub", Num(3), Num(2), 1},
		{"add", Num(WholeMax - 1), Num(1), WholeMax},
		{"sub", Num(-WholeMax + 1), Num(1), -WholeMax},
		{"add", Num(WholeMax), Num(-WholeMax), 0},
		{"add", NumLexeme(100, "1e2"), Num(1), 101},
		// Zero is 0, whatever the zeros it came from.
		{"add", Num(math.Copysign(0, -1)), Num(math.Copysign(0, -1)), 0},
		{"sub", Num(math.Copysign(0, -1)), Num(0), 0},
		{"sub", Num(5), Num(5), 0},
	} {
		v, f := ops[c.op](runtimeOf(t, ""), []Val{c.x, c.y}, at)
		got, ok := v.(NumVal)
		if f != nil || !ok || got.Value != c.want || got.HasLexeme || (got.Value == 0 && math.Signbit(got.Value)) {
			t.Errorf("%s %s %s: %#v %v", c.op, DebugString(c.x), DebugString(c.y), v, f)
		}
	}
	if got, ok := mustEval(t, "", "sub 3 5").(NumVal); !ok || got.Value != -2 {
		t.Errorf("%#v", got)
	}
	r := "a whole number from -9007199254740991 to 9007199254740991"
	for _, c := range []struct {
		op      string
		x, y    Val
		message string
	}{
		{"add", Num(WholeMax), Num(1), "the sum must be " + r + ", not 9007199254740992"},
		{"add", Num(WholeMax), Num(2), "the sum must be " + r + ", not 9007199254740993"},
		{"add", Num(-WholeMax), Num(-WholeMax), "the sum must be " + r + ", not -18014398509481982"},
		{"sub", Num(-WholeMax), Num(2), "the difference must be " + r + ", not -9007199254740993"},
		{"sub", Num(WholeMax), Num(-1), "the difference must be " + r + ", not 9007199254740992"},
		{"add", Num(WholeMax + 1), Num(0), "the first must be " + r + ", not 9007199254740992"},
		{"sub", Num(0), Num(1.5), "the second must be " + r + ", not 1.5"},
		{"add", NumLexeme(0.5, "5e-1"), Num(0), "the first must be " + r + ", not 5e-1"},
		{"add", Num(math.Inf(1)), Num(0), "the first must be " + r + ", not inf"},
		{"sub", Num(0), Num(math.NaN()), "the second must be " + r + ", not NaN"},
		{"add", StrVal("1"), Num(0), "the first must be a number, not a string"},
		{"sub", Num(0), NullVal{}, "the second must be a number, not null"},
	} {
		_, f := ops[c.op](runtimeOf(t, ""), []Val{c.x, c.y}, at)
		if f == nil || f.Code != CodeDSLTypeError || f.Message != "type_mismatch: "+c.op+": "+c.message {
			t.Errorf("%s: %v", c.message, f)
		}
	}
}

// get-or answers what get answers, or the default where get answers
// missing; it fails where get fails, naming itself.
func TestGetOrAnswersTheDefaultWhereGetAnswersMissing(t *testing.T) {
	rec := "(record (entry :a 1) (entry :m missing) (entry :n null))"
	for _, c := range []struct {
		expr string
		want Val
	}{
		{"get-or :a \"d\" " + rec, Num(1)},
		{"get-or \"a\" \"d\" " + rec, Num(1)},
		{"get-or :n \"d\" " + rec, NullVal{}},
		{"get-or :b \"d\" " + rec, StrVal("d")},
		{"get-or :m \"d\" " + rec, StrVal("d")},
		{"get-or :a \"d\" missing", StrVal("d")},
	} {
		if v := mustEval(t, "", c.expr); !Equal(v, c.want) {
			t.Errorf("%s: %s", c.expr, DebugString(v))
		}
	}
	for _, c := range []struct {
		expr    string
		code    Code
		message string
	}{
		{"get-or 1 \"d\" " + rec, CodeDSLTypeError, "type_mismatch: get-or: the key must be a keyword or a string, not a number"},
		{"get-or :a \"d\" 1", CodeInputInvalid, "get-or: a number has no member \"a\"; an object was expected"},
		{"get-or :a \"d\" each-index", CodeDSLTypeError, "type_mismatch: get-or: a record was expected, not a selector"},
	} {
		if f := mustFail(t, "", c.expr); f.Code != c.code || f.Message != c.message {
			t.Errorf("%s: %v", c.expr, f)
		}
	}
}

// trim takes off both ends the white space ECMAScript's
// String.prototype.trim takes, and nothing else: not U+0085, which
// unicode.IsSpace takes, nor U+180E, which Unicode no longer counts.
func TestTrimTakesTheWhiteSpaceECMAScriptTrims(t *testing.T) {
	at := SourceSpan{File: NewFile("t.alc")}
	trimmed := func(s string) string {
		v, f := nTrim(runtimeOf(t, ""), []Val{StrVal(s)}, at)
		if f != nil {
			t.Fatal(f)
		}
		return string(v.(StrVal))
	}
	space := "\t\n\v\f\r \u00a0\u1680\u2000\u2005\u200a\u2028\u2029\u202f\u205f\u3000\ufeff"
	for _, c := range [][2]string{
		{space + "a b" + space, "a b"},
		{space, ""},
		{"", ""},
		{"a\u3000b", "a\u3000b"},
		{"\u0085a\u0085", "\u0085a\u0085"},
		{"\u180ea\u180e", "\u180ea\u180e"},
		{"\u200ba\u200b", "\u200ba\u200b"},
		{"\u2060a\u2060", "\u2060a\u2060"},
		{"\x00a\x00", "\x00a\x00"},
	} {
		if got := trimmed(c[0]); got != c[1] {
			t.Errorf("%q: %q, want %q", c[0], got, c[1])
		}
	}
	if f := mustFail(t, "", "trim 1"); f.Code != CodeDSLTypeError || f.Message != "type_mismatch: trim: the string must be a string, not a number" {
		t.Errorf("%v", f)
	}
}

// first-split cuts a string at the separator's first occurrence, or
// answers it alone; its separator is held as split's is.
func TestFirstSplitCutsAtTheFirstOccurrence(t *testing.T) {
	at := SourceSpan{File: NewFile("t")}
	for _, c := range []struct {
		separator, s string
		want         []string
	}{
		{"=", "a=b=c", []string{"a", "b=c"}},
		{"=", "=a", []string{"", "a"}},
		{"=", "a=", []string{"a", ""}},
		{"=", "abc", []string{"abc"}},
		{"=", "", []string{""}},
		{"::", "a:b::c::d", []string{"a:b", "c::d"}},
		{" (", "j@x.org (Jo Ann)", []string{"j@x.org", "Jo Ann)"}},
		{"→", "α→β→γ", []string{"α", "β→γ"}},
	} {
		v, f := nFirstSplit(runtimeOf(t, ""), []Val{StrVal(c.separator), StrVal(c.s)}, at)
		want := make([]Val, len(c.want))
		for i, s := range c.want {
			want[i] = StrVal(s)
		}
		if f != nil || !Equal(v, Vector(want...)) {
			t.Errorf("first-split %q %q: %v %v", c.separator, c.s, DebugString(v), f)
		}
	}
	for _, c := range []struct{ expr, message string }{
		{`first-split "" "a"`, "the separator must be a non-empty string, not the empty string"},
		{`first-split 1 "a"`, "the separator must be a string, not a number"},
		{`first-split "." 1`, "the string must be a string, not a number"},
	} {
		if f := mustFail(t, "", c.expr); f.Code != CodeDSLTypeError || f.Message != "type_mismatch: first-split: "+c.message {
			t.Errorf("%s: %v", c.expr, f)
		}
	}
}

// starts-with, ends-with and contains compare exactly, code point by code
// point; an empty part is in every string.
func TestTheStringTestsCompareExactly(t *testing.T) {
	at := SourceSpan{File: NewFile("t")}
	ops := map[string]nativeCall{"starts-with": nStartsWith, "ends-with": nEndsWith, "contains": nContains}
	for _, c := range []struct {
		op, part, s string
		want        bool
	}{
		{"starts-with", "ab", "abc", true},
		{"starts-with", "bc", "abc", false},
		{"starts-with", "", "abc", true},
		{"starts-with", "", "", true},
		{"starts-with", "abcd", "abc", false},
		{"starts-with", "A", "abc", false},
		{"starts-with", "é", "école", true},
		{"starts-with", "e", "école", false},
		{"ends-with", "bc", "abc", true},
		{"ends-with", "ab", "abc", false},
		{"ends-with", "", "abc", true},
		{"ends-with", "😀", "a😀", true},
		{"ends-with", "zabc", "abc", false},
		{"contains", "b", "abc", true},
		{"contains", "abc", "abc", true},
		{"contains", "d", "abc", false},
		{"contains", "", "", true},
		{"contains", "β→", "α→β→γ", true},
		{"contains", "ß", "SS", false},
	} {
		v, f := ops[c.op](runtimeOf(t, ""), []Val{StrVal(c.part), StrVal(c.s)}, at)
		if f != nil || v != BoolVal(c.want) {
			t.Errorf("%s %q %q: %v %v", c.op, c.part, c.s, v, f)
		}
	}
	for _, c := range [][2]string{{"starts-with", "the prefix"}, {"ends-with", "the suffix"}, {"contains", "the part"}} {
		if f := mustFail(t, "", c[0]+` 1 "a"`); f.Code != CodeDSLTypeError || f.Message != "type_mismatch: "+c[0]+": "+c[1]+" must be a string, not a number" {
			t.Errorf("%s: %v", c[0], f)
		}
		if f := mustFail(t, "", c[0]+` "a" null`); f.Code != CodeDSLTypeError || f.Message != "type_mismatch: "+c[0]+": the string must be a string, not null" {
			t.Errorf("%s: %v", c[0], f)
		}
	}
}

// The string natives take a step for each StringStep characters of the
// string, or part of them: ten steps of fuel read ten times that many
// characters and no more, however many bytes a character takes.
func TestTheStringNativesTakeAStepPerCharactersRead(t *testing.T) {
	at := SourceSpan{File: NewFile("t.alc")}
	limit := func(f *Fail) bool { return f != nil && f.Limit != nil && f.Limit.Name == "max_plan_steps" }
	for _, c := range []string{"a", "\U0001F680"} {
		for _, op := range []struct {
			name string
			call nativeCall
			part string
		}{
			{"trim", nTrim, ""},
			{"first-split", nFirstSplit, "."},
			{"starts-with", nStartsWith, "a"},
			{"ends-with", nEndsWith, "a"},
			{"contains", nContains, "."},
		} {
			run := func(n int) *Fail {
				s := StrVal(strings.Repeat(c, n))
				args := []Val{StrVal(op.part), s}
				if op.name == "trim" {
					args = []Val{s}
				}
				_, f := op.call(runtimeOf(t, "").WithFuel(10), args, at)
				return f
			}
			if f := run(10 * StringStep); f != nil {
				t.Errorf("%s %q: %v", op.name, c, f)
			}
			if f := run(10*StringStep + 1); !limit(f) {
				t.Errorf("%s %q: %v", op.name, c, f)
			}
		}
	}
}

// flatten takes one level out: each vector among the items gives its
// items, and any other item stays; a vector inside one of those stays a
// vector.
func TestFlattenTakesOneLevelOut(t *testing.T) {
	for _, c := range []struct {
		expr string
		want Val
	}{
		{"flatten [[1 2] 3 [] [4]]", Vector(Num(1), Num(2), Num(3), Num(4))},
		{"flatten []", Vector()},
		{"flatten [[] []]", Vector()},
		{"flatten [[[1] \"a\"] null]", Vector(Vector(Num(1)), StrVal("a"), NullVal{})},
	} {
		if v := mustEval(t, "", c.expr); !Equal(v, c.want) {
			t.Errorf("%s: %s", c.expr, DebugString(v))
		}
	}
	if f := mustFail(t, "", `flatten "a"`); f.Code != CodeDSLTypeError || f.Message != "type_mismatch: flatten: the data must be a vector, not a string" {
		t.Errorf("%v", f)
	}
	// A step for each item and for each item a vector among them gives: ten
	// steps of fuel take two vectors of four items, and not five.
	at := SourceSpan{File: NewFile("t.alc")}
	nums := func(k int) Val {
		items := make([]Val, k)
		for i := range items {
			items[i] = Num(float64(i))
		}
		return Vector(items...)
	}
	if _, f := nFlatten(runtimeOf(t, "").WithFuel(10), []Val{Vector(nums(4), nums(4))}, at); f != nil {
		t.Errorf("%v", f)
	}
	if _, f := nFlatten(runtimeOf(t, "").WithFuel(10), []Val{Vector(nums(4), nums(5))}, at); f == nil || f.Limit == nil || f.Limit.Name != "max_plan_steps" {
		t.Errorf("%v", f)
	}
}

// sort orders numbers by value and strings by code point, stably; a
// vector of another kind, of both kinds, or holding NaN is refused at the
// first item that is.
func TestSortOrdersNumbersByValueAndStringsByCodePoint(t *testing.T) {
	at := SourceSpan{File: NewFile("t")}
	sorted := func(items ...Val) ([]Val, *Fail) {
		v, f := nSort(runtimeOf(t, ""), []Val{Vector(items...)}, at)
		if f != nil {
			return nil, f
		}
		return v.(*VectorVal).Items, nil
	}
	got, f := sorted(Num(3), Num(-1.5), Num(math.Inf(1)), Num(0), Num(math.Inf(-1)), Num(2))
	if f != nil {
		t.Fatal(f)
	}
	for i, want := range []float64{math.Inf(-1), -1.5, 0, 2, 3, math.Inf(1)} {
		if got[i].(NumVal).Value != want {
			t.Errorf("%d: %s", i, DebugString(got[i]))
		}
	}
	if got, f := sorted(); f != nil || len(got) != 0 {
		t.Errorf("%v %v", got, f)
	}
	// Stable: 0 and -0 are equal and keep their order, and a number keeps
	// its lexeme.
	got, _ = sorted(NumLexeme(0, "0"), NumLexeme(1, "1e0"), NumLexeme(math.Copysign(0, -1), "-0"), NumLexeme(0.5, "5e-1"))
	var lexemes []string
	for _, v := range got {
		lexemes = append(lexemes, v.(NumVal).Lexeme)
	}
	if strings.Join(lexemes, " ") != "0 -0 5e-1 1e0" {
		t.Errorf("%v", lexemes)
	}
	// By code point: é (U+00E9) after z, U+FFFF before 😀 (U+1F600), which
	// UTF-16 code units would put first.
	got, _ = sorted(StrVal("z"), StrVal("é"), StrVal("\U0001F600"), StrVal("\uffff"), StrVal("Z"), StrVal(""), StrVal("a"), StrVal("ab"))
	var texts []string
	for _, v := range got {
		texts = append(texts, string(v.(StrVal)))
	}
	if strings.Join(texts, ",") != ",Z,a,ab,z,é,\uffff,\U0001F600" {
		t.Errorf("%q", texts)
	}
	for _, c := range []struct {
		items   []Val
		message string
	}{
		{[]Val{Num(1), StrVal("a")}, "every item must be a number, as the first is, not a string"},
		{[]Val{StrVal("a"), Num(1)}, "every item must be a string, as the first is, not a number"},
		{[]Val{StrVal("a"), Num(math.NaN())}, "every item must be a string, as the first is, not a number"},
		{[]Val{Num(1), Num(math.NaN())}, "every number must be ordered, not NaN"},
		{[]Val{BoolVal(true)}, "every item must be a number or a string, not a boolean"},
		{[]Val{Num(1), Vector(Num(2))}, "every item must be a number or a string, not a vector"},
		{[]Val{Num(1), Missing()}, "every item must be a number or a string, not missing"},
	} {
		if _, f := sorted(c.items...); f == nil || f.Code != CodeDSLTypeError || f.Message != "type_mismatch: sort: "+c.message {
			t.Errorf("%s: %v", c.message, f)
		}
	}
	if f := mustFail(t, "", `sort "a"`); f.Code != CodeDSLTypeError || f.Message != "type_mismatch: sort: the data must be a vector, not a string" {
		t.Errorf("%v", f)
	}
	// A step for each item: ten steps of fuel sort ten items and not
	// eleven.
	nums := func(k int) Val {
		items := make([]Val, k)
		for i := range items {
			items[i] = Num(float64(k - i))
		}
		return Vector(items...)
	}
	if _, f := nSort(runtimeOf(t, "").WithFuel(10), []Val{nums(10)}, at); f != nil {
		t.Errorf("%v", f)
	}
	if _, f := nSort(runtimeOf(t, "").WithFuel(10), []Val{nums(11)}, at); f == nil || f.Limit == nil || f.Limit.Name != "max_plan_steps" {
		t.Errorf("%v", f)
	}
}

// equal compares as a match pattern compares a constant: data
// structurally, numbers by value, a record's members in any order; a
// function equals nothing, and a stream or a text is refused.
func TestEqualComparesValuesAsAPatternComparesAConstant(t *testing.T) {
	ab := "(record (entry :a 1) (entry :b [2]))"
	for _, c := range []struct {
		a, b string
		want bool
	}{
		{`"FEN"`, `"FEN"`, true},
		{`"FEN"`, `"Fen"`, false},
		{`"1"`, "1", false},
		{"null", "null", true},
		{"null", "missing", false},
		{"missing", "missing", true},
		{":w", ":w", true},
		{":w", `"w"`, false},
		{"true", "true", true},
		{"1e2", "100", true},
		{"-0", "0", true},
		{"[1 2]", "[1 2]", true},
		{"[1 2]", "[2 1]", false},
		{"[1]", "[1 1]", false},
		{ab, "(record (entry :b [2]) (entry :a 1))", true},
		{ab, "(record (entry :a 1) (entry :c [2]))", false},
		{ab, "(record (entry :a 1))", false},
		{`(key "a")`, `(key "a")`, true},
		{`(key "a")`, `(key "b")`, false},
		{`(key "a")`, `(scalar "a")`, false},
		{"object-start", "object-start", true},
		{"object-start", "object-end", false},
		{"[object-start [array-end]]", "[object-start [array-end]]", true},
		{"count", "count", false},
	} {
		expr := "equal " + c.a + " " + c.b
		if got := mustEval(t, "", expr); got != BoolVal(c.want) {
			t.Errorf("%s: %s", expr, DebugString(got))
		}
	}
	at := SourceSpan{File: NewFile("t.alc")}
	nan := Num(math.NaN())
	if v, f := nEqual(runtimeOf(t, ""), []Val{nan, nan}, at); f != nil || v != BoolVal(false) {
		t.Errorf("NaN: %v %v", v, f)
	}
	for _, expr := range []string{`equal (text "x") "x"`, `equal "x" (text "x")`} {
		want := "type_mismatch: equal: a text cannot be compared; a stream or a text is used where it is, not inspected"
		if f := mustFail(t, "", expr); f.Code != CodeDSLTypeError || f.Message != want {
			t.Errorf("%s: %v", expr, f)
		}
	}
}

// equal takes a step for each pair of values it compares, depth first,
// and stops at the first that differs: ten steps of fuel compare two
// vectors of nine equal items, and two that differ at their first item of
// any length.
func TestEqualTakesAStepPerPairCompared(t *testing.T) {
	at := SourceSpan{File: NewFile("t.alc")}
	same := func(n int, x float64) []float64 {
		xs := make([]float64, n)
		for i := range xs {
			xs[i] = x
		}
		return xs
	}
	run := func(a, b []float64) (Val, *Fail) {
		return nEqual(runtimeOf(t, "").WithFuel(10), []Val{numsVal(a...), numsVal(b...)}, at)
	}
	if v, f := run(same(9, 7), same(9, 7)); f != nil || v != BoolVal(true) {
		t.Errorf("nine: %v %v", v, f)
	}
	if _, f := run(same(10, 7), same(10, 7)); f == nil || f.Limit == nil || f.Limit.Name != "max_plan_steps" {
		t.Errorf("ten: %v", f)
	}
	differ := same(100, 7)
	differ[0] = 8
	if v, f := run(differ, same(100, 7)); f != nil || v != BoolVal(false) {
		t.Errorf("differ: %v %v", v, f)
	}
	// Two steps, the vectors and their first items, with fuel for two.
	if v, f := nEqual(runtimeOf(t, "").WithFuel(2), []Val{numsVal(differ...), numsVal(same(100, 7)...)}, at); f != nil || v != BoolVal(false) {
		t.Errorf("wide: %v %v", v, f)
	}
	// A member the other record lacks ends the walk where it reaches that
	// member: the records, a's vectors and their two items, four steps,
	// then b, which the other lacks.
	rec := func(last string) Val {
		return mustEval(t, "", "(record (entry :a [1 2]) (entry :"+last+" 1))")
	}
	records := runtimeOf(t, "").WithFuel(10)
	for i := 0; i < 2; i++ {
		if v, f := nEqual(records, []Val{rec("b"), rec("c")}, at); f != nil || v != BoolVal(false) {
			t.Errorf("records %d: %v %v", i, v, f)
		}
	}
	if _, f := nEqual(records, []Val{rec("b"), rec("c")}, at); f == nil || f.Limit == nil || f.Limit.Name != "max_plan_steps" {
		t.Errorf("records: %v", f)
	}
	// Nested: the vector, its first item's vector and the two items in it,
	// then the second item: five steps.
	nested := func(x float64) Val { return Vector(numsVal(1, 2), Num(x)) }
	rt := runtimeOf(t, "").WithFuel(10)
	for i := 0; i < 2; i++ {
		if v, f := nEqual(rt, []Val{nested(3), nested(3)}, at); f != nil || v != BoolVal(true) {
			t.Errorf("nested %d: %v %v", i, v, f)
		}
	}
	if _, f := nEqual(rt, []Val{nested(3), nested(3)}, at); f == nil || f.Limit == nil || f.Limit.Name != "max_plan_steps" {
		t.Errorf("nested: %v", f)
	}
}

// A CSV options record's :no-columns: :refuse, the default, or :empty; any
// other value is a type error where it is read.
func TestNoColumnsNamesAPolicyForATableOfNoColumns(t *testing.T) {
	for _, c := range []struct {
		v    Val
		want bool
	}{
		{NullVal{}, false},
		{mustEval(t, "", "(record)"), false},
		{mustEval(t, "", "(record (entry :no-columns :refuse))"), false},
		{mustEval(t, "", "(record (entry :no-columns :empty))"), true},
	} {
		if got, f := NoColumnsEmpty(c.v); f != nil || got != c.want {
			t.Errorf("%s: %v %v", DebugString(c.v), got, f)
		}
	}
	for _, c := range []struct{ record, message string }{
		{"(record (entry :no-columns :nope))", "type_mismatch: :no-columns must be :refuse or :empty, not :nope"},
		{`(record (entry :no-columns "empty"))`, "type_mismatch: :no-columns must be :refuse or :empty, not a string"},
	} {
		if _, f := NoColumnsEmpty(mustEval(t, "", c.record)); f == nil || f.Code != CodeDSLTypeError || f.Message != c.message {
			t.Errorf("%s: %v", c.record, f)
		}
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
// hold the layout at its edges. dmog (go/e2e), which has both,
// holds the copy to render's own on real documents.
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

// fail takes a code before the message: one of four keywords, each its
// code (the fourth leading its message with its finer code, as the
// checker's STREAMABILITY_UNKNOWN failures do); any other keyword, or a
// code that is not one, is a type error, as the checker's is for a
// literal keyword.
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
	if f := mustFail(t, src, "boom :unstreamable"); f.Code != CodeStreamabilityUnknown || f.Message != "unstreamable: no" || f.Row != 1 || f.Column != 14 {
		t.Errorf("boom :unstreamable: %v", f)
	}
	for _, c := range []struct{ src, expr, message string }{
		{src, "boom :nope", "type_mismatch: the code of fail must be :invalid, :unrepresentable, :protocol-order or :unstreamable, not :nope"},
		{src, "boom 1", "type_mismatch: fail: the code must be a keyword, not a number"},
		{`def boom [k] (fail k)`, "boom 1", "type_mismatch: fail: the message must be a string, not a number"},
	} {
		if f := mustFail(t, c.src, c.expr); f.Code != CodeDSLTypeError || f.Message != c.message {
			t.Errorf("%s: %v", c.expr, f)
		}
	}
}
