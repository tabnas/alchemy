# The alchemy language

This is the reference for the **reader**: how a program's text becomes
forms, how the forms print, and how the conveniences desugar into the
core forms. The checker, the planner and the interpreter that give the
forms meaning are not written yet; where a later section is needed it is
named as missing rather than sketched. The design this implements is
section 4 of transduce's `docs/architecture.md`.

Every `alchemy` example below, together with the `canonical` or `core`
result that follows it, is a row in [`test/spec/reader.tsv`](../test/spec/reader.tsv)
or [`test/spec/pipe.tsv`](../test/spec/pipe.tsv), and
`rs/tests/spec_test.rs` fails when one is not, so what this page shows is
what the reader does.

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
aside, for every program in the fixtures.

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
as they are; patterns stay expressions for the checker to read.

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
pipeline, and it is the convention the standard library will keep. The
entry point of a program is `def export [input] ...`, applied by the host
to the source stream.

## Programs

A transducer and its renderer, as the design brief and the README write
them. The reader reads these today; running them waits for the checker
and the interpreter.

```alchemy
def api-binding
  record
    entry :columns
      path "response" "metadata" "fields"
    entry :rows
      path "response" "payload" "deep" "records" each-index
    entry :column column-from-meta
```
```canonical
(def api-binding (record (entry :columns (path "response" "metadata" "fields")) (entry :rows (path "response" "payload" "deep" "records" each-index)) (entry :column column-from-meta)))
```

```alchemy
def export [input]
  pipe input
    table-from-json api-binding
    csv csv-options
```
```core
(def export (fn [input] (csv csv-options (table-from-json api-binding input))))
```

```alchemy
def table-step [state selected]
  match (get :tag selected)
    case :columns
      transition (ready state selected) []
    case :rows
      transition state [(row state selected)]
```
```canonical
(def table-step [state selected] (match (get :tag selected) (case :columns (transition (ready state selected) [])) (case :rows (transition state [(row state selected)]))))
```

```alchemy
def table-from-json [binding input]
  pipe input
    route (captures binding)
    scan-emit table-step no-schema
```
```core
(def table-from-json (fn [binding input] (scan-emit table-step no-schema (route (captures binding) input))))
```

```alchemy
def csv-row [values]
  concat
    join ","
      map csv-field values
    (newline)
```
```core
(def csv-row (fn [values] (concat (join "," (map csv-field values)) (newline))))
```

```alchemy
def csv [options rows]
  concat
    csv-row (columns options)
    concat-map csv-row rows
```
```core
(def csv (fn [options rows] (concat (csv-row (columns options)) (concat-map csv-row rows))))
```

## Spans and errors

Every form carries a `SourceSpan`: the file name and the byte range of
its text. Rows and columns (1-based, columns in characters) are derived
from the source when a diagnostic needs them, and a generated node
carries the span of the form it came from.

Failures are `tabnas_transduce::Fail` values with code `DSL_PARSE_ERROR`,
whether the reader or the desugarer found them. The message begins with
the finer code and a colon, `bad_dedent: dedent to a level no block
opened, before: c`, so a script can branch on the word before the first
`: `, and `row` and `col` name the offending form. The finer codes this
crate declares, in the grammar document's `options.error` and
`options.hint`, are `tab_indent`, `bad_indent`, `bad_dedent`,
`unbalanced`, `empty_step`, `bad_def`, `bad_let`, `bad_if` and
`bad_match`; the engine's own codes (`unterminated_string`,
`unprintable`, `unexpected`) pass through in the same position. A code is
never renamed or repurposed; one may be added.

## The command

```text
alchemy canon FILE     print the program in canonical form
alchemy format FILE    print the program in layout form
alchemy check FILE     parse and desugar; print nothing and exit 0
```

`FILE` may be `-` for standard input. A failure is the `Fail` as one JSON
object on standard error (`code`, `message`, `row`, `col`, `output`) with
exit status 2, as is a usage error or an unreadable file; standard output
carries nothing but the answer. `check` will grow the checker's
`DSL_TYPE_ERROR`s when the checker exists; `explain` and `run` join the
command with the planner and the interpreter.
