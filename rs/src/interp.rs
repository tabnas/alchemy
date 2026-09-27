//! The evaluator (design brief section 4.4; spec sections 10.1 and 10.3).
//!
//! Values are evaluated eagerly; streams and texts are [`Plan`]s that
//! evaluation builds and never runs. The [`Runtime`] holds the program's
//! and the standard library's definitions, evaluates a definition once on
//! first use (recursion was refused by the resolver, so there is no cycle
//! to meet), and applies functions: a closure binds its parameters in the
//! environment it closed over and evaluates its body in its own scope, a
//! native runs its implementation, a partial supplies its arguments first.
//! [`Runtime::export`] applies the program's `export` to the host's input
//! plan; what comes back is the plan [`crate::lower`] turns into sinks.
//!
//! Two scopes exist: a program's definitions see the program first, then
//! the library, then the natives; the library's see the library and the
//! natives. Lexical, so a program that defines its own `csv` changes what
//! its own code means and nothing the library does.
//!
//! The two standard compositions run natively when their arguments have
//! the standard shapes (`table-from-json BINDING input` and `csv OPTIONS
//! events`, from the library's own definitions, not a program's shadowing
//! ones); [`Runtime::with_native`] turns that off so the differential test
//! can run the interpreted text.
//!
//! Failures: a wrong argument count is `DSL_TYPE_ERROR` (`arity`); a value
//! of the wrong kind where the program chose it is `DSL_TYPE_ERROR`
//! (`type_mismatch`); a `fail "message"` is `INPUT_INVALID` at the form's
//! position. The checker reports what it can before anything runs; these
//! are the runtime's own answers for what it could not see.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tabnas_transduce::{Code, Datum, Duplicates, Fail};

use crate::ast::{Expr, SourceSpan};
use crate::resolve::{fn_form, fn_params, Resolved};
use crate::stdlib::registry::{self, native, truth, Kind, Native};
use crate::stdlib::{self, Stdlib};
use crate::value::{type_error, Closure, Env, Func, Partial, Plan, Scope, Val};

/// The evaluator for one program.
pub struct Runtime {
    program: Arc<Resolved>,
    src: Arc<str>,
    stdlib: &'static Stdlib,
    native: bool,
    duplicates: Duplicates,
    /// Definition values, evaluated on first use.
    cache: Mutex<HashMap<(Scope, Arc<str>), Val>>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("file", &self.program.file)
            .field("native", &self.native)
            .finish_non_exhaustive()
    }
}

/// A `DSL_TYPE_ERROR` with the `arity` finer code.
pub fn arity_error(what: &str, wanted: impl std::fmt::Display, got: usize) -> Fail {
    Fail::new(
        Code::DslTypeError,
        format!("arity: {what} takes {wanted} argument(s), got {got}"),
    )
}

