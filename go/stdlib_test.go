// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// stdlib_test.go: the natives' table and the embedded library
// (rs/src/stdlib/registry.rs's and rs/src/stdlib/mod.rs's tests), and the
// reference page held to both.

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestEveryNativeIsFoundByNameAndNamesAreUnique(t *testing.T) {
	seen := map[string]bool{}
	for i := range natives {
		n := &natives[i]
		if seen[n.Name] {
			t.Errorf("%s twice", n.Name)
		}
		seen[n.Name] = true
		if LookupNative(n.Name) != n {
			t.Errorf("%s is not found by name", n.Name)
		}
		if !strings.HasPrefix(n.Signature, n.Name) {
			t.Errorf("%s: %s", n.Name, n.Signature)
		}
	}
	if LookupNative("nope") != nil {
		t.Error("nope is a native")
	}
	if len(Natives()) != 66 {
		t.Errorf("%d natives", len(Natives()))
	}
}

func TestConstantsTakeNoArgumentsAndConstructorsTagWithTheirName(t *testing.T) {
	for _, n := range natives {
		switch n.Kind {
		case KindConstant:
			if k, ok := n.Arity.Exact(); !ok || k != 0 {
				t.Errorf("%s: %s", n.Name, n.Arity)
			}
		case KindConstructor:
			if !n.Arity.Accepts(1) && !n.Arity.Accepts(2) {
				t.Errorf("%s: %s", n.Name, n.Arity)
			}
		}
	}
	for _, c := range []struct {
		a    Arity
		want string
	}{{exact(2), "2"}, {atLeast(1), "at least 1"}, {between(2, 3), "2 to 3"}} {
		if c.a.String() != c.want {
			t.Errorf("%s", c.a)
		}
	}
}

// Signature and effect are what the reference prints: every native with a
// row of its own in docs/language.md's table reads there as it does here.
func TestANativeReadsAsItsReferenceRow(t *testing.T) {
	doc, err := os.ReadFile(filepath.Join(repoRoot(t), "docs", "language.md"))
	if err != nil {
		t.Fatal(err)
	}
	spans := func(cell string) (string, bool) {
		if len(cell) < 2 || cell[0] != '`' || cell[len(cell)-1] != '`' {
			return "", false
		}
		inner := cell[1 : len(cell)-1]
		if strings.Contains(inner, "`") {
			return "", false
		}
		return inner, true
	}
	rows := map[string][2]string{}
	for _, line := range strings.Split(string(doc), "\n") {
		if !strings.HasPrefix(line, "| ") || !strings.HasSuffix(line, " |") {
			continue
		}
		cells := strings.Split(line[2:len(line)-2], " | ")
		if len(cells) != 3 {
			continue
		}
		name, ok1 := spans(cells[0])
		signature, ok2 := spans(cells[1])
		if ok1 && ok2 {
			rows[name] = [2]string{signature, strings.ReplaceAll(cells[2], "`", "")}
		}
	}
	compared := 0
	for _, n := range natives {
		if row, ok := rows[n.Name]; ok {
			if n.Signature != row[0] {
				t.Errorf("the signature of %s: %q, the page %q", n.Name, n.Signature, row[0])
			}
			if n.Effect != row[1] {
				t.Errorf("the effect of %s: %q, the page %q", n.Name, n.Effect, row[1])
			}
			compared++
		}
	}
	if compared != 48 {
		t.Errorf("%d natives compared", compared)
	}
}

// The library loads, with the definitions the differential test runs
// interpreted (alchemy-cli's go/e2e/differential_test.go, which runs it
// against the native compositions on transduce's and render's stages).
func TestTheLibraryLoads(t *testing.T) {
	lib := StdlibLoaded()
	if lib.Get("table-from-json") == nil || lib.Get("csv") == nil {
		t.Error("the library lacks table-from-json or csv")
	}
}

func TestTheLibraryLoadsWithTheSpecDefinitions(t *testing.T) {
	lib := StdlibLoaded()
	want := "[public-column table-inferred-column table-row table-positional-column table-value-column table-inferred-columns table-first-row table-step table-finish table-finish-for table-captures table-from-json csv-options csv-field csv-row csv wrap-object-close wrap-object-step wrap-finish wrap-object wrap-array-close wrap-array-step wrap-array]"
	if got := fmt.Sprint(lib.Names()); got != want {
		t.Errorf("%s", got)
	}
	if len(lib.Files) != 3 {
		t.Errorf("%d files", len(lib.Files))
	}
	if p, ok := lib.Get("table-from-json").Params(); !ok || fmt.Sprint(p) != "[binding input]" {
		t.Errorf("%v", p)
	}
	if _, ok := lib.Get("csv-options").Params(); ok {
		t.Error("csv-options has params")
	}
}

