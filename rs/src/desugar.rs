//! Desugaring: the conveniences rewritten into the core forms (design
//! brief section 4.2; spec sections 9.3, 9.4 and Appendix B).
//!
//! - `(def name [params] body)` becomes `(def name (fn [params] body))`;
//!   `(def name value)` is already core.
//! - `(pipe init step ...)` becomes nested data-last application: a bare
//!   symbol step `f` is `(f acc)`, a non-empty list step `(f a b)` is
//!   `(f a b acc)`, and an empty list, a literal, a keyword or a vector is
//!   an error, because there is nothing to apply.
//! - `let`, `if` and `match` are checked for shape and left as they are:
//!   one binding and one body, exactly two branches, and `(case pattern
//!   body)` clauses. Patterns stay expressions; the checker reads them.
//!
//! Anything else passes through unchanged, `(pipe)` included: with no
//! initial value there is nothing to thread, and the checker reports the
//! unknown operator it then is.
//!
//! Spans survive: a generated node carries the span of the form it came
//! from, so a diagnostic about a `fn` the user never wrote points at the
//! `def` they did. Failures are `DSL_PARSE_ERROR`s whose message begins
//! with the code the grammar document declares (`empty_step`, `bad_def`,
//! `bad_let`, `bad_if`, `bad_match`, `too_deep`), the same convention the
//! reader's own failures follow, and they carry the row and column of the
//! offending form. The source is passed in for that: a span holds byte
//! offsets, and [`crate::shared::Fail`] names positions.
//!
//! A rewrite can nest deeper than what it read: `def` wraps a body in a
//! `fn`, and `pipe` nests the threaded value one level per step, so a flat
//! line of a hundred thousand steps would become a tree that deep. The
//! depth of every result is therefore carried alongside it and checked
//! against [`MAX_NESTING`] as each container is built, so the bound the
//! reader holds still holds after desugaring, and a `pipe` fails at the
//! step that passes it rather than after building the whole chain.

use crate::ast::{Expr, SourceSpan, MAX_NESTING, TOO_DEEP};
use crate::shared::{Code, Fail};

/// The messages, by code, as the grammar document also declares them; a
/// test in [`crate::grammar`] holds the two in step.
pub(crate) const MESSAGES: [(&str, &str); 6] = [
    (
        "empty_step",
        "a pipe step must be a symbol or a non-empty list",
    ),
    (
        "bad_def",
        "def takes a name and a value, or a name, [params] and a body",
    ),
    ("bad_let", "let takes one binding [name value] and one body"),
    ("bad_if", "if takes a condition and exactly two branches"),
    (
        "bad_match",
        "match takes a value and (case pattern body) clauses",
    ),
    ("too_deep", TOO_DEEP),
];

fn message(code: &str) -> &'static str {
    MESSAGES
        .iter()
        .find(|(known, _)| *known == code)
        .map_or("invalid form", |(_, message)| message)
}

/// The failure of a form whose shape the desugarer owns (`bad_def`,
/// `bad_let`, `bad_if`, `bad_match`), not yet positioned: a
/// `DSL_PARSE_ERROR` with the desugarer's text, wherever the form is
/// found. The resolver meets one a `pipe` builds, since a step's form
/// grows by the threaded value after this pass read it; the checker and
/// the evaluator check the shapes again before reading a form.
pub(crate) fn shape_error(code: &str) -> Fail {
    Fail::new(Code::DslParseError, format!("{code}: {}", message(code)))
}

fn fail(code: &str, span: &SourceSpan, src: &str) -> Fail {
    let (row, col) = span.position(src);
    shape_error(code).at(row as u64, col as u64)
}

/// Desugar a whole program, form by form. `src` is the program's source,
/// read only to give a failure its row and column.
pub fn program(forms: Vec<Expr>, src: &str) -> Result<Vec<Expr>, Fail> {
    forms.into_iter().map(|form| expr(form, src)).collect()
}

/// Desugar one form, innermost first, so a rewrite sees core items.
pub fn expr(form: Expr, src: &str) -> Result<Expr, Fail> {
    deep(form, src).map(|form| form.expr)
}