impl Runtime {
    /// A runtime over a resolved program and its source text (read only
    /// for positions in diagnostics), with the fast paths on and duplicate
    /// members rejected.
    pub fn new(program: Arc<Resolved>, src: &str) -> Runtime {
        Runtime {
            program,
            src: Arc::from(src),
            stdlib: stdlib::stdlib(),
            native: true,
            duplicates: Duplicates::Reject,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Run the standard compositions natively (the default) or through
    /// the library's own definitions.
    pub fn with_native(mut self, native: bool) -> Runtime {
        self.native = native;
        self
    }

    /// How a materialized capture treats a repeated member name; the
    /// default rejects it (spec section 18.2, the strict mapping profile).
    pub fn with_duplicates(mut self, duplicates: Duplicates) -> Runtime {
        self.duplicates = duplicates;
        self
    }

    pub fn native(&self) -> bool {
        self.native
    }

    pub fn duplicates(&self) -> Duplicates {
        self.duplicates
    }

    pub fn program(&self) -> &Arc<Resolved> {
        &self.program
    }

    /// The 1-based row and column of a span, in the file it names: the
    /// program's, or an embedded standard-library file's.
    pub fn position(&self, span: &SourceSpan) -> Option<(u64, u64)> {
        let src: &str = if *span.file == *self.program.file {
            &self.src
        } else {
            stdlib::source(&span.file)?
        };
        let (row, col) = span.position(src);
        Some((row as u64, col as u64))
    }

    /// `fail` with the position of `at`, when it has one.
    pub fn fail_at(&self, mut fail: Fail, at: &SourceSpan) -> Fail {
        if fail.row.is_none() {
            if let Some((row, col)) = self.position(at) {
                fail = fail.at(row, col);
            }
        }
        fail
    }

    /// The definition `name` denotes in `scope`, with the scope it was
    /// found in.
    fn find_def(&self, scope: Scope, name: &str) -> Option<(Scope, &crate::resolve::Def)> {
        if scope == Scope::Program {
            if let Some(def) = self.program.get(name) {
                return Some((Scope::Program, def));
            }
        }
        self.stdlib.get(name).map(|def| (Scope::Stdlib, def))
    }

    /// The value of a definition, evaluated once.
    pub fn def_value(&self, scope: Scope, name: &str) -> Result<Option<Val>, Fail> {
        let Some((found, def)) = self.find_def(scope, name) else {
            return Ok(None);
        };
        let key = (found, def.name.clone());
        if let Some(v) = self.cache.lock().map_err(poisoned)?.get(&key) {
            return Ok(Some(v.clone()));
        }
        let value = match fn_params(&def.value) {
            Some(params) => {
                let Expr::List { items, span } = &*def.value else {
                    unreachable!("fn_params answered for a fn form");
                };
                Val::Fn(Func::Closure(Arc::new(Closure {
                    name: Some(def.name.clone()),
                    params: params.iter().map(|p| Arc::from(*p)).collect(),
                    body: Arc::new(items[2].clone()),
                    env: Env::new(),
                    scope: found,
                    span: span.clone(),
                })))
            }
            None => self.eval(&def.value, &Env::new(), found)?,
        };
        self.cache
            .lock()
            .map_err(poisoned)?
            .insert(key, value.clone());
        Ok(Some(value))
    }

    /// The value a free symbol denotes in `scope`: a definition, a native
    /// constant's value, or a native function.
    fn global(&self, scope: Scope, name: &str, span: &SourceSpan) -> Result<Val, Fail> {
        if let Some(v) = self.def_value(scope, name)? {
            return Ok(v);
        }
        if let Some(n) = native(name) {
            return match n.kind {
                Kind::Constant => (n.call)(self, &[], span),
                Kind::Function | Kind::Constructor => Ok(Val::Fn(Func::Native(n))),
            };
        }
        Err(self.fail_at(
            Fail::new(
                Code::DslTypeError,
                format!("unknown_name: {name} is not defined"),
            ),
            span,
        ))
    }

    /// Evaluate one form.
    pub fn eval(&self, expr: &Expr, env: &Env, scope: Scope) -> Result<Val, Fail> {
        match expr {
            Expr::Symbol { name, span } => match env.get(name) {
                Some(v) => Ok(v.clone()),
                None => self.global(scope, name, span),
            },
            Expr::Keyword { name, .. } => Ok(Val::keyword(name)),
            Expr::Str { value, .. } => Ok(Val::str(value)),
            Expr::Num { lexeme, span } => {
                let value: f64 = lexeme.parse().map_err(|_| {
                    self.fail_at(type_error(format!("{lexeme} is not a number")), span)
                })?;
                Ok(Val::Num {
                    value,
                    lexeme: Some(Arc::from(lexeme.as_str())),
                })
            }
            Expr::Bool { value, .. } => Ok(Val::Bool(*value)),
            Expr::Null { .. } => Ok(Val::Null),
            Expr::Vector { items, span } => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    let v = self.eval(item, env, scope)?;
                    if v.is_live() {
                        return Err(self.fail_at(
                            type_error(format!(
                                "a vector cannot hold {}; a stream is used once, where it is",
                                v.kind()
                            )),
                            span,
                        ));
                    }
                    out.push(v);
                }
                Ok(Val::vector(out))
            }
            Expr::List { items, span } => self.list(items, span, env, scope),
        }
    }

