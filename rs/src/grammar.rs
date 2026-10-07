//! The alchemy grammar plugin: the grammar document (embedded from
//! `alchemy-grammar.jsonic` at the repository root, the one source every
//! runtime embeds), the native actions it names, and the parse entry
//! points.
//!
//! The reader (design brief section 4.1, spec section 9.1) is a grammar
//! over the token stream [`crate::lex`] produces. A program is a sequence
//! of layout lines; a line is one or more inline forms followed by either
//! a `#NL` (another line at this level), a `#DE` or `#ZZ` (this level
//! closes), or an `#IN` opening a block of child lines closed by `#DE` or
//! `#ZZ`. A line with several inline forms or with children is a list
//! whose items are the inline forms then the children; a line with exactly
//! one inline form and no children is that form. `(` forms `)` is a list,
//! `[` forms `]` a vector, and an atom is a symbol, keyword, string,
//! number, `true`, `false` or `null`.
//!
//! The rules build a tagged value tree, one object per node, that
//! [`crate::ast`] turns into [`Expr`] with spans:
//!
//! ```text
//! {"$":"list",   "items":[…], "span":[start,end]}
//! {"$":"vector", "items":[…], "span":[start,end]}
//! {"$":"sym",    "name":…,    "span":[start,end]}
//! {"$":"kw",     "name":…,    "span":[start,end]}
//! {"$":"str",    "value":…,   "span":[start,end]}
//! {"$":"num",    "lexeme":…,  "span":[start,end]}
//! {"$":"bool",   "value":…,   "span":[start,end]}
//! {"$":"null",                "span":[start,end]}
//! ```
//!
//! `start` and `end` are byte offsets into the source, from the tokens'
//! sites (`Site.si`). The tree is built by native actions rather than the
//! engine's `$`-builtins because every node carries a span the builtins
//! have no way to record.
//!
//! # How the rules repeat
//!
//! Every repetition is a replace loop, never a push per item. The
//! container pushes its first item (`p`); the item's close alternate, on a
//! token that continues the sequence, replaces the item with the next one
//! in the same frame (`r`); the container's close runs once, after the
//! last item. `program` and `block` push the first `line`, and a `line`
//! replaces itself with the next on `#NL`, or after a block on the next
//! line's first token. A `line`, a `paren` and a `bracket` push their
//! first `form`, and a `form` replaces itself with the next until the
//! token that ends its parent's sequence. The items of a sequence
//! therefore all run at one rule depth, and depth follows the program's
//! nesting, never its length. The pushes that remain are structure:
//! `line`'s `#IN` into `block`, and `form`'s openers into `paren` and
//! `bracket`.
//!
//! # How the rules share cells
//!
//! A pushed rule starts on its parent's node cell, and a replacing rule on
//! the replaced one's, so every rule here that builds a value of its own
//! first replaces its cell (`@alchemy-array`, `@alchemy-atom`, the two
//! openers). A replace hands the parent on, so every item of a sequence
//! sees its container's node as [`Rule::parent_node`], and each item
//! appends its finished value there in its own close action
//! (`@alchemy-item` for a form, `@alchemy-line-end` for a line). Nothing
//! reads a finished item back through the push link, which names only the
//! first item of a replace loop.
//!
//! The containers that build nothing of their own run on their parent's
//! cell, so their items land in the parent's value directly: `paren` and
//! `bracket` are pushed onto the tagged list their `form` has just
//! created, and `block` onto its line's array of items, after the inline
//! forms. When they pop, the parent's node already holds every item.

// The engine's error carries a code, position, hint and a formatted
// report, so it is large by design and `Result<_, TabnasError>` trips
// clippy's `result_large_err`. The engine and every grammar crate allow
// the lint at their roots for the same reason.
#![allow(clippy::result_large_err)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use indexmap::IndexMap;
use serde_json::json;
use tabnas::{GrammarError, GrammarSpec, Rule, Tabnas, TabnasError, Token, Value};

use crate::ast::Expr;
use crate::lex;
use crate::shared::{Code, Fail};

/// The file name spans carry when a program is parsed from a string
/// rather than a file.
pub const UNNAMED: &str = "<program>";

/// The action and condition references the document
/// (`alchemy-grammar.jsonic`) names, all registered by [`alchemy`] before
/// the document is installed. `refs_the_document_names_are_the_ones_registered`
/// holds the two lists together.
const A_ARRAY: &str = "@alchemy-array";
const A_CHILDREN: &str = "@alchemy-children";
const A_LINE_END: &str = "@alchemy-line-end";
const A_OPEN_LIST: &str = "@alchemy-open-list";
const A_OPEN_VECTOR: &str = "@alchemy-open-vector";
const A_ITEM: &str = "@alchemy-item";
const A_CLOSE_SEQ: &str = "@alchemy-close-seq";
const A_ATOM: &str = "@alchemy-atom";
const C_CHILDREN: &str = "@alchemy-has-children";
const C_IN_LINE: &str = "@alchemy-in-line";
const C_IN_PAREN: &str = "@alchemy-in-paren";
const C_IN_BRACKET: &str = "@alchemy-in-bracket";
const E_UNBALANCED: &str = "@alchemy-unbalanced";

