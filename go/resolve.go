// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"fmt"
	"strings"
)

// resolve.go: scopes and linking (rs/src/resolve.rs).
//
// A program is a sequence of `def`s, in any order: every definition sees
// every other, and a name bound nowhere (not a local, not a definition, not
// a standard-library name) is DSL_TYPE_ERROR `unknown_name`. Locals come
// from `fn` parameters, `let` bindings and the bindings a `match` pattern
// makes; each shadows what is outside it.
//
// Recursion is refused here: a definition that reaches itself through the
// references its body makes is STREAMABILITY_UNKNOWN `recursion`.
//
// Patterns: `_` matches anything; a symbol naming a standard-library
// constant or a definition of the program compares equal to it; any other
// symbol binds; a keyword, string, number, boolean or `null` compares; a
// vector matches a vector of the same length, item by item;
// `(constructor p ...)` matches the tagged value that constructor makes.

// NameKind is what kind of thing a name outside the program denotes.
type NameKind uint8

// The kinds of outer name.
const (
	// NameNone: not bound outside the program.
	NameNone NameKind = iota
	// NameConstant: a value the symbol denotes without a call (`table-end`).
	NameConstant
	// NameConstructor: a function whose value is a tagged value: also a
	// pattern head.
	NameConstructor
	// NameFunction: a function.
	NameFunction
	// NameValue: a definition's value of some other kind (a record).
	NameValue
)

// SpecialForms are the heads of the core forms and the conveniences the
// desugarer rewrites: none may be defined, and each is read by shape.
var SpecialForms = []string{"def", "fn", "let", "if", "match", "case", "pipe"}

func isSpecialForm(name string) bool {
	for _, s := range SpecialForms {
		if s == name {
			return true
		}
	}
	return false
}

// Def is one top-level definition.
type Def struct {
	Name  string
	Value *Expr
	// Span is the span of the whole def form.
	Span SourceSpan
}

// Params are the parameters, when the value is a fn.
func (d *Def) Params() ([]string, bool) {
	return FnParams(d.Value)
}

// FnParams are the parameter names of a `(fn [params] body)` form.
func FnParams(e *Expr) ([]string, bool) {
	if e.Kind != ExprList {
		return nil, false
	}
	params, _, ok := FnForm(e.Items)
	return params, ok
}

// FnForm is the parameters and the body of the items of a
// `(fn [params] body)` list; ok is false for any other shape.
func FnForm(items []*Expr) (params []string, body *Expr, ok bool) {
	if len(items) != 3 || !items[0].IsSymbol("fn") || items[1].Kind != ExprVector {
		return nil, nil, false
	}
	params = make([]string, 0, len(items[1].Items))
	for _, p := range items[1].Items {
		name, ok := p.Symbol()
		if !ok {
			return nil, nil, false
		}
		params = append(params, name)
	}
	return params, items[2], true
}

// Resolved is a resolved program: its definitions, in order, and the
// references between them.
type Resolved struct {
	// File is the file the program is named by: its first source's.
	File  string
	Defs  map[string]*Def
	Order []string
	// References are, for each definition, the definitions of this program
	// its value refers to, in first-reference order.
	References map[string][]string
}

// Get is the definition name, or nil.
func (r *Resolved) Get(name string) *Def { return r.Defs[name] }

// Reachable are the definitions name reaches, transitively, in
// first-reference order; name itself is not among them.
func (r *Resolved) Reachable(name string) []string {
	var out []string
	seen := map[string]bool{}
	refs := r.References[name]
	pending := make([]string, 0, len(refs))
	for i := len(refs) - 1; i >= 0; i-- {
		pending = append(pending, refs[i])
	}
	for len(pending) > 0 {
		next := pending[len(pending)-1]
		pending = pending[:len(pending)-1]
		if seen[next] {
			continue
		}
		seen[next] = true
		more := r.References[next]
		for i := len(more) - 1; i >= 0; i-- {
			pending = append(pending, more[i])
		}
		out = append(out, next)
	}
	return out
}

func resolveFail(code Code, finer, message string, span SourceSpan, sources *Sources) *Fail {
	return sources.FailAt(NewFail(code, finer+": "+message), span)
}

func typeFail(finer, message string, span SourceSpan, sources *Sources) *Fail {
	return resolveFail(CodeDSLTypeError, finer, message, span, sources)
}

