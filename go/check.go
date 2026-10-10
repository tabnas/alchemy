// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"fmt"
)

// check.go: the checker, conservative inference over the core forms
// (rs/src/check.rs, whose module documentation is the full account).
//
// Every definition of a program is checked, `export` first with its
// `input` bound to JsonEvents. A native's arguments are checked against
// its signature, a standard-library definition's against the signature
// the library declares for it (StdlibSignature), a program's own
// definition's against what its body inferred. What cannot be inferred is
// Unknown and passes; what is known to be wrong is DSL_TYPE_ERROR with a
// finer code (`arity`, `type_mismatch`, `protocol_mismatch`, `no_export`,
// `bad_output`).
//
// Ownership: a binding of Stream, JsonEvents or Text type is affine. Used
// twice in its scope it is STREAM_REUSED `reused`; used inside a fn body
// from the scope around it, `captured`. Strict mode: the function given to
// map, filter or concat-map over a stream, and the step and finish of
// scan-emit, must resolve statically, else STREAMABILITY_UNKNOWN
// `dynamic`. A result of export that cannot be typed is
// STREAMABILITY_UNKNOWN `unknown_output`.

// Output is what a program's export produces.
type Output uint8

// The outputs.
const (
	// OutputText: the program renders its own text.
	OutputText Output = iota
	// OutputTableRows: TableRows/1, which the host renders (CSV by
	// default, or JSON records).
	OutputTableRows
	// OutputJsonEvents: JsonEvents/1, which the host renders as JSON.
	OutputJsonEvents
)

// String is the output as the report names it.
func (o Output) String() string {
	switch o {
	case OutputTableRows:
		return "TableRows/1"
	case OutputJsonEvents:
		return "JsonEvents/1"
	}
	return "Text"
}

// Checked is what the checker learned about a program.
type Checked struct {
	// Export is the type of export's result.
	Export Type
	// Output is the protocol the host renders, decided by that type.
	Output Output
	// Defs is every other definition's type, by name.
	Defs map[string]Type
	// Order is the order of Defs: the program's.
	Order []string
}

// StdlibSignature is the signature the standard library declares for one
// of its definitions; the library's text is checked against it and
// programs call it through it.
func StdlibSignature(name string) (Type, bool) {
	switch name {
	case "public-column":
		return Func([]Type{RecordT}, RecordT), true
	case "table-inferred-column":
		return Func([]Type{StringT}, RecordT), true
	case "table-positional-column":
		return Func([]Type{NumberT}, RecordT), true
	case "table-value-column":
		return RecordT, true
	case "table-inferred-columns":
		return Func([]Type{ValueT}, VectorOf(RecordT)), true
	// The root adapters (stdlib/root.alc): a state and an event in, a
	// transition out; the entries answer JSON events, since each is a
	// stream of events said to be so (`as-events`).
	case "wrap-object-close":
		return Func([]Type{Unknown, Unknown}, Tagged("transition")), true
	case "wrap-object-step":
		return Func([]Type{StringT, Unknown, Unknown}, Tagged("transition")), true
	case "wrap-finish":
		return Func([]Type{Unknown}, VectorOf(EventT)), true
	case "wrap-object":
		return Func([]Type{StringT, JsonEvents}, JsonEvents), true
	case "wrap-array-close":
		return Func([]Type{Unknown, Unknown}, Tagged("transition")), true
	case "wrap-array-step":
		return Func([]Type{Unknown, Unknown}, Tagged("transition")), true
	case "wrap-array":
		return Func([]Type{JsonEvents}, JsonEvents), true
	case "table-row":
		return Func([]Type{Unknown, ValueT}, Tagged("row")), true
	case "table-first-row":
		return Func([]Type{RecordT, Unknown, ValueT}, Tagged("transition")), true
	case "table-step":
		return Func([]Type{RecordT, Unknown, Unknown}, Tagged("transition")), true
	case "table-finish":
		return Func([]Type{Unknown}, VectorOf(TableEventT)), true
	case "table-finish-for":
		return Func([]Type{RecordT, Unknown}, VectorOf(TableEventT)), true
	case "table-captures":
		return Func([]Type{RecordT}, VectorOf(CaptureSpec)), true
	case "table-from-json":
		return Func([]Type{RecordT, JsonEvents}, TableEvents()), true
	case "csv-options":
		return RecordT, true
	case "csv-field":
		return Func([]Type{RecordT, ValueT}, TextT), true
	case "csv-row":
		return Func([]Type{RecordT, VectorOf(ValueT)}, TextT), true
	case "csv":
		return Func([]Type{RecordT, TableEvents()}, TextT), true
	}
	return Type{}, false
}

func hasStdlibSignature(name string) bool {
	_, ok := StdlibSignature(name)
	return ok
}

// nativeType is the type of a native as a value: a constant's value, or
// the function.
func nativeType(n *Native) Type {
	if n.Kind == KindConstant {
		switch n.Name {
		case "root", "each-index", "each-member":
			return SelectorT
		case "missing":
			return ValueT
		}
		return Tagged(n.Name)
	}
	if k, ok := n.Arity.Exact(); ok {
		return FuncOf(k)
	}
	return Unknown
}

// local is a local binding.
type local struct {
	name string
	ty   Type
}

type env []local

func (e env) lookup(name string) (Type, bool) {
	for i := len(e) - 1; i >= 0; i-- {
		if e[i].name == name {
			return e[i].ty, true
		}
	}
	return Type{}, false
}

func (e env) has(name string) bool {
	_, ok := e.lookup(name)
	return ok
}

// MaxApplied is how many definitions deep the checker follows a stream
// into the bodies it is passed to. Past it a definition is typed by what
// its body inferred with its parameters unknown.
const MaxApplied = 32

type appliedType struct {
	name string
	args []Type
	ty   Type
}

// checker is the checker for one scope's definitions: a program's, or one
// standard library file's.
type checker struct {
	sources *Sources
	defs    map[string]*Def
	// declared: the library file's declared signatures stand for its
	// definitions; a program's are inferred.
	declared bool
	memo     map[string]Type
	applied  []appliedType
	applying int
}