/// What a `line` records once it has taken a block of children: where
/// that block's lines begin among the line's items. In `u`, the bag that
/// neither descends nor passes to the next line, because it describes
/// this line alone.
const U_CHILDREN: &str = "children";

// --- BEGIN EMBEDDED alchemy-grammar.jsonic ---
pub const GRAMMAR_TEXT: &str = r##"
# alchemy grammar definition: the reader of the alchemy language.
#
# SINGLE SOURCE OF TRUTH for the grammar document, options and rules
# together, so there is one definition of the language rather than two
# halves that can drift. Every runtime embeds this file VERBATIM between
# its `--- BEGIN/END EMBEDDED alchemy-grammar.jsonic ---` markers (Rust:
# `GRAMMAR_TEXT` in rs/src/grammar.rs) and parses it once at run time.
# Never hand-edit between the markers: edit this file and run
# `node scripts/embed.js` (`make embed`).
#
# The text is JSON plus `#` line comments: every key and every string is
# double-quoted, numbers are JSON numbers, and there are no trailing
# commas. That is valid jsonic, and it is also what the strict JSON
# grammar reads once comment lexing is on, which is how the Rust crate
# reads it without a jsonic dependency (`tabnas_json` with
# `options.comment.lex`). Keep it in that subset: an unquoted key or a
# single-quoted string here parses in jsonic and fails in Rust. Numbers
# arrive as doubles; a loader puts whole numbers back into integer form
# (`b` is an integer field).
#
# The `@alchemy-*` names are the actions, conditions, error hooks and the
# lex matcher each runtime registers on the instance BEFORE it installs
# this document, which is what looks them up.
#
# The reader (design brief section 4.1, spec section 9.1) is a grammar
# over the token stream the layout matcher (`@alchemy-layout`) produces:
# `#IN`, `#DE` and `#NL` for layout, `#KW` for a keyword, `#TX` for a
# symbol, `#ST`, `#NR` and `#VL` for strings, numbers and values, `(`
# and `)` as `#OP`/`#CP`, `[` and `]` as `#OS`/`#CS`.
#
# Every repetition is a replace loop, never a push per item (AGENTS.md,
# "The reader repeats by replacement"): the container pushes its first
# item (`p`), the item's close replaces it with the next one in the same
# frame (`r`), and the container's close runs once, after the last item.

