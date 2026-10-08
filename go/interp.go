// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"errors"
	"fmt"
	"math"
	"strconv"
	"strings"

	"github.com/tabnas/alchemy/go/shared"
)

// interp.go: the evaluator (rs/src/interp.rs; design brief section 4.4,
// spec sections 10.1 and 10.3).
//
// Values are evaluated eagerly; streams and texts are Plans that
// evaluation builds and never runs. The Runtime holds the program's and
// the standard library's definitions, evaluates a definition once on
// first use (recursion was refused by the resolver, so there is no cycle
// to meet), and applies functions: a closure binds its parameters in the
// environment it closed over and evaluates its body in its own scope, a
// native runs its implementation, a partial supplies its arguments first.
// Runtime.Export applies the program's `export` to the host's input plan;
// what comes back is the plan lower.go turns into sinks.
//
// Two scopes exist: a program's definitions see the program first, then
// the library, then the natives; the library's see the library and the
// natives. Lexical, so a program that defines its own `csv` changes what
// its own code means and nothing the library does.
//
// The two standard compositions run natively when their arguments have
// the standard shapes (`table-from-json BINDING input` and `csv OPTIONS
// events`, from the library's own definitions, not a program's shadowing
// ones); WithNative(false) turns that off so the differential test can
// run the interpreted text.
//
// Failures: a wrong argument count is DSL_TYPE_ERROR (`arity`); a value
// of the wrong kind where the program chose it is DSL_TYPE_ERROR
// (`type_mismatch`); a `fail "message"` is INPUT_INVALID at the form's
// position. The checker reports what it can before anything runs; these
// are the runtime's own answers for what it could not see.
//
// Depth. Evaluation recurses per level, a form inside a form, a function's
// body inside the call that applied it, and MaxEvalDepth bounds the
// levels. The Rust crate runs every evaluation on a thread of 64 MiB so
// the bound is met before the stack's end; a goroutine's stack grows as it
// needs (to Go's 1 GB default), far past what MaxEvalDepth levels take, so
// no thread of a fixed size is needed here, and depth_test.go holds the
// bound to failing first.

// MaxEvalDepth is how deep evaluation may nest: a form inside a form, a
// function's body inside the call that applied it, a definition's value
// inside the form that named it. The resolver refuses a definition that
// names itself, but a function that is handed itself (`def w [f] (f f)`,
// then `(w w)`) recurses through values the resolver cannot see; this
// bound turns that into a `recursion` failure. A chain of definitions each
// naming the next, or of functions each calling the next, nests a level
// per link and meets the same bound, which is therefore also the bound on
// how deep a value a program builds can nest.
const MaxEvalDepth = 1_000

// MaxPlanSteps is how many forms building a plan may evaluate: Compile
// runs export over the input's plan, which evaluates everything not under
// a stream, and a small program can ask for an exponential amount of that
// work (`def d [g] (fn [x] (g (g x)))` nested forty deep). Past this bound
// building the plan is RESOURCE_LIMIT_EXCEEDED naming max_plan_steps. The
// work a stream does per item is bounded by the host's abort flag instead
// (Runtime.WithAbort).
const MaxPlanSteps uint64 = 1_000_000

// abortEvery is how often, in evaluation steps, the abort flag is read.
const abortEvery = 64

// libraryAt is what a message that already names a standard-library
// position holds.
const libraryAt = " (at stdlib/"

// Runtime is the evaluator for one program. It is not safe for concurrent
// use: a Program makes one per sink, and a run drives its sink from one
// goroutine.
type Runtime struct {
	program    *Resolved
	sources    *Sources
	stdlib     *Stdlib
	native     bool
	duplicates shared.Duplicates
	// limits are what the values a program builds are measured against (a
	// cell's text under max_scalar_bytes, a scan-emit state under
	// max_metadata_bytes and max_depth): the host's at run time, the
	// defaults while Compile builds the plan.
	limits shared.Limits
	// abort is the host's cancellation, read every abortEvery steps.
	abort *shared.AbortFlag
	// routers and renderers are the transduce and render stages the
	// lowering builds: the host's, handed in (WithRouters, WithRenderers).
	// Evaluation never calls them (a number's text is shortestNumber's),
	// so a runtime that only evaluates, as Compile's does, needs neither.
	routers   shared.Routers
	renderers shared.Renderers
	// fuel is the evaluation steps this runtime may take, when bounded
	// (MaxPlanSteps while building a plan).
	fuel    uint64
	bounded bool
	steps   uint64
	depth   int
	// cache holds definition values, evaluated on first use.
	cache map[defKey]Val
}

