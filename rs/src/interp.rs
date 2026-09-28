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
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tabnas_transduce::{AbortFlag, Code, Datum, Duplicates, Fail, Limits};

use crate::ast::{Expr, SourceSpan};
use crate::resolve::{fn_form, fn_params, Resolved};
use crate::stdlib::registry::{self, native, truth, Kind, Native};
use crate::stdlib::{self, Stdlib};
use crate::value::{type_error, Closure, Env, Func, Partial, Plan, Scope, Seq, Val};

/// How deep evaluation may nest: a form inside a form, a function's body
/// inside the call that applied it, a definition's value inside the form
/// that named it. The resolver refuses a definition that names itself, but
/// a function that is handed itself (`def w [f] (f f)`, then `(w w)`)
/// recurses through values the resolver cannot see; this bound turns that
/// into a `recursion` failure where it would otherwise overflow the stack
/// and abort the process. A chain of definitions each naming the next, or
/// of functions each calling the next, nests a level per link and meets
/// the same bound, which is therefore also the bound on how deep a value a
/// program builds can nest: what the evaluator, the equality of patterns
/// and `Drop` walk one level at a time. Evaluation at this depth needs
/// [`crate::STACK_BYTES`] of stack, which `compile` and the command give it
/// and a host's run thread must.
pub const MAX_EVAL_DEPTH: usize = 1_000;

/// How many forms building a plan may evaluate: `compile` runs `export`
/// over the input's plan, which evaluates everything not under a stream,
/// and a small program can ask for an exponential amount of that work
/// (`def d [g] (fn [x] (g (g x)))` nested forty deep). Past this bound
/// building the plan is `RESOURCE_LIMIT_EXCEEDED` naming `max_plan_steps`.
/// The work a stream does per item is bounded by the host's abort flag
/// instead ([`Runtime::with_abort`]).
pub const MAX_PLAN_STEPS: u64 = 1_000_000;

/// How often, in evaluation steps, the abort flag is read.
const ABORT_EVERY: u64 = 64;

/// What a message that already names a standard-library position holds.
const LIBRARY_AT: &str = " (at stdlib/";

/// The evaluator for one program.
pub struct Runtime {
    program: Arc<Resolved>,
    src: Arc<str>,
    stdlib: &'static Stdlib,
    native: bool,
    duplicates: Duplicates,
    /// The limits values a program builds are measured against (a cell's
    /// text under `max_scalar_bytes`, a `scan-emit` state under
    /// `max_metadata_bytes` and `max_depth`): the host's at run time, the
    /// defaults while `compile` builds the plan.
    limits: Limits,
    /// The host's cancellation, read every [`ABORT_EVERY`] steps.
    abort: AbortFlag,
    /// The evaluation steps this runtime may take, when bounded
    /// ([`MAX_PLAN_STEPS`] while building a plan).
    fuel: Option<u64>,
    steps: AtomicU64,
    depth: AtomicUsize,
    /// Definition values, evaluated on first use.
    cache: Mutex<HashMap<(Scope, Arc<str>), Val>>,
}

/// One level of evaluation, given back when it ends.
struct Level<'r>(&'r AtomicUsize);