{
  "options": {
    "rule": { "start": "program" },
    # The engine answers a literally empty source before any rule runs;
    # an empty program is `[]`, as it is from the `program` rule for a
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
    # structural `\xHH` and `\u{...}` off, and the non-standard `\v`, `\'`
    # and `` \` `` removed (a null entry deletes an escape; `""` would map
    # it to the empty string).
    "string": {
      "chars": "\"",
      "multiChars": "",
      "allowUnknown": false,
      "escapeStrict": true,
      "escape": { "v": null, "'": null, "`": null }
    },
    # `;` to the end of the line, and nothing else.
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
    # pins `ERROR:<code>` for either kind. `too_deep` is both: the
    # reader's bound on nesting (256, MAX_NESTING), and the desugarer's on
    # what its rewrites add. A form of the wrong shape is the desugarer's
    # DSL_PARSE_ERROR, with its text, wherever it is found: the resolver
    # holds a `let`, `if` or `match` that a `pipe` builds to the same
    # shapes (a step's form grows by the threaded value after the
    # desugarer read it), and the later stages' own checks of the shapes
    # fail the same way. The resolver's, the checker's and the runtime's
    # codes follow, declared here too and raised the same way: the code
    # leads the message of a DSL_TYPE_ERROR, a STREAM_REUSED or a
    # STREAMABILITY_UNKNOWN. A finer code comes with the same code wherever
    # it is raised. A message is the text its raising sites write, each
    # `{name}` standing for what a site fills in; a code raised in more
    # than one sentence holds one per line, and a situation two stages
    # report is one sentence. The texts are those a program compiled on its
    # own can meet (AGENTS.md, "Error codes", names the few raised only
    # otherwise). Each runtime holds its own raising sites' texts to these
    # (rs: `the_desugaring_messages_match_the_document`,
    # `the_too_deep_texts_name_the_bound`, and
    # `the_raised_messages_match_the_document`, which reads every failure
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
      "bad_pattern": "{name} is not a constructor; a list pattern is (constructor pattern...)\na list pattern is (constructor pattern...)",
      # The resolver's and the evaluator's.
      "recursion": "{name} reaches itself through {names}; strict mode refuses recursion\nevaluation nested past 1000 levels: a function applied to itself, or definitions or calls chained that deep; strict mode refuses recursion without a bound",
      # The checker's, and the runtime's where a value read at run time meets the same refusal.
      # A value both stages describe is named by its kind (`{kind}`: `a stream`, `a text`,
      # `a number`), the runtime's words, onto which the checker's types map; a requirement
      # (`{expected}`, `{actual}`) is in each stage's words, a type at the checker.
      "arity": "{callee} takes {count} argument(s), got {given}\nthe pattern ({constructor} ...) takes {count} field(s), got {given}\n{what} takes a function of {count} argument(s), not {given}\npartial supplies {given} argument(s) to a function of {count}\nexport takes one parameter, the input, not {given}",
      "type_mismatch": "{what} must be {expected}, not {actual}\n{what} must answer {expected}, not {actual}\n{what} was expected, not {actual}\n{native}: expected a string or a text, not {actual}\n{what} must name one location, not {selector}\n{what} cannot hold {kind}; a stream is used once, where it is\n{what} cannot hold {kind}; a stream or a text is used once, where it is\nthe state of scan-emit cannot be {kind}\n{callee} is {kind} and cannot be called\nan empty list is not a call\nexport must be a fn [input]\npop: the vector is empty; there is no last item to remove\ntop: the vector is empty; there is no last item\n{native}: the options have no :{key}\nkind: {kind} cannot be asked; a stream or a text is used where it is, not inspected",
      "protocol_mismatch": "{what} must be {expected}, not {actual}\n{what} must be a vector or a stream of items, not JsonEvents; select or route what the stream should yield, or read its events\ncsv renders table events; the program's result is JSON events (render it as json, or make a table of it with table-from-json)",
      "no_export": "the program has no `def export [input]`",
      "bad_output": "export answers a Stream<{item}>; render it as a text (join, concat-map), or make table events of it\nexport answers a {type}; it must answer a text, table events or JSON events",
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
      "bad_indent": "A child line is exactly two spaces deeper than its parent, and the\nfirst line of a program is not indented.",
      "bad_dedent": "A line may only return to the indentation of a line above it.",
      "unbalanced": "Every ( and [ needs its ) or ], and nothing closes what was not\nopened. Inside delimiters, line breaks and indentation do not count.",
      "too_deep": "A program nests at most 256 levels, counting a layout line, each\nindentation level and each open ( or [ as one; a pipe adds a level per\nstep. Split the form into definitions.",
      "empty_step": "In pipe VALUE STEP..., each step is a symbol (called with the value)\nor a list (the value is appended as its last argument).",
      "bad_def": "Write def NAME VALUE, or def NAME [PARAMS] BODY.",
      "bad_let": "Write let [NAME VALUE] BODY.",
      "bad_if": "Write if CONDITION THEN ELSE.",
      "bad_match": "Write match VALUE (case PATTERN BODY)...",
      "unknown_name": "A name is a definition of the program, a parameter or a binding in\nscope, or one of the library's. Define it, or correct its spelling.",
      "not_def": "Each top-level form is a definition: def NAME VALUE, or def NAME\n[PARAMS] BODY. Put the expression inside one.",
      "duplicate_def": "A program defines each name once. Rename or remove one of the\ndefinitions.",
      "reserved": "def, fn, let, if, match, case and pipe are special forms. Choose\nanother name.",
      "bad_fn": "Write fn [PARAMS] BODY: a vector of parameter names, then one body.",
      "misplaced_def": "def defines a name at the top level only. Inside a body, bind a value\nwith let [NAME VALUE] BODY.",
      "bad_pattern": "A list pattern is (CONSTRUCTOR PATTERN...), its head a constructor\nsuch as transition, key or scalar; any other pattern is a value, a\nname or _.",
      "recursion": "Strict mode refuses recursion: a definition may not reach itself, and\nevaluation nests at most 1000 levels. Repeat over data with map,\nfilter, concat-map or scan-emit instead.",
      "arity": "Give a function, a native, a constructor pattern or a partial exactly\nas many arguments as it takes, and a stream operator a function of as\nmany parameters as it passes; export takes one, the input.",
      "type_mismatch": "A value of one type is given where another is wanted. The message\nnames the place, what it wants and what came.",
      "protocol_mismatch": "A stream's protocol (JSON events, table events, a stream of items, a\ntext) must be the one its taker reads: select or route makes items of\nJSON events, table-from-json makes table events of them, and json or\ncsv renders them.",
      "no_export": "A program answers through def export [input]: one function of the\ninput, whose result is the output.",
      "bad_output": "export answers a text, table events or JSON events. Render a stream\nof items as a text (join, concat-map), or make table events of it.",
      "unknown_output": "The type of export's result must be known when the program is\nchecked: a text, table events or JSON events, not a value read at run\ntime.",
      "reused": "A stream is read once, so it is used in one place only. Take\neverything needed from it in that one pass (route captures several\nparts at once).",
      "captured": "A fn may run more than once, and a stream is read once, so a fn cannot\nuse a stream from the scope around it. Pass what it needs as an\nargument instead.",
      "dynamic": "Strict mode needs to know, when the program is checked, the function\na stream operator runs. Pass a fn, a definition, a native or a\npartial of one, not a function read from data.",
      "duplicate_key": "A record holds each key once. Remove or rename the repeated entry.",
      "no_match": "Add a case for the value, or end with (case _ BODY), which takes any\nvalue.",
      "render_of_text": "A program that renders its own text takes no renderer. Leave --render\nout, or have export answer table events or JSON events."
    }
  },

  "rule": {
    # A program: layout lines until the source ends. The node is the array
    # of finished lines. The program pushes the first line and the lines
    # follow it in one frame (see `line`), so the close runs once, on the
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
    # token or the end of the source (see `form`). A block's `#IN` is
    # taken here and the block pushed. `#NL` is the next line of this
    # level, and so is whatever follows a block that did not also close
    # this level: either way the line appends itself to the level's node
    # and replaces itself with the next line (`r`). `#DE` and `#ZZ` end the
    # level and are left (`b: 1`) for the block or program above. A layout
    # token is always followed by a form (the layout matcher reads a
    # line's indentation up to its first form, and a row with nothing else
    # on it is blank), so `#NL` never stands right before `#DE` or `#ZZ`.
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
    # The children of a line, one per level: lines until the `#DE` that
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
    # here and is `unbalanced`.
    #
    # A form is an item of the sequence its parent reads: a line's inline
    # forms, or the items of `( )` or `[ ]`. Its close appends it to the
    # parent's node and then either ends the sequence, leaving the token
    # that ends it (`b: 1`) for the parent, or goes on to the next item in
    # this frame (`r`). What ends a sequence is the parent's: the closer
    # for `paren` and `bracket`, a layout token or the end of the source
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
    # The items of `( ... )`, filled into the form's list in place: the
    # list pushes its first item and the rest follow it in one frame (see
    # `form`), so the close runs once, after the last item, and takes the
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
    # The items of `[ ... ]`, likewise.
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
"##;
// --- END EMBEDDED alchemy-grammar.jsonic ---

/// The grammar source every runtime embeds, verbatim: the text of
/// `alchemy-grammar.jsonic` at the repository root, options and rules
/// together, so there is one definition of the language rather than two
/// halves that can drift. Exposed so a test can hold the embedded copy to
/// the file on disk.
pub fn grammar_text() -> &'static str {
    GRAMMAR_TEXT
}