type defKey struct {
	scope Scope
	name  string
}

// arityError is a DSL_TYPE_ERROR with the `arity` finer code.
func arityError(what, wanted string, got int) *Fail {
	return NewFail(CodeDSLTypeError, fmt.Sprintf("arity: %s takes %s argument(s), got %d", what, wanted, got))
}

// NewRuntime is a runtime over a resolved program and its source texts
// (read only for positions in diagnostics), with the fast paths on,
// duplicate members rejected, and the default limits. The routers and the
// renderers it runs on are the host's: WithRouters and WithRenderers set
// them, before anything is lowered. Evaluating asks nothing of them.
func NewRuntime(program *Resolved, sources *Sources) *Runtime {
	return &Runtime{
		program:    program,
		sources:    sources,
		stdlib:     StdlibLoaded(),
		native:     true,
		duplicates: shared.Reject,
		limits:     shared.DefaultLimits(),
		abort:      shared.NewAbortFlag(),
		cache:      map[defKey]Val{},
	}
}

// WithLimits sets the limits the values a program builds are measured
// against.
func (rt *Runtime) WithLimits(limits shared.Limits) *Runtime {
	rt.limits = limits
	return rt
}

// WithAbort sets the host's cancellation: once it is set, the next
// evaluation step that reads it fails with ABORTED, however long the
// item's computation would have run.
func (rt *Runtime) WithAbort(abort *shared.AbortFlag) *Runtime {
	if abort == nil {
		abort = shared.NewAbortFlag()
	}
	rt.abort = abort
	return rt
}

// Abort is the abort flag the program's stages read between steps, for a
// stage of the host's or of transduce's that polls it too.
func (rt *Runtime) Abort() *shared.AbortFlag { return rt.abort }

// WithRouters sets the transducer stages the lowering builds: transduce's
// Routers.
func (rt *Runtime) WithRouters(routers shared.Routers) *Runtime {
	rt.routers = routers
	return rt
}

// WithRenderers sets the renderers and text stages the lowering builds:
// render's Renderers. The natives format a number themselves
// (shortestNumber, render's layout byte for byte), so they call none.
func (rt *Runtime) WithRenderers(renderers shared.Renderers) *Runtime {
	rt.renderers = renderers
	return rt
}

// Routers are the transducer stages the lowering builds.
func (rt *Runtime) Routers() shared.Routers { return rt.routers }

// Renderers are the renderers and text stages the lowering builds.
func (rt *Runtime) Renderers() shared.Renderers { return rt.renderers }

// WithFuel bounds the evaluation steps this runtime may take.
func (rt *Runtime) WithFuel(fuel uint64) *Runtime {
	rt.fuel, rt.bounded = fuel, true
	return rt
}

// WithNative runs the standard compositions natively (the default) or
// through the library's own definitions.
func (rt *Runtime) WithNative(native bool) *Runtime {
	rt.native = native
	return rt
}

// WithDuplicates sets how a materialized capture treats a repeated member
// name; the default rejects it (spec section 18.2, the strict mapping
// profile).
func (rt *Runtime) WithDuplicates(duplicates shared.Duplicates) *Runtime {
	rt.duplicates = duplicates
	return rt
}

// Native reports whether the standard compositions run natively.
func (rt *Runtime) Native() bool { return rt.native }

// Duplicates is the policy for a repeated member name.
func (rt *Runtime) Duplicates() shared.Duplicates { return rt.duplicates }

// Program is the resolved program.
func (rt *Runtime) Program() *Resolved { return rt.program }

// Limits are the limits the values a program builds are measured against.
func (rt *Runtime) Limits() shared.Limits { return rt.limits }

// Tick is one evaluation step: the fuel, and every abortEvery steps the
// abort flag. Every form evaluated takes one, and so does every node a
// finite text or a measured value walks.
func (rt *Runtime) Tick() *Fail {
	rt.steps++
	n := rt.steps
	if rt.bounded && n > rt.fuel {
		return shared.LimitFail("max_plan_steps", rt.fuel, fmt.Sprintf(
			"building the plan took more than %d evaluation steps; a program that computes this much before it reads its input is refused",
			rt.fuel))
	}
	if n%abortEvery == 0 && rt.abort.IsAborted() {
		return shared.AbortedFail()
	}
	return nil
}

