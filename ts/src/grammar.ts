/* Copyright (c) 2026 tabnas, MIT License */

// The alchemy grammar plugin: the grammar document (embedded from
// `alchemy-grammar.jsonic` at the repository root, the one source every
// runtime embeds), the native actions it names, and the parse entry
// points. A port of rs/src/grammar.rs.
//
// The reader is a grammar over the token stream `./lex` produces. A
// program is a sequence of layout lines; a line is one or more inline
// forms followed by either a `#NL` (another line at this level), a `#DE`
// or `#ZZ` (this level closes), or an `#IN` opening a block of child lines
// closed by `#DE` or `#ZZ`. A line with several inline forms or with
// children is a list whose items are the inline forms then the children; a
// line with exactly one inline form and no children is that form. `(`
// forms `)` is a list, `[` forms `]` a vector, and an atom is a symbol,
// keyword, string, number, `true`, `false` or `null`.
//
// The rules build a tagged value tree, one object per node, that `./ast`
// turns into `Expr` with spans:
//
//   {"$":"list",   "items":[…], "span":[start,end]}
//   {"$":"vector", "items":[…], "span":[start,end]}
//   {"$":"sym",    "name":…,    "span":[start,end]}
//   {"$":"kw",     "name":…,    "span":[start,end]}
//   {"$":"str",    "value":…,   "span":[start,end]}
//   {"$":"num",    "lexeme":…,  "span":[start,end]}
//   {"$":"bool",   "value":…,   "span":[start,end]}
//   {"$":"null",                "span":[start,end]}
//
// `start` and `end` are offsets into the source, from the tokens' `sI`.
//
// How the rules repeat (AGENTS.md, "The reader repeats by replacement"):
// every repetition is a replace loop, never a push per item. The
// container pushes its first item (`p`); the item's close alternate, on a
// token that continues the sequence, replaces the item with the next one
// in the same frame (`r`); the container's close runs once, after the last
// item. The pushes that remain are structure: `line`'s `#IN` into `block`,
// and `form`'s openers into `paren` and `bracket`.
//
// How the rules share nodes: a pushed rule starts on its parent's node,
// and a replacing rule on the replaced one's, so every rule here that
// builds a value of its own first replaces its node (`@alchemy-array`,
// `@alchemy-atom`, the two openers). A replace hands the parent on, so
// every item of a sequence sees its container as `rule.parent`, and each
// item appends its finished value to the container's node in its own
// close action (`@alchemy-item` for a form, `@alchemy-line-end` for a
// line). Nothing reads a finished item back through the push link
// (`rule.child`), which names only the first item of a replace loop.

import { Tabnas, TabnasError } from '@tabnas/parser'
import type { Rule, Context, Token, Plugin } from '@tabnas/parser'
import { make as makeJson } from '@tabnas/json'

import { Expr, programFromValue, sourceFile } from './ast'
import { isFail } from './fail'
import { DE, IN, KW, MATCHER, NL, makeLayoutMatcher } from './lex'
import { Fail } from './shared'

// The file name spans carry when a program is parsed from a string rather
// than a file.
export const UNNAMED = '<program>'

// The action, condition and error references the document names, all
// registered (on the grammar spec, the TypeScript engine's ref map) before
// the document is installed. A test holds this list to the document.
export const A_ARRAY = '@alchemy-array'
export const A_CHILDREN = '@alchemy-children'
export const A_LINE_END = '@alchemy-line-end'
export const A_OPEN_LIST = '@alchemy-open-list'
export const A_OPEN_VECTOR = '@alchemy-open-vector'
export const A_ITEM = '@alchemy-item'
export const A_CLOSE_SEQ = '@alchemy-close-seq'
export const A_ATOM = '@alchemy-atom'
export const C_CHILDREN = '@alchemy-has-children'
export const C_IN_LINE = '@alchemy-in-line'
export const C_IN_PAREN = '@alchemy-in-paren'
export const C_IN_BRACKET = '@alchemy-in-bracket'
export const E_UNBALANCED = '@alchemy-unbalanced'

// What a `line` records once it has taken a block of children: where that
// block's lines begin among the line's items. In `u`, the bag that neither
// descends nor passes to the next line, because it describes this line
// alone.
const U_CHILDREN = 'children'