    fn list(
        &self,
        items: &[Expr],
        span: &SourceSpan,
        env: &Env,
        scope: Scope,
    ) -> Result<Val, Fail> {
        let Some(head) = items.first() else {
            return Err(self.fail_at(type_error("an empty list is not a call"), span));
        };
        match head.symbol() {
            Some("fn") if env.get("fn").is_none() => {
                let Some((params, body)) = fn_form(items) else {
                    return Err(self.fail_at(
                        Fail::new(
                            Code::DslTypeError,
                            "bad_fn: fn takes [params] of symbols and one body",
                        ),
                        span,
                    ));
                };
                Ok(Val::Fn(Func::Closure(Arc::new(Closure {
                    name: None,
                    params: params.iter().map(|p| Arc::from(*p)).collect(),
                    body: Arc::new(body.clone()),
                    env: env.clone(),
                    scope,
                    span: span.clone(),
                }))))
            }
            Some("let") if env.get("let").is_none() => {
                let (Some(Expr::Vector { items: binding, .. }), Some(body), 3) =
                    (items.get(1), items.get(2), items.len())
                else {
                    return Err(self.fail_at(
                        Fail::new(
                            Code::DslTypeError,
                            "bad_let: let takes one binding [name value] and one body",
                        ),
                        span,
                    ));
                };
                let (Some(name), Some(value), 2) = (
                    binding.first().and_then(Expr::symbol),
                    binding.get(1),
                    binding.len(),
                ) else {
                    return Err(self.fail_at(
                        Fail::new(
                            Code::DslTypeError,
                            "bad_let: let takes one binding [name value] and one body",
                        ),
                        span,
                    ));
                };
                let value = self.eval(value, env, scope)?;
                let inner = env.bind(Arc::from(name), value);
                self.eval(body, &inner, scope)
            }
            Some("if") if env.get("if").is_none() => {
                if items.len() != 4 {
                    return Err(self.fail_at(
                        Fail::new(
                            Code::DslTypeError,
                            "bad_if: if takes a condition and exactly two branches",
                        ),
                        span,
                    ));
                }
                let condition = self.eval(&items[1], env, scope)?;
                let chosen =
                    if truth("if", &condition).map_err(|f| self.fail_at(f, items[1].span()))? {
                        &items[2]
                    } else {
                        &items[3]
                    };
                self.eval(chosen, env, scope)
            }
            Some("match") if env.get("match").is_none() => {
                let Some(value) = items.get(1) else {
                    return Err(self.fail_at(
                        Fail::new(
                            Code::DslTypeError,
                            "bad_match: match takes a value and (case pattern body) clauses",
                        ),
                        span,
                    ));
                };
                let value = self.eval(value, env, scope)?;
                for clause in &items[2..] {
                    let Expr::List { items: parts, .. } = clause else {
                        continue;
                    };
                    if parts.len() != 3 || parts[0].symbol() != Some("case") {
                        continue;
                    }
                    if let Some(bound) = self.matches(&parts[1], &value, env, scope)? {
                        return self.eval(&parts[2], &bound, scope);
                    }
                }
                Err(self.fail_at(
                    Fail::new(
                        Code::DslTypeError,
                        format!("no_match: no case matches {value:?}"),
                    ),
                    span,
                ))
            }
            Some("def") if env.get("def").is_none() => Err(self.fail_at(
                Fail::new(
                    Code::DslTypeError,
                    "misplaced_def: def is only allowed at the top level",
                ),
                span,
            )),
            _ => {
                let f = self.eval(head, env, scope)?;
                let Val::Fn(f) = f else {
                    return Err(self.fail_at(
                        type_error(format!(
                            "{} is not a function and cannot be called",
                            f.kind()
                        )),
                        head.span(),
                    ));
                };
                let mut args = Vec::with_capacity(items.len() - 1);
                for item in &items[1..] {
                    args.push(self.eval(item, env, scope)?);
                }
                self.apply(&f, args, span)
            }
        }
    }

