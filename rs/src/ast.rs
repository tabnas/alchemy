//! The syntax tree: [`Expr`] with source spans, built from the reader's
//! tagged value tree, and the two printers, canonical and layout.
//!
//! This module is the input of the checker, planner and interpreter that
//! follow (design brief section 4.2): they read `Expr` and never the tagged
//! tree, so the grammar's output shape is the reader's business alone.
//!
//! A span is a pair of byte offsets into the source plus the file name;
//! the 1-based row and column a diagnostic prints are derived from the
//! source when needed rather than stored, so a span costs two words and a
//! shared name.
//!
//! Every `Expr` this crate builds nests at most [`MAX_NESTING`] levels:
//! the reader refuses a deeper program before building anything,
//! [`Expr::from_value`] refuses a deeper tagged tree without recursing
//! over it, and the desugarer refuses a rewrite that would nest deeper.
//! That bound is what lets the printers, the desugarer and `Drop` recurse
//! per level, and this module's recursive functions rely on it.

use std::fmt;
use std::sync::Arc;

use tabnas::Value;
use tabnas_transduce::{Code, Fail};

/// The most levels a form may nest: a list or vector inside a list or
/// vector, this many times over.
///
/// The reader counts a layout line, each indentation level and each open
/// `(` or `[` as one and refuses the opener or the line that would pass
/// the bound with `too_deep`; the desugarer counts what its rewrites add
/// (a `pipe` nests one level per step) and refuses the same way. Without
/// a bound, a two-kilobyte program of nested parentheses took the process
/// down with a stack overflow in the recursive stages after the parse,
/// instead of failing as a `DSL_PARSE_ERROR`. 256 is the depth
/// `tabnas_transduce::Limits` gives documents, and far past any program
/// written by hand.
pub const MAX_NESTING: usize = 256;

/// The message of a `too_deep` failure, one text for the grammar document,
/// this module and the desugarer; a test in [`crate::grammar`] holds it to
/// [`MAX_NESTING`].
pub(crate) const TOO_DEEP: &str = "nesting deeper than 256 levels";

/// Where a form came from: `file`, and the byte range `start..end` of its
/// source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSpan {
    pub file: Arc<str>,
    pub start: usize,
    pub end: usize,
}

impl SourceSpan {
    pub fn new(file: &Arc<str>, start: usize, end: usize) -> SourceSpan {
        SourceSpan {
            file: Arc::clone(file),
            start,
            end,
        }
    }

    /// The 1-based row and column of `start` in `src`, the column counted
    /// in characters, as the engine counts them: a row ends at a line
    /// feed, and a column restarts at a line feed or at a carriage return,
    /// whether or not a line feed follows it (the engine's line matcher
    /// resets the column after any run of line characters). Holding to
    /// that here keeps a position derived from a span in step with one the
    /// engine reports, on a source with lone carriage returns included. A
    /// start past the end of `src` answers the position just after its
    /// last character.
    pub fn position(&self, src: &str) -> (usize, usize) {
        let start = self.start.min(src.len());
        let before = &src[..floor_boundary(src, start)];
        let row = before.matches('\n').count() + 1;
        let line_start = before.rfind(['\n', '\r']).map_or(0, |index| index + 1);
        let col = before[line_start..].chars().count() + 1;
        (row, col)
    }
}

/// The texts a program is compiled from, each under the file name its
/// spans carry: one for [`crate::compile`], several for
/// [`crate::compile_sources`]. A diagnostic positions a span in the text
/// of the span's own file, and names the file when there are several.
#[derive(Clone, Debug)]
pub struct Sources {
    files: Arc<[(Arc<str>, Arc<str>)]>,
}

impl Sources {
    /// One text, named `file`.
    pub fn one(file: &str, text: &str) -> Sources {
        Sources {
            files: Arc::from(vec![(Arc::from(file), Arc::from(text))]),
        }
    }

    /// Several texts, each under its name, in order; the caller holds the
    /// names distinct, since a span names its file by name.
    pub(crate) fn several(files: Vec<(Arc<str>, Arc<str>)>) -> Sources {
        debug_assert!(!files.is_empty(), "a program has at least one source");
        Sources {
            files: Arc::from(files),
        }
    }