// --- BEGIN EMBEDDED alchemy-grammar.jsonic ---
const grammarText = `
# alchemy grammar definition: the reader of the alchemy language.
#
# SINGLE SOURCE OF TRUTH for the grammar document, options and rules
# together, so there is one definition of the language rather than two
# halves that can drift. Every runtime embeds this file VERBATIM between
# its \`--- BEGIN/END EMBEDDED alchemy-grammar.jsonic ---\` markers (Rust:
# \`GRAMMAR_TEXT\` in rs/src/grammar.rs) and parses it once at run time.
# Never hand-edit between the markers: edit this file and run
# \`node scripts/embed.js\` (\`make embed\`).
#
# The text is JSON plus \`#\` line comments: every key and every string is
# double-quoted, numbers are JSON numbers, and there are no trailing
# commas. That is valid jsonic, and it is also what the strict JSON
# grammar reads once comment lexing is on, which is how the Rust crate
# reads it without a jsonic dependency (\`tabnas_json\` with
# \`options.comment.lex\`). Keep it in that subset: an unquoted key or a
# single-quoted string here parses in jsonic and fails in Rust. Numbers
# arrive as doubles; a loader puts whole numbers back into integer form
# (\`b\` is an integer field).
#
# The \`@alchemy-*\` names are the actions, conditions, error hooks and the
# lex matcher each runtime registers on the instance BEFORE it installs
# this document, which is what looks them up.
#
# The reader (design brief section 4.1, spec section 9.1) is a grammar
# over the token stream the layout matcher (\`@alchemy-layout\`) produces:
# \`#IN\`, \`#DE\` and \`#NL\` for layout, \`#KW\` for a keyword, \`#TX\` for a
# symbol, \`#ST\`, \`#NR\` and \`#VL\` for strings, numbers and values, \`(\`
# and \`)\` as \`#OP\`/\`#CP\`, \`[\` and \`]\` as \`#OS\`/\`#CS\`.
#
# Every repetition is a replace loop, never a push per item (AGENTS.md,
# "The reader repeats by replacement"): the container pushes its first
# item (\`p\`), the item's close replaces it with the next one in the same
# frame (\`r\`), and the container's close runs once, after the last item.

{
  "options": {
    "rule": { "start": "program" },
    # The engine answers a literally empty source before any rule runs;
    # an empty program is \`[]\`, as it is from the \`program\` rule for a
    # source holding only blank and comment lines.
    "lex": {
      "empty": true,
      "emptyResult": [],
      "match": {
        # Below the first built-in band (1e6): indentation and words are
        # seen before the space, fixed and text matchers.
        "alchemy": { "order": 100000, "make": "@alchemy-layout" }
      }
    },
    # Parens and brackets are the only fixed tokens; the engine's JSON
    # punctuation goes.
    "fixed": {
      "token": {
        "#OB": null, "#CB": null, "#CL": null, "#CA": null,
        "#OP": "(", "#CP": ")"
      }
    },
    # Words are lexed by the layout matcher, whose boundaries are the
    # symbol alphabet; the engine's text, number and value matchers would
    # cut them differently.
    "text": { "lex": false },
    "number": { "lex": false },
    "value": { "lex": false },
    # Double-quoted strings with exactly the JSON escapes, as the json
    # grammar configures them: unknown escapes refused, the engine's
    # structural \`\\xHH\` and \`\\u{...}\` off, and the non-standard \`\\v\`, \`\\'\`
    # and \`\` \\\` \`\` removed (a null entry deletes an escape; \`""\` would map
    # it to the empty string).
    "string": {
      "chars": "\\"",
      "multiChars": "",
      "allowUnknown": false,
      "escapeStrict": true,
      "escape": { "v": null, "'": null, "\`": null }
    },
    # \`;\` to the end of the line, and nothing else.
    "comment": {
      "lex": true,
      "def": {
        "hash": null,
        "slash": null,
        "multi": null,
        "semi": { "line": true, "start": ";", "lex": true, "eatline": false }
      }
    },
    # This grammar's own codes. The first five are raised by the layout
    # matcher; the next five are the desugarer's, which reports them
    # through its failure with the code leading the message, so a fixture
    # pins \`ERROR:<code>\` for either kind. \`too_deep\` is both: the
    # reader's bound on nesting (256, MAX_NESTING), and the desugarer's on
    # what its rewrites add. A form of the wrong shape is the desugarer's
    # DSL_PARSE_ERROR, with its text, wherever it is found: the resolver
    # holds a \`let\`, \`if\` or \`match\` that a \`pipe\` builds to the same
    # shapes (a step's form grows by the threaded value after the
    # desugarer read it), and the later stages' own checks of the shapes
    # fail the same way. The resolver's, the checker's and the runtime's
    # codes follow, declared here too and raised the same way: the code
    # leads the message of a DSL_TYPE_ERROR, a STREAM_REUSED or a
    # STREAMABILITY_UNKNOWN. A finer code comes with the same code wherever
    # it is raised. A message is the text its raising sites write, each
    # \`{name}\` standing for what a site fills in; a code raised in more
    # than one sentence holds one per line, and a situation two stages
    # report is one sentence. The texts are those a program compiled on its
    # own can meet (AGENTS.md, "Error codes", names the few raised only
    # otherwise). Each runtime holds its own raising sites' texts to these
    # (rs: \`the_desugaring_messages_match_the_document\`,
    # \`the_too_deep_texts_name_the_bound\`, and
    # \`the_raised_messages_match_the_document\`, which reads every failure
    # the shared fixtures meet, wants its code and text here and one code
    # for each finer code, and wants every line here met by one).
    "error": {
      "tab_indent": "tab in indentation",
      "bad_indent": "unexpected indentation before: {src}",
      "bad_dedent": "dedent to a level no block opened, before: {src}",
      "unbalanced": "unbalanced delimiter: {src}",
      "too_deep": "nesting deeper than 256 levels",
      "empty_step": "a pipe step must be a symbol or a non-empty list",
      "bad_def": "def takes a name and a value, or a name, [params] and a body",
      "bad_let": "let takes one binding [name value] and one body",
      "bad_if": "if takes a condition and exactly two branches",
      "bad_match": "match takes a value and (case pattern body) clauses",
      # The resolver's.
      "unknown_name": "{name} is not defined",
      "not_def": "a top-level form must be a def",
      "duplicate_def": "{name} is defined twice",
      "reserved": "{name} is a special form and cannot be defined",
      "bad_fn": "fn takes [params] of symbols and one body",
      "misplaced_def": "def is only allowed at the top level",
      "bad_pattern": "{name} is not a constructor; a list pattern is (constructor pattern...)\\na list pattern is (constructor pattern...)",
      # The resolver's and the evaluator's.
      "recursion": "{name} reaches itself through {names}; strict mode refuses recursion\\nevaluation nested past 1000 levels: a function applied to itself, or definitions or calls chained that deep; strict mode refuses recursion without a bound",
      # The checker's, and the runtime's where a value read at run time meets the same refusal.
      # A value both stages describe is named by its kind (\`{kind}\`: \`a stream\`, \`a text\`,
      # \`a number\`), the runtime's words, onto which the checker's types map; a requirement
      # (\`{expected}\`, \`{actual}\`) is in each stage's words, a type at the checker.
      "arity": "{callee} takes {count} argument(s), got {given}\\nthe pattern ({constructor} ...) takes {count} field(s), got {given}\\n{what} takes a function of {count} argument(s), not {given}\\npartial supplies {given} argument(s) to a function of {count}\\nexport takes one parameter, the input, not {given}",
      "type_mismatch": "{what} must be {expected}, not {actual}\\n{what} must answer {expected}, not {actual}\\n{what} was expected, not {actual}\\n{native}: expected a string or a text, not {actual}\\n{what} must name one location, not {selector}\\n{what} cannot hold {kind}; a stream is used once, where it is\\n{what} cannot hold {kind}; a stream or a text is used once, where it is\\nthe state of scan-emit cannot be {kind}\\n{callee} is {kind} and cannot be called\\nan empty list is not a call\\nexport must be a fn [input]\\npop: the vector is empty; there is no last item to remove\\ntop: the vector is empty; there is no last item\\n{native}: the options have no :{key}\\nkind: {kind} cannot be asked; a stream or a text is used where it is, not inspected",
      "protocol_mismatch": "{what} must be {expected}, not {actual}\\n{what} must be a vector or a stream of items, not JsonEvents; select or route what the stream should yield, or read its events\\ncsv renders table events; the program's result is JSON events (render it as json, or make a table of it with table-from-json)",
      "no_export": "the program has no \`def export [input]\`",
      "bad_output": "export answers a Stream<{item}>; render it as a text (join, concat-map), or make table events of it\\nexport answers a {type}; it must answer a text, table events or JSON events",
      "unknown_output": "the result of export cannot be typed; it must be a text, table events or JSON events",
      "reused": "{name} is a stream and is used {count} times; a stream is consumed once",
      "captured": "{name} is a stream and is captured by a fn; a function may run more than once, and a stream is consumed once",
      "dynamic": "{what} must be a fn, a definition, a native or a partial of one, so the plan can be analyzed; strict mode refuses a function obtained at run time",
      # The runtime's, where a value is built or a match is taken.
      "duplicate_key": "record has two entries for :{key}",
      "no_match": "no case matches {value}",
      "render_of_text": "the program renders its own text; --render applies to a table or JSON events result"
    },
    "hint": {
      "tab_indent": "Indentation is spaces only, two per level. Replace the tab with spaces.",
      "bad_indent": "A child line is exactly two spaces deeper than its parent, and the\\nfirst line of a program is not indented.",
      "bad_dedent": "A line may only return to the indentation of a line above it.",
      "unbalanced": "Every ( and [ needs its ) or ], and nothing closes what was not\\nopened. Inside delimiters, line breaks and indentation do not count.",
      "too_deep": "A program nests at most 256 levels, counting a layout line, each\\nindentation level and each open ( or [ as one; a pipe adds a level per\\nstep. Split the form into definitions.",
      "empty_step": "In pipe VALUE STEP..., each step is a symbol (called with the value)\\nor a list (the value is appended as its last argument).",
      "bad_def": "Write def NAME VALUE, or def NAME [PARAMS] BODY.",
      "bad_let": "Write let [NAME VALUE] BODY.",
      "bad_if": "Write if CONDITION THEN ELSE.",
      "bad_match": "Write match VALUE (case PATTERN BODY)...",
      "unknown_name": "A name is a definition of the program, a parameter or a binding in\\nscope, or one of the library's. Define it, or correct its spelling.",
      "not_def": "Each top-level form is a definition: def NAME VALUE, or def NAME\\n[PARAMS] BODY. Put the expression inside one.",
      "duplicate_def": "A program defines each name once. Rename or remove one of the\\ndefinitions.",
      "reserved": "def, fn, let, if, match, case and pipe are special forms. Choose\\nanother name.",
      "bad_fn": "Write fn [PARAMS] BODY: a vector of parameter names, then one body.",
      "misplaced_def": "def defines a name at the top level only. Inside a body, bind a value\\nwith let [NAME VALUE] BODY.",
      "bad_pattern": "A list pattern is (CONSTRUCTOR PATTERN...), its head a constructor\\nsuch as transition, key or scalar; any other pattern is a value, a\\nname or _.",
      "recursion": "Strict mode refuses recursion: a definition may not reach itself, and\\nevaluation nests at most 1000 levels. Repeat over data with map,\\nfilter, concat-map or scan-emit instead.",
      "arity": "Give a function, a native, a constructor pattern or a partial exactly\\nas many arguments as it takes, and a stream operator a function of as\\nmany parameters as it passes; export takes one, the input.",
      "type_mismatch": "A value of one type is given where another is wanted. The message\\nnames the place, what it wants and what came.",
      "protocol_mismatch": "A stream's protocol (JSON events, table events, a stream of items, a\\ntext) must be the one its taker reads: select or route makes items of\\nJSON events, table-from-json makes table events of them, and json or\\ncsv renders them.",
      "no_export": "A program answers through def export [input]: one function of the\\ninput, whose result is the output.",
      "bad_output": "export answers a text, table events or JSON events. Render a stream\\nof items as a text (join, concat-map), or make table events of it.",
      "unknown_output": "The type of export's result must be known when the program is\\nchecked: a text, table events or JSON events, not a value read at run\\ntime.",
      "reused": "A stream is read once, so it is used in one place only. Take\\neverything needed from it in that one pass (route captures several\\nparts at once).",
      "captured": "A fn may run more than once, and a stream is read once, so a fn cannot\\nuse a stream from the scope around it. Pass what it needs as an\\nargument instead.",
      "dynamic": "Strict mode needs to know, when the program is checked, the function\\na stream operator runs. Pass a fn, a definition, a native or a\\npartial of one, not a function read from data.",
      "duplicate_key": "A record holds each key once. Remove or rename the repeated entry.",
      "no_match": "Add a case for the value, or end with (case _ BODY), which takes any\\nvalue.",
      "render_of_text": "A program that renders its own text takes no renderer. Leave --render\\nout, or have export answer table events or JSON events."
    }
  },

  "rule": {
    # A program: layout lines until the source ends. The node is the array
    # of finished lines. The program pushes the first line and the lines
    # follow it in one frame (see \`line\`), so the close runs once, on the
    # end of the source.
    "program": {
      "open": [
        { "s": "#ZZ", "a": "@alchemy-array", "g": "alchemy" },
        { "p": "line", "a": "@alchemy-array", "g": "alchemy" }
      ],
      "close": [
        { "s": "#ZZ", "g": "alchemy" }
      ]
    },
    # A layout line: inline forms, then what ends it. The line pushes its
    # first form, and the forms follow it in one frame up to a layout
    # token or the end of the source (see \`form\`). A block's \`#IN\` is
    # taken here and the block pushed. \`#NL\` is the next line of this
    # level, and so is whatever follows a block that did not also close
    # this level: either way the line appends itself to the level's node
    # and replaces itself with the next line (\`r\`). \`#DE\` and \`#ZZ\` end the
    # level and are left (\`b: 1\`) for the block or program above. A layout
    # token is always followed by a form (the layout matcher reads a
    # line's indentation up to its first form, and a row with nothing else
    # on it is blank), so \`#NL\` never stands right before \`#DE\` or \`#ZZ\`.
    "line": {
      "open": [
        { "p": "form", "a": "@alchemy-array", "g": "alchemy" }
      ],
      "close": [
        { "s": "#IN", "p": "block", "a": "@alchemy-children", "g": "alchemy" },
        { "s": "#NL", "r": "line", "a": "@alchemy-line-end", "g": "alchemy" },
        { "s": "#DE", "b": 1, "a": "@alchemy-line-end", "g": "alchemy" },
        { "s": "#ZZ", "b": 1, "a": "@alchemy-line-end", "g": "alchemy" },
        { "c": "@alchemy-has-children", "r": "line", "a": "@alchemy-line-end", "g": "alchemy" }
      ]
    },
    # The children of a line, one per level: lines until the \`#DE\` that
    # closes the level, or the end of the source. The block runs on its
    # line's array, so each child line appends itself there, after the
    # line's inline forms.
    "block": {
      "open": [
        { "p": "line", "g": "alchemy" }
      ],
      "close": [
        { "s": "#DE", "g": "alchemy" },
        { "s": "#ZZ", "b": 1, "g": "alchemy" }
      ]
    },
    # One form: an explicit list or vector, or an atom. A closer with
    # nothing open, or the end of the source inside a delimiter, arrives
    # here and is \`unbalanced\`.
    #
    # A form is an item of the sequence its parent reads: a line's inline
    # forms, or the items of \`( )\` or \`[ ]\`. Its close appends it to the
    # parent's node and then either ends the sequence, leaving the token
    # that ends it (\`b: 1\`) for the parent, or goes on to the next item in
    # this frame (\`r\`). What ends a sequence is the parent's: the closer
    # for \`paren\` and \`bracket\`, a layout token or the end of the source
    # for a line. The layout tokens only arrive outside delimiters, where
    # the parent is a line; the end of the source inside a delimiter goes
    # on to the next form, whose open reports it.
    "form": {
      "open": [
        { "s": "#OP", "p": "paren", "a": "@alchemy-open-list", "g": "alchemy" },
        { "s": "#OS", "p": "bracket", "a": "@alchemy-open-vector", "g": "alchemy" },
        { "s": "#TX", "a": "@alchemy-atom", "g": "alchemy" },
        { "s": "#KW", "a": "@alchemy-atom", "g": "alchemy" },
        { "s": "#ST", "a": "@alchemy-atom", "g": "alchemy" },
        { "s": "#NR", "a": "@alchemy-atom", "g": "alchemy" },
        { "s": "#VL", "a": "@alchemy-atom", "g": "alchemy" },
        { "s": "#CP", "e": "@alchemy-unbalanced", "g": "alchemy" },
        { "s": "#CS", "e": "@alchemy-unbalanced", "g": "alchemy" },
        { "s": "#ZZ", "e": "@alchemy-unbalanced", "g": "alchemy" }
      ],
      "close": [
        { "s": "#CP", "c": "@alchemy-in-paren", "b": 1, "a": "@alchemy-item", "g": "alchemy" },
        { "s": "#CS", "c": "@alchemy-in-bracket", "b": 1, "a": "@alchemy-item", "g": "alchemy" },
        { "s": "#IN", "b": 1, "a": "@alchemy-item", "g": "alchemy" },
        { "s": "#NL", "b": 1, "a": "@alchemy-item", "g": "alchemy" },
        { "s": "#DE", "b": 1, "a": "@alchemy-item", "g": "alchemy" },
        { "s": "#ZZ", "c": "@alchemy-in-line", "b": 1, "a": "@alchemy-item", "g": "alchemy" },
        { "r": "form", "a": "@alchemy-item", "g": "alchemy" }
      ]
    },
    # The items of \`( ... )\`, filled into the form's list in place: the
    # list pushes its first item and the rest follow it in one frame (see
    # \`form\`), so the close runs once, after the last item, and takes the
    # closer. An empty list leaves its closer for the close phase.
    "paren": {
      "open": [
        { "s": "#CP", "b": 1, "g": "alchemy" },
        { "p": "form", "g": "alchemy" }
      ],
      "close": [
        { "s": "#CP", "a": "@alchemy-close-seq", "g": "alchemy" }
      ]
    },
    # The items of \`[ ... ]\`, likewise.
    "bracket": {
      "open": [
        { "s": "#CS", "b": 1, "g": "alchemy" },
        { "p": "form", "g": "alchemy" }
      ],
      "close": [
        { "s": "#CS", "a": "@alchemy-close-seq", "g": "alchemy" }
      ]
    }
  },

  "ruleOrder": ["program", "line", "block", "form", "paren", "bracket"]
}
`
// --- END EMBEDDED alchemy-grammar.jsonic ---