/// A desugared form with its nesting: 0 for an atom, one more than its
/// deepest item for a list or vector. Carried so a rewrite that nests
/// deeper than what it read can refuse at the moment it would pass
/// [`MAX_NESTING`], before a tree the printers and `Drop` would recurse
/// over exists.
struct Deep {
    expr: Expr,
    depth: usize,
}

fn deep(form: Expr, src: &str) -> Result<Deep, Fail> {
    match form {
        Expr::List { items, span } => rewrite(each(items, src)?, span, src),
        Expr::Vector { items, span } => container(each(items, src)?, span, src, |items, span| {
            Expr::Vector { items, span }
        }),
        atom => Ok(Deep {
            expr: atom,
            depth: 0,
        }),
    }
}

fn each(items: Vec<Expr>, src: &str) -> Result<Vec<Deep>, Fail> {
    items.into_iter().map(|item| deep(item, src)).collect()
}

/// `deep` when it is within the bound, `too_deep` at `span` otherwise.
fn bounded(deep: Deep, span: &SourceSpan, src: &str) -> Result<Deep, Fail> {
    if deep.depth > MAX_NESTING {
        Err(fail("too_deep", span, src))
    } else {
        Ok(deep)
    }
}

/// A list or vector over `items`, one level deeper than its deepest item.
fn container(
    items: Vec<Deep>,
    span: SourceSpan,
    src: &str,
    make: impl FnOnce(Vec<Expr>, SourceSpan) -> Expr,
) -> Result<Deep, Fail> {
    let depth = 1 + items.iter().map(|item| item.depth).max().unwrap_or(0);
    let items = items.into_iter().map(|item| item.expr).collect();
    bounded(
        Deep {
            expr: make(items, span.clone()),
            depth,
        },
        &span,
        src,
    )
}

fn list(items: Vec<Deep>, span: SourceSpan, src: &str) -> Result<Deep, Fail> {
    container(items, span, src, |items, span| Expr::List { items, span })
}

fn rewrite(items: Vec<Deep>, span: SourceSpan, src: &str) -> Result<Deep, Fail> {
    match items.first().and_then(|head| head.expr.symbol()) {
        Some("def") => def(items, span, src),
        Some("pipe") => pipe(items, span, src),
        Some("let") => shape(items, span, src, "bad_let", is_let),
        Some("if") => shape(items, span, src, "bad_if", |items| items.len() == 4),
        Some("match") => shape(items, span, src, "bad_match", is_match),
        _ => list(items, span, src),
    }
}

/// `(def name value)` stays; `(def name [params] body)` wraps the body in
/// a `fn` carrying the `def`'s span.
fn def(mut items: Vec<Deep>, span: SourceSpan, src: &str) -> Result<Deep, Fail> {
    let named = items
        .get(1)
        .is_some_and(|name| name.expr.symbol().is_some());
    match items.len() {
        3 if named => list(items, span, src),
        4 if named && matches!(items[2].expr, Expr::Vector { .. }) => {
            let body = items.pop().expect("four items were counted");
            let params = items.pop().expect("four items were counted");
            let function = Deep {
                depth: 1 + params.depth.max(body.depth),
                expr: Expr::List {
                    items: vec![
                        Expr::Symbol {
                            name: "fn".into(),
                            span: span.clone(),
                        },
                        params.expr,
                        body.expr,
                    ],
                    span: span.clone(),
                },
            };
            items.push(function);
            list(items, span, src)
        }
        _ => Err(fail("bad_def", &span, src)),
    }
}