/// The grammar document, parsed from [`GRAMMAR_TEXT`] once per process.
///
/// The text is JSON with `#` line comments (valid jsonic, and the subset
/// the file's header holds it to), so the strict JSON grammar reads it
/// once comment lexing is on: no jsonic at run time. The engine's
/// numbers are doubles, and the alternate decoder reads `b` as an
/// integer, so whole numbers go back into integer form.
fn document() -> serde_json::Value {
    static DOCUMENT: OnceLock<serde_json::Value> = OnceLock::new();
    DOCUMENT
        .get_or_init(|| {
            let mut reader = Tabnas::new();
            tabnas_json::json(&mut reader).expect("the JSON grammar installs");
            reader
                .set_options(|options: &mut tabnas::Options| options.comment.lex = true)
                .expect("comment lexing applies");
            let value = reader
                .parse(GRAMMAR_TEXT)
                .unwrap_or_else(|error| panic!("alchemy-grammar.jsonic does not read: {error}"));
            integral_numbers(value.to_json())
        })
        .clone()
}

/// Whole numbers back to integers: the engine reads every number as a
/// double, and the grammar's integer fields want integers.
fn integral_numbers(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Number(number) => match number.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => json!(f as i64),
            _ => serde_json::Value::Number(number),
        },
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(integral_numbers).collect())
        }
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key, integral_numbers(value)))
                .collect(),
        ),
        other => other,
    }
}

// ---------------------------------------------------------------------------
// The tagged values
// ---------------------------------------------------------------------------

fn fresh(rule: &mut Rule, value: Value) {
    rule.node = Rc::new(RefCell::new(value));
}

fn tagged(tag: &str, fields: Vec<(&str, Value)>, span: (usize, usize)) -> Value {
    let mut node = IndexMap::new();
    node.insert("$".to_string(), Value::String(tag.to_string()));
    for (name, value) in fields {
        node.insert(name.to_string(), value);
    }
    node.insert(
        "span".to_string(),
        Value::array(vec![
            Value::Number(span.0 as f64),
            Value::Number(span.1 as f64),
        ]),
    );
    Value::object(node)
}

/// The byte span a token covers.
fn token_span(token: &Token) -> (usize, usize) {
    let start = token.site.si;
    (start, start + token.src.len())
}

/// The span recorded on a tagged node, when it has one.
fn span_of(node: &Value) -> Option<(usize, usize)> {
    let Value::Object(fields) = node else {
        return None;
    };
    let Some(Value::Array(span)) = fields.get("span") else {
        return None;
    };
    match (span.first(), span.get(1)) {
        (Some(Value::Number(start)), Some(Value::Number(end))) => {
            Some((*start as usize, *end as usize))
        }
        _ => None,
    }
}