func (c *checker) fail(code Code, finer, message string, span SourceSpan) *Fail {
	return c.sources.FailAt(NewFail(code, finer+": "+message), span)
}

func (c *checker) typeError(finer, message string, span SourceSpan) *Fail {
	return c.fail(CodeDSLTypeError, finer, message, span)
}

// vectorCannotHold is a vector refusing a stream or the source, named by
// its kind as the runtime names what it refuses where a vector is built (a
// vector may hold a text, which may be finite; the runtime refuses a live
// one): one sentence for a vector literal, `vector` and `push`, at both
// stages.
func (c *checker) vectorCannotHold(held Type, span SourceSpan) *Fail {
	return c.typeError("type_mismatch",
		"a vector cannot hold "+held.KindText()+"; a stream is used once, where it is", span)
}

// mismatch is `type_mismatch` or `protocol_mismatch`, by what was wanted
// and what came.
func (c *checker) mismatch(what string, expected, actual Type, span SourceSpan) *Fail {
	finer := "type_mismatch"
	if expected.IsProtocol() && actual.IsProtocol() {
		finer = "protocol_mismatch"
	}
	return c.typeError(finer, fmt.Sprintf("%s must be %s, not %s", what, expected, actual), span)
}

func (c *checker) expect(what string, expected, actual Type, span SourceSpan) *Fail {
	if expected.Accepts(actual) {
		return nil
	}
	return c.mismatch(what, expected, actual, span)
}

func (c *checker) isDef(name string) bool {
	_, ok := c.defs[name]
	return ok
}

// globalType is the type of name when it is bound outside the locals: a
// definition of this scope, a library definition, or a native.
func (c *checker) globalType(name string) (Type, bool, *Fail) {
	if c.isDef(name) {
		t, f := c.defType(name)
		return t, true, f
	}
	if t, ok := StdlibSignature(name); ok {
		return t, true, nil
	}
	if n := LookupNative(name); n != nil {
		return nativeType(n), true, nil
	}
	return Type{}, false, nil
}

// defType is the type of one of this scope's definitions, inferred once.
func (c *checker) defType(name string) (Type, *Fail) {
	if t, ok := c.memo[name]; ok {
		return t, nil
	}
	if c.declared {
		if t, ok := StdlibSignature(name); ok {
			c.memo[name] = t
			return t, nil
		}
	}
	def := c.defs[name]
	var ty Type
	var f *Fail
	if params, _, ok := fnFormOf(def.Value); ok {
		args := make([]Type, len(params))
		for i := range args {
			args[i] = Unknown
		}
		ty, f = c.fnType(def.Value.Items, args, &env{})
	} else {
		ty, f = c.infer(def.Value, &env{})
	}
	if f != nil {
		return Type{}, f
	}
	c.memo[name] = ty
	return ty, nil
}

// fnFormOf is FnForm over an expression that may not be a list.
func fnFormOf(e *Expr) ([]string, *Expr, bool) {
	if e.Kind != ExprList {
		return nil, nil, false
	}
	return FnForm(e.Items)
}

// checkDeclared checks a definition whose signature is declared: the body
// against the declaration.
func (c *checker) checkDeclared(def *Def, declared Type) *Fail {
	if names, _, ok := fnFormOf(def.Value); ok && declared.Kind == TFn {
		if len(names) != len(declared.Params) {
			return c.typeError("arity", fmt.Sprintf("%s is declared with %d parameter(s) but takes %d",
				def.Name, len(declared.Params), len(names)), def.Span)
		}
		got, f := c.fnType(def.Value.Items, declared.Params, &env{})
		if f != nil {
			return f
		}
		return c.expect("the result of "+def.Name, *declared.Result, *got.Result, def.Span)
	}
	got, f := c.infer(def.Value, &env{})
	if f != nil {
		return f
	}
	return c.expect("the value of "+def.Name, declared, got, def.Span)
}

// fnType is the type of a fn form with its parameters bound to params; the
// body is checked, and an affine parameter's uses counted.
func (c *checker) fnType(items []*Expr, params []Type, e *env) (Type, *Fail) {
	names, body, _ := FnForm(items)
	depth := len(*e)
	for i, name := range names {
		if i >= len(params) {
			break
		}
		ty := params[i]
		if ty.IsAffine() {
			if f := c.affine(name, body, *e); f != nil {
				*e = (*e)[:depth]
				return Type{}, f
			}
		}
		*e = append(*e, local{name, ty})
	}
	result, f := c.infer(body, e)
	*e = (*e)[:depth]
	if f != nil {
		return Type{}, f
	}
	return Func(append([]Type(nil), params...), result), nil
}

// affine checks an affine binding name scoped over body: at most one use,
// and none inside a nested fn.
func (c *checker) affine(name string, body *Expr, e env) *Fail {
	uses, f := c.uses(body, name, e)
	if f != nil {
		return f
	}
	if len(uses) > 1 {
		return c.fail(CodeStreamReused, "reused",
			fmt.Sprintf("%s is a stream and is used %d times; a stream is consumed once", name, len(uses)),
			uses[1])
	}
	return nil
}

// patternBinds is whether a symbol in a pattern binds (rather than
// compares): the resolver's rule.
func (c *checker) patternBinds(name string, e env) bool {
	if name == "_" {
		return false
	}
	if e.has(name) {
		return true
	}
	if c.isDef(name) {
		return false
	}
	n := LookupNative(name)
	return !(n != nil && n.Kind == KindConstant)
}

func (c *checker) patternBindsName(p *Expr, name string, e env) bool {
	switch p.Kind {
	case ExprSymbol:
		return p.Text == name && c.patternBinds(p.Text, e)
	case ExprVector:
		for _, item := range p.Items {
			if c.patternBindsName(item, name, e) {
				return true
			}
		}
	case ExprList:
		for i, item := range p.Items {
			if i > 0 && c.patternBindsName(item, name, e) {
				return true
			}
		}
	}
	return false
}

