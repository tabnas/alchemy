// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

// desugar.go: the conveniences rewritten into the core forms
// (rs/src/desugar.rs).
//
//   - `(def name [params] body)` becomes `(def name (fn [params] body))`;
//     `(def name value)` is already core.
//   - `(pipe init step ...)` becomes nested data-last application: a bare
//     symbol step `f` is `(f acc)`, a non-empty list step `(f a b)` is
//     `(f a b acc)`, and anything else is `empty_step`.
//   - `let`, `if` and `match` are checked for shape and left as they are.
//
// Anything else passes through unchanged, `(pipe)` included. A generated
// node carries the span of the form it came from. Failures are
// DSL_PARSE_ERRORs whose message begins with the code the grammar document
// declares, at the row and column of the offending form.
//
// A rewrite can nest deeper than what it read (`def` wraps a body in a
// `fn`, `pipe` nests a level per step), so the depth of every result is
// carried alongside it and checked against MaxNesting as each container is
// built.

// desugarMessages are the messages, by code, as the grammar document also
// declares them; a test holds the two in step.
var desugarMessages = [][2]string{
	{"empty_step", "a pipe step must be a symbol or a non-empty list"},
	{"bad_def", "def takes a name and a value, or a name, [params] and a body"},
	{"bad_let", "let takes one binding [name value] and one body"},
	{"bad_if", "if takes a condition and exactly two branches"},
	{"bad_match", "match takes a value and (case pattern body) clauses"},
	{"too_deep", tooDeep},
}

func desugarMessage(code string) string {
	for _, m := range desugarMessages {
		if m[0] == code {
			return m[1]
		}
	}
	return "invalid form"
}

// shapeError is the failure of a form whose shape the desugarer owns
// (bad_def, bad_let, bad_if, bad_match), not yet positioned: a
// DSL_PARSE_ERROR with the desugarer's text, wherever the form is found.
// The resolver meets one a `pipe` builds, since a step's form grows by the
// threaded value after this pass read it; the checker and the evaluator
// check the shapes again before reading a form.
func shapeError(code string) *Fail {
	return NewFail(CodeDSLParseError, code+": "+desugarMessage(code))
}

func desugarFail(code string, span SourceSpan, src string) *Fail {
	row, col := span.Position(src)
	return shapeError(code).At(uint64(row), uint64(col))
}

// Desugar desugars a whole program, form by form. src is the program's
// source, read only to give a failure its row and column.
func Desugar(forms []*Expr, src string) ([]*Expr, *Fail) {
	out := make([]*Expr, 0, len(forms))
	for _, form := range forms {
		e, f := DesugarExpr(form, src)
		if f != nil {
			return nil, f
		}
		out = append(out, e)
	}
	return out, nil
}

// DesugarExpr desugars one form, innermost first, so a rewrite sees core
// items.
func DesugarExpr(form *Expr, src string) (*Expr, *Fail) {
	d, f := deep(form, src)
	if f != nil {
		return nil, f
	}
	return d.expr, nil
}

// deepExpr is a desugared form with its nesting: 0 for an atom, one more
// than its deepest item for a list or vector.
type deepExpr struct {
	expr  *Expr
	depth int
}

func deep(form *Expr, src string) (deepExpr, *Fail) {
	switch form.Kind {
	case ExprList:
		items, f := each(form.Items, src)
		if f != nil {
			return deepExpr{}, f
		}
		return rewrite(items, form.Span, src)
	case ExprVector:
		items, f := each(form.Items, src)
		if f != nil {
			return deepExpr{}, f
		}
		return container(items, form.Span, src, ExprVector)
	}
	return deepExpr{expr: form, depth: 0}, nil
}

func each(items []*Expr, src string) ([]deepExpr, *Fail) {
	out := make([]deepExpr, 0, len(items))
	for _, item := range items {
		d, f := deep(item, src)
		if f != nil {
			return nil, f
		}
		out = append(out, d)
	}
	return out, nil
}

// bounded is d when it is within the bound, `too_deep` at span otherwise.
func bounded(d deepExpr, span SourceSpan, src string) (deepExpr, *Fail) {
	if d.depth > MaxNesting {
		return deepExpr{}, desugarFail("too_deep", span, src)
	}
	return d, nil
}