    /// Apply a function. `at` is the span of the call, for diagnostics.
    pub fn apply(&self, f: &Func, args: Vec<Val>, at: &SourceSpan) -> Result<Val, Fail> {
        match f {
            Func::Closure(c) => {
                if c.params.len() != args.len() {
                    return Err(
                        self.fail_at(arity_error(&f.describe(), c.params.len(), args.len()), at)
                    );
                }
                if self.native && c.scope == Scope::Stdlib {
                    if let Some(fast) = self.fast_path(c, &args, at)? {
                        return Ok(fast);
                    }
                }
                let mut env = c.env.clone();
                for (param, arg) in c.params.iter().zip(args) {
                    env = env.bind(param.clone(), arg);
                }
                self.eval(&c.body, &env, c.scope)
            }
            Func::Native(n) => {
                if !n.arity.accepts(args.len()) {
                    return Err(self.fail_at(arity_error(n.name, n.arity, args.len()), at));
                }
                (n.call)(self, &args, at).map_err(|f| self.fail_at(f, at))
            }
            Func::Partial(p) => {
                let mut all = p.args.clone();
                all.extend(args);
                self.apply(&p.f, all, at)
            }
        }
    }

    /// The native plan for a standard composition, when the arguments have
    /// the standard shapes; `None` runs the library's text.
    fn fast_path(
        &self,
        closure: &Closure,
        args: &[Val],
        at: &SourceSpan,
    ) -> Result<Option<Val>, Fail> {
        match closure.name.as_deref() {
            Some("table-from-json") => {
                let [binding, input] = args else {
                    return Ok(None);
                };
                let Val::Stream(source) = input else {
                    return Ok(None);
                };
                if !crate::lower::is_table_binding(binding) {
                    return Ok(None);
                }
                Ok(Some(Val::Stream(Arc::new(Plan::TableFromJson {
                    binding: binding.clone(),
                    source: source.clone(),
                    at: at.clone(),
                }))))
            }
            Some("csv") => {
                let [options, events] = args else {
                    return Ok(None);
                };
                let Val::Stream(source) = events else {
                    return Ok(None);
                };
                if crate::lower::csv_options(options).is_none() {
                    return Ok(None);
                }
                Ok(Some(Val::Text(Arc::new(Plan::Csv {
                    options: options.clone(),
                    source: source.clone(),
                }))))
            }
            _ => Ok(None),
        }
    }

    /// Whether a symbol in a pattern compares rather than binds: the same
    /// rule the resolver applies (a local binds; a definition or a native
    /// constant compares; anything else binds).
    fn pattern_constant(
        &self,
        name: &str,
        env: &Env,
        scope: Scope,
        span: &SourceSpan,
    ) -> Result<Option<Val>, Fail> {
        if env.has(name) {
            return Ok(None);
        }
        if let Some(v) = self.def_value(scope, name)? {
            return Ok(Some(v));
        }
        match native(name) {
            Some(n) if n.kind == Kind::Constant => Ok(Some((n.call)(self, &[], span)?)),
            _ => Ok(None),
        }
    }

    /// Match `value` against `pattern`: the environment extended with the
    /// pattern's bindings, or `None`.
    pub fn matches(
        &self,
        pattern: &Expr,
        value: &Val,
        env: &Env,
        scope: Scope,
    ) -> Result<Option<Env>, Fail> {
        match pattern {
            Expr::Symbol { name, .. } if name == "_" => Ok(Some(env.clone())),
            Expr::Symbol { name, span } => match self.pattern_constant(name, env, scope, span)? {
                Some(constant) => Ok((constant == *value).then(|| env.clone())),
                None => Ok(Some(env.bind(Arc::from(name.as_str()), value.clone()))),
            },
            Expr::Keyword { .. }
            | Expr::Str { .. }
            | Expr::Num { .. }
            | Expr::Bool { .. }
            | Expr::Null { .. } => {
                let literal = self.eval(pattern, env, scope)?;
                Ok((literal == *value).then(|| env.clone()))
            }
            Expr::Vector { items, .. } => {
                let Val::Vector(values) = value else {
                    return Ok(None);
                };
                if values.len() != items.len() {
                    return Ok(None);
                }
                let mut bound = env.clone();
                for (p, v) in items.iter().zip(values.iter()) {
                    match self.matches(p, v, &bound, scope)? {
                        Some(next) => bound = next,
                        None => return Ok(None),
                    }
                }
                Ok(Some(bound))
            }
            Expr::List { items, span } => {
                let head = items.first().and_then(Expr::symbol).ok_or_else(|| {
                    self.fail_at(
                        Fail::new(
                            Code::DslTypeError,
                            "bad_pattern: a list pattern is (constructor pattern...)",
                        ),
                        span,
                    )
                })?;
                let Val::Tagged { tag, fields } = value else {
                    return Ok(None);
                };
                if &**tag != head || fields.len() != items.len() - 1 {
                    return Ok(None);
                }
                let mut bound = env.clone();
                for (p, v) in items[1..].iter().zip(fields.iter()) {
                    match self.matches(p, v, &bound, scope)? {
                        Some(next) => bound = next,
                        None => return Ok(None),
                    }
                }
                Ok(Some(bound))
            }
        }
    }

