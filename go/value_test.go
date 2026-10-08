// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// value_test.go: runtime values and plans (rs/src/value.rs's tests).

import (
	"testing"

	"github.com/tabnas/alchemy/go/shared"
)

func TestADatumRoundTripsWithItsLexemes(t *testing.T) {
	d, err := shared.DatumFromJSON(`{"a": [1, "x", null, true], "b": {"c": 2.5}}`)
	if err != nil {
		t.Fatal(err)
	}
	v := FromDatum(&d)
	back, f := ToDatum(v)
	if f != nil || !back.Equal(d) {
		t.Fatalf("%v %v", back, f)
	}
	big := shared.NumberDatumLexeme(1.5, "1.50")
	if text, f := JSONText(FromDatum(&big)); f != nil || text != "1.50" {
		t.Errorf("%q %v", text, f)
	}
	if text, f := JSONText(v); f != nil || text != `{"a":[1,"x",null,true],"b":{"c":2.5}}` {
		t.Errorf("%q %v", text, f)
	}
}

func TestGetPathWalksRecordsAndVectorsAndAnswersMissing(t *testing.T) {
	d, err := shared.DatumFromJSON(`{"a": [{"b": 1}]}`)
	if err != nil {
		t.Fatal(err)
	}
	v := FromDatum(&d)
	p := []shared.Segment{shared.KeySegment("a"), shared.IndexSegment(0), shared.KeySegment("b")}
	if got := GetPath(v, p); !Equal(got, Num(1)) {
		t.Errorf("%s", DebugString(got))
	}
	if !IsMissing(GetPath(v, []shared.Segment{shared.KeySegment("z")})) {
		t.Error("an absent key is not missing")
	}
	if !IsMissing(GetPath(v, []shared.Segment{shared.IndexSegment(0)})) {
		t.Error("an index into a record is not missing")
	}
	if got := GetPath(v, nil); !Equal(got, v) {
		t.Error("the empty path is not the value")
	}
	if z, ok := Field(v, "z"); !ok || !IsMissing(z) {
		t.Error("an absent field is not missing")
	}
	if _, ok := Field(NullVal{}, "z"); ok {
		t.Error("null has fields")
	}
}

func TestEnvironmentsShadowInnermostFirst(t *testing.T) {
	env := Env{}.Bind("x", Num(1))
	inner := env.Bind("x", Num(2))
	if v, _ := env.Get("x"); !Equal(v, Num(1)) {
		t.Error("the outer binding changed")
	}
	if v, _ := inner.Get("x"); !Equal(v, Num(2)) {
		t.Error("the inner binding does not shadow")
	}
	if inner.Has("y") {
		t.Error("y is bound")
	}
}

func TestLivenessFollowsTheInput(t *testing.T) {
	live := &Plan{Kind: PlanMap, F: LookupNative("text"), Source: inputPlan(), At: SourceSpan{File: NewFile("t")}}
	if !live.IsLive() || live.Protocol() != ProtocolItems {
		t.Error("a map over the input")
	}
	finite := newConcat([]Val{StrVal("a"), TextVal{Plan: litPlan("b")}})
	if finite.IsLive() || !finite.IsText() {
		t.Error("a concat of literals")
	}
	mixed := newConcat([]Val{StrVal("a"), TextVal{Plan: live}})
	if !mixed.IsLive() {
		t.Error("a concat around a live text")
	}
	if inputPlan().Protocol() != ProtocolJSONEvents {
		t.Error("the input's protocol")
	}
}

func TestSelectorSegmentsNameOneLocationOnly(t *testing.T) {
	one := shared.Root().Property("a").Index(2)
	segments, ok := selectorSegments(one)
	if !ok || len(segments) != 2 || segments[0] != shared.KeySegment("a") || segments[1] != shared.IndexSegment(2) {
		t.Errorf("%v %v", segments, ok)
	}
	if _, ok := selectorSegments(shared.Root().EachIndex()); ok {
		t.Error("each-index names one location")
	}
}

func TestEqualityIsStructuralForDataAndFalseForResources(t *testing.T) {
	if !Equal(NewTagged("table-end"), NewTagged("table-end")) {
		t.Error("two table-ends differ")
	}
	if Equal(NewTagged("table-end"), NewTagged("no-schema")) {
		t.Error("table-end is no-schema")
	}
	if !Equal(NumLexeme(1, "1.0"), Num(1)) {
		t.Error("numbers compare by value")
	}
	if Equal(StrVal("a"), KeywordVal("a")) {
		t.Error("a string is a keyword")
	}
	text := TextVal{Plan: litPlan("x")}
	if Equal(text, text) {
		t.Error("a text equals itself")
	}
	a, b := NewRecord(), NewRecord()
	a.Set("x", Num(1))
	a.Set("y", Num(2))
	b.Set("y", Num(2))
	b.Set("x", Num(1))
	if !Equal(a, b) {
		t.Error("records compare in any order")
	}
}

// A record keeps its order and replaces a repeated key in its place, past
// the size at which it indexes its keys too.
func TestARecordKeepsItsOrderAndReplacesInPlace(t *testing.T) {
	r := NewRecord()
	for i := 0; i < 2*recordIndexAt; i++ {
		if r.Set(string(rune('a'+i)), Num(float64(i))) {
			t.Fatalf("%d replaced", i)
		}
	}
	if !r.Set("c", StrVal("again")) {
		t.Error("a repeated key was not replaced")
	}
	if v, _ := r.Get("c"); !Equal(v, StrVal("again")) || r.Keys()[2] != "c" || r.Len() != 2*recordIndexAt {
		t.Errorf("%s", DebugString(r))
	}
	if v, _ := r.Get(string(rune('a' + 2*recordIndexAt - 1))); !Equal(v, Num(float64(2*recordIndexAt-1))) {
		t.Error("the last key is not found")
	}
}