// enter enters one level of evaluation at at (nil for none), or answers
// the `recursion` failure past MaxEvalDepth; leave gives the level back.
// The evaluator takes a level per form, and so does writing a finite text,
// whose concat-map applies its function as it writes: a function that
// answers a text applying itself recurses there, not in the evaluator.
func (rt *Runtime) enter(at *SourceSpan) *Fail {
	rt.depth++
	if rt.depth > MaxEvalDepth {
		rt.depth--
		f := NewFail(CodeStreamabilityUnknown, fmt.Sprintf(
			"recursion: evaluation nested past %d levels: a function applied to itself, or definitions or calls chained that deep; strict mode refuses recursion without a bound",
			MaxEvalDepth))
		if at != nil {
			return rt.FailAt(f, *at)
		}
		return f
	}
	return nil
}

func (rt *Runtime) leave() { rt.depth-- }

// Position is the 1-based row and column of a span of the program's own
// text, in its own file's text when the program has several sources; ok
// is false for a span of the standard library's, whose rows are not the
// program's.
func (rt *Runtime) Position(span SourceSpan) (row, col int, ok bool) {
	if _, _, lib := stdlibFileOf(span); lib {
		return 0, 0, false
	}
	row, col = rt.sources.Position(span)
	return row, col, true
}

// FailAt is f with the position of at, when it has none yet. A form of the
// program gives its row and column, and its file when the program was
// compiled from several sources. A form of the standard library gives
// none, since a row there would name a line of the user's file that says
// something else; the message ends with the library file, row and column
// instead, `(at stdlib/table.alc:68:5)`, once, and a form of the program
// around the library's call, when the failure passes one on its way out,
// gives the row and column.
func (rt *Runtime) FailAt(f *Fail, at SourceSpan) *Fail {
	if f.Row > 0 {
		return f
	}
	if file, src, ok := stdlibFileOf(at); ok {
		if !strings.Contains(f.Message, libraryAt) {
			row, col := at.Position(src)
			f.Message = fmt.Sprintf("%s (at %s:%d:%d)", f.Message, file, row, col)
		}
		return f
	}
	return rt.sources.FailAt(f, at)
}

// findDef is the definition name denotes in scope, with the scope it was
// found in.
func (rt *Runtime) findDef(scope Scope, name string) (Scope, *Def) {
	if scope == ScopeProgram {
		if def := rt.program.Get(name); def != nil {
			return ScopeProgram, def
		}
	}
	if def := rt.stdlib.Get(name); def != nil {
		return ScopeStdlib, def
	}
	return 0, nil
}

// DefValue is the value of a definition, evaluated once; nil, nil when
// scope sees no definition of name.
func (rt *Runtime) DefValue(scope Scope, name string) (Val, *Fail) {
	found, def := rt.findDef(scope, name)
	if def == nil {
		return nil, nil
	}
	key := defKey{found, def.Name}
	if v, ok := rt.cache[key]; ok {
		return v, nil
	}
	var value Val
	if params, ok := FnParams(def.Value); ok {
		value = &Closure{
			Name:   def.Name,
			Params: params,
			Body:   def.Value.Items[2],
			Scope:  found,
			Span:   def.Value.Span,
		}
	} else {
		v, f := rt.eval(def.Value, Env{}, found)
		if f != nil {
			return nil, f
		}
		value = v
	}
	rt.cache[key] = value
	return value, nil
}

// callNative runs a native's implementation.
func (rt *Runtime) callNative(n *Native, args []Val, at SourceSpan) (Val, *Fail) {
	impl, ok := nativeImpls[n.Name]
	if !ok {
		return nil, NewFail(CodeDSLTypeError, "type_mismatch: "+n.Name+" has no implementation")
	}
	return impl(rt, args, at)
}

// global is the value a free symbol denotes in scope: a definition, a
// native constant's value, or a native function.
func (rt *Runtime) global(scope Scope, name string, span SourceSpan) (Val, *Fail) {
	v, f := rt.DefValue(scope, name)
	if f != nil {
		return nil, f
	}
	if v != nil {
		return v, nil
	}
	if n := LookupNative(name); n != nil {
		if n.Kind == KindConstant {
			return rt.callNative(n, nil, span)
		}
		return n, nil
	}
	return nil, rt.FailAt(NewFail(CodeDSLTypeError, "unknown_name: "+name+" is not defined"), span)
}

