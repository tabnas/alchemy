//! Runtime values (design brief section 4.3; spec sections 10.2 and 10.3).
//!
//! Every value is `Send` and `Sync`: aless runs a pipeline on its parse
//! thread and transduce's `run_owned` needs a `Sink + Send + 'static`, so
//! sharing goes through `Arc`, never `Rc`, and nothing here has interior
//! mutability. A [`Val`] is cheap to clone: vectors, records, functions,
//! selectors and plans are shared behind an `Arc`.
//!
//! Streams and texts are **plans**. Evaluating `route`, `scan-emit`, `map`
//! over a stream, `concat`, `join` or `csv` builds a [`Plan`] and consumes
//! nothing; the runtime lowers the plan to a chain of transduce and render
//! sinks once, when the host asks for the program's sink (`interp.rs`).
//! A plan is live when it reaches the host's input ([`Plan::Input`]); a
//! finite text (a literal, a join over a vector) is a plan too, written
//! synchronously wherever it lands.
//!
//! A transduce [`Datum`] converts to a `Val` and back, so `get`,
//! `get-path`, `as-vector` and `as-path` work on captured JSON values and
//! on a program's own records alike. A value the source had no member for
//! is the constant tagged value `missing` ([`Val::missing`]), distinct from
//! `null`, as the spec asks (section 18.3); a policy maps it later.

use std::fmt;
use std::sync::Arc;

use indexmap::IndexMap;
use tabnas_transduce::{CaptureSpec, Code, Datum, Fail, Segment, Selector, Step};

use crate::ast::{Expr, SourceSpan};
use crate::stdlib::registry::Native;

/// The tag of the constant tagged value a `get` or `get-path` answers for
/// an absent member.
pub const MISSING: &str = "missing";

/// One runtime value.
#[derive(Clone)]
pub enum Val {
    Null,
    Bool(bool),
    /// A number as the source or the program spelled it, when known: a
    /// renderer writes the lexeme, so `50.25` stays `50.25`.
    Num {
        value: f64,
        lexeme: Option<Arc<str>>,
    },
    Str(Arc<str>),
    Keyword(Arc<str>),
    Vector(Arc<[Val]>),
    /// Fields in construction order; keys are the keyword names.
    Record(Arc<IndexMap<Arc<str>, Val>>),
    Fn(Func),
    Selector(Arc<Selector>),
    CaptureSpec(Arc<CaptureSpec>),
    /// A constructor's value: `schema`, `row`, `table-end`, `no-schema`,
    /// `ready`, `selected`, `transition`, `entry`, `missing`.
    Tagged {
        tag: Arc<str>,
        fields: Arc<[Val]>,
    },
    /// A single-use stream: a plan producing items, table events or JSON
    /// events.
    Stream(Arc<Plan>),
    /// A single-use text: a plan producing fragments.
    Text(Arc<Plan>),
}

/// A function value.
#[derive(Clone)]
pub enum Func {
    Closure(Arc<Closure>),
    Native(&'static Native),
    Partial(Arc<Partial>),
}

/// Which global scope a closure's free names resolve in: a standard
/// library definition sees the library and the natives, a program's
/// definition sees the program first. Lexical, so a program's `csv-row`
/// never hijacks the library's `csv`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    Program,
    Stdlib,
}

/// A `fn` value: its parameters, its body and the environment it closed
/// over. `name` is the `def` it came from, for messages.
pub struct Closure {
    pub name: Option<Arc<str>>,
    pub params: Vec<Arc<str>>,
    pub body: Arc<Expr>,
    pub env: Env,
    pub scope: Scope,
    pub span: SourceSpan,
}

/// `partial f a b`: `f` with its first arguments supplied.
pub struct Partial {
    pub f: Func,
    pub args: Vec<Val>,
}

impl Func {
    /// The arguments the function still takes, when that is fixed.
    pub fn arity(&self) -> Option<usize> {
        match self {
            Func::Closure(c) => Some(c.params.len()),
            Func::Native(n) => n.arity.exact(),
            Func::Partial(p) => p.f.arity().map(|n| n.saturating_sub(p.args.len())),
        }
    }

