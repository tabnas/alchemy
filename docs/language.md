# The alchemy language

This is the reference: how a program's text becomes forms (the reader),
how the forms are checked (types, ownership, protocols), what the
standard library provides, what a program means when it runs, what
`explain` reports, and how a host embeds the crate. The design this
implements is section 4 of transduce's `docs/architecture.md`.

Every `alchemy` example below, together with the `canonical`, `core` or
`check` result that follows it, is a row in
[`test/spec/reader.tsv`](../test/spec/reader.tsv),
[`test/spec/pipe.tsv`](../test/spec/pipe.tsv) or
[`test/spec/check.tsv`](../test/spec/check.tsv), and
`rs/tests/spec_test.rs` fails when one is not, so what this page shows is
what the crate does.

## Forms

A program is a sequence of **forms**. A form is an atom, a list or a
vector.

| Atom | Written | Notes |
|---|---|---|
| symbol | `def`, `active?`, `->x`, `a.b`, `-` | a run of `A-Z a-z 0-9 _ - ? ! * + / < > = . $ % & \| ^ ~ @` that is not a number or a value word |
| keyword | `:label`, `:delimiter` | a colon then a symbol run; names a field, an option, a tag |
| string | `"text"`, `"a\nb"`, `"café"` | double quotes, exactly the JSON escapes; no line breaks inside |
| number | `42`, `-1.5e3`, `1E+2` | exactly a JSON number; the lexeme is kept as written |
| value | `true`, `false`, `null` | |

A word that looks like a number but is not one (`1a`, `.5`, `+1`,
`1_000`, `0x1f`) is a symbol. `;` starts a comment that runs to the end
of the line. A tab is whitespace inside a line and an error in
indentation.

A **list** is `(` forms `)` and a **vector** is `[` forms `]`. Lists are
calls and special forms; vectors are data (parameters, bindings,
literal sequences).

## Layout

Programs are written in layout form, where lines and indentation stand
for parentheses. The rules (design brief section 4.1, spec section 9.1):

1. A line with **several inline forms** is a list of them.
2. A line with **exactly one inline form and no children** is that form.
3. Lines indented **exactly two spaces** more than a line are its
   **children**, and the line is a list of its inline forms followed by
   its children, in order. A child line follows the same rules.
4. Inside `(` `)` and `[` `]` layout is suspended: line breaks and
   indentation are ordinary whitespace, and the delimiters decide.
5. Blank lines and comment-only lines do not count, anywhere.
6. A dedent returns to the indentation of a line above; every level
   still open at the end of the source closes.
7. A line ends at a line feed, alone or after a carriage return. A
   carriage return anywhere else is whitespace, which is how the engine
   counts rows, so the row a diagnostic names is the line the layout saw.
   (A lone carriage return does restart the engine's column, and the
   columns this crate derives from spans restart with it. In a line's
   leading whitespace it restarts the indentation too: the indentation
   of a line is its run of spaces after the last lone carriage return
   before its first form, and a line holding only whitespace and
   comments after one is blank.)
8. A program nests at most 256 levels, counting a layout line, each
   indentation level and each open `(` or `[` as one. The bound holds
   after desugaring too, where a `pipe` adds a level per step.

```alchemy
a b c
```
```canonical
(a b c)
```

```alchemy
a
  b
  c
```
```canonical
(a b c)
```

```alchemy
a
  b c
  d
```
```canonical
(a (b c) d)
```

```alchemy
a b
  c
```
```canonical
(a b c)
```

```alchemy
a
  b
    c
  d
```
```canonical
(a (b c) d)
```

A dedent of two levels closes two blocks, and the next top-level line is
a form of its own:

```alchemy
a
  b
    c
d
```
```canonical
(a (b c))
d
```

A line with one form is that form, so a zero-argument call needs its
parentheses, and a one-element list can only be written explicitly:

```alchemy
x
```
```canonical
x
```

```alchemy
(newline)
```
```canonical
(newline)
```

Explicit and layout forms mix freely. Inside delimiters the layout rules
are off:

```alchemy
a (b c) d
```
```canonical
(a (b c) d)
```

```alchemy
f (a
      b
  c) d
```
```canonical
(f (a b c) d)
```

```alchemy
def f [x]
  concat (a x) (b x)
    (c x)
```
```canonical
(def f [x] (concat (a x) (b x) (c x)))
```

```alchemy
[1
2
  3]
```
```canonical
[1 2 3]
```

Comments and blank lines may sit anywhere, between children included:

```alchemy
; leading
def x 1 ; trailing
  ; between
  y ; after
; end
```
```canonical
(def x 1 y)
```

The idioms the standard library is written in follow from the rules. A
call whose last argument is itself a call puts that argument on a child
line:

```alchemy
join ","
  map csv-field values
```
```canonical
(join "," (map csv-field values))
```

A sequence of arguments, one per child line, including a zero-argument
call written with its parens:

```alchemy
concat
  prefix
  (newline)
  suffix
```
```canonical
(concat prefix (newline) suffix)
```

A constructor whose arguments are calls:

```alchemy
record
  entry :delimiter ","
```
```canonical
(record (entry :delimiter ","))
```

A function literal as a non-final argument, then the data:

```alchemy
map
  fn [column]
    get :label column
  columns
```
```canonical
(map (fn [column] (get :label column)) columns)
```

### Layout errors

