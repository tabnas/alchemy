//! The checker: conservative inference over the core forms (spec sections
//! 10.2 and 15; design brief 4.3).
//!
//! Every definition of a program is checked, `export` first with its
//! `input` bound to `JsonEvents`, the others with their parameters
//! unknown. A native's arguments are checked against its signature, a
//! standard-library definition's against the signature the library
//! declares for it ([`stdlib_signature`]), a program's own definition's
//! against what its body inferred. What cannot be inferred is `Unknown`
//! and passes; what is known to be wrong is reported, `DSL_TYPE_ERROR`
//! with a finer code as the first word of the message:
//!
//! - `arity`: a wrong argument count;
//! - `type_mismatch`: an argument or a condition of the wrong kind, a
//!   vector or record holding a stream, a value that is not a function
//!   called;
//! - `protocol_mismatch`: one protocol where another was wanted (`csv`
//!   given `JsonEvents`, `table-from-json` given `Text`, `json` given a
//!   stream of items);
//! - `no_export`: no `def export`; `bad_output`: an `export` whose result
//!   is not a text, table events or JSON events.
//!
//! Ownership (spec 10.2): a binding of `Stream`, `JsonEvents` or `Text`
//! type is affine. Used twice in its scope it is `STREAM_REUSED`
//! (`reused`), where the two arms of an `if` or the cases of a `match`
//! are alternatives, not two uses; used inside a `fn` body from the scope
//! around it, it is `STREAM_REUSED` (`captured`), since the function could
//! run more than once. Strict mode, the only mode: the function given to
//! `map`, `filter` or `concat-map` over a stream, and the step and finish
//! of `scan-emit`, must resolve statically (a `fn`, a definition, a
//! native, or a `partial` of one), else `STREAMABILITY_UNKNOWN`
//! (`dynamic`); recursion is refused by the resolver (`recursion`). A
//! result of `export` that cannot be typed is `STREAMABILITY_UNKNOWN`
//! (`unknown_output`).
//!
//! The standard library's own text is checked the same way, each
//! definition's body against its declared signature; `stdlib::load` does
//! it once, and the differential test asserts it holds.

use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;
use tabnas_transduce::{Code, Fail};

use crate::ast::{Expr, SourceSpan, Sources};
use crate::program::Output;
use crate::resolve::{fn_form, Def, Resolved};
use crate::stdlib::registry::{native, Kind, Native};
use crate::types::Type;

/// What the checker learned about a program.
#[derive(Clone, Debug, PartialEq)]
pub struct Checked {
    /// The type of `export`'s result.
    pub export: Type,
    /// The protocol the host renders, decided by that type.
    pub output: Output,
    /// Every other definition's type.
    pub defs: IndexMap<Arc<str>, Type>,
}

/// The signature the standard library declares for one of its
/// definitions; the library's text is checked against it and programs
/// call it through it.
pub fn stdlib_signature(name: &str) -> Option<Type> {
    use Type::*;
    Some(match name {
        "public-column" => Type::func(vec![Record], Record),
        "table-inferred-column" => Type::func(vec![Type::String], Record),
        "table-row" => Type::func(vec![Unknown, Value], Type::tagged("row")),
        "table-first-row" => Type::func(vec![Record, Unknown, Value], Type::tagged("transition")),
        "table-step" => Type::func(vec![Record, Unknown, Unknown], Type::tagged("transition")),
        "table-finish" => Type::func(vec![Unknown], Type::vector(TableEvent)),
        "table-finish-for" => Type::func(vec![Record, Unknown], Type::vector(TableEvent)),
        "table-captures" => Type::func(vec![Record], Type::vector(CaptureSpec)),
        "table-from-json" => Type::func(vec![Record, JsonEvents], Type::table_events()),
        "csv-options" => Record,
        "csv-field" => Type::func(vec![Record, Value], Text),
        "csv-row" => Type::func(vec![Record, Type::vector(Value)], Text),
        "csv" => Type::func(vec![Record, Type::table_events()], Text),
        _ => return None,
    })
}

/// The type of a native as a value: a constant's value, or the function.
fn native_type(n: &Native) -> Type {
    match (n.kind, n.name) {
        (Kind::Constant, "root" | "each-index" | "each-member") => Type::Selector,
        (Kind::Constant, "missing") => Type::Value,
        (Kind::Constant, name) => Type::tagged(name),
        (_, _) => match n.arity.exact() {
            Some(k) => Type::func_of(k),
            None => Type::Unknown,
        },
    }
}

/// A local binding.
struct Local {
    name: Arc<str>,
    ty: Type,
}

type Env = Vec<Local>;

fn lookup<'e>(env: &'e Env, name: &str) -> Option<&'e Type> {
    env.iter().rev().find(|l| &*l.name == name).map(|l| &l.ty)
}

/// How many definitions deep the checker follows a stream into the
/// bodies it is passed to ([`Checker::applied`]). Each level types a body
/// again inside the call that reached it, so the bound keeps the checker's
/// own recursion shallow; past it a definition is typed by what its body
/// inferred with its parameters unknown, and the runtime's guard (a live
/// plan is lowered once: `concat` refuses two live texts, a vector or a
/// finite text refuses a live one) is what stops a second use.
pub const MAX_APPLIED: usize = 32;

/// The checker for one scope's definitions: a program's, or one standard
/// library file's.
struct Checker<'a> {
    sources: &'a Sources,
    defs: &'a IndexMap<Arc<str>, Def>,
    /// The library file's declared signatures stand for its definitions;
    /// a program's are inferred.
    declared: bool,
    memo: HashMap<Arc<str>, Type>,
    /// A definition's type with its parameters bound to the argument
    /// types of a call that passes it a stream, by name and argument
    /// types.
    applied: Vec<(Arc<str>, Vec<Type>, Type)>,
    /// How many [`Checker::applied`] bodies are being typed, one inside
    /// the next.
    applying: usize,
}

