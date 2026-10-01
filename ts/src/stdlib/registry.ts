/* Copyright (c) 2026 tabnas, MIT License */

// The native operators: every name a program may use that is not a
// standard-library definition in `stdlib/*.alc`, each with its arity, its
// kind (a function, a constant, a constructor), and the signature and
// effect the reference and the planner report. A port of the table in
// rs/src/stdlib/registry.rs.
//
// Every operator takes its data last. The front end (the resolver, the
// checker, the plan report) needs only what is here; the implementations
// are the interpreter's, and it installs them with `installCalls`, keyed by
// name, so the table stays the one list of natives whichever runtime
// pieces are loaded.

// How many arguments an operator takes.
export type Arity =
  | { readonly kind: 'exact'; readonly n: number }
  | { readonly kind: 'atLeast'; readonly n: number }
  // From the first to the second, both included.
  | { readonly kind: 'between'; readonly low: number; readonly high: number }

export function exact(n: number): Arity {
  return { kind: 'exact', n }
}

export function atLeast(n: number): Arity {
  return { kind: 'atLeast', n }
}

export function between(low: number, high: number): Arity {
  return { kind: 'between', low, high }
}

// The exact count, when the arity is one.
export function arityExact(a: Arity): number | undefined {
  return 'exact' === a.kind ? a.n : undefined
}

export function arityAccepts(a: Arity, n: number): boolean {
  switch (a.kind) {
    case 'exact':
      return n === a.n
    case 'atLeast':
      return n >= a.n
    case 'between':
      return a.low <= n && n <= a.high
  }
}

// The arity as the messages print it: `2`, `at least 1`, `2 to 3`.
export function arityText(a: Arity): string {
  switch (a.kind) {
    case 'exact':
      return String(a.n)
    case 'atLeast':
      return `at least ${a.n}`
    case 'between':
      return `${a.low} to ${a.high}`
  }
}

// What kind of name an operator is.
export type Kind =
  // Applied to arguments.
  | 'function'
  // A value in its own right (`table-end`, `each-index`): the symbol
  // denotes it, without a call.
  | 'constant'
  // A function whose value is a tagged value with the operator's name as
  // tag, so `(name p ...)` is also a pattern.
  | 'constructor'

// One native operator.
export type Native = {
  readonly name: string
  readonly arity: Arity
  readonly kind: Kind
  // The signature as the reference prints it.
  readonly signature: string
  // The effect as the reference and the planner print it.
  readonly effect: string
}