    /// A name for messages.
    pub fn describe(&self) -> String {
        match self {
            Func::Closure(c) => match &c.name {
                Some(name) => format!("fn {name}"),
                None => format!("fn [{}]", c.params.join(" ")),
            },
            Func::Native(n) => n.name.to_string(),
            Func::Partial(p) => format!("partial {}", p.f.describe()),
        }
    }
}

impl fmt::Debug for Func {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}>", self.describe())
    }
}

/// The local environment: a persistent chain of bindings, shared by the
/// closures that captured it.
#[derive(Clone, Default)]
pub struct Env(Option<Arc<Frame>>);

struct Frame {
    name: Arc<str>,
    value: Val,
    next: Env,
}

impl Env {
    pub fn new() -> Env {
        Env(None)
    }

    /// This environment with one more binding, innermost.
    pub fn bind(&self, name: Arc<str>, value: Val) -> Env {
        Env(Some(Arc::new(Frame {
            name,
            value,
            next: self.clone(),
        })))
    }

    pub fn get(&self, name: &str) -> Option<&Val> {
        let mut here = self.0.as_deref();
        while let Some(frame) = here {
            if &*frame.name == name {
                return Some(&frame.value);
            }
            here = frame.next.0.as_deref();
        }
        None
    }

    /// Whether `name` is bound here.
    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// The innermost binding's value and the environment outside it.
    pub fn frame(&self) -> Option<(&Val, &Env)> {
        self.0.as_deref().map(|f| (&f.value, &f.next))
    }
}

impl fmt::Debug for Env {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut names = Vec::new();
        let mut here = self.0.as_deref();
        while let Some(frame) = here {
            names.push(frame.name.to_string());
            here = frame.next.0.as_deref();
        }
        write!(f, "Env[{}]", names.join(" "))
    }
}

/// A finite or single-use sequence an operator was given.
#[derive(Clone, Debug)]
pub enum Seq {
    Vector(Arc<[Val]>),
    Stream(Arc<Plan>),
}

/// A stream or text, as a description of how to produce it.
#[derive(Clone, Debug)]
pub enum Plan {
    /// The host's `JsonEvents/1`.
    Input,
    /// `route specs input`: a stream of `selected` values.
    Route {
        specs: Vec<CaptureSpec>,
        source: Arc<Plan>,
    },
    /// `select selector input`: a stream of the selected values.
    Select {
        selector: Selector,
        source: Arc<Plan>,
    },
    /// `scan-emit init step finish stream`.
    ScanEmit {
        init: Val,
        step: Func,
        finish: Func,
        source: Arc<Plan>,
        at: SourceSpan,
    },
    Map {
        f: Func,
        source: Arc<Plan>,
        at: SourceSpan,
    },
    Filter {
        f: Func,
        source: Arc<Plan>,
        at: SourceSpan,
    },
    /// The standard table transducer, run natively: `TableRows/1` from
    /// `JsonEvents/1`.
    TableFromJson {
        binding: Val,
        source: Arc<Plan>,
        at: SourceSpan,
    },
    /// `records table-events`: `JsonEvents/1` from `TableRows/1`.
    Records { source: Arc<Plan> },
    /// `csv-table options events`: the tagged table events, validated as
    /// the CSV renderer validates them, passed on unchanged.
    CsvTable { options: Val, source: Arc<Plan> },
    /// A literal text.
    Lit(Arc<str>),
    /// `concat items...`: each a string or a text; `live` is the position
    /// of the one item that reaches the input, found once when the plan is
    /// built ([`Plan::concat`]), so asking whether a concat is live never
    /// walks its items again: a program that nests `concat` over shared
    /// definitions builds a plan whose items, unshared, are exponentially
    /// many.
    Concat {
        items: Vec<Val>,
        live: Option<usize>,
    },
    /// `join separator items`.
    Join { sep: Arc<str>, items: Seq },
    /// `concat-map f items`: `f` answers a string or a text per item.
    ConcatMap { f: Func, items: Seq, at: SourceSpan },
    /// `replace-text from to text`: `source` is a string or a text.
    Replace {
        from: Arc<str>,
        to: Arc<str>,
        source: Val,
    },
    /// The standard CSV renderer, run natively.
    Csv { options: Val, source: Arc<Plan> },
    /// `json events`.
    Json { source: Arc<Plan> },
}