    /// The first file's name: the program's, when there is one.
    pub fn first(&self) -> &Arc<str> {
        &self.files[0].0
    }

    /// Whether a diagnostic names its file: when the sources are several.
    pub fn names_files(&self) -> bool {
        self.files.len() > 1
    }

    /// The text of `span`'s file, when it is one of these.
    fn find(&self, span: &SourceSpan) -> Option<&str> {
        self.files
            .iter()
            .find(|(file, _)| **file == *span.file)
            .map(|(_, text)| &**text)
    }

    /// The text a span is positioned in: its file's, or the first when its
    /// file is not among these (a span of the standard library, whose text
    /// [`crate::stdlib::file_of`] gives, or of an unnamed parse).
    pub fn text_of(&self, span: &SourceSpan) -> &str {
        self.find(span).unwrap_or(&self.files[0].1)
    }

    /// The 1-based row and column of `span` in its file's text.
    pub fn position(&self, span: &SourceSpan) -> (usize, usize) {
        span.position(self.text_of(span))
    }

    /// `fail` at `span`: its row and column, and its file when the sources
    /// are several and the span is in one of them.
    pub fn fail_at(&self, fail: Fail, span: &SourceSpan) -> Fail {
        let (row, col) = self.position(span);
        let fail = fail.at(row as u64, col as u64);
        if self.names_files() && self.find(span).is_some() {
            fail.in_file(&*span.file)
        } else {
            fail
        }
    }
}

/// The largest character boundary at or before `index`.
fn floor_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

impl fmt::Display for SourceSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}..{}", self.file, self.start, self.end)
    }
}

/// One form of a program.
///
/// A number keeps its lexeme rather than a value: the language's numbers
/// are JSON numbers, and what a renderer prints is the text the author
/// wrote, exactly as `tabnas-transduce` keeps a source number's lexeme.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Symbol { name: String, span: SourceSpan },
    Keyword { name: String, span: SourceSpan },
    Str { value: String, span: SourceSpan },
    Num { lexeme: String, span: SourceSpan },
    Bool { value: bool, span: SourceSpan },
    Null { span: SourceSpan },
    List { items: Vec<Expr>, span: SourceSpan },
    Vector { items: Vec<Expr>, span: SourceSpan },
}

fn malformed(what: impl fmt::Display) -> Fail {
    Fail::new(
        Code::DslParseError,
        format!("malformed reader output: {what}"),
    )
}

fn field<'v>(fields: &'v indexmap::IndexMap<String, Value>, name: &str) -> Result<&'v Value, Fail> {
    fields
        .get(name)
        .ok_or_else(|| malformed(format!("a node without {name:?}")))
}

fn text(fields: &indexmap::IndexMap<String, Value>, name: &str) -> Result<String, Fail> {
    match field(fields, name)? {
        Value::String(text) => Ok(text.clone()),
        other => Err(malformed(format!("{name:?} is not a string: {other}"))),
    }
}

fn offset(value: &Value) -> Result<usize, Fail> {
    match value {
        Value::Number(n) if n.is_finite() && *n >= 0.0 && n.fract() == 0.0 => Ok(*n as usize),
        other => Err(malformed(format!(
            "a span offset that is not a whole number: {other}"
        ))),
    }
}

/// One node of the tagged tree, read before its items are: an atom is a
/// finished form, a container still has its items to build.
enum Node<'v> {
    Atom(Expr),
    Container {
        list: bool,
        span: SourceSpan,
        items: &'v [Value],
    },
}

/// A container whose items are being built, on [`Expr::from_value`]'s own
/// stack rather than the call stack.
struct Frame<'v> {
    list: bool,
    span: SourceSpan,
    items: std::slice::Iter<'v, Value>,
    built: Vec<Expr>,
}

impl Frame<'_> {
    fn finish(self) -> Expr {
        if self.list {
            Expr::List {
                items: self.built,
                span: self.span,
            }
        } else {
            Expr::Vector {
                items: self.built,
                span: self.span,
            }
        }
    }
}