func longer(a, b []SourceSpan) []SourceSpan {
	if len(b) > len(a) {
		return b
	}
	return a
}

// uses are the uses of the local name in expr: the spans, where the arms
// of an if and the cases of a match count as alternatives (the longer
// arm), and a use inside a nested fn is `captured`.
func (c *checker) uses(expr *Expr, name string, e env) ([]SourceSpan, *Fail) {
	switch expr.Kind {
	case ExprSymbol:
		if expr.Text == name {
			return []SourceSpan{expr.Span}, nil
		}
		return nil, nil
	case ExprVector:
		var all []SourceSpan
		for _, item := range expr.Items {
			u, f := c.uses(item, name, e)
			if f != nil {
				return nil, f
			}
			all = append(all, u...)
		}
		return all, nil
	case ExprList:
	default:
		return nil, nil
	}
	items := expr.Items
	switch expr.head() {
	case "fn":
		if params, body, ok := FnForm(items); ok {
			for _, p := range params {
				if p == name {
					return nil, nil
				}
			}
			inner, f := c.uses(body, name, e)
			if f != nil {
				return nil, f
			}
			if len(inner) > 0 {
				return nil, c.fail(CodeStreamReused, "captured",
					name+" is a stream and is captured by a fn; a function may run more than once, and a stream is consumed once",
					inner[0])
			}
			return nil, nil
		}
	case "let":
		if len(items) == 3 {
			if items[1].Kind != ExprVector {
				return nil, nil
			}
			binding := items[1].Items
			var all []SourceSpan
			if len(binding) > 1 {
				u, f := c.uses(binding[1], name, e)
				if f != nil {
					return nil, f
				}
				all = append(all, u...)
			}
			if !(len(binding) > 0 && binding[0].IsSymbol(name)) {
				u, f := c.uses(items[2], name, e)
				if f != nil {
					return nil, f
				}
				all = append(all, u...)
			}
			return all, nil
		}
	case "if":
		if len(items) == 4 {
			all, f := c.uses(items[1], name, e)
			if f != nil {
				return nil, f
			}
			then, f := c.uses(items[2], name, e)
			if f != nil {
				return nil, f
			}
			otherwise, f := c.uses(items[3], name, e)
			if f != nil {
				return nil, f
			}
			return append(all, longer(then, otherwise)...), nil
		}
	case "match":
		if len(items) >= 2 {
			all, f := c.uses(items[1], name, e)
			if f != nil {
				return nil, f
			}
			var cases []SourceSpan
			for _, clause := range items[2:] {
				if clause.Kind != ExprList {
					continue
				}
				parts := clause.Items
				if len(parts) != 3 || c.patternBindsName(parts[1], name, e) {
					continue
				}
				u, f := c.uses(parts[2], name, e)
				if f != nil {
					return nil, f
				}
				cases = longer(cases, u)
			}
			return append(all, cases...), nil
		}
	}
	var all []SourceSpan
	for _, item := range items {
		u, f := c.uses(item, name, e)
		if f != nil {
			return nil, f
		}
		all = append(all, u...)
	}
	return all, nil
}

// isStaticFn is whether expr names a function the planner can see: a fn,
// a definition, a native, or a partial of one.
func (c *checker) isStaticFn(expr *Expr, e env) bool {
	switch expr.Kind {
	case ExprSymbol:
		name := expr.Text
		if e.has(name) {
			return false
		}
		if c.isDef(name) || hasStdlibSignature(name) {
			return true
		}
		n := LookupNative(name)
		return n != nil && n.Kind != KindConstant
	case ExprList:
		switch expr.head() {
		case "fn":
			_, _, ok := FnForm(expr.Items)
			return ok
		case "partial":
			if e.has("partial") {
				return false
			}
			return len(expr.Items) > 1 && c.isStaticFn(expr.Items[1], e)
		}
	}
	return false
}

func (c *checker) requireStatic(what string, expr *Expr, e env) *Fail {
	if c.isStaticFn(expr, e) {
		return nil
	}
	return c.fail(CodeStreamabilityUnknown, "dynamic",
		what+" must be a fn, a definition, a native or a partial of one, so the plan can be analyzed; strict mode refuses a function obtained at run time",
		expr.Span)
}

// pattern binds the types a pattern binds, and checks that a constructor
// pattern has the constructor's arity.
func (c *checker) pattern(p *Expr, matched Type, e *env) *Fail {
	switch p.Kind {
	case ExprSymbol:
		if c.patternBinds(p.Text, *e) {
			*e = append(*e, local{p.Text, matched})
		}
		return nil
	case ExprVector:
		item, ok := matched.ItemType()
		if !ok {
			item = Unknown
		}
		for _, q := range p.Items {
			if f := c.pattern(q, item, e); f != nil {
				return f
			}
		}
		return nil
	case ExprList:
		head := p.head()
		var fields []Type
		switch head {
		case "selected":
			fields = []Type{KeywordT, ValueT}
		case "schema", "ready":
			fields = []Type{VectorOf(RecordT)}
		case "row":
			fields = []Type{VectorOf(ValueT)}
		case "transition":
			fields = []Type{Unknown, VectorOf(Unknown)}
		case "entry":
			fields = []Type{KeywordT, Unknown}
		case "key":
			fields = []Type{StringT}
		case "scalar":
			fields = []Type{ValueT}
		default:
			n := len(p.Items) - 1
			if n < 0 {
				n = 0
			}
			fields = make([]Type, n)
			for i := range fields {
				fields[i] = Unknown
			}
		}
		if n := LookupNative(head); n != nil {
			if !n.Arity.Accepts(len(p.Items) - 1) {
				return c.typeError("arity", fmt.Sprintf("the pattern (%s ...) takes %s field(s), got %d",
					head, n.Arity, len(p.Items)-1), p.Span)
			}
		}
		for i, q := range p.Items {
			if i == 0 {
				continue
			}
			if i-1 >= len(fields) {
				break
			}
			if f := c.pattern(q, fields[i-1], e); f != nil {
				return f
			}
		}
	}
	return nil
}