func TestLibraryNamesAndNativesAreOuterToAProgram(t *testing.T) {
	for _, c := range []struct {
		name string
		want NameKind
	}{
		{"csv", NameValue}, {"map", NameFunction}, {"table-end", NameConstant}, {"selected", NameConstructor}, {"nope", NameNone},
	} {
		if got := Outer(c.name); got != c.want {
			t.Errorf("%s: %d", c.name, got)
		}
	}
	if src, ok := StdlibSource("stdlib/csv.alc"); !ok || !strings.Contains(src, "def csv-options") {
		t.Error("stdlib/csv.alc")
	}
	if _, ok := StdlibSource("nope"); ok {
		t.Error("nope is a library file")
	}
}

// A span of a library file is known by identity, not by name: a program
// named like a library file is not one.
func TestALibrarySpanIsKnownByIdentity(t *testing.T) {
	lib := StdlibLoaded()
	span := lib.Get("csv").Span
	if name, src, ok := stdlibFileOf(span); !ok || name != "stdlib/csv.alc" || !strings.Contains(src, "def csv [options events]") {
		t.Errorf("%s %v", name, ok)
	}
	if _, _, ok := stdlibFileOf(SourceSpan{File: NewFile("stdlib/csv.alc")}); ok {
		t.Error("a program named stdlib/csv.alc is taken for the library")
	}
}

// Every definition of the standard library appears on the reference page
// as it is in stdlib/*.alc, word for word.
func TestTheLibraryDefinitionsOnThePageAreTheLibraryText(t *testing.T) {
	doc, err := os.ReadFile(filepath.Join(repoRoot(t), "docs", "language.md"))
	if err != nil {
		t.Fatal(err)
	}
	shown := 0
	for _, file := range StdlibFiles {
		src, _ := StdlibSource(file)
		var defs [][]string
		for _, line := range strings.Split(strings.TrimSuffix(src, "\n"), "\n") {
			switch {
			case strings.HasPrefix(line, "def "):
				defs = append(defs, []string{line})
			case strings.TrimSpace(line) == "" || strings.HasPrefix(line, ";"):
				if n := len(defs); n > 0 && len(defs[n-1]) > 0 && defs[n-1][len(defs[n-1])-1] != "" {
					defs = append(defs, nil)
				}
			default:
				if n := len(defs); n > 0 {
					defs[n-1] = append(defs[n-1], line)
				}
			}
		}
		for _, def := range defs {
			if len(def) == 0 {
				continue
			}
			block := "```alchemy\n" + strings.Join(def, "\n") + "\n```"
			if !strings.Contains(string(doc), block) {
				t.Errorf("%s: docs/language.md does not show %q as the library has it", file, def[0])
			}
			shown++
		}
	}
	if shown != len(StdlibLoaded().Names()) {
		t.Errorf("%d shown", shown)
	}
}

// The inferred table's columns come from the first row of any kind
// (stdlib/table.alc): an object's members by name, an array's cells by
// position (indices), a scalar as the one column `value`; a later row of
// another kind projects through those sources, missing where a path does
// not apply to it.
func TestTheInferredColumnsTakeEveryRowKind(t *testing.T) {
	labels := func(row string) Val {
		return mustEval(t, "", "map (fn [c] (get :label c)) (table-inferred-columns "+row+")")
	}
	for _, c := range []struct {
		row  string
		want Val
	}{
		{"(record (entry :a 1) (entry :b 2))", Vector(StrVal("a"), StrVal("b"))},
		{"[10 20 30]", Vector(StrVal("0"), StrVal("1"), StrVal("2"))},
		{`"x"`, Vector(StrVal("value"))},
		{"[]", Vector()},
	} {
		if got := labels(c.row); !Equal(got, c.want) {
			t.Errorf("%s: %s", c.row, DebugString(got))
		}
	}
	for _, c := range []struct {
		first, later string
		want         Val
	}{
		{"[10 20]", "[30 40]", NewTagged("row", Vector(Num(30), Num(40)))},
		{"5", `"six"`, NewTagged("row", Vector(StrVal("six")))},
		{"(record (entry :a 1))", "(record (entry :a 2) (entry :b 3))", NewTagged("row", Vector(Num(2)))},
		{"[10 20]", "(record (entry :a 1))", NewTagged("row", Vector(Missing(), Missing()))},
	} {
		if got := mustEval(t, "", "table-row (table-inferred-columns "+c.first+") "+c.later); !Equal(got, c.want) {
			t.Errorf("%s then %s: %s", c.first, c.later, DebugString(got))
		}
	}
	// The first row binds the columns and writes the schema before itself.
	got := mustEval(t, "", "match (table-first-row (record (entry :columns :infer)) no-schema [10 20]) (case (transition s out) out)")
	want := mustEval(t, "", `[(schema [(record (entry :label "0")) (record (entry :label "1"))]) (row [10 20])]`)
	if !Equal(got, want) {
		t.Errorf("%s", DebugString(got))
	}
}