// numberValue is a number literal's value: its lexeme read as the nearest
// float64, an infinity past float64's range (as Rust's parse answers it).
func numberValue(lexeme string) (float64, bool) {
	v, err := strconv.ParseFloat(lexeme, 64)
	if err != nil {
		var ne *strconv.NumError
		if errors.As(err, &ne) && errors.Is(ne.Err, strconv.ErrRange) {
			return v, true
		}
		return 0, false
	}
	return v, true
}

// Eval evaluates one form in env and scope.
func (rt *Runtime) Eval(e *Expr, env Env, scope Scope) (Val, *Fail) { return rt.eval(e, env, scope) }

func (rt *Runtime) eval(e *Expr, env Env, scope Scope) (Val, *Fail) {
	if f := rt.Tick(); f != nil {
		return nil, f
	}
	if f := rt.enter(&e.Span); f != nil {
		return nil, f
	}
	defer rt.leave()
	switch e.Kind {
	case ExprSymbol:
		if v, ok := env.Get(e.Text); ok {
			return v, nil
		}
		return rt.global(scope, e.Text, e.Span)
	case ExprKeyword:
		return KeywordVal(e.Text), nil
	case ExprStr:
		return StrVal(e.Text), nil
	case ExprNum:
		v, ok := numberValue(e.Text)
		if !ok {
			return nil, rt.FailAt(typeError(e.Text+" is not a number"), e.Span)
		}
		return NumLexeme(v, e.Text), nil
	case ExprBool:
		return BoolVal(e.Bool), nil
	case ExprNull:
		return NullVal{}, nil
	case ExprVector:
		out := make([]Val, 0, len(e.Items))
		for _, item := range e.Items {
			v, f := rt.eval(item, env, scope)
			if f != nil {
				return nil, f
			}
			if IsLive(v) {
				return nil, rt.FailAt(typeError(fmt.Sprintf(
					"a vector cannot hold %s; a stream is used once, where it is", liveKind(v))), e.Span)
			}
			out = append(out, v)
		}
		return Vector(out...), nil
	}
	return rt.list(e.Items, e.Span, env, scope)
}

func (rt *Runtime) list(items []*Expr, span SourceSpan, env Env, scope Scope) (Val, *Fail) {
	if len(items) == 0 {
		return nil, rt.FailAt(typeError("an empty list is not a call"), span)
	}
	// A special form's head is read by shape, unless a local shadows it.
	head, _ := items[0].Symbol()
	special := false
	switch head {
	case "fn", "let", "if", "match", "def":
		special = !env.Has(head)
	}
	switch {
	case special && head == "fn":
		params, body, ok := FnForm(items)
		if !ok {
			return nil, rt.FailAt(NewFail(CodeDSLTypeError, "bad_fn: fn takes [params] of symbols and one body"), span)
		}
		return &Closure{Params: params, Body: body, Env: env, Scope: scope, Span: span}, nil
	// The shapes of `let`, `if` and `match` are the desugarer's, held again
	// by the resolver; a form that is not one fails as the desugarer's
	// would.
	case special && head == "let":
		badLet := func() *Fail {
			return rt.FailAt(shapeError("bad_let"), span)
		}
		if len(items) != 3 || items[1].Kind != ExprVector {
			return nil, badLet()
		}
		binding := items[1].Items
		if len(binding) != 2 {
			return nil, badLet()
		}
		name, ok := binding[0].Symbol()
		if !ok {
			return nil, badLet()
		}
		value, f := rt.eval(binding[1], env, scope)
		if f != nil {
			return nil, f
		}
		return rt.eval(items[2], env.Bind(name, value), scope)
	case special && head == "if":
		if len(items) != 4 {
			return nil, rt.FailAt(shapeError("bad_if"), span)
		}
		condition, f := rt.eval(items[1], env, scope)
		if f != nil {
			return nil, f
		}
		yes, f := truth("if", condition)
		if f != nil {
			return nil, rt.FailAt(f, items[1].Span)
		}
		if yes {
			return rt.eval(items[2], env, scope)
		}
		return rt.eval(items[3], env, scope)
	case special && head == "match":
		if len(items) < 2 {
			return nil, rt.FailAt(shapeError("bad_match"), span)
		}
		value, f := rt.eval(items[1], env, scope)
		if f != nil {
			return nil, f
		}
		for _, clause := range items[2:] {
			if clause.Kind != ExprList {
				continue
			}
			parts := clause.Items
			if len(parts) != 3 || !parts[0].IsSymbol("case") {
				continue
			}
			bound, ok, f := rt.Matches(parts[1], value, env, scope)
			if f != nil {
				return nil, f
			}
			if ok {
				return rt.eval(parts[2], bound, scope)
			}
		}
		return nil, rt.FailAt(NewFail(CodeDSLTypeError, "no_match: no case matches "+brief(value)), span)
	case special && head == "def":
		return nil, rt.FailAt(NewFail(CodeDSLTypeError, "misplaced_def: def is only allowed at the top level"), span)
	}
	callee, f := rt.eval(items[0], env, scope)
	if f != nil {
		return nil, f
	}
	fn, ok := callee.(Fn)
	if !ok {
		return nil, rt.FailAt(typeError(describe(items[0])+" is "+KindOf(callee)+" and cannot be called"), items[0].Span)
	}
	args := make([]Val, 0, len(items)-1)
	for _, item := range items[1:] {
		v, f := rt.eval(item, env, scope)
		if f != nil {
			return nil, f
		}
		args = append(args, v)
	}
	return rt.Apply(fn, args, span)
}

