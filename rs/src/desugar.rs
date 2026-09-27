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
//! `bad_let`, `bad_if`, `bad_match`), the same convention the reader's own
//! failures follow, and they carry the row and column of the offending
//! form. The source is passed in for that: a span holds byte offsets, and
//! [`tabnas_transduce::Fail`] names positions.

use tabnas_transduce::{Code, Fail};

use crate::ast::{Expr, SourceSpan};

/// The messages, by code, as the grammar document also declares them; a
/// test in [`crate::grammar`] holds the two in step.
pub(crate) const MESSAGES: [(&str, &str); 5] = [
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
];

fn message(code: &str) -> &'static str {
    MESSAGES
        .iter()
        .find(|(known, _)| *known == code)
        .map_or("invalid form", |(_, message)| message)
}

fn fail(code: &str, span: &SourceSpan, src: &str) -> Fail {
    let (row, col) = span.position(src);
    Fail::new(Code::DslParseError, format!("{code}: {}", message(code))).at(row as u64, col as u64)
}

/// Desugar a whole program, form by form. `src` is the program's source,
/// read only to give a failure its row and column.
pub fn program(forms: Vec<Expr>, src: &str) -> Result<Vec<Expr>, Fail> {
    forms.into_iter().map(|form| expr(form, src)).collect()
}

/// Desugar one form, innermost first, so a rewrite sees core items.
pub fn expr(form: Expr, src: &str) -> Result<Expr, Fail> {
    match form {
        Expr::List { items, span } => {
            let items = items
                .into_iter()
                .map(|item| expr(item, src))
                .collect::<Result<Vec<_>, _>>()?;
            rewrite(items, span, src)
        }
        Expr::Vector { items, span } => Ok(Expr::Vector {
            items: items
                .into_iter()
                .map(|item| expr(item, src))
                .collect::<Result<Vec<_>, _>>()?,
            span,
        }),
        atom => Ok(atom),
    }
}

fn rewrite(items: Vec<Expr>, span: SourceSpan, src: &str) -> Result<Expr, Fail> {
    match items.first().and_then(Expr::symbol) {
        Some("def") => def(items, span, src),
        Some("pipe") => pipe(items, span, src),
        Some("let") => shape(items, span, src, "bad_let", is_let),
        Some("if") => shape(items, span, src, "bad_if", |items| items.len() == 4),
        Some("match") => shape(items, span, src, "bad_match", is_match),
        _ => Ok(Expr::List { items, span }),
    }
}

/// `(def name value)` stays; `(def name [params] body)` wraps the body in
/// a `fn` carrying the `def`'s span.
fn def(mut items: Vec<Expr>, span: SourceSpan, src: &str) -> Result<Expr, Fail> {
    let named = items.get(1).is_some_and(|name| name.symbol().is_some());
    match items.len() {
        3 if named => Ok(Expr::List { items, span }),
        4 if named && matches!(items[2], Expr::Vector { .. }) => {
            let body = items.pop().expect("four items were counted");
            let params = items.pop().expect("four items were counted");
            let function = Expr::List {
                items: vec![
                    Expr::Symbol {
                        name: "fn".into(),
                        span: span.clone(),
                    },
                    params,
                    body,
                ],
                span: span.clone(),
            };
            items.push(function);
            Ok(Expr::List { items, span })
        }
        _ => Err(fail("bad_def", &span, src)),
    }
}

/// `(pipe init step ...)` threaded data-last; `(pipe)` passes through.
fn pipe(items: Vec<Expr>, span: SourceSpan, src: &str) -> Result<Expr, Fail> {
    let mut steps = items.into_iter();
    let head = steps.next().expect("a rewrite has a head");
    let Some(mut acc) = steps.next() else {
        return Ok(Expr::List {
            items: vec![head],
            span,
        });
    };
    for step in steps {
        acc = match step {
            Expr::Symbol { name, span } => Expr::List {
                items: vec![
                    Expr::Symbol {
                        name,
                        span: span.clone(),
                    },
                    acc,
                ],
                span,
            },
            Expr::List { mut items, span } if !items.is_empty() => {
                items.push(acc);
                Expr::List { items, span }
            }
            other => return Err(fail("empty_step", other.span(), src)),
        };
    }
    Ok(acc)
}

fn shape(
    items: Vec<Expr>,
    span: SourceSpan,
    src: &str,
    code: &str,
    ok: impl Fn(&[Expr]) -> bool,
) -> Result<Expr, Fail> {
    if ok(&items) {
        Ok(Expr::List { items, span })
    } else {
        Err(fail(code, &span, src))
    }
}

/// `(let [name value] body)`.
fn is_let(items: &[Expr]) -> bool {
    items.len() == 3
        && matches!(
            &items[1],
            Expr::Vector { items: binding, .. }
                if binding.len() == 2 && binding[0].symbol().is_some()
        )
}

/// `(match value (case pattern body) ...)`.
fn is_match(items: &[Expr]) -> bool {
    items.len() >= 2
        && items[2..].iter().all(|clause| {
            matches!(
                clause,
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
    fn other_forms_pass_through_unchanged() {
        assert_eq!(
            core("fn [x] (get :label x)\n(case 1 2)\ndef x (pipe y f)"),
            "(fn [x] (get :label x))\n(case 1 2)\n(def x (f y))"
        );
    }
}