impl Drop for Level<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
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
            limits: Limits::default(),
            abort: AbortFlag::new(),
            fuel: None,
            steps: AtomicU64::new(0),
            depth: AtomicUsize::new(0),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The limits the values a program builds are measured against.
    pub fn with_limits(mut self, limits: &Limits) -> Runtime {
        self.limits = limits.clone();
        self
    }

    /// The host's cancellation: once it is set, the next evaluation step
    /// fails with `ABORTED`, however long the item's computation would
    /// have run.
    pub fn with_abort(mut self, abort: AbortFlag) -> Runtime {
        self.abort = abort;
        self
    }

    /// Bound the evaluation steps this runtime may take (`None`: no bound).
    pub fn with_fuel(mut self, fuel: Option<u64>) -> Runtime {
        self.fuel = fuel;
        self
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// One evaluation step: the fuel, and every [`ABORT_EVERY`] steps the
    /// abort flag. Every form evaluated takes one, and so does every node
    /// a finite text or a measured value walks.
    pub fn tick(&self) -> Result<(), Fail> {
        let n = self.steps.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(max) = self.fuel {
            if n > max {
                return Err(Fail::limit(
                    "max_plan_steps",
                    max,
                    format!(
                        "building the plan took more than {max} evaluation steps; a program that computes this much before it reads its input is refused"
                    ),
                ));
            }
        }
        if n % ABORT_EVERY == 0 && self.abort.is_aborted() {
            return Err(Fail::aborted());
        }
        Ok(())
    }

    /// Enter one level of evaluation at `at`, or the `recursion` failure
    /// past [`MAX_EVAL_DEPTH`]. The evaluator takes a level per form, and
    /// so does writing a finite text, whose `concat-map` applies its
    /// function as it writes: a function that answers a text applying
    /// itself recurses there, not in the evaluator.
    pub fn enter(&self, at: Option<&SourceSpan>) -> Result<impl Drop + '_, Fail> {
        let depth = self.depth.fetch_add(1, Ordering::Relaxed) + 1;
        let level = Level(&self.depth);
        if depth > MAX_EVAL_DEPTH {
            let fail = Fail::new(
                Code::StreamabilityUnknown,
                format!(
                    "recursion: evaluation nested past {MAX_EVAL_DEPTH} levels: a function applied to itself, or definitions or calls chained that deep; strict mode refuses recursion without a bound"
                ),
            );
            return Err(match at {
                Some(at) => self.fail_at(fail, at),
                None => fail,
            });
        }
        Ok(level)
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

    /// The 1-based row and column of a span of the program's own text;
    /// `None` for a span of the standard library's, whose rows are not
    /// the program's.
    pub fn position(&self, span: &SourceSpan) -> Option<(u64, u64)> {
        if stdlib::file_of(span).is_some() {
            return None;
        }
        let (row, col) = span.position(&self.src);
        Some((row as u64, col as u64))
    }

    /// `fail` with the position of `at`, when it has none yet. A form of
    /// the program gives its row and column. A form of the standard
    /// library gives none, since a row there would name a line of the
    /// user's file that says something else; the message ends with the
    /// library file, row and column instead, `(at stdlib/table.alc:38:5)`,
    /// once, and a form of the program around the library's call, when
    /// the failure passes one on its way out, gives the row and column.
    pub fn fail_at(&self, mut fail: Fail, at: &SourceSpan) -> Fail {
        if fail.row.is_some() {
            return fail;
        }
        match stdlib::file_of(at) {
            Some((file, src)) => {
                if !fail.message.contains(LIBRARY_AT) {
                    let (row, col) = at.position(src);
                    fail.message = format!("{} (at {file}:{row}:{col})", fail.message);
                }
                fail
            }
            None => {
                let (row, col) = at.position(&self.src);
                fail.at(row as u64, col as u64)
            }
        }
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
        self.tick()?;
        let _level = self.enter(Some(expr.span()))?;
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
                                v.live_kind()
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
                        format!("no_match: no case matches {}", crate::lower::brief(&value)),
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

/// What a bounded walk over a value found: its size in the transduce
/// measure (payload bytes plus an allowance per node) and its nesting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Measure {
    pub bytes: u64,
    pub depth: usize,
}

/// The bounds a [`Runtime::measure`] walk holds a value to.
#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    /// Bytes counted for every node on top of its payload.
    pub node_bytes: u64,
    pub max_bytes: u64,
    /// The `Limits` field `max_bytes` came from, for the failure.
    pub bytes_limit: &'static str,
    /// The deepest nesting allowed, under `max_depth`.
    pub max_depth: usize,
    /// What is being measured, for the message.
    pub what: &'static str,
}