// The root adapters (stdlib/root.alc), driven event by event as scan-emit
// drives them: each decides at the first event, passes a root of the kind
// its target needs through, wraps any other in one, and refuses events
// that are no tree's (PROTOCOL_ORDER_ERROR).
func TestTheRootAdaptersWrapTheRootATargetNeeds(t *testing.T) {
	rt := runtimeOf(t, "")
	at := SourceSpan{File: NewFile("t")}
	eval := func(expr string) Val {
		t.Helper()
		v, f := evalWith(t, rt, expr)
		if f != nil {
			t.Fatalf("%s: %v", expr, f)
		}
		return v
	}
	items := func(v Val) []Val {
		t.Helper()
		x, ok := v.(*VectorVal)
		if !ok {
			t.Fatalf("%s is not a vector", DebugString(v))
		}
		return x.Items
	}
	drive := func(step, events string) (Val, *Fail) {
		t.Helper()
		s, ok := eval(step).(Fn)
		if !ok {
			t.Fatalf("%s is not a function", step)
		}
		finish := eval("wrap-finish").(Fn)
		state := eval("[:start]")
		var out []Val
		for _, event := range items(eval(events)) {
			v, f := rt.Apply(s, []Val{state, event}, at)
			if f != nil {
				return nil, f
			}
			tr, ok := v.(*TaggedVal)
			if !ok || tr.Tag != "transition" {
				t.Fatalf("%s is not a transition", DebugString(v))
			}
			state = tr.Fields[0]
			out = append(out, items(tr.Fields[1])...)
		}
		last, f := rt.Apply(finish, []Val{state}, at)
		if f != nil {
			return nil, f
		}
		return Vector(append(out, items(last)...)...), nil
	}
	object, array := `partial wrap-object-step "doc"`, "wrap-array-step"
	for _, c := range []struct{ step, events, want string }{
		// An object root passes through; an array or a scalar is the
		// member `doc` of an object, closed when the wrapped root closes.
		{object, `[object-start (key "a") (scalar 1) object-end]`, `[object-start (key "a") (scalar 1) object-end]`},
		{object, "[array-start array-start array-end (scalar 1) array-end]", `[object-start (key "doc") array-start array-start array-end (scalar 1) array-end object-end]`},
		{object, `[(scalar "x")]`, `[object-start (key "doc") (scalar "x") object-end]`},
		// An array root passes through; an object or a scalar is the one
		// element of an array.
		{array, "[array-start (scalar 1) array-end]", "[array-start (scalar 1) array-end]"},
		{array, `[object-start (key "a") object-start object-end object-end]`, `[array-start object-start (key "a") object-start object-end object-end array-end]`},
		{array, "[(scalar null)]", "[array-start (scalar null) array-end]"},
	} {
		got, f := drive(c.step, c.events)
		if want := eval(c.want); f != nil || !Equal(got, want) {
			t.Errorf("%s %s: %v %v", c.step, c.events, DebugString(got), f)
		}
	}
	// A failure in the library names its form there.
	for _, c := range []struct{ step, events, message string }{
		{object, "[object-end]", "the events begin with an end or a key, which a tree's never do (at stdlib/root.alc:36:16)"},
		{array, `[(key "a")]`, "the events begin with an end or a key, which a tree's never do (at stdlib/root.alc:72:16)"},
		{object, "[]", "the events hold no value, where a tree's hold one (at stdlib/root.alc:49:19)"},
		{array, `[object-start (key "a")]`, "the events ended inside a container, which a tree's never do (at stdlib/root.alc:52:12)"},
		{object, "[(scalar 1) (scalar 2)]", "the events hold more after the root value, which a tree's never do (at stdlib/root.alc:45:12)"},
		{array, "[object-start object-end array-start]", "the events hold more after the root value, which a tree's never do (at stdlib/root.alc:81:12)"},
	} {
		if _, f := drive(c.step, c.events); f == nil || f.Code != CodeProtocolOrderError || f.Message != c.message {
			t.Errorf("%s %s: %v", c.step, c.events, f)
		}
	}
}