// Apply applies a function; at is the span of the call, for diagnostics.
func (rt *Runtime) Apply(fn Fn, args []Val, at SourceSpan) (Val, *Fail) {
	switch c := fn.(type) {
	case *Closure:
		if len(c.Params) != len(args) {
			return nil, rt.FailAt(arityError(c.Describe(), strconv.Itoa(len(c.Params)), len(args)), at)
		}
		if rt.native && c.Scope == ScopeStdlib {
			if fast := rt.fastPath(c, args, at); fast != nil {
				return fast, nil
			}
		}
		env := c.Env
		for i, param := range c.Params {
			env = env.Bind(param, args[i])
		}
		return rt.eval(c.Body, env, c.Scope)
	case *Native:
		if !c.Arity.Accepts(len(args)) {
			return nil, rt.FailAt(arityError(c.Name, c.Arity.String(), len(args)), at)
		}
		v, f := rt.callNative(c, args, at)
		if f != nil {
			return nil, rt.FailAt(f, at)
		}
		return v, nil
	case *Partial:
		all := make([]Val, 0, len(c.Args)+len(args))
		all = append(all, c.Args...)
		all = append(all, args...)
		return rt.Apply(c.F, all, at)
	}
	// Closures, natives and partials are every function there is; the
	// switch needs an end, in the evaluator's sentence for a callee.
	return nil, rt.FailAt(typeError(fn.Describe()+" is "+KindOf(fn)+" and cannot be called"), at)
}

// fastPath is the native plan for a standard composition, when the
// arguments have the standard shapes; nil runs the library's text.
func (rt *Runtime) fastPath(c *Closure, args []Val, at SourceSpan) Val {
	switch c.Name {
	case "table-from-json":
		if len(args) != 2 {
			return nil
		}
		input, ok := args[1].(StreamVal)
		if !ok || !isTableBinding(args[0]) {
			return nil
		}
		return StreamVal{Plan: &Plan{Kind: PlanTableFromJSON, Binding: args[0], Source: input.Plan, At: at}}
	case "csv":
		if len(args) != 2 {
			return nil
		}
		events, ok := args[1].(StreamVal)
		if !ok {
			return nil
		}
		if _, ok := csvOptions(args[0]); !ok {
			return nil
		}
		return TextVal{Plan: &Plan{Kind: PlanCsv, Options: args[0], Source: events.Plan}}
	}
	return nil
}

// patternConstant is whether a symbol in a pattern compares rather than
// binds, and the value it compares with: the same rule the resolver
// applies (a local binds; a definition or a native constant compares;
// anything else binds).
func (rt *Runtime) patternConstant(name string, env Env, scope Scope, span SourceSpan) (Val, *Fail) {
	if env.Has(name) {
		return nil, nil
	}
	v, f := rt.DefValue(scope, name)
	if f != nil || v != nil {
		return v, f
	}
	if n := LookupNative(name); n != nil && n.Kind == KindConstant {
		return rt.callNative(n, nil, span)
	}
	return nil, nil
}

