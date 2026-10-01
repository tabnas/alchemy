// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// debug_model_test.go: the alchemy grammar plugin layered with the
// tabnas-debug introspection plugin, as every grammar in the fleet carries
// (rs/tests/debug_model_test.rs).

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"testing"

	debug "github.com/tabnas/debug/go"
	tabnas "github.com/tabnas/parser/go"
)

// debugBuild is an alchemy instance with the debug plugin layered on top,
// quiet: introspection only, no USE dump and no tracing.
func debugBuild(t *testing.T) *tabnas.Tabnas {
	t.Helper()
	j := Make()
	if err := j.Use(debug.Debug, map[string]any{"print": false}); err != nil {
		t.Fatalf("the debug plugin installs over alchemy: %v", err)
	}
	return j
}

func TestParsesNormallyWithTheDebugPluginInstalled(t *testing.T) {
	j := debugBuild(t)
	v, err := j.Parse("def x [a]\n  f a 1")
	if err != nil {
		t.Fatal(err)
	}
	got := valueJSON(plain(v))
	for _, part := range []string{`[{"$":"list","items":[{"$":"sym","name":"def"`, `{"$":"list","items":[{"$":"sym","name":"f"`} {
		if !strings.Contains(got, part) {
			t.Errorf("%s", got)
		}
	}
}

func TestTheModelIsTheStructuredAlchemyGrammar(t *testing.T) {
	m, err := debug.Model(debugBuild(t))
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, r := range m.Rules {
		names = append(names, r.Name)
	}
	sort.Strings(names)
	if fmt.Sprint(names) != "[block bracket form line paren program]" {
		t.Errorf("%v", names)
	}
	if m.Config.Start != "program" {
		t.Errorf("start %q", m.Config.Start)
	}
	// The layout matcher is the one custom lexer entry, below the first
	// built-in band.
	found := false
	for _, l := range m.Lexer {
		if l.Matcher == "alchemy" {
			found = true
			if l.Order >= 1_000_000 {
				t.Errorf("order %d is not below the bands", l.Order)
			}
		}
	}
	if !found {
		t.Errorf("the layout matcher is not listed: %+v", m.Lexer)
	}
	edge := func(name string) debug.DebugRuleEdges {
		for _, e := range m.Graph {
			if e.Name == name {
				return e
			}
		}
		t.Fatalf("no edge entry for %s", name)
		return debug.DebugRuleEdges{}
	}
	eq := func(what string, got []string, want ...string) {
		t.Helper()
		g := append([]string(nil), got...)
		sort.Strings(g)
		if strings.Join(g, " ") != strings.Join(want, " ") {
			t.Errorf("%s: %v, want %v", what, got, want)
		}
	}
	eq("program open push", edge("program").OpenPush, "line")
	eq("program close push", edge("program").ClosePush)
	eq("line open push", edge("line").OpenPush, "form")
	eq("line close push", edge("line").ClosePush, "block")
	eq("line close replace", edge("line").CloseReplace, "line")
	eq("block open push", edge("block").OpenPush, "line")
	eq("block close push", edge("block").ClosePush)
	eq("form open push", edge("form").OpenPush, "bracket", "paren")
	eq("form close replace", edge("form").CloseReplace, "form")
	for _, seq := range []string{"paren", "bracket"} {
		eq(seq+" open push", edge(seq).OpenPush, "form")
		eq(seq+" close push", edge(seq).ClosePush)
	}
	for _, e := range m.Graph {
		eq(e.Name+" open replace", e.OpenReplace)
	}
	tokens := map[string]bool{}
	for _, tk := range m.Tokens {
		tokens[tk.Name] = true
	}
	for _, name := range []string{"#IN", "#DE", "#NL", "#KW", "#OP", "#CP"} {
		if !tokens[name] {
			t.Errorf("token %s is not registered", name)
		}
	}
}

// The grammar portion of the model, serialised as a tool would receive it
// and read back, still describes this grammar.
func TestTheGrammarPortionSerialisesAndReadsBackWithItsContent(t *testing.T) {
	m, err := debug.Model(debugBuild(t))
	if err != nil {
		t.Fatal(err)
	}
	text, err := json.Marshal(map[string]any{
		"tokens": m.Tokens, "rules": m.Rules, "graph": m.Graph, "config": m.Config, "abnf": m.Abnf,
	})
	if err != nil {
		t.Fatal(err)
	}
	var back struct {
		Tokens []debug.DebugTokenInfo `json:"tokens"`
		Rules  []debug.DebugRuleInfo  `json:"rules"`
		Config debug.DebugConfigInfo  `json:"config"`
		Abnf   string                 `json:"abnf"`
	}
	if err := json.Unmarshal(text, &back); err != nil {
		t.Fatal(err)
	}
	var line *debug.DebugRuleInfo
	for i := range back.Rules {
		if back.Rules[i].Name == "line" {
			line = &back.Rules[i]
		}
	}
	if line == nil {
		t.Fatal("no line rule")
	}
	// A line opens on a form; it closes on #IN (pushing its block), on #NL
	// (replacing itself with the next line), on #DE or #ZZ left for the
	// rule above, or, on the condition that it took a block, on the next
	// line's first token (replacing itself again).
	alts := func(list []debug.DebugAltInfo) string {
		var out []string
		for _, a := range list {
			out = append(out, fmt.Sprintf("%v|%s|%s|%d|%v", a.Seq, a.Push, a.Replace, a.Back, a.Cond))
		}
		return strings.Join(out, " ; ")
	}
	if got := alts(line.Open); got != "[]|form||0|false" {
		t.Errorf("open: %s", got)
	}
	if got := alts(line.Close); got != "[#IN]|block||0|false ; [#NL]||line|0|false ; [#DE]|||1|false ; [#ZZ]|||1|false ; []||line|0|true" {
		t.Errorf("close: %s", got)
	}
	// The layout matcher's tokens are registered, the delimiters are the
	// only fixed tokens, and the engine lexes strings and comments but
	// leaves words to the matcher.
	fixed := map[string]string{}
	for _, tk := range back.Tokens {
		if tk.Fixed != "" {
			fixed[tk.Name] = tk.Fixed
		}
	}
	if fmt.Sprint(fixed) != "map[#CP:) #CS:] #OP:( #OS:[]" {
		t.Errorf("fixed tokens %v", fixed)
	}
	if back.Config.Start != "program" {
		t.Errorf("start %q", back.Config.Start)
	}
	wantLex := map[string]bool{"fixed": true, "space": true, "line": true, "text": false,
		"number": false, "comment": true, "string": true, "value": false}
	if fmt.Sprint(back.Config.Lex) != fmt.Sprint(wantLex) {
		t.Errorf("lex %v", back.Config.Lex)
	}
	// The block is optional: `line = form [ IN block ]`, or the older
	// rendering, `line = form IN block`.
	if !strings.Contains(back.Abnf, "line = form [ IN block ]") && !strings.Contains(back.Abnf, "line = form IN block") {
		t.Errorf("%s", back.Abnf)
	}
}