// infer infers the type of one form.
func (c *checker) infer(expr *Expr, e *env) (Type, *Fail) {
	switch expr.Kind {
	case ExprSymbol:
		if t, ok := e.lookup(expr.Text); ok {
			return t, nil
		}
		t, ok, f := c.globalType(expr.Text)
		if f != nil {
			return Type{}, f
		}
		if !ok {
			return Type{}, c.typeError("unknown_name", expr.Text+" is not defined", expr.Span)
		}
		return t, nil
	case ExprKeyword:
		return KeywordT, nil
	case ExprStr:
		return StringT, nil
	case ExprNum:
		return NumberT, nil
	case ExprBool:
		return BoolT, nil
	case ExprNull:
		return NullT, nil
	case ExprVector:
		item := Never
		for _, i := range expr.Items {
			t, f := c.infer(i, e)
			if f != nil {
				return Type{}, f
			}
			if t.IsStreamOrSource() {
				return Type{}, c.vectorCannotHold(t, i.Span)
			}
			item = JoinTypes(item, t)
		}
		if item.Kind == TNever {
			item = Unknown
		}
		return VectorOf(item), nil
	}
	return c.list(expr.Items, expr.Span, e)
}

func (c *checker) list(items []*Expr, span SourceSpan, e *env) (Type, *Fail) {
	if len(items) == 0 {
		return Type{}, c.typeError("type_mismatch", "an empty list is not a call", span)
	}
	special := ""
	if name, ok := items[0].Symbol(); ok && !e.has(name) && !c.isDef(name) {
		special = name
	}
	switch special {
	case "fn":
		if params, _, ok := FnForm(items); ok {
			args := make([]Type, len(params))
			for i := range args {
				args[i] = Unknown
			}
			return c.fnType(items, args, e)
		}
	case "let":
		if len(items) == 3 {
			// The resolver has held the shape; a form that is not one fails
			// as the desugarer's would.
			badLet := func() (Type, *Fail) {
				return Type{}, c.sources.FailAt(shapeError("bad_let"), span)
			}
			if items[1].Kind != ExprVector {
				return badLet()
			}
			binding := items[1].Items
			if len(binding) < 2 {
				return badLet()
			}
			name, ok := binding[0].Symbol()
			if !ok {
				return badLet()
			}
			ty, f := c.infer(binding[1], e)
			if f != nil {
				return Type{}, f
			}
			if ty.IsAffine() {
				if f := c.affine(name, items[2], *e); f != nil {
					return Type{}, f
				}
			}
			*e = append(*e, local{name, ty})
			result, f := c.infer(items[2], e)
			*e = (*e)[:len(*e)-1]
			return result, f
		}
	case "if":
		if len(items) == 4 {
			condition, f := c.infer(items[1], e)
			if f != nil {
				return Type{}, f
			}
			if f := c.expect("the condition of if", BoolT, condition, items[1].Span); f != nil {
				return Type{}, f
			}
			then, f := c.infer(items[2], e)
			if f != nil {
				return Type{}, f
			}
			otherwise, f := c.infer(items[3], e)
			if f != nil {
				return Type{}, f
			}
			return JoinTypes(then, otherwise), nil
		}
	case "match":
		if len(items) >= 2 {
			matched, f := c.infer(items[1], e)
			if f != nil {
				return Type{}, f
			}
			result := Never
			for _, clause := range items[2:] {
				if clause.Kind != ExprList || len(clause.Items) != 3 {
					continue
				}
				parts := clause.Items
				depth := len(*e)
				if f := c.pattern(parts[1], matched, e); f != nil {
					*e = (*e)[:depth]
					return Type{}, f
				}
				body, f := c.infer(parts[2], e)
				*e = (*e)[:depth]
				if f != nil {
					return Type{}, f
				}
				result = JoinTypes(result, body)
			}
			if result.Kind == TNever && len(items) == 2 {
				return Unknown, nil
			}
			return result, nil
		}
	}
	return c.call(items, span, e)
}

// describe is a head, for messages: the checker's and the evaluator's,
// which name a callee the same way.
func describe(head *Expr) string {
	if name, ok := head.Symbol(); ok {
		return name
	}
	return CanonicalForm(head)
}

// call checks a call: the head's type decides how the arguments are
// checked.
func (c *checker) call(items []*Expr, span SourceSpan, e *env) (Type, *Fail) {
	head := items[0]
	args := items[1:]
	// A native named directly (and not shadowed) is checked by its own
	// signature, which knows more than a function type can say.
	if name, ok := head.Symbol(); ok && !e.has(name) && !c.isDef(name) && !hasStdlibSignature(name) {
		if n := LookupNative(name); n != nil {
			return c.nativeCall(n, args, span, e)
		}
	}
	fn, f := c.infer(head, e)
	if f != nil {
		return Type{}, f
	}
	argTypes := make([]Type, 0, len(args))
	affine := false
	for _, arg := range args {
		t, f := c.infer(arg, e)
		if f != nil {
			return Type{}, f
		}
		affine = affine || t.IsAffine()
		argTypes = append(argTypes, t)
	}
	if affine {
		t, ok, f := c.appliedType(head, argTypes, e)
		if f != nil {
			return Type{}, f
		}
		if ok {
			fn = t
		}
	}
	switch fn.Kind {
	case TFn:
		if len(fn.Params) != len(args) {
			return Type{}, c.typeError("arity", fmt.Sprintf("%s takes %d argument(s), got %d",
				describe(head), len(fn.Params), len(args)), span)
		}
		for i, param := range fn.Params {
			if f := c.expect("an argument of "+describe(head), param, argTypes[i], args[i].Span); f != nil {
				return Type{}, f
			}
		}
		return *fn.Result, nil
	case TUnknown, TNever:
		return Unknown, nil
	}
	return Type{}, c.typeError("type_mismatch",
		fmt.Sprintf("%s is %s and cannot be called", describe(head), fn.KindText()), head.Span)
}

