// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"fmt"
	"math"
	"strings"
	"unicode/utf8"

	tabnas "github.com/tabnas/parser/go"
)

// ast.go: the syntax tree, Expr with source spans, built from the
// reader's tagged value tree, and the two printers, canonical and layout
// (rs/src/ast.rs).
//
// Every Expr this package builds nests at most MaxNesting levels: the
// reader refuses a deeper program before building anything, ExprFromValue
// refuses a deeper tagged tree without recursing over it, and the
// desugarer refuses a rewrite that would nest deeper. That bound is what
// lets the printers and the desugarer recurse per level.

// MaxNesting is the most levels a form may nest: a list or vector inside a
// list or vector, this many times over. The reader counts a layout line,
// each indentation level and each open `(` or `[` as one and refuses the
// opener or the line that would pass the bound with `too_deep`.
const MaxNesting = 256

// tooDeep is the message of a `too_deep` failure, one text for the grammar
// document, this file and the desugarer; a test holds it to MaxNesting.
const tooDeep = "nesting deeper than 256 levels"

// File names the source a span is in. Every span of one parse shares one
// File, so identity tells the standard library's files from a program
// that happens to share a name with one (see stdlibFileOf).
type File struct {
	Name string
}

// NewFile is a file named name.
func NewFile(name string) *File { return &File{Name: name} }

// SourceSpan is where a form came from: its file, and the byte range
// Start..End of its source text.
type SourceSpan struct {
	File  *File
	Start int
	End   int
}

// FileName is the name of the span's file, "" when it has none.
func (s SourceSpan) FileName() string {
	if s.File == nil {
		return ""
	}
	return s.File.Name
}

// String is `file:start..end`.
func (s SourceSpan) String() string {
	return fmt.Sprintf("%s:%d..%d", s.FileName(), s.Start, s.End)
}

// Position is the 1-based row and column of Start in src, the column
// counted in characters, as the engine counts them: a row ends at a line
// feed, and a column restarts at a line feed or at a carriage return,
// whether or not a line feed follows it. A start past the end of src
// answers the position just after its last character.
func (s SourceSpan) Position(src string) (int, int) {
	start := s.Start
	if start > len(src) {
		start = len(src)
	}
	if start < 0 {
		start = 0
	}
	before := src[:floorBoundary(src, start)]
	row := strings.Count(before, "\n") + 1
	lineStart := strings.LastIndexAny(before, "\n\r") + 1
	col := utf8.RuneCountInString(before[lineStart:]) + 1
	return row, col
}

// floorBoundary is the largest character boundary at or before index.
func floorBoundary(text string, index int) int {
	if index >= len(text) {
		return len(text)
	}
	for index > 0 && !utf8.RuneStart(text[index]) {
		index--
	}
	return index
}

// Sources are the texts a program is compiled from, each under the file
// name its spans carry. A diagnostic positions a span in the text of the
// span's own file, and names the file when there are several.
type Sources struct {
	names []string
	texts []string
}

// OneSource is one text, named file.
func OneSource(file, text string) *Sources {
	return &Sources{names: []string{file}, texts: []string{text}}
}

// severalSources is several texts, each under its name, in order; the
// caller holds the names distinct.
func severalSources(names, texts []string) *Sources {
	return &Sources{names: names, texts: texts}
}

// First is the first file's name: the program's, when there is one.
func (s *Sources) First() string { return s.names[0] }

// NamesFiles is whether a diagnostic names its file: when the sources are
// several.
func (s *Sources) NamesFiles() bool { return len(s.names) > 1 }

func (s *Sources) find(span SourceSpan) (string, bool) {
	name := span.FileName()
	for i, n := range s.names {
		if n == name {
			return s.texts[i], true
		}
	}
	return "", false
}

// TextOf is the text a span is positioned in: its file's, or the first
// when its file is not among these.
func (s *Sources) TextOf(span SourceSpan) string {
	if text, ok := s.find(span); ok {
		return text
	}
	return s.texts[0]
}

// Position is the 1-based row and column of span in its file's text.
func (s *Sources) Position(span SourceSpan) (int, int) {
	return span.Position(s.TextOf(span))
}

// FailAt is f at span: its row and column, and its file when the sources
// are several and the span is in one of them.
func (s *Sources) FailAt(f *Fail, span SourceSpan) *Fail {
	row, col := s.Position(span)
	f.At(uint64(row), uint64(col))
	if _, ok := s.find(span); s.NamesFiles() && ok {
		f.InFile(span.FileName())
	}
	return f
}