// The grammar source every runtime embeds, verbatim: the text of
// `alchemy-grammar.jsonic` at the repository root, with the line feed the
// embedder writes before it. Exposed so a test can hold the embedded copy
// to the file on disk.
export function grammarSource(): string {
  return grammarText
}

let documentCache: any = undefined

// The grammar document, parsed from the embedded text once per process.
//
// The text is JSON with `#` line comments (valid jsonic, and the subset the
// file's header holds it to), so the strict JSON grammar reads it once
// comment lexing is on: no jsonic at run time. Whole numbers go back into
// integer form, as the Rust crate puts them (a JavaScript number that is
// whole already is one; the walk is kept so the two loaders read alike).
// Each call answers a fresh copy, since the engine and a caller may hold
// on to what they are given.
export function grammarDocument(): any {
  if (undefined === documentCache) {
    const reader = makeJson({ comment: { lex: true } })
    let value: any
    try {
      value = reader.parse(grammarText)
    } catch (error) {
      throw new Error(`alchemy-grammar.jsonic does not read: ${error}`)
    }
    documentCache = integralNumbers(value)
  }
  return JSON.parse(JSON.stringify(documentCache))
}

// Whole numbers back to integers: the engine reads every number as a
// double, and the grammar's integer fields want integers.
function integralNumbers(value: any): any {
  if ('number' === typeof value) {
    return Number.isInteger(value) && Math.abs(value) < 9.0e15 ? Math.trunc(value) : value
  }
  if (Array.isArray(value)) return value.map(integralNumbers)
  if (null != value && 'object' === typeof value) {
    const out: Record<string, any> = {}
    for (const key of Object.keys(value)) out[key] = integralNumbers(value[key])
    return out
  }
  return value
}

