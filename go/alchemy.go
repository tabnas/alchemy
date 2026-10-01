// Copyright (c) 2026 tabnas, MIT License

// Package tabnasalchemy is the alchemy language for the tabnas parsing
// engine: a small, typed, functional streaming language in which
// transducers and renderers are written, read by a tabnas grammar plugin
// like every other grammar in the fleet. This is the Go port of the Rust
// crate in rs/, the reference implementation.
//
// The reader (this file and lex.go) is a grammar over layout
// S-expressions: two-space indentation makes lists, a line with several
// forms or with children is a list, a line with exactly one form and no
// children is that form, parens are explicit lists, brackets are vectors,
// `;` starts a comment, strings take JSON escapes, `:name` is a keyword.
// The grammar builds a tagged value tree that ExprFromValue turns into
// Expr with source spans (ast.go). Desugar rewrites `def name [params]
// body`, `pipe` and the other conveniences into the core forms
// (desugar.go); Resolve links the definitions and refuses recursion
// (resolve.go); CheckProgram infers types, enforces affine streams and
// protocol composition (types.go, check.go); the effects summary
// (effects.go) prints the `explain` report from the stages of a plan.
//
// Front (frontend.go) runs the whole front end over one or several
// sources. Building the plan (the interpreter), lowering it to transduce
// and render sinks, the Program API and the `alchemy` command are the
// second half of the port.
package tabnasalchemy

import (
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"sync"

	tabnasjson "github.com/tabnas/json/go"
	tabnas "github.com/tabnas/parser/go"
)

// VERSION is this module's version. It MUST equal the version rs/Cargo.toml
// declares; TestVersionMatchesTheCrate fails the build if they drift.
const VERSION = "0.1.1"

// Unnamed is the file name spans carry when a program is parsed from a
// string rather than a file.
const Unnamed = "<program>"

// The action, condition and error references the document names, all
// registered by Alchemy before the document is installed.
// TestRefsTheDocumentNamesAreTheOnesRegistered holds the two lists
// together.
const (
	refArray       = "@alchemy-array"
	refChildren    = "@alchemy-children"
	refLineEnd     = "@alchemy-line-end"
	refOpenList    = "@alchemy-open-list"
	refOpenVector  = "@alchemy-open-vector"
	refItem        = "@alchemy-item"
	refCloseSeq    = "@alchemy-close-seq"
	refAtom        = "@alchemy-atom"
	refHasChildren = "@alchemy-has-children"
	refInLine      = "@alchemy-in-line"
	refInParen     = "@alchemy-in-paren"
	refInBracket   = "@alchemy-in-bracket"
	refUnbalanced  = "@alchemy-unbalanced"
)

// registeredRefs is every reference Alchemy registers, the layout matcher
// included.
var registeredRefs = []string{
	refArray, refChildren, refLineEnd, refOpenList, refOpenVector, refItem,
	refCloseSeq, refAtom, refHasChildren, refInLine, refInParen, refInBracket,
	refUnbalanced, layoutMatcherRef,
}

// uChildren is what a line records, in its own U bag, once it has taken a
// block of children: where that block's lines begin among its items.
const uChildren = "children"