func typesEqual(a, b []Type) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if !a[i].Equal(b[i]) {
			return false
		}
	}
	return true
}

// appliedType is the type of a definition or a fn literal applied to
// arguments of which one is a stream, a text or the source, typed again
// with its parameters bound to the arguments' types: the parameter is
// then affine in the body. ok is false for any other head, a count that
// does not match, or past MaxApplied.
func (c *checker) appliedType(head *Expr, args []Type, e *env) (Type, bool, *Fail) {
	if c.applying >= MaxApplied {
		return Type{}, false, nil
	}
	switch head.Kind {
	case ExprSymbol:
		name := head.Text
		if e.has(name) || !c.isDef(name) {
			return Type{}, false, nil
		}
		def := c.defs[name]
		params, _, ok := fnFormOf(def.Value)
		if !ok || len(params) != len(args) {
			return Type{}, false, nil
		}
		for _, a := range c.applied {
			if a.name == name && typesEqual(a.args, args) {
				return a.ty, true, nil
			}
		}
		c.applying++
		typed, f := c.fnType(def.Value.Items, args, &env{})
		c.applying--
		if f != nil {
			return Type{}, false, f
		}
		c.applied = append(c.applied, appliedType{def.Name, append([]Type(nil), args...), typed})
		return typed, true, nil
	case ExprList:
		if e.has("fn") {
			return Type{}, false, nil
		}
		params, _, ok := FnForm(head.Items)
		if !ok || len(params) != len(args) {
			return Type{}, false, nil
		}
		c.applying++
		typed, f := c.fnType(head.Items, args, e)
		c.applying--
		if f != nil {
			return Type{}, false, f
		}
		return typed, true, nil
	}
	return Type{}, false, nil
}

// fnArg is a function argument of a higher-order native: its type, with a
// fn literal's parameters bound to the item types it will see.
func (c *checker) fnArg(arg *Expr, params []Type, e *env) (Type, *Fail) {
	if names, _, ok := fnFormOf(arg); ok && len(names) == len(params) {
		return c.fnType(arg.Items, params, e)
	}
	return c.infer(arg, e)
}

// resultOf is the result type of a function type applied, when known.
func resultOf(f Type) Type {
	if f.Kind == TFn {
		return *f.Result
	}
	return Unknown
}

func (c *checker) expectFn(what string, arity int, f Type, span SourceSpan) *Fail {
	switch f.Kind {
	case TFn:
		if len(f.Params) != arity {
			return c.typeError("arity", fmt.Sprintf("%s takes a function of %d argument(s), not %d",
				what, arity, len(f.Params)), span)
		}
		return nil
	case TUnknown, TNever:
		return nil
	}
	return c.mismatch(what, FuncOf(arity), f, span)
}

// expectParams checks a function argument's parameters against the types
// a higher-order native gives it, as a direct call checks its arguments.
func (c *checker) expectParams(what string, f Type, given []Type, span SourceSpan) *Fail {
	if f.Kind != TFn {
		return nil
	}
	for i, param := range f.Params {
		if i >= len(given) {
			break
		}
		if fail := c.expect("an item given to "+what, param, given[i], span); fail != nil {
			return fail
		}
	}
	return nil
}

// expectSeq checks a sequence argument: a vector or a stream of items; the
// items' type and whether it is a stream.
func (c *checker) expectSeq(what string, t Type, span SourceSpan) (Type, bool, *Fail) {
	switch t.Kind {
	case TVector:
		return *t.Item, false, nil
	case TStream:
		return *t.Item, true, nil
	case TUnknown, TNever:
		return Unknown, false, nil
	case TJsonEvents:
		return Type{}, false, c.typeError("protocol_mismatch",
			what+" must be a vector or a stream of items, not JsonEvents; select or route what the stream should yield, or read its events",
			span)
	}
	return Type{}, false, c.mismatch(what, VectorOf(Unknown), t, span)
}