// ExprKind is the kind of an Expr.
type ExprKind uint8

// The kinds of form.
const (
	ExprSymbol ExprKind = iota
	ExprKeyword
	ExprStr
	ExprNum
	ExprBool
	ExprNull
	ExprList
	ExprVector
)

// Expr is one form of a program.
//
// Text is a symbol's or a keyword's name, a string's value, or a number's
// lexeme: a number keeps the text the author wrote rather than a value,
// as tabnas-transduce keeps a source number's lexeme. Bool is a boolean's
// value; Items are a list's or a vector's.
type Expr struct {
	Kind  ExprKind
	Text  string
	Bool  bool
	Items []*Expr
	Span  SourceSpan
}

// Sym is a symbol.
func Sym(name string, span SourceSpan) *Expr {
	return &Expr{Kind: ExprSymbol, Text: name, Span: span}
}

// ListOf is a list of items.
func ListOf(items []*Expr, span SourceSpan) *Expr {
	return &Expr{Kind: ExprList, Items: items, Span: span}
}

// Symbol is the symbol's name, when this form is a symbol.
func (e *Expr) Symbol() (string, bool) {
	if e != nil && e.Kind == ExprSymbol {
		return e.Text, true
	}
	return "", false
}

// IsSymbol is whether this form is the symbol name.
func (e *Expr) IsSymbol(name string) bool {
	s, ok := e.Symbol()
	return ok && s == name
}

// IsAtom is whether this form is an atom: anything but a list or a vector.
func (e *Expr) IsAtom() bool {
	return e.Kind != ExprList && e.Kind != ExprVector
}

// SameShape is structural equality that ignores spans: the same forms
// with the same names, values and lexemes, wherever they came from.
func (e *Expr) SameShape(o *Expr) bool {
	if e.Kind != o.Kind {
		return false
	}
	switch e.Kind {
	case ExprSymbol, ExprKeyword, ExprStr, ExprNum:
		return e.Text == o.Text
	case ExprBool:
		return e.Bool == o.Bool
	case ExprNull:
		return true
	default:
		return SameProgram(e.Items, o.Items)
	}
}

// SameProgram is SameShape over two programs, form by form.
func SameProgram(a, b []*Expr) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if !a[i].SameShape(b[i]) {
			return false
		}
	}
	return true
}

// head is the symbol a list begins with, "" for anything else.
func (e *Expr) head() string {
	if e.Kind != ExprList || len(e.Items) == 0 {
		return ""
	}
	s, _ := e.Items[0].Symbol()
	return s
}

// ---------------------------------------------------------------------------
// From the reader's tagged tree
// ---------------------------------------------------------------------------

func malformed(format string, args ...any) *Fail {
	return NewFail(CodeDSLParseError, "malformed reader output: "+fmt.Sprintf(format, args...))
}

// nodeFields reads a tagged node's fields, in either object form the
// engine hands out.
func nodeField(node any, name string) (any, bool) {
	switch n := node.(type) {
	case *tabnas.OrderedMap:
		return n.Get(name)
	case map[string]any:
		v, ok := n[name]
		return v, ok
	}
	return nil, false
}

func isObject(node any) bool {
	switch node.(type) {
	case *tabnas.OrderedMap, map[string]any:
		return true
	}
	return false
}

func field(node any, name string) (any, *Fail) {
	v, ok := nodeField(node, name)
	if !ok {
		return nil, malformed("a node without %q", name)
	}
	return v, nil
}

func textField(node any, name string) (string, *Fail) {
	v, f := field(node, name)
	if f != nil {
		return "", f
	}
	s, ok := v.(string)
	if !ok {
		return "", malformed("%q is not a string: %v", name, v)
	}
	return s, nil
}

func offset(v any) (int, *Fail) {
	var n float64
	switch x := v.(type) {
	case int:
		return x, nil
	case int64:
		n = float64(x)
	case float64:
		n = x
	default:
		return 0, malformed("a span offset that is not a whole number: %v", v)
	}
	if math.IsInf(n, 0) || math.IsNaN(n) || n < 0 || n != math.Trunc(n) {
		return 0, malformed("a span offset that is not a whole number: %v", v)
	}
	return int(n), nil
}

