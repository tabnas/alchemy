//! The alchemy grammar plugin: the serialized grammar document, the native
//! actions it names, and the parse entry points.
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
//! # How the rules share cells
//!
//! A pushed rule starts on its parent's node cell, so every rule here that
//! builds a value of its own first replaces its cell (`@alchemy-array`,
//! `@alchemy-atom`, the two openers), and a parent reads a finished child
//! through [`Rule::child_value`]. The one deliberate exception is `paren`
//! and `bracket`, which are pushed onto the tagged list their `form`
//! parent has just created and fill its `items` in place; when they pop,
//! the parent's node already is the finished list.

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
use tabnas_transduce::{Code, Fail};

use crate::ast::Expr;
use crate::lex;

/// The file name spans carry when a program is parsed from a string
/// rather than a file.
pub const UNNAMED: &str = "<program>";

/// The action and condition references the document names, all registered
/// by [`alchemy`] before the document is installed.
const A_ARRAY: &str = "@alchemy-array";
const A_COLLECT: &str = "@alchemy-collect";
const A_CHILDREN: &str = "@alchemy-children";
const A_LINE_END: &str = "@alchemy-line-end";
const A_OPEN_LIST: &str = "@alchemy-open-list";
const A_OPEN_VECTOR: &str = "@alchemy-open-vector";
const A_ITEM: &str = "@alchemy-item";
const A_CLOSE_SEQ: &str = "@alchemy-close-seq";
const A_ATOM: &str = "@alchemy-atom";
const C_CHILDREN: &str = "@alchemy-has-children";
const E_UNBALANCED: &str = "@alchemy-unbalanced";

/// The rule-local flag a `line` sets once it has taken a block of
/// children. In `u`, the bag that does not descend, because it describes
/// this line alone.
const U_CHILDREN: &str = "children";