// ---------------------------------------------------------------------------
// The tagged values
// ---------------------------------------------------------------------------

type Tagged = Record<string, any>

function tagged(tag: string, fields: Record<string, any>, sp: [number, number]): Tagged {
  return { $: tag, ...fields, span: [sp[0], sp[1]] }
}

// The span a token covers.
function tokenSpan(token: Token): [number, number] {
  return [token.sI, token.sI + token.src.length]
}

// The span recorded on a tagged node, when it has one.
function spanOf(node: any): [number, number] | undefined {
  const sp = node?.span
  return Array.isArray(sp) && 'number' === typeof sp[0] && 'number' === typeof sp[1]
    ? [sp[0], sp[1]]
    : undefined
}

// The atom a token denotes, by token name.
function atom(token: Token): Tagged {
  const sp = tokenSpan(token)
  switch (token.name) {
    case KW:
      return tagged('kw', { name: token.val }, sp)
    case '#ST':
      return tagged('str', { value: token.val }, sp)
    case '#NR':
      return tagged('num', { lexeme: token.src }, sp)
    case '#VL':
      return 'boolean' === typeof token.val
        ? tagged('bool', { value: token.val }, sp)
        : tagged('null', {}, sp)
    default:
      // `#TX`, and anything a layered grammar might route here.
      return tagged('sym', { name: token.val }, sp)
  }
}

