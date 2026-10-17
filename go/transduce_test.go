// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// transduce_test.go: the declarative transducer (stdlib/transduce.alc)
// over tabnas-json's own rule passes, as rs/src/lower.rs's test runs it.
// The scan runs here by hand, the step applied to each item as
// transduce's scan-emit applies it, since running the plan itself needs
// transduce's stages.

import (
	"sort"
	"strings"
	"testing"

	"github.com/tabnas/alchemy/go/shared"
	tabnasjson "github.com/tabnas/json/go"
	tabnas "github.com/tabnas/parser/go"
)

// jsonPasses are tabnas-json's rule passes for src, as a parser-events
// source makes them: each pass's own tokens (those its alternate gave
// back are the next rule's), a number token's value with its source text
// as the lexeme, a close pass's scalar node as its value, then one end.
func jsonPasses(t *testing.T, src string) []shared.ParserEvent {
	t.Helper()
	datum := func(v any) shared.Datum {
		switch x := v.(type) {
		case bool:
			return shared.BoolDatum(x)
		case float64:
			return shared.NumberDatum(x)
		case string:
			return shared.StringDatum(x)
		}
		return shared.NullDatum()
	}
	var out []shared.ParserEvent
	parser := tabnasjson.Make()
	parser.SubRuleDone(func(r *tabnas.Rule, ctx *tabnas.Context, done tabnas.RuleDone) {
		open := done.State == tabnas.OPEN
		all := r.C[:r.CN]
		if open {
			all = r.O[:r.ON]
		}
		b := 0
		if done.Alt != nil {
			b = done.Alt.B
		}
		pass := &shared.RulePass{Name: r.Name, D: uint64(r.D), I: uint64(r.I), Forced: done.Forced, Tokens: []shared.PassToken{}}
		// An alternate may give back a token it did not record (a close
		// that only backs up), so the trim stops at none.
		keep := len(all) - b
		if keep < 0 {
			keep = 0
		}
		for _, tok := range all[:keep] {
			val := datum(tok.ResolveVal(r, ctx))
			if f, ok := tok.ResolveVal(r, ctx).(float64); ok && tok.Name == "#NR" {
				val = shared.NumberDatumLexeme(f, tok.Src)
			}
			pass.Tokens = append(pass.Tokens, shared.PassToken{Name: tok.Name, Val: val, Src: tok.Src, Row: uint64(tok.RI), Col: uint64(tok.CI)})
		}
		if done.Alt != nil {
			pass.Alt = &shared.PassAlt{P: done.Alt.P, R: done.Alt.R, B: uint64(done.Alt.B), G: done.Alt.G}
		}
		var names []string
		for k := range r.N {
			names = append(names, k)
		}
		sort.Strings(names)
		for _, k := range names {
			pass.N = append(pass.N, shared.PassCounter{Name: k, Value: int64(r.N[k])})
		}
		kind := shared.ParserOpen
		if !open {
			kind = shared.ParserClose
			switch r.Node.(type) {
			case nil, bool, float64, string:
				v := datum(r.Node)
				pass.Value = &v
			}
		}
		out = append(out, shared.ParserEvent{Kind: kind, Pass: pass})
	})
	if _, err := parser.Parse(src); err != nil {
		t.Fatalf("%s: %v", src, err)
	}
	return append(out, shared.ParserEvent{Kind: shared.ParserEnd})
}

// treeScan is the scan a transduce-tree builds, run here by hand over
// tabnas-json's own rule passes: step applied to each item as transduce's
// scan-emit applies it, from init, and finish to the last state, since
// running the plan itself needs transduce's stages (rs/src/lower.rs's
// tree_scan). The outputs, each as the value prints.
func treeScan(t *testing.T, rt *Runtime, specName, stepName, finishName string, init Val, src string) string {
	t.Helper()
	spec, f := rt.DefValue(ScopeProgram, specName)
	if f != nil {
		t.Fatal(f)
	}
	stepVal, _ := rt.DefValue(ScopeStdlib, stepName)
	finishVal, _ := rt.DefValue(ScopeStdlib, finishName)
	step, finish := stepVal.(Fn), finishVal.(Fn)
	at := SourceSpan{File: NewFile("t")}
	state := init
	var out []string
	var item Val
	items := &parserEventsToItems{down: itemFunc(func(v Val) { item = v })}
	for _, ev := range jsonPasses(t, src) {
		if ev.Kind == shared.ParserEnd {
			tail, f := rt.Apply(finish, []Val{state}, at)
			if f != nil {
				t.Fatalf("%s: %v", src, f)
			}
			for _, v := range tail.(*VectorVal).Items {
				out = append(out, DebugString(v))
			}
			break
		}
		if _, f := items.ParserEvent(ev); f != nil {
			t.Fatal(f)
		}
		next, f := rt.Apply(step, []Val{spec, state, item}, at)
		if f != nil {
			t.Fatalf("%s: %v", src, f)
		}
		tr := next.(*TaggedVal)
		if tr.Tag != "transition" {
			t.Fatalf("the step answers %s", tr.Tag)
		}
		for _, v := range tr.Fields[1].(*VectorVal).Items {
			out = append(out, DebugString(v))
		}
		state = tr.Fields[0]
	}
	return strings.Join(out, " ")
}

