// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// depth_test.go: evaluation meets MaxEvalDepth before the stack's end.
//
// The Rust crate runs compile, the command and a host's run on a thread of
// STACK_BYTES (64 MiB) so that its bounds are reached before the stack's
// end. A goroutine's stack grows as it needs, to Go's maximum (1 GB by
// default), so this port makes no such thread; these tests hold the
// process to the Rust crate's 64 MiB instead (debug.SetMaxStack) and drive
// every recursion a program can ask for to its bound: a stack overflow
// would end the test binary, where the bound answers `recursion`.

import (
	"fmt"
	"runtime/debug"
	"strings"
	"testing"

	tt "github.com/tabnas/transduce/go"
)

// stackBytes is the Rust crate's STACK_BYTES.
const stackBytes = 64 << 20

func withRustStack(t *testing.T) {
	t.Helper()
	old := debug.SetMaxStack(stackBytes)
	t.Cleanup(func() { debug.SetMaxStack(old) })
}

func isRecursion(f *Fail) bool {
	return f != nil && f.Code == CodeStreamabilityUnknown && strings.HasPrefix(f.Message, "recursion: ")
}

// chain is n definitions, each the vector of the one before.
func chain(n int) string {
	var b strings.Builder
	b.WriteString("def v0 [1]\n")
	for i := 1; i <= n; i++ {
		fmt.Fprintf(&b, "def v%d [v%d]\n", i, i-1)
	}
	return b.String()
}

func TestEvaluationMeetsItsBoundBeforeTheStacksEnd(t *testing.T) {
	withRustStack(t)
	// A function applied to itself, while the plan is built.
	if _, f := Compile("def w [f] (f f)\ndef export [input]\n  let [x (w w)]\n    json input\n", "omega.alc", routers, renderers); !isRecursion(f) || f.Row != 1 || f.Column != 12 {
		t.Errorf("%v", f)
	}
	// A chain of definitions each naming the next nests a level per form:
	// within the bound it is a value, past it recursion.
	inside := chain(MaxEvalDepth/2-20) + fmt.Sprintf("def export [input] (concat (scalar-text csv-options v%d) (json input))\n", MaxEvalDepth/2-20)
	if _, f := Compile(inside, "inside.alc", routers, renderers); f != nil {
		t.Errorf("inside the bound: %v", f)
	}
	past := chain(MaxEvalDepth) + fmt.Sprintf("def export [input] (let [x v%d] (json input))\n", MaxEvalDepth)
	if _, f := Compile(past, "past.alc", routers, renderers); !isRecursion(f) {
		t.Errorf("past the bound: %v", f)
	}
	// A finite text whose concat-map applies a function that answers the
	// same text again recurses while it is written, at run time.
	text := "def g [h x] (concat-map (partial h h) [x])\ndef export [input] (concat (g g 1) (json input))\n"
	program := mustCompile(t, text, "text.alc")
	for _, p := range []*Program{program, interpreted(t, program)} {
		if _, f := replayRun(t, p, "[1]", RenderDefault, tt.DefaultLimits(), nil); !isRecursion(f) {
			t.Errorf("native %v: %v", p.Native(), f)
		}
	}
	// A function applied to itself per item, at run time.
	omega := mustCompile(t, "def w [f] (f f)\ndef export [input]\n  pipe input\n    select (path each-index)\n    map (fn [x] (w w))\n    join \",\"\n", "omega.alc")
	if _, f := replayRun(t, omega, "[1]", RenderDefault, tt.DefaultLimits(), nil); !isRecursion(f) {
		t.Errorf("%v", f)
	}
}

// A value as deep as evaluation can build is compared, printed and
// measured within the stack: the equality of patterns, the brief a
// no_match names it by, its JSON text, and the state measure.
func TestADeepValueIsComparedPrintedAndMeasured(t *testing.T) {
	withRustStack(t)
	n := MaxEvalDepth/2 - 20
	src := chain(n) + fmt.Sprintf("def export [input] (concat (match v%d (case v%d \"same\")) (scalar-text csv-options v%d) (json input))\n", n, n, n)
	program := mustCompile(t, src, "deep.alc")
	out, f := replayRun(t, program, "1", RenderDefault, tt.DefaultLimits(), nil)
	if f != nil || !strings.HasPrefix(out, "same"+strings.Repeat("[", n+1)+"1") {
		t.Errorf("%.40q %v", out, f)
	}
	nomatch := chain(n) + fmt.Sprintf("def export [input] (concat (match v%d (case 1 \"one\")) (json input))\n", n)
	if _, f := Compile(nomatch, "nomatch.alc", routers, renderers); f == nil || !strings.HasPrefix(f.Message, "no_match: no case matches a vector ([[[") {
		t.Errorf("%v", f)
	}
	rt := runtimeOf(t, chain(n))
	v, f := rt.DefValue(ScopeProgram, fmt.Sprintf("v%d", n))
	if f != nil {
		t.Fatal(f)
	}
	if m, f := rt.Measure(v, stateBounds(tt.DefaultLimits())); f == nil || f.Limit.Name != "max_depth" {
		t.Errorf("%+v %v", m, f)
	}
}