fn read_node<'v>(value: &'v Value, file: &Arc<str>) -> Result<Node<'v>, Fail> {
    let Value::Object(fields) = value else {
        return Err(malformed(format!("a node that is not an object: {value}")));
    };
    let span = match field(fields, "span")? {
        Value::Array(pair) if pair.len() == 2 => {
            SourceSpan::new(file, offset(&pair[0])?, offset(&pair[1])?)
        }
        other => return Err(malformed(format!("a span that is not a pair: {other}"))),
    };
    let tag = text(fields, "$")?;
    Ok(match tag.as_str() {
        "sym" => Node::Atom(Expr::Symbol {
            name: text(fields, "name")?,
            span,
        }),
        "kw" => Node::Atom(Expr::Keyword {
            name: text(fields, "name")?,
            span,
        }),
        "str" => Node::Atom(Expr::Str {
            value: text(fields, "value")?,
            span,
        }),
        "num" => Node::Atom(Expr::Num {
            lexeme: text(fields, "lexeme")?,
            span,
        }),
        "bool" => match field(fields, "value")? {
            Value::Bool(value) => Node::Atom(Expr::Bool {
                value: *value,
                span,
            }),
            other => return Err(malformed(format!("a bool that is not a bool: {other}"))),
        },
        "null" => Node::Atom(Expr::Null { span }),
        "list" | "vector" => match field(fields, "items")? {
            Value::Array(items) => Node::Container {
                list: tag == "list",
                span,
                items: items.as_slice(),
            },
            other => return Err(malformed(format!("items that are not an array: {other}"))),
        },
        other => return Err(malformed(format!("an unknown tag {other:?}"))),
    })
}

fn too_deep(span: &SourceSpan) -> Fail {
    Fail::new(
        Code::DslParseError,
        format!("too_deep: {TOO_DEEP} (at {span})"),
    )
}

impl Expr {
    /// Build a form from one node of the reader's tagged tree (see
    /// [`crate::grammar`] for the shape). A tree the reader did not build
    /// may be malformed; that is a `DSL_PARSE_ERROR`, never a panic.
    ///
    /// Iterative, over an explicit stack of open containers: the reader
    /// bounds the depth of its own tree, but a grammar layered on it could
    /// hand over a deeper one, and the conversion must refuse it as
    /// `too_deep` (past [`MAX_NESTING`] open containers) rather than
    /// recurse into it.
    pub fn from_value(value: &Value, file: &Arc<str>) -> Result<Expr, Fail> {
        let mut stack: Vec<Frame<'_>> = Vec::new();
        let mut pending = value;
        loop {
            let mut built = match read_node(pending, file)? {
                Node::Atom(expr) => expr,
                Node::Container { list, span, items } => {
                    if stack.len() >= MAX_NESTING {
                        return Err(too_deep(&span));
                    }
                    let mut frame = Frame {
                        list,
                        span,
                        items: items.iter(),
                        built: Vec::with_capacity(items.len()),
                    };
                    match frame.items.next() {
                        Some(first) => {
                            stack.push(frame);
                            pending = first;
                            continue;
                        }
                        None => frame.finish(),
                    }
                }
            };
            // A finished form belongs to the innermost open container,
            // whose next item is built next; a container out of items is
            // finished in turn, up to the root.
            loop {
                let Some(mut frame) = stack.pop() else {
                    return Ok(built);
                };
                frame.built.push(built);
                match frame.items.next() {
                    Some(item) => {
                        pending = item;
                        stack.push(frame);
                        break;
                    }
                    None => built = frame.finish(),
                }
            }
        }
    }

    /// Build a whole program from the reader's array of top-level nodes.
    pub fn program_from_value(value: &Value, file: &Arc<str>) -> Result<Vec<Expr>, Fail> {
        match value {
            Value::Array(forms) => forms
                .iter()
                .map(|form| Expr::from_value(form, file))
                .collect(),
            other => Err(malformed(format!(
                "a program that is not an array: {other}"
            ))),
        }
    }

    pub fn span(&self) -> &SourceSpan {
        match self {
            Expr::Symbol { span, .. }
            | Expr::Keyword { span, .. }
            | Expr::Str { span, .. }
            | Expr::Num { span, .. }
            | Expr::Bool { span, .. }
            | Expr::Null { span }
            | Expr::List { span, .. }
            | Expr::Vector { span, .. } => span,
        }
    }

    /// The symbol's name, when this form is a symbol.
    pub fn symbol(&self) -> Option<&str> {
        match self {
            Expr::Symbol { name, .. } => Some(name),
            _ => None,
        }
    }