// Append `value` to the sequence in `node`: the array of a program's lines
// or of a line's items, or the `items` of a tagged list or vector. Any
// other value is left alone (the rules only ever append to those).
function append(node: any, value: any) {
  if (Array.isArray(node)) node.push(value)
  else if (null != node && 'object' === typeof node && Array.isArray(node.items)) node.items.push(value)
}

// Whether `rule` is a real rule rather than the engine's no-rule sentinel.
function isRule(rule: Rule | undefined): rule is Rule {
  return null != rule && '' !== rule.name
}

// Append `rule`'s finished node to its container's: how an item of a
// replace loop reaches the sequence it belongs to. A replace hands the
// parent on, so every item of one sequence has the container as its
// `rule.parent`.
function appendToParent(rule: Rule) {
  const parent = rule.parent
  if (!isRule(parent)) return
  // Appending a node to itself would be a cycle, never a value.
  if (parent.node === rule.node) return
  append(parent.node, rule.node)
}

// Where the lines of `rule`'s latest block begin among its items, once the
// line has taken a block.
function childrenStart(rule: Rule): number | undefined {
  const start = rule.rawu()?.[U_CHILDREN]
  return 'number' === typeof start ? start : undefined
}

function hasChildren(rule: Rule): boolean {
  return undefined !== childrenStart(rule)
}