/// `(pipe init step ...)` threaded data-last; `(pipe)` passes through.
fn pipe(items: Vec<Deep>, span: SourceSpan, src: &str) -> Result<Deep, Fail> {
    let mut steps = items.into_iter();
    let head = steps.next().expect("a rewrite has a head");
    let Some(mut acc) = steps.next() else {
        return list(vec![head], span, src);
    };
    for step in steps {
        acc = match step.expr {
            Expr::Symbol { name, span: at } => Deep {
                depth: acc.depth + 1,
                expr: Expr::List {
                    items: vec![
                        Expr::Symbol {
                            name,
                            span: at.clone(),
                        },
                        acc.expr,
                    ],
                    span: at,
                },
            },
            Expr::List {
                mut items,
                span: at,
            } if !items.is_empty() => {
                // The step's own items are one level below it; the
                // threaded value joins them.
                let depth = step.depth.max(acc.depth + 1);
                items.push(acc.expr);
                Deep {
                    depth,
                    expr: Expr::List { items, span: at },
                }
            }
            other => return Err(fail("empty_step", other.span(), src)),
        };
        // Checked per step: a long pipe fails at the step that passes the
        // bound, not after the whole chain is built.
        acc = bounded(acc, &span, src)?;
    }
    Ok(acc)
}

fn shape(
    items: Vec<Deep>,
    span: SourceSpan,
    src: &str,
    code: &str,
    ok: impl Fn(&[Deep]) -> bool,
) -> Result<Deep, Fail> {
    if ok(&items) {
        list(items, span, src)
    } else {
        Err(fail(code, &span, src))
    }
}

/// `(let [name value] body)`.
fn is_let(items: &[Deep]) -> bool {
    items.len() == 3
        && matches!(
            &items[1].expr,
            Expr::Vector { items: binding, .. }
                if binding.len() == 2 && binding[0].symbol().is_some()
        )
}