// Matches matches value against pattern: the environment extended with the
// pattern's bindings, and whether it matched.
func (rt *Runtime) Matches(pattern *Expr, value Val, env Env, scope Scope) (Env, bool, *Fail) {
	switch pattern.Kind {
	case ExprSymbol:
		if pattern.Text == "_" {
			return env, true, nil
		}
		constant, f := rt.patternConstant(pattern.Text, env, scope, pattern.Span)
		if f != nil {
			return Env{}, false, f
		}
		if constant != nil {
			return env, Equal(constant, value), nil
		}
		return env.Bind(pattern.Text, value), true, nil
	case ExprKeyword, ExprStr, ExprNum, ExprBool, ExprNull:
		literal, f := rt.eval(pattern, env, scope)
		if f != nil {
			return Env{}, false, f
		}
		return env, Equal(literal, value), nil
	case ExprVector:
		values, ok := value.(*VectorVal)
		if !ok || len(values.Items) != len(pattern.Items) {
			return Env{}, false, nil
		}
		bound := env
		for i, p := range pattern.Items {
			next, ok, f := rt.Matches(p, values.Items[i], bound, scope)
			if f != nil || !ok {
				return Env{}, false, f
			}
			bound = next
		}
		return bound, true, nil
	}
	head, ok := "", false
	if len(pattern.Items) > 0 {
		head, ok = pattern.Items[0].Symbol()
	}
	if !ok {
		return Env{}, false, rt.FailAt(NewFail(CodeDSLTypeError, "bad_pattern: a list pattern is (constructor pattern...)"), pattern.Span)
	}
	tagged, isTagged := value.(*TaggedVal)
	if !isTagged || tagged.Tag != head || len(tagged.Fields) != len(pattern.Items)-1 {
		return Env{}, false, nil
	}
	bound := env
	for i, p := range pattern.Items[1:] {
		next, ok, f := rt.Matches(p, tagged.Fields[i], bound, scope)
		if f != nil || !ok {
			return Env{}, false, f
		}
		bound = next
	}
	return bound, true, nil
}

// Export is the program's result: export applied to the host's input plan.
// A stream or a text plan comes back; nothing has been consumed.
func (rt *Runtime) Export() (Val, *Fail) {
	export, f := rt.DefValue(ScopeProgram, "export")
	if f != nil {
		return nil, f
	}
	if export == nil {
		return nil, NewFail(CodeDSLTypeError, "no_export: the program has no `def export [input]`")
	}
	fn, ok := export.(Fn)
	if !ok {
		// The checker refuses an export that is not written `def export
		// [input] ...` before anything is evaluated, so the value here is a
		// function; held again, with the checker's text.
		return nil, typeError("export must be a fn [input]")
	}
	at := SourceSpan{File: NewFile(rt.program.File)}
	if def := rt.program.Get("export"); def != nil {
		at = def.Span
	}
	return rt.Apply(fn, []Val{StreamVal{Plan: inputPlan()}}, at)
}

// EvalProgramExpr evaluates a standalone expression in the program's
// scope, for tests and tools.
func (rt *Runtime) EvalProgramExpr(e *Expr) (Val, *Fail) {
	return rt.eval(e, Env{}, ScopeProgram)
}

// ---------------------------------------------------------------------------
// Measuring values
// ---------------------------------------------------------------------------

// Measure is what a bounded walk over a value found: its size in the
// transduce measure (payload bytes plus an allowance per node) and its
// nesting.
type Measure struct {
	Bytes uint64
	Depth int
}

// Bounds are what a Runtime.Measure walk holds a value to.
type Bounds struct {
	// NodeBytes are counted for every node on top of its payload.
	NodeBytes uint64
	MaxBytes  uint64
	// BytesLimit is the Limits field MaxBytes came from, for the failure.
	BytesLimit string
	// MaxDepth is the deepest nesting allowed, under max_depth.
	MaxDepth int
	// What is what is being measured, for the message.
	What string
}

type nodeKind uint8

const (
	nodeVal nodeKind = iota
	nodePlan
	nodeEnv
	// nodeFn is a function held by something other than a value: the
	// function a partial applies, when it is itself a partial, or the one
	// a plan applies.
	nodeFn
)

// measureNode is one thing a measure walk still has to visit.
type measureNode struct {
	kind  nodeKind
	val   Val
	plan  *Plan
	env   Env
	fn    Fn
	depth int
}