// container is a list or vector over items, one level deeper than its
// deepest item.
func container(items []deepExpr, span SourceSpan, src string, kind ExprKind) (deepExpr, *Fail) {
	depth := 0
	exprs := make([]*Expr, len(items))
	for i, item := range items {
		if item.depth > depth {
			depth = item.depth
		}
		exprs[i] = item.expr
	}
	return bounded(deepExpr{expr: &Expr{Kind: kind, Items: exprs, Span: span}, depth: 1 + depth}, span, src)
}

func list(items []deepExpr, span SourceSpan, src string) (deepExpr, *Fail) {
	return container(items, span, src, ExprList)
}

func rewrite(items []deepExpr, span SourceSpan, src string) (deepExpr, *Fail) {
	head := ""
	if len(items) > 0 {
		head, _ = items[0].expr.Symbol()
	}
	switch head {
	case "def":
		return desugarDef(items, span, src)
	case "pipe":
		return desugarPipe(items, span, src)
	case "let":
		return shape(items, span, src, "bad_let", isLet)
	case "if":
		return shape(items, span, src, "bad_if", func(items []deepExpr) bool { return len(items) == 4 })
	case "match":
		return shape(items, span, src, "bad_match", isMatch)
	}
	return list(items, span, src)
}

// desugarDef keeps `(def name value)`; `(def name [params] body)` wraps the
// body in a `fn` carrying the def's span.
func desugarDef(items []deepExpr, span SourceSpan, src string) (deepExpr, *Fail) {
	named := len(items) > 1 && items[1].expr.Kind == ExprSymbol
	switch {
	case len(items) == 3 && named:
		return list(items, span, src)
	case len(items) == 4 && named && items[2].expr.Kind == ExprVector:
		params, body := items[2], items[3]
		depth := params.depth
		if body.depth > depth {
			depth = body.depth
		}
		function := deepExpr{
			depth: 1 + depth,
			expr:  ListOf([]*Expr{Sym("fn", span), params.expr, body.expr}, span),
		}
		return list([]deepExpr{items[0], items[1], function}, span, src)
	}
	return deepExpr{}, desugarFail("bad_def", span, src)
}

// desugarPipe threads `(pipe init step ...)` data-last; `(pipe)` passes
// through.
func desugarPipe(items []deepExpr, span SourceSpan, src string) (deepExpr, *Fail) {
	if len(items) < 2 {
		return list(items[:1], span, src)
	}
	acc := items[1]
	for _, step := range items[2:] {
		switch {
		case step.expr.Kind == ExprSymbol:
			at := step.expr.Span
			acc = deepExpr{
				depth: acc.depth + 1,
				expr:  ListOf([]*Expr{Sym(step.expr.Text, at), acc.expr}, at),
			}
		case step.expr.Kind == ExprList && len(step.expr.Items) > 0:
			// The step's own items are one level below it; the threaded
			// value joins them.
			depth := step.depth
			if acc.depth+1 > depth {
				depth = acc.depth + 1
			}
			stepItems := append(append([]*Expr(nil), step.expr.Items...), acc.expr)
			acc = deepExpr{depth: depth, expr: ListOf(stepItems, step.expr.Span)}
		default:
			return deepExpr{}, desugarFail("empty_step", step.expr.Span, src)
		}
		// Checked per step: a long pipe fails at the step that passes the
		// bound, not after the whole chain is built.
		var f *Fail
		if acc, f = bounded(acc, span, src); f != nil {
			return deepExpr{}, f
		}
	}
	return acc, nil
}

func shape(items []deepExpr, span SourceSpan, src, code string, ok func([]deepExpr) bool) (deepExpr, *Fail) {
	if ok(items) {
		return list(items, span, src)
	}
	return deepExpr{}, desugarFail(code, span, src)
}

// isLet is `(let [name value] body)`.
func isLet(items []deepExpr) bool {
	if len(items) != 3 || items[1].expr.Kind != ExprVector {
		return false
	}
	binding := items[1].expr.Items
	return len(binding) == 2 && binding[0].Kind == ExprSymbol
}

// isMatch is `(match value (case pattern body) ...)`.
func isMatch(items []deepExpr) bool {
	if len(items) < 2 {
		return false
	}
	for _, clause := range items[2:] {
		e := clause.expr
		if e.Kind != ExprList || len(e.Items) != 3 || !e.Items[0].IsSymbol("case") {
			return false
		}
	}
	return true
}
