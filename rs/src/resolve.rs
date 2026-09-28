//! Scopes and linking (design brief section 4.2).
//!
//! A program is a sequence of `def`s, in any order: every definition sees
//! every other, so there is no forward-reference problem, and a name that
//! is bound nowhere (not a local, not a definition, not a standard-library
//! name) is `DSL_TYPE_ERROR` with the finer code `unknown_name`, before
//! anything runs. Locals come from `fn` parameters, `let` bindings and the
//! bindings a `match` pattern makes; each shadows what is outside it.
//!
//! Recursion is refused here: a definition that reaches itself through the
//! references its body makes, directly or through other definitions, is
//! `STREAMABILITY_UNKNOWN` with the finer code `recursion` (spec section
//! 15.3: strict mode refuses unrestricted recursion). That is static, so
//! the interpreter can cache definition values without ever meeting a
//! cycle.
//!
//! Patterns: `_` matches anything; a symbol naming a standard-library
//! constant (`table-end`, `no-schema`, `missing`, the selector constants)
//! or a definition of the program compares equal to it; any other symbol
//! binds; a keyword, string, number, boolean or `null` compares; a vector
//! matches a vector of the same length, item by item; `(constructor p ...)`
//! matches the tagged value that constructor makes, field by field.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use indexmap::IndexMap;
use tabnas_transduce::{Code, Fail};

use crate::ast::{Expr, SourceSpan, Sources};

/// What kind of thing a name outside the program denotes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameKind {
    /// A value the symbol denotes without a call (`table-end`).
    Constant,
    /// A function whose value is a tagged value: also a pattern head.
    Constructor,
    /// A function.
    Function,
    /// A definition's value of some other kind (a record).
    Value,
}

/// The heads of the core forms and the conveniences the desugarer
/// rewrites: none may be defined, and each is read by shape.
pub const SPECIAL_FORMS: &[&str] = &["def", "fn", "let", "if", "match", "case", "pipe"];

/// One top-level definition.
#[derive(Clone, Debug)]
pub struct Def {
    pub name: Arc<str>,
    pub value: Arc<Expr>,
    /// The span of the whole `def` form.
    pub span: SourceSpan,
}

impl Def {
    /// The parameters, when the value is a `fn`.
    pub fn params(&self) -> Option<Vec<&str>> {
        fn_params(&self.value)
    }
}

/// The parameter names of a `(fn [params] body)` form.
pub fn fn_params(expr: &Expr) -> Option<Vec<&str>> {
    let Expr::List { items, .. } = expr else {
        return None;
    };
    fn_form(items).map(|(params, _)| params)
}

/// The parameters and the body of the items of a `(fn [params] body)`
/// list; `None` for any other shape.
pub fn fn_form(items: &[Expr]) -> Option<(Vec<&str>, &Expr)> {
    if items.len() != 3 || items[0].symbol() != Some("fn") {
        return None;
    }
    let Expr::Vector { items: params, .. } = &items[1] else {
        return None;
    };
    let params: Option<Vec<&str>> = params.iter().map(Expr::symbol).collect();
    params.map(|params| (params, &items[2]))
}

/// A resolved program: its definitions and the references between them.
#[derive(Clone, Debug)]
pub struct Resolved {
    /// The file the program is named by: its first source's.
    pub file: Arc<str>,
    pub defs: IndexMap<Arc<str>, Def>,
    /// For each definition, the definitions of this program its value
    /// refers to, in first-reference order.
    pub references: IndexMap<Arc<str>, Vec<Arc<str>>>,
}

impl Resolved {
    pub fn get(&self, name: &str) -> Option<&Def> {
        self.defs.get(name)
    }

    /// The definitions `name` reaches, transitively, in first-reference
    /// order; `name` itself is not among them (recursion was refused).
    pub fn reachable(&self, name: &str) -> Vec<Arc<str>> {
        let mut out = Vec::new();
        let mut seen: HashSet<Arc<str>> = HashSet::new();
        let mut pending: Vec<Arc<str>> = self
            .references
            .get(name)
            .map(|refs| refs.iter().rev().cloned().collect())
            .unwrap_or_default();
        while let Some(next) = pending.pop() {
            if !seen.insert(next.clone()) {
                continue;
            }
            if let Some(refs) = self.references.get(&next) {
                pending.extend(refs.iter().rev().cloned());
            }
            out.push(next);
        }
        out
    }
}