impl Plan {
    /// A `concat` of `items`, with its live item found once.
    pub fn concat(items: Vec<Val>) -> Plan {
        let live = items.iter().position(Val::is_live);
        Plan::Concat { items, live }
    }

    /// Whether the plan reaches the host's input: a live plan runs as the
    /// events arrive; a plan that does not is finite and is written whole.
    pub fn is_live(&self) -> bool {
        match self {
            Plan::Input => true,
            Plan::Route { source, .. }
            | Plan::Select { source, .. }
            | Plan::ScanEmit { source, .. }
            | Plan::Map { source, .. }
            | Plan::Filter { source, .. }
            | Plan::TableFromJson { source, .. }
            | Plan::Records { source }
            | Plan::CsvTable { source, .. }
            | Plan::Csv { source, .. }
            | Plan::Json { source } => source.is_live(),
            Plan::Lit(_) => false,
            Plan::Concat { live, .. } => live.is_some(),
            Plan::Join { items, .. } | Plan::ConcatMap { items, .. } => {
                matches!(items, Seq::Stream(_))
            }
            Plan::Replace { source, .. } => source.is_live(),
        }
    }

    /// Whether the plan is a text rather than a stream.
    pub fn is_text(&self) -> bool {
        matches!(
            self,
            Plan::Lit(_)
                | Plan::Concat { .. }
                | Plan::Join { .. }
                | Plan::ConcatMap { .. }
                | Plan::Replace { .. }
                | Plan::Csv { .. }
                | Plan::Json { .. }
        )
    }

    /// The protocol the plan produces, when it is a stream: `JsonEvents`,
    /// `TableRows` (natively), or `Items` for a stream of values whose
    /// shape the runtime learns item by item.
    pub fn protocol(&self) -> Protocol {
        match self {
            Plan::Input | Plan::Records { .. } => Protocol::JsonEvents,
            Plan::TableFromJson { .. } => Protocol::TableRows,
            Plan::Route { .. }
            | Plan::Select { .. }
            | Plan::ScanEmit { .. }
            | Plan::Map { .. }
            | Plan::Filter { .. }
            | Plan::CsvTable { .. } => Protocol::Items,
            _ => Protocol::Text,
        }
    }
}

/// What a plan produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    JsonEvents,
    TableRows,
    Items,
    Text,
}

impl Val {
    pub fn str(s: &str) -> Val {
        Val::Str(Arc::from(s))
    }

    pub fn keyword(s: &str) -> Val {
        Val::Keyword(Arc::from(s))
    }

    pub fn num(value: f64) -> Val {
        Val::Num {
            value,
            lexeme: None,
        }
    }

    pub fn vector(items: Vec<Val>) -> Val {
        Val::Vector(Arc::from(items))
    }

    pub fn tagged(tag: &str, fields: Vec<Val>) -> Val {
        Val::Tagged {
            tag: Arc::from(tag),
            fields: Arc::from(fields),
        }
    }

    /// The constant a lookup answers for an absent member.
    pub fn missing() -> Val {
        Val::tagged(MISSING, Vec::new())
    }

    pub fn is_missing(&self) -> bool {
        matches!(self, Val::Tagged { tag, fields } if &**tag == MISSING && fields.is_empty())
    }