/// The atom a token denotes, by token name.
fn atom(token: &Token) -> Value {
    let span = token_span(token);
    match token.name.as_str() {
        lex::KW => tagged("kw", vec![("name", token.val.clone())], span),
        "#ST" => tagged("str", vec![("value", token.val.clone())], span),
        "#NR" => tagged(
            "num",
            vec![("lexeme", Value::String(token.src.to_string()))],
            span,
        ),
        "#VL" => match &token.val {
            Value::Bool(b) => tagged("bool", vec![("value", Value::Bool(*b))], span),
            _ => tagged("null", Vec::new(), span),
        },
        // `#TX`, and anything a layered grammar might route here.
        _ => tagged("sym", vec![("name", token.val.clone())], span),
    }
}

/// Append `value` to the sequence in `cell`: the array of a program's
/// lines or of a line's items, or the `items` of a tagged list or vector.
/// Any other value is left alone (the rules only ever append to those).
fn append(cell: &RefCell<Value>, value: Value) {
    match &mut *cell.borrow_mut() {
        Value::Array(items) => Arc::make_mut(items).push(value),
        Value::Object(fields) => {
            if let Some(Value::Array(items)) = Arc::make_mut(fields).get_mut("items") {
                Arc::make_mut(items).push(value);
            }
        }
        _ => {}
    }
}

/// Append `rule`'s finished node to its container's: how an item of a
/// replace loop reaches the sequence it belongs to. A replace hands the
/// parent on, so every item of one sequence has the container's node as
/// its [`Rule::parent_node`].
fn append_to_parent(rule: &Rule) {
    let Some(parent) = rule.parent_node.as_ref() else {
        return;
    };
    // Every item has a cell of its own by its close; appending a cell to
    // itself would be a cycle, never a value.
    if Rc::ptr_eq(parent, &rule.node) {
        return;
    }
    let value = rule.node.borrow().clone();
    append(parent, value);
}

/// Where the lines of `rule`'s latest block begin among its items, once
/// the line has taken a block.
fn children_start(rule: &Rule) -> Option<usize> {
    match rule.u.get(U_CHILDREN) {
        Some(Value::Number(start)) => Some(*start as usize),
        _ => None,
    }
}

fn has_children(rule: &Rule) -> bool {
    children_start(rule).is_some()
}

/// Whether `rule` is an item of a sequence the rule `name` reads: its
/// parent, which a replace hands on to every item after the first.
fn parent_is(rule: &Rule, name: &str) -> bool {
    rule.parent_rule
        .as_ref()
        .is_some_and(|parent| parent.name.as_str() == name)
}

/// Register every action, condition and error hook the document names.
fn register_refs(parser: &mut Tabnas) {
    parser.action(A_ARRAY, |rule| fresh(rule, Value::array(Vec::new())));
    // A line takes one block. A second can only follow a layout line with
    // no form on it, whose first character is a lone `\r` (a line to the
    // layout, a space to the engine): the earlier block then stays one
    // array item of this line, before the next block's lines, which is
    // the value the reader has always built there and `Expr` refuses as
    // malformed.
    parser.action(A_CHILDREN, |rule| {
        let earlier = children_start(rule);
        let start = match &mut *rule.node.borrow_mut() {
            Value::Array(items) => {
                if let Some(earlier) = earlier.filter(|earlier| *earlier <= items.len()) {
                    let lines = Arc::make_mut(items).split_off(earlier);
                    Arc::make_mut(items).push(Value::array(lines));
                }
                items.len()
            }
            _ => 0,
        };
        rule.u_mut()
            .insert(U_CHILDREN.to_string(), Value::Number(start as f64));
    });
    // The line's items are in its node: the inline forms, each appended by
    // its own close, then the child lines, appended by theirs through the
    // block that ran on this node.
    parser.action(A_LINE_END, |rule| {
        let children = has_children(rule);
        let mut items: Vec<Value> = match &*rule.node.borrow() {
            Value::Array(items) => items.as_ref().clone(),
            _ => Vec::new(),
        };
        if items.len() == 1 && !children {
            let only = items.pop().expect("one item was just counted");
            fresh(rule, only);
        } else {
            let start = items.first().and_then(span_of).map_or(0, |span| span.0);
            let end = items.last().and_then(span_of).map_or(start, |span| span.1);
            fresh(
                rule,
                tagged("list", vec![("items", Value::array(items))], (start, end)),
            );
        }
        append_to_parent(rule);
    });
    parser.action(A_OPEN_LIST, |rule| {
        let span = rule.o0().map_or((0, 0), token_span);
        fresh(
            rule,
            tagged("list", vec![("items", Value::array(Vec::new()))], span),
        );
    });
    parser.action(A_OPEN_VECTOR, |rule| {
        let span = rule.o0().map_or((0, 0), token_span);
        fresh(
            rule,
            tagged("vector", vec![("items", Value::array(Vec::new()))], span),
        );
    });
    parser.action(A_ITEM, |rule| append_to_parent(rule));
    // The items are already in the list; the closer ends its span.
    parser.action(A_CLOSE_SEQ, |rule| {
        let end = rule.c0().map(|closer| token_span(closer).1);
        if let (Some(end), Value::Object(fields)) = (end, &mut *rule.node.borrow_mut()) {
            if let Some(Value::Array(span)) = Arc::make_mut(fields).get_mut("span") {
                if let Some(slot) = Arc::make_mut(span).get_mut(1) {
                    *slot = Value::Number(end as f64);
                }
            }
        }
    });
    parser.action(A_ATOM, |rule| {
        if let Some(value) = rule.o0().map(atom) {
            fresh(rule, value);
        }
    });
    parser.alt_condition(C_CHILDREN, |rule, _context| has_children(rule));
    parser.alt_condition(C_IN_LINE, |rule, _context| parent_is(rule, "line"));
    parser.alt_condition(C_IN_PAREN, |rule, _context| parent_is(rule, "paren"));
    parser.alt_condition(C_IN_BRACKET, |rule, _context| parent_is(rule, "bracket"));
    // The offending token is the one under the cursor: the closer with
    // nothing open, or the `#ZZ` inside an open delimiter.
    parser.alt_error(E_UNBALANCED, |_rule, context| {
        context.t.first().cloned().map(|mut token| {
            token.bad("unbalanced");
            token
        })
    });
}