impl Checker<'_> {
    fn fail(
        &self,
        code: Code,
        finer: &str,
        message: impl std::fmt::Display,
        span: &SourceSpan,
    ) -> Fail {
        self.sources
            .fail_at(Fail::new(code, format!("{finer}: {message}")), span)
    }

    fn type_error(&self, finer: &str, message: impl std::fmt::Display, span: &SourceSpan) -> Fail {
        self.fail(Code::DslTypeError, finer, message, span)
    }

    /// `type_mismatch` or `protocol_mismatch`, by what was wanted and what
    /// came.
    fn mismatch(&self, what: &str, expected: &Type, actual: &Type, span: &SourceSpan) -> Fail {
        let finer = if expected.is_protocol() && actual.is_protocol() {
            "protocol_mismatch"
        } else {
            "type_mismatch"
        };
        self.type_error(
            finer,
            format!("{what} must be {expected}, not {actual}"),
            span,
        )
    }

    fn expect(
        &self,
        what: &str,
        expected: &Type,
        actual: &Type,
        span: &SourceSpan,
    ) -> Result<(), Fail> {
        if expected.accepts(actual) {
            Ok(())
        } else {
            Err(self.mismatch(what, expected, actual, span))
        }
    }

    /// Whether `name` is bound outside the locals: a definition of this
    /// scope, a library definition, or a native.
    fn global_type(&mut self, name: &str) -> Result<Option<Type>, Fail> {
        if self.defs.contains_key(name) {
            return self.def_type(name).map(Some);
        }
        if let Some(t) = stdlib_signature(name) {
            return Ok(Some(t));
        }
        Ok(native(name).map(native_type))
    }

    /// The type of one of this scope's definitions, inferred once.
    fn def_type(&mut self, name: &str) -> Result<Type, Fail> {
        if let Some(t) = self.memo.get(name) {
            return Ok(t.clone());
        }
        if self.declared {
            if let Some(t) = stdlib_signature(name) {
                self.memo.insert(Arc::from(name), t.clone());
                return Ok(t);
            }
        }
        let def = self
            .defs
            .get(name)
            .expect("a definition of this scope")
            .clone();
        let ty = match &*def.value {
            Expr::List { items, .. } if fn_form(items).is_some() => {
                let (params, _) = fn_form(items).expect("checked");
                let params: Vec<Type> = vec![Type::Unknown; params.len()];
                self.fn_type(items, &params, &mut Vec::new())?
            }
            other => self.infer(other, &mut Vec::new())?,
        };
        self.memo.insert(Arc::from(name), ty.clone());
        Ok(ty)
    }

    /// Check a definition whose signature is declared: the body against
    /// the declaration.
    fn check_declared(&mut self, def: &Def, declared: &Type) -> Result<(), Fail> {
        match (&*def.value, declared) {
            (Expr::List { items, .. }, Type::Fn(params, result)) if fn_form(items).is_some() => {
                let (names, _) = fn_form(items).expect("checked");
                if names.len() != params.len() {
                    return Err(self.type_error(
                        "arity",
                        format!(
                            "{} is declared with {} parameter(s) but takes {}",
                            def.name,
                            params.len(),
                            names.len()
                        ),
                        &def.span,
                    ));
                }
                let got = self.fn_type(items, params, &mut Vec::new())?;
                let Type::Fn(_, body) = got else {
                    unreachable!("fn_type answers a Fn");
                };
                self.expect(
                    &format!("the result of {}", def.name),
                    result,
                    &body,
                    &def.span,
                )
            }
            (value, declared) => {
                let got = self.infer(value, &mut Vec::new())?;
                self.expect(
                    &format!("the value of {}", def.name),
                    declared,
                    &got,
                    &def.span,
                )
            }
        }
    }

    /// The type of a `fn` form with its parameters bound to `params`; the
    /// body is checked, and an affine parameter's uses counted.
    fn fn_type(&mut self, items: &[Expr], params: &[Type], env: &mut Env) -> Result<Type, Fail> {
        let (names, body) = fn_form(items).expect("a fn form");
        let depth = env.len();
        for (name, ty) in names.iter().zip(params) {
            if ty.is_affine() {
                self.affine(name, body, env)?;
            }
            env.push(Local {
                name: Arc::from(*name),
                ty: ty.clone(),
            });
        }
        let result = self.infer(body, env);
        env.truncate(depth);
        Ok(Type::func(params.to_vec(), result?))
    }

    /// An affine binding `name` scoped over `body`: at most one use, and
    /// none inside a nested `fn`.
    fn affine(&self, name: &str, body: &Expr, env: &Env) -> Result<(), Fail> {
        let uses = self.uses(body, name, env)?;
        if uses.len() > 1 {
            return Err(self.fail(
                Code::StreamReused,
                "reused",
                format!(
                    "{name} is a stream and is used {} times; a stream is consumed once",
                    uses.len()
                ),
                &uses[1],
            ));
        }
        Ok(())
    }

    /// Whether a symbol in a pattern binds (rather than compares): the
    /// resolver's rule.
    fn pattern_binds(&self, name: &str, env: &Env) -> bool {
        if name == "_" {
            return false;
        }
        if lookup(env, name).is_some() {
            return true;
        }
        if self.defs.contains_key(name) {
            return false;
        }
        !matches!(native(name), Some(n) if n.kind == Kind::Constant)
    }

    fn pattern_binds_name(&self, pattern: &Expr, name: &str, env: &Env) -> bool {
        match pattern {
            Expr::Symbol { name: n, .. } => n == name && self.pattern_binds(n, env),
            Expr::Vector { items, .. } => {
                items.iter().any(|p| self.pattern_binds_name(p, name, env))
            }
            Expr::List { items, .. } => items
                .iter()
                .skip(1)
                .any(|p| self.pattern_binds_name(p, name, env)),
            _ => false,
        }
    }

    /// The uses of the local `name` in `expr`: the spans, where the arms
    /// of an `if` and the cases of a `match` count as alternatives (the
    /// longer arm), and a use inside a nested `fn` is `captured`.
    fn uses(&self, expr: &Expr, name: &str, env: &Env) -> Result<Vec<SourceSpan>, Fail> {
        fn longer(a: Vec<SourceSpan>, b: Vec<SourceSpan>) -> Vec<SourceSpan> {
            if b.len() > a.len() {
                b
            } else {
                a
            }
        }
        match expr {
            Expr::Symbol { name: n, span } if n == name => Ok(vec![span.clone()]),
            Expr::Symbol { .. }
            | Expr::Keyword { .. }
            | Expr::Str { .. }
            | Expr::Num { .. }
            | Expr::Bool { .. }
            | Expr::Null { .. } => Ok(Vec::new()),
            Expr::Vector { items, .. } => {
                let mut all = Vec::new();
                for item in items {
                    all.extend(self.uses(item, name, env)?);
                }
                Ok(all)
            }
            Expr::List { items, .. } => match items.first().and_then(Expr::symbol) {
                Some("fn") if fn_form(items).is_some() => {
                    let (params, body) = fn_form(items).expect("checked");
                    if params.contains(&name) {
                        return Ok(Vec::new());
                    }
                    let inner = self.uses(body, name, env)?;
                    if let Some(first) = inner.first() {
                        return Err(self.fail(
                            Code::StreamReused,
                            "captured",
                            format!(
                                "{name} is a stream and is captured by a fn; a function may run more than once, and a stream is consumed once"
                            ),
                            first,
                        ));
                    }
                    Ok(Vec::new())
                }
                Some("let") if items.len() == 3 => {
                    let Expr::Vector { items: binding, .. } = &items[1] else {
                        return Ok(Vec::new());
                    };
                    let mut all = Vec::new();
                    if let Some(value) = binding.get(1) {
                        all.extend(self.uses(value, name, env)?);
                    }
                    if binding.first().and_then(Expr::symbol) != Some(name) {
                        all.extend(self.uses(&items[2], name, env)?);
                    }
                    Ok(all)
                }
                Some("if") if items.len() == 4 => {
                    let mut all = self.uses(&items[1], name, env)?;
                    let then = self.uses(&items[2], name, env)?;
                    let otherwise = self.uses(&items[3], name, env)?;
                    all.extend(longer(then, otherwise));
                    Ok(all)
                }
                Some("match") if items.len() >= 2 => {
                    let mut all = self.uses(&items[1], name, env)?;
                    let mut cases = Vec::new();
                    for clause in &items[2..] {
                        let Expr::List { items: parts, .. } = clause else {
                            continue;
                        };
                        if parts.len() != 3 || self.pattern_binds_name(&parts[1], name, env) {
                            continue;
                        }
                        cases = longer(cases, self.uses(&parts[2], name, env)?);
                    }
                    all.extend(cases);
                    Ok(all)
                }
                _ => {
                    let mut all = Vec::new();
                    for item in items {
                        all.extend(self.uses(item, name, env)?);
                    }
                    Ok(all)
                }
            },
        }
    }

    /// Whether `expr` names a function the planner can see: a `fn`, a
    /// definition, a native, or a `partial` of one.
    fn is_static_fn(&self, expr: &Expr, env: &Env) -> bool {
        match expr {
            Expr::Symbol { name, .. } => {
                lookup(env, name).is_none()
                    && (self.defs.contains_key(name.as_str())
                        || stdlib_signature(name).is_some()
                        || native(name).is_some_and(|n| n.kind != Kind::Constant))
            }
            Expr::List { items, .. } => match items.first().and_then(Expr::symbol) {
                Some("fn") => fn_form(items).is_some(),
                Some("partial") if lookup(env, "partial").is_none() => {
                    items.get(1).is_some_and(|f| self.is_static_fn(f, env))
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn require_static(&self, what: &str, expr: &Expr, env: &Env) -> Result<(), Fail> {
        if self.is_static_fn(expr, env) {
            Ok(())
        } else {
            Err(self.fail(
                Code::StreamabilityUnknown,
                "dynamic",
                format!(
                    "{what} must be a fn, a definition, a native or a partial of one, so the plan can be analyzed; strict mode refuses a function obtained at run time"
                ),
                expr.span(),
            ))
        }
    }

    /// The types a pattern binds, and a check that a constructor pattern
    /// has the constructor's arity.
    fn pattern(&mut self, pattern: &Expr, matched: &Type, env: &mut Env) -> Result<(), Fail> {
        match pattern {
            Expr::Symbol { name, .. } => {
                if self.pattern_binds(name, env) {
                    env.push(Local {
                        name: Arc::from(name.as_str()),
                        ty: matched.clone(),
                    });
                }
                Ok(())
            }
            Expr::Vector { items, .. } => {
                let item = matched.item().cloned().unwrap_or(Type::Unknown);
                for p in items {
                    self.pattern(p, &item, env)?;
                }
                Ok(())
            }
            Expr::List { items, span } => {
                let head = items.first().and_then(Expr::symbol).unwrap_or("");
                let fields: Vec<Type> = match head {
                    "selected" => vec![Type::Keyword, Type::Value],
                    "schema" => vec![Type::vector(Type::Record)],
                    "row" => vec![Type::vector(Type::Value)],
                    "ready" => vec![Type::vector(Type::Record)],
                    "transition" => vec![Type::Unknown, Type::vector(Type::Unknown)],
                    "entry" => vec![Type::Keyword, Type::Unknown],
                    "key" => vec![Type::String],
                    "scalar" => vec![Type::Value],
                    _ => vec![Type::Unknown; items.len().saturating_sub(1)],
                };
                if let Some(n) = native(head) {
                    if !n.arity.accepts(items.len() - 1) {
                        return Err(self.type_error(
                            "arity",
                            format!(
                                "the pattern ({head} ...) takes {} field(s), got {}",
                                n.arity,
                                items.len() - 1
                            ),
                            span,
                        ));
                    }
                }
                for (p, t) in items[1..].iter().zip(fields.iter()) {
                    self.pattern(p, t, env)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Infer the type of one form.
    fn infer(&mut self, expr: &Expr, env: &mut Env) -> Result<Type, Fail> {
        match expr {
            Expr::Symbol { name, span } => {
                if let Some(t) = lookup(env, name) {
                    return Ok(t.clone());
                }
                match self.global_type(name)? {
                    Some(t) => Ok(t),
                    None => {
                        Err(self.type_error("unknown_name", format!("{name} is not defined"), span))
                    }
                }
            }
            Expr::Keyword { .. } => Ok(Type::Keyword),
            Expr::Str { .. } => Ok(Type::String),
            Expr::Num { .. } => Ok(Type::Number),
            Expr::Bool { .. } => Ok(Type::Bool),
            Expr::Null { .. } => Ok(Type::Null),
            Expr::Vector { items, .. } => {
                let mut item = Type::Never;
                for i in items {
                    let t = self.infer(i, env)?;
                    if t.is_stream_or_source() {
                        return Err(self.type_error(
                            "type_mismatch",
                            format!(
                                "a vector cannot hold a {t}; a stream is used once, where it is"
                            ),
                            i.span(),
                        ));
                    }
                    item = Type::join(&item, &t);
                }
                Ok(Type::vector(if item == Type::Never {
                    Type::Unknown
                } else {
                    item
                }))
            }
            Expr::List { items, span } => self.list(items, span, env),
        }
    }

    fn list(&mut self, items: &[Expr], span: &SourceSpan, env: &mut Env) -> Result<Type, Fail> {
        let Some(head) = items.first() else {
            return Err(self.type_error("type_mismatch", "an empty list is not a call", span));
        };
        let special = head
            .symbol()
            .filter(|name| lookup(env, name).is_none() && !self.defs.contains_key(*name));
        match special {
            Some("fn") if fn_form(items).is_some() => {
                let (params, _) = fn_form(items).expect("checked");
                let params = vec![Type::Unknown; params.len()];
                self.fn_type(items, &params, env)
            }
            Some("let") if items.len() == 3 => {
                let Expr::Vector { items: binding, .. } = &items[1] else {
                    return Err(self.type_error(
                        "bad_let",
                        "let takes one binding [name value] and one body",
                        span,
                    ));
                };
                let (Some(name), Some(value)) =
                    (binding.first().and_then(Expr::symbol), binding.get(1))
                else {
                    return Err(self.type_error(
                        "bad_let",
                        "let takes one binding [name value] and one body",
                        span,
                    ));
                };
                let ty = self.infer(value, env)?;
                if ty.is_affine() {
                    self.affine(name, &items[2], env)?;
                }
                env.push(Local {
                    name: Arc::from(name),
                    ty,
                });
                let result = self.infer(&items[2], env);
                env.pop();
                result
            }
            Some("if") if items.len() == 4 => {
                let condition = self.infer(&items[1], env)?;
                self.expect(
                    "the condition of if",
                    &Type::Bool,
                    &condition,
                    items[1].span(),
                )?;
                let then = self.infer(&items[2], env)?;
                let otherwise = self.infer(&items[3], env)?;
                Ok(Type::join(&then, &otherwise))
            }
            Some("match") if items.len() >= 2 => {
                let matched = self.infer(&items[1], env)?;
                let mut result = Type::Never;
                for clause in &items[2..] {
                    let Expr::List { items: parts, .. } = clause else {
                        continue;
                    };
                    if parts.len() != 3 {
                        continue;
                    }
                    let depth = env.len();
                    self.pattern(&parts[1], &matched, env)?;
                    let body = self.infer(&parts[2], env);
                    env.truncate(depth);
                    result = Type::join(&result, &body?);
                }
                Ok(if result == Type::Never && items.len() == 2 {
                    Type::Unknown
                } else {
                    result
                })
            }
            _ => self.call(items, span, env),
        }
    }

    /// A call: the head's type decides how the arguments are checked.
    fn call(&mut self, items: &[Expr], span: &SourceSpan, env: &mut Env) -> Result<Type, Fail> {
        let head = &items[0];
        let args = &items[1..];
        // A native named directly (and not shadowed) is checked by its
        // own signature, which knows more than a function type can say.
        if let Some(name) = head.symbol() {
            if lookup(env, name).is_none()
                && !self.defs.contains_key(name)
                && stdlib_signature(name).is_none()
            {
                if let Some(n) = native(name) {
                    return self.native_call(n, args, span, env);
                }
            }
        }
        let f = self.infer(head, env)?;
        let mut arg_types = Vec::with_capacity(args.len());
        for arg in args {
            arg_types.push(self.infer(arg, env)?);
        }
        let f = if arg_types.iter().any(Type::is_affine) {
            self.applied(head, &arg_types, env)?.unwrap_or(f)
        } else {
            f
        };
        match f {
            Type::Fn(params, result) => {
                if params.len() != args.len() {
                    return Err(self.type_error(
                        "arity",
                        format!(
                            "{} takes {} argument(s), got {}",
                            describe(head),
                            params.len(),
                            args.len()
                        ),
                        span,
                    ));
                }
                for ((param, actual), arg) in params.iter().zip(&arg_types).zip(args) {
                    self.expect(
                        &format!("an argument of {}", describe(head)),
                        param,
                        actual,
                        arg.span(),
                    )?;
                }
                Ok(*result)
            }
            Type::Unknown | Type::Never => Ok(Type::Unknown),
            other => Err(self.type_error(
                "type_mismatch",
                format!("{} is a {other} and cannot be called", describe(head)),
                head.span(),
            )),
        }
    }

    /// The type of a definition or a `fn` literal applied to arguments of
    /// which one is a stream, a text or the source, typed again with its
    /// parameters bound to the arguments' types (spec 10.2): the parameter
    /// is then affine in the body, as `export`'s `input` is in its own, so
    /// a second use of it (`reused`), a use inside a nested `fn`
    /// (`captured`), or a vector, record or partial application holding it
    /// (`type_mismatch`) is reported where the body does it, however many
    /// definitions the stream was passed through (to [`MAX_APPLIED`]). A
    /// definition typed with its parameters unknown cannot see any of
    /// that. `None` for any other head, a count that does not match (the
    /// call reports it), or past the bound.
    fn applied(&mut self, head: &Expr, args: &[Type], env: &mut Env) -> Result<Option<Type>, Fail> {
        if self.applying >= MAX_APPLIED {
            return Ok(None);
        }
        match head {
            Expr::Symbol { name, .. }
                if lookup(env, name).is_none() && self.defs.contains_key(name.as_str()) =>
            {
                let def = self.defs[name.as_str()].clone();
                let Expr::List { items, .. } = &*def.value else {
                    return Ok(None);
                };
                match fn_form(items) {
                    Some((params, _)) if params.len() == args.len() => {}
                    _ => return Ok(None),
                }
                if let Some((.., t)) = self
                    .applied
                    .iter()
                    .find(|(n, a, _)| **n == **name && a.as_slice() == args)
                {
                    return Ok(Some(t.clone()));
                }
                self.applying += 1;
                let typed = self.fn_type(items, args, &mut Vec::new());
                self.applying -= 1;
                let typed = typed?;
                self.applied
                    .push((def.name.clone(), args.to_vec(), typed.clone()));
                Ok(Some(typed))
            }
            Expr::List { items, .. }
                if lookup(env, "fn").is_none()
                    && fn_form(items).is_some_and(|(params, _)| params.len() == args.len()) =>
            {
                self.applying += 1;
                let typed = self.fn_type(items, args, env);
                self.applying -= 1;
                typed.map(Some)
            }
            _ => Ok(None),
        }
    }

    /// A function argument of a higher-order native: its type, with a
    /// `fn` literal's parameters bound to the item types it will see.
    fn fn_arg(&mut self, arg: &Expr, params: &[Type], env: &mut Env) -> Result<Type, Fail> {
        if let Expr::List { items, .. } = arg {
            if fn_form(items).is_some_and(|(names, _)| names.len() == params.len()) {
                return self.fn_type(items, params, env);
            }
        }
        self.infer(arg, env)
    }

    /// The result type of a function type applied, when known.
    fn result_of(f: &Type) -> Type {
        match f {
            Type::Fn(_, r) => (**r).clone(),
            _ => Type::Unknown,
        }
    }

    fn expect_fn(&self, what: &str, arity: usize, f: &Type, span: &SourceSpan) -> Result<(), Fail> {
        match f {
            Type::Fn(params, _) if params.len() != arity => Err(self.type_error(
                "arity",
                format!(
                    "{what} takes a function of {arity} argument(s), not {}",
                    params.len()
                ),
                span,
            )),
            Type::Fn(..) | Type::Unknown | Type::Never => Ok(()),
            other => Err(self.mismatch(what, &Type::func_of(arity), other, span)),
        }
    }

    /// A function argument's parameters against the types a higher-order
    /// native gives it, as a direct call checks its arguments, so a
    /// definition, or a partial of one, is checked as its eta expansion is
    /// (a `fn` literal's parameters are bound to exactly these types, and
    /// pass). Items typed `Value` pass a parameter that takes data, as they
    /// would in the call ([`Type::accepts`]). A native's parameters are not
    /// typed here (its type is its arity: [`native_type`]), so a native or
    /// a partial of one passes, and its own checks run on the items at run
    /// time. The arity is [`Self::expect_fn`]'s; `span` is the data the
    /// items come from.
    fn expect_params(
        &self,
        what: &str,
        f: &Type,
        given: &[Type],
        span: &SourceSpan,
    ) -> Result<(), Fail> {
        if let Type::Fn(params, _) = f {
            for (param, actual) in params.iter().zip(given) {
                self.expect(&format!("an item given to {what}"), param, actual, span)?;
            }
        }
        Ok(())
    }

    /// A sequence argument: a vector or a stream of items; the items'
    /// type and whether it is a stream.
    fn expect_seq(&self, what: &str, t: &Type, span: &SourceSpan) -> Result<(Type, bool), Fail> {
        match t {
            Type::Vector(item) => Ok(((**item).clone(), false)),
            Type::Stream(item) => Ok(((**item).clone(), true)),
            Type::Unknown | Type::Never => Ok((Type::Unknown, false)),
            Type::JsonEvents => Err(self.type_error(
                "protocol_mismatch",
                format!("{what} must be a vector or a stream of items, not JsonEvents; select or route what the stream should yield, or read its events"),
                span,
            )),
            other => Err(self.mismatch(what, &Type::vector(Type::Unknown), other, span)),
        }
    }

    fn native_call(
        &mut self,
        n: &Native,
        args: &[Expr],
        span: &SourceSpan,
        env: &mut Env,
    ) -> Result<Type, Fail> {
        use Type::*;
        if !n.arity.accepts(args.len()) {
            return Err(self.type_error(
                "arity",
                format!(
                    "{} takes {} argument(s), got {}",
                    n.name,
                    n.arity,
                    args.len()
                ),
                span,
            ));
        }
        let name = n.name;
        // The natives whose function argument is typed by the items it sees.
        match name {
            "map" | "filter" | "concat-map" => {
                let data = self.infer(&args[1], env)?;
                let (item, streaming) =
                    self.expect_seq(&format!("the data of {name}"), &data, args[1].span())?;
                if streaming {
                    self.require_static(
                        &format!("the function of {name} over a stream"),
                        &args[0],
                        env,
                    )?;
                }
                let f = self.fn_arg(&args[0], std::slice::from_ref(&item), env)?;
                self.expect_fn(&format!("the function of {name}"), 1, &f, args[0].span())?;
                self.expect_params(
                    &format!("the function of {name}"),
                    &f,
                    &[item],
                    args[1].span(),
                )?;
                // A text per item is a finite text of a value (the
                // library's `csv-row` maps cells to fields); a live one
                // could only come from a captured stream, which the
                // affine rule refuses, and the runtime refuses the rest.
                let result = Self::result_of(&f);
                return Ok(match name {
                    "map" if streaming => Type::stream(result),
                    "map" => Type::vector(result),
                    "filter" => {
                        self.expect(
                            &format!("the result of the predicate of {name}"),
                            &Bool,
                            &result,
                            args[0].span(),
                        )?;
                        data
                    }
                    _ => {
                        if !result.is_textlike() {
                            return Err(self.mismatch(
                                "the result of the function of concat-map",
                                &Text,
                                &result,
                                args[0].span(),
                            ));
                        }
                        Text
                    }
                });
            }
            "scan-emit" => {
                let init = self.infer(&args[0], env)?;
                if init.is_affine() {
                    return Err(self.type_error(
                        "type_mismatch",
                        format!("the state of scan-emit cannot be a {init}"),
                        args[0].span(),
                    ));
                }
                let source = self.infer(&args[3], env)?;
                let (item, streaming) =
                    self.expect_seq("the stream of scan-emit", &source, args[3].span())?;
                if !streaming && source != Unknown && source != Never {
                    return Err(self.mismatch(
                        "the stream of scan-emit",
                        &Type::stream(Unknown),
                        &source,
                        args[3].span(),
                    ));
                }
                self.require_static("the step of scan-emit", &args[1], env)?;
                self.require_static("the finish of scan-emit", &args[2], env)?;
                let step = self.fn_arg(&args[1], &[Unknown, item.clone()], env)?;
                self.expect_fn("the step of scan-emit", 2, &step, args[1].span())?;
                self.expect_params(
                    "the step of scan-emit",
                    &step,
                    &[Unknown, item],
                    args[3].span(),
                )?;
                self.expect(
                    "the result of the step of scan-emit",
                    &Type::tagged("transition"),
                    &Self::result_of(&step),
                    args[1].span(),
                )?;
                let finish = self.fn_arg(&args[2], &[Unknown], env)?;
                self.expect_fn("the finish of scan-emit", 1, &finish, args[2].span())?;
                self.expect(
                    "the result of the finish of scan-emit",
                    &Type::vector(Unknown),
                    &Self::result_of(&finish),
                    args[2].span(),
                )?;
                return Ok(Type::stream(Unknown));
            }
            _ => {}
        }
        let mut types = Vec::with_capacity(args.len());
        for arg in args {
            types.push(self.infer(arg, env)?);
        }
        let at = |i: usize| args[i].span();
        let t = |i: usize| &types[i];
        // Helpers for the common parameter kinds.
        let data = |this: &Self, i: usize, what: &str| -> Result<(), Fail> {
            let actual = t(i);
            if actual.is_data() {
                Ok(())
            } else {
                Err(this.mismatch(what, &Value, actual, at(i)))
            }
        };
        let no_stream = |this: &Self, i: usize, what: &str| -> Result<(), Fail> {
            if t(i).is_affine() {
                Err(this.type_error(
                    "type_mismatch",
                    format!(
                        "{what} cannot hold a {}; a stream or a text is used once, where it is",
                        t(i)
                    ),
                    at(i),
                ))
            } else {
                Ok(())
            }
        };
        let textlike = |this: &Self, i: usize, what: &str| -> Result<(), Fail> {
            if t(i).is_textlike() {
                Ok(())
            } else {
                Err(this.mismatch(what, &Text, t(i), at(i)))
            }
        };
        Ok(match name {
            "get" => {
                match t(0) {
                    Keyword | String | Unknown | Never => {}
                    other => return Err(self.mismatch("the key of get", &Keyword, other, at(0))),
                }
                match t(1) {
                    Record | Value | Unknown | Never => {}
                    Tagged(tag) if &**tag == "missing" => {}
                    other => return Err(self.mismatch("the data of get", &Record, other, at(1))),
                }
                Unknown
            }
            "get-path" => {
                self.expect("the path of get-path", &Selector, t(0), at(0))?;
                data(self, 1, "the data of get-path")?;
                Value
            }
            "as-path" => {
                data(self, 0, "the data of as-path")?;
                Selector
            }
            "as-vector" => {
                data(self, 0, "the data of as-vector")?;
                Type::vector(Value)
            }
            "record" => {
                for i in 0..args.len() {
                    self.expect("an argument of record", &Type::tagged("entry"), t(i), at(i))?;
                }
                Record
            }
            "entry" => {
                self.expect("the key of entry", &Keyword, t(0), at(0))?;
                no_stream(self, 1, "a record")?;
                Type::tagged("entry")
            }
            "vector" => {
                let mut item = Never;
                for i in 0..args.len() {
                    if t(i).is_stream_or_source() {
                        no_stream(self, i, "a vector")?;
                    }
                    item = Type::join(&item, t(i));
                }
                Type::vector(if item == Never { Unknown } else { item })
            }
            "push" => {
                if t(0).is_stream_or_source() {
                    no_stream(self, 0, "a vector")?;
                }
                self.expect("the vector of push", &Type::vector(Unknown), t(1), at(1))?;
                Type::vector(match t(1) {
                    Vector(item) => Type::join(item, t(0)),
                    _ => Unknown,
                })
            }
            "pop" => {
                self.expect("the vector of pop", &Type::vector(Unknown), t(0), at(0))?;
                match t(0) {
                    Vector(_) => t(0).clone(),
                    _ => Type::vector(Unknown),
                }
            }
            "top" => {
                self.expect("the vector of top", &Type::vector(Unknown), t(0), at(0))?;
                t(0).item().cloned().unwrap_or(Unknown)
            }
            "count" => {
                self.expect("the vector of count", &Type::vector(Unknown), t(0), at(0))?;
                Number
            }
            "keys" => {
                self.expect("the record of keys", &Record, t(0), at(0))?;
                Type::vector(Type::String)
            }
            "length" => {
                self.expect("the string of length", &String, t(0), at(0))?;
                Number
            }
            "compare" => {
                self.expect("the first number of compare", &Number, t(0), at(0))?;
                self.expect("the second number of compare", &Number, t(1), at(1))?;
                Keyword
            }
            "number-class" => {
                self.expect("the number of number-class", &Number, t(0), at(0))?;
                Keyword
            }
            "kind" => {
                if t(0).is_affine() {
                    return Err(self.mismatch("the value of kind", &Value, t(0), at(0)));
                }
                Keyword
            }
            "path" => {
                for i in 0..args.len() {
                    match t(i) {
                        String | Number | Selector | Unknown | Never => {}
                        other => {
                            return Err(self.mismatch("a segment of path", &String, other, at(i)))
                        }
                    }
                }
                Selector
            }
            "property" => {
                self.expect("the name of property", &String, t(0), at(0))?;
                Selector
            }
            "index" => {
                self.expect("the position of index", &Number, t(0), at(0))?;
                Selector
            }
            "compose" => {
                self.expect("the first selector of compose", &Selector, t(0), at(0))?;
                self.expect("the second selector of compose", &Selector, t(1), at(1))?;
                Selector
            }
            "capture" => {
                self.expect("the tag of capture", &Keyword, t(0), at(0))?;
                self.expect("the selector of capture", &Selector, t(1), at(1))?;
                if args.len() == 3 {
                    self.expect("the limit of capture", &Keyword, t(2), at(2))?;
                }
                CaptureSpec
            }
            "route" => {
                self.expect(
                    "the captures of route",
                    &Type::vector(CaptureSpec),
                    t(0),
                    at(0),
                )?;
                self.expect("the input of route", &JsonEvents, t(1), at(1))?;
                Type::stream(Type::tagged("selected"))
            }
            "select" => {
                self.expect("the selector of select", &Selector, t(0), at(0))?;
                self.expect("the input of select", &JsonEvents, t(1), at(1))?;
                Type::stream(Value)
            }
            "events" => {
                self.expect("the input of events", &JsonEvents, t(0), at(0))?;
                Type::events()
            }
            "transition" => {
                no_stream(self, 0, "a state")?;
                self.expect(
                    "the outputs of transition",
                    &Type::vector(Unknown),
                    t(1),
                    at(1),
                )?;
                Type::tagged("transition")
            }
            "partial" => match t(0) {
                Fn(params, result) => {
                    if params.len() < args.len() - 1 {
                        return Err(self.type_error(
                            "arity",
                            format!(
                                "partial supplies {} argument(s) to a function of {}",
                                args.len() - 1,
                                params.len()
                            ),
                            span,
                        ));
                    }
                    for i in 1..args.len() {
                        no_stream(self, i, "a partial application")?;
                        self.expect("an argument of partial", &params[i - 1], t(i), at(i))?;
                    }
                    Type::func(params[args.len() - 1..].to_vec(), (**result).clone())
                }
                Unknown | Never => Unknown,
                other => {
                    return Err(self.mismatch(
                        "the function of partial",
                        &Type::func_of(1),
                        other,
                        at(0),
                    ))
                }
            },
            "join" => {
                self.expect("the separator of join", &String, t(0), at(0))?;
                let (item, _) = self.expect_seq("the items of join", t(1), at(1))?;
                if !item.is_textlike() {
                    return Err(self.mismatch("an item of join", &Text, &item, at(1)));
                }
                Text
            }
            "concat" => {
                for i in 0..args.len() {
                    textlike(self, i, "an item of concat")?;
                }
                Text
            }
            "text" => {
                self.expect("the argument of text", &String, t(0), at(0))?;
                Text
            }
            "replace-text" => {
                self.expect("the literal of replace-text", &String, t(0), at(0))?;
                self.expect("the replacement of replace-text", &String, t(1), at(1))?;
                textlike(self, 2, "the text of replace-text")?;
                Text
            }
            "scalar-text" => {
                self.expect("the options of scalar-text", &Record, t(0), at(0))?;
                data(self, 1, "the cell of scalar-text")?;
                String
            }
            "quoted" => {
                self.expect("the string of quoted", &String, t(0), at(0))?;
                String
            }
            "repeat" => {
                self.expect("the count of repeat", &Number, t(0), at(0))?;
                self.expect("the string of repeat", &String, t(1), at(1))?;
                String
            }
            "string-join" => {
                self.expect("the separator of string-join", &String, t(0), at(0))?;
                self.expect(
                    "the strings of string-join",
                    &Type::vector(Unknown),
                    t(1),
                    at(1),
                )?;
                String
            }
            "fail" => {
                self.expect("the message of fail", &String, t(0), at(0))?;
                Never
            }
            "is-ready" => Bool,
            "require-columns" => Type::vector(Record),
            "schema" => {
                self.expect("the columns of schema", &Type::vector(Unknown), t(0), at(0))?;
                Type::tagged("schema")
            }
            "row" => {
                self.expect("the cells of row", &Type::vector(Unknown), t(0), at(0))?;
                Type::tagged("row")
            }
            "ready" => {
                no_stream(self, 0, "a state")?;
                Type::tagged("ready")
            }
            "selected" => {
                self.expect("the tag of selected", &Keyword, t(0), at(0))?;
                Type::tagged("selected")
            }
            "key" => {
                self.expect("the name of key", &String, t(0), at(0))?;
                Type::tagged("key")
            }
            "scalar" => {
                match t(0) {
                    Null | Bool | Number | String | Value | Unknown | Never => {}
                    other => {
                        return Err(self.type_error(
                            "type_mismatch",
                            format!(
                                "the value of scalar must be null, a boolean, a number or a string, not {other}"
                            ),
                            at(0),
                        ))
                    }
                }
                Type::tagged("scalar")
            }
            "json" => {
                self.expect("the events of json", &JsonEvents, t(0), at(0))?;
                Text
            }
            "csv-table" => {
                self.expect("the options of csv-table", &Record, t(0), at(0))?;
                self.expect(
                    "the table events of csv-table",
                    &Type::table_events(),
                    t(1),
                    at(1),
                )?;
                Type::table_events()
            }
            "records" => {
                self.expect(
                    "the table events of records",
                    &Type::table_events(),
                    t(0),
                    at(0),
                )?;
                JsonEvents
            }
            // The constants are not calls; a call of one is an arity error
            // above. Anything else is a native this table does not know.
            other => {
                return Err(self.type_error(
                    "type_mismatch",
                    format!("{other} is not callable here"),
                    span,
                ))
            }
        })
    }
}

/// A head, for messages.
fn describe(head: &Expr) -> String {
    match head {
        Expr::Symbol { name, .. } => name.clone(),
        other => crate::ast::canonical_form(other),
    }
}

/// The output protocol an `export` result type decides, or why it does
/// not.
fn output_of(export: &Type) -> Result<Output, (Code, &'static str, String)> {
    match export {
        Type::Text | Type::String => Ok(Output::Text),
        Type::JsonEvents => Ok(Output::JsonEvents),
        Type::Stream(item) if Type::TableEvent.accepts(item) => Ok(Output::TableRows),
        Type::Stream(item) => Err((
            Code::DslTypeError,
            "bad_output",
            format!(
                "export answers a Stream<{item}>; render it as a text (join, concat-map), or make table events of it"
            ),
        )),
        Type::Unknown => Err((
            Code::StreamabilityUnknown,
            "unknown_output",
            "the result of export cannot be typed; it must be a text, table events or JSON events".to_string(),
        )),
        other => Err((
            Code::DslTypeError,
            "bad_output",
            format!("export answers a {other}; it must answer a text, table events or JSON events"),
        )),
    }
}

/// Check a resolved program: `export` with its input, then every other
/// definition.
pub fn program(resolved: &Resolved, sources: &Sources) -> Result<Checked, Fail> {
    let mut checker = Checker {
        sources,
        defs: &resolved.defs,
        declared: false,
        memo: HashMap::new(),
        applied: Vec::new(),
        applying: 0,
    };
    let Some(export) = resolved.get("export") else {
        return Err(no_export());
    };
    let Expr::List { items, .. } = &*export.value else {
        return Err(checker.type_error(
            "type_mismatch",
            "export must be a fn [input]",
            &export.span,
        ));
    };
    let Some((params, _)) = fn_form(items) else {
        return Err(checker.type_error(
            "type_mismatch",
            "export must be a fn [input]",
            &export.span,
        ));
    };
    if params.len() != 1 {
        return Err(checker.type_error(
            "arity",
            format!(
                "export takes one parameter, the input, not {}",
                params.len()
            ),
            &export.span,
        ));
    }
    // The definitions `export` reaches, typed first and each after the
    // ones it names, so typing `export` finds them memoized and never
    // recurses along a chain of definitions (the order is the resolver's
    // iterative search; a chain ten thousand long would otherwise nest the
    // checker ten thousand deep).
    let order = resolved.dependency_order();
    let reached: std::collections::HashSet<Arc<str>> =
        resolved.reachable("export").into_iter().collect();
    for name in order.iter().filter(|n| reached.contains(*n)) {
        checker.def_type(name)?;
    }
    let Type::Fn(_, result) = checker.fn_type(items, &[Type::JsonEvents], &mut Vec::new())? else {
        unreachable!("fn_type answers a Fn");
    };
    let export_type = *result;
    let output = output_of(&export_type)
        .map_err(|(code, finer, message)| checker.fail(code, finer, message, &export.span))?;
    checker.memo.insert(
        Arc::from("export"),
        Type::func(vec![Type::JsonEvents], export_type.clone()),
    );
    for name in &order {
        if &**name != "export" {
            checker.def_type(name)?;
        }
    }
    let mut defs = IndexMap::new();
    for name in resolved.defs.keys() {
        if &**name != "export" {
            defs.insert(name.clone(), checker.def_type(name)?);
        }
    }
    Ok(Checked {
        export: export_type,
        output,
        defs,
    })
}

/// The failure of a program with no `export`.
pub(crate) fn no_export() -> Fail {
    Fail::new(
        Code::DslTypeError,
        "no_export: the program has no `def export [input]`",
    )
}

/// Check one standard library file: every definition against its
/// declared signature; a definition without one is a defect of the
/// library.
pub fn stdlib_file(resolved: &Resolved, src: &str) -> Result<(), Fail> {
    let sources = Sources::one(&resolved.file, src);
    let mut checker = Checker {
        sources: &sources,
        defs: &resolved.defs,
        declared: true,
        memo: HashMap::new(),
        applied: Vec::new(),
        applying: 0,
    };
    for def in resolved.defs.values() {
        let Some(declared) = stdlib_signature(&def.name) else {
            return Err(checker.type_error(
                "undeclared",
                format!(
                    "the standard library defines {} without a declared signature",
                    def.name
                ),
                &def.span,
            ));
        };
        checker.check_declared(def, &declared)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::resolve;
    use crate::{desugar, parse_file, stdlib};

    fn check(src: &str) -> Result<Checked, Fail> {
        let forms = desugar::program(parse_file(src, "t.alc").unwrap(), src).unwrap();
        let sources = Sources::one("t.alc", src);
        let resolved = resolve(forms, &sources, &stdlib::outer)?;
        program(&resolved, &sources)
    }

    fn code(src: &str) -> (Code, String, Option<u64>, Option<u64>) {
        let f = check(src).expect_err("should fail");
        let finer = f
            .message
            .split_once(": ")
            .map(|(c, _)| c.to_string())
            .unwrap_or_default();
        (f.code, finer, f.row, f.column)
    }

    const BINDING: &str = "def column-from-meta [source]\n  record\n    entry :label (get \"title\" source)\n    entry :source (as-path (get \"path\" source))\ndef api-binding\n  record\n    entry :columns (path \"response\" \"metadata\" \"fields\")\n    entry :rows (path \"response\" \"payload\" \"deep\" \"records\" each-index)\n    entry :column column-from-meta\n";

    #[test]
    fn the_standard_library_checks_clean_against_its_signatures() {
        let lib = stdlib::stdlib();
        for (resolved, (_, src)) in lib.files.iter().zip(stdlib::SOURCES) {
            stdlib_file(resolved, src).unwrap_or_else(|f| panic!("{f}"));
        }
        for name in lib.names() {
            assert!(stdlib_signature(name).is_some(), "{name} has a signature");
        }
    }

    #[test]
    fn the_worked_example_is_a_text_over_table_events() {
        let src = format!("{BINDING}def api-table [input]\n  table-from-json api-binding input\ndef export [input]\n  pipe input\n    api-table\n    csv csv-options\n");
        let checked = check(&src).unwrap();
        assert_eq!(checked.output, Output::Text);
        assert_eq!(checked.export, Type::Text);
        assert_eq!(checked.defs["api-binding"], Type::Record);
        assert_eq!(
            checked.defs["api-table"],
            Type::func(vec![Type::Unknown], Type::table_events())
        );
        assert_eq!(
            checked.defs["column-from-meta"],
            Type::func(vec![Type::Unknown], Type::Record)
        );
    }

    #[test]
    fn outputs() {
        assert_eq!(
            check("def export [input] input").unwrap().output,
            Output::JsonEvents
        );
        assert_eq!(
            check("def export [input] (json input)").unwrap().output,
            Output::Text
        );
        assert_eq!(
            check("def export [input] \"x\"").unwrap().output,
            Output::Text
        );
        let table = format!("{BINDING}def export [input] (table-from-json api-binding input)");
        assert_eq!(check(&table).unwrap().output, Output::TableRows);
        let records =
            format!("{BINDING}def export [input] (records (table-from-json api-binding input))");
        assert_eq!(check(&records).unwrap().output, Output::JsonEvents);
        // A user's own scan-emit is a stream of unknown items: rendered as
        // a table, checked at run time.
        let scan = "def step [s x] (transition s [(row [x])])\ndef fin [s] [table-end]\ndef export [input] (scan-emit null step fin (select (path each-index) input))";
        assert_eq!(check(scan).unwrap().output, Output::TableRows);
        assert_eq!(
            code("def export [input] (select (path each-index) input)").1,
            "bad_output"
        );
        assert_eq!(code("def export [input] 1").1, "bad_output");
        assert_eq!(code("def export [input] csv-options").1, "bad_output");
        let (c, finer, _, _) = code("def export [input] (get :x csv-options)");
        assert_eq!(
            (c, finer.as_str()),
            (Code::StreamabilityUnknown, "unknown_output")
        );
        assert_eq!(code("def x 1").1, "no_export");
        assert_eq!(code("def export 1").1, "type_mismatch");
        assert_eq!(code("def export [a b] a").1, "arity");
    }

    #[test]
    fn arity_type_and_protocol_mismatches() {
        assert_eq!(code("def export [input] (json input 1)").1, "arity");
        assert_eq!(code("def export [input] (csv csv-options)").1, "arity");
        assert_eq!(
            code("def f [a b] a\ndef export [input] (json (f input))").1,
            "arity"
        );
        assert_eq!(code("def export [input] (text 1 (json input))").1, "arity");
        let (c, finer, row, col) = code("def export [input] (json (text 1))");
        assert_eq!((c, finer.as_str()), (Code::DslTypeError, "type_mismatch"));
        assert_eq!((row, col), (Some(1), Some(32)));
        assert_eq!(
            code("def export [input] (if 1 (json input) (json input))").1,
            "type_mismatch"
        );
        assert_eq!(
            code("def export [input] (csv csv-options input)").1,
            "protocol_mismatch"
        );
        assert_eq!(
            code("def export [input] (table-from-json csv-options (json input))").1,
            "protocol_mismatch"
        );
        assert_eq!(
            code("def export [input] (json (select (path each-index) input))").1,
            "protocol_mismatch"
        );
        assert_eq!(
            code("def export [input] (map (fn [x] x) input)").1,
            "protocol_mismatch"
        );
        assert_eq!(
            code("def export [input] (records input)").1,
            "protocol_mismatch"
        );
        assert_eq!(code("def export [input] (json [input])").1, "type_mismatch");
        assert_eq!(
            code("def export [input] (json (vector input))").1,
            "type_mismatch"
        );
        assert_eq!(
            code("def export [input] (concat (record (entry :x input)))").1,
            "type_mismatch"
        );
        assert_eq!(
            code("def export [input] (concat 1 (json input))").1,
            "type_mismatch"
        );
        assert_eq!(code("def export [input] (1 input)").1, "type_mismatch");
        assert_eq!(
            code("def export [input] (csv-options input)").1,
            "type_mismatch"
        );
        assert_eq!(
            code("def export [input] (concat-map (fn [x] 1) (select (path each-index) input))").1,
            "type_mismatch"
        );
        assert_eq!(
            code("def export [input] (join 1 (select (path each-index) input))").1,
            "type_mismatch"
        );
        assert_eq!(
            code("def export [input] (match input (case (selected :a) \"x\"))").1,
            "arity"
        );
    }

    #[test]
    fn streams_are_affine() {
        let (c, finer, row, col) = code("def export [input] (concat (json input) (json input))");
        assert_eq!((c, finer.as_str()), (Code::StreamReused, "reused"));
        assert_eq!((row, col), (Some(1), Some(47)));
        let (c, finer, _, _) = code("def export [input]\n  concat-map (fn [x] (json input)) (select (path each-index) input)");
        assert_eq!((c, finer.as_str()), (Code::StreamReused, "captured"));
        let (c, finer, _, _) = code("def export [input]\n  let [s (select (path each-index) input)]\n    concat (join \",\" s) (join \";\" s)");
        assert_eq!((c, finer.as_str()), (Code::StreamReused, "reused"));
        // Alternatives are not two uses.
        check("def export [input] (if true (json input) (json input))").unwrap();
        check("def export [input]\n  match 1\n    case 1 (json input)\n    case _ (json input)")
            .unwrap();
        // A let that consumes it once, then uses the binding once.
        check("def export [input]\n  let [s (select (path each-index) input)]\n    join \",\" s")
            .unwrap();
        // Shadowing ends the scope.
        check("def export [input] (concat (json input) (concat-map (fn [input] input) [\"a\"]))")
            .unwrap();
    }

    /// The column of the `n`th (from 1) occurrence of `needle` in the
    /// one-line program `src`, 1-based: where a diagnostic should point.
    fn col(src: &str, needle: &str, n: usize) -> Option<u64> {
        let line = src.lines().find(|l| l.contains(needle))?;
        let at = line.match_indices(needle).nth(n - 1)?.0;
        Some(line[..at].chars().count() as u64 + 1)
    }

    #[test]
    fn a_stream_passed_to_a_definition_is_affine_in_its_body() {
        // Captured by a fn inside the definition: at the capture.
        let src = "def g [s] (concat-map (fn [x] (json s)) [1])\ndef export [input] (g input)";
        let (c, finer, row, column) = code(src);
        assert_eq!((c, finer.as_str()), (Code::StreamReused, "captured"));
        assert_eq!((row, column), (Some(1), col(src, "s)", 1)));
        let src =
            "def g [s] (join \",\" (map (fn [x] (json s)) [1 2]))\ndef export [input] (g input)";
        assert_eq!(code(src).1, "captured");
        // Returned inside a fn, to be applied later.
        let src = "def mk [s] (fn [] s)\ndef export [input]\n  let [g (mk input)]\n    concat (json (g)) \"x\"";
        let (c, finer, row, column) = code(src);
        assert_eq!(
            (c, finer.as_str(), row),
            (Code::StreamReused, "captured", Some(1))
        );
        assert_eq!(column, col(src, "s)", 1));
        // Used twice in the body.
        let src = "def twice [s] (concat (json s) (json s))\ndef export [input] (twice input)";
        let (c, finer, row, column) = code(src);
        assert_eq!(
            (c, finer.as_str(), row),
            (Code::StreamReused, "reused", Some(1))
        );
        assert_eq!(column, col(src, "s)", 2));
        // Held by a partial application or a record, to be used again.
        let partial = "def mk [s] (partial json s)\ndef export [input]\n  let [g (mk input)]\n    concat (g) (g)";
        assert_eq!(code(partial).1, "type_mismatch");
        let record = "def mk [s] (record (entry :s s))\ndef export [input]\n  let [r (mk input)]\n    concat (json (get :s r)) (json (get :s r))";
        assert_eq!(code(record).1, "type_mismatch");
        // Through a chain of definitions, at the use that captures it.
        let src = "def h [t] (concat-map (fn [x] (json t)) [1])\ndef g [s] (h s)\ndef export [input] (g input)";
        let (c, finer, row, _) = code(src);
        assert_eq!(
            (c, finer.as_str(), row),
            (Code::StreamReused, "captured", Some(1))
        );
        // A text passed to a definition that writes it twice.
        let src = "def twice [t] (concat t t)\ndef export [input] (twice (json input))";
        assert_eq!(code(src).1, "reused");
        // A fn literal applied to the stream directly.
        let src = "def export [input] ((fn [s] (concat (json s) (json s))) input)";
        assert_eq!(code(src).1, "reused");
        // Passed once, it is fine, and the call's type is the body's with
        // the stream's type in it.
        assert_eq!(
            check("def g [s] s\ndef export [input] (g input)")
                .unwrap()
                .output,
            Output::JsonEvents
        );
        assert_eq!(
            check("def g [s] (json s)\ndef export [input] (g input)")
                .unwrap()
                .export,
            Type::Text
        );
        // An alternative is not a second use, in a body as in export.
        check("def g [s] (if true (json s) (json s))\ndef export [input] (g input)").unwrap();
    }

    #[test]
    fn a_vector_may_hold_a_finite_text_and_never_a_stream() {
        check("def export [input] (concat (join \",\" [(text \"i\")]) (json input))").unwrap();
        check("def export [input] (concat (join \",\" (vector (text \"i\") \"j\")) (json input))")
            .unwrap();
        check("def export [input] (concat (join \",\" (map text [\"a\" \"b\"])) (json input))")
            .unwrap();
        assert_eq!(code("def export [input] (json [input])").1, "type_mismatch");
        assert_eq!(
            code("def export [input] (join \",\" [(select (path each-index) input)])").1,
            "type_mismatch"
        );
        assert_eq!(
            code("def export [input] (join \",\" (vector (select (path each-index) input)))").1,
            "type_mismatch"
        );
        // A live text is refused where the vector is built, with its name.
        let f =
            crate::compile("def export [input] (join \",\" [(json input)])", "t.alc").unwrap_err();
        assert_eq!(f.code, Code::DslTypeError);
        assert!(f.message.contains("cannot hold a live text"), "{f}");
        let f = crate::compile(
            "def export [input] (join \",\" (vector (json input)))",
            "t.alc",
        )
        .unwrap_err();
        assert!(f.message.contains("cannot hold a live text"), "{f}");
        assert_eq!(
            crate::compile(
                "def export [input] (concat (join \",\" [(text \"i\") \"j\"]) (json input))",
                "t.alc"
            )
            .map(|p| p.output()),
            Ok(Output::Text)
        );
    }

    #[test]
    fn a_chain_of_definitions_is_typed_without_nesting_the_checker() {
        // Ten thousand definitions each naming the next: typed in
        // dependency order, each finds the one it names already typed.
        let mut src = String::from("def a0 1\n");
        for i in 1..10_000 {
            src.push_str(&format!("def a{i} a{}\n", i - 1));
        }
        src.push_str("def export [input] (if false (let [y a9999] (json input)) (json input))\n");
        let checked = check(&src).unwrap();
        assert_eq!(checked.defs["a9999"], Type::Number);
        // A stream passed down five hundred definitions is followed to
        // MAX_APPLIED of them and typed from there on as the body inferred,
        // so the checker's recursion stays bounded.
        let mut src = String::from("def f0 [s] (json s)\n");
        for i in 1..500 {
            src.push_str(&format!("def f{i} [s] (f{} s)\n", i - 1));
        }
        src.push_str("def export [input] (f499 input)\n");
        check(&src).unwrap();
    }

    #[test]
    fn strict_mode_wants_static_functions_over_streams() {
        let (c, finer, _, _) = code("def go [f input] (concat-map f (select (path each-index) input))\ndef export [input] (go text input)");
        assert_eq!((c, finer.as_str()), (Code::StreamabilityUnknown, "dynamic"));
        assert_eq!(code("def export [input] (scan-emit null (get :f csv-options) (fn [s] []) (select (path each-index) input))").1, "dynamic");
        // A def, a native, a fn and a partial of a def are static.
        check("def step [b s x] (transition s [x])\ndef fin [s] []\ndef export [input] (join \",\" (scan-emit null (partial step 1) fin (select (path each-index) input)))").unwrap();
        check("def export [input] (concat-map text (select (path each-index) input))").unwrap();
        // Over a vector anything goes.
        check("def export [input] (concat-map (get :f csv-options) [\"a\"])").unwrap();
        // A step that answers something other than a transition.
        assert_eq!(code("def step [s x] [x]\ndef fin [s] []\ndef export [input] (join \",\" (scan-emit null step fin (select (path each-index) input)))").1, "type_mismatch");
        assert_eq!(code("def step [s] s\ndef fin [s] []\ndef export [input] (join \",\" (scan-emit null step fin (select (path each-index) input)))").1, "arity");
    }

    /// A definition given to `map`, `filter` or `concat-map` by name, or a
    /// partial of one, is checked as its eta expansion is: its parameter
    /// against the items the data holds, as a direct call checks its
    /// arguments, with the failure at the data.
    #[test]
    fn a_definition_over_items_is_checked_as_its_eta_expansion() {
        let (c, finer, row, col) =
            code("def bad (map public-column [1])\ndef export [input] (json input)");
        assert_eq!((c, finer.as_str()), (Code::DslTypeError, "type_mismatch"));
        assert_eq!((row, col), (Some(1), Some(28)));
        // Without the parameter check this is the predicate's result (a
        // Record, not a Bool) at 1:17; the item is refused first, at the
        // data.
        let f = check("def bad (filter public-column [1])\ndef export [input] (json input)")
            .unwrap_err();
        assert!(
            f.message.starts_with(
                "type_mismatch: an item given to the function of filter must be Record, not Number"
            ),
            "{f}"
        );
        assert_eq!((f.row, f.column), (Some(1), Some(31)));
        // A partial of a library definition, over a stream.
        assert_eq!(code("def export [input]\n  concat-map (partial csv-row csv-options) (map (fn [v] 1) (select (path each-index) input))").1, "type_mismatch");
        assert_eq!(code("def export [input]\n  concat-map (fn [r] (scalar-text csv-options (get :label r))) (map public-column (map (fn [v] \"s\") (select (path each-index) input)))").1, "type_mismatch");
        // The named form and its eta expansion agree, both ways.
        for f in ["public-column", "(fn [x] (public-column x))"] {
            assert_eq!(
                code(&format!(
                    "def bad (map {f} [1])\ndef export [input] (json input)"
                ))
                .1,
                "type_mismatch",
                "{f}"
            );
            check(&format!(
                "def ok (map {f} [(record (entry :label \"x\"))])\ndef export [input] (json input)"
            ))
            .unwrap();
            check(&format!(
                "def ok [xs] (map {f} xs)\ndef export [input] (json input)"
            ))
            .unwrap();
            // Items typed Value, as every selection's are, may be records:
            // both forms pass, and the runtime checks each item.
            check(&format!(
                "def cols [input] (map {f} (select (path \"cols\" each-index) input))\ndef export [input] (join \",\" (map (fn [c] (get :label c)) (cols input)))"
            ))
            .unwrap_or_else(|e| panic!("{f}: {e}"));
        }
        for f in [
            "(partial csv-row csv-options)",
            "(fn [cells] (csv-row csv-options cells))",
        ] {
            check(&format!(
                "def export [input]\n  concat-map {f} (select (path \"rows\" each-index) input)"
            ))
            .unwrap_or_else(|e| panic!("{f}: {e}"));
        }
        // A native's parameters are untyped here: a partial of one passes,
        // and its own check runs on the items at run time, where its eta
        // expansion, a call, is checked now.
        check("def ok (map (partial get :a) [1])\ndef export [input] (json input)").unwrap();
        assert_eq!(
            code("def bad (map (fn [x] (get :a x)) [1])\ndef export [input] (json input)").1,
            "type_mismatch"
        );
    }

    /// `events` takes the input and yields a stream of items: affine like
    /// the input, not JSON events, not table events, not an output; the
    /// stack operators take a vector and answer what it held.
    #[test]
    fn events_and_the_stack_operators() {
        let render = "def export [input]\n  join \"\"\n    map\n      fn [e]\n        match e\n          case (key n) (quoted n)\n          case (scalar v) (scalar-text csv-options v)\n          case object-start \"{\"\n          case _ \"\"\n      events input";
        assert_eq!(check(render).unwrap().export, Type::Text);
        let keep = "def keep [s e] (transition (push e s) [])\ndef fin [s] [(repeat (count s) \"  \") (quoted (top s))]\ndef export [input] (join \"\" (scan-emit [] keep fin (events input)))";
        let checked = check(keep).unwrap();
        assert_eq!(checked.output, Output::Text);
        assert_eq!(
            checked.defs["fin"],
            Type::func(vec![Type::Unknown], Type::vector(Type::String))
        );
        assert_eq!(
            check("def s (push :b [:a])\ndef n (count s)\ndef t (top s)\ndef p (pop s)\ndef export [input] (json input)")
                .unwrap()
                .defs,
            IndexMap::from([
                (Arc::from("s"), Type::vector(Type::Keyword)),
                (Arc::from("n"), Type::Number),
                (Arc::from("t"), Type::Keyword),
                (Arc::from("p"), Type::vector(Type::Keyword)),
            ])
        );
        assert_eq!(code("def export [input] (events input)").1, "bad_output");
        // A stream of events reaches a taker of JSON events; a stream of
        // values does not, nor do events reach a taker of table events.
        assert_eq!(
            check("def export [input] (json (events input))")
                .unwrap()
                .export,
            Type::Text
        );
        assert_eq!(
            code("def export [input] (json (select (path each-index) input))").1,
            "protocol_mismatch"
        );
        assert_eq!(
            code("def export [input] (csv csv-options (events input))").1,
            "protocol_mismatch"
        );
        assert_eq!(
            code("def export [input] (events (select (path each-index) input))").1,
            "protocol_mismatch"
        );
        let (c, finer, _, _) = code("def export [input] (concat (json input) (join \"\" (map (fn [e] \"\") (events input))))");
        assert_eq!((c, finer.as_str()), (Code::StreamReused, "reused"));
        assert_eq!(
            code("def export [input] (let [x (push (events input) [])] \"done\")").1,
            "type_mismatch"
        );
        assert_eq!(
            code("def export [input] (let [x (count 1)] (json input))").1,
            "type_mismatch"
        );
        assert_eq!(
            code("def export [input] (let [x (top csv-options)] (json input))").1,
            "type_mismatch"
        );
        assert_eq!(
            code("def export [input] (let [x (quoted 1)] (json input))").1,
            "type_mismatch"
        );
        assert_eq!(
            code("def export [input] (let [x (repeat \"a\" 1)] (json input))").1,
            "type_mismatch"
        );
        assert_eq!(
            code("def export [input] (let [x (key :a)] (json input))").1,
            "type_mismatch"
        );
        assert_eq!(
            code("def export [input] (let [x (scalar csv-options)] (json input))").1,
            "type_mismatch"
        );
        assert_eq!(
            code("def export [input] (join \"\" (map (fn [e] (match e (case (key a b) a) (case _ \"\"))) (events input)))").1,
            "arity"
        );
    }

    #[test]
    fn patterns_type_their_bindings() {
        // `cells` of a row is a vector of values: a text of them joins.
        check("def export [input]\n  concat-map\n    fn [e]\n      match e\n        case (row cells) (join \",\" (map (fn [c] (scalar-text csv-options c)) cells))\n        case _ \"\"\n    select (path each-index) input").unwrap();
        // A constant pattern compares; a fresh symbol binds.
        check("def export [input]\n  concat-map\n    fn [e]\n      match e\n        case table-end \"end\"\n        case other (scalar-text csv-options other)\n    select (path each-index) input").unwrap();
    }
}