// pushFn is what a function value holds, one level below depth: a closure
// its frames; a partial its arguments and the function it applies. A
// partial of a partial holds the one before it, so a chain of them (a
// state that wraps itself once per item) is walked link by link, each one
// node and one level.
func pushFn(fn Fn, depth int, pending []measureNode) []measureNode {
	switch f := fn.(type) {
	case *Closure:
		pending = append(pending, measureNode{kind: nodeEnv, env: f.Env, depth: depth + 1})
	case *Partial:
		for _, arg := range f.Args {
			pending = append(pending, measureNode{kind: nodeVal, val: arg, depth: depth + 1})
		}
		switch inner := f.F.(type) {
		case *Closure:
			pending = append(pending, measureNode{kind: nodeEnv, env: inner.Env, depth: depth + 1})
		case *Partial:
			pending = append(pending, measureNode{kind: nodeFn, fn: inner, depth: depth + 1})
		}
	}
	return pending
}

// Measure walks v without recursing, counting its size and nesting, and
// fails as soon as either passes its bound: RESOURCE_LIMIT_EXCEEDED naming
// the byte limit, or max_depth. The walk visits a shared part once per
// reference, so a value built by sharing (a vector of itself twice, forty
// times over) counts as large as it would be written, and the walk ends at
// the bound rather than visiting it all. Every node takes an evaluation
// step, so the abort flag stops it too.
func (rt *Runtime) Measure(v Val, bounds Bounds) (Measure, *Fail) {
	var found Measure
	pending := []measureNode{{kind: nodeVal, val: v, depth: 1}}
	vals := func(items []Val, depth int) {
		for _, item := range items {
			pending = append(pending, measureNode{kind: nodeVal, val: item, depth: depth + 1})
		}
	}
	for len(pending) > 0 {
		node := pending[len(pending)-1]
		pending = pending[:len(pending)-1]
		if f := rt.Tick(); f != nil {
			return found, f
		}
		depth := node.depth
		if depth > bounds.MaxDepth {
			return found, shared.LimitFail("max_depth", uint64(bounds.MaxDepth),
				fmt.Sprintf("%s nests more than %d levels deep", bounds.What, bounds.MaxDepth))
		}
		if depth > found.Depth {
			found.Depth = depth
		}
		payload := 0
		switch node.kind {
		case nodeVal:
			switch x := node.val.(type) {
			case NumVal:
				payload = 8
				if x.HasLexeme {
					payload = len(x.Lexeme)
				}
			case StrVal:
				payload = len(x)
			case KeywordVal:
				payload = len(x)
			case *VectorVal:
				vals(x.Items, depth)
			case *RecordVal:
				vals(x.vals, depth)
				for _, k := range x.keys {
					payload += len(k)
				}
			case *TaggedVal:
				vals(x.Fields, depth)
				payload = len(x.Tag)
			case *SelectorVal:
				payload = len(x.Selector.String())
			case *CaptureVal:
				payload = len(x.Spec.Tag) + len(x.Spec.Selector.String())
			case Fn:
				pending = pushFn(x, depth, pending)
			case StreamVal:
				pending = append(pending, measureNode{kind: nodePlan, plan: x.Plan, depth: depth + 1})
			case TextVal:
				pending = append(pending, measureNode{kind: nodePlan, plan: x.Plan, depth: depth + 1})
			}
		case nodePlan:
			p := node.plan
			switch p.Kind {
			case PlanLit:
				payload = len(p.Lit)
			case PlanConcat:
				vals(p.Items, depth)
			case PlanJoin:
				if !p.Seq.IsStream() {
					vals(p.Seq.Vector, depth)
				}
				payload = len(p.Sep)
			// A finite concat-map keeps the function it applies until it
			// is written: a partial over the state before holds that
			// state, so the function is walked like any value the plan
			// holds.
			case PlanConcatMap:
				pending = append(pending, measureNode{kind: nodeFn, fn: p.F, depth: depth + 1})
				if !p.Seq.IsStream() {
					vals(p.Seq.Vector, depth)
				}
			case PlanReplace:
				pending = append(pending, measureNode{kind: nodeVal, val: p.Text, depth: depth + 1})
				payload = len(p.From) + len(p.To)
			// A live plan's source is the one pass over the input, not a
			// retained value, and is not walked; what the plan itself
			// holds (its functions, its options, a scan's initial state)
			// is.
			case PlanMap, PlanFilter:
				pending = append(pending, measureNode{kind: nodeFn, fn: p.F, depth: depth + 1})
			case PlanScanEmit:
				pending = append(pending,
					measureNode{kind: nodeVal, val: p.Init, depth: depth + 1},
					measureNode{kind: nodeFn, fn: p.Step, depth: depth + 1},
					measureNode{kind: nodeFn, fn: p.Finish, depth: depth + 1})
			case PlanTableFromJSON:
				pending = append(pending, measureNode{kind: nodeVal, val: p.Binding, depth: depth + 1})
			case PlanCsvTable, PlanCsv:
				pending = append(pending, measureNode{kind: nodeVal, val: p.Options, depth: depth + 1})
			case PlanRoute:
				for _, c := range p.Specs {
					payload += len(c.Tag) + len(c.Selector.String())
				}
			case PlanSelect:
				payload = len(p.Selector.String())
			}
		// A frame's value, and the frames outside it one level further: a
		// chain of frames is walked one inside another.
		case nodeEnv:
			if value, next, ok := node.env.Frame(); ok {
				pending = append(pending,
					measureNode{kind: nodeVal, val: value, depth: depth + 1},
					measureNode{kind: nodeEnv, env: next, depth: depth + 1})
			}
		case nodeFn:
			pending = pushFn(node.fn, depth, pending)
		}
		found.Bytes = saturatingAdd(saturatingAdd(found.Bytes, bounds.NodeBytes), uint64(payload))
		if found.Bytes > bounds.MaxBytes {
			return found, shared.LimitFail(bounds.BytesLimit, bounds.MaxBytes,
				fmt.Sprintf("%s holds more than %d bytes", bounds.What, bounds.MaxBytes))
		}
	}
	return found, nil
}