// --- BEGIN EMBEDDED alchemy-grammar.jsonic ---
const grammarText = `
# alchemy grammar definition: the reader of the alchemy language.
#
# SINGLE SOURCE OF TRUTH for the grammar document, options and rules
# together, so there is one definition of the language rather than two
# halves that can drift. Every runtime embeds this file VERBATIM between
# its ` + "`" + `--- BEGIN/END EMBEDDED alchemy-grammar.jsonic ---` + "`" + ` markers (Rust:
# ` + "`" + `GRAMMAR_TEXT` + "`" + ` in rs/src/grammar.rs) and parses it once at run time.
# Never hand-edit between the markers: edit this file and run
# ` + "`" + `node scripts/embed.js` + "`" + ` (` + "`" + `make embed` + "`" + `).
#
# The text is JSON plus ` + "`" + `#` + "`" + ` line comments: every key and every string is
# double-quoted, numbers are JSON numbers, and there are no trailing
# commas. That is valid jsonic, and it is also what the strict JSON
# grammar reads once comment lexing is on, which is how the Rust crate
# reads it without a jsonic dependency (` + "`" + `tabnas_json` + "`" + ` with
# ` + "`" + `options.comment.lex` + "`" + `). Keep it in that subset: an unquoted key or a
# single-quoted string here parses in jsonic and fails in Rust. Numbers
# arrive as doubles; a loader puts whole numbers back into integer form
# (` + "`" + `b` + "`" + ` is an integer field).
#
# The ` + "`" + `@alchemy-*` + "`" + ` names are the actions, conditions, error hooks and the
# lex matcher each runtime registers on the instance BEFORE it installs
# this document, which is what looks them up.
#
# The reader (design brief section 4.1, spec section 9.1) is a grammar
# over the token stream the layout matcher (` + "`" + `@alchemy-layout` + "`" + `) produces:
# ` + "`" + `#IN` + "`" + `, ` + "`" + `#DE` + "`" + ` and ` + "`" + `#NL` + "`" + ` for layout, ` + "`" + `#KW` + "`" + ` for a keyword, ` + "`" + `#TX` + "`" + ` for a
# symbol, ` + "`" + `#ST` + "`" + `, ` + "`" + `#NR` + "`" + ` and ` + "`" + `#VL` + "`" + ` for strings, numbers and values, ` + "`" + `(` + "`" + `
# and ` + "`" + `)` + "`" + ` as ` + "`" + `#OP` + "`" + `/` + "`" + `#CP` + "`" + `, ` + "`" + `[` + "`" + ` and ` + "`" + `]` + "`" + ` as ` + "`" + `#OS` + "`" + `/` + "`" + `#CS` + "`" + `.
#
# Every repetition is a replace loop, never a push per item (AGENTS.md,
# "The reader repeats by replacement"): the container pushes its first
# item (` + "`" + `p` + "`" + `), the item's close replaces it with the next one in the same
# frame (` + "`" + `r` + "`" + `), and the container's close runs once, after the last item.

{
  "options": {
    "rule": { "start": "program" },
    # The engine answers a literally empty source before any rule runs;
    # an empty program is ` + "`" + `[]` + "`" + `, as it is from the ` + "`" + `program` + "`" + ` rule for a
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
    # structural ` + "`" + `\xHH` + "`" + ` and ` + "`" + `\u{...}` + "`" + ` off, and the non-standard ` + "`" + `\v` + "`" + `, ` + "`" + `\'` + "`" + `
    # and ` + "`" + `` + "`" + ` \` + "`" + ` ` + "`" + `` + "`" + ` removed (a null entry deletes an escape; ` + "`" + `""` + "`" + ` would map
    # it to the empty string).
    "string": {
      "chars": "\"",
      "multiChars": "",
      "allowUnknown": false,
      "escapeStrict": true,
      "escape": { "v": null, "'": null, "` + "`" + `": null }
    },
    # ` + "`" + `;` + "`" + ` to the end of the line, and nothing else.
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
    # matcher; the rest are reserved for desugaring, which reports them
    # through its failure with the code leading the message, so a fixture
    # pins ` + "`" + `ERROR:<code>` + "`" + ` for either kind. ` + "`" + `too_deep` + "`" + ` is both: the
    # reader's bound on nesting (256, MAX_NESTING), and the desugarer's on
    # what its rewrites add. Each runtime holds its own raising sites'
    # texts to these (rs: ` + "`" + `the_desugaring_messages_match_the_document` + "`" + `,
    # ` + "`" + `the_too_deep_texts_name_the_bound` + "`" + `).
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
      "bad_match": "match takes a value and (case pattern body) clauses"
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
      "bad_match": "Write match VALUE (case PATTERN BODY)..."
    }
  },

  "rule": {
    # A program: layout lines until the source ends. The node is the array
    # of finished lines. The program pushes the first line and the lines
    # follow it in one frame (see ` + "`" + `line` + "`" + `), so the close runs once, on the
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
    # token or the end of the source (see ` + "`" + `form` + "`" + `). A block's ` + "`" + `#IN` + "`" + ` is
    # taken here and the block pushed. ` + "`" + `#NL` + "`" + ` is the next line of this
    # level, and so is whatever follows a block that did not also close
    # this level: either way the line appends itself to the level's node
    # and replaces itself with the next line (` + "`" + `r` + "`" + `). ` + "`" + `#DE` + "`" + ` and ` + "`" + `#ZZ` + "`" + ` end the
    # level and are left (` + "`" + `b: 1` + "`" + `) for the block or program above. A layout
    # token is always followed by a form (the layout matcher reads a
    # line's indentation up to its first form, and a row with nothing else
    # on it is blank), so ` + "`" + `#NL` + "`" + ` never stands right before ` + "`" + `#DE` + "`" + ` or ` + "`" + `#ZZ` + "`" + `.
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
    # The children of a line, one per level: lines until the ` + "`" + `#DE` + "`" + ` that
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
    # here and is ` + "`" + `unbalanced` + "`" + `.
    #
    # A form is an item of the sequence its parent reads: a line's inline
    # forms, or the items of ` + "`" + `( )` + "`" + ` or ` + "`" + `[ ]` + "`" + `. Its close appends it to the
    # parent's node and then either ends the sequence, leaving the token
    # that ends it (` + "`" + `b: 1` + "`" + `) for the parent, or goes on to the next item in
    # this frame (` + "`" + `r` + "`" + `). What ends a sequence is the parent's: the closer
    # for ` + "`" + `paren` + "`" + ` and ` + "`" + `bracket` + "`" + `, a layout token or the end of the source
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
    # The items of ` + "`" + `( ... )` + "`" + `, filled into the form's list in place: the
    # list pushes its first item and the rest follow it in one frame (see
    # ` + "`" + `form` + "`" + `), so the close runs once, after the last item, and takes the
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
    # The items of ` + "`" + `[ ... ]` + "`" + `, likewise.
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

// GrammarText is the grammar source every runtime embeds, verbatim: the
// text of alchemy-grammar.jsonic at the repository root (after the line
// feed the embedder writes first).
func GrammarText() string { return grammarText }

var (
	documentOnce sync.Once
	documentData []byte
	documentErr  error
)

// document is the grammar document, read from grammarText once per
// process with the strict JSON grammar, comment lexing on, and encoded
// back to JSON, its keys in their written order.
//
// The text is JSON with `#` line comments, so no jsonic is needed. The
// engine's numbers are doubles, and the alternate decoder reads `b` as an
// integer, so whole numbers go back into integer form.
func document() ([]byte, error) {
	documentOnce.Do(func() {
		reader := tabnasjson.Make()
		on := true
		reader.SetOptions(tabnas.Options{Comment: &tabnas.CommentOptions{Lex: &on}})
		value, err := reader.Parse(grammarText)
		if err != nil {
			documentErr = fmt.Errorf("alchemy-grammar.jsonic does not read: %w", err)
			return
		}
		documentData, documentErr = json.Marshal(integralNumbers(value))
	})
	return documentData, documentErr
}

// documentValue is the grammar document as a plain value, for the tests
// that read it.
func documentValue() (map[string]any, error) {
	data, err := document()
	if err != nil {
		return nil, err
	}
	var out map[string]any
	err = json.Unmarshal(data, &out)
	return out, err
}

// integralNumbers puts whole numbers back into integer form.
func integralNumbers(value any) any {
	switch v := value.(type) {
	case float64:
		if v == math.Trunc(v) && math.Abs(v) < 9.0e15 {
			return int64(v)
		}
		return v
	case []any:
		out := make([]any, len(v))
		for i, item := range v {
			out[i] = integralNumbers(item)
		}
		return out
	case *tabnas.OrderedMap:
		out := tabnas.NewOrderedMap()
		for _, key := range v.Keys {
			item, _ := v.Get(key)
			out.Set(key, integralNumbers(item))
		}
		return out
	case map[string]any:
		out := make(map[string]any, len(v))
		for key, item := range v {
			out[key] = integralNumbers(item)
		}
		return out
	}
	return value
}

// ---------------------------------------------------------------------------
// The tagged values
// ---------------------------------------------------------------------------

// seq is a sequence the rules append to: a program's lines, a line's
// items. A pointer, so a rule that starts on its parent's node shares it,
// as the engine's node cells are shared in TypeScript and Rust.
type seq struct {
	items []any
}

// tagged is one node of the tagged tree:
//
//	{"$":"list",   "items":[…], "span":[start,end]}
//	{"$":"vector", "items":[…], "span":[start,end]}
//	{"$":"sym",    "name":…,    "span":[start,end]}
//	{"$":"kw",     "name":…,    "span":[start,end]}
//	{"$":"str",    "value":…,   "span":[start,end]}
//	{"$":"num",    "lexeme":…,  "span":[start,end]}
//	{"$":"bool",   "value":…,   "span":[start,end]}
//	{"$":"null",                "span":[start,end]}
//
// start and end are byte offsets into the source, from the tokens' sites.
type tagged struct {
	tag   string
	key   string
	value any
	items []any
	start int
	end   int
}

func (t *tagged) isSeq() bool { return t.tag == "list" || t.tag == "vector" }

// plain is a node as the tagged value tree ParseValue answers: arrays as
// []any, nodes as ordered maps.
func plain(node any) any {
	switch n := node.(type) {
	case *seq:
		out := make([]any, len(n.items))
		for i, item := range n.items {
			out[i] = plain(item)
		}
		return out
	case *tagged:
		m := tabnas.NewOrderedMap()
		m.Set("$", n.tag)
		if n.isSeq() {
			items := make([]any, len(n.items))
			for i, item := range n.items {
				items[i] = plain(item)
			}
			m.Set("items", items)
		} else if n.key != "" {
			m.Set(n.key, n.value)
		}
		m.Set("span", []any{n.start, n.end})
		return m
	case []any:
		out := make([]any, len(n))
		for i, item := range n {
			out[i] = plain(item)
		}
		return out
	}
	return node
}

func spanOf(node any) (int, int, bool) {
	if t, ok := node.(*tagged); ok {
		return t.start, t.end, true
	}
	return 0, 0, false
}

func tokenSpan(t *tabnas.Token) (int, int) {
	return t.SI, t.SI + len(t.Src)
}

func hasToken(t *tabnas.Token) bool {
	return t != nil && t != tabnas.NoToken && !t.IsNoToken()
}

// atom is the atom a token denotes, by token name.
func atom(t *tabnas.Token, r *tabnas.Rule, ctx *tabnas.Context) *tagged {
	start, end := tokenSpan(t)
	val := t.ResolveVal(r, ctx)
	switch t.Name {
	case tokenKW:
		return &tagged{tag: "kw", key: "name", value: val, start: start, end: end}
	case "#ST":
		return &tagged{tag: "str", key: "value", value: val, start: start, end: end}
	case "#NR":
		return &tagged{tag: "num", key: "lexeme", value: t.Src, start: start, end: end}
	case "#VL":
		if b, ok := val.(bool); ok {
			return &tagged{tag: "bool", key: "value", value: b, start: start, end: end}
		}
		return &tagged{tag: "null", start: start, end: end}
	}
	// `#TX`, and anything a layered grammar might route here.
	return &tagged{tag: "sym", key: "name", value: val, start: start, end: end}
}

// appendTo appends value to the sequence in node: the array of a
// program's lines or of a line's items, or the items of a tagged list or
// vector. Any other node is left alone.
func appendTo(node any, value any) {
	switch n := node.(type) {
	case *seq:
		n.items = append(n.items, value)
	case *tagged:
		if n.isSeq() {
			n.items = append(n.items, value)
		}
	}
}

// isCell is whether node is one this grammar builds.
func isCell(node any) bool {
	switch node.(type) {
	case *seq, *tagged:
		return true
	}
	return false
}

// appendToParent appends r's finished node to its container's: how an item
// of a replace loop reaches the sequence it belongs to. A replace hands the
// parent on, so every item of one sequence has the container as its
// parent.
func appendToParent(r *tabnas.Rule) {
	parent := r.Parent
	if parent == nil || parent == tabnas.NoRule || !isCell(parent.Node) {
		return
	}
	// Every item has a cell of its own by its close; appending a cell to
	// itself would be a cycle, never a value.
	if isCell(r.Node) && parent.Node == r.Node {
		return
	}
	appendTo(parent.Node, r.Node)
}

// childrenStart is where the lines of r's latest block begin among its
// items, once the line has taken a block.
func childrenStart(r *tabnas.Rule) (int, bool) {
	v, ok := r.U[uChildren].(int)
	return v, ok
}

// parentIs is whether r is an item of a sequence the rule name reads.
func parentIs(r *tabnas.Rule, name string) bool {
	return r.Parent != nil && r.Parent != tabnas.NoRule && r.Parent.Name == name
}

// refs is every action, condition and error hook the document names.
func refs(layout tabnas.MakeLexMatcher) map[tabnas.FuncRef]any {
	return map[tabnas.FuncRef]any{
		layoutMatcherRef: layout,
		refArray: tabnas.AltAction(func(r *tabnas.Rule, _ *tabnas.Context) {
			r.Node = &seq{}
		}),
		// A line takes one block. A second can only follow a layout line
		// with no form on it, whose first character is a lone `\r`: the
		// earlier block then stays one array item of this line, before the
		// next block's lines, which Expr refuses as malformed.
		refChildren: tabnas.AltAction(func(r *tabnas.Rule, _ *tabnas.Context) {
			earlier, has := childrenStart(r)
			start := 0
			if s, ok := r.Node.(*seq); ok {
				if has && earlier <= len(s.items) {
					lines := append([]any(nil), s.items[earlier:]...)
					s.items = append(s.items[:earlier:earlier], &seq{items: lines})
				}
				start = len(s.items)
			}
			r.EnsureU()[uChildren] = start
		}),
		// The line's items are in its node: the inline forms, each
		// appended by its own close, then the child lines, appended by
		// theirs through the block that ran on this node.
		refLineEnd: tabnas.AltAction(func(r *tabnas.Rule, _ *tabnas.Context) {
			_, children := childrenStart(r)
			var items []any
			if s, ok := r.Node.(*seq); ok {
				items = append([]any(nil), s.items...)
			}
			if len(items) == 1 && !children {
				r.Node = items[0]
			} else {
				start, end := 0, 0
				if len(items) > 0 {
					if s, _, ok := spanOf(items[0]); ok {
						start = s
					}
					end = start
					if _, e, ok := spanOf(items[len(items)-1]); ok {
						end = e
					}
				}
				r.Node = &tagged{tag: "list", items: items, start: start, end: end}
			}
			appendToParent(r)
		}),
		refOpenList: tabnas.AltAction(func(r *tabnas.Rule, _ *tabnas.Context) {
			start, end := 0, 0
			if hasToken(r.O0) {
				start, end = tokenSpan(r.O0)
			}
			r.Node = &tagged{tag: "list", items: []any{}, start: start, end: end}
		}),
		refOpenVector: tabnas.AltAction(func(r *tabnas.Rule, _ *tabnas.Context) {
			start, end := 0, 0
			if hasToken(r.O0) {
				start, end = tokenSpan(r.O0)
			}
			r.Node = &tagged{tag: "vector", items: []any{}, start: start, end: end}
		}),
		refItem: tabnas.AltAction(func(r *tabnas.Rule, _ *tabnas.Context) {
			appendToParent(r)
		}),
		// The items are already in the list; the closer ends its span.
		refCloseSeq: tabnas.AltAction(func(r *tabnas.Rule, _ *tabnas.Context) {
			if t, ok := r.Node.(*tagged); ok && hasToken(r.C0) {
				_, t.end = tokenSpan(r.C0)
			}
		}),
		refAtom: tabnas.AltAction(func(r *tabnas.Rule, ctx *tabnas.Context) {
			if hasToken(r.O0) {
				r.Node = atom(r.O0, r, ctx)
			}
		}),
		refHasChildren: tabnas.AltCond(func(r *tabnas.Rule, _ *tabnas.Context) bool {
			_, ok := childrenStart(r)
			return ok
		}),
		refInLine: tabnas.AltCond(func(r *tabnas.Rule, _ *tabnas.Context) bool {
			return parentIs(r, "line")
		}),
		refInParen: tabnas.AltCond(func(r *tabnas.Rule, _ *tabnas.Context) bool {
			return parentIs(r, "paren")
		}),
		refInBracket: tabnas.AltCond(func(r *tabnas.Rule, _ *tabnas.Context) bool {
			return parentIs(r, "bracket")
		}),
		// The offending token is the one under the cursor: the closer with
		// nothing open, or the `#ZZ` inside an open delimiter.
		refUnbalanced: tabnas.AltError(func(_ *tabnas.Rule, ctx *tabnas.Context) *tabnas.Token {
			if !hasToken(ctx.T0) {
				return nil
			}
			t := *ctx.T0
			t.Use = nil
			return t.Bad("unbalanced")
		}),
	}
}

// Alchemy installs the alchemy grammar on j: the plugin. Make goes through
// it too, so the two construction paths cannot drift apart. The layout
// tokens and every reference the document names are registered before the
// document is installed.
func Alchemy(j *tabnas.Tabnas, _ map[string]any) error {
	data, err := document()
	if err != nil {
		return err
	}
	layout := registerLayout(j)
	gs, err := tabnas.GrammarSpecFromJSON(data)
	if err != nil {
		return fmt.Errorf("alchemy: the grammar document does not load: %w", err)
	}
	gs.Ref = refs(layout)
	if err := j.Grammar(gs); err != nil {
		return fmt.Errorf("alchemy: the grammar does not install: %w", err)
	}
	return nil
}

// Make builds an alchemy parser instance. It panics only on a defect of
// this package: the document is a fixed literal.
func Make() *tabnas.Tabnas {
	j := tabnas.Make()
	if err := j.Use(Alchemy); err != nil {
		panic(fmt.Sprintf("the alchemy grammar document is fixed and valid: %v", err))
	}
	return j
}

var (
	defaultOnce   sync.Once
	defaultParser *tabnas.Tabnas
)

func shared() *tabnas.Tabnas {
	defaultOnce.Do(func() { defaultParser = Make() })
	return defaultParser
}

// ParseValue parses a program to the tagged value tree, with the engine's
// own error (a *tabnas.TabnasError). The engine is built once, on first
// use, and shared: a parse builds a fresh context, and the layout state
// lives in that context.
func ParseValue(src string) (any, error) {
	v, err := shared().Parse(src)
	if err != nil {
		return nil, err
	}
	return plain(v), nil
}

// ParseFile parses a program read from file (the name spans carry) to its
// forms.
func ParseFile(src, file string) ([]*Expr, *Fail) {
	value, err := ParseValue(src)
	if err != nil {
		var te *tabnas.TabnasError
		if errors.As(err, &te) {
			return nil, failFromTabnas(te)
		}
		return nil, NewFail(CodeDSLParseError, err.Error())
	}
	return ProgramFromValue(value, NewFile(file))
}

// Parse parses a program to its forms, with spans naming Unnamed.
func Parse(src string) ([]*Expr, *Fail) {
	return ParseFile(src, Unnamed)
}