fn fail(
    code: Code,
    finer: &str,
    message: impl std::fmt::Display,
    span: &SourceSpan,
    sources: &Sources,
) -> Fail {
    sources.fail_at(Fail::new(code, format!("{finer}: {message}")), span)
}

fn type_fail(
    finer: &str,
    message: impl std::fmt::Display,
    span: &SourceSpan,
    sources: &Sources,
) -> Fail {
    fail(Code::DslTypeError, finer, message, span, sources)
}

/// Link a desugared program, whose forms may come from several sources:
/// `sources` are their texts, which position a failure in the file its
/// form was read from. `outer` answers the kind of a name bound outside
/// the program (the standard library and the natives), or `None`.
pub fn resolve(
    forms: Vec<Expr>,
    sources: &Sources,
    outer: &dyn Fn(&str) -> Option<NameKind>,
) -> Result<Resolved, Fail> {
    let mut defs: IndexMap<Arc<str>, Def> = IndexMap::new();
    for form in forms {
        let span = form.span().clone();
        let Expr::List { mut items, .. } = form else {
            return Err(type_fail(
                "not_def",
                "a top-level form must be a def",
                &span,
                sources,
            ));
        };
        if items.len() != 3 || items[0].symbol() != Some("def") {
            return Err(type_fail(
                "not_def",
                "a top-level form must be a def",
                &span,
                sources,
            ));
        }
        let value = items
            .pop()
            .unwrap_or_else(|| Expr::Null { span: span.clone() });
        let name = match &items[1] {
            Expr::Symbol { name, .. } => name.clone(),
            other => {
                return Err(type_fail(
                    "not_def",
                    "a def names a symbol",
                    other.span(),
                    sources,
                ))
            }
        };
        if SPECIAL_FORMS.contains(&name.as_str()) {
            return Err(type_fail(
                "reserved",
                format!("{name} is a special form and cannot be defined"),
                &span,
                sources,
            ));
        }
        let name: Arc<str> = Arc::from(name);
        if let Some(first) = defs.get(&name) {
            // Across several sources the first definition may be in
            // another file, so the message says where it is.
            let message = if sources.names_files() {
                let (row, col) = sources.position(&first.span);
                format!(
                    "{name} is defined twice; first at {}:{row}:{col}",
                    first.span.file
                )
            } else {
                format!("{name} is defined twice")
            };
            return Err(type_fail("duplicate_def", message, &span, sources));
        }
        defs.insert(
            name.clone(),
            Def {
                name,
                value: Arc::new(value),
                span,
            },
        );
    }

    let mut references: IndexMap<Arc<str>, Vec<Arc<str>>> = IndexMap::new();
    for def in defs.values() {
        let mut walker = Walker {
            sources,
            outer,
            defs: &defs,
            locals: Vec::new(),
            refs: Vec::new(),
            seen: BTreeSet::new(),
        };
        walker.expr(&def.value)?;
        references.insert(def.name.clone(), walker.refs);
    }

    let resolved = Resolved {
        file: sources.first().clone(),
        defs,
        references,
    };
    resolved.refuse_recursion(sources)?;
    Ok(resolved)
}

impl Resolved {
    /// Every definition, each after all the definitions it refers to: the
    /// order in which typing or evaluating them one by one never has to
    /// reach through a definition not yet done, so no walk nests deeper
    /// than one definition's own forms however long a chain of
    /// definitions naming definitions is. An iterative depth-first
    /// search; recursion was refused, so the graph has no cycle.
    pub fn dependency_order(&self) -> Vec<Arc<str>> {
        let mut order = Vec::with_capacity(self.defs.len());
        let mut done: HashSet<&str> = HashSet::with_capacity(self.defs.len());
        let mut open: HashSet<&str> = HashSet::new();
        for start in self.defs.keys() {
            if done.contains(&**start) {
                continue;
            }
            let mut path: Vec<(&Arc<str>, usize)> = vec![(start, 0)];
            open.insert(start);
            while let Some((name, next)) = path.last_mut() {
                let refs = self.references.get(*name).map(Vec::as_slice).unwrap_or(&[]);
                match refs.get(*next) {
                    Some(child) => {
                        *next += 1;
                        if !done.contains(&**child) && open.insert(child) {
                            path.push((child, 0));
                        }
                    }
                    None => {
                        let name: &Arc<str> = name;
                        open.remove(&**name);
                        done.insert(name);
                        order.push(name.clone());
                        path.pop();
                    }
                }
            }
        }
        order
    }