    /// Whether this form is an atom: anything but a list or a vector.
    pub fn is_atom(&self) -> bool {
        !matches!(self, Expr::List { .. } | Expr::Vector { .. })
    }

    /// Structural equality that ignores spans: the same forms with the
    /// same names, values and lexemes, wherever they came from.
    pub fn same_shape(&self, other: &Expr) -> bool {
        match (self, other) {
            (Expr::Symbol { name: a, .. }, Expr::Symbol { name: b, .. })
            | (Expr::Keyword { name: a, .. }, Expr::Keyword { name: b, .. })
            | (Expr::Str { value: a, .. }, Expr::Str { value: b, .. })
            | (Expr::Num { lexeme: a, .. }, Expr::Num { lexeme: b, .. }) => a == b,
            (Expr::Bool { value: a, .. }, Expr::Bool { value: b, .. }) => a == b,
            (Expr::Null { .. }, Expr::Null { .. }) => true,
            (Expr::List { items: a, .. }, Expr::List { items: b, .. })
            | (Expr::Vector { items: a, .. }, Expr::Vector { items: b, .. }) => same_program(a, b),
            _ => false,
        }
    }
}

/// [`Expr::same_shape`] over two programs, form by form.
pub fn same_program(a: &[Expr], b: &[Expr]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same_shape(y))
}

// ---------------------------------------------------------------------------
// Canonical form
// ---------------------------------------------------------------------------

/// One form, fully parenthesized on one line: strings as JSON literals,
/// numbers by lexeme, vectors in brackets, keywords with their colon.
///
/// Recursive per level, which [`MAX_NESTING`] bounds for every form this
/// crate builds.
pub fn canonical_form(expr: &Expr) -> String {
    let mut out = String::new();
    write_canonical(expr, &mut out);
    out
}

fn write_canonical(expr: &Expr, out: &mut String) {
    match expr {
        Expr::Symbol { name, .. } => out.push_str(name),
        Expr::Keyword { name, .. } => {
            out.push(':');
            out.push_str(name);
        }
        Expr::Str { value, .. } => out.push_str(&json_string(value)),
        Expr::Num { lexeme, .. } => out.push_str(lexeme),
        Expr::Bool { value, .. } => out.push_str(if *value { "true" } else { "false" }),
        Expr::Null { .. } => out.push_str("null"),
        Expr::List { items, .. } => write_items(items, '(', ')', out),
        Expr::Vector { items, .. } => write_items(items, '[', ']', out),
    }
}

fn write_items(items: &[Expr], open: char, close: char, out: &mut String) {
    out.push(open);
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        write_canonical(item, out);
    }
    out.push(close);
}

/// The JSON literal for `value`. `serde_json` escapes exactly the way the
/// reader unescapes, so a canonical string reads back to the same value.
fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| {
        // Serializing a `str` to JSON cannot fail; this arm only keeps the
        // signature free of a Result for an impossible error.
        String::from("\"\"")
    })
}

/// A whole program in canonical form: one top-level form per line.
///
/// ```
/// use tabnas_alchemy::{canonical, parse};
/// let program = parse("join \",\"\n  map csv-field values\n\n(newline)")?;
/// assert_eq!(canonical(&program), "(join \",\" (map csv-field values))\n(newline)");
/// # Ok::<(), tabnas_transduce::Fail>(())
/// ```
pub fn canonical(program: &[Expr]) -> String {
    program
        .iter()
        .map(canonical_form)
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Layout form
// ---------------------------------------------------------------------------

/// A whole program in layout form, the shape the reader reads back to the
/// same program (see `format_round_trips_every_fixture` in the crate's
/// tests).
///
/// The rules: an atom or a vector prints on one line in canonical form (a
/// vector's brackets are explicit, so its contents are canonical too). A
/// list with fewer than two items, or whose first item is a list, prints
/// canonically with explicit parens, because a layout line cannot express
/// it: `(newline)` as a line would read as the symbol, and a line cannot
/// begin with a child. Any other list puts its leading atoms and vectors
/// on one line and each remaining item on its own line two spaces deeper,
/// so `(join "," (map f xs))` is `join ","` over a child `map f xs`.
/// Top-level forms are separated by a blank line.
///
/// The reader counts a layout line as a level whether or not it makes a
/// list, so its count of a printed form can exceed the form's depth by
/// one: a program that reads exactly at [`MAX_NESTING`] may print, in this
/// form or the canonical one, to a text the reader refuses as `too_deep`.
/// Below the bound both forms read back exactly.
///
/// ```
/// use tabnas_alchemy::{format, parse};
/// let program = parse("(def csv-row [values] (concat (join \",\" (map csv-field values)) (newline)))")?;
/// assert_eq!(
///     format(&program),
///     "def csv-row [values]\n  concat\n    join \",\"\n      map csv-field values\n    (newline)\n"
/// );
/// # Ok::<(), tabnas_transduce::Fail>(())
/// ```
pub fn format(program: &[Expr]) -> String {
    let mut out = String::new();
    for (index, form) in program.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        write_layout(form, 0, &mut out);
    }
    out
}