func saturatingAdd(a, b uint64) uint64 {
	if a > math.MaxUint64-b {
		return math.MaxUint64
	}
	return a + b
}

// jsonText is the compact JSON text of a vector or a record, as a cell
// writes it (number lexemes kept), bounded by max_scalar_bytes: the text
// is one scalar of the output, and the native table holds a container
// cell's text to the same bound. The failure comes exactly when the text
// is longer than the bound; a lower bound of its length is counted first,
// without writing it, so a value built by sharing (a vector of a vector of
// itself, forty levels deep) fails at the limit rather than being written
// out.
func (rt *Runtime) jsonText(v Val) (string, *Fail) {
	max := uint64(rt.limits.MaxScalarBytes)
	tooLong := func() *Fail {
		return shared.LimitFail("max_scalar_bytes", max, fmt.Sprintf("a cell's JSON text holds more than %d bytes", max))
	}
	floor, f := rt.jsonTextFloor(v, max)
	if f != nil {
		return "", f
	}
	if floor > max {
		return "", tooLong()
	}
	text, f := JSONText(v)
	if f != nil {
		return "", f
	}
	if uint64(len(text)) > max {
		return "", tooLong()
	}
	return text, nil
}

// jsonTextFloor is at most the length of v's compact JSON text, counted
// without writing it or recursing, and without going on once it passes
// max: a container's brackets, a member's quoted key and colon, a string's
// quotes around its bytes (escapes only lengthen it), a number's lexeme
// (or one digit), four for null and the booleans. Every value counts at
// least one, so the walk ends within max values, and every one takes an
// evaluation step, so the abort flag stops it too.
func (rt *Runtime) jsonTextFloor(v Val, max uint64) (uint64, *Fail) {
	var total uint64
	pending := []Val{v}
	for len(pending) > 0 {
		v := pending[len(pending)-1]
		pending = pending[:len(pending)-1]
		if f := rt.Tick(); f != nil {
			return total, f
		}
		here := 0
		switch x := v.(type) {
		case NullVal, BoolVal:
			here = 4
		case NumVal:
			here = 1
			if x.HasLexeme && len(x.Lexeme) > 1 {
				here = len(x.Lexeme)
			}
		case StrVal:
			here = len(x) + 2
		case *VectorVal:
			pending = append(pending, x.Items...)
			here = 2
		case *RecordVal:
			pending = append(pending, x.vals...)
			here = 2
			for _, k := range x.keys {
				here += len(k) + 3
			}
		}
		total = saturatingAdd(total, uint64(here))
		if total > max {
			break
		}
	}
	return total, nil
}
