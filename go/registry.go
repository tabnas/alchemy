// Copyright (c) 2026 tabnas, MIT License

package tabnasalchemy

import (
	"fmt"
	"sync"
)

// registry.go: the native operators (rs/src/stdlib/registry.rs): every
// name a program may use that is not a standard-library definition in
// stdlib/*.alc, each with its arity, its kind (a function, a constant, a
// constructor), and the signature and effect the reference and the
// planner report.
//
// This is the front end's half of the table: what the resolver and the
// checker read. The implementations are keyed by name beside this table
// (natives.go), and TestEveryNativeHasAnImplementation holds the two to
// the same names, so the metadata here stays the one copy.

// Arity is how many arguments an operator takes: from Min to Max, both
// included, where Max < 0 is no upper bound.
type Arity struct {
	Min int
	Max int
}

func exact(n int) Arity        { return Arity{Min: n, Max: n} }
func atLeast(n int) Arity      { return Arity{Min: n, Max: -1} }
func between(lo, hi int) Arity { return Arity{Min: lo, Max: hi} }

// Exact is the count when the arity is exactly one.
func (a Arity) Exact() (int, bool) {
	if a.Min == a.Max {
		return a.Min, true
	}
	return 0, false
}

// Accepts is whether n arguments are accepted.
func (a Arity) Accepts(n int) bool {
	return n >= a.Min && (a.Max < 0 || n <= a.Max)
}

// String is the arity as the messages print it: `2`, `at least 1`,
// `2 to 3`.
func (a Arity) String() string {
	switch {
	case a.Min == a.Max:
		return fmt.Sprint(a.Min)
	case a.Max < 0:
		return fmt.Sprintf("at least %d", a.Min)
	}
	return fmt.Sprintf("%d to %d", a.Min, a.Max)
}

// Kind is what kind of name an operator is.
type Kind uint8

// The kinds of native.
const (
	// KindFunction: applied to arguments.
	KindFunction Kind = iota
	// KindConstant: a value in its own right (`table-end`, `each-index`):
	// the symbol denotes it, without a call.
	KindConstant
	// KindConstructor: a function whose value is a tagged value with the
	// operator's name as tag, so `(name p ...)` is also a pattern.
	KindConstructor
)

// Native is one native operator.
type Native struct {
	Name  string
	Arity Arity
	Kind  Kind
	// Signature is the signature as the reference prints it.
	Signature string
	// Effect is the effect as the reference and the planner print it:
	// what the operator retains, when its output is ready, what it
	// consumes.
	Effect string
}

var (
	nativeIndexOnce sync.Once
	nativeIndex     map[string]*Native
)

// LookupNative is the native by name, or nil.
func LookupNative(name string) *Native {
	nativeIndexOnce.Do(func() {
		nativeIndex = make(map[string]*Native, len(natives))
		for i := range natives {
			nativeIndex[natives[i].Name] = &natives[i]
		}
	})
	return nativeIndex[name]
}

// Natives is every native, in the order the reference lists them.
func Natives() []Native {
	return append([]Native(nil), natives...)
}