// Resolve links a desugared program, whose forms may come from several
// sources: sources are their texts, which position a failure in the file
// its form was read from. outer answers the kind of a name bound outside
// the program (the standard library and the natives), or NameNone.
func Resolve(forms []*Expr, sources *Sources, outer func(string) NameKind) (*Resolved, *Fail) {
	defs := map[string]*Def{}
	var order []string
	for _, form := range forms {
		span := form.Span
		if form.Kind != ExprList || len(form.Items) != 3 || !form.Items[0].IsSymbol("def") {
			return nil, typeFail("not_def", "a top-level form must be a def", span, sources)
		}
		value := form.Items[2]
		name, ok := form.Items[1].Symbol()
		if !ok {
			return nil, typeFail("not_def", "a def names a symbol", form.Items[1].Span, sources)
		}
		if isSpecialForm(name) {
			return nil, typeFail("reserved", name+" is a special form and cannot be defined", span, sources)
		}
		if first, dup := defs[name]; dup {
			// Across several sources the first definition may be in
			// another file, so the message says where it is.
			message := name + " is defined twice"
			if sources.NamesFiles() {
				row, col := sources.Position(first.Span)
				message = fmt.Sprintf("%s is defined twice; first at %s:%d:%d", name, first.Span.FileName(), row, col)
			}
			return nil, typeFail("duplicate_def", message, span, sources)
		}
		defs[name] = &Def{Name: name, Value: value, Span: span}
		order = append(order, name)
	}

	references := make(map[string][]string, len(defs))
	for _, name := range order {
		w := &walker{sources: sources, outer: outer, defs: defs, seen: map[string]bool{}}
		if f := w.expr(defs[name].Value); f != nil {
			return nil, f
		}
		references[name] = w.refs
	}

	resolved := &Resolved{File: sources.First(), Defs: defs, Order: order, References: references}
	if f := resolved.refuseRecursion(sources); f != nil {
		return nil, f
	}
	return resolved, nil
}

// DependencyOrder is every definition, each after all the definitions it
// refers to: an iterative depth-first search; recursion was refused, so
// the graph has no cycle.
func (r *Resolved) DependencyOrder() []string {
	order := make([]string, 0, len(r.Order))
	done := map[string]bool{}
	open := map[string]bool{}
	type step struct {
		name string
		next int
	}
	for _, start := range r.Order {
		if done[start] {
			continue
		}
		path := []step{{start, 0}}
		open[start] = true
		for len(path) > 0 {
			top := &path[len(path)-1]
			refs := r.References[top.name]
			if top.next < len(refs) {
				child := refs[top.next]
				top.next++
				if !done[child] && !open[child] {
					open[child] = true
					path = append(path, step{child, 0})
				}
				continue
			}
			delete(open, top.name)
			done[top.name] = true
			order = append(order, top.name)
			path = path[:len(path)-1]
		}
	}
	return order
}

// refuseRecursion is `recursion` at the first definition on a cycle,
// naming the cycle.
func (r *Resolved) refuseRecursion(sources *Sources) *Fail {
	const (
		markOpen = 1
		markDone = 2
	)
	marks := map[string]int{}
	type step struct {
		name string
		next int
	}
	for _, start := range r.Order {
		if marks[start] != 0 {
			continue
		}
		path := []step{{start, 0}}
		marks[start] = markOpen
		for len(path) > 0 {
			top := &path[len(path)-1]
			refs := r.References[top.name]
			if top.next >= len(refs) {
				marks[top.name] = markDone
				path = path[:len(path)-1]
				continue
			}
			child := refs[top.next]
			top.next++
			switch marks[child] {
			case markDone:
			case markOpen:
				from := 0
				for i, s := range path {
					if s.name == child {
						from = i
						break
					}
				}
				cycle := make([]string, 0, len(path)-from)
				for _, s := range path[from:] {
					cycle = append(cycle, s.name)
				}
				through := "itself"
				if len(cycle) > 1 {
					through = strings.Join(cycle[1:], ", then ")
				}
				return resolveFail(CodeStreamabilityUnknown, "recursion",
					child+" reaches itself through "+through+"; strict mode refuses recursion",
					r.Defs[child].Span, sources)
			default:
				marks[child] = markOpen
				path = append(path, step{child, 0})
			}
		}
	}
	return nil
}

type walker struct {
	sources *Sources
	outer   func(string) NameKind
	defs    map[string]*Def
	locals  []string
	refs    []string
	seen    map[string]bool
}

func (w *walker) isLocal(name string) bool {
	for _, l := range w.locals {
		if l == name {
			return true
		}
	}
	return false
}

func (w *walker) refer(name string) {
	if !w.seen[name] {
		w.seen[name] = true
		w.refs = append(w.refs, name)
	}
}