/// One thing a measure walk still has to visit.
enum Node<'v> {
    Val(&'v Val),
    Plan(&'v Plan),
    Env(&'v Env),
    /// A function held by something other than a value: the function a
    /// partial applies, when it is itself a partial, or the one a plan
    /// applies.
    Fn(&'v Func),
}

/// What a function value holds, one level below `depth`: a closure its
/// frames; a partial its arguments and the function it applies. A partial
/// of a partial holds the one before it, so a chain of them (a state that
/// wraps itself once per item) is walked link by link, each one node and
/// one level, as it is dropped.
fn push_fn<'v>(f: &'v Func, depth: usize, pending: &mut Vec<(Node<'v>, usize)>) {
    match f {
        Func::Native(_) => {}
        Func::Closure(c) => pending.push((Node::Env(&c.env), depth + 1)),
        Func::Partial(p) => {
            pending.extend(p.args.iter().map(|i| (Node::Val(i), depth + 1)));
            match &p.f {
                Func::Native(_) => {}
                Func::Closure(c) => pending.push((Node::Env(&c.env), depth + 1)),
                inner @ Func::Partial(_) => pending.push((Node::Fn(inner), depth + 1)),
            }
        }
    }
}

impl Runtime {
    /// Walk `v` without recursing, counting its size and nesting, and fail
    /// as soon as either passes its bound: `RESOURCE_LIMIT_EXCEEDED`
    /// naming the byte limit, or `max_depth`. The walk visits a shared
    /// part once per reference, so a value built by sharing (a vector of
    /// itself twice, forty times over) counts as large as it would be
    /// written, and the walk ends at the bound rather than visiting it
    /// all. Every node takes an evaluation step, so the abort flag stops
    /// it too.
    pub fn measure(&self, v: &Val, bounds: Bounds) -> Result<Measure, Fail> {
        let mut found = Measure::default();
        let mut pending: Vec<(Node<'_>, usize)> = vec![(Node::Val(v), 1)];
        while let Some((node, depth)) = pending.pop() {
            self.tick()?;
            if depth > bounds.max_depth {
                return Err(Fail::limit(
                    "max_depth",
                    bounds.max_depth as u64,
                    format!(
                        "{} nests more than {} levels deep",
                        bounds.what, bounds.max_depth
                    ),
                ));
            }
            found.depth = found.depth.max(depth);
            let payload: usize = match node {
                Node::Val(v) => match v {
                    Val::Null | Val::Bool(_) => 0,
                    Val::Num { lexeme, .. } => lexeme.as_ref().map_or(8, |l| l.len()),
                    Val::Str(s) | Val::Keyword(s) => s.len(),
                    Val::Vector(items) => {
                        pending.extend(items.iter().map(|i| (Node::Val(i), depth + 1)));
                        0
                    }
                    Val::Record(fields) => {
                        pending.extend(fields.values().map(|i| (Node::Val(i), depth + 1)));
                        fields.keys().map(|k| k.len()).sum()
                    }
                    Val::Tagged { tag, fields } => {
                        pending.extend(fields.iter().map(|i| (Node::Val(i), depth + 1)));
                        tag.len()
                    }
                    Val::Selector(s) => s.to_string().len(),
                    Val::CaptureSpec(c) => c.tag.len() + c.selector.to_string().len(),
                    Val::Fn(f) => {
                        push_fn(f, depth, &mut pending);
                        0
                    }
                    Val::Stream(plan) | Val::Text(plan) => {
                        pending.push((Node::Plan(plan), depth + 1));
                        0
                    }
                },
                Node::Plan(plan) => match plan {
                    Plan::Lit(s) => s.len(),
                    Plan::Concat { items, .. } => {
                        pending.extend(items.iter().map(|i| (Node::Val(i), depth + 1)));
                        0
                    }
                    Plan::Join {
                        sep,
                        items: Seq::Vector(items),
                    } => {
                        pending.extend(items.iter().map(|i| (Node::Val(i), depth + 1)));
                        sep.len()
                    }
                    // A finite concat-map keeps the function it applies
                    // until it is written: a partial over the state before
                    // holds that state, so the function is walked like any
                    // value the plan holds.
                    Plan::ConcatMap { f, items, .. } => {
                        pending.push((Node::Fn(f), depth + 1));
                        if let Seq::Vector(items) = items {
                            pending.extend(items.iter().map(|i| (Node::Val(i), depth + 1)));
                        }
                        0
                    }
                    Plan::Replace { from, to, source } => {
                        pending.push((Node::Val(source), depth + 1));
                        from.len() + to.len()
                    }
                    // A live plan's source is the one pass over the input,
                    // not a retained value, and is not walked; what the
                    // plan itself holds (its functions, its options, a
                    // scan's initial state) is.
                    Plan::Map { f, .. } | Plan::Filter { f, .. } => {
                        pending.push((Node::Fn(f), depth + 1));
                        0
                    }
                    Plan::ScanEmit {
                        init, step, finish, ..
                    } => {
                        pending.push((Node::Val(init), depth + 1));
                        pending.push((Node::Fn(step), depth + 1));
                        pending.push((Node::Fn(finish), depth + 1));
                        0
                    }
                    Plan::TableFromJson { binding: v, .. }
                    | Plan::CsvTable { options: v, .. }
                    | Plan::Csv { options: v, .. } => {
                        pending.push((Node::Val(v), depth + 1));
                        0
                    }
                    Plan::Route { specs, .. } => specs
                        .iter()
                        .map(|c| c.tag.len() + c.selector.to_string().len())
                        .sum(),
                    Plan::Select { selector, .. } => selector.to_string().len(),
                    Plan::Join {
                        sep,
                        items: Seq::Stream(_),
                    } => sep.len(),
                    Plan::Input
                    | Plan::Events { .. }
                    | Plan::Records { .. }
                    | Plan::Json { .. } => 0,
                },
                // A frame's value, and the frames outside it one level
                // further: a chain of frames is dropped one inside another.
                Node::Env(env) => match env.frame() {
                    Some((value, next)) => {
                        pending.push((Node::Val(value), depth + 1));
                        pending.push((Node::Env(next), depth + 1));
                        0
                    }
                    None => 0,
                },
                Node::Fn(f) => {
                    push_fn(f, depth, &mut pending);
                    0
                }
            };
            found.bytes = found
                .bytes
                .saturating_add(bounds.node_bytes)
                .saturating_add(payload as u64);
            if found.bytes > bounds.max_bytes {
                return Err(Fail::limit(
                    bounds.bytes_limit,
                    bounds.max_bytes,
                    format!("{} holds more than {} bytes", bounds.what, bounds.max_bytes),
                ));
            }
        }
        Ok(found)
    }

    /// The compact JSON text of a vector or a record, as a cell writes it
    /// (number lexemes kept), bounded by `max_scalar_bytes`: the text is
    /// one scalar of the output, and the native table holds a container
    /// cell's text to the same bound. The failure comes exactly when the
    /// text is longer than the bound; a lower bound of its length is
    /// counted first, without writing it, so a value built by sharing (a
    /// vector of a vector of itself, forty levels deep) fails at the limit
    /// rather than being written out.
    pub fn json_text(&self, v: &Val) -> Result<String, Fail> {
        let max = self.limits.max_scalar_bytes as u64;
        let too_long = || {
            Fail::limit(
                "max_scalar_bytes",
                max,
                format!("a cell's JSON text holds more than {max} bytes"),
            )
        };
        if self.json_text_floor(v, max)? > max {
            return Err(too_long());
        }
        let text = v.to_json_text()?;
        if text.len() as u64 > max {
            return Err(too_long());
        }
        Ok(text)
    }

    /// At most the length of `v`'s compact JSON text, counted without
    /// writing it or recursing, and without going on once it passes `max`:
    /// a container's brackets, a member's quoted key and colon, a string's
    /// quotes around its bytes (escapes only lengthen it), a number's
    /// lexeme (or one digit), four for `null` and the booleans. Every value
    /// counts at least one, so the walk ends within `max` values, and every
    /// one takes an evaluation step, so the abort flag stops it too.
    fn json_text_floor(&self, v: &Val, max: u64) -> Result<u64, Fail> {
        let mut total: u64 = 0;
        let mut pending: Vec<&Val> = vec![v];
        while let Some(v) = pending.pop() {
            self.tick()?;
            let here: usize = match v {
                Val::Null | Val::Bool(_) => 4,
                Val::Num { lexeme, .. } => lexeme.as_ref().map_or(1, |l| l.len().max(1)),
                Val::Str(s) => s.len() + 2,
                Val::Vector(items) => {
                    pending.extend(items.iter());
                    2
                }
                Val::Record(fields) => {
                    pending.extend(fields.values());
                    2 + fields.keys().map(|k| k.len() + 3).sum::<usize>()
                }
                // Not JSON: the conversion refuses it.
                _ => 0,
            };
            total = total.saturating_add(here as u64);
            if total > max {
                break;
            }
        }
        Ok(total)
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
        // A value no case takes is named by its kind and a short prefix,
        // so a failure over a large document does not carry the document.
        let big = format!("match [{}] (case 1 1)", "\"abcdefgh\" ".repeat(10_000));
        let no = eval("", &big).unwrap_err();
        assert!(
            no.message
                .starts_with("no_match: no case matches a vector ("),
            "{no}"
        );
        assert!(no.message.len() < 200, "{}", no.message.len());
    }

    /// The stack operators: data last, a new vector each time, bounded by
    /// the vector's length; an empty vector refuses `pop` and `top` with a
    /// type error naming the operator.
    #[test]
    fn the_stack_operators_and_the_string_forms() {
        assert_eq!(
            eval("", "push :b [:a]").unwrap(),
            Val::vector(vec![Val::keyword("a"), Val::keyword("b")])
        );
        assert_eq!(
            eval("", "pop [1 2 3]").unwrap(),
            Val::vector(vec![Val::num(1.0), Val::num(2.0)])
        );
        assert_eq!(eval("", "top [1 2 3]").unwrap(), Val::num(3.0));
        assert_eq!(eval("", "count [1 2 3]").unwrap(), Val::num(3.0));
        assert_eq!(eval("", "count []").unwrap(), Val::num(0.0));
        assert_eq!(
            eval("", "top (push \"x\" (pop [1 2]))").unwrap(),
            Val::str("x")
        );
        assert_eq!(eval("", "pop [1]").unwrap(), Val::vector(vec![]));
        for (expr, message) in [
            ("pop []", "pop: the vector is empty"),
            ("top []", "top: the vector is empty"),
            ("count 1", "count: the data must be a vector"),
            ("push 1 :k", "push: the data must be a vector"),
            ("pop (record)", "pop: the data must be a vector"),
            ("quoted 1", "quoted: the string must be a string"),
            (
                "repeat 2.5 \"a\"",
                "repeat: the count must be a whole number of at least 0",
            ),
            ("repeat 2 1", "repeat: the string must be a string"),
        ] {
            let f = eval("", expr).unwrap_err();
            assert_eq!(f.code, Code::DslTypeError, "{expr}");
            assert!(
                f.message.starts_with(&format!("type_mismatch: {message}")),
                "{expr}: {f}"
            );
        }
        assert_eq!(
            eval("", "quoted \"a\\\"b\\\\c\\n\"").unwrap(),
            Val::str("\"a\\\"b\\\\c\\n\"")
        );
        assert_eq!(eval("", "repeat 3 \"ab\"").unwrap(), Val::str("ababab"));
        assert_eq!(eval("", "repeat 0 \"ab\"").unwrap(), Val::str(""));
        assert_eq!(eval("", "repeat 2 \"\"").unwrap(), Val::str(""));
        // The result is one scalar of the output: past max_scalar_bytes it
        // is refused before it is built, naming the limit.
        let f = eval("", "repeat 1000000000 \"abcdefghij\"").unwrap_err();
        assert_eq!(f.code, Code::ResourceLimitExceeded, "{f}");
        assert_eq!(f.limit.as_ref().unwrap().name, "max_scalar_bytes");
        // A count past what an index holds is the limit's refusal too,
        // judged before the count is narrowed; a fraction or a negative
        // count is a type error.
        let f = eval("", "repeat 100000000000000000000 \"a\"").unwrap_err();
        assert_eq!(f.code, Code::ResourceLimitExceeded, "{f}");
        assert_eq!(f.limit.as_ref().unwrap().name, "max_scalar_bytes");
        assert_eq!(
            eval("", "repeat 100000000000000000000 \"\"").unwrap(),
            Val::str("")
        );
        let f = eval("", "repeat 1.5 \"a\"").unwrap_err();
        assert_eq!(f.code, Code::DslTypeError, "{f}");
        assert!(f.message.contains("not 1.5"), "{f}");
        // `kind` passed as a function meets what the checker refuses where
        // it is named: a finite text is refused as a live one is, and no
        // `:text` or `:stream` is ever answered.
        assert_eq!(
            eval("", "map kind [1 \"s\"]").unwrap(),
            Val::vector(vec![Val::keyword("number"), Val::keyword("string")])
        );
        let f = eval("", "map kind [(text \"x\")]").unwrap_err();
        assert_eq!(f.code, Code::DslTypeError, "{f}");
        assert!(
            f.message
                .starts_with("type_mismatch: kind: a text cannot be asked"),
            "{f}"
        );
        // The quoted form of a string can be six times the string: it is
        // held to the same limit, refused before it is built.
        let f = eval("", "quoted (repeat 3000000 \"\\u0001\")").unwrap_err();
        assert_eq!(f.code, Code::ResourceLimitExceeded, "{f}");
        assert_eq!(f.limit.as_ref().unwrap().name, "max_scalar_bytes");
        assert!(f.message.starts_with("quoted:"), "{f}");
        // The event values a program builds are the ones `events` delivers.
        assert_eq!(
            eval("", "key \"k\"").unwrap(),
            Val::tagged("key", vec![Val::str("k")])
        );
        assert_eq!(
            eval("", "scalar null").unwrap(),
            Val::tagged("scalar", vec![Val::Null])
        );
        assert_eq!(
            eval("", "object-start").unwrap(),
            Val::tagged("object-start", vec![])
        );
        assert!(eval("", "key :k")
            .unwrap_err()
            .message
            .starts_with("type_mismatch: key: "));
        assert!(eval("", "scalar [1]")
            .unwrap_err()
            .message
            .starts_with("type_mismatch: scalar: "));
        // Matched as the table events are: the constants compare, the
        // constructors bind their one field.
        let src = "def tag [e]\n  match e\n    case object-start \"{\"\n    case array-end \"]\"\n    case (key name) name\n    case (scalar v) v\n    case _ \"?\"";
        assert_eq!(eval(src, "tag object-start").unwrap(), Val::str("{"));
        assert_eq!(eval(src, "tag array-end").unwrap(), Val::str("]"));
        assert_eq!(eval(src, "tag object-end").unwrap(), Val::str("?"));
        assert_eq!(eval(src, "tag (key \"k\")").unwrap(), Val::str("k"));
        assert_eq!(eval(src, "tag (scalar 5)").unwrap(), Val::num(5.0));
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
        // A failure inside the library names the library's file and line
        // in its message, and no row of the program's.
        let f = eval("", "table-finish no-schema").unwrap_err();
        assert_eq!(f.code, Code::InputInvalid);
        let lib = stdlib::source("stdlib/table.alc").unwrap();
        let row = lib
            .lines()
            .position(|l| l.contains("fail \"Required metadata was not found\""))
            .unwrap()
            + 1;
        assert_eq!(
            f.message,
            format!("Required metadata was not found (at stdlib/table.alc:{row}:5)")
        );
        assert_eq!((f.row, f.column), (None, None));
        // A program named like a library file is still its own text.
        let src = "def boom [x]\n  fail \"no\"";
        let forms = desugar::program(parse_file(src, "stdlib/table.alc").unwrap(), src).unwrap();
        let resolved =
            crate::resolve::resolve(forms, src, "stdlib/table.alc", &stdlib::outer).unwrap();
        let rt = Runtime::new(Arc::new(resolved), src);
        let call = desugar::program(parse_file("boom 1", "e.alc").unwrap(), "boom 1").unwrap();
        let f = rt.eval_program_expr(&call[0]).unwrap_err();
        assert_eq!(
            (f.message.as_str(), f.row, f.column),
            ("no", Some(2), Some(3))
        );
        // A library failure under a native the program called takes the
        // program's position too.
        let f = eval("", "map (fn [s] (table-finish s)) [no-schema]").unwrap_err();
        assert!(
            f.message
                .ends_with(&format!("(at stdlib/table.alc:{row}:5)")),
            "{f}"
        );
        assert_eq!((f.row, f.column), (Some(1), Some(1)));
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

    /// A partial whose function is itself a partial holds the one before
    /// it, so a chain of them (a scan-emit state that wraps itself once per
    /// item) is measured link by link: each link one node and one level,
    /// as it is dropped, and every link's arguments counted.
    #[test]
    fn a_chain_of_partials_is_measured() {
        let rt = runtime("");
        let arg = Val::str(&"a".repeat(100));
        let chain = |links: usize| {
            let mut f = Func::Native(registry::native("vector").unwrap());
            for _ in 0..links {
                let Val::Fn(next) = partial(f, vec![arg.clone()]) else {
                    unreachable!()
                };
                f = next;
            }
            Val::Fn(f)
        };
        let bounds = |max_bytes: u64, max_depth: usize| Bounds {
            node_bytes: 16,
            max_bytes,
            bytes_limit: "max_metadata_bytes",
            max_depth,
            what: "the state",
        };
        let three = rt.measure(&chain(3), bounds(u64::MAX, usize::MAX)).unwrap();
        assert_eq!(
            three,
            Measure {
                bytes: 6 * 16 + 300,
                depth: 4
            }
        );
        let long = chain(1000);
        let fail = rt.measure(&long, bounds(1 << 30, 256)).unwrap_err();
        assert_eq!(fail.limit.as_ref().unwrap().name, "max_depth", "{fail}");
        let fail = rt.measure(&long, bounds(4096, usize::MAX)).unwrap_err();
        assert_eq!(
            fail.limit.as_ref().unwrap().name,
            "max_metadata_bytes",
            "{fail}"
        );
    }

    /// A finite text keeps the function its concat-map applies, so a state
    /// that hides the one before it in that function's partial (a state of
    /// `[(concat-map (partial g s) ["x"])]`) is walked through it: vector,
    /// text, plan, function, then the state before, each a level.
    #[test]
    fn the_function_a_finite_text_applies_is_measured() {
        let rt = runtime("");
        let file: Arc<str> = Arc::from("t.alc");
        let at = SourceSpan::new(&file, 0, 0);
        let g = || Func::Native(registry::native("vector").unwrap());
        let link = |prev: Val| {
            let Val::Fn(f) = partial(g(), vec![prev]) else {
                unreachable!()
            };
            let items: Arc<[Val]> = Arc::from(vec![Val::str("x")]);
            let text = Val::Text(Arc::new(Plan::ConcatMap {
                f,
                items: Seq::Vector(items),
                at: at.clone(),
            }));
            Val::Vector(Arc::from(vec![text]))
        };
        let chain = |links: usize| {
            let mut v = Val::Vector(Arc::from(Vec::<Val>::new()));
            for _ in 0..links {
                v = link(v);
            }
            v
        };
        let bounds = |max_bytes: u64, max_depth: usize| Bounds {
            node_bytes: 16,
            max_bytes,
            bytes_limit: "max_metadata_bytes",
            max_depth,
            what: "the state",
        };
        // One link: the vector (1), its text (2), the plan (3), the
        // function and the item "x" (4), the state before (5).
        let one = rt.measure(&chain(1), bounds(u64::MAX, usize::MAX)).unwrap();
        assert_eq!(
            one,
            Measure {
                bytes: 6 * 16 + 1,
                depth: 5
            }
        );
        // Each further link is four more levels and five more nodes.
        let three = rt.measure(&chain(3), bounds(u64::MAX, usize::MAX)).unwrap();
        assert_eq!(
            three,
            Measure {
                bytes: 16 * 16 + 3,
                depth: 13
            }
        );
        let long = chain(1000);
        let fail = rt.measure(&long, bounds(1 << 30, 256)).unwrap_err();
        assert_eq!(fail.limit.as_ref().unwrap().name, "max_depth", "{fail}");
        let fail = rt.measure(&long, bounds(4096, usize::MAX)).unwrap_err();
        assert_eq!(
            fail.limit.as_ref().unwrap().name,
            "max_metadata_bytes",
            "{fail}"
        );
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