// readNode reads one node of the tagged tree before its items are: an atom
// is a finished form, a container still has its items to build.
func readNode(value any, file *File) (atom *Expr, list bool, span SourceSpan, items []any, fail *Fail) {
	if !isObject(value) {
		return nil, false, span, nil, malformed("a node that is not an object: %v", value)
	}
	raw, f := field(value, "span")
	if f != nil {
		return nil, false, span, nil, f
	}
	pair, ok := raw.([]any)
	if !ok || len(pair) != 2 {
		return nil, false, span, nil, malformed("a span that is not a pair: %v", raw)
	}
	start, f := offset(pair[0])
	if f != nil {
		return nil, false, span, nil, f
	}
	end, f := offset(pair[1])
	if f != nil {
		return nil, false, span, nil, f
	}
	span = SourceSpan{File: file, Start: start, End: end}
	tag, f := textField(value, "$")
	if f != nil {
		return nil, false, span, nil, f
	}
	switch tag {
	case "sym", "kw":
		name, f := textField(value, "name")
		if f != nil {
			return nil, false, span, nil, f
		}
		kind := ExprSymbol
		if tag == "kw" {
			kind = ExprKeyword
		}
		return &Expr{Kind: kind, Text: name, Span: span}, false, span, nil, nil
	case "str":
		s, f := textField(value, "value")
		if f != nil {
			return nil, false, span, nil, f
		}
		return &Expr{Kind: ExprStr, Text: s, Span: span}, false, span, nil, nil
	case "num":
		s, f := textField(value, "lexeme")
		if f != nil {
			return nil, false, span, nil, f
		}
		return &Expr{Kind: ExprNum, Text: s, Span: span}, false, span, nil, nil
	case "bool":
		v, f := field(value, "value")
		if f != nil {
			return nil, false, span, nil, f
		}
		b, ok := v.(bool)
		if !ok {
			return nil, false, span, nil, malformed("a bool that is not a bool: %v", v)
		}
		return &Expr{Kind: ExprBool, Bool: b, Span: span}, false, span, nil, nil
	case "null":
		return &Expr{Kind: ExprNull, Span: span}, false, span, nil, nil
	case "list", "vector":
		v, f := field(value, "items")
		if f != nil {
			return nil, false, span, nil, f
		}
		arr, ok := v.([]any)
		if !ok {
			return nil, false, span, nil, malformed("items that are not an array: %v", v)
		}
		return nil, tag == "list", span, arr, nil
	}
	return nil, false, span, nil, malformed("an unknown tag %q", tag)
}

func tooDeepAt(span SourceSpan) *Fail {
	return NewFail(CodeDSLParseError, fmt.Sprintf("too_deep: %s (at %s)", tooDeep, span))
}

type frame struct {
	list  bool
	span  SourceSpan
	items []any
	next  int
	built []*Expr
}

func (f *frame) finish() *Expr {
	kind := ExprVector
	if f.list {
		kind = ExprList
	}
	return &Expr{Kind: kind, Items: f.built, Span: f.span}
}

// ExprFromValue builds a form from one node of the reader's tagged tree.
// A tree the reader did not build may be malformed; that is a
// DSL_PARSE_ERROR, never a panic. Iterative, over an explicit stack of
// open containers, refusing a tree deeper than MaxNesting as `too_deep`
// rather than recursing into it.
func ExprFromValue(value any, file *File) (*Expr, *Fail) {
	var stack []*frame
	pending := value
	for {
		atom, list, span, items, fail := readNode(pending, file)
		if fail != nil {
			return nil, fail
		}
		built := atom
		if built == nil {
			if len(stack) >= MaxNesting {
				return nil, tooDeepAt(span)
			}
			fr := &frame{list: list, span: span, items: items, built: make([]*Expr, 0, len(items))}
			if len(items) > 0 {
				fr.next = 1
				stack = append(stack, fr)
				pending = items[0]
				continue
			}
			built = fr.finish()
		}
		// A finished form belongs to the innermost open container, whose
		// next item is built next; a container out of items is finished in
		// turn, up to the root.
		for {
			if len(stack) == 0 {
				return built, nil
			}
			fr := stack[len(stack)-1]
			stack = stack[:len(stack)-1]
			fr.built = append(fr.built, built)
			if fr.next < len(fr.items) {
				pending = fr.items[fr.next]
				fr.next++
				stack = append(stack, fr)
				break
			}
			built = fr.finish()
		}
	}
}

// ProgramFromValue builds a whole program from the reader's array of
// top-level nodes.
func ProgramFromValue(value any, file *File) ([]*Expr, *Fail) {
	forms, ok := value.([]any)
	if !ok {
		return nil, malformed("a program that is not an array: %v", value)
	}
	out := make([]*Expr, 0, len(forms))
	for _, form := range forms {
		e, f := ExprFromValue(form, file)
		if f != nil {
			return nil, f
		}
		out = append(out, e)
	}
	return out, nil
}