    /// The program's result: `export` applied to the host's input plan. A
    /// stream or a text plan comes back; nothing has been consumed.
    pub fn export(&self) -> Result<Val, Fail> {
        let Some(export) = self.def_value(Scope::Program, "export")? else {
            return Err(Fail::new(
                Code::DslTypeError,
                "no_export: the program has no `def export [input]`",
            ));
        };
        let Val::Fn(f) = &export else {
            return Err(Fail::new(
                Code::DslTypeError,
                format!(
                    "type_mismatch: export must be a fn [input], not {}",
                    export.kind()
                ),
            ));
        };
        let at = self
            .program
            .get("export")
            .map(|d| d.span.clone())
            .unwrap_or_else(|| SourceSpan::new(&self.program.file, 0, 0));
        self.apply(f, vec![Val::Stream(Arc::new(Plan::Input))], &at)
    }

    /// Evaluate a standalone expression in the program's scope, for tests
    /// and tools.
    pub fn eval_program_expr(&self, expr: &Expr) -> Result<Val, Fail> {
        self.eval(expr, &Env::new(), Scope::Program)
    }

    /// A retained value as the standard library sees it.
    pub fn datum_to_val(d: &Datum) -> Val {
        Val::from_datum(d)
    }

    /// The natives, for tools that list them.
    pub fn natives() -> &'static [Native] {
        registry::natives()
    }
}

fn poisoned<T>(_: std::sync::PoisonError<T>) -> Fail {
    Fail::new(
        Code::DslTypeError,
        "internal: the definition cache is poisoned",
    )
}