    /// `recursion` at the first definition on a cycle, naming the cycle.
    fn refuse_recursion(&self, sources: &Sources) -> Result<(), Fail> {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            Open,
            Done,
        }
        let mut marks: IndexMap<&str, Mark> = IndexMap::new();
        for start in self.defs.keys() {
            if marks.contains_key(&**start) {
                continue;
            }
            // Iterative depth-first search over the reference graph, with
            // the path kept so the cycle can be named.
            let mut path: Vec<(&str, usize)> = vec![(start, 0)];
            marks.insert(start, Mark::Open);
            while let Some((name, next)) = path.last_mut() {
                let refs = self.references.get(*name).map(Vec::as_slice).unwrap_or(&[]);
                if *next >= refs.len() {
                    marks.insert(name, Mark::Done);
                    path.pop();
                    continue;
                }
                let child: &str = &refs[*next];
                *next += 1;
                match marks.get(child) {
                    Some(Mark::Done) => {}
                    Some(Mark::Open) => {
                        let from = path.iter().position(|(n, _)| *n == child).unwrap_or(0);
                        let cycle: Vec<&str> = path[from..].iter().map(|(n, _)| *n).collect();
                        let def = &self.defs[child];
                        let through = if cycle.len() == 1 {
                            "itself".to_string()
                        } else {
                            cycle[1..].join(", then ")
                        };
                        return Err(fail(
                            Code::StreamabilityUnknown,
                            "recursion",
                            format!(
                                "{child} reaches itself through {through}; strict mode refuses recursion"
                            ),
                            &def.span,
                            sources,
                        ));
                    }
                    None => {
                        marks.insert(child, Mark::Open);
                        path.push((child, 0));
                    }
                }
            }
        }
        Ok(())
    }
}

struct Walker<'a> {
    sources: &'a Sources,
    outer: &'a dyn Fn(&str) -> Option<NameKind>,
    defs: &'a IndexMap<Arc<str>, Def>,
    locals: Vec<Arc<str>>,
    refs: Vec<Arc<str>>,
    seen: BTreeSet<Arc<str>>,
}