    /// Whether two values are the same retained value, not merely equal:
    /// one allocation behind both. A scalar is never the same as anything,
    /// which only costs a re-measure where this is asked.
    pub fn same(&self, other: &Val) -> bool {
        match (self, other) {
            (Val::Vector(a), Val::Vector(b)) => Arc::ptr_eq(a, b),
            (Val::Record(a), Val::Record(b)) => Arc::ptr_eq(a, b),
            (Val::Tagged { fields: a, .. }, Val::Tagged { fields: b, .. }) => Arc::ptr_eq(a, b),
            (Val::Selector(a), Val::Selector(b)) => Arc::ptr_eq(a, b),
            (Val::Stream(a), Val::Stream(b)) | (Val::Text(a), Val::Text(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Whether this value holds a live stream or text: the one-shot
    /// resources a vector may not hide and a `fn` may not capture.
    pub fn is_live(&self) -> bool {
        match self {
            Val::Stream(plan) | Val::Text(plan) => plan.is_live(),
            _ => false,
        }
    }

    /// A one-word name of the value's kind, for messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Val::Null => "null",
            Val::Bool(_) => "a boolean",
            Val::Num { .. } => "a number",
            Val::Str(_) => "a string",
            Val::Keyword(_) => "a keyword",
            Val::Vector(_) => "a vector",
            Val::Record(_) => "a record",
            Val::Fn(_) => "a function",
            Val::Selector(_) => "a selector",
            Val::CaptureSpec(_) => "a capture",
            Val::Tagged { .. } if self.is_missing() => "missing",
            Val::Tagged { .. } => "a tagged value",
            Val::Stream(_) => "a stream",
            Val::Text(_) => "a text",
        }
    }

    /// What a live value is, for a message that refuses to hold it: a
    /// text is named live, since a finite one could have been held.
    pub fn live_kind(&self) -> &'static str {
        match self {
            Val::Text(_) => "a live text",
            other => other.kind(),
        }
    }

    /// From a retained transduce value. Numbers keep their lexemes.
    pub fn from_datum(d: &Datum) -> Val {
        match d {
            Datum::Null => Val::Null,
            Datum::Bool(b) => Val::Bool(*b),
            Datum::Number { value, lexeme } => Val::Num {
                value: *value,
                lexeme: lexeme.as_deref().map(Arc::from),
            },
            Datum::String(s) => Val::Str(Arc::from(&**s)),
            Datum::Array(items) => Val::Vector(items.iter().map(Val::from_datum).collect()),
            Datum::Object(members) => Val::Record(Arc::new(
                members
                    .iter()
                    .map(|(k, v)| (Arc::from(&**k), Val::from_datum(v)))
                    .collect(),
            )),
        }
    }

    /// [`Val::from_datum`] for a value the caller owns.
    pub fn from_owned_datum(d: Datum) -> Val {
        match d {
            Datum::Array(items) => {
                Val::Vector(items.into_iter().map(Val::from_owned_datum).collect())
            }
            Datum::Object(members) => Val::Record(Arc::new(
                members
                    .into_iter()
                    .map(|(k, v)| (Arc::from(&*k), Val::from_owned_datum(v)))
                    .collect(),
            )),
            other => Val::from_datum(&other),
        }
    }

    /// As a retained transduce value: the data kinds convert, a keyword
    /// becomes its name, `missing` becomes `null`, and a function, a
    /// selector, a capture, a stream or a text has no data form.
    pub fn to_datum(&self) -> Result<Datum, Fail> {
        Ok(match self {
            Val::Null => Datum::Null,
            Val::Bool(b) => Datum::Bool(*b),
            Val::Num { value, lexeme } => Datum::Number {
                value: *value,
                lexeme: lexeme.as_deref().map(Box::from),
            },
            Val::Str(s) | Val::Keyword(s) => Datum::String(Box::from(&**s)),
            Val::Vector(items) => {
                Datum::Array(items.iter().map(Val::to_datum).collect::<Result<_, _>>()?)
            }
            Val::Record(fields) => Datum::Object(
                fields
                    .iter()
                    .map(|(k, v)| Ok((Box::from(&**k), v.to_datum()?)))
                    .collect::<Result<_, Fail>>()?,
            ),
            Val::Tagged { .. } if self.is_missing() => Datum::Null,
            other => return Err(type_error(format!("{} has no data form", other.kind()))),
        })
    }

    /// The field of a record, or `missing`.
    pub fn field(&self, key: &str) -> Option<Val> {
        match self {
            Val::Record(fields) => Some(fields.get(key).cloned().unwrap_or_else(Val::missing)),
            _ => None,
        }
    }

    /// The value at a concrete path below this one, walking records by
    /// key and vectors by index; `missing` where the path leaves the data.
    pub fn get_path(&self, segments: &[Segment]) -> Val {
        let mut here = self;
        for segment in segments {
            here = match (segment, here) {
                (Segment::Key(k), Val::Record(fields)) => match fields.get(&**k) {
                    Some(v) => v,
                    None => return Val::missing(),
                },
                (Segment::Index(i), Val::Vector(items)) => match items.get(*i) {
                    Some(v) => v,
                    None => return Val::missing(),
                },
                _ => return Val::missing(),
            };
        }
        here.clone()
    }

    /// The compact JSON text of a data value, keeping number lexemes.
    pub fn to_json_text(&self) -> Result<String, Fail> {
        Ok(self.to_datum()?.to_string())
    }
}

/// A selector that names one location, as segments; `None` when it names
/// many (`each-index`, `each-member`).
pub fn selector_segments(selector: &Selector) -> Option<Vec<Segment>> {
    selector
        .steps()
        .iter()
        .map(|step| match step {
            Step::Property(k) => Some(Segment::Key(k.clone())),
            Step::Index(i) => Some(Segment::Index(*i)),
            Step::EachIndex | Step::EachMember => None,
        })
        .collect()
}

/// A `DSL_TYPE_ERROR` found while running, with the `type_mismatch` finer
/// code the checker also uses.
pub fn type_error(message: impl fmt::Display) -> Fail {
    Fail::new(Code::DslTypeError, format!("type_mismatch: {message}"))
}

impl PartialEq for Val {
    /// Structural equality of data; numbers by value; functions, streams
    /// and texts never equal.
    fn eq(&self, other: &Val) -> bool {
        match (self, other) {
            (Val::Null, Val::Null) => true,
            (Val::Bool(a), Val::Bool(b)) => a == b,
            (Val::Num { value: a, .. }, Val::Num { value: b, .. }) => a == b,
            (Val::Str(a), Val::Str(b)) | (Val::Keyword(a), Val::Keyword(b)) => a == b,
            (Val::Vector(a), Val::Vector(b)) => a == b,
            (Val::Record(a), Val::Record(b)) => {
                a.len() == b.len() && a.iter().all(|(k, v)| b.get(k) == Some(v))
            }
            (Val::Selector(a), Val::Selector(b)) => a == b,
            (Val::CaptureSpec(a), Val::CaptureSpec(b)) => a == b,
            (Val::Tagged { tag: a, fields: fa }, Val::Tagged { tag: b, fields: fb }) => {
                a == b && fa == fb
            }
            _ => false,
        }
    }
}

impl fmt::Debug for Val {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Val::Null => f.write_str("null"),
            Val::Bool(b) => write!(f, "{b}"),
            Val::Num { value, lexeme } => match lexeme {
                Some(l) => f.write_str(l),
                None => write!(f, "{value}"),
            },
            Val::Str(s) => write!(f, "{s:?}"),
            Val::Keyword(k) => write!(f, ":{k}"),
            Val::Vector(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" ")?;
                    }
                    write!(f, "{item:?}")?;
                }
                f.write_str("]")
            }
            Val::Record(fields) => {
                f.write_str("(record")?;
                for (k, v) in fields.iter() {
                    write!(f, " (entry :{k} {v:?})")?;
                }
                f.write_str(")")
            }
            Val::Fn(func) => write!(f, "{func:?}"),
            Val::Selector(s) => write!(f, "<selector {s}>"),
            Val::CaptureSpec(c) => write!(f, "<capture :{} {}>", c.tag, c.selector),
            Val::Tagged { tag, fields } => {
                if fields.is_empty() {
                    return f.write_str(tag);
                }
                write!(f, "({tag}")?;
                for field in fields.iter() {
                    write!(f, " {field:?}")?;
                }
                f.write_str(")")
            }
            Val::Stream(plan) => write!(f, "<stream {}>", plan_name(plan)),
            Val::Text(plan) => write!(f, "<text {}>", plan_name(plan)),
        }
    }
}