func (c *checker) nativeCall(n *Native, args []*Expr, span SourceSpan, e *env) (Type, *Fail) {
	if !n.Arity.Accepts(len(args)) {
		return Type{}, c.typeError("arity", fmt.Sprintf("%s takes %s argument(s), got %d", n.Name, n.Arity, len(args)), span)
	}
	name := n.Name
	// The natives whose function argument is typed by the items it sees.
	switch name {
	case "map", "filter", "concat-map":
		data, f := c.infer(args[1], e)
		if f != nil {
			return Type{}, f
		}
		item, streaming, f := c.expectSeq("the data of "+name, data, args[1].Span)
		if f != nil {
			return Type{}, f
		}
		if streaming {
			if f := c.requireStatic("the function of "+name+" over a stream", args[0], *e); f != nil {
				return Type{}, f
			}
		}
		fn, f := c.fnArg(args[0], []Type{item}, e)
		if f != nil {
			return Type{}, f
		}
		if f := c.expectFn("the function of "+name, 1, fn, args[0].Span); f != nil {
			return Type{}, f
		}
		if f := c.expectParams("the function of "+name, fn, []Type{item}, args[1].Span); f != nil {
			return Type{}, f
		}
		result := resultOf(fn)
		switch {
		case name == "map" && streaming:
			return StreamOf(result), nil
		case name == "map":
			return VectorOf(result), nil
		case name == "filter":
			if f := c.expect("the result of the predicate of "+name, BoolT, result, args[0].Span); f != nil {
				return Type{}, f
			}
			return data, nil
		}
		if !result.IsTextlike() {
			return Type{}, c.mismatch("the result of the function of concat-map", TextT, result, args[0].Span)
		}
		return TextT, nil
	case "scan-emit":
		init, f := c.infer(args[0], e)
		if f != nil {
			return Type{}, f
		}
		if init.IsAffine() {
			return Type{}, c.typeError("type_mismatch", "the state of scan-emit cannot be "+init.KindText(), args[0].Span)
		}
		source, f := c.infer(args[3], e)
		if f != nil {
			return Type{}, f
		}
		item, streaming, f := c.expectSeq("the stream of scan-emit", source, args[3].Span)
		if f != nil {
			return Type{}, f
		}
		if !streaming && source.Kind != TUnknown && source.Kind != TNever {
			return Type{}, c.mismatch("the stream of scan-emit", StreamOf(Unknown), source, args[3].Span)
		}
		if f := c.requireStatic("the step of scan-emit", args[1], *e); f != nil {
			return Type{}, f
		}
		if f := c.requireStatic("the finish of scan-emit", args[2], *e); f != nil {
			return Type{}, f
		}
		step, f := c.fnArg(args[1], []Type{Unknown, item}, e)
		if f != nil {
			return Type{}, f
		}
		if f := c.expectFn("the step of scan-emit", 2, step, args[1].Span); f != nil {
			return Type{}, f
		}
		if f := c.expectParams("the step of scan-emit", step, []Type{Unknown, item}, args[3].Span); f != nil {
			return Type{}, f
		}
		if f := c.expect("the result of the step of scan-emit", Tagged("transition"), resultOf(step), args[1].Span); f != nil {
			return Type{}, f
		}
		finish, f := c.fnArg(args[2], []Type{Unknown}, e)
		if f != nil {
			return Type{}, f
		}
		if f := c.expectFn("the finish of scan-emit", 1, finish, args[2].Span); f != nil {
			return Type{}, f
		}
		if f := c.expect("the result of the finish of scan-emit", VectorOf(Unknown), resultOf(finish), args[2].Span); f != nil {
			return Type{}, f
		}
		return StreamOf(Unknown), nil
	}

	types := make([]Type, 0, len(args))
	for _, arg := range args {
		t, f := c.infer(arg, e)
		if f != nil {
			return Type{}, f
		}
		types = append(types, t)
	}
	at := func(i int) SourceSpan { return args[i].Span }
	t := func(i int) Type { return types[i] }
	data := func(i int, what string) *Fail {
		if t(i).IsData() {
			return nil
		}
		return c.mismatch(what, ValueT, t(i), at(i))
	}
	noStream := func(i int, what string) *Fail {
		if t(i).IsAffine() {
			return c.typeError("type_mismatch",
				fmt.Sprintf("%s cannot hold %s; a stream or a text is used once, where it is", what, t(i).KindText()), at(i))
		}
		return nil
	}
	textlike := func(i int, what string) *Fail {
		if t(i).IsTextlike() {
			return nil
		}
		return c.mismatch(what, TextT, t(i), at(i))
	}
	// first is the first failure of fs, or nil.
	first := func(fs ...func() *Fail) *Fail {
		for _, f := range fs {
			if fail := f(); fail != nil {
				return fail
			}
		}
		return nil
	}
	exp := func(what string, expected Type, i int) func() *Fail {
		return func() *Fail { return c.expect(what, expected, t(i), at(i)) }
	}
	ret := func(ty Type, f *Fail) (Type, *Fail) {
		if f != nil {
			return Type{}, f
		}
		return ty, nil
	}

	switch name {
	case "get":
		switch t(0).Kind {
		case TKeyword, TString, TUnknown, TNever:
		default:
			return Type{}, c.mismatch("the key of get", KeywordT, t(0), at(0))
		}
		switch {
		case t(1).Kind == TRecord, t(1).Kind == TValue, t(1).Kind == TUnknown, t(1).Kind == TNever:
		case t(1).Kind == TTagged && t(1).Tag == "missing":
		default:
			return Type{}, c.mismatch("the data of get", RecordT, t(1), at(1))
		}
		return Unknown, nil
	case "get-path":
		return ret(ValueT, first(exp("the path of get-path", SelectorT, 0), func() *Fail { return data(1, "the data of get-path") }))
	case "as-path":
		return ret(SelectorT, data(0, "the data of as-path"))
	case "as-vector":
		return ret(VectorOf(ValueT), data(0, "the data of as-vector"))
	case "record":
		for i := range args {
			if f := c.expect("an argument of record", Tagged("entry"), t(i), at(i)); f != nil {
				return Type{}, f
			}
		}
		return RecordT, nil
	case "entry":
		return ret(Tagged("entry"), first(exp("the key of entry", KeywordT, 0), func() *Fail { return noStream(1, "a record") }))
	case "vector":
		item := Never
		for i := range args {
			if t(i).IsStreamOrSource() {
				return Type{}, c.vectorCannotHold(t(i), at(i))
			}
			item = JoinTypes(item, t(i))
		}
		if item.Kind == TNever {
			item = Unknown
		}
		return VectorOf(item), nil
	case "push":
		if t(0).IsStreamOrSource() {
			return Type{}, c.vectorCannotHold(t(0), at(0))
		}
		if f := c.expect("the vector of push", VectorOf(Unknown), t(1), at(1)); f != nil {
			return Type{}, f
		}
		if t(1).Kind == TVector {
			return VectorOf(JoinTypes(*t(1).Item, t(0))), nil
		}
		return VectorOf(Unknown), nil
	case "pop":
		if f := c.expect("the vector of pop", VectorOf(Unknown), t(0), at(0)); f != nil {
			return Type{}, f
		}
		if t(0).Kind == TVector {
			return t(0), nil
		}
		return VectorOf(Unknown), nil
	case "top":
		if f := c.expect("the vector of top", VectorOf(Unknown), t(0), at(0)); f != nil {
			return Type{}, f
		}
		if item, ok := t(0).ItemType(); ok {
			return item, nil
		}
		return Unknown, nil
	case "count":
		return ret(NumberT, exp("the vector of count", VectorOf(Unknown), 0)())
	case "keys":
		return ret(VectorOf(StringT), exp("the record of keys", RecordT, 0)())
	case "length":
		return ret(NumberT, exp("the string of length", StringT, 0)())
	case "number":
		return ret(NumberT, exp("the string of number", StringT, 0)())
	case "is-number":
		return ret(BoolT, exp("the string of is-number", StringT, 0)())
	case "unquoted":
		return ret(StringT, exp("the string of unquoted", StringT, 0)())
	case "chars-within":
		return ret(BoolT, first(exp("the ranges of chars-within", VectorOf(Unknown), 0), exp("the string of chars-within", StringT, 1)))
	case "compare":
		return ret(KeywordT, first(exp("the first number of compare", NumberT, 0), exp("the second number of compare", NumberT, 1)))
	case "number-class":
		return ret(KeywordT, exp("the number of number-class", NumberT, 0)())
	case "kind":
		if t(0).IsAffine() {
			return Type{}, c.mismatch("the value of kind", ValueT, t(0), at(0))
		}
		return KeywordT, nil
	case "path":
		for i := range args {
			switch t(i).Kind {
			case TString, TNumber, TSelector, TUnknown, TNever:
			default:
				return Type{}, c.mismatch("a segment of path", StringT, t(i), at(i))
			}
		}
		return SelectorT, nil
	case "property":
		return ret(SelectorT, exp("the name of property", StringT, 0)())
	case "index":
		return ret(SelectorT, exp("the position of index", NumberT, 0)())
	case "compose":
		return ret(SelectorT, first(exp("the first selector of compose", SelectorT, 0), exp("the second selector of compose", SelectorT, 1)))
	case "capture":
		if f := first(exp("the tag of capture", KeywordT, 0), exp("the selector of capture", SelectorT, 1)); f != nil {
			return Type{}, f
		}
		if len(args) == 3 {
			if f := exp("the limit of capture", KeywordT, 2)(); f != nil {
				return Type{}, f
			}
		}
		return CaptureSpec, nil
	case "route":
		return ret(StreamOf(Tagged("selected")), first(exp("the captures of route", VectorOf(CaptureSpec), 0), exp("the input of route", JsonEvents, 1)))
	case "select":
		return ret(StreamOf(ValueT), first(exp("the selector of select", SelectorT, 0), exp("the input of select", JsonEvents, 1)))
	case "events":
		return ret(Events(), exp("the input of events", JsonEvents, 0)())
	case "as-events":
		return ret(JsonEvents, exp("the items of as-events", StreamOf(Unknown), 0)())
	case "indices":
		return ret(VectorOf(NumberT), exp("the vector of indices", VectorOf(Unknown), 0)())
	case "transition":
		return ret(Tagged("transition"), first(func() *Fail { return noStream(0, "a state") }, exp("the outputs of transition", VectorOf(Unknown), 1)))
	case "partial":
		switch t(0).Kind {
		case TFn:
			params := t(0).Params
			if len(params) < len(args)-1 {
				return Type{}, c.typeError("arity", fmt.Sprintf("partial supplies %d argument(s) to a function of %d",
					len(args)-1, len(params)), span)
			}
			for i := 1; i < len(args); i++ {
				if f := noStream(i, "a partial application"); f != nil {
					return Type{}, f
				}
				if f := c.expect("an argument of partial", params[i-1], t(i), at(i)); f != nil {
					return Type{}, f
				}
			}
			return Func(append([]Type(nil), params[len(args)-1:]...), *t(0).Result), nil
		case TUnknown, TNever:
			// A function of more than one arity (`json`, `fail`) or of a
			// type only the run knows: its arguments are still held, so
			// none may be a stream.
			for i := 1; i < len(args); i++ {
				if f := noStream(i, "a partial application"); f != nil {
					return Type{}, f
				}
			}
			return Unknown, nil
		}
		return Type{}, c.mismatch("the function of partial", FuncOf(1), t(0), at(0))
	case "join":
		if f := exp("the separator of join", StringT, 0)(); f != nil {
			return Type{}, f
		}
		item, _, f := c.expectSeq("the items of join", t(1), at(1))
		if f != nil {
			return Type{}, f
		}
		if !item.IsTextlike() {
			return Type{}, c.mismatch("an item of join", TextT, item, at(1))
		}
		return TextT, nil
	case "concat":
		for i := range args {
			if f := textlike(i, "an item of concat"); f != nil {
				return Type{}, f
			}
		}
		return TextT, nil
	case "text":
		return ret(TextT, exp("the argument of text", StringT, 0)())
	case "replace-text":
		return ret(TextT, first(exp("the literal of replace-text", StringT, 0), exp("the replacement of replace-text", StringT, 1),
			func() *Fail { return textlike(2, "the text of replace-text") }))
	case "scalar-text":
		return ret(StringT, first(exp("the options of scalar-text", RecordT, 0), func() *Fail { return data(1, "the cell of scalar-text") }))
	case "quoted":
		return ret(StringT, exp("the string of quoted", StringT, 0)())
	case "repeat":
		return ret(StringT, first(exp("the count of repeat", NumberT, 0), exp("the string of repeat", StringT, 1)))
	case "string-join":
		return ret(StringT, first(exp("the separator of string-join", StringT, 0), exp("the strings of string-join", VectorOf(Unknown), 1)))
	case "split":
		return ret(VectorOf(StringT), first(exp("the separator of split", StringT, 0), exp("the string of split", StringT, 1)))
	case "fail":
		// `(fail message)` is INPUT_INVALID; `(fail :code message)` names
		// the code: a literal keyword is held to the three here, any other
		// keyword at run time.
		message := len(args) - 1
		if len(args) == 2 {
			if f := exp("the code of fail", KeywordT, 0)(); f != nil {
				return Type{}, f
			}
			if args[0].Kind == ExprKeyword {
				switch args[0].Text {
				case "invalid", "unrepresentable", "protocol-order":
				default:
					return Type{}, c.typeError("type_mismatch",
						"the code of fail must be :invalid, :unrepresentable or :protocol-order, not :"+args[0].Text, args[0].Span)
				}
			}
		}
		return ret(Never, exp("the message of fail", StringT, message)())
	case "is-ready":
		return BoolT, nil
	case "require-columns":
		return VectorOf(RecordT), nil
	case "schema":
		return ret(Tagged("schema"), exp("the columns of schema", VectorOf(Unknown), 0)())
	case "row":
		return ret(Tagged("row"), exp("the cells of row", VectorOf(Unknown), 0)())
	case "ready":
		return ret(Tagged("ready"), noStream(0, "a state"))
	case "selected":
		return ret(Tagged("selected"), exp("the tag of selected", KeywordT, 0)())
	case "key":
		return ret(Tagged("key"), exp("the name of key", StringT, 0)())
	case "scalar":
		switch t(0).Kind {
		case TNull, TBool, TNumber, TString, TValue, TUnknown, TNever:
		default:
			return Type{}, c.typeError("type_mismatch",
				fmt.Sprintf("the value of scalar must be null, a boolean, a number or a string, not %s", t(0)), at(0))
		}
		return Tagged("scalar"), nil
	case "json":
		// `json events`, or `json options events`.
		events := len(args) - 1
		if len(args) == 2 {
			if f := exp("the options of json", RecordT, 0)(); f != nil {
				return Type{}, f
			}
		}
		return ret(TextT, exp("the events of json", JsonEvents, events)())
	case "csv-table":
		return ret(TableEvents(), first(exp("the options of csv-table", RecordT, 0), exp("the table events of csv-table", TableEvents(), 1)))
	case "records":
		return ret(JsonEvents, exp("the table events of records", TableEvents(), 0)())
	}
	// The constants are not calls; a call of one is an arity error above.
	// Anything else is a native this table does not know.
	return Type{}, c.typeError("type_mismatch", name+" is not callable here", span)
}