var natives = []Native{
	{Name: "get", Arity: exact(2), Kind: KindFunction, Signature: "get key data -> Value", Effect: "reads a retained record or object; missing for an absent member"},
	{Name: "get-path", Arity: exact(2), Kind: KindFunction, Signature: "get-path path data -> Value", Effect: "walks a retained value by one concrete path; missing where the path leaves it"},
	{Name: "as-path", Arity: exact(1), Kind: KindFunction, Signature: "as-path data -> Selector", Effect: "validates a data-supplied array of segments; never reads data as source"},
	{Name: "as-vector", Arity: exact(1), Kind: KindFunction, Signature: "as-vector data -> Vector", Effect: "a captured array as a vector; INPUT_INVALID otherwise"},
	{Name: "record", Arity: atLeast(0), Kind: KindFunction, Signature: "record entry... -> Record", Effect: "a retained record; duplicate keys are an error"},
	{Name: "entry", Arity: exact(2), Kind: KindConstructor, Signature: "entry :key value -> Entry", Effect: "one record field"},
	{Name: "vector", Arity: atLeast(0), Kind: KindFunction, Signature: "vector item... -> Vector", Effect: "a retained vector; it cannot hold a stream or a live text"},
	{Name: "push", Arity: exact(2), Kind: KindFunction, Signature: "push item vector -> Vector", Effect: "a new vector with the item appended; it cannot hold a stream or a live text"},
	{Name: "pop", Arity: exact(1), Kind: KindFunction, Signature: "pop vector -> Vector", Effect: "the vector without its last item; an empty vector is a type error"},
	{Name: "top", Arity: exact(1), Kind: KindFunction, Signature: "top vector -> Value", Effect: "the last item; an empty vector is a type error"},
	{Name: "count", Arity: exact(1), Kind: KindFunction, Signature: "count vector -> Number", Effect: "how many items the vector holds"},
	{Name: "keys", Arity: exact(1), Kind: KindFunction, Signature: "keys record -> Vector", Effect: "the record's keys as strings, in its order, which for a captured object is the document's"},
	{Name: "length", Arity: exact(1), Kind: KindFunction, Signature: "length string -> Number", Effect: "how many characters the string holds"},
	{Name: "compare", Arity: exact(2), Kind: KindFunction, Signature: "compare a b -> Keyword", Effect: "how two numbers are ordered: :less, :equal or :greater, and :unordered when either is NaN"},
	{Name: "number-class", Arity: exact(1), Kind: KindFunction, Signature: "number-class number -> Keyword", Effect: ":finite, :infinity, :negative-infinity or :nan"},
	{Name: "kind", Arity: exact(1), Kind: KindFunction, Signature: "kind value -> Keyword", Effect: "the kind of a value as a keyword: :null, :boolean, :number, :string, :keyword, :vector, :record, :missing, :tagged, :function, :selector or :capture; a stream or a text cannot be asked"},
	{Name: "path", Arity: atLeast(0), Kind: KindFunction, Signature: "path segment... -> Selector", Effect: "a selector from strings, indexes and selectors"},
	{Name: "root", Arity: exact(0), Kind: KindConstant, Signature: "root -> Selector", Effect: "the document"},
	{Name: "each-index", Arity: exact(0), Kind: KindConstant, Signature: "each-index -> Selector", Effect: "every element of an array"},
	{Name: "each-member", Arity: exact(0), Kind: KindConstant, Signature: "each-member -> Selector", Effect: "every member value of an object"},
	{Name: "property", Arity: exact(1), Kind: KindFunction, Signature: "property name -> Selector", Effect: "one member"},
	{Name: "index", Arity: exact(1), Kind: KindFunction, Signature: "index n -> Selector", Effect: "one element"},
	{Name: "compose", Arity: exact(2), Kind: KindFunction, Signature: "compose outer inner -> Selector", Effect: "inner below every location outer names"},
	{Name: "capture", Arity: between(2, 3), Kind: KindFunction, Signature: "capture :tag selector [:limit] -> CaptureSpec", Effect: "materialize each selected scope under max_capture_bytes, or under the Limits field the keyword names (:max_metadata_bytes, :max_record_bytes), the host's value for it"},
	{Name: "route", Arity: exact(2), Kind: KindFunction, Signature: "route captures input -> Stream<Selected>", Effect: "one pass, a shared prefix matcher; retains one selected scope at a time; captures may not overlap"},
	{Name: "select", Arity: exact(2), Kind: KindFunction, Signature: "select selector input -> Stream<Value>", Effect: "route with one capture, delivering the values"},
	{Name: "events", Arity: exact(1), Kind: KindFunction, Signature: "events input -> Stream<Event>", Effect: "every event of JsonEvents as one item, as it arrives: the container events as constants, key and scalar with their one field; End ends the stream and is no item; nothing is retained between events"},
	{Name: "scan-emit", Arity: exact(4), Kind: KindFunction, Signature: "scan-emit init step finish stream -> Stream<Output>", Effect: "retains its initial state and the state the step returns, measured when the stage is built and as the state changes, through every closure, partial and finite text it holds (a text's items and the function its concat-map applies included): at most max_metadata_bytes, no deeper than max_depth, reported in retained_bytes_high; ready after each item; finish runs once at the validated end"},
	{Name: "transition", Arity: exact(2), Kind: KindConstructor, Signature: "transition state outputs -> Transition", Effect: "one step's result: the next state and a vector of outputs"},
	{Name: "partial", Arity: atLeast(1), Kind: KindFunction, Signature: "partial f arg... -> Fn", Effect: "f with its first arguments supplied"},
	{Name: "map", Arity: exact(2), Kind: KindFunction, Signature: "map f items -> Vector | Stream", Effect: "eager over a vector; per item over a stream, retaining nothing"},
	{Name: "filter", Arity: exact(2), Kind: KindFunction, Signature: "filter predicate items -> Vector | Stream", Effect: "eager over a vector; per item over a stream"},
	{Name: "concat-map", Arity: exact(2), Kind: KindFunction, Signature: "concat-map f items -> Text", Effect: "f answers a string or a text per item; each item's text is assembled whole, under max_output_bytes, and written as items arrive, so a failure leaves no half item"},
	{Name: "join", Arity: exact(2), Kind: KindFunction, Signature: "join separator items -> Text", Effect: "the separator between items, never between the fragments of one; each item assembled as concat-map's is"},
	{Name: "concat", Arity: atLeast(0), Kind: KindFunction, Signature: "concat item... -> Text", Effect: "in order, without assembling the result"},
	{Name: "text", Arity: exact(1), Kind: KindFunction, Signature: "text string -> Text", Effect: "a string as a text"},
	{Name: "replace-text", Arity: exact(3), Kind: KindFunction, Signature: "replace-text from to text -> Text", Effect: "a fixed literal replaced across fragment boundaries, a finite text's as a live one's; retains at most the literal's length"},
	{Name: "scalar-text", Arity: exact(2), Kind: KindFunction, Signature: "scalar-text options cell -> String", Effect: "a cell's text under the options' null and missing policies: a string as it is, a number by its lexeme, a boolean by its name, a vector or a record as its compact JSON text (number lexemes kept, quotes as JSON writes them) under max_scalar_bytes; the native renderer writes the same cell the same way"},
	{Name: "quoted", Arity: exact(1), Kind: KindFunction, Signature: "quoted string -> String", Effect: "the double-quoted form: a leading and a trailing quote, the quote and the backslash escaped by a backslash, U+0000 to U+001F as \\n, \\t, \\r, \\b, \\f or \\u00XX, and U+007F to U+009F as \\u00XX (the JSON string form, which YAML's double-quoted style reads too, plus the C1 controls its printable set excludes); refused past max_scalar_bytes, before it is built"},
	{Name: "string-join", Arity: exact(2), Kind: KindFunction, Signature: "string-join separator strings -> String", Effect: "the strings of a vector joined into one string, the separator between them; refused past max_scalar_bytes, before it is built"},
	{Name: "repeat", Arity: exact(2), Kind: KindFunction, Signature: "repeat count string -> String", Effect: "the string count times over; refused past max_scalar_bytes, before it is built"},
	{Name: "fail", Arity: exact(1), Kind: KindFunction, Signature: "fail message -> Never", Effect: "INPUT_INVALID with the message and the form's position"},
	{Name: "is-ready", Arity: exact(1), Kind: KindFunction, Signature: "is-ready state -> Bool", Effect: "whether the state holds columns"},
	{Name: "require-columns", Arity: exact(1), Kind: KindFunction, Signature: "require-columns state -> Vector<Column>", Effect: "the columns, or INPUT_ORDER_VIOLATION"},
	{Name: "schema", Arity: exact(1), Kind: KindConstructor, Signature: "schema columns -> TableEvent", Effect: "the table's one schema, of at most max_columns columns, refused where it is built past them"},
	{Name: "row", Arity: exact(1), Kind: KindConstructor, Signature: "row cells -> TableEvent", Effect: "one row, as wide as the schema"},
	{Name: "ready", Arity: exact(1), Kind: KindConstructor, Signature: "ready columns -> State", Effect: "the state once the metadata is bound"},
	{Name: "selected", Arity: exact(2), Kind: KindConstructor, Signature: "selected :tag value -> Selected", Effect: "what route delivers: the capture's tag and its value"},
	{Name: "table-end", Arity: exact(0), Kind: KindConstant, Signature: "table-end -> TableEvent", Effect: "the table's end, after the source validated"},
	{Name: "no-schema", Arity: exact(0), Kind: KindConstant, Signature: "no-schema -> State", Effect: "the state before the metadata"},
	{Name: "missing", Arity: exact(0), Kind: KindConstant, Signature: "missing -> Value", Effect: "an absent member, distinct from null"},
	{Name: "object-start", Arity: exact(0), Kind: KindConstant, Signature: "object-start -> Event", Effect: "an object begins"},
	{Name: "object-end", Arity: exact(0), Kind: KindConstant, Signature: "object-end -> Event", Effect: "an object ends"},
	{Name: "array-start", Arity: exact(0), Kind: KindConstant, Signature: "array-start -> Event", Effect: "an array begins"},
	{Name: "array-end", Arity: exact(0), Kind: KindConstant, Signature: "array-end -> Event", Effect: "an array ends"},
	{Name: "key", Arity: exact(1), Kind: KindConstructor, Signature: "key name -> Event", Effect: "the name of the member whose value follows, inside an object"},
	{Name: "scalar", Arity: exact(1), Kind: KindConstructor, Signature: "scalar value -> Event", Effect: "one scalar of the source: null, a boolean, a number with its lexeme, or a string"},
	{Name: "json", Arity: exact(1), Kind: KindFunction, Signature: "json events -> Text", Effect: "JsonEvents, or a Stream<Event> a program built, as compact JSON text, event by event, with a final newline"},
	{Name: "records", Arity: exact(1), Kind: KindFunction, Signature: "records table-events -> JsonEvents", Effect: "one object per row keyed by label; retains the labels"},
	{Name: "csv-table", Arity: exact(2), Kind: KindFunction, Signature: "csv-table options events -> TableEvents", Effect: "the events unchanged, validated as the CSV renderer validates them: one schema first, of at least one column and at most max_columns, labels strings, numbers or booleans; rows as wide as the schema; one table-end; a delimiter that holds the quote, a line break or NUL is refused before anything runs"},
}