/// The operator at the root of a plan, for messages.
pub fn plan_name(plan: &Plan) -> &'static str {
    match plan {
        Plan::Input => "input",
        Plan::Route { .. } => "route",
        Plan::Select { .. } => "select",
        Plan::ScanEmit { .. } => "scan-emit",
        Plan::Map { .. } => "map",
        Plan::Filter { .. } => "filter",
        Plan::TableFromJson { .. } => "table-from-json",
        Plan::Records { .. } => "records",
        Plan::CsvTable { .. } => "csv-table",
        Plan::Lit(_) => "text",
        Plan::Concat { .. } => "concat",
        Plan::Join { .. } => "join",
        Plan::ConcatMap { .. } => "concat-map",
        Plan::Replace { .. } => "replace-text",
        Plan::Csv { .. } => "csv",
        Plan::Json { .. } => "json",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn values_plans_and_environments_are_send_and_sync() {
        assert_send_sync::<Val>();
        assert_send_sync::<Plan>();
        assert_send_sync::<Env>();
        assert_send_sync::<Func>();
    }

    #[test]
    fn a_datum_round_trips_with_its_lexemes() {
        let d = Datum::from_json(&serde_json::json!({"a": [1, "x", null, true], "b": {"c": 2.5}}));
        let v = Val::from_datum(&d);
        assert_eq!(v.to_datum().unwrap(), d);
        let big = Datum::Number {
            value: 1.5,
            lexeme: Some("1.50".into()),
        };
        assert_eq!(Val::from_datum(&big).to_json_text().unwrap(), "1.50");
        assert_eq!(
            v.to_json_text().unwrap(),
            r#"{"a":[1,"x",null,true],"b":{"c":2.5}}"#
        );
    }

    #[test]
    fn get_path_walks_records_and_vectors_and_answers_missing() {
        let v = Val::from_datum(&Datum::from_json(&serde_json::json!({"a": [{"b": 1}]})));
        let p = [Segment::key("a"), Segment::Index(0), Segment::key("b")];
        assert_eq!(v.get_path(&p), Val::num(1.0));
        assert!(v.get_path(&[Segment::key("z")]).is_missing());
        assert!(v.get_path(&[Segment::Index(0)]).is_missing());
        assert_eq!(v.get_path(&[]), v);
        assert!(v.field("z").unwrap().is_missing());
        assert!(Val::Null.field("z").is_none());
    }

    #[test]
    fn environments_shadow_innermost_first() {
        let env = Env::new().bind(Arc::from("x"), Val::num(1.0));
        let inner = env.bind(Arc::from("x"), Val::num(2.0));
        assert_eq!(env.get("x"), Some(&Val::num(1.0)));
        assert_eq!(inner.get("x"), Some(&Val::num(2.0)));
        assert!(!inner.has("y"));
    }

    #[test]
    fn liveness_follows_the_input() {
        let live = Plan::Map {
            f: Func::Native(crate::stdlib::registry::native("text").unwrap()),
            source: Arc::new(Plan::Input),
            at: SourceSpan::new(&Arc::from("t"), 0, 0),
        };
        assert!(live.is_live());
        assert_eq!(live.protocol(), Protocol::Items);
        let finite = Plan::concat(vec![
            Val::str("a"),
            Val::Text(Arc::new(Plan::Lit("b".into()))),
        ]);
        assert!(!finite.is_live());
        assert!(finite.is_text());
        let mixed = Plan::concat(vec![Val::str("a"), Val::Text(Arc::new(live))]);
        assert!(mixed.is_live());
        assert_eq!(Plan::Input.protocol(), Protocol::JsonEvents);
    }

    #[test]
    fn selector_segments_name_one_location_only() {
        let one = Selector::root().property("a").index(2);
        assert_eq!(
            selector_segments(&one),
            Some(vec![Segment::key("a"), Segment::Index(2)])
        );
        assert_eq!(selector_segments(&Selector::root().each_index()), None);
    }

    #[test]
    fn equality_is_structural_for_data_and_false_for_resources() {
        assert_eq!(
            Val::tagged("table-end", vec![]),
            Val::tagged("table-end", vec![])
        );
        assert_ne!(
            Val::tagged("table-end", vec![]),
            Val::tagged("no-schema", vec![])
        );
        assert_eq!(
            Val::Num {
                value: 1.0,
                lexeme: Some("1.0".into())
            },
            Val::num(1.0)
        );
        assert_ne!(Val::str("a"), Val::keyword("a"));
        let t = Val::Text(Arc::new(Plan::Lit("x".into())));
        assert_ne!(t.clone(), t);
    }
}