// ---------------------------------------------------------------------------
// Canonical form
// ---------------------------------------------------------------------------

// CanonicalForm is one form, fully parenthesized on one line: strings as
// JSON literals, numbers by lexeme, vectors in brackets, keywords with
// their colon.
func CanonicalForm(e *Expr) string {
	var b strings.Builder
	writeCanonical(e, &b)
	return b.String()
}

func writeCanonical(e *Expr, b *strings.Builder) {
	switch e.Kind {
	case ExprSymbol:
		b.WriteString(e.Text)
	case ExprKeyword:
		b.WriteByte(':')
		b.WriteString(e.Text)
	case ExprStr:
		writeJSONString(e.Text, b)
	case ExprNum:
		b.WriteString(e.Text)
	case ExprBool:
		if e.Bool {
			b.WriteString("true")
		} else {
			b.WriteString("false")
		}
	case ExprNull:
		b.WriteString("null")
	case ExprList:
		writeItems(e.Items, '(', ')', b)
	case ExprVector:
		writeItems(e.Items, '[', ']', b)
	}
}

func writeItems(items []*Expr, open, close byte, b *strings.Builder) {
	b.WriteByte(open)
	for i, item := range items {
		if i > 0 {
			b.WriteByte(' ')
		}
		writeCanonical(item, b)
	}
	b.WriteByte(close)
}

// writeJSONString writes the JSON literal for s exactly as serde_json does
// (the Rust crate's printer): the quote and the backslash escaped, the
// controls below U+0020 as \b \t \n \f \r or \u00xx in lowercase hex, and
// everything else, U+007F and non-ASCII included, as itself.
func writeJSONString(s string, b *strings.Builder) {
	const hex = "0123456789abcdef"
	b.WriteByte('"')
	start := 0
	for i := 0; i < len(s); i++ {
		c := s[i]
		var esc string
		switch {
		case c == '"':
			esc = `\"`
		case c == '\\':
			esc = `\\`
		case c == '\b':
			esc = `\b`
		case c == '\t':
			esc = `\t`
		case c == '\n':
			esc = `\n`
		case c == '\f':
			esc = `\f`
		case c == '\r':
			esc = `\r`
		case c < 0x20:
			esc = `\u00` + string([]byte{hex[c>>4], hex[c&0xf]})
		default:
			continue
		}
		b.WriteString(s[start:i])
		b.WriteString(esc)
		start = i + 1
	}
	b.WriteString(s[start:])
	b.WriteByte('"')
}

// jsonString is writeJSONString as a string.
func jsonString(s string) string {
	var b strings.Builder
	writeJSONString(s, &b)
	return b.String()
}

// Canonical is a whole program in canonical form: one top-level form per
// line.
func Canonical(program []*Expr) string {
	parts := make([]string, len(program))
	for i, form := range program {
		parts[i] = CanonicalForm(form)
	}
	return strings.Join(parts, "\n")
}

// ---------------------------------------------------------------------------
// Layout form
// ---------------------------------------------------------------------------

// Format is a whole program in layout form, the shape the reader reads
// back to the same program.
//
// An atom or a vector prints on one line in canonical form. A list with
// fewer than two items, or whose first item is a list, prints canonically
// with explicit parens, because a layout line cannot express it. Any other
// list puts its leading atoms and vectors on one line and each remaining
// item on its own line two spaces deeper. Top-level forms are separated by
// a blank line.
func Format(program []*Expr) string {
	var b strings.Builder
	for i, form := range program {
		if i > 0 {
			b.WriteByte('\n')
		}
		writeLayout(form, 0, &b)
	}
	return b.String()
}

func writeLayout(e *Expr, depth int, b *strings.Builder) {
	indent := strings.Repeat("  ", depth)
	if e.Kind != ExprList {
		b.WriteString(indent)
		b.WriteString(CanonicalForm(e))
		b.WriteByte('\n')
		return
	}
	head := 0
	for head < len(e.Items) && e.Items[head].Kind != ExprList {
		head++
	}
	if len(e.Items) < 2 || head == 0 {
		b.WriteString(indent)
		b.WriteString(CanonicalForm(e))
		b.WriteByte('\n')
		return
	}
	b.WriteString(indent)
	for i, item := range e.Items[:head] {
		if i > 0 {
			b.WriteByte(' ')
		}
		b.WriteString(CanonicalForm(item))
	}
	b.WriteByte('\n')
	for _, child := range e.Items[head:] {
		writeLayout(child, depth+1, b)
	}
}