impl Walker<'_> {
    fn is_local(&self, name: &str) -> bool {
        self.locals.iter().any(|l| &**l == name)
    }

    fn refer(&mut self, name: &Arc<str>) {
        if self.seen.insert(name.clone()) {
            self.refs.push(name.clone());
        }
    }

    fn symbol(&mut self, name: &str, span: &SourceSpan) -> Result<(), Fail> {
        if self.is_local(name) {
            return Ok(());
        }
        if let Some((key, _)) = self.defs.get_key_value(name) {
            let key = key.clone();
            self.refer(&key);
            return Ok(());
        }
        if (self.outer)(name).is_some() {
            return Ok(());
        }
        Err(type_fail(
            "unknown_name",
            format!("{name} is not defined"),
            span,
            self.sources,
        ))
    }

    fn expr(&mut self, expr: &Expr) -> Result<(), Fail> {
        match expr {
            Expr::Symbol { name, span } => self.symbol(name, span),
            Expr::Keyword { .. }
            | Expr::Str { .. }
            | Expr::Num { .. }
            | Expr::Bool { .. }
            | Expr::Null { .. } => Ok(()),
            Expr::Vector { items, .. } => items.iter().try_for_each(|item| self.expr(item)),
            Expr::List { items, span } => self.list(items, span),
        }
    }

    fn list(&mut self, items: &[Expr], span: &SourceSpan) -> Result<(), Fail> {
        match items.first().and_then(Expr::symbol) {
            Some("fn") => {
                let Some((params, body)) = fn_form(items) else {
                    return Err(type_fail(
                        "bad_fn",
                        "fn takes [params] of symbols and one body",
                        span,
                        self.sources,
                    ));
                };
                let depth = self.locals.len();
                self.locals.extend(params.iter().map(|p| Arc::from(*p)));
                let r = self.expr(body);
                self.locals.truncate(depth);
                r
            }
            Some("let") => {
                // The desugarer checked the shape: `(let [name value] body)`.
                let Some(Expr::Vector { items: binding, .. }) = items.get(1) else {
                    return Err(type_fail(
                        "bad_let",
                        "let takes one binding [name value] and one body",
                        span,
                        self.sources,
                    ));
                };
                let (Some(name), Some(value), Some(body)) = (
                    binding.first().and_then(Expr::symbol),
                    binding.get(1),
                    items.get(2),
                ) else {
                    return Err(type_fail(
                        "bad_let",
                        "let takes one binding [name value] and one body",
                        span,
                        self.sources,
                    ));
                };
                self.expr(value)?;
                self.locals.push(Arc::from(name));
                let r = self.expr(body);
                self.locals.pop();
                r
            }
            Some("if") => items[1..].iter().try_for_each(|item| self.expr(item)),
            Some("match") => {
                let Some(value) = items.get(1) else {
                    return Err(type_fail(
                        "bad_match",
                        "match takes a value and (case pattern body) clauses",
                        span,
                        self.sources,
                    ));
                };
                self.expr(value)?;
                for clause in &items[2..] {
                    let Expr::List {
                        items: parts,
                        span: at,
                    } = clause
                    else {
                        return Err(type_fail(
                            "bad_match",
                            "match takes (case pattern body) clauses",
                            clause.span(),
                            self.sources,
                        ));
                    };
                    if parts.len() != 3 || parts[0].symbol() != Some("case") {
                        return Err(type_fail(
                            "bad_match",
                            "match takes (case pattern body) clauses",
                            at,
                            self.sources,
                        ));
                    }
                    let depth = self.locals.len();
                    let mut bound = Vec::new();
                    self.pattern(&parts[1], &mut bound)?;
                    self.locals.extend(bound);
                    let r = self.expr(&parts[2]);
                    self.locals.truncate(depth);
                    r?;
                }
                Ok(())
            }
            Some("def") => Err(type_fail(
                "misplaced_def",
                "def is only allowed at the top level",
                span,
                self.sources,
            )),
            _ => items.iter().try_for_each(|item| self.expr(item)),
        }
    }

    /// Whether a symbol in a pattern compares rather than binds.
    fn is_constant_pattern(&mut self, name: &str) -> bool {
        if self.is_local(name) {
            return false;
        }
        if let Some((key, _)) = self.defs.get_key_value(name) {
            let key = key.clone();
            self.refer(&key);
            return true;
        }
        (self.outer)(name) == Some(NameKind::Constant)
    }

    fn pattern(&mut self, pattern: &Expr, bound: &mut Vec<Arc<str>>) -> Result<(), Fail> {
        match pattern {
            Expr::Symbol { name, .. } if name == "_" => Ok(()),
            Expr::Symbol { name, .. } => {
                if !self.is_constant_pattern(name) {
                    bound.push(Arc::from(name.as_str()));
                }
                Ok(())
            }
            Expr::Keyword { .. }
            | Expr::Str { .. }
            | Expr::Num { .. }
            | Expr::Bool { .. }
            | Expr::Null { .. } => Ok(()),
            Expr::Vector { items, .. } => {
                items.iter().try_for_each(|item| self.pattern(item, bound))
            }
            Expr::List { items, span } => {
                let head = items.first().and_then(Expr::symbol);
                match head {
                    Some(name) if (self.outer)(name) == Some(NameKind::Constructor) => items[1..]
                        .iter()
                        .try_for_each(|item| self.pattern(item, bound)),
                    Some(name) => Err(type_fail(
                        "bad_pattern",
                        format!("{name} is not a constructor; a list pattern is (constructor pattern...)"),
                        span,
                        self.sources,
                    )),
                    None => Err(type_fail(
                        "bad_pattern",
                        "a list pattern is (constructor pattern...)",
                        span,
                        self.sources,
                    )),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{desugar, parse};

    fn outer(name: &str) -> Option<NameKind> {
        match name {
            "table-end" | "no-schema" | "missing" => Some(NameKind::Constant),
            "selected" | "schema" | "row" => Some(NameKind::Constructor),
            "map" | "get" | "csv" | "table-from-json" => Some(NameKind::Function),
            "csv-options" => Some(NameKind::Value),
            _ => None,
        }
    }

    fn resolved(src: &str) -> Result<Resolved, Fail> {
        let forms = desugar::program(parse(src).unwrap(), src).unwrap();
        resolve(forms, &Sources::one("t", src), &outer)
    }

    fn code(src: &str) -> (Code, String, Option<u64>, Option<u64>) {
        let f = resolved(src).expect_err("should fail");
        let finer = f
            .message
            .split_once(": ")
            .map(|(c, _)| c.to_string())
            .unwrap();
        (f.code, finer, f.row, f.column)
    }

    #[test]
    fn definitions_see_each_other_in_any_order() {
        let r = resolved("def export [input]\n  csv csv-options (api-table input)\ndef api-table [input]\n  table-from-json binding input\ndef binding 1").unwrap();
        assert_eq!(r.defs.len(), 3);
        assert_eq!(r.references["export"], vec![Arc::<str>::from("api-table")]);
        assert_eq!(r.references["api-table"], vec![Arc::<str>::from("binding")]);
        assert_eq!(
            r.reachable("export"),
            vec![Arc::<str>::from("api-table"), Arc::<str>::from("binding")]
        );
        assert_eq!(r.get("binding").unwrap().params(), None);
        assert_eq!(r.get("export").unwrap().params(), Some(vec!["input"]));
    }

    #[test]
    fn locals_shadow_and_patterns_bind() {
        resolved(
            "def f [x]\n  let [y (map x x)]\n    match y\n      case (selected :columns raw) raw\n      case [a b] (get a b)\n      case table-end y\n      case _ x",
        )
        .unwrap();
        // `raw` is only bound inside its clause.
        let (c, finer, row, col) =
            code("def f [x]\n  match x\n    case (selected :a raw) raw\n    case _ raw");
        assert_eq!(c, Code::DslTypeError);
        assert_eq!(finer, "unknown_name");
        assert_eq!((row, col), (Some(4), Some(12)));
    }

    #[test]
    fn the_finer_codes() {
        assert_eq!(code("def f [x] (nope x)").1, "unknown_name");
        assert_eq!(code("csv 1").1, "not_def");
        assert_eq!(code("def x 1\ndef x 2").1, "duplicate_def");
        assert_eq!(code("def if 1").1, "reserved");
        assert_eq!(code("def f (fn x x)").1, "bad_fn");
        assert_eq!(code("def f (fn [1] x)").1, "bad_fn");
        assert_eq!(code("def f [x] (def y x)").1, "misplaced_def");
        assert_eq!(
            code("def f [x] (match x (case (map a) a))").1,
            "bad_pattern"
        );
        assert_eq!(code("def f [x] (match x (case (1 a) a))").1, "bad_pattern");
        let (c, finer, row, _) = code("def a [x] (b x)\ndef b [x] (c x)\ndef c [x] (a x)");
        assert_eq!(c, Code::StreamabilityUnknown);
        assert_eq!(finer, "recursion");
        assert_eq!(row, Some(1));
        assert_eq!(code("def a [x] (a x)").1, "recursion");
        assert_eq!(code("def a (fn [x] (b x))\ndef b [y] (a y)").1, "recursion");
    }

    #[test]
    fn a_program_may_shadow_a_library_name() {
        let r = resolved("def csv [x] x\ndef export [input] (csv input)").unwrap();
        assert_eq!(r.references["export"], vec![Arc::<str>::from("csv")]);
    }
}