/// Install the alchemy grammar on `parser`.
///
/// This is the one entry point: [`make`] goes through it too, so the two
/// construction paths cannot drift apart.
///
/// ```
/// let mut parser = tabnas::Tabnas::new();
/// tabnas_alchemy::alchemy(&mut parser)?;
/// let value = parser.parse("def x 1")?;
/// assert_eq!(value.to_json()[0]["$"], "list");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn alchemy(parser: &mut Tabnas) -> Result<(), GrammarError> {
    lex::register(parser);
    register_refs(parser);
    let spec = GrammarSpec::from_value(document())?;
    parser.grammar(&spec)?;
    Ok(())
}

/// Build an alchemy parser instance.
///
/// Infallible by design: the document is a fixed literal, so a failure
/// here is a bug in this crate rather than anything a caller did.
///
/// ```
/// let parser = tabnas_alchemy::make();
/// assert!(parser.parse("(a b").is_err());
/// ```
pub fn make() -> Tabnas {
    let mut parser = Tabnas::new();
    alchemy(&mut parser).expect("the alchemy grammar document is fixed and valid");
    parser
}

/// Parse a program to the tagged value tree, with the engine's own error.
///
/// The engine is built once, on first use, and shared: [`Tabnas::parse`]
/// takes `&self` and builds a fresh context per call, and the layout
/// state lives in that context.
///
/// ```
/// let value = tabnas_alchemy::parse_value("a\n  b c")?;
/// assert_eq!(
///     value.to_json().to_string(),
///     r#"[{"$":"list","items":[{"$":"sym","name":"a","span":[0.0,1.0]},{"$":"list","items":[{"$":"sym","name":"b","span":[4.0,5.0]},{"$":"sym","name":"c","span":[6.0,7.0]}],"span":[4.0,7.0]}],"span":[0.0,7.0]}]"#
/// );
/// # Ok::<(), tabnas::TabnasError>(())
/// ```
pub fn parse_value(src: &str) -> Result<Value, TabnasError> {
    static DEFAULT: OnceLock<Tabnas> = OnceLock::new();
    DEFAULT.get_or_init(make).parse(src)
}

/// The engine's error as this crate reports it: `DSL_PARSE_ERROR`, the
/// engine's code leading the message, and the 1-based position when the
/// engine has one.
fn fail_from(error: &TabnasError) -> Fail {
    let mut fail = Fail::new(
        Code::DslParseError,
        format!("{}: {}", error.code, error.detail.trim_end()),
    );
    if error.row > 0 {
        fail = fail.at(error.row as u64, error.col as u64);
    }
    fail
}

/// Parse a program read from `file` (the name spans carry) to its forms.
pub fn parse_file(src: &str, file: &str) -> Result<Vec<Expr>, Fail> {
    let value = parse_value(src).map_err(|error| fail_from(&error))?;
    Expr::program_from_value(&value, &Arc::from(file))
}

