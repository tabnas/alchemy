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
	for _, s := range []string{"", "plain", "q\" \\ \n \x1f é", "del\u007f pad\u0080 日本 🚀"} {
		if quotedLen(s) != len(quote(s)) {
			t.Errorf("%q: %d, built %d", s, quotedLen(s), len(quote(s)))
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