// Whether `rule` is an item of a sequence the rule `name` reads: its
// parent, which a replace hands on to every item after the first.
function parentIs(rule: Rule, name: string): boolean {
  return isRule(rule.parent) && rule.parent.name === name
}

// Every action, condition, error hook and the lex matcher the document
// names, by reference.
export function refs(): Record<string, Function> {
  return {
    [MATCHER]: makeLayoutMatcher,
    [A_ARRAY]: (rule: Rule) => {
      rule.node = []
    },
    // A line takes one block. A second can only follow a layout line with
    // no form on it, whose first character is a lone `\r`: the earlier
    // block then stays one array item of this line, before the next
    // block's lines, which is the value the reader has always built there
    // and `Expr` refuses as malformed.
    [A_CHILDREN]: (rule: Rule) => {
      const earlier = childrenStart(rule)
      let start = 0
      const items = rule.node
      if (Array.isArray(items)) {
        if (undefined !== earlier && earlier <= items.length) {
          const lines = items.splice(earlier)
          items.push(lines)
        }
        start = items.length
      }
      rule.u[U_CHILDREN] = start
    },
    // The line's items are in its node: the inline forms, each appended by
    // its own close, then the child lines, appended by theirs through the
    // block that ran on this node.
    [A_LINE_END]: (rule: Rule) => {
      const children = hasChildren(rule)
      const items: any[] = Array.isArray(rule.node) ? rule.node.slice() : []
      if (1 === items.length && !children) {
        rule.node = items[0]
      } else {
        const start = spanOf(items[0])?.[0] ?? 0
        const end = spanOf(items[items.length - 1])?.[1] ?? start
        rule.node = tagged('list', { items }, [start, end])
      }
      appendToParent(rule)
    },
    [A_OPEN_LIST]: (rule: Rule) => {
      rule.node = tagged('list', { items: [] }, 0 < rule.os ? tokenSpan(rule.o0) : [0, 0])
    },
    [A_OPEN_VECTOR]: (rule: Rule) => {
      rule.node = tagged('vector', { items: [] }, 0 < rule.os ? tokenSpan(rule.o0) : [0, 0])
    },
    [A_ITEM]: (rule: Rule) => {
      appendToParent(rule)
    },
    // The items are already in the list; the closer ends its span.
    [A_CLOSE_SEQ]: (rule: Rule) => {
      if (0 < rule.cs && Array.isArray(rule.node?.span)) {
        rule.node.span[1] = tokenSpan(rule.c0)[1]
      }
    },
    [A_ATOM]: (rule: Rule) => {
      if (0 < rule.os) rule.node = atom(rule.o0)
    },
    [C_CHILDREN]: (rule: Rule) => hasChildren(rule),
    [C_IN_LINE]: (rule: Rule) => parentIs(rule, 'line'),
    [C_IN_PAREN]: (rule: Rule) => parentIs(rule, 'paren'),
    [C_IN_BRACKET]: (rule: Rule) => parentIs(rule, 'bracket'),
    // The offending token is the one under the cursor: the closer with
    // nothing open, or the `#ZZ` inside an open delimiter.
    [E_UNBALANCED]: (_rule: Rule, ctx: Context) => {
      const token = ctx.t0
      if (null == token) return undefined
      token.err = 'unbalanced'
      return token
    },
  }
}