/// Whether `expr` can sit on a layout line beside other forms without
/// changing what the line means.
fn is_inline(expr: &Expr) -> bool {
    !matches!(expr, Expr::List { .. })
}

fn write_layout(expr: &Expr, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    let Expr::List { items, .. } = expr else {
        out.push_str(&indent);
        out.push_str(&canonical_form(expr));
        out.push('\n');
        return;
    };
    let head = items.iter().take_while(|item| is_inline(item)).count();
    if items.len() < 2 || head == 0 {
        out.push_str(&indent);
        out.push_str(&canonical_form(expr));
        out.push('\n');
        return;
    }
    out.push_str(&indent);
    out.push_str(
        &items[..head]
            .iter()
            .map(canonical_form)
            .collect::<Vec<_>>()
            .join(" "),
    );
    out.push('\n');
    for child in &items[head..] {
        write_layout(child, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn sym(name: &str) -> Expr {
        Expr::Symbol {
            name: name.into(),
            span: SourceSpan::new(&Arc::from("t"), 0, 0),
        }
    }

    /// A span is positioned in its own file's text, and a program of
    /// several sources names the file; one source, or a span of a file
    /// not among them, names none.
    #[test]
    fn sources_position_a_span_in_its_file_and_name_it_when_several() {
        let a: Arc<str> = Arc::from("a.alc");
        let b: Arc<str> = Arc::from("b.alc");
        let one = Sources::one("a.alc", "x\ny");
        let f = one.fail_at(
            Fail::new(Code::InputInvalid, "m"),
            &SourceSpan::new(&a, 2, 3),
        );
        assert_eq!(
            (f.row, f.column, f.file.as_deref()),
            (Some(2), Some(1), None)
        );
        assert_eq!(f.to_string(), "INPUT_INVALID: m (2:1)");
        let two = Sources::several(vec![
            (a.clone(), Arc::from("x\ny")),
            (b.clone(), Arc::from("\n\nz")),
        ]);
        let f = two.fail_at(
            Fail::new(Code::InputInvalid, "m"),
            &SourceSpan::new(&b, 2, 3),
        );
        assert_eq!(
            (f.row, f.column, f.file.as_deref()),
            (Some(3), Some(1), Some("b.alc"))
        );
        assert_eq!(f.to_string(), "INPUT_INVALID: m (b.alc:3:1)");
        let f = two.fail_at(
            Fail::new(Code::InputInvalid, "m"),
            &SourceSpan::new(&a, 2, 3),
        );
        assert_eq!(
            (f.row, f.column, f.file.as_deref()),
            (Some(2), Some(1), Some("a.alc"))
        );
        let other: Arc<str> = Arc::from("stdlib/table.alc");
        let f = two.fail_at(
            Fail::new(Code::InputInvalid, "m"),
            &SourceSpan::new(&other, 2, 3),
        );
        assert_eq!((f.row, f.column, f.file), (Some(2), Some(1), None));
    }

    #[test]
    fn positions_are_one_based_rows_and_character_columns() {
        let src = "ab\ncdé f\n";
        let file = Arc::from("t");
        assert_eq!(SourceSpan::new(&file, 0, 1).position(src), (1, 1));
        assert_eq!(SourceSpan::new(&file, 3, 4).position(src), (2, 1));
        // `é` is two bytes and one column.
        assert_eq!(SourceSpan::new(&file, 8, 9).position(src), (2, 5));
        assert_eq!(SourceSpan::new(&file, 99, 99).position(src), (3, 1));
        // A lone carriage return restarts the column and not the row, as
        // the engine has it; `\r\n` is one line end.
        assert_eq!(SourceSpan::new(&file, 3, 4).position("a\r b"), (1, 2));
        assert_eq!(SourceSpan::new(&file, 3, 4).position("a\r\nb"), (2, 1));
    }

    #[test]
    fn same_shape_ignores_spans_and_nothing_else() {
        let a = parse("a (b 1) [\"x\"]").unwrap();
        let b = parse("a\n  b 1\n  [\"x\"]").unwrap();
        assert!(same_program(&a, &b));
        assert!(!same_program(&a, &parse("a (b 2) [\"x\"]").unwrap()));
        assert!(!sym("a").same_shape(&Expr::Keyword {
            name: "a".into(),
            span: sym("a").span().clone()
        }));
    }

    #[test]
    fn canonical_prints_every_atom_kind() {
        let program = parse("s :k \"a\\\"b\\n\" -1.5e3 true false null [] ()").unwrap();
        assert_eq!(
            canonical(&program),
            r#"(s :k "a\"b\n" -1.5e3 true false null [] ())"#
        );
    }

    #[test]
    fn layout_keeps_explicit_parens_only_where_layout_cannot_express_the_shape() {
        let program = parse("(concat prefix (newline) suffix)\n((f x) y)\n(a (b c) d)").unwrap();
        assert_eq!(
            format(&program),
            "concat prefix\n  (newline)\n  suffix\n\n((f x) y)\n\na\n  b c\n  d\n"
        );
    }

    /// The tagged tree for `x` nested in `depth` lists, as a layered
    /// grammar could hand over without the reader's own bound.
    fn nested_tree(depth: usize) -> Value {
        fn node(tag: &str, fields: Vec<(&str, Value)>) -> Value {
            let mut object = indexmap::IndexMap::new();
            object.insert("$".to_string(), Value::String(tag.into()));
            for (name, value) in fields {
                object.insert(name.to_string(), value);
            }
            object.insert(
                "span".to_string(),
                Value::array(vec![Value::Number(0.0), Value::Number(1.0)]),
            );
            Value::object(object)
        }
        let mut tree = node("sym", vec![("name", Value::String("x".into()))]);
        for _ in 0..depth {
            tree = node("list", vec![("items", Value::array(vec![tree]))]);
        }
        tree
    }

    #[test]
    fn a_tagged_tree_at_the_bound_converts_and_one_past_it_is_too_deep() {
        let file = Arc::from("t");
        let at_limit = Expr::from_value(&nested_tree(MAX_NESTING), &file).expect("converts");
        assert_eq!(
            canonical_form(&at_limit),
            format!("{}x{}", "(".repeat(MAX_NESTING), ")".repeat(MAX_NESTING))
        );
        let fail = Expr::from_value(&nested_tree(MAX_NESTING + 1), &file).expect_err("too deep");
        assert_eq!(fail.code, Code::DslParseError);
        assert!(fail.message.starts_with("too_deep: "), "{}", fail.message);
    }

    #[test]
    fn a_far_deeper_tagged_tree_is_refused_without_recursing_into_it() {
        // Converting recursively overflowed a test thread's stack from a
        // few hundred levels; the explicit stack stops at the bound
        // instead. (Deeper still and the engine's own `Value` drop, which
        // is recursive, would be what overflows: a tree this deep never
        // comes from the reader.)
        let file = Arc::from("t");
        let fail = Expr::from_value(&nested_tree(1_000), &file).expect_err("too deep");
        assert!(fail.message.starts_with("too_deep: "), "{}", fail.message);
    }

    #[test]
    fn malformed_reader_output_is_a_parse_error_not_a_panic() {
        let file = Arc::from("t");
        let bad = Value::String("nope".into());
        let fail = Expr::from_value(&bad, &file).expect_err("not a node");
        assert_eq!(fail.code, Code::DslParseError);
        let mut fields = indexmap::IndexMap::new();
        fields.insert("$".to_string(), Value::String("sym".into()));
        fields.insert("span".to_string(), Value::array(vec![Value::Number(0.0)]));
        assert!(Expr::from_value(&Value::object(fields), &file).is_err());
    }
}