// outputOf is the output protocol an export result type decides, or why
// it does not.
func outputOf(export Type) (Output, Code, string, string) {
	switch export.Kind {
	case TText, TString:
		return OutputText, 0, "", ""
	case TJsonEvents:
		return OutputJsonEvents, 0, "", ""
	case TStream:
		// Items the checker could not type could be anything, and Unknown
		// is accepted everywhere: asked first, so that such a stream is
		// never taken for a table's rows.
		if export.Item.Kind == TUnknown {
			return 0, CodeStreamabilityUnknown, "unknown_output",
				"export answers a stream of items whose type is not known; as-events says they are events, or render them as a text (join, concat-map), or make table events of them"
		}
		if TableEventT.Accepts(*export.Item) {
			return OutputTableRows, 0, "", ""
		}
		// A rewritten tree: every item an event, handed on as JSON events.
		if EventT.Accepts(*export.Item) {
			return OutputJsonEvents, 0, "", ""
		}
		return 0, CodeDSLTypeError, "bad_output",
			fmt.Sprintf("export answers a Stream<%s>; render it as a text (join, concat-map), or make table events of it", export.Item)
	case TUnknown:
		return 0, CodeStreamabilityUnknown, "unknown_output",
			"the result of export cannot be typed; it must be a text, table events or JSON events"
	}
	return 0, CodeDSLTypeError, "bad_output",
		fmt.Sprintf("export answers a %s; it must answer a text, table events or JSON events", export)
}