// The json grammar's part, as the plan writes it: transduce-tree over
// tabnas-json's own rule passes yields the document's events, lexemes
// kept, in the order a walk of its value gives them, through
// transduce-step and through transduce-tree's own step alike.
func TestTransduceTreeMakesJSONEventsOfTabnasJSONRulePasses(t *testing.T) {
	rt := runtimeOf(t, "def json-spec\n  record\n    entry :map (record (entry :container :object))\n    entry :list (record (entry :container :array))\n    entry :pair (record (entry :key 0))\n    entry :val (record (entry :scalar 0))\n\ndef export [input] (transduce-tree json-spec input)\n")
	tree := func() Val { return &VectorVal{Items: []Val{&VectorVal{}, BoolVal(false)}} }
	steps := map[string]func(string) string{
		"transduce-step": func(src string) string {
			return treeScan(t, rt, "json-spec", "transduce-step", "transduce-finish", tree(), src)
		},
		"transduce-pass": func(src string) string {
			return treeScan(t, rt, "json-spec", "transduce-pass", "transduce-done", &VectorVal{Items: []Val{tree(), &VectorVal{}}}, src)
		},
	}
	for name, events := range steps {
		for _, c := range []struct{ src, want string }{
			{`{"a":[1,2.50,"x",{"b":null}],"c":{},"d":[],"e":1e21,"f":true}`, `object-start (key "a") array-start (scalar 1) (scalar 2.50) (scalar "x") object-start (key "b") (scalar null) object-end array-end (key "c") object-start object-end (key "d") array-start array-end (key "e") (scalar 1e21) (key "f") (scalar true) object-end`},
			{"7", "(scalar 7)"},
			{`"s"`, `(scalar "s")`},
			{"[]", "array-start array-end"},
			{"[[[{}]]]", "array-start array-start array-start object-start object-end array-end array-end array-end"},
		} {
			if got := events(c.src); got != c.want {
				t.Errorf("%s, %s:\n  got  %s\n  want %s", name, c.src, got, c.want)
			}
		}
	}
}

// transduce-tree's own step reads :member and :text: a member after the
// container its rule begins, and a :text rule's text, the :src of the
// tokens the passes between its open and its close consumed, first on its
// close; a :text rule closed inside another joins its text to the one
// around it, with its own passes' tokens (rs/src/lower.rs's test).
func TestTransduceTreeReadsAMemberAndAText(t *testing.T) {
	rt := runtimeOf(t, "def values\n  record\n    entry :map (record (entry :container :object))\n    entry :pair (record (entry :key 0) (entry :text true))\n\ndef lists\n  record\n    entry :list (record (entry :container :object) (entry :member \"items\") (entry :text true))\n\ndef export [input] (transduce-tree values input)\n")
	init := func() Val {
		return &VectorVal{Items: []Val{&VectorVal{Items: []Val{&VectorVal{}, BoolVal(false)}}, &VectorVal{}}}
	}
	for _, c := range []struct{ spec, src, want string }{
		{"values", `{"a":[1, 2],"b":"x","d":7}`, `object-start (key "a") (scalar "[1,2]") (key "b") (scalar "\"x\"") (key "d") (scalar "7") object-end`},
		{"lists", `[1,[2,3],"x"]`, `object-start (key "items") object-start (key "items") (scalar "2,3") object-end (scalar "1,[2,3],\"x\"") object-end`},
		{"lists", "[]", `object-start (key "items") (scalar "") object-end`},
	} {
		if got := treeScan(t, rt, c.spec, "transduce-pass", "transduce-done", init(), c.src); got != c.want {
			t.Errorf("%s, %s:\n  got  %s\n  want %s", c.spec, c.src, got, c.want)
		}
	}
}

// itemFunc is an item sink made of a function, for one item at a time.
type itemFunc func(v Val)

func (f itemFunc) Item(v Val) (shared.Flow, *Fail) {
	f(v)
	return shared.Continue, nil
}

func (f itemFunc) End() (shared.Flow, *Fail) { return shared.Continue, nil }