/// The grammar document: options and rules together, so there is one
/// definition of the language rather than two halves that can drift.
fn document() -> serde_json::Value {
    json!({
        "options": {
            "rule": { "start": "program" },
            // The engine answers a literally empty source before any rule
            // runs; an empty program is `[]`, as it is from the `program`
            // rule for a source holding only blank and comment lines.
            "lex": {
                "empty": true,
                "emptyResult": [],
                "match": {
                    // Below the first built-in band (1e6): indentation and
                    // words are seen before the space, fixed and text
                    // matchers.
                    "alchemy": { "order": 1e5, "make": lex::MATCHER },
                },
            },
            // Parens and brackets are the only fixed tokens; the engine's
            // JSON punctuation goes.
            "fixed": {
                "token": {
                    "#OB": null, "#CB": null, "#CL": null, "#CA": null,
                    "#OP": "(", "#CP": ")",
                },
            },
            // Words are lexed by the layout matcher, whose boundaries are
            // the symbol alphabet; the engine's text, number and value
            // matchers would cut them differently.
            "text": { "lex": false },
            "number": { "lex": false },
            "value": { "lex": false },
            // Double-quoted strings with exactly the JSON escapes, as the
            // json grammar configures them: unknown escapes refused, the
            // engine's structural `\xHH` and `\u{...}` off, and the
            // non-standard `\v`, `\'` and `` \` `` removed (a null entry
            // deletes an escape; `""` would map it to the empty string).
            "string": {
                "chars": "\"",
                "multiChars": "",
                "allowUnknown": false,
                "escapeStrict": true,
                "escape": { "v": null, "'": null, "`": null },
            },
            // `;` to the end of the line, and nothing else.
            "comment": {
                "lex": true,
                "def": {
                    "hash": null,
                    "slash": null,
                    "multi": null,
                    "semi": { "line": true, "start": ";", "lex": true, "eatline": false },
                },
            },
            // This grammar's own codes. The first four are raised by the
            // layout matcher; the rest are reserved for `desugar`, which
            // reports them through `Fail` with the code leading the
            // message, so a fixture pins `ERROR:<code>` for either kind.
            "error": {
                "tab_indent": "tab in indentation",
                "bad_indent": "unexpected indentation before: {src}",
                "bad_dedent": "dedent to a level no block opened, before: {src}",
                "unbalanced": "unbalanced delimiter: {src}",
                "empty_step": "a pipe step must be a symbol or a non-empty list",
                "bad_def": "def takes a name and a value, or a name, [params] and a body",
                "bad_let": "let takes one binding [name value] and one body",
                "bad_if": "if takes a condition and exactly two branches",
                "bad_match": "match takes a value and (case pattern body) clauses",
            },
            "hint": {
                "tab_indent": "Indentation is spaces only, two per level. Replace the tab with spaces.",
                "bad_indent": "A child line is exactly two spaces deeper than its parent, and the\nfirst line of a program is not indented.",
                "bad_dedent": "A line may only return to the indentation of a line above it.",
                "unbalanced": "Every ( and [ needs its ) or ], and nothing closes what was not\nopened. Inside delimiters, line breaks and indentation do not count.",
                "empty_step": "In pipe VALUE STEP..., each step is a symbol (called with the value)\nor a list (the value is appended as its last argument).",
                "bad_def": "Write def NAME VALUE, or def NAME [PARAMS] BODY.",
                "bad_let": "Write let [NAME VALUE] BODY.",
                "bad_if": "Write if CONDITION THEN ELSE.",
                "bad_match": "Write match VALUE (case PATTERN BODY)...",
            },
        },

        "rule": {
            // A program: layout lines until the source ends. The node is
            // the array of finished lines.
            "program": {
                "open": [
                    { "s": "#ZZ", "a": A_ARRAY, "g": "alchemy" },
                    { "p": "line", "a": A_ARRAY, "g": "alchemy" },
                ],
                "close": [
                    { "s": "#ZZ", "a": A_COLLECT, "g": "alchemy" },
                    { "p": "line", "a": A_COLLECT, "g": "alchemy" },
                ],
            },
            // A layout line: inline forms, then what ends it. `#DE` and
            // `#ZZ` are left (`b: 1`) for the block or program above; a
            // block's `#IN` is taken here and the block pushed, and once
            // the block is back the line closes on whatever follows
            // without consuming it.
            "line": {
                "open": [
                    { "p": "form", "a": A_ARRAY, "g": "alchemy" },
                ],
                "close": [
                    { "s": "#IN", "p": "block", "a": A_CHILDREN, "g": "alchemy" },
                    { "s": "#NL", "a": A_LINE_END, "g": "alchemy" },
                    { "s": "#DE", "b": 1, "a": A_LINE_END, "g": "alchemy" },
                    { "s": "#ZZ", "b": 1, "a": A_LINE_END, "g": "alchemy" },
                    { "c": C_CHILDREN, "a": A_LINE_END, "g": "alchemy" },
                    { "p": "form", "a": A_COLLECT, "g": "alchemy" },
                ],
            },
            // The children of a line, one per level: lines until the
            // `#DE` that closes the level, or the end of the source.
            "block": {
                "open": [
                    { "p": "line", "a": A_ARRAY, "g": "alchemy" },
                ],
                "close": [
                    { "s": "#DE", "a": A_COLLECT, "g": "alchemy" },
                    { "s": "#ZZ", "b": 1, "a": A_COLLECT, "g": "alchemy" },
                    { "p": "line", "a": A_COLLECT, "g": "alchemy" },
                ],
            },
            // One form: an explicit list or vector, or an atom. A closer
            // with nothing open, or the end of the source inside a
            // delimiter, arrives here and is `unbalanced`.
            "form": {
                "open": [
                    { "s": "#OP", "p": "paren", "a": A_OPEN_LIST, "g": "alchemy" },
                    { "s": "#OS", "p": "bracket", "a": A_OPEN_VECTOR, "g": "alchemy" },
                    { "s": "#TX", "a": A_ATOM, "g": "alchemy" },
                    { "s": "#KW", "a": A_ATOM, "g": "alchemy" },
                    { "s": "#ST", "a": A_ATOM, "g": "alchemy" },
                    { "s": "#NR", "a": A_ATOM, "g": "alchemy" },
                    { "s": "#VL", "a": A_ATOM, "g": "alchemy" },
                    { "s": "#CP", "e": E_UNBALANCED, "g": "alchemy" },
                    { "s": "#CS", "e": E_UNBALANCED, "g": "alchemy" },
                    { "s": "#ZZ", "e": E_UNBALANCED, "g": "alchemy" },
                ],
                "close": [],
            },
            // The items of `( ... )`, filled into the form's list in
            // place. An empty list leaves its closer for the close phase.
            "paren": {
                "open": [
                    { "s": "#CP", "b": 1, "g": "alchemy" },
                    { "p": "form", "g": "alchemy" },
                ],
                "close": [
                    { "s": "#CP", "a": A_CLOSE_SEQ, "g": "alchemy" },
                    { "p": "form", "a": A_ITEM, "g": "alchemy" },
                ],
            },
            // The items of `[ ... ]`, likewise.
            "bracket": {
                "open": [
                    { "s": "#CS", "b": 1, "g": "alchemy" },
                    { "p": "form", "g": "alchemy" },
                ],
                "close": [
                    { "s": "#CS", "a": A_CLOSE_SEQ, "g": "alchemy" },
                    { "p": "form", "a": A_ITEM, "g": "alchemy" },
                ],
            },
        },

        "ruleOrder": ["program", "line", "block", "form", "paren", "bracket"],
    })
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

/// Append `value` to the array in `rule`'s node; a node that is not an
/// array is left alone (the rules only ever call this on one that is).
fn push_node(rule: &mut Rule, value: Value) {
    if let Value::Array(items) = &mut *rule.node.borrow_mut() {
        Arc::make_mut(items).push(value);
    }
}

/// Append `value` to the `items` of the tagged list in `rule`'s node.
fn push_item(rule: &mut Rule, value: Value) {
    if let Value::Object(fields) = &mut *rule.node.borrow_mut() {
        if let Some(Value::Array(items)) = Arc::make_mut(fields).get_mut("items") {
            Arc::make_mut(items).push(value);
        }
    }
}

fn has_children(rule: &Rule) -> bool {
    matches!(rule.u.get(U_CHILDREN), Some(Value::Bool(true)))
}

/// Register every action, condition and error hook the document names.
fn register_refs(parser: &mut Tabnas) {
    parser.action(A_ARRAY, |rule| fresh(rule, Value::array(Vec::new())));
    parser.action(A_COLLECT, |rule| {
        if rule.has_child_value() {
            let child = rule.child_value();
            push_node(rule, child);
        }
    });
    parser.action(A_CHILDREN, |rule| {
        if rule.has_child_value() {
            let child = rule.child_value();
            push_node(rule, child);
        }
        rule.u_mut()
            .insert(U_CHILDREN.to_string(), Value::Bool(true));
    });
    parser.action(A_LINE_END, |rule| {
        let children = has_children(rule);
        let mut items: Vec<Value> = match &*rule.node.borrow() {
            Value::Array(items) => items.as_ref().clone(),
            _ => Vec::new(),
        };
        if rule.has_child_value() {
            match rule.child_value() {
                // The block's lines, when this line took a block.
                Value::Array(lines) if children => items.extend(lines.iter().cloned()),
                form => items.push(form),
            }
        }
        if items.len() == 1 && !children {
            let only = items.pop().expect("one item was just counted");
            fresh(rule, only);
            return;
        }
        let start = items.first().and_then(span_of).map_or(0, |span| span.0);
        let end = items.last().and_then(span_of).map_or(start, |span| span.1);
        fresh(
            rule,
            tagged("list", vec![("items", Value::array(items))], (start, end)),
        );
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
    parser.action(A_ITEM, |rule| {
        if rule.has_child_value() {
            let child = rule.child_value();
            push_item(rule, child);
        }
    });
    parser.action(A_CLOSE_SEQ, |rule| {
        if rule.has_child_value() {
            let child = rule.child_value();
            push_item(rule, child);
        }
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
/// # Ok::<(), tabnas_transduce::Fail>(())
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
    }

    #[test]
    fn the_fail_carries_the_code_and_the_position() {
        let fail = parse("a\n  b\n c").expect_err("a bad dedent");
        assert_eq!(fail.code, Code::DslParseError);
        assert!(fail.message.starts_with("bad_dedent: "), "{}", fail.message);
        assert_eq!((fail.row, fail.column), (Some(3), Some(2)));
    }
}