// noExport is the failure of a program with no export.
func noExport() *Fail {
	return NewFail(CodeDSLTypeError, "no_export: the program has no `def export [input]`")
}

// CheckProgram checks a resolved program: export with its input, then
// every other definition.
func CheckProgram(resolved *Resolved, sources *Sources) (*Checked, *Fail) {
	c := &checker{sources: sources, defs: resolved.Defs, memo: map[string]Type{}}
	export := resolved.Get("export")
	if export == nil {
		return nil, noExport()
	}
	params, _, ok := fnFormOf(export.Value)
	if !ok {
		return nil, c.typeError("type_mismatch", "export must be a fn [input]", export.Span)
	}
	if len(params) != 1 {
		return nil, c.typeError("arity", fmt.Sprintf("export takes one parameter, the input, not %d", len(params)), export.Span)
	}
	// The definitions export reaches, typed first and each after the ones
	// it names, so typing export finds them memoized and never recurses
	// along a chain of definitions.
	order := resolved.DependencyOrder()
	reached := map[string]bool{}
	for _, name := range resolved.Reachable("export") {
		reached[name] = true
	}
	for _, name := range order {
		if reached[name] {
			if _, f := c.defType(name); f != nil {
				return nil, f
			}
		}
	}
	fn, f := c.fnType(export.Value.Items, []Type{JsonEvents}, &env{})
	if f != nil {
		return nil, f
	}
	exportType := *fn.Result
	output, code, finer, message := outputOf(exportType)
	if finer != "" {
		return nil, c.fail(code, finer, message, export.Span)
	}
	c.memo["export"] = Func([]Type{JsonEvents}, exportType)
	for _, name := range order {
		if name != "export" {
			if _, f := c.defType(name); f != nil {
				return nil, f
			}
		}
	}
	checked := &Checked{Export: exportType, Output: output, Defs: map[string]Type{}}
	for _, name := range resolved.Order {
		if name != "export" {
			t, f := c.defType(name)
			if f != nil {
				return nil, f
			}
			checked.Defs[name] = t
			checked.Order = append(checked.Order, name)
		}
	}
	return checked, nil
}

// checkStdlibFile checks one standard library file: every definition
// against its declared signature; a definition without one is a defect of
// the library.
func checkStdlibFile(resolved *Resolved, src string) *Fail {
	c := &checker{sources: OneSource(resolved.File, src), defs: resolved.Defs, declared: true, memo: map[string]Type{}}
	for _, name := range resolved.Order {
		def := resolved.Defs[name]
		declared, ok := StdlibSignature(name)
		if !ok {
			return c.typeError("undeclared", "the standard library defines "+name+" without a declared signature", def.Span)
		}
		if f := c.checkDeclared(def, declared); f != nil {
			return f
		}
	}
	return nil
}