/// Parse a program to its forms, with spans naming [`UNNAMED`].
///
/// ```
/// use tabnas_alchemy::{canonical, parse};
/// let program = parse("def export [input]\n  csv input")?;
/// assert_eq!(canonical(&program), "(def export [input] (csv input))");
/// # Ok::<(), tabnas_alchemy::shared::Fail>(())
/// ```
pub fn parse(src: &str) -> Result<Vec<Expr>, Fail> {
    parse_file(src, UNNAMED)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(src: &str) -> String {
        let value = parse_value(src).unwrap_or_else(|error| panic!("{src:?}: {error}"));
        // Whole numbers back to integers, so the expected text reads.
        let text = value.to_json();
        fn integral(value: serde_json::Value) -> serde_json::Value {
            match value {
                serde_json::Value::Number(n) => match n.as_f64() {
                    Some(f) if f.fract() == 0.0 => serde_json::json!(f as i64),
                    _ => serde_json::Value::Number(n),
                },
                serde_json::Value::Array(items) => {
                    serde_json::Value::Array(items.into_iter().map(integral).collect())
                }
                serde_json::Value::Object(fields) => serde_json::Value::Object(
                    fields
                        .into_iter()
                        .map(|(key, value)| (key, integral(value)))
                        .collect(),
                ),
                other => other,
            }
        }
        integral(text).to_string()
    }

    fn code(src: &str) -> String {
        match parse_value(src) {
            Ok(value) => panic!("{src:?} parsed: {}", value.to_json()),
            Err(error) => error.code,
        }
    }

    #[test]
    fn an_empty_program_is_an_empty_array() {
        assert_eq!(json(""), "[]");
        assert_eq!(json("\n\n; only a comment\n  \n"), "[]");
    }

    #[test]
    fn a_line_with_one_form_is_that_form() {
        assert_eq!(json("x"), r#"[{"$":"sym","name":"x","span":[0,1]}]"#);
        assert_eq!(
            json("\"a\\nb\""),
            r#"[{"$":"str","value":"a\nb","span":[0,6]}]"#
        );
        assert_eq!(
            json("-1.5e3"),
            r#"[{"$":"num","lexeme":"-1.5e3","span":[0,6]}]"#
        );
        assert_eq!(json("true"), r#"[{"$":"bool","value":true,"span":[0,4]}]"#);
        assert_eq!(json("null"), r#"[{"$":"null","span":[0,4]}]"#);
        assert_eq!(json(":k"), r#"[{"$":"kw","name":"k","span":[0,2]}]"#);
    }

    #[test]
    fn several_inline_forms_make_a_list() {
        assert_eq!(
            json("a b"),
            r#"[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[2,3]}],"span":[0,3]}]"#
        );
    }

    #[test]
    fn explicit_delimiters_carry_their_own_spans() {
        assert_eq!(
            json("( a )"),
            r#"[{"$":"list","items":[{"$":"sym","name":"a","span":[2,3]}],"span":[0,5]}]"#
        );
        assert_eq!(json("()"), r#"[{"$":"list","items":[],"span":[0,2]}]"#);
        assert_eq!(json("[]"), r#"[{"$":"vector","items":[],"span":[0,2]}]"#);
        assert_eq!(
            json("[1 [2]]"),
            r#"[{"$":"vector","items":[{"$":"num","lexeme":"1","span":[1,2]},{"$":"vector","items":[{"$":"num","lexeme":"2","span":[4,5]}],"span":[3,6]}],"span":[0,7]}]"#
        );
    }

    #[test]
    fn children_follow_the_inline_forms() {
        assert_eq!(
            json("a\n  b\n  c d\ne"),
            r#"[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[4,5]},{"$":"list","items":[{"$":"sym","name":"c","span":[8,9]},{"$":"sym","name":"d","span":[10,11]}],"span":[8,11]}],"span":[0,11]},{"$":"sym","name":"e","span":[12,13]}]"#
        );
    }

    #[test]
    fn a_dedent_of_several_levels_closes_each_block() {
        assert_eq!(
            json("a\n  b\n    c\nd"),
            r#"[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"list","items":[{"$":"sym","name":"b","span":[4,5]},{"$":"sym","name":"c","span":[10,11]}],"span":[4,11]}],"span":[0,11]},{"$":"sym","name":"d","span":[12,13]}]"#
        );
    }

    /// A lone `\r` at the start of a line is whitespace there too (layout
    /// rule 7): it restarts the indentation as it restarts the engine's
    /// column, a row holding nothing else after it is blank, and a block
    /// indented under such a row belongs to the line before it. No input
    /// reaches the reader as a layout line with no form on it.
    #[test]
    fn a_lone_carriage_return_at_a_line_start_is_whitespace() {
        assert_eq!(
            json("f x\n\r"),
            r#"[{"$":"list","items":[{"$":"sym","name":"f","span":[0,1]},{"$":"sym","name":"x","span":[2,3]}],"span":[0,3]}]"#
        );
        assert_eq!(
            json("a\n  b\n  \r;c\nd"),
            r#"[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[4,5]}],"span":[0,5]},{"$":"sym","name":"d","span":[12,13]}]"#
        );
        assert_eq!(
            json("a\n  b\n\r;x\n  c"),
            r#"[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[4,5]},{"$":"sym","name":"c","span":[12,13]}],"span":[0,13]}]"#
        );
        assert_eq!(
            json("a\n\r  b"),
            r#"[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[5,6]}],"span":[0,6]}]"#
        );
        assert_eq!(
            json("a\n  \r  \rb"),
            r#"[{"$":"sym","name":"a","span":[0,1]},{"$":"sym","name":"b","span":[8,9]}]"#
        );
        // A comment ends at a lone `\r` too: what follows one on the same
        // row is a form of the line the row opens, at the indentation
        // after the `\r`; a lone `\r` between forms is whitespace.
        assert_eq!(
            json("a\n;x\r  c\r;y\r  d"),
            r#"[{"$":"list","items":[{"$":"sym","name":"a","span":[0,1]},{"$":"list","items":[{"$":"sym","name":"c","span":[7,8]},{"$":"sym","name":"d","span":[14,15]}],"span":[7,15]}],"span":[0,15]}]"#
        );
        // The column a diagnostic names counts from the carriage return,
        // as the engine counts it: the indentation plus one.
        let fail = parse("\r  a").expect_err("an indented first line");
        assert_eq!(fail.message.split(": ").next(), Some("bad_indent"));
        assert_eq!((fail.row, fail.column), (Some(1), Some(3)));
        let fail = parse("a\n  b\n  \r   c").expect_err("a bad indent");
        assert_eq!(fail.message.split(": ").next(), Some("bad_indent"));
        assert_eq!((fail.row, fail.column), (Some(3), Some(4)));
        let fail = parse("a\n\r \tb").expect_err("a tab in the indentation");
        assert_eq!(fail.message.split(": ").next(), Some("tab_indent"));
        assert_eq!((fail.row, fail.column), (Some(2), Some(2)));
        // Several lone carriage returns on the row: the column counts from
        // the last, where the engine's restarts.
        let fail = parse("\r \r   a").expect_err("an indented first line");
        assert_eq!(fail.message.split(": ").next(), Some("bad_indent"));
        assert_eq!((fail.row, fail.column), (Some(1), Some(4)));
        let fail = parse("a\n  \r \r   b").expect_err("a bad indent");
        assert_eq!(fail.message.split(": ").next(), Some("bad_indent"));
        assert_eq!((fail.row, fail.column), (Some(2), Some(4)));
    }

    #[test]
    fn the_errors_carry_the_grammar_codes() {
        assert_eq!(code("a\n\tb"), "tab_indent");
        assert_eq!(code("  a"), "bad_indent");
        assert_eq!(code("a\n   b"), "bad_indent");
        assert_eq!(code("a\n  b\n c"), "bad_dedent");
        assert_eq!(code("a )"), "unbalanced");
        assert_eq!(code("(a b"), "unbalanced");
        assert_eq!(code("[a b"), "unbalanced");
        assert_eq!(code("(a]"), "unbalanced");
        assert_eq!(code("\"abc"), "unterminated_string");
        assert_eq!(code("a { b"), "unexpected");
        assert_eq!(code("\"\\q\""), "unexpected");
        assert_eq!(code(&"(".repeat(crate::ast::MAX_NESTING)), "too_deep");
    }

    /// The bound is named in the message and the hint the document
    /// declares, so a change to one without the other fails here.
    #[test]
    fn the_too_deep_texts_name_the_bound() {
        let document = document();
        let bound = crate::ast::MAX_NESTING.to_string();
        assert!(crate::ast::TOO_DEEP.contains(&bound));
        assert!(document["options"]["hint"]["too_deep"]
            .as_str()
            .is_some_and(|hint| hint.contains(&bound)));
    }

    /// The desugaring codes are declared here so a fixture can pin them
    /// like the reader's, and raised in `desugar` with its own text; the
    /// two are one message.
    #[test]
    fn the_desugaring_messages_match_the_document() {
        let document = document();
        for (code, message) in crate::desugar::MESSAGES {
            assert_eq!(
                document["options"]["error"][code].as_str(),
                Some(message),
                "options.error.{code}"
            );
            assert!(
                document["options"]["hint"][code].is_string(),
                "options.hint.{code} is declared"
            );
        }
    }

    #[test]
    fn the_fail_carries_the_code_and_the_position() {
        let fail = parse("a\n  b\n c").expect_err("a bad dedent");
        assert_eq!(fail.code, Code::DslParseError);
        assert!(fail.message.starts_with("bad_dedent: "), "{}", fail.message);
        assert_eq!((fail.row, fail.column), (Some(3), Some(2)));
    }

    /// Every `@` reference the document names is one this crate
    /// registers, and every one it registers is named: the grammar text
    /// and the code that installs it cannot drift apart.
    #[test]
    fn refs_the_document_names_are_the_ones_registered() {
        fn refs(value: &serde_json::Value, found: &mut std::collections::BTreeSet<String>) {
            match value {
                serde_json::Value::String(text) if text.starts_with('@') => {
                    found.insert(text.clone());
                }
                serde_json::Value::Array(items) => items.iter().for_each(|item| refs(item, found)),
                serde_json::Value::Object(fields) => {
                    fields.values().for_each(|field| refs(field, found))
                }
                _ => {}
            }
        }
        let mut named = std::collections::BTreeSet::new();
        refs(&document(), &mut named);
        let registered: std::collections::BTreeSet<String> = [
            A_ARRAY,
            A_CHILDREN,
            A_LINE_END,
            A_OPEN_LIST,
            A_OPEN_VECTOR,
            A_ITEM,
            A_CLOSE_SEQ,
            A_ATOM,
            C_CHILDREN,
            C_IN_LINE,
            C_IN_PAREN,
            C_IN_BRACKET,
            E_UNBALANCED,
            lex::MATCHER,
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        assert_eq!(named, registered);
    }

    /// The bound's message in the document is the one the desugarer and
    /// the reader's nesting check raise.
    #[test]
    fn the_too_deep_message_is_the_documents() {
        assert_eq!(
            document()["options"]["error"]["too_deep"].as_str(),
            Some(crate::ast::TOO_DEEP)
        );
    }
}