Each is a `DSL_PARSE_ERROR` whose message begins with the code (see
[Spans and errors](#spans-and-errors)).

| Code | Raised by | Example |
|---|---|---|
| `tab_indent` | a tab in the indentation of a content line | `a` over a line `<TAB>b` |
| `bad_indent` | an indented first line, or a line deeper than its parent by anything but two spaces | `a` over a line indented three spaces |
| `bad_dedent` | a dedent to a column no open block has | `a` / `  b` / ` c` |
| `unbalanced` | an unclosed `(` or `[`, or a `)` or `]` with nothing open | `(a b` |
| `too_deep` | the opener, the indented line or the desugared form that would nest past 256 levels | 256 nested `(`; `pipe x` with 257 steps |
| `unterminated_string`, `unprintable`, `unexpected` | the engine's own: a string without its closing quote, a control character (a line break) inside one, and a character no rule accepts (`{`, `,`, a bare `:`, an unknown escape) | |

## Canonical and layout form

The **canonical** form of a program is fully parenthesized, one
top-level form per line: strings as JSON literals, numbers by lexeme,
vectors in brackets, keywords with their colon. `alchemy canon FILE`
prints it, and the `canonical` blocks on this page are it.

The **layout** form is what `alchemy format FILE` prints: an atom or a
vector on one line; a list of fewer than two items, or one whose first
item is a list, in explicit parens (`(newline)`, `((f x) y)`); any other
list with its leading atoms and vectors on one line and every remaining
item on its own line two spaces deeper. Top-level forms are separated by
a blank line. Reading the layout form back gives the same program, spans
aside, for every program in the fixtures. The one caveat is the nesting
bound: the reader counts a layout line as a level whether or not it makes
a list, so a program that reads exactly at the bound may print, in either
form, one level too deep to read back; below the bound both forms read
back exactly.

## Core forms and desugaring

The core forms are `def`, `fn`, `let`, `if` and `match`. Desugaring
(`rs/src/desugar.rs`) runs after the reader and before the checker, and
does four things. Everything else, calls included, passes through
unchanged.

**`def` with parameters.** `(def name [params] body)` is
`(def name (fn [params] body))`; `(def name value)` is already core.
Any other shape is `bad_def`.

```alchemy
def csv-field [value]
  scalar-text value
```
```core
(def csv-field (fn [value] (scalar-text value)))
```

**`pipe`.** `(pipe init step ...)` threads a value through steps,
data-last: a symbol step `f` becomes `(f acc)`, and a non-empty list
step `(f a b)` becomes `(f a b acc)`, where `acc` is the result so far.
An empty list, a literal, a keyword or a vector is not a step, because
there is nothing to apply, and is `empty_step`, reported at the step.
`(pipe init)` is `init`; a bare `(pipe)` passes through for the checker
to report.

```alchemy
pipe input (select (path "payload" "records" each-index)) (map normalize) (filter active?)
```
```core
(filter active? (map normalize (select (path "payload" "records" each-index) input)))
```

The same, in layout:

```alchemy
pipe input
  select (path "payload" "records" each-index)
  map normalize
  filter active?
```
```core
(filter active? (map normalize (select (path "payload" "records" each-index) input)))
```

```alchemy
pipe x f g
```
```core
(g (f x))
```

```alchemy
pipe x
  f
  (g 1)
  h
```
```core
(h (g 1 (f x)))
```

Steps that are not applicable:

```alchemy
pipe x
  f
  []
```
```core
ERROR:empty_step
```

**`let`, `if`, `match`.** `(let [name value] body)` has one binding and
one body (`bad_let` otherwise); `(if condition then else)` has exactly
two branches (`bad_if`); `(match value (case pattern body) ...)` has
`case` clauses of a pattern and a body (`bad_match`). The forms are left
as they are; patterns stay expressions for the checker and the
interpreter to read.

```alchemy
match v
  case 1 "one"
  case _ "other"
```
```core
(match v (case 1 "one") (case _ "other"))
```

Desugaring is bottom-up, so a `pipe` inside a `let` inside a `def` is
rewritten wherever it sits:

```alchemy
def f [x]
  let [y (pipe x g)]
    if y
      y
      x
```
```core
(def f (fn [x] (let [y (g x)] (if y y x))))
```

A generated node carries the span of the form it came from: the `fn`
above points at its `def`, so a diagnostic about a function the author
never wrote names the line they did.

## The pipe convention

Every operator takes its data **last**: `(map f xs)`, `(get :label
record)`, `(csv options rows)`, `(table-from-json binding input)`. That
is what lets `pipe` append the threaded value and read as a left-to-right
pipeline, and it is the convention the standard library keeps. The
entry point of a program is `def export [input] ...`, applied by the host
to the source stream.

## Meaning

A program is a sequence of `def`s, in any order: every definition sees
every other, so there is no forward-reference problem, and a name bound
nowhere is `unknown_name`. A definition that reaches itself, directly or
through others, is refused (`recursion`): strict mode allows known
combinators, not unrestricted recursion. A function handed itself
recurses through values the resolver cannot see, so evaluation carries a
bound of its own: nested past `MAX_EVAL_DEPTH` (1,000) levels, a form
inside a form, a body inside the call that applied it, a definition's
value inside the form that named it, it is `recursion` too. A chain of
definitions or calls that deep meets the same bound, which is also what
bounds how deep a value a program builds can nest.

```alchemy
def w [f] (f f)
def export [input]
  let [x (w w)]
    json input
```
```check
ERROR:recursion@1:12
```

Locals come from `fn`
parameters, `let` bindings and the bindings a `match` pattern makes;
each shadows what is outside it. A program's definition of a library
name (`csv`, say) shadows the library's for that program and for nothing
the library itself does: the library's definitions resolve in their own
scope.

Values are evaluated eagerly: a `record`, a `path`, a `map` over a
vector. Streams and texts are **plans** (spec section 10.3): evaluating
`route`, `scan-emit`, `map` over a stream, `concat`, `join` or `csv`
builds a description of how to produce the result and consumes nothing.
The runtime lowers the plan `export` answers to a chain of transduce and
render sinks once, and the host pushes the source's events through it.

**Patterns.** In `(match value (case pattern body) ...)`, `_` matches
anything; a symbol naming a native constant (`table-end`, `no-schema`,
`missing`, the container events `object-start`, `object-end`,
`array-start` and `array-end`) or a definition of the program compares
equal to it; any other symbol binds; a keyword, string, number, boolean
or `null` compares; a vector matches a vector of the same length, item
by item; `(constructor pattern ...)` matches the tagged value that
constructor makes (`selected`, `schema`, `row`, `ready`, `transition`,
`entry`, `key`, `scalar`), field by field. A `match` no case takes is
`no_match`. `if` decides by a boolean and nothing else: no value is
implicitly true or false.

## Types and ownership

The checker (`rs/src/check.rs`) infers a type for every definition,
conservatively: what it cannot decide is `Unknown` and passes; what it
can see is wrong is reported before anything runs. `Value` is any data,
so where one kind of data is wanted (a `Record`, a `Vector<Value>`, a
`Number`) a `Value` passes, since it may be that kind, and the runtime
checks it; a type that can never be it (a `Number` where a `Record` is
wanted) is reported. The types (spec section 10.2):

| Type | What it is | Affine |
|---|---|---|
| `Value` | any data a document holds: `Null`, `Bool`, `Number`, `String`, `Vector<T>`, `Record`, `missing` | |
| `Keyword`, `Selector`, `CaptureSpec`, `Fn(params -> result)` | a program's own values | |
| `Vector<T>` | a finite retained collection; it cannot hold a stream | |
| `Stream<T>` | an ordered single-use sequence of items | yes |
| `TableEvents` | `Stream<TableEvent>`: `schema`, `row`s, `table-end`, the `TableRows/1` protocol as tagged values | yes |
| `Stream<Event>` | the source's own events as items, what `events` yields: `object-start`, `object-end`, `array-start`, `array-end`, `(key name)`, `(scalar value)` | yes |
| `JsonEvents` | the single-use source, `JsonEvents/1` | yes |
| `Text` | single-use incremental text | yes |
| `String` | a finite retained string; where a text is wanted, a string lifts to one | |

A binding of an affine type is **consumed at most once** in its scope
(`STREAM_REUSED`, finer code `reused`); the two arms of an `if` and the
cases of a `match` are alternatives, not two uses. A `fn` body may not
use an affine binding of the scope around it (`captured`): a function
may run more than once, and a stream is consumed once. A record, a
partial application and a state cannot hold a stream or a text
(`type_mismatch`). A vector cannot hold a stream either; it may hold a
text, since a text may be finite (the library's `csv-row` joins the
texts a `map` answers), and the runtime refuses a live one where the
vector is built, naming it a live text.

```alchemy
def export [input] (concat (json input) (json input))
```
```check
ERROR:reused@1:47
```

```alchemy
def export [input]
  concat-map (fn [x] (json input)) (select (path each-index) input)
```
```check
ERROR:captured@2:28
```

A stream passed to a definition, or to a `fn` applied to it, is affine
in that body as well: the checker types the body again with its
parameters bound to the arguments' types, and reports a second use, a
capture or a value holding it where the body does it, however many
definitions the stream went through, to `MAX_APPLIED` (32) of them.
Past that bound, and through a function value (a `partial`, a
parameter), the runtime's guard is what holds: a live plan is lowered
once, so `concat` refuses two live texts, and a vector or a finite text
refuses a live one.

```alchemy
def g [s] (concat-map (fn [x] (json s)) [1])
def export [input] (g input)
```
```check
ERROR:captured@1:37
```

Arguments are checked against a native's signature, a library
definition's declared signature, or what a program definition's body
inferred: a wrong count is `arity`, a wrong kind `type_mismatch`, and one
protocol where another was wanted `protocol_mismatch` (`csv` given
`JsonEvents`, `table-from-json` given `Text`, `json` given a stream of
items):

```alchemy
def export [input] (csv csv-options input)
```
```check
ERROR:protocol_mismatch@1:37
```

A function given to `map`, `filter` or `concat-map` by name (a
definition, or a `partial` of one) is checked as the `fn` that calls it
would be, its parameter against the items of the data, and the failure
names the data. Items typed `Value`, as a `select`'s are, pass as they
would in the `fn`; a native, or a `partial` of one, is checked by its
arity here and its own checks run on each item at run time.
`public-column` takes a record, so this is `type_mismatch` as
`(map (fn [x] (public-column x)) [1])` is:

```alchemy
def bad (map public-column [1])
def export [input] (json input)
```
```check
ERROR:type_mismatch@1:28
```

**Strict mode**, the only mode: the function given to `map`, `filter`
or `concat-map` over a stream, and the step and finish of `scan-emit`,
must resolve statically to a `fn`, a definition, a native, or a
`partial` of one, so the plan can be analyzed; a function obtained at
run time is `STREAMABILITY_UNKNOWN` (`dynamic`). Over a finite vector
anything goes.

```alchemy
def go [f input] (concat-map f (select (path each-index) input))
def export [input] (go text input)
```
```check
ERROR:dynamic@1:30
```

**The entry.** `def export [input]` takes one parameter, the source's
`JsonEvents`, and its result decides the output: a `Text` (or a string)
is written as it is; `TableEvents` are rendered by the host (CSV by
default, or JSON records with `--render json`); `JsonEvents` are
rendered as JSON. A program without one is `no_export`; a result of
another type is `bad_output`; one that cannot be typed is
`STREAMABILITY_UNKNOWN` (`unknown_output`).

## The standard library

Every operator takes its data last. The **natives** (`rs/src/stdlib/registry.rs`):

| Operator | Signature | Effect |
|---|---|---|
| `get` | `get key data -> Value` | reads a retained record or object; `missing` for an absent member |
| `get-path` | `get-path path data -> Value` | walks a retained value by one concrete path; `missing` where the path leaves it |
| `as-path` | `as-path data -> Selector` | validates a data-supplied array of segments; never reads data as source |
| `as-vector` | `as-vector data -> Vector` | a captured array as a vector; `INPUT_INVALID` otherwise |
| `record`, `entry` | `record entry... -> Record`, `entry :key value -> Entry` | a retained record; duplicate keys are `duplicate_key` |
| `vector` | `vector item... -> Vector` | a retained vector; it cannot hold a stream or a live text |
| `push` | `push item vector -> Vector` | a new vector with the item appended; it cannot hold a stream or a live text |
| `pop` | `pop vector -> Vector` | the vector without its last item; an empty vector is a type error |
| `top` | `top vector -> Value` | the last item; an empty vector is a type error |
| `count` | `count vector -> Number` | how many items the vector holds |
| `kind` | `kind value -> Keyword` | the kind of a value as a keyword: `:null`, `:boolean`, `:number`, `:string`, `:keyword`, `:vector`, `:record`, `:missing`, `:tagged`, `:function`, `:selector` or `:capture`; a stream or a text cannot be asked |
| `path` | `path segment... -> Selector` | a selector from strings, indexes and selectors |
| `root`, `each-index`, `each-member` | `-> Selector` | the document; every element of an array; every member value of an object |
| `property`, `index`, `compose` | `property name`, `index n`, `compose outer inner -> Selector` | one member; one element; inner below every location outer names |
| `capture` | `capture :tag selector [:limit] -> CaptureSpec` | materialize each selected scope under `max_capture_bytes`, or under the `Limits` field the keyword names (`:max_metadata_bytes`, `:max_record_bytes`), the host's value for it |
| `route` | `route captures input -> Stream<Selected>` | one pass, a shared prefix matcher; retains one selected scope at a time; captures may not overlap |
| `select` | `select selector input -> Stream<Value>` | route with one capture, delivering the values |
| `events` | `events input -> Stream<Event>` | every event of `JsonEvents` as one item, as it arrives: the container events as constants, `key` and `scalar` with their one field; `End` ends the stream and is no item; nothing is retained between events |
| `scan-emit` | `scan-emit init step finish stream -> Stream<Output>` | retains its initial state and the state the step returns, measured when the stage is built and as the state changes, through every closure, partial and finite text it holds (a text's items and the function its `concat-map` applies included): at most `max_metadata_bytes`, no deeper than `max_depth`, reported in `retained_bytes_high`; ready after each item; finish runs once at the validated end |
| `transition` | `transition state outputs -> Transition` | one step's result: the next state and a vector of outputs |
| `partial` | `partial f arg... -> Fn` | `f` with its first arguments supplied |
| `map`, `filter` | `map f items`, `filter predicate items -> Vector \| Stream` | eager over a vector; per item over a stream, retaining nothing |
| `concat-map` | `concat-map f items -> Text` | `f` answers a string or a text per item; each item's text is assembled whole, under `max_output_bytes`, and written as items arrive, so a failure leaves no half item |
| `join` | `join separator items -> Text` | the separator between items, never between the fragments of one; each item assembled as `concat-map`'s is |
| `concat` | `concat item... -> Text` | in order, without assembling the result |
| `text` | `text string -> Text` | a string as a text |
| `replace-text` | `replace-text from to text -> Text` | a fixed literal replaced across fragment boundaries, a finite text's as a live one's; retains at most the literal's length |
| `scalar-text` | `scalar-text options cell -> String` | a cell's text under the options' null and missing policies: a string as it is, a number by its lexeme, a boolean by its name, a vector or a record as its compact JSON text (number lexemes kept, quotes as JSON writes them) under `max_scalar_bytes`; the native renderer writes the same cell the same way |
| `quoted` | `quoted string -> String` | the double-quoted form: a leading and a trailing quote, the quote and the backslash escaped by a backslash, U+0000 to U+001F as `\n`, `\t`, `\r`, `\b`, `\f` or `\u00XX`, and U+007F to U+009F as `\u00XX` (the JSON string form, which YAML's double-quoted style reads too, plus the C1 controls its printable set excludes) |
| `repeat` | `repeat count string -> String` | the string `count` times over; refused past `max_scalar_bytes`, before it is built |
| `fail` | `fail message -> Never` | `INPUT_INVALID` with the message and the form's position |
| `is-ready`, `require-columns` | `is-ready state -> Bool`, `require-columns state -> Vector<Column>` | whether the state holds columns; the columns, or `INPUT_ORDER_VIOLATION` |
| `schema`, `row`, `table-end` | `schema columns`, `row cells`, `table-end -> TableEvent` | the table's one schema; one row, as wide as the schema; the end, after the source validated |
| `ready`, `no-schema`, `selected` | `ready columns -> State`, `no-schema -> State`, `selected :tag value -> Selected` | the state once the metadata is bound; the state before it; what `route` delivers |
| `missing` | `missing -> Value` | an absent member, distinct from `null` |
| `object-start` | `object-start -> Event` | an object begins |
| `object-end` | `object-end -> Event` | an object ends |
| `array-start` | `array-start -> Event` | an array begins |
| `array-end` | `array-end -> Event` | an array ends |
| `key` | `key name -> Event` | the name of the member whose value follows, inside an object |
| `scalar` | `scalar value -> Event` | one scalar of the source: `null`, a boolean, a number with its lexeme, or a string |
| `json` | `json events -> Text` | `JsonEvents` as compact JSON text, event by event, with a final newline |
| `records` | `records table-events -> JsonEvents` | one object per row keyed by label; retains the labels |
| `csv-table` | `csv-table options events -> TableEvents` | the events unchanged, validated as the CSV renderer validates them: one schema first, of at least one column and at most `max_columns`, labels strings, numbers or booleans; rows as wide as the schema; one `table-end`; a delimiter that holds the quote, a line break or NUL is refused before anything runs |

Selecting, capturing and scanning:

```alchemy
select (path "items" each-index) input
```
```canonical
(select (path "items" each-index) input)
```

```alchemy
route [(capture :meta (path "meta")) (capture :row (path "rows" each-index))] input
```
```canonical
(route [(capture :meta (path "meta")) (capture :row (path "rows" each-index))] input)
```

```alchemy
scan-emit no-schema (partial table-step binding) table-finish selected
```
```canonical
(scan-emit no-schema (partial table-step binding) table-finish selected)
```

The source's events as items, for a program that reads the document one
event at a time rather than by selection:

```alchemy
scan-emit [] step finish (events input)
```
```canonical
(scan-emit [] step finish (events input))
```

Values and the table protocol:

```alchemy
get-path (as-path ["account" "balance"]) row
```
```canonical
(get-path (as-path ["account" "balance"]) row)
```

```alchemy
transition (ready columns) [(schema (map public-column columns))]
```
```canonical
(transition (ready columns) [(schema (map public-column columns))])
```

The stack a renderer over the events keeps, one keyword marker per open
container, and the string forms a line of its output is made of:

```alchemy
push :object (pop stack)
```
```canonical
(push :object (pop stack))
```

```alchemy
repeat (count stack) "  "
```
```canonical
(repeat (count stack) "  ")
```

```alchemy
quoted (top stack)
```
```canonical
(quoted (top stack))
```

`pop` and `top` of an empty vector are type errors naming the operator,
where the plan's evaluation reaches them as at run time:

```alchemy
def export [input] (let [x (pop [])] (json input))
```
```check
ERROR:type_mismatch@1:28
```

The text algebra, and the renderers:

```alchemy
concat "[" (join "," (map csv-field cells)) "]"
```
```canonical
(concat "[" (join "," (map csv-field cells)) "]")
```

```alchemy
replace-text "a" "b" text
```
```canonical
(replace-text "a" "b" text)
```

```alchemy
scalar-text csv-options cell
```
```canonical
(scalar-text csv-options cell)
```

```alchemy
records (table-from-json api-binding input)
```
```canonical
(records (table-from-json api-binding input))
```

```alchemy
fail "Required metadata was not found"
```
```canonical
(fail "Required metadata was not found")
```

### The library's own definitions

The rest of the library is written in alchemy, embedded from
[`stdlib/table.alc`](../stdlib/table.alc) and
[`stdlib/csv.alc`](../stdlib/csv.alc), and checked against its declared
signatures as it loads. The blocks below are those files' text, word for
word (`rs/tests/spec_test.rs` holds the two together), and they are the
reference: the runtime runs `table-from-json` and `csv` natively when
their arguments have the standard shapes, and `rs/tests/stdlib_test.rs`
proves the native path and this text produce the same bytes, or fail
with the same code.

They are the spec's definitions (sections 12.2, 13.1 and 13.2) with
three differences. The spec writes a `let` as a line `let columns` over
two child lines, the value and the body; here `let` has one shape,
`let [name value] body`, and the library is written in it. The two
captures of `table-from-json` name the limits the native table holds the
same scopes to (`:max_metadata_bytes`, `:max_record_bytes`), where the
spec's generic `capture` would take `max_capture_bytes`. And `csv`
passes its events through `csv-table`, the protocol validator section
13.2 asks for, so a stream that breaks the table protocol fails as the
native renderer fails it rather than being printed.

The metadata-first table transducer, `JsonEvents` in, `TableEvents` out:

```alchemy
def public-column [column]
  record
    entry :label (get :label column)
```
```core
(def public-column (fn [column] (record (entry :label (get :label column)))))
```

```alchemy
def table-step [binding state event]
  match event
    case (selected :columns raw)
      if (is-ready state)
        fail "Metadata selected more than once"
        let [columns (map (get :column binding) (as-vector raw))]
          transition (ready columns)
            vector
              schema
                map public-column columns
    case (selected :row raw)
      let [columns (require-columns state)]
        transition state
          vector
            row
              map
                fn [column]
                  get-path (get :source column) raw
                columns
```
```core
(def table-step (fn [binding state event] (match event (case (selected :columns raw) (if (is-ready state) (fail "Metadata selected more than once") (let [columns (map (get :column binding) (as-vector raw))] (transition (ready columns) (vector (schema (map public-column columns))))))) (case (selected :row raw) (let [columns (require-columns state)] (transition state (vector (row (map (fn [column] (get-path (get :source column) raw)) columns)))))))))
```

```alchemy
def table-finish [state]
  if (is-ready state)
    vector
      table-end
    fail "Required metadata was not found"
```
```core
(def table-finish (fn [state] (if (is-ready state) (vector table-end) (fail "Required metadata was not found"))))
```

```alchemy
def table-from-json [binding input]
  pipe input
    route
      vector
        capture :columns (get :columns binding) :max_metadata_bytes
        capture :row (get :rows binding) :max_record_bytes
    scan-emit no-schema (partial table-step binding) table-finish
```
```core
(def table-from-json (fn [binding input] (scan-emit no-schema (partial table-step binding) table-finish (route (vector (capture :columns (get :columns binding) :max_metadata_bytes) (capture :row (get :rows binding) :max_record_bytes)) input))))
```

The binding is a record of `:columns` (the selector of the metadata
array), `:rows` (the selector of each row) and `:column` (a function from
one descriptor to a column record with `:label` and `:source`). Rows
that begin before the metadata has completed are `INPUT_ORDER_VIOLATION`;
a document with no rows is a valid empty table.

The always-quoted CSV renderer, `TableEvents` in, `Text` out:

```alchemy
def csv-options
  record
    entry :delimiter ","
    entry :newline "\r\n"
    entry :header true
    entry :null-text ""
    entry :missing :error
```
```core
(def csv-options (record (entry :delimiter ",") (entry :newline "\r\n") (entry :header true) (entry :null-text "") (entry :missing :error)))
```

```alchemy
def csv-field [options cell]
  concat
    "\""
    replace-text "\"" "\"\""
      scalar-text options cell
    "\""
```
```core
(def csv-field (fn [options cell] (concat "\"" (replace-text "\"" "\"\"" (scalar-text options cell)) "\"")))
```

```alchemy
def csv-row [options cells]
  concat
    join (get :delimiter options)
      map
        fn [cell]
          csv-field options cell
        cells
    get :newline options
```
```core
(def csv-row (fn [options cells] (concat (join (get :delimiter options) (map (fn [cell] (csv-field options cell)) cells)) (get :newline options))))
```

```alchemy
def csv [options events]
  concat-map
    fn [event]
      match event
        case (schema columns)
          if (get :header options)
            csv-row options
              map
                fn [column]
                  get :label column
                columns
            ""
        case (row cells)
          csv-row options cells
        case table-end
          ""
    csv-table options events
```
```core
(def csv (fn [options events] (concat-map (fn [event] (match event (case (schema columns) (if (get :header options) (csv-row options (map (fn [column] (get :label column)) columns)) "")) (case (row cells) (csv-row options cells)) (case table-end ""))) (csv-table options events))))
```

The native path substitutes transduce's `TableFromJson` for
`table-from-json` when the binding has the standard shape, and render's
`CsvRenderer` for `csv` when every option maps onto the renderer's
dialect (`:delimiter` one character, `:newline` CRLF or LF, `:header` a
boolean, `:null-text` a string, `:missing` `:error` or a string); any
other options run the text above. The two paths agree on more than the
standard shapes, and `rs/tests/stdlib_test.rs` pins each agreement:

- **the protocol**: a stream that has rows before its schema, a second
  schema, a row wider or narrower than the schema, no `table-end` or a
  second one, an item that is not a table event, or no items at all is
  `PROTOCOL_ORDER_ERROR` both ways;
- **what has no CSV form**: a schema of no columns (a document whose
  metadata array is empty, say) and a delimiter that holds the quote, a
  line break or NUL are `TARGET_VALUE_UNREPRESENTABLE` both ways;
- **labels**: one policy for every table, a string as it is, a number by
  its lexeme, a boolean by its name, an absent one `MISSING_VALUE` and
  anything else `INPUT_INVALID`; a column function that answers
  something other than a record fails as `get` does;
- **cells**: a vector or a record is its compact JSON text, number
  lexemes kept, under `max_scalar_bytes`;
- **limits**: the metadata is held to `max_metadata_bytes`, each row to
  `max_record_bytes` and the schema to `max_columns`, and a failure names
  the same limit both ways; `max_capture_bytes` decides neither table;
  both count each row once in `metrics.rows`, by the last table stage
  it passes (the host's renderer, a `csv`, or a `csv-table` whose rows
  reach no later one), however the stages compose: a `map`, a `filter`
  or a `scan-emit` between two of them hands the count on, so a row a
  filter drops is not counted, and a table's events made into a text
  some other way (a `join` over a `map` of them) count no rows at all.

One difference remains, in a shape the standard binding never reaches,
and is pinned: metadata selected twice (a `:columns` selector naming
several locations) is `INPUT_ORDER_VIOLATION` natively, since a table has
one schema, and `INPUT_INVALID` interpreted, the text's own `fail`.

## Programs

The spec's worked example (sections 5, 12.1 and 13.4): an application
binding, the standard table, the standard renderer. `check` accepts it,
`explain` prints the report shown, and `run` over the spec's document
prints `"Identifier","Full name","Balance"`, `"123","Alice","50.25"` and
`"456","Bob","72"`, each record ending in CRLF, with Bob's cells in the
schema's order although his members are in another.

```alchemy
def column-from-meta [source]
  record
    entry :label (get "title" source)
    entry :source
      as-path
        get "path" source

def api-binding
  record
    entry :columns
      path "response" "metadata" "fields"
    entry :rows
      path "response" "payload" "deep" "records" each-index
    entry :column column-from-meta

def api-table [input]
  table-from-json api-binding input

def export [input]
  pipe input
    api-table
    csv csv-options
```
```check
export: api-table → csv

Source reads:          1
Protocol:              JsonEvents/1 → TableRows/1 → Text
Selection:             shared prefix matcher, two capture routes
Duplicate members:     rejected in captured scopes (DUPLICATE_MEMBER)
Retained metadata:     .response.metadata.fields, capped at max_metadata_bytes
Row capture:           one .response.payload.deep.records[*], capped at max_record_bytes
Output order:          schema first; cells in schema order
Ordering contract:     metadata completes before first row begins
Contract verification: runtime
CSV quoting:           always
External storage:      disabled

Guarantee:
  Memory is independent of the number of rows under the configured
  depth, metadata, record, scalar/key and output limits.

Qualification:
  Valid JSON that violates the metadata-first contract is rejected.
  A later error can occur after earlier output has been written.
```

A program that reads the source's events one by one, `events` in place
of a selection. A renderer of a document of any nesting, a YAML-like
block form say, is a `scan-emit` over them whose state is the stack of
open containers, one keyword marker each, kept with `push`, `pop`, `top`
and `count`, its indentation `repeat`ed from the stack's height and its
strings `quoted`; a step answers several items for one event, and
`join ""` writes them (`rs/tests/events_test.rs` holds one). The
smallest such program, the keys of every object one per line, and its
report: `events` retains nothing, and the state is the stage's own:

```alchemy
def key-lines [s event]
  match event
    case (key name) (transition s [(quoted name) "\n"])
    case _ (transition s [])

def export [input]
  join ""
    scan-emit [] key-lines (fn [s] []) (events input)
```
```check
export: events → scan-emit → join

Source reads:          1
Protocol:              JsonEvents/1 → Stream<Event> → Stream<Value> → Text
Selection:             none; every event is delivered as an item
Duplicate members:     preserved; the events are copied as they arrive, not mapped by key
Retained state:        what the step returns, no deeper than max_depth, capped at max_metadata_bytes
Output order:          source order
Ordering contract:     each item before its outputs
Contract verification: static
External storage:      disabled

Guarantee:
  Memory is independent of the number of rows under the configured
  depth, capture, metadata, scalar/key and output limits: the state the
  step returns is capped at max_metadata_bytes. What one step computes
  is the program's, bounded by the host's abort flag.

Qualification:
  A later error can occur after earlier output has been written.
```

A program that leaves the table to the host (`def export [input]
(api-table input)`) is rendered as CSV by default, or as JSON records
with `--render json`. The echo, whose result is the source's own events:

```alchemy
def export [input]
  json input
```
```check
export: json

Source reads:          1
Protocol:              JsonEvents/1 → Text
Selection:             none; every event passes through
Duplicate members:     preserved; the events are copied as they arrive, not mapped by key
Output order:          source order
Ordering contract:     none
Contract verification: static
JSON profile:          compact, one document, trailing newline
External storage:      disabled

Guarantee:
  Memory is independent of the document's size under the configured
  depth, scalar/key and output limits: nothing is retained beyond the
  renderer's nesting stack.

Qualification:
  A later error can occur after earlier output has been written.
```

## explain

`alchemy explain FILE` prints the plan report in the layout of spec
section 15.5, computed from the plan the program built rather than from
its text: the chain of calls on `export`'s data-last spine; the source
reads (one pass); the protocol chain; the selection (how many capture
routes share the one matcher); what happens to a member name an object
repeats (spec 18.2: rejected in captured scopes under the default
policy, or the first or last value winning under `with_duplicates`, and
preserved where the events are copied as they arrive, as the `json` echo
copies them, which is not a unique-key mapping); each live retention
requirement with the selector it holds and the `Limits` field that caps
it, summed rather than maxed, since the standard table holds its
metadata and one row at the same time; the output order; the ordering
contract and whether the run checks it (`runtime`) or the plan cannot
break it (`static`); the renderer's profile; external storage (always
disabled); then the guarantee and its qualification. A `scan-emit`
state is capped at `max_metadata_bytes` and `max_depth`; what one step
computes is the program's, so it makes the guarantee conditional, and
the report says so. `events` retains nothing and selects nothing: every
event is delivered as an item, and the report says that too. A result
that never reaches the input, a text of
the program's own, is reported as such: the input is read and validated
and nothing of it is used, and nothing is written before it has
validated.

```alchemy
def export [input] "done"
```
```check
export: a text of its own

Source reads:          1
Protocol:              Text
Selection:             none; the input is read and validated, and nothing of it is used
Duplicate members:     not examined; the input is only validated
Output order:          the program's own text, written once the input has validated
Ordering contract:     none
Contract verification: static
External storage:      disabled

Guarantee:
  Memory is independent of the document's size under the configured
  depth and scalar/key limits: nothing of the input is kept. The text
  is the program's own, written under the output limit.

Qualification:
  The text is written only after the whole input has validated; an
  invalid input writes nothing.
```

`Program::explain_json` is the same information as one object
(`chain`, `finite`, `protocol`, `selection`, `duplicates`, `retention`,
`readiness`, `order_constraints`, `renderer`, `confidence`,
`guarantee`, `qualification`, ...), for a host's `--explain`. A CSV
`renderer` carries the dialect it is built with (`delimiter`, `newline`,
`header`, `null_text`, and `missing`, `"error"` or `"text"` with its
`missing_text`, `null` when there is none): the program's own options
when its `csv` runs natively, the defaults when the host renders a
table.

## run

```text
alchemy run [--render csv|json] [--no-native] [--max-output-bytes N] PROGRAM INPUT
```

Parses `INPUT`, a JSON document, with the tabnas JSON grammar through
transduce's `ParserSource`, incrementally, pruning the parsed tree under
the program's row selector when the plan knows one, and pushes the
events through the program's sink to standard output, coalesced and
flushed at the end. `--render` chooses how a table or JSON-events result
is rendered (a program that renders its own text takes none:
`render_of_text`); `--no-native` runs the standard compositions through
the library's text; `--max-output-bytes` sets the writer's
`max_output_bytes`, which also bounds what a finite text or one item's
text may hold as it is built (the other limits are transduce's
defaults). Other input formats are aless's business: it runs alchemy
programs against every format its parsers read.

A failure is one `Fail` JSON object on standard error, with `output:
"partial"` when bytes had already reached standard output, and the exit
status follows the code: 2 for a program that does not read, resolve or
check (`DSL_PARSE_ERROR`, `DSL_TYPE_ERROR`, `STREAM_REUSED`,
`STREAMABILITY_UNKNOWN`), for a usage error and for an unreadable file;
1 for an input or protocol failure (`INPUT_INVALID`,
`INPUT_ORDER_VIOLATION`, `MISSING_VALUE`, `PROTOCOL_ORDER_ERROR`, the
rest); 5 for `RESOURCE_LIMIT_EXCEEDED`; 3 for `OUTPUT_FAILED`; 6 for
`ABORTED`.

## Embedding

The crate's API is small and every part of it is `Send`, because a host
runs a pipeline on the thread that parses:

```rust
use std::sync::Arc;
use tabnas_alchemy::{compile, Output, Renderer};
use tabnas_transduce::{JsonEvent, Limits, Metrics, Sink};

let program = compile("def export [input] (json input)", "echo.alc")?;
assert_eq!(program.output(), Output::Text);
assert!(program.row_selector().is_none());
let metrics = Metrics::new();
let mut sink = program.sink(Box::new(Vec::new()), None, &Limits::default(), metrics)?;
sink.event(JsonEvent::ArrayStart)?;
sink.event(JsonEvent::ArrayEnd)?;
sink.event(JsonEvent::End)?;
let _ = Renderer::Json;
# Ok::<(), tabnas_transduce::Fail>(())
```

- `compile(src, file)` parses, desugars, resolves and checks, then
  builds the plan: it applies `export` to the input's plan, which
  evaluates every value a stream does not defer (definitions, records,
  selectors, `let` values on `export`'s own path) and consumes nothing.
  A failure is a `Fail` with the code and the finer code, at the
  position it names in `file`. Since building the plan runs the program's
  definitions, a failure that evaluation finds on that path comes back
  from `compile` with its own code: a `fail` is `INPUT_INVALID` (exit 1
  from `alchemy check`, although no document was read), a record with a
  key twice `duplicate_key`, a `match` no case takes `no_match`. The
  work is bounded: at most `MAX_PLAN_STEPS` (1,000,000) evaluation steps,
  `RESOURCE_LIMIT_EXCEEDED` naming `max_plan_steps` past them, and at
  most `MAX_EVAL_DEPTH` (1,000) levels of nesting, `recursion` past them.
  It runs on a thread of `STACK_BYTES` (64 MiB) whatever thread calls it,
  so those bounds hold in a debug build as in a release one.
- `Program::output()` is what the program produces (`Text`,
  `TableRows`, `JsonEvents`), so the host knows whether `--render`
  applies; `row_selector()` is the selector under which the source is
  read one row at a time, when the plan knows one (the table binding's
  `:rows`, a `select`'s selector, the one multi-location capture of a
  `route`; none for `events`, which needs every event), for the host's
  pruning choice.
- `explain()` and `explain_json()` are the report.
- `sink(out, render, limits, metrics)` returns the `Sink + Send` the
  host pushes `JsonEvent`s into, then one `End`; the output reaches
  `out` through a coalescing writer that enforces `limits.
  max_output_bytes` and counts `output_bytes` in `metrics`, and is
  flushed at `End`. `sink_out` takes any `TextOut` instead. `render` is
  `Some(Renderer::Csv | Renderer::Json)` for a table or JSON-events
  result; for a `Text` result it must be `None` (`render_of_text`).
- A failure comes back from the event call that found it, with
  `committed_output` set when text this crate wrote had reached the
  writer. A failure the source found (invalid JSON after the rows) never
  passes through the sink, so the host reads `metrics.output_bytes`
  after the run and marks the failure as partial when it is not zero;
  the `run` command does exactly that.
- `with_native(false)` runs the standard compositions through the
  library's text; `with_duplicates` sets the policy for repeated member
  names in captured values (rejected by default, spec section 18.2).
- `with_abort(flag)` hands the program the host's `AbortFlag`: the
  program's own functions read it every few evaluation steps, so a long
  computation on one item stops with `ABORTED` rather than finishing the
  item first. The host gives the source the same flag
  (`ParserSource::abort`), which stops between events.
- The per-item functions run on the thread that pushes the events, and
  that thread needs `STACK_BYTES` of stack for the evaluation bound to be
  reached before the stack's end (the `alchemy` command runs on such a
  thread; aless's parse thread is one).
- What bounds a run is the host's `Limits`, each where the stage holding
  the data names it: the source's `max_depth`, `max_key_bytes` and
  `max_scalar_bytes`; the tables' `max_metadata_bytes`,
  `max_record_bytes` and `max_columns`; a program's `capture` under
  `max_capture_bytes` or the limit it names; a `scan-emit` state under
  `max_metadata_bytes` and `max_depth`; a cell's JSON text under
  `max_scalar_bytes`; and the writer's `max_output_bytes`, which also
  bounds a finite text and one item's text as they are built. A host
  that runs programs it did not write sets `max_output_bytes` and a
  timeout through the abort flag.

## Spans and errors

Every form carries a `SourceSpan`: the file name and the byte range of
its text. Rows and columns (1-based, columns in characters) are derived
from the source when a diagnostic needs them, and a generated node
carries the span of the form it came from. A failure raised inside the
standard library carries no row or column of its own, since a row there
would name a line of the user's file that says something else; its
message ends with the library file, row and column instead,
`Required metadata was not found (at stdlib/table.alc:38:5)`, and when
it passes a form of the program on its way out (a native the program
called applied the library's function), that form's row and column are
the failure's. Library spans are told apart from the program's by the
file they were read from, not by its name, so a program that happens to
be named `stdlib/table.alc` is still read as its own text.

Failures are `tabnas_transduce::Fail` values. The message begins with the
finer code and a colon, `bad_dedent: dedent to a level no block opened,
before: c`, so a script can branch on the word before the first `: `,
and `row` and `col` name the offending form. A code is never renamed or
repurposed; one may be added.

| Code | Finer codes |
|---|---|
| `DSL_PARSE_ERROR` | the reader's `tab_indent`, `bad_indent`, `bad_dedent`, `unbalanced`, `too_deep`; the desugarer's `empty_step`, `bad_def`, `bad_let`, `bad_if`, `bad_match`; the engine's `unterminated_string`, `unprintable`, `unexpected` |
| `DSL_TYPE_ERROR` | the resolver's `unknown_name`, `not_def`, `duplicate_def`, `reserved`, `bad_fn`, `misplaced_def`, `bad_pattern`; the checker's `arity`, `type_mismatch`, `protocol_mismatch`, `no_export`, `bad_output`; the runtime's `duplicate_key`, `no_match`, `render_of_text` |
| `STREAM_REUSED` | `reused`, `captured` |
| `STREAMABILITY_UNKNOWN` | `recursion`, `dynamic`, `unknown_output` |

Runtime failures carry the transduce and render codes unchanged
(`INPUT_ORDER_VIOLATION`, `MISSING_VALUE`, `RESOURCE_LIMIT_EXCEEDED` with
the limit's name, `PROTOCOL_ORDER_ERROR`, ...); a `fail "message"` in a
program is `INPUT_INVALID` with the message and the form's position. A
failure's message names a value by its kind and a short prefix of its
text (`no_match: no case matches a vector ([...)`), never the whole of
it, so a failure over a large document does not carry the document.

## The command

```text
alchemy canon FILE       print the program in canonical form
alchemy format FILE      print the program in layout form
alchemy check FILE       parse, desugar, resolve, check and build the plan; print nothing and exit 0
alchemy explain FILE     print the plan report
alchemy run [--render csv|json] [--no-native] [--max-output-bytes N] PROGRAM INPUT
                         run the program over the JSON document INPUT
```

`FILE`, `PROGRAM` and `INPUT` may be `-` for standard input (one of them
per run). A failure is the `Fail` as one JSON object on standard error
(`code`, `message`, `row`, `col`, `output`, and `path` and `limit` when
they apply) with the exit status [run](#run) lists; standard output
carries nothing but the answer. A standard error that cannot be
written loses the report, not the status.