/// `(match value (case pattern body) ...)`.
fn is_match(items: &[Deep]) -> bool {
    items.len() >= 2
        && items[2..].iter().all(|clause| {
            matches!(
                &clause.expr,
                Expr::List { items: parts, .. }
                    if parts.len() == 3 && parts[0].symbol() == Some("case")
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{canonical, parse};

    fn core(src: &str) -> String {
        let forms = parse(src).unwrap_or_else(|fail| panic!("{src:?}: {fail}"));
        let core = program(forms, src).unwrap_or_else(|fail| panic!("{src:?}: {fail}"));
        canonical(&core)
    }

    fn failure(src: &str) -> Fail {
        let forms = parse(src).unwrap_or_else(|fail| panic!("{src:?}: {fail}"));
        program(forms, src).expect_err("the program should not desugar")
    }

    #[test]
    fn a_def_with_parameters_becomes_a_def_of_a_fn() {
        assert_eq!(
            core("def csv-field [value]\n  scalar-text value"),
            "(def csv-field (fn [value] (scalar-text value)))"
        );
        assert_eq!(core("def sep \",\""), "(def sep \",\")");
    }

    #[test]
    fn the_generated_fn_carries_the_def_span() {
        let src = "def f [x]\n  x";
        let core = program(parse(src).unwrap(), src).unwrap();
        let Expr::List { items, span } = &core[0] else {
            panic!("a def list");
        };
        let Expr::List {
            items: function,
            span: fn_span,
        } = &items[2]
        else {
            panic!("a fn list");
        };
        assert_eq!(fn_span, span);
        assert_eq!(function[0].span(), span);
        // The parameters and the body keep their own.
        assert_eq!((function[1].span().start, function[1].span().end), (6, 9));
        assert_eq!((function[2].span().start, function[2].span().end), (12, 13));
    }

    #[test]
    fn pipe_threads_the_value_data_last() {
        assert_eq!(
            core("pipe input (select (path \"payload\" \"records\" each-index)) (map normalize) (filter active?)"),
            "(filter active? (map normalize (select (path \"payload\" \"records\" each-index) input)))"
        );
        assert_eq!(core("pipe x f g"), "(g (f x))");
        assert_eq!(core("pipe x"), "x");
        assert_eq!(core("(pipe)"), "(pipe)");
    }

    #[test]
    fn a_pipe_inside_a_def_desugars_both() {
        assert_eq!(
            core("def export [input]\n  pipe input\n    table-from-json api-binding\n    csv csv-options"),
            "(def export (fn [input] (csv csv-options (table-from-json api-binding input))))"
        );
    }

    #[test]
    fn a_step_that_is_not_applicable_is_an_empty_step_error() {
        for (src, col) in [
            ("pipe x ()", 8),
            ("pipe x 1", 8),
            ("pipe x \"s\"", 8),
            ("pipe x [f]", 8),
            ("pipe x :k", 8),
            ("pipe x\n  f\n  []", 3),
            // A lone carriage return restarts the column, as the engine
            // counts it.
            ("pipe x\r ()", 2),
        ] {
            let fail = failure(src);
            assert_eq!(fail.code, Code::DslParseError, "{src:?}");
            assert!(
                fail.message.starts_with("empty_step: "),
                "{src:?}: {}",
                fail.message
            );
            assert_eq!(fail.column, Some(col), "{src:?}");
        }
        assert_eq!(failure("pipe x\n  f\n  []").row, Some(3));
    }

    #[test]
    fn the_core_form_shapes_are_checked() {
        assert_eq!(core("let [x 1] x"), "(let [x 1] x)");
        assert_eq!(core("if a b c"), "(if a b c)");
        assert_eq!(
            core("match v\n  case 1 \"one\"\n  case _ \"other\""),
            "(match v (case 1 \"one\") (case _ \"other\"))"
        );
        assert_eq!(core("match v"), "(match v)");
        for (src, code) in [
            ("let [x] x", "bad_let"),
            ("let [x 1 2] x", "bad_let"),
            ("let [1 x] x", "bad_let"),
            ("let [x 1]", "bad_let"),
            ("if a b", "bad_if"),
            ("if a b c d", "bad_if"),
            ("(match)", "bad_match"),
            ("match v (case 1)", "bad_match"),
            ("match v (when 1 2)", "bad_match"),
            ("match v 1", "bad_match"),
            ("(def)", "bad_def"),
            ("def x", "bad_def"),
            ("def 1 2", "bad_def"),
            ("def x y z", "bad_def"),
            ("def x [a] b c", "bad_def"),
        ] {
            let fail = failure(src);
            assert!(
                fail.message.starts_with(&format!("{code}: ")),
                "{src:?}: {}",
                fail.message
            );
            assert_eq!((fail.row, fail.column), (Some(1), Some(1)), "{src:?}");
        }
    }

    #[test]
    fn a_rewrite_that_nests_past_the_bound_is_too_deep_at_the_form() {
        // A pipe nests one level per step: 256 steps reach the bound.
        let at_limit = format!("pipe x{}", " f".repeat(MAX_NESTING));
        assert_eq!(
            core(&at_limit),
            format!("{}x{}", "(f ".repeat(MAX_NESTING), ")".repeat(MAX_NESTING))
        );
        let fail = failure(&format!("pipe x{}", " f".repeat(MAX_NESTING + 1)));
        assert!(fail.message.starts_with("too_deep: "), "{}", fail.message);
        assert_eq!((fail.row, fail.column), (Some(1), Some(1)));
        // A pipe of a hundred thousand steps reads flat and fails at the
        // 257th step, without building the chain a printer or `Drop` would
        // then recurse over.
        assert!(failure(&format!("pipe x{}", " f".repeat(100_000)))
            .message
            .starts_with("too_deep: "));

        // `def` adds the `fn` level: a body two below the bound still
        // fits under the `def`, a body one below it does not, though the
        // reader accepted both.
        let body = |depth: usize| format!("{}x{}", "(".repeat(depth), ")".repeat(depth));
        let fits = format!("def f [x] {}", body(MAX_NESTING - 2));
        assert!(core(&fits).starts_with("(def f (fn [x] "));
        let fail = failure(&format!("def f [x] {}", body(MAX_NESTING - 1)));
        assert!(fail.message.starts_with("too_deep: "), "{}", fail.message);
    }

    #[test]
    fn other_forms_pass_through_unchanged() {
        assert_eq!(
            core("fn [x] (get :label x)\n(case 1 2)\ndef x (pipe y f)"),
            "(fn [x] (get :label x))\n(case 1 2)\n(def x (f y))"
        );
    }
}