func (w *walker) symbol(name string, span SourceSpan) *Fail {
	if w.isLocal(name) {
		return nil
	}
	if _, ok := w.defs[name]; ok {
		w.refer(name)
		return nil
	}
	if w.outer(name) != NameNone {
		return nil
	}
	return typeFail("unknown_name", name+" is not defined", span, w.sources)
}

func (w *walker) expr(e *Expr) *Fail {
	switch e.Kind {
	case ExprSymbol:
		return w.symbol(e.Text, e.Span)
	case ExprVector:
		for _, item := range e.Items {
			if f := w.expr(item); f != nil {
				return f
			}
		}
		return nil
	case ExprList:
		return w.list(e.Items, e.Span)
	}
	return nil
}

func (w *walker) list(items []*Expr, span SourceSpan) *Fail {
	head := ""
	if len(items) > 0 {
		head, _ = items[0].Symbol()
	}
	switch head {
	case "fn":
		params, body, ok := FnForm(items)
		if !ok {
			return typeFail("bad_fn", "fn takes [params] of symbols and one body", span, w.sources)
		}
		depth := len(w.locals)
		w.locals = append(w.locals, params...)
		f := w.expr(body)
		w.locals = w.locals[:depth]
		return f
	case "let":
		// The desugarer checked the shape: `(let [name value] body)`.
		if len(items) < 3 || items[1].Kind != ExprVector || len(items[1].Items) < 2 {
			return typeFail("bad_let", "let takes one binding [name value] and one body", span, w.sources)
		}
		binding := items[1].Items
		name, ok := binding[0].Symbol()
		if !ok {
			return typeFail("bad_let", "let takes one binding [name value] and one body", span, w.sources)
		}
		if f := w.expr(binding[1]); f != nil {
			return f
		}
		w.locals = append(w.locals, name)
		f := w.expr(items[2])
		w.locals = w.locals[:len(w.locals)-1]
		return f
	case "if":
		for _, item := range items[1:] {
			if f := w.expr(item); f != nil {
				return f
			}
		}
		return nil
	case "match":
		if len(items) < 2 {
			return typeFail("bad_match", "match takes a value and (case pattern body) clauses", span, w.sources)
		}
		if f := w.expr(items[1]); f != nil {
			return f
		}
		for _, clause := range items[2:] {
			if clause.Kind != ExprList {
				return typeFail("bad_match", "match takes (case pattern body) clauses", clause.Span, w.sources)
			}
			parts := clause.Items
			if len(parts) != 3 || !parts[0].IsSymbol("case") {
				return typeFail("bad_match", "match takes (case pattern body) clauses", clause.Span, w.sources)
			}
			depth := len(w.locals)
			var bound []string
			if f := w.pattern(parts[1], &bound); f != nil {
				return f
			}
			w.locals = append(w.locals, bound...)
			f := w.expr(parts[2])
			w.locals = w.locals[:depth]
			if f != nil {
				return f
			}
		}
		return nil
	case "def":
		return typeFail("misplaced_def", "def is only allowed at the top level", span, w.sources)
	}
	for _, item := range items {
		if f := w.expr(item); f != nil {
			return f
		}
	}
	return nil
}

// isConstantPattern is whether a symbol in a pattern compares rather than
// binds.
func (w *walker) isConstantPattern(name string) bool {
	if w.isLocal(name) {
		return false
	}
	if _, ok := w.defs[name]; ok {
		w.refer(name)
		return true
	}
	return w.outer(name) == NameConstant
}

func (w *walker) pattern(p *Expr, bound *[]string) *Fail {
	switch p.Kind {
	case ExprSymbol:
		if p.Text == "_" {
			return nil
		}
		if !w.isConstantPattern(p.Text) {
			*bound = append(*bound, p.Text)
		}
		return nil
	case ExprVector:
		for _, item := range p.Items {
			if f := w.pattern(item, bound); f != nil {
				return f
			}
		}
		return nil
	case ExprList:
		head, ok := "", false
		if len(p.Items) > 0 {
			head, ok = p.Items[0].Symbol()
		}
		switch {
		case ok && w.outer(head) == NameConstructor:
			for _, item := range p.Items[1:] {
				if f := w.pattern(item, bound); f != nil {
					return f
				}
			}
			return nil
		case ok:
			return typeFail("bad_pattern", head+" is not a constructor; a list pattern is (constructor pattern...)", p.Span, w.sources)
		default:
			return typeFail("bad_pattern", "a list pattern is (constructor pattern...)", p.Span, w.sources)
		}
	}
	return nil
}