// Every native, in the order the reference lists them.
const NATIVES: ReadonlyArray<Native> = [
  // Values and records.
  { name: "get", arity: exact(2), kind: 'function', signature: "get key data -> Value", effect: "reads a retained record or object; missing for an absent member" },
  { name: "get-path", arity: exact(2), kind: 'function', signature: "get-path path data -> Value", effect: "walks a retained value by one concrete path; missing where the path leaves it" },
  { name: "as-path", arity: exact(1), kind: 'function', signature: "as-path data -> Selector", effect: "validates a data-supplied array of segments; never reads data as source" },
  { name: "as-vector", arity: exact(1), kind: 'function', signature: "as-vector data -> Vector", effect: "a captured array as a vector; INPUT_INVALID otherwise" },
  { name: "record", arity: atLeast(0), kind: 'function', signature: "record entry... -> Record", effect: "a retained record; duplicate keys are an error" },
  { name: "entry", arity: exact(2), kind: 'constructor', signature: "entry :key value -> Entry", effect: "one record field" },
  { name: "vector", arity: atLeast(0), kind: 'function', signature: "vector item... -> Vector", effect: "a retained vector; it cannot hold a stream or a live text" },
  { name: "push", arity: exact(2), kind: 'function', signature: "push item vector -> Vector", effect: "a new vector with the item appended; it cannot hold a stream or a live text" },
  { name: "pop", arity: exact(1), kind: 'function', signature: "pop vector -> Vector", effect: "the vector without its last item; an empty vector is a type error" },
  { name: "top", arity: exact(1), kind: 'function', signature: "top vector -> Value", effect: "the last item; an empty vector is a type error" },
  { name: "count", arity: exact(1), kind: 'function', signature: "count vector -> Number", effect: "how many items the vector holds" },
  { name: "keys", arity: exact(1), kind: 'function', signature: "keys record -> Vector", effect: "the record's keys as strings, in its order, which for a captured object is the document's" },
  { name: "length", arity: exact(1), kind: 'function', signature: "length string -> Number", effect: "how many characters the string holds" },
  { name: "compare", arity: exact(2), kind: 'function', signature: "compare a b -> Keyword", effect: "how two numbers are ordered: :less, :equal or :greater, and :unordered when either is NaN" },
  { name: "number-class", arity: exact(1), kind: 'function', signature: "number-class number -> Keyword", effect: ":finite, :infinity, :negative-infinity or :nan" },
  { name: "kind", arity: exact(1), kind: 'function', signature: "kind value -> Keyword", effect: "the kind of a value as a keyword: :null, :boolean, :number, :string, :keyword, :vector, :record, :missing, :tagged, :function, :selector or :capture; a stream or a text cannot be asked" },
  // Selectors.
  { name: "path", arity: atLeast(0), kind: 'function', signature: "path segment... -> Selector", effect: "a selector from strings, indexes and selectors" },
  { name: "root", arity: exact(0), kind: 'constant', signature: "root -> Selector", effect: "the document" },
  { name: "each-index", arity: exact(0), kind: 'constant', signature: "each-index -> Selector", effect: "every element of an array" },
  { name: "each-member", arity: exact(0), kind: 'constant', signature: "each-member -> Selector", effect: "every member value of an object" },
  { name: "property", arity: exact(1), kind: 'function', signature: "property name -> Selector", effect: "one member" },
  { name: "index", arity: exact(1), kind: 'function', signature: "index n -> Selector", effect: "one element" },
  { name: "compose", arity: exact(2), kind: 'function', signature: "compose outer inner -> Selector", effect: "inner below every location outer names" },
  // Streams.
  { name: "capture", arity: between(2, 3), kind: 'function', signature: "capture :tag selector [:limit] -> CaptureSpec", effect: "materialize each selected scope under max_capture_bytes, or under the Limits field the keyword names (:max_metadata_bytes, :max_record_bytes), the host's value for it" },
  { name: "route", arity: exact(2), kind: 'function', signature: "route captures input -> Stream<Selected>", effect: "one pass, a shared prefix matcher; retains one selected scope at a time; captures may not overlap" },
  { name: "select", arity: exact(2), kind: 'function', signature: "select selector input -> Stream<Value>", effect: "route with one capture, delivering the values" },
  { name: "events", arity: exact(1), kind: 'function', signature: "events input -> Stream<Event>", effect: "every event of JsonEvents as one item, as it arrives: the container events as constants, key and scalar with their one field; End ends the stream and is no item; nothing is retained between events" },
  { name: "scan-emit", arity: exact(4), kind: 'function', signature: "scan-emit init step finish stream -> Stream<Output>", effect: "retains its initial state and the state the step returns, measured when the stage is built and as the state changes, through every closure, partial and finite text it holds (a text's items and the function its concat-map applies included): at most max_metadata_bytes, no deeper than max_depth, reported in retained_bytes_high; ready after each item; finish runs once at the validated end" },
  { name: "transition", arity: exact(2), kind: 'constructor', signature: "transition state outputs -> Transition", effect: "one step's result: the next state and a vector of outputs" },
  { name: "partial", arity: atLeast(1), kind: 'function', signature: "partial f arg... -> Fn", effect: "f with its first arguments supplied" },
  { name: "map", arity: exact(2), kind: 'function', signature: "map f items -> Vector | Stream", effect: "eager over a vector; per item over a stream, retaining nothing" },
  { name: "filter", arity: exact(2), kind: 'function', signature: "filter predicate items -> Vector | Stream", effect: "eager over a vector; per item over a stream" },
  // Text.
  { name: "concat-map", arity: exact(2), kind: 'function', signature: "concat-map f items -> Text", effect: "f answers a string or a text per item; each item's text is assembled whole, under max_output_bytes, and written as items arrive, so a failure leaves no half item" },
  { name: "join", arity: exact(2), kind: 'function', signature: "join separator items -> Text", effect: "the separator between items, never between the fragments of one; each item assembled as concat-map's is" },
  { name: "concat", arity: atLeast(0), kind: 'function', signature: "concat item... -> Text", effect: "in order, without assembling the result" },
  { name: "text", arity: exact(1), kind: 'function', signature: "text string -> Text", effect: "a string as a text" },
  { name: "replace-text", arity: exact(3), kind: 'function', signature: "replace-text from to text -> Text", effect: "a fixed literal replaced across fragment boundaries, a finite text's as a live one's; retains at most the literal's length" },
  { name: "scalar-text", arity: exact(2), kind: 'function', signature: "scalar-text options cell -> String", effect: "a cell's text under the options' null and missing policies: a string as it is, a number by its lexeme, a boolean by its name, a vector or a record as its compact JSON text (number lexemes kept, quotes as JSON writes them) under max_scalar_bytes; the native renderer writes the same cell the same way" },
  { name: "quoted", arity: exact(1), kind: 'function', signature: "quoted string -> String", effect: "the double-quoted form: a leading and a trailing quote, the quote and the backslash escaped by a backslash, U+0000 to U+001F as \\n, \\t, \\r, \\b, \\f or \\u00XX, and U+007F to U+009F as \\u00XX (the JSON string form, which YAML's double-quoted style reads too, plus the C1 controls its printable set excludes); refused past max_scalar_bytes, before it is built" },
  { name: "string-join", arity: exact(2), kind: 'function', signature: "string-join separator strings -> String", effect: "the strings of a vector joined into one string, the separator between them; refused past max_scalar_bytes, before it is built" },
  { name: "repeat", arity: exact(2), kind: 'function', signature: "repeat count string -> String", effect: "the string count times over; refused past max_scalar_bytes, before it is built" },
  { name: "fail", arity: exact(1), kind: 'function', signature: "fail message -> Never", effect: "INPUT_INVALID with the message and the form's position" },
  // The table protocol.
  { name: "is-ready", arity: exact(1), kind: 'function', signature: "is-ready state -> Bool", effect: "whether the state holds columns" },
  { name: "require-columns", arity: exact(1), kind: 'function', signature: "require-columns state -> Vector<Column>", effect: "the columns, or INPUT_ORDER_VIOLATION" },
  { name: "schema", arity: exact(1), kind: 'constructor', signature: "schema columns -> TableEvent", effect: "the table's one schema, of at most max_columns columns, refused where it is built past them" },
  { name: "row", arity: exact(1), kind: 'constructor', signature: "row cells -> TableEvent", effect: "one row, as wide as the schema" },
  { name: "ready", arity: exact(1), kind: 'constructor', signature: "ready columns -> State", effect: "the state once the metadata is bound" },
  { name: "selected", arity: exact(2), kind: 'constructor', signature: "selected :tag value -> Selected", effect: "what route delivers: the capture's tag and its value" },
  { name: "table-end", arity: exact(0), kind: 'constant', signature: "table-end -> TableEvent", effect: "the table's end, after the source validated" },
  { name: "no-schema", arity: exact(0), kind: 'constant', signature: "no-schema -> State", effect: "the state before the metadata" },
  { name: "missing", arity: exact(0), kind: 'constant', signature: "missing -> Value", effect: "an absent member, distinct from null" },
  // The source's events, as `events` delivers them.
  { name: "object-start", arity: exact(0), kind: 'constant', signature: "object-start -> Event", effect: "an object begins" },
  { name: "object-end", arity: exact(0), kind: 'constant', signature: "object-end -> Event", effect: "an object ends" },
  { name: "array-start", arity: exact(0), kind: 'constant', signature: "array-start -> Event", effect: "an array begins" },
  { name: "array-end", arity: exact(0), kind: 'constant', signature: "array-end -> Event", effect: "an array ends" },
  { name: "key", arity: exact(1), kind: 'constructor', signature: "key name -> Event", effect: "the name of the member whose value follows, inside an object" },
  { name: "scalar", arity: exact(1), kind: 'constructor', signature: "scalar value -> Event", effect: "one scalar of the source: null, a boolean, a number with its lexeme, or a string" },
  // Renderers and protocol adapters.
  { name: "json", arity: exact(1), kind: 'function', signature: "json events -> Text", effect: "JsonEvents, or a Stream<Event> a program built, as compact JSON text, event by event, with a final newline" },
  { name: "records", arity: exact(1), kind: 'function', signature: "records table-events -> JsonEvents", effect: "one object per row keyed by label; retains the labels" },
  { name: "csv-table", arity: exact(2), kind: 'function', signature: "csv-table options events -> TableEvents", effect: "the events unchanged, validated as the CSV renderer validates them: one schema first, of at least one column and at most max_columns, labels strings, numbers or booleans; rows as wide as the schema; one table-end; a delimiter that holds the quote, a line break or NUL is refused before anything runs" },
]

const INDEX: Map<string, Native> = new Map(NATIVES.map((n) => [n.name, n]))

// The native by name.
export function native(name: string): Native | undefined {
  return INDEX.get(name)
}

// Every native, in the order the reference lists them.
export function natives(): ReadonlyArray<Native> {
  return NATIVES
}

// The implementations, by name, once the interpreter has installed them
// (pass 2 of the port): `call(name)` is undefined until then.
const CALLS: Map<string, unknown> = new Map()

// Install implementations for natives, by name. A name the table does not
// hold is refused, so the table stays the one list of natives.
export function installCalls(calls: Record<string, unknown>): void {
  for (const [name, call] of Object.entries(calls)) {
    if (!INDEX.has(name)) throw new Error(`installCalls: ${name} is not a native`)
    CALLS.set(name, call)
  }
}

// The implementation of the native `name`, when one is installed.
export function call(name: string): unknown {
  return CALLS.get(name)
}