// Install the alchemy grammar on `tn`: the plugin. `make` goes through it
// too, so the two construction paths cannot drift apart.
export const alchemy: Plugin = (tn: Tabnas) => {
  for (const name of [IN, DE, NL, KW]) tn.token(name)
  const spec = grammarDocument()
  spec.ref = refs()
  tn.grammar(spec)
}

// Build an alchemy parser instance.
export function make(): Tabnas {
  return new Tabnas({ plugins: [alchemy] })
}

let defaultParser: Tabnas | undefined

// Parse a program to the tagged value tree, with the engine's own error
// (a `TabnasError`). The engine is built once, on first use, and shared:
// each parse has a fresh context, and the layout state lives there.
export function parseValue(src: string): any {
  const value = (defaultParser ??= make()).parse(src)
  // The engine's answer to an empty source is the document's one
  // `emptyResult` array; hand out a fresh one.
  return Array.isArray(value) && 0 === value.length ? [] : value
}

// The engine error's code (`unexpected`, `unbalanced`, ...).
function engineCode(e: any): string {
  return 'string' === typeof e?.code ? e.code : 'unknown'
}

// The engine error's one-line description, without the code.
function engineDetail(e: any): string {
  let detail = ''
  try {
    if ('function' === typeof e?.toJSON) detail = e.toJSON()?.message ?? ''
  } catch (_err) {
    detail = ''
  }
  if ('' === detail) detail = String(e?.message ?? e)
  return detail.trimEnd()
}

// The engine's error as this package reports it: DSL_PARSE_ERROR, the
// engine's code leading the message, and the 1-based position when the
// engine has one.
export function failFrom(error: unknown): Fail {
  if (isFail(error)) return error
  if (!(error instanceof TabnasError)) throw error
  const e: any = error
  const fail = new Fail('DSL_PARSE_ERROR', `${engineCode(e)}: ${engineDetail(e)}`)
  let row: unknown = e.lineNumber
  let col: unknown = e.columnNumber
  if ('number' !== typeof row) {
    try {
      const j = e.toJSON()
      row = j?.row
      col = j?.col
    } catch (_err) {
      row = undefined
    }
  }
  if ('number' === typeof row && row > 0) {
    fail.at(row, 'number' === typeof col ? col : 0)
  }
  return fail
}

// Parse a program read from `file` (the name spans carry) to its forms.
export function parseFile(src: string, file: string): Expr[] {
  let value: unknown
  try {
    value = parseValue(src)
  } catch (error) {
    throw failFrom(error)
  }
  return programFromValue(value, sourceFile(file))
}

// Parse a program to its forms, with spans naming UNNAMED.
export function parse(src: string): Expr[] {
  return parseFile(src, UNNAMED)
}