/// Make a partial application value, for callers outside the evaluator.
pub fn partial(f: Func, args: Vec<Val>) -> Val {
    Val::Fn(Func::Partial(Arc::new(Partial { f, args })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{desugar, parse_file};

    fn runtime(src: &str) -> Runtime {
        let forms = desugar::program(parse_file(src, "t.alc").unwrap(), src).unwrap();
        let resolved = crate::resolve::resolve(forms, src, "t.alc", &stdlib::outer).unwrap();
        Runtime::new(Arc::new(resolved), src)
    }

    fn eval(src: &str, expr: &str) -> Result<Val, Fail> {
        let rt = runtime(src);
        let forms = desugar::program(parse_file(expr, "e.alc").unwrap(), expr).unwrap();
        rt.eval_program_expr(&forms[0])
    }

    #[test]
    fn values_records_and_lookups() {
        let v = eval(
            "def opts\n  record\n    entry :delimiter \",\"\n    entry :header true",
            "get :delimiter opts",
        )
        .unwrap();
        assert_eq!(v, Val::str(","));
        assert_eq!(eval("", "get :x (record)").unwrap(), Val::missing());
        assert_eq!(
            eval("", "get-path (as-path [\"a\" 0]) (record (entry :a [7]))").unwrap(),
            Val::Num {
                value: 7.0,
                lexeme: Some("7".into())
            }
        );
        assert_eq!(
            eval(
                "",
                "map (fn [x] (get :a x)) [(record (entry :a 1)) (record (entry :a 2))]"
            )
            .unwrap(),
            Val::vector(vec![Val::num(1.0), Val::num(2.0)])
        );
    }

    #[test]
    fn let_if_and_match_choose() {
        assert_eq!(
            eval("", "let [x true] (if x \"yes\" \"no\")").unwrap(),
            Val::str("yes")
        );
        let src = "def tag [v]\n  match v\n    case (selected :columns raw) raw\n    case table-end \"end\"\n    case [a b] b\n    case 1 \"one\"\n    case _ \"other\"";
        assert_eq!(
            eval(src, "tag (selected :columns 5)").unwrap(),
            Val::num(5.0)
        );
        assert_eq!(eval(src, "tag table-end").unwrap(), Val::str("end"));
        assert_eq!(eval(src, "tag [1 2]").unwrap(), Val::num(2.0));
        assert_eq!(eval(src, "tag 1").unwrap(), Val::str("one"));
        assert_eq!(eval(src, "tag no-schema").unwrap(), Val::str("other"));
        let no = eval("", "match 3 (case 1 1)").unwrap_err();
        assert!(no.message.starts_with("no_match: "), "{no}");
    }

    #[test]
    fn closures_partials_and_arity() {
        assert_eq!(
            eval("def add [a b] [a b]", "(partial add 1) 2").unwrap(),
            Val::vector(vec![Val::num(1.0), Val::num(2.0)])
        );
        let f = eval("def add [a b] [a b]", "add 1").unwrap_err();
        assert_eq!(f.code, Code::DslTypeError);
        assert!(f.message.starts_with("arity: fn add takes 2"), "{f}");
        let f = eval("", "get 1").unwrap_err();
        assert!(f.message.starts_with("arity: get takes 2"), "{f}");
        let f = eval("", "1 2").unwrap_err();
        assert!(f.message.contains("not a function"), "{f}");
    }

    #[test]
    fn a_program_def_shadows_the_library_and_the_library_keeps_its_own() {
        // The program's `csv-row` is not what the library's `csv` calls.
        let rt = runtime("def csv-row [o c] 1\ndef export [input] (csv csv-options (table-from-json b input))\ndef b 1");
        let lib = rt.def_value(Scope::Stdlib, "csv-row").unwrap().unwrap();
        let mine = rt.def_value(Scope::Program, "csv-row").unwrap().unwrap();
        assert!(matches!(lib, Val::Fn(Func::Closure(ref c)) if c.scope == Scope::Stdlib));
        assert!(matches!(mine, Val::Fn(Func::Closure(ref c)) if c.scope == Scope::Program));
    }

    #[test]
    fn fail_names_the_form_position_in_the_right_file() {
        let f = eval("def boom [x]\n  fail \"no\"", "boom 1").unwrap_err();
        assert_eq!(f.code, Code::InputInvalid);
        assert_eq!(f.message, "no");
        assert_eq!((f.row, f.column), (Some(2), Some(3)));
        // A failure inside the library names the library's line.
        let f = eval("", "table-finish no-schema").unwrap_err();
        assert_eq!(f.code, Code::InputInvalid);
        assert_eq!(f.message, "Required metadata was not found");
        assert!(f.row.is_some());
    }

    #[test]
    fn streams_are_plans_and_export_applies_to_the_input() {
        let rt = runtime("def export [input] (json input)");
        let out = rt.export().unwrap();
        assert!(matches!(out, Val::Text(ref p) if matches!(**p, Plan::Json { .. })));
        let rt = runtime("def x 1");
        let f = rt.export().unwrap_err();
        assert!(f.message.starts_with("no_export: "), "{f}");
        let rt = runtime("def export 1");
        assert!(rt
            .export()
            .unwrap_err()
            .message
            .starts_with("type_mismatch: "));
    }

    #[test]
    fn the_fast_paths_switch() {
        let src = "def b\n  record\n    entry :columns (path \"m\")\n    entry :rows (path \"r\" each-index)\n    entry :column (fn [d] d)\ndef export [input] (csv csv-options (table-from-json b input))";
        let fast = runtime(src).export().unwrap();
        assert!(matches!(fast, Val::Text(ref p) if matches!(**p, Plan::Csv { .. })));
        let slow = runtime(src).with_native(false).export().unwrap();
        assert!(matches!(slow, Val::Text(ref p) if matches!(**p, Plan::ConcatMap { .. })));
    }
}
