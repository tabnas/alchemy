//! Lowering: a [`Plan`] becomes a chain of sinks, the routing stages made
//! by the host's [`Routers`] and the rendering ones by its [`Renderers`]
//! (transduce's and render's, which the host passes in).
//!
//! The evaluator answers a plan; the host has a `JsonEvents/1` producer
//! and a text output. This module joins them. The chain is push-based and
//! synchronous, as the two crates are: the host calls the returned
//! [`Sink`] once per event and once with `End`, each stage does its work
//! and calls the next, and the output is flushed once, at the end, by the
//! stage that owns it. Four kinds of stage exist, one per protocol
//! boundary:
//!
//! - **events** (`JsonEvents/1`): the host's input, a `records` stage
//!   ([`Renderers::records_to_json`]) over table events, or an adapter
//!   ([`TaggedToJson`]) reading the tagged events an interpreted stream
//!   yields, the reverse of `events`;
//! - **table** (`TableRows/1`): an adapter reading the tagged `schema`,
//!   `row` and `table-end` values a stream yields;
//! - **items** (a stream of values): a router ([`Routers::router`]) over
//!   events for `route` and `select`, [`EventsToItems`] for `events`,
//!   [`ParserEventsToItems`] for `parser-events` (the head of the chain
//!   when the host declares parser events: it is what the host pushes the
//!   rule passes into, [`Lowering::parser_head`]), the
//!   operator [`Routers::scan_emit`] answers for `scan-emit`, and per-item
//!   stages for `map` and `filter`;
//! - **text**: the JSON renderer ([`Renderers::json`]) over events for
//!   `json`, the host's CSV renderer ([`Renderers::csv`]) or
//!   [`Renderers::records_to_json`] over a table result
//!   ([`Lowering::sink_as`]), `concat-map` and `join`
//!   over a stream of items, `replace-text` as [`Renderers::replace_text`],
//!   and `concat` around one live text as a frame that writes its prefix
//!   before the first fragment and its suffix at the flush.
//!
//! A text that does not reach the input is finite and is written whole
//! wherever it lands ([`write_finite`]): a row of the library's CSV is
//! rendered into a scratch string and written as one fragment, so a
//! failure in the middle of a row leaves no half row behind, as the host's
//! CSV renderer promises. Errors are `Fail`s with the transduce and render
//! codes unchanged; a stream given to a stage of another protocol is
//! `DSL_TYPE_ERROR` with the `protocol_mismatch` finer code, the same word
//! the checker uses when it can see it first.

use std::cell::RefCell;
use std::sync::{Arc, Mutex, OnceLock};

use indexmap::IndexMap;

use crate::ast::SourceSpan;
use crate::interp::{Bounds, Runtime};
use crate::program::Output;
use crate::shared::limits::NODE_BYTES;
use crate::shared::{
    CaptureSpec, Cell, Code, CsvOptions, Datum, Fail, Flow, JoinOut, JsonEvent, JsonOptions,
    Limits, Metrics, Number, ParserEvent, ParserSink, PublicColumn, Renderers, RouteSink, Routers,
    RulePass, ScanEmitter, ScanFinish, ScanOut, ScanStep, Selected, Sink, TableEvent, TableSink,
    TextOut, Transition, TreeContract,
};
use crate::stdlib::registry::{capture_budget, number_text, truth};
use crate::value::{type_error, Func, NonFinite, Plan, Protocol, Seq, Val};

/// The output every text stage writes to.
pub type Out = Box<dyn TextOut + Send>;
/// The sink the host drives.
pub type EventSink = Box<dyn Sink + Send>;
type Table = Box<dyn TableSink + Send>;
type Items = Box<dyn ItemSink>;

/// A consumer of a stream's items.
pub trait ItemSink: Send {
    fn item(&mut self, v: Val) -> Result<Flow, Fail>;
    /// The stream ended, validated. Exactly once, after the last item.
    fn end(&mut self) -> Result<Flow, Fail>;
}

/// How the host renders a stream result: CSV for table events (the
/// default), JSON as records; JSON for a JSON-events result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Renderer {
    Csv,
    Json,
}

impl Renderer {
    /// By the name the command line uses.
    pub fn named(name: &str) -> Option<Renderer> {
        match name {
            "csv" => Some(Renderer::Csv),
            "json" => Some(Renderer::Json),
            _ => None,
        }
    }
}

/// The options the `json` renderer and the `--render json` echo use:
/// compact, one document, a final newline.
pub fn json_options() -> JsonOptions {
    JsonOptions {
        indent: None,
        trailing_newline: true,
    }
}

fn protocol_mismatch(message: impl std::fmt::Display) -> Fail {
    Fail::new(Code::DslTypeError, format!("protocol_mismatch: {message}"))
}

/// The `json` renderer's adapter for `:non-finite :null`: a number that
/// is not finite goes on as null, everything else as it came.
struct NullNonFinite(Box<dyn Sink + Send>);

impl Sink for NullNonFinite {
    fn event(&mut self, ev: JsonEvent<'_>) -> Result<Flow, Fail> {
        match ev {
            JsonEvent::Number(n) if !n.value.is_finite() => self.0.event(JsonEvent::Null),
            ev => self.0.event(ev),
        }
    }
}

/// A cell as the table protocol carries it, from the value a program
/// produced: the scalars as themselves, `missing` as missing, a vector or
/// a record as its compact JSON text, as `scalar-text` writes it, under
/// the same `max_scalar_bytes`.
fn cell(rt: &Runtime, v: &Val) -> Result<Cell, Fail> {
    Ok(match v {
        Val::Null => Cell::Null,
        Val::Bool(b) => Cell::Bool(*b),
        Val::Num { value, lexeme } => Cell::Number {
            value: *value,
            lexeme: lexeme.as_deref().map(Box::from),
        },
        Val::Str(s) => Cell::String(Box::from(&**s)),
        v if v.is_missing() => Cell::Missing,
        Val::Vector(_) | Val::Record(_) => Cell::String(rt.json_text(v)?.into()),
        other => {
            return Err(type_error(format!(
                "a cell must be a scalar, a vector or a record, not {}",
                other.kind()
            )))
        }
    })
}

/// The text of a column's label, one policy for every table: a string as
/// it is, a number by its lexeme, a boolean by its name (as `scalar-text`
/// would write the same header cell); `missing` is `MISSING_VALUE`, and
/// anything else `INPUT_INVALID`. The adapter reading a program's `schema`
/// values takes it, and `csv-table` validates the library's schema by it,
/// so a label is accepted or refused alike whichever renders it.
fn label_text(label: &Val) -> Result<Box<str>, Fail> {
    Ok(match label {
        Val::Str(s) => Box::from(&**s),
        Val::Num { value, lexeme } => number_text(*value, lexeme.as_deref())?.into(),
        Val::Bool(b) => b.to_string().into(),
        v if v.is_missing() => {
            return Err(Fail::new(Code::MissingValue, "a column has no label"));
        }
        other => {
            return Err(Fail::input(format!(
                "a column's label must be a string, a number or a boolean, not {}",
                other.kind()
            )))
        }
    })
}

/// The label of a `schema` value's column: a record's `:label`.
fn label(column: &Val) -> Result<Box<str>, Fail> {
    let label = column
        .field("label")
        .ok_or_else(|| Fail::protocol("a schema column is not a record"))?;
    label_text(&label)
}

/// The columns of a `schema` value, as the table protocol carries them:
/// a vector of column records, at most `max_columns` of them.
fn schema_columns(fields: &[Val], limits: &Limits) -> Result<Vec<PublicColumn>, Fail> {
    let Val::Vector(columns) = &fields[0] else {
        return Err(Fail::protocol("schema takes a vector of columns"));
    };
    if columns.len() > limits.max_columns {
        return Err(Fail::limit(
            "max_columns",
            limits.max_columns as u64,
            format!(
                "the schema declares {} columns, more than {}",
                columns.len(),
                limits.max_columns
            ),
        ));
    }
    columns
        .iter()
        .map(|c| label(c).map(PublicColumn::new))
        .collect()
}

/// The cells of a `row` value, as the table protocol carries them.
fn row_cells(rt: &Runtime, fields: &[Val], cells: &mut Vec<Cell>) -> Result<(), Fail> {
    let Val::Vector(values) = &fields[0] else {
        return Err(Fail::protocol("row takes a vector of cells"));
    };
    cells.clear();
    for c in values.iter() {
        cells.push(cell(rt, c)?);
    }
    Ok(())
}

/// A value for a message: its kind, and at most a short prefix of its
/// text, so a failure over a large document does not carry the document.
pub fn brief(v: &Val) -> String {
    const MAX: usize = 60;
    let text = format!("{v:?}");
    if text.chars().count() <= MAX {
        return text;
    }
    let cut: String = text.chars().take(MAX).collect();
    format!("{} ({cut}...)", v.kind())
}

/// Write a finite text or string to `out`, whole. Every node takes an
/// evaluation step and a level ([`Runtime::tick`], [`Runtime::enter`]), so
/// the host's abort flag stops a text of exponentially many fragments, and
/// a `concat-map` whose function answers a text that applies it again is
/// the `recursion` failure rather than a stack overflow. A `join` and a
/// `replace-text` inside the text are the host's [`Renderers`]'. A
/// `join` writes `:record-end` among its items as
/// [`TextOut::record_end`], no item and no text.
pub fn write_finite(
    rt: &Runtime,
    renderers: &dyn Renderers,
    v: &Val,
    out: &mut (dyn TextOut + Send),
) -> Result<(), Fail> {
    rt.tick()?;
    let at = match v {
        Val::Text(plan) => match &**plan {
            Plan::ConcatMap { at, .. } => Some(at),
            _ => None,
        },
        _ => None,
    };
    let _level = rt.enter(at)?;
    match v {
        Val::Str(s) => out.write_str(s),
        Val::Text(plan) => match &**plan {
            Plan::Lit(s) => out.write_str(s),
            Plan::Concat { items, .. } => items
                .iter()
                .try_for_each(|item| write_finite(rt, renderers, item, out)),
            Plan::Join {
                sep,
                items: Seq::Vector(items),
            } => {
                let mut join = renderers.join(Box::new(&mut *out), sep);
                for item in items.iter() {
                    if item.is_record_end() {
                        join.record_end()?;
                        continue;
                    }
                    join.item_start()?;
                    write_finite(rt, renderers, item, &mut Unmarked(&mut join))?;
                    join.item_end()?;
                }
                Ok(())
            }
            Plan::ConcatMap {
                f,
                items: Seq::Vector(items),
                at,
            } => {
                for item in items.iter() {
                    let text = rt.apply(f, vec![item.clone()], at)?;
                    write_finite(rt, renderers, &text, &mut Unmarked(&mut *out))?;
                    out.record_end()?;
                }
                Ok(())
            }
            // Streamed through the replacer, as a live text is: it holds at
            // most the literal's length, never the text.
            Plan::Replace { from, to, source } => {
                let mut replace = renderers.replace_text(Box::new(Held(out)), from, to);
                write_finite(rt, renderers, source, &mut *replace)?;
                replace.flush()
            }
            _ => Err(type_error(format!(
                "{} is a live text and cannot be written as a value",
                crate::value::plan_name(plan)
            ))),
        },
        other => Err(type_error(format!(
            "a string or a text was expected, not {}",
            other.kind()
        ))),
    }
}

/// An output that passes fragments on and keeps its flush: a combinator
/// written into the middle of a text (`replace-text` inside a `concat`)
/// hands over what it holds at its own flush, and the text around it goes
/// on.
struct Held<'o>(&'o mut (dyn TextOut + Send));

impl TextOut for Held<'_> {
    fn write_str(&mut self, s: &str) -> Result<(), Fail> {
        self.0.write_str(s)
    }

    fn flush(&mut self) -> Result<(), Fail> {
        Ok(())
    }

    fn has_committed(&self) -> bool {
        self.0.has_committed()
    }

    fn record_end(&mut self) -> Result<(), Fail> {
        self.0.record_end()
    }
}

/// An item of a `join` or a `concat-map` over a vector, written through to
/// the output beneath with its own record ends dropped (the default
/// [`TextOut::record_end`]), as the stream stages' [`Scratch`] drops them:
/// an item is written whole, and the marks of the text are the ones
/// between its items. Its flush is kept, as [`Held`]'s is.
struct Unmarked<'o>(&'o mut (dyn TextOut + Send));

impl TextOut for Unmarked<'_> {
    fn write_str(&mut self, s: &str) -> Result<(), Fail> {
        self.0.write_str(s)
    }

    fn flush(&mut self) -> Result<(), Fail> {
        Ok(())
    }

    fn has_committed(&self) -> bool {
        self.0.has_committed()
    }
}

/// One item's text, assembled whole before it is written, so a failure
/// half way through an item (a missing cell in a row) leaves nothing of
/// it behind. It is bounded by `max_output_bytes`: an item longer than the
/// whole output may be could never be written, and assembling it anyway
/// is what would let a small program exhaust memory under an output
/// limit. A record end inside an item marks nothing (the default
/// [`TextOut::record_end`]): the item is written whole.
struct Scratch {
    text: String,
    max: Option<u64>,
}

impl Scratch {
    fn new(limits: &Limits) -> Scratch {
        Scratch {
            text: String::new(),
            max: limits.max_output_bytes,
        }
    }
}

impl TextOut for Scratch {
    fn write_str(&mut self, s: &str) -> Result<(), Fail> {
        if let Some(max) = self.max {
            let len = (self.text.len() + s.len()) as u64;
            if len > max {
                return Err(Fail::limit(
                    "max_output_bytes",
                    max,
                    format!("one item's text reached {len} bytes; the output may not exceed {max}"),
                ));
            }
        }
        self.text.push_str(s);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Fail> {
        Ok(())
    }

    fn has_committed(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Item stages
// ---------------------------------------------------------------------------

/// `route` and `select`: what the router delivers, as items.
struct RouteToItems {
    down: Items,
    /// `select` delivers the values; `route` wraps each in `selected`.
    values_only: bool,
}

impl RouteSink for RouteToItems {
    fn selected(&mut self, selected: Selected) -> Result<Flow, Fail> {
        let value = Val::from_owned_datum(selected.value.unwrap_or(Datum::Null));
        let item = if self.values_only {
            value
        } else {
            Val::tagged("selected", vec![Val::Keyword(selected.tag), value])
        };
        self.down.item(item)
    }

    fn end(&mut self) -> Result<Flow, Fail> {
        self.down.end()
    }
}

/// `events`: every event of `JsonEvents/1` as the tagged value a program
/// matches on, pushed down as it arrives, and `End` as the stream's end.
/// Nothing is kept between events: a key is delivered as it is read,
/// twice when the source repeats it, and a scalar with its lexeme.
struct EventsToItems {
    down: Items,
}

impl Sink for EventsToItems {
    fn event(&mut self, ev: JsonEvent<'_>) -> Result<Flow, Fail> {
        let item = match ev {
            JsonEvent::ObjectStart => Val::tagged("object-start", Vec::new()),
            JsonEvent::ObjectEnd => Val::tagged("object-end", Vec::new()),
            JsonEvent::ArrayStart => Val::tagged("array-start", Vec::new()),
            JsonEvent::ArrayEnd => Val::tagged("array-end", Vec::new()),
            JsonEvent::Key(name) => Val::tagged("key", vec![Val::str(name)]),
            JsonEvent::Null => Val::tagged("scalar", vec![Val::Null]),
            JsonEvent::Bool(b) => Val::tagged("scalar", vec![Val::Bool(b)]),
            JsonEvent::Number(n) => Val::tagged(
                "scalar",
                vec![Val::Num {
                    value: n.value,
                    lexeme: n.lexeme.map(Arc::from),
                }],
            ),
            JsonEvent::String(s) => Val::tagged("scalar", vec![Val::str(s)]),
            JsonEvent::End => return self.down.end(),
        };
        self.down.item(item)
    }
}

/// The parser's rule passes as the items `parser-events` yields: `(open
/// rule)` and `(close rule)`, the rule a record of the pass's fields, a
/// field the pass lacks left out (`get` answers `missing` for it). The
/// parse's `End` is the stream's end. The head of a program's chain when
/// the host declares parser events: the host pushes each pass into it.
pub struct ParserEventsToItems {
    down: Items,
}

impl ParserSink for ParserEventsToItems {
    fn event(&mut self, ev: &ParserEvent) -> Result<Flow, Fail> {
        let shapes = pass_shapes();
        let item = match ev {
            ParserEvent::Open(pass) => Val::Tagged {
                tag: shapes.open.clone(),
                fields: Arc::from([rule_record(pass)]),
            },
            ParserEvent::Close(pass) => Val::Tagged {
                tag: shapes.close.clone(),
                fields: Arc::from([rule_record(pass)]),
            },
            ParserEvent::End => return self.down.end(),
        };
        self.down.item(item)
    }
}

/// The shapes of the records a rule pass is carried in, made once: each
/// pass clones one, which copies its keys' table rather than hashing and
/// allocating every key again, and fills in the values by position.
struct PassShapes {
    open: Arc<str>,
    close: Arc<str>,
    /// `name d i alt tokens forced n value`, by whether the pass has an
    /// alternate (bit 0) and a value (bit 1): the fields it lacks are left
    /// out, as `get` answers `missing` for them.
    rule: [IndexMap<Arc<str>, Val>; 4],
    alt: IndexMap<Arc<str>, Val>,
    token: IndexMap<Arc<str>, Val>,
}

fn pass_shapes() -> &'static PassShapes {
    static SHAPES: OnceLock<PassShapes> = OnceLock::new();
    SHAPES.get_or_init(|| {
        let shape = |keys: &[&str]| -> IndexMap<Arc<str>, Val> {
            keys.iter().map(|k| (Arc::from(*k), Val::Null)).collect()
        };
        PassShapes {
            open: Arc::from("open"),
            close: Arc::from("close"),
            rule: [
                shape(&["name", "d", "i", "tokens", "forced", "n"]),
                shape(&["name", "d", "i", "alt", "tokens", "forced", "n"]),
                shape(&["name", "d", "i", "tokens", "forced", "n", "value"]),
                shape(&["name", "d", "i", "alt", "tokens", "forced", "n", "value"]),
            ],
            alt: shape(&["p", "r", "b", "g"]),
            token: shape(&["name", "val", "src", "row", "col"]),
        }
    })
}

/// A record of `shape`, its values `values` in the shape's order.
fn shaped(shape: &IndexMap<Arc<str>, Val>, values: impl IntoIterator<Item = Val>) -> Val {
    let mut record = shape.clone();
    for (slot, value) in record.values_mut().zip(values) {
        *slot = value;
    }
    Val::Record(Arc::new(record))
}

/// A rule pass as the record `(open rule)` and `(close rule)` carry.
fn rule_record(pass: &RulePass) -> Val {
    let shapes = pass_shapes();
    let count = |n: u64| Val::num(n as f64);
    let tokens = Val::Vector(
        pass.tokens
            .iter()
            .map(|t| {
                shaped(
                    &shapes.token,
                    [
                        Val::str(&t.name),
                        Val::from_datum(&t.val),
                        Val::str(&t.src),
                        count(t.row),
                        count(t.col),
                    ],
                )
            })
            .collect(),
    );
    let n = Val::Record(Arc::new(
        pass.n
            .iter()
            .map(|(k, v)| (Arc::from(&**k), Val::num(*v as f64)))
            .collect(),
    ));
    let alt = pass.alt.as_ref().map(|alt| {
        shaped(
            &shapes.alt,
            [
                Val::str(&alt.p),
                Val::str(&alt.r),
                count(alt.b),
                Val::Vector(alt.g.iter().map(|g| Val::str(g)).collect()),
            ],
        )
    });
    let value = pass.value.as_ref().map(Val::from_datum);
    let shape = usize::from(alt.is_some()) | (usize::from(value.is_some()) << 1);
    let fields = [Val::str(&pass.name), count(pass.d), count(pass.i)]
        .into_iter()
        .chain(alt)
        .chain([tokens, Val::Bool(pass.forced), n])
        .chain(value);
    shaped(&shapes.rule[shape], fields)
}

/// What the host pushes parser events into: the chain's head
/// ([`ParserEventsToItems`]) when the program reads them, and otherwise
/// (a text of the program's own, which reads nothing of its input) the
/// parse's end alone, handed to the sink that writes the text once the
/// input has validated.
struct ParserHead {
    head: Option<Box<dyn ParserSink + Send>>,
    events: EventSink,
}

impl ParserSink for ParserHead {
    fn event(&mut self, ev: &ParserEvent) -> Result<Flow, Fail> {
        match (&mut self.head, ev) {
            (Some(head), _) => head.event(ev),
            (None, ParserEvent::End) => self.events.event(JsonEvent::End),
            (None, _) => Ok(Flow::Continue),
        }
    }
}

/// The JSON-events sink [`Lowering::items`] answers where it lowers
/// `parser-events`: the host pushes the rule passes into the head it
/// keeps instead ([`Lowering::parser_head`]), never JSON events into this.
struct ParserEventsOnly;

impl Sink for ParserEventsOnly {
    fn event(&mut self, _: JsonEvent<'_>) -> Result<Flow, Fail> {
        Err(protocol_mismatch(
            "the program reads parser events; push them into Program::parser_sink",
        ))
    }
}

/// The tagged events an interpreted stream yields, as the JSON events a
/// taker of `JsonEvents` reads (`json`, `table-from-json`, `select`,
/// `route`, `events` again): the reverse of [`EventsToItems`], so a
/// program can rewrite a document event by event (`events` through a
/// `scan-emit`), or build events with their constructors, and hand them
/// on. Each item must be an event of the shape the constructors make
/// (`object-start`, `object-end`, `array-start`, `array-end`, `(key
/// name)`, `(scalar value)`); anything else is `PROTOCOL_ORDER_ERROR`
/// naming it. The sequence is the taker's to validate, as the source's own
/// events are, and the stream's end is the events' `End`. The sink beneath
/// is the source's guard ([`Routers::guarded`]), so the source's limits
/// hold on these events as on its own.
struct TaggedToJson {
    down: EventSink,
}

impl ItemSink for TaggedToJson {
    fn item(&mut self, v: Val) -> Result<Flow, Fail> {
        let Val::Tagged { tag, fields } = &v else {
            return Err(Fail::protocol(format!(
                "an event was expected, not {}",
                v.kind()
            )));
        };
        let event = match (&**tag, &**fields) {
            ("object-start", []) => JsonEvent::ObjectStart,
            ("object-end", []) => JsonEvent::ObjectEnd,
            ("array-start", []) => JsonEvent::ArrayStart,
            ("array-end", []) => JsonEvent::ArrayEnd,
            ("key", [Val::Str(name)]) => JsonEvent::Key(name),
            ("scalar", [Val::Null]) => JsonEvent::Null,
            ("scalar", [Val::Bool(b)]) => JsonEvent::Bool(*b),
            ("scalar", [Val::Num { value, lexeme }]) => JsonEvent::Number(Number {
                value: *value,
                lexeme: lexeme.as_deref(),
            }),
            ("scalar", [Val::Str(s)]) => JsonEvent::String(s),
            _ => {
                return Err(Fail::protocol(format!(
                    "an event was expected, not {}",
                    brief(&v)
                )))
            }
        };
        self.down.event(event)
    }

    fn end(&mut self) -> Result<Flow, Fail> {
        self.down.event(JsonEvent::End)
    }
}

type StepFn = ScanStep<Val>;
type FinishFn = ScanFinish<Val>;
type OutFn = ScanOut<Val>;

/// `scan-emit`: the host's operator ([`Routers::scan_emit`]) over the
/// program's step and finish.
struct ScanStage {
    scan: Box<dyn ScanEmitter<Val>>,
    /// Shared with the operator's output closure, so the end can reach it.
    down: Arc<Mutex<Items>>,
}

/// What a `scan-emit` state may hold: what a stage keeps from row to row,
/// as the library's table keeps its bound columns, so `max_metadata_bytes`
/// in the transduce measure; and no deeper than a captured value may be,
/// `max_depth`, since a state that wraps itself once per item would
/// otherwise nest without bound (and be dropped, one level inside the
/// next, on a stack that has a bottom).
pub fn state_bounds(limits: &Limits) -> Bounds {
    Bounds {
        node_bytes: NODE_BYTES as u64,
        max_bytes: limits.max_metadata_bytes as u64,
        bytes_limit: "max_metadata_bytes",
        max_depth: limits.max_depth,
        what: "the scan-emit state",
    }
}

fn locked<T>(_: std::sync::PoisonError<T>) -> Fail {
    Fail::new(Code::OutputFailed, "a stage downstream panicked")
}

impl ScanStage {
    fn new(
        lowering: &Lowering<'_>,
        init: Val,
        step: Func,
        finish: Func,
        at: SourceSpan,
        down: Items,
    ) -> Result<Self, Fail> {
        let (rt, metrics) = (lowering.rt.clone(), lowering.metrics.clone());
        // The initial state is retained like any the step returns, and the
        // step measures only a state that changed: a step that hands the
        // same one back, or a source with no items, would never measure
        // it, so it is measured here, before anything is read.
        let size = rt
            .measure(&init, state_bounds(rt.limits()))
            .map_err(|f| rt.fail_at(f, &at))?;
        let captured = Metrics::get(&metrics.captured_bytes);
        Metrics::raise(&metrics.retained_bytes_high, captured + size.bytes);
        let down = Arc::new(Mutex::new(down));
        let step_rt = rt.clone();
        let step_at = at.clone();
        let step_fn: StepFn = Box::new(move |state: Val, item: Val| {
            let before = state.clone();
            let t = step_rt.apply(&step, vec![state, item], &step_at)?;
            match t {
                Val::Tagged { tag, fields } if &*tag == "transition" && fields.len() == 2 => {
                    let Val::Vector(outputs) = &fields[1] else {
                        return Err(step_rt.fail_at(
                            type_error("scan-emit: a transition's outputs must be a vector"),
                            &step_at,
                        ));
                    };
                    // The state is what this stage retains across items:
                    // measured when it changes, against the limits that
                    // bound what a stage keeps from row to row, and
                    // reported with the captures in `retained_bytes_high`.
                    if !fields[0].same(&before) {
                        let size = step_rt
                            .measure(&fields[0], state_bounds(step_rt.limits()))
                            .map_err(|f| step_rt.fail_at(f, &step_at))?;
                        let captured = Metrics::get(&metrics.captured_bytes);
                        Metrics::raise(&metrics.retained_bytes_high, captured + size.bytes);
                    }
                    Ok(Transition::new(fields[0].clone(), outputs.to_vec()))
                }
                other => Err(step_rt.fail_at(
                    type_error(format!(
                        "scan-emit: the step must answer a transition, not {}",
                        other.kind()
                    )),
                    &step_at,
                )),
            }
        });
        let finish_rt = rt.clone();
        let finish_at = at.clone();
        let finish_fn: FinishFn = Box::new(move |state: Val| {
            match finish_rt.apply(&finish, vec![state], &finish_at)? {
                Val::Vector(outputs) => Ok(outputs.to_vec()),
                other => Err(finish_rt.fail_at(
                    type_error(format!(
                        "scan-emit: finish must answer a vector of outputs, not {}",
                        other.kind()
                    )),
                    &finish_at,
                )),
            }
        });
        let out_down = down.clone();
        let out_fn: OutFn = Box::new(move |o: Val| out_down.lock().map_err(locked)?.item(o));
        Ok(ScanStage {
            scan: lowering.routers.scan_emit(init, step_fn, finish_fn, out_fn),
            down,
        })
    }
}

impl ItemSink for ScanStage {
    fn item(&mut self, v: Val) -> Result<Flow, Fail> {
        self.scan.item(v)
    }

    fn end(&mut self) -> Result<Flow, Fail> {
        if self.scan.finish()? == Flow::Stop {
            return Ok(Flow::Stop);
        }
        self.down.lock().map_err(locked)?.end()
    }
}

/// `map` over a stream.
struct MapStage {
    rt: Arc<Runtime>,
    f: Func,
    at: SourceSpan,
    down: Items,
}

impl ItemSink for MapStage {
    fn item(&mut self, v: Val) -> Result<Flow, Fail> {
        let mapped = self.rt.apply(&self.f, vec![v], &self.at)?;
        if mapped.is_live() {
            return Err(self.rt.fail_at(
                type_error("map: the function must answer a value per item, not a live stream"),
                &self.at,
            ));
        }
        self.down.item(mapped)
    }

    fn end(&mut self) -> Result<Flow, Fail> {
        self.down.end()
    }
}

/// `filter` over a stream.
struct FilterStage {
    rt: Arc<Runtime>,
    f: Func,
    at: SourceSpan,
    down: Items,
}

impl ItemSink for FilterStage {
    fn item(&mut self, v: Val) -> Result<Flow, Fail> {
        let keep = self.rt.apply(&self.f, vec![v.clone()], &self.at)?;
        if truth("filter", &keep).map_err(|f| self.rt.fail_at(f, &self.at))? {
            self.down.item(v)
        } else {
            Ok(Flow::Continue)
        }
    }

    fn end(&mut self) -> Result<Flow, Fail> {
        self.down.end()
    }
}

/// The tagged values a stream yields, as table events. One schema first, rows, one `table-end`: the renderer beneath
/// validates the sequence; what is validated here is that each item is a
/// table event at all, of the table protocol's shape (at most
/// `max_columns` columns, labels by [`label_text`]), and that the stream
/// did end with `table-end`. It is the last table stage before a
/// renderer, so it counts each row in `metrics.rows`, unless `count_rows`
/// says a later stage does ([`Lowering::items`]).
struct TaggedToTable {
    rt: Arc<Runtime>,
    metrics: Arc<Metrics>,
    count_rows: bool,
    table: Table,
    columns: Vec<PublicColumn>,
    cells: Vec<Cell>,
    ended: bool,
}

impl ItemSink for TaggedToTable {
    fn item(&mut self, v: Val) -> Result<Flow, Fail> {
        let Val::Tagged { tag, fields } = &v else {
            return Err(Fail::protocol(format!(
                "a table event was expected, not {}",
                v.kind()
            )));
        };
        match (&**tag, fields.len()) {
            ("schema", 1) => {
                self.columns = schema_columns(fields, self.rt.limits())?;
                self.table.table_event(TableEvent::Schema(&self.columns))
            }
            ("row", 1) => {
                row_cells(&self.rt, fields, &mut self.cells)?;
                if self.count_rows {
                    Metrics::add(&self.metrics.rows, 1);
                }
                self.table.table_event(TableEvent::Row(&self.cells))
            }
            ("table-end", 0) => {
                self.ended = true;
                self.table.table_event(TableEvent::End)
            }
            _ => Err(Fail::protocol(format!(
                "a table event was expected, not {}",
                brief(&v)
            ))),
        }
    }

    fn end(&mut self) -> Result<Flow, Fail> {
        if self.ended {
            Ok(Flow::Continue)
        } else {
            Err(Fail::protocol("the table events ended without table-end"))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    BeforeSchema,
    Rows,
    Done,
}

/// `csv-table options events`: the protocol validator of spec section
/// 13.2 in the library's own `csv`. It checks what the host's CSV renderer
/// checks of a table result, in the same order and with the same codes
/// (the adapter above, then the renderer: the shape of each event,
/// `max_columns` and the labels, then one schema first, at least one
/// column, rows as wide as the schema, one `table-end`), and passes each
/// event on unchanged for the text to render; the cells' own checks (a
/// number's lexeme, a missing value) are the text's `scalar-text`, as they
/// are the renderer's.
struct CsvTableStage {
    rt: Arc<Runtime>,
    metrics: Arc<Metrics>,
    /// The options say `(entry :no-columns :empty)`: a table of no columns
    /// passes, for the text to write as the empty document.
    no_columns: bool,
    /// Whether this stage counts the rows in `metrics.rows`: when no later
    /// table stage does ([`Lowering::items`]).
    count_rows: bool,
    down: Items,
    phase: Phase,
    width: usize,
    rows: u64,
    cells: Vec<Cell>,
}

impl ItemSink for CsvTableStage {
    fn item(&mut self, v: Val) -> Result<Flow, Fail> {
        let Val::Tagged { tag, fields } = &v else {
            return Err(Fail::protocol(format!(
                "a table event was expected, not {}",
                v.kind()
            )));
        };
        match (&**tag, fields.len()) {
            ("schema", 1) => {
                let columns = schema_columns(fields, self.rt.limits())?;
                match self.phase {
                    Phase::BeforeSchema => {}
                    Phase::Rows => return Err(Fail::protocol("a second schema")),
                    Phase::Done => return Err(Fail::protocol("a schema after the end")),
                }
                if columns.is_empty() && !self.no_columns {
                    return Err(Fail::new(
                        Code::TargetValueUnrepresentable,
                        "a table with no columns has no CSV form",
                    ));
                }
                self.width = columns.len();
                self.phase = Phase::Rows;
            }
            ("row", 1) => {
                row_cells(&self.rt, fields, &mut self.cells)?;
                match self.phase {
                    Phase::Rows => {}
                    Phase::BeforeSchema => return Err(Fail::protocol("a row before the schema")),
                    Phase::Done => return Err(Fail::protocol("a row after the end")),
                }
                if self.cells.len() != self.width {
                    return Err(Fail::protocol(format!(
                        "row {} has {} cells; the schema has {} columns",
                        self.rows + 1,
                        self.cells.len(),
                        self.width
                    )));
                }
                self.rows += 1;
                if self.count_rows {
                    Metrics::add(&self.metrics.rows, 1);
                }
            }
            ("table-end", 0) => match self.phase {
                Phase::Rows => self.phase = Phase::Done,
                Phase::BeforeSchema => return Err(Fail::protocol("the end before the schema")),
                Phase::Done => return Err(Fail::protocol("a second end")),
            },
            _ => {
                return Err(Fail::protocol(format!(
                    "a table event was expected, not {}",
                    brief(&v)
                )))
            }
        }
        self.down.item(v)
    }

    fn end(&mut self) -> Result<Flow, Fail> {
        if self.phase != Phase::Done {
            return Err(Fail::protocol("the table events ended without table-end"));
        }
        self.down.end()
    }
}

/// The delimiter a `csv-table`'s options name, refused when no CSV reader
/// could take it: the quote, a line break or NUL, as the renderer refuses
/// one when it is built. A delimiter that is not a string is the text's to
/// refuse, where it joins the fields.
fn check_delimiter(options: &Val) -> Result<(), Fail> {
    if let Some(Val::Str(d)) = options.field("delimiter") {
        if d.contains(['"', '\r', '\n', '\0']) {
            return Err(Fail::new(
                Code::TargetValueUnrepresentable,
                format!(
                    "{:?} cannot be a CSV delimiter: it holds the quote, a line break or NUL",
                    &*d
                ),
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Text stages
// ---------------------------------------------------------------------------

/// `concat-map` over a stream: each item's text, rendered whole, and the
/// end of a record after it.
struct ConcatMapStage {
    rt: Arc<Runtime>,
    renderers: Arc<dyn Renderers>,
    f: Func,
    at: SourceSpan,
    out: Out,
    scratch: Scratch,
}

impl ItemSink for ConcatMapStage {
    fn item(&mut self, v: Val) -> Result<Flow, Fail> {
        let text = self.rt.apply(&self.f, vec![v], &self.at)?;
        self.scratch.text.clear();
        write_finite(&self.rt, &*self.renderers, &text, &mut self.scratch)
            .map_err(|f| self.rt.fail_at(f, &self.at))?;
        self.out.write_str(&self.scratch.text)?;
        self.out.record_end()?;
        Ok(Flow::Continue)
    }

    fn end(&mut self) -> Result<Flow, Fail> {
        self.out.flush()?;
        Ok(Flow::Continue)
    }
}

/// `join` over a stream: each item is one logical item of the join;
/// `:record-end` is none, and marks the end of a record.
struct JoinStage {
    rt: Arc<Runtime>,
    renderers: Arc<dyn Renderers>,
    join: Box<dyn JoinOut + Send>,
    scratch: Scratch,
}

impl ItemSink for JoinStage {
    fn item(&mut self, v: Val) -> Result<Flow, Fail> {
        if v.is_record_end() {
            self.join.record_end()?;
            return Ok(Flow::Continue);
        }
        self.scratch.text.clear();
        write_finite(&self.rt, &*self.renderers, &v, &mut self.scratch)?;
        self.join.item_start()?;
        self.join.write_str(&self.scratch.text)?;
        self.join.item_end()?;
        Ok(Flow::Continue)
    }

    fn end(&mut self) -> Result<Flow, Fail> {
        self.join.flush()?;
        Ok(Flow::Continue)
    }
}

/// `concat` around one live text: the finite items before it are written
/// before its first fragment or record end (or at the flush, when it
/// wrote nothing), the items after it at the flush, before the output
/// beneath is flushed.
struct Framed {
    rt: Arc<Runtime>,
    renderers: Arc<dyn Renderers>,
    inner: Out,
    prefix: Option<Vec<Val>>,
    suffix: Option<Vec<Val>>,
}

impl Framed {
    fn start(&mut self) -> Result<(), Fail> {
        if let Some(prefix) = self.prefix.take() {
            for item in &prefix {
                write_finite(&self.rt, &*self.renderers, item, &mut *self.inner)?;
            }
        }
        Ok(())
    }
}

impl TextOut for Framed {
    fn write_str(&mut self, s: &str) -> Result<(), Fail> {
        self.start()?;
        self.inner.write_str(s)
    }

    fn flush(&mut self) -> Result<(), Fail> {
        self.start()?;
        if let Some(suffix) = self.suffix.take() {
            for item in &suffix {
                write_finite(&self.rt, &*self.renderers, item, &mut *self.inner)?;
            }
        }
        self.inner.flush()
    }

    fn has_committed(&self) -> bool {
        self.inner.has_committed()
    }

    fn record_end(&mut self) -> Result<(), Fail> {
        self.start()?;
        self.inner.record_end()
    }
}

/// A text that never reaches the input: the events are consumed and the
/// text is written at the end.
struct FiniteTextSink {
    rt: Arc<Runtime>,
    renderers: Arc<dyn Renderers>,
    text: Val,
    out: Out,
}

impl Sink for FiniteTextSink {
    fn event(&mut self, ev: JsonEvent<'_>) -> Result<Flow, Fail> {
        if ev == JsonEvent::End {
            write_finite(&self.rt, &*self.renderers, &self.text, &mut *self.out)?;
            self.out.flush()?;
        }
        Ok(Flow::Continue)
    }
}

// ---------------------------------------------------------------------------
// The lowering
// ---------------------------------------------------------------------------

/// Lowers plans for one run: the runtime the stages call back into, the
/// limits and metrics the transduce stages take, and the host's routers and
/// renderers, which make every stage this module does not define.
pub struct Lowering<'a> {
    rt: Arc<Runtime>,
    limits: &'a Limits,
    metrics: Arc<Metrics>,
    routers: Arc<dyn Routers<Val>>,
    renderers: Arc<dyn Renderers>,
    /// The head of the chain when the program reads parser events: set
    /// where `parser-events` is lowered, taken by [`Lowering::parser_head`].
    parser_head: RefCell<Option<Box<dyn ParserSink + Send>>>,
}

impl<'a> Lowering<'a> {
    pub fn new(
        rt: Arc<Runtime>,
        limits: &'a Limits,
        metrics: Arc<Metrics>,
        routers: Arc<dyn Routers<Val>>,
        renderers: Arc<dyn Renderers>,
    ) -> Self {
        Lowering {
            rt,
            limits,
            metrics,
            routers,
            renderers,
            parser_head: RefCell::new(None),
        }
    }

    /// Keep `head` for the host to push the parser events into: the one
    /// reader of them a program may have. The JSON-events side of the
    /// chain the caller answers is never driven.
    fn keep_parser_head(&self, head: Box<dyn ParserSink + Send>) -> Result<EventSink, Fail> {
        let mut kept = self.parser_head.borrow_mut();
        if kept.is_some() {
            return Err(Fail::new(
                Code::StreamReused,
                "reused: the parser events are read twice",
            ));
        }
        *kept = Some(head);
        Ok(Box::new(ParserEventsOnly))
    }

    /// What the host pushes parser events into, once [`Lowering::sink_as`]
    /// has answered `events`, the chain's JSON-events head: the head
    /// `parser-events` was lowered to, or, when the program reads nothing
    /// of its input, the parse's end alone into `events`.
    pub fn parser_head(&self, events: EventSink) -> Box<dyn ParserSink + Send> {
        Box::new(ParserHead {
            head: self.parser_head.borrow_mut().take(),
            events,
        })
    }

    /// The sink for a program's result over `out`. `render` is the host's
    /// choice for a stream result; a text result takes none
    /// (`render_of_text`). A stream of items is taken for a table's rows,
    /// as the runtime cannot tell an item's shape before it arrives;
    /// [`Lowering::sink_as`] takes the checker's word instead.
    pub fn sink(
        &self,
        result: &Val,
        out: Out,
        render: Option<Renderer>,
    ) -> Result<EventSink, Fail> {
        let output = match result {
            Val::Stream(plan) => match plan.protocol() {
                Protocol::JsonEvents => Output::JsonEvents,
                _ => Output::TableRows,
            },
            _ => Output::Text,
        };
        self.sink_as(result, out, render, output)
    }

    /// [`Lowering::sink`] with `output`, what the checker decided the
    /// result is: a stream of items the runtime cannot tell the protocol
    /// of is JSON events when the checker typed every item an event (a
    /// rewritten tree), and a table's rows otherwise.
    pub fn sink_as(
        &self,
        result: &Val,
        out: Out,
        render: Option<Renderer>,
        output: Output,
    ) -> Result<EventSink, Fail> {
        match result {
            // A string is a text where a text is expected (spec 10.4).
            Val::Str(s) => self.sink_as(
                &Val::Text(Arc::new(Plan::Lit(s.arc()))),
                out,
                render,
                output,
            ),
            Val::Text(plan) => {
                if render.is_some() {
                    return Err(Fail::new(
                        Code::DslTypeError,
                        "render_of_text: the program renders its own text; --render applies to a table or JSON events result",
                    ));
                }
                self.text(plan, out)
            }
            Val::Stream(plan) => {
                let protocol = match (plan.protocol(), output) {
                    // A rewritten tree: the checker saw every item an event.
                    (Protocol::Items, Output::JsonEvents) => Protocol::JsonEvents,
                    (protocol, _) => protocol,
                };
                let render = render.unwrap_or(match protocol {
                    Protocol::JsonEvents => Renderer::Json,
                    _ => Renderer::Csv,
                });
                match (protocol, render) {
                    (Protocol::JsonEvents, Renderer::Json) => {
                        self.events(plan, self.renderers.json(out, json_options()))
                    }
                    (Protocol::JsonEvents, Renderer::Csv) => Err(protocol_mismatch(
                        "csv renders table events; the program's result is JSON events (render it as json, or make a table of it with table-from-json)",
                    )),
                    (_, Renderer::Csv) => {
                        let renderer = self.renderers.csv(out, CsvOptions::default())?;
                        self.table(plan, renderer, false)
                    }
                    (_, Renderer::Json) => {
                        let json = self.renderers.json(out, json_options());
                        self.table(plan, self.renderers.records_to_json(json), false)
                    }
                }
            }
            other => Err(type_error(format!(
                "export must answer a text or a stream, not {}",
                other.kind()
            ))),
        }
    }

    /// The sink for a program's JSON-events result whose events go into
    /// the host's own `down` rather than to a renderer: the host's stages
    /// take them as a source's (a renderer of its own, a scope over a
    /// path), and `down` sees the program's `End` after the root.
    /// `output` is the checker's word, as for [`Lowering::sink_as`]. A
    /// result that is a text or a table is refused (`protocol_mismatch`).
    pub fn events_into(
        &self,
        result: &Val,
        output: Output,
        down: EventSink,
    ) -> Result<EventSink, Fail> {
        let Val::Stream(plan) = result else {
            return Err(protocol_mismatch(format!(
                "the program's result is {}, and a host takes JSON events into its own sink",
                result.kind()
            )));
        };
        match (plan.protocol(), output) {
            (Protocol::JsonEvents, _) | (Protocol::Items, Output::JsonEvents) => {
                self.events(plan, down)
            }
            _ => Err(protocol_mismatch(
                "the program's result is a table, and a host takes JSON events into its own sink",
            )),
        }
    }

    fn text(&self, plan: &Arc<Plan>, out: Out) -> Result<EventSink, Fail> {
        if !plan.is_live() {
            return Ok(Box::new(FiniteTextSink {
                rt: self.rt.clone(),
                renderers: self.renderers.clone(),
                text: Val::Text(plan.clone()),
                out,
            }));
        }
        match &**plan {
            Plan::Json { source, non_finite } => {
                let json = self.renderers.json(out, json_options());
                match non_finite {
                    NonFinite::Null => self.events(source, Box::new(NullNonFinite(json))),
                    _ => self.events(source, json),
                }
            }
            Plan::ConcatMap {
                f,
                items: Seq::Stream(source),
                at,
            } => self.items(
                source,
                Box::new(ConcatMapStage {
                    rt: self.rt.clone(),
                    renderers: self.renderers.clone(),
                    f: f.clone(),
                    at: at.clone(),
                    out,
                    scratch: Scratch::new(self.limits),
                }),
                false,
            ),
            Plan::Join {
                sep,
                items: Seq::Stream(source),
            } => self.items(
                source,
                Box::new(JoinStage {
                    rt: self.rt.clone(),
                    renderers: self.renderers.clone(),
                    join: self.renderers.join(out, sep),
                    scratch: Scratch::new(self.limits),
                }),
                false,
            ),
            Plan::Concat { items, live } => {
                let Some(live) = *live else {
                    unreachable!("a live concat holds a live item");
                };
                let Val::Text(inner) = &items[live] else {
                    return Err(type_error("concat: a stream is not a text"));
                };
                let framed = Framed {
                    rt: self.rt.clone(),
                    renderers: self.renderers.clone(),
                    inner: out,
                    prefix: Some(items[..live].to_vec()),
                    suffix: Some(items[live + 1..].to_vec()),
                };
                self.text(inner, Box::new(framed))
            }
            Plan::Replace {
                from,
                to,
                source: Val::Text(inner),
            } => self.text(inner, self.renderers.replace_text(out, from, to)),
            other => Err(type_error(format!(
                "{} is not a text",
                crate::value::plan_name(other)
            ))),
        }
    }

    fn events(&self, plan: &Arc<Plan>, sink: EventSink) -> Result<EventSink, Fail> {
        match &**plan {
            Plan::Input => Ok(sink),
            // `records` ends a table: after it the rows are JSON events,
            // of which a later table makes rows of its own.
            Plan::Records { source } => {
                self.table(source, self.renderers.records_to_json(sink), false)
            }
            // `as-events` names what the arm below does for any items
            // plan: the program's items, each an event, as JSON events.
            Plan::AsEvents { source } => self.events(source, sink),
            // The events as they are, held to a tree's contract in front
            // of whatever takes them.
            Plan::TreeContract { source } => self.events(source, Box::new(TreeContract::new(sink))),
            // A stream whose items may be events (`events` itself, or a
            // `scan-emit`, `map` or `filter` over anything): each item is
            // turned back into an event as the stream runs, the reverse of
            // `events`, so a program can rewrite a document event by event,
            // or build events, and hand them to any taker of JSON events.
            Plan::Events { .. }
            | Plan::ScanEmit { .. }
            | Plan::Map { .. }
            | Plan::Filter { .. } => {
                // The source's three limits hold on the events a program
                // made as on the source's own, through the source's guard:
                // a document built deeper than `max_depth`, a key longer
                // than `max_key_bytes` or a scalar past `max_scalar_bytes`
                // is refused where it arrives, whatever the input held. The
                // guard counts into metrics of its own: the source's count
                // the source's events, and these are the program's.
                let guarded =
                    self.routers
                        .guarded(sink, self.limits, self.rt.abort(), Metrics::new());
                self.items(plan, Box::new(TaggedToJson { down: guarded }), false)
            }
            // A stream that never yields events: `select`'s values,
            // `route`'s selections, a table's events as items.
            other => Err(protocol_mismatch(format!(
                "{} yields a stream of items where JSON events were expected",
                crate::value::plan_name(other)
            ))),
        }
    }

    /// The sink for the table `plan` yields, handing its events to `table`.
    /// `counted_later` is as for [`Lowering::items`]: whether the rows
    /// `table` receives are counted after it rather than here.
    fn table(
        &self,
        plan: &Arc<Plan>,
        table: Table,
        counted_later: bool,
    ) -> Result<EventSink, Fail> {
        match &**plan {
            Plan::Input
            | Plan::Records { .. }
            | Plan::AsEvents { .. }
            | Plan::TreeContract { .. } => Err(protocol_mismatch(
                "table events were expected, not JSON events (table-from-json makes a table of them)",
            )),
            _ => self.items(
                plan,
                Box::new(TaggedToTable {
                    rt: self.rt.clone(),
                    metrics: self.metrics.clone(),
                    count_rows: !counted_later,
                    table,
                    columns: Vec::new(),
                    cells: Vec::new(),
                    ended: false,
                }),
                true,
            ),
        }
    }

    /// The sink for the stream of items `plan` yields, handing them to
    /// `down`.
    ///
    /// `counted_later` says whether a table stage after `down` counts the
    /// rows these items carry. Each row is counted in `metrics.rows` once,
    /// by the last table stage it passes: the adapter to a renderer, or a
    /// `csv-table` whose rows reach no later table stage. The flag passes
    /// through a `map`, a `filter` and a `scan-emit` unchanged, so a row a
    /// filter drops is not counted and one a scan adds is; a table stage
    /// sets it for its own source; a text over items, and `records`, clear
    /// it. The library's table, a `scan-emit`, counts none.
    fn items(&self, plan: &Arc<Plan>, down: Items, counted_later: bool) -> Result<EventSink, Fail> {
        match &**plan {
            Plan::Route { specs, source } => {
                // A capture bounded by a named limit takes the host's value
                // for it: the plan was built under the defaults.
                let specs = specs
                    .iter()
                    .map(|spec| {
                        let mut spec = spec.clone();
                        if let Some(budget) = &mut spec.budget {
                            if let Some(bytes) = capture_budget(self.limits, budget.name) {
                                budget.bytes = bytes;
                            }
                        }
                        spec
                    })
                    .collect();
                let router = self.routers.router(
                    specs,
                    self.limits,
                    self.rt.duplicates(),
                    self.metrics.clone(),
                    Box::new(RouteToItems {
                        down,
                        values_only: false,
                    }),
                )?;
                self.events(source, router)
            }
            Plan::Select { selector, source } => {
                let router = self.routers.router(
                    vec![CaptureSpec::materialize("selected", selector.clone())],
                    self.limits,
                    self.rt.duplicates(),
                    self.metrics.clone(),
                    Box::new(RouteToItems {
                        down,
                        values_only: true,
                    }),
                )?;
                self.events(source, router)
            }
            Plan::Events { source } => self.events(source, Box::new(EventsToItems { down })),
            // The host's parser events, the one source `parser-events`
            // reads (its native refuses any other): the head is kept for
            // the host, which pushes the rule passes into it, and the JSON
            // events side of the chain is never driven.
            Plan::ParserEvents { source } => {
                if !matches!(**source, Plan::ParserInput) {
                    return Err(protocol_mismatch(
                        "parser-events reads the host's parser events alone",
                    ));
                }
                self.keep_parser_head(Box::new(ParserEventsToItems { down }))
            }
            Plan::ScanEmit {
                init,
                step,
                finish,
                source,
                at,
            } => self.items(
                source,
                Box::new(ScanStage::new(
                    self,
                    init.clone(),
                    step.clone(),
                    finish.clone(),
                    at.clone(),
                    down,
                )?),
                counted_later,
            ),
            Plan::Map { f, source, at } => self.items(
                source,
                Box::new(MapStage {
                    rt: self.rt.clone(),
                    f: f.clone(),
                    at: at.clone(),
                    down,
                }),
                counted_later,
            ),
            Plan::Filter { f, source, at } => self.items(
                source,
                Box::new(FilterStage {
                    rt: self.rt.clone(),
                    f: f.clone(),
                    at: at.clone(),
                    down,
                }),
                counted_later,
            ),
            Plan::CsvTable { options, source } => {
                check_delimiter(options)?;
                let no_columns = crate::stdlib::registry::no_columns_empty(options)?;
                self.items(
                    source,
                    Box::new(CsvTableStage {
                        rt: self.rt.clone(),
                        metrics: self.metrics.clone(),
                        no_columns,
                        count_rows: !counted_later,
                        down,
                        phase: Phase::BeforeSchema,
                        width: 0,
                        rows: 0,
                        cells: Vec::new(),
                    }),
                    true,
                )
            }
            Plan::Input
            | Plan::Records { .. }
            | Plan::AsEvents { .. }
            | Plan::TreeContract { .. } => Err(protocol_mismatch(
                "JSON events cannot be read item by item; select or route what the stream should yield, or read its events",
            )),
            Plan::ParserInput => Err(protocol_mismatch(
                "the parser's rule passes are read item by item through parser-events",
            )),
            other => Err(type_error(format!(
                "{} is a text, not a stream",
                crate::value::plan_name(other)
            ))),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ast::Sources;
    use crate::program::Program;
    use crate::resolve::resolve;
    use crate::shared::{AbortFlag, Duplicates};
    use crate::value::Scope;
    use crate::{desugar, parse_file, stdlib};

    /// The spec's program: sections 12.1 and 13.4.
    pub(crate) const PROGRAM: &str = "def column-from-meta [source]\n  record\n    entry :label (get \"title\" source)\n    entry :source\n      as-path\n        get \"path\" source\n\ndef api-binding\n  record\n    entry :columns\n      path \"response\" \"metadata\" \"fields\"\n    entry :rows\n      path \"response\" \"payload\" \"deep\" \"records\" each-index\n    entry :column column-from-meta\n\ndef api-table [input]\n  table-from-json api-binding input\n\ndef export [input]\n  pipe input\n    api-table\n    csv csv-options\n";

    pub(crate) fn runtime(src: &str) -> Arc<Runtime> {
        let forms = desugar::program(parse_file(src, "t.alc").unwrap(), src).unwrap();
        let sources = Sources::one("t.alc", src);
        let resolved = resolve(forms, &sources, &stdlib::outer).unwrap();
        Arc::new(Runtime::new(Arc::new(resolved), sources))
    }

    /// [`runtime`] with the host's abort flag handed to it.
    pub(crate) fn runtime_with_abort(src: &str, abort: AbortFlag) -> Arc<Runtime> {
        let forms = desugar::program(parse_file(src, "t.alc").unwrap(), src).unwrap();
        let sources = Sources::one("t.alc", src);
        let resolved = resolve(forms, &sources, &stdlib::outer).unwrap();
        Arc::new(Runtime::new(Arc::new(resolved), sources).with_abort(abort))
    }

    /// The routers and renderers of the unit tests that compile a program
    /// and never lower it: compiling builds the plan and keeps the two for
    /// the sinks, and asks nothing of them. The real ones are transduce's
    /// and render's, which this crate does not depend on (they are built
    /// on its shared types), so the tests that lower and run a program are
    /// in tabnas-dmog's `rs/tests/`. Every method is unreachable.
    pub(crate) struct Unlowered;

    const UNLOWERED: &str = "a unit test lowered a program; run it from tabnas-dmog's tests";

    impl Routers<Val> for Unlowered {
        fn router(
            &self,
            _: Vec<CaptureSpec>,
            _: &Limits,
            _: Duplicates,
            _: Arc<Metrics>,
            _: Box<dyn RouteSink + Send>,
        ) -> Result<Box<dyn Sink + Send>, Fail> {
            unreachable!("{UNLOWERED}")
        }

        fn scan_emit(&self, _: Val, _: StepFn, _: FinishFn, _: OutFn) -> Box<dyn ScanEmitter<Val>> {
            unreachable!("{UNLOWERED}")
        }

        fn guarded(
            &self,
            _: Box<dyn Sink + Send>,
            _: &Limits,
            _: AbortFlag,
            _: Arc<Metrics>,
        ) -> Box<dyn Sink + Send> {
            unreachable!("{UNLOWERED}")
        }
    }

    impl Renderers for Unlowered {
        fn json(&self, _: Out, _: JsonOptions) -> Box<dyn Sink + Send> {
            unreachable!("{UNLOWERED}")
        }

        fn csv(&self, _: Out, _: CsvOptions) -> Result<Box<dyn TableSink + Send>, Fail> {
            unreachable!("{UNLOWERED}")
        }

        fn records_to_json(&self, _: Box<dyn Sink + Send>) -> Box<dyn TableSink + Send> {
            unreachable!("{UNLOWERED}")
        }

        fn join<'a>(
            &self,
            _: Box<dyn TextOut + Send + 'a>,
            _: &str,
        ) -> Box<dyn JoinOut + Send + 'a> {
            unreachable!("{UNLOWERED}")
        }

        fn replace_text<'a>(
            &self,
            _: Box<dyn TextOut + Send + 'a>,
            _: &str,
            _: &str,
        ) -> Box<dyn TextOut + Send + 'a> {
            unreachable!("{UNLOWERED}")
        }

        fn write_out(
            &self,
            _: Box<dyn std::io::Write + Send>,
            _: &Limits,
            _: Arc<Metrics>,
        ) -> Box<dyn TextOut + Send> {
            unreachable!("{UNLOWERED}")
        }
    }

    /// [`crate::compile`] with [`Unlowered`] routers and renderers, for a
    /// test that reads what compiling decided and runs nothing.
    pub(crate) fn compile(src: &str, file: &str) -> Result<Program, Fail> {
        crate::compile(src, file, Arc::new(Unlowered), Arc::new(Unlowered))
    }

    /// The csv grammar's spec for `transduce-records`.
    const CSV_SPEC: &str = "def csv-spec\n  record\n    entry :document \"csv\"\n    entry :record \"record\"\n    entry :cell \"val\"\n    entry :empty \"empty\"\n    entry :header true\n    entry :unnamed \"field~\"\n\n";

    /// The library's two scans for `transduce-records`, run here by hand
    /// over `passes` as transduce's scan-emit runs them: the inner step
    /// from `[:before [] []]` and its finish, each output through the
    /// outer step. The events, each as the value prints, or the failure.
    fn records_scan(
        rt: &Runtime,
        spec: &str,
        passes: Vec<ParserEvent>,
    ) -> Result<Vec<String>, Fail> {
        let spec = rt.def_value(Scope::Program, spec).unwrap().unwrap();
        let library = |name: &str| match rt.def_value(Scope::Stdlib, name).unwrap() {
            Some(Val::Fn(f)) => f,
            _ => panic!("{name} is a function"),
        };
        let (step, finish, expand) = (
            library("transduce-records-step"),
            library("transduce-records-finish"),
            library("transduce-records-expand"),
        );
        let at = SourceSpan::new(&Arc::from("t"), 0, 0);
        let events = |items: &[Val], out: &mut Vec<String>| -> Result<(), Fail> {
            for item in items {
                let t = rt.apply(&expand, vec![Val::vector(vec![]), item.clone()], &at)?;
                let Val::Tagged { fields, .. } = &t else {
                    panic!("expand answers a transition");
                };
                let Val::Vector(outputs) = &fields[1] else {
                    panic!("a transition's outputs are a vector");
                };
                out.extend(outputs.iter().map(|v| format!("{v:?}")));
            }
            Ok(())
        };
        let mut state = Val::vector(vec![
            Val::keyword("before"),
            Val::vector(vec![]),
            Val::vector(vec![]),
        ]);
        let mut out = Vec::new();
        for ev in passes {
            let item = match &ev {
                ParserEvent::Open(pass) => Val::tagged("open", vec![rule_record(pass)]),
                ParserEvent::Close(pass) => Val::tagged("close", vec![rule_record(pass)]),
                ParserEvent::End => {
                    let Val::Vector(tail) = rt.apply(&finish, vec![state.clone()], &at)? else {
                        panic!("finish answers a vector");
                    };
                    events(&tail.to_vec(), &mut out)?;
                    break;
                }
            };
            let next = rt.apply(&step, vec![spec.clone(), state.clone(), item], &at)?;
            let Val::Tagged { fields, .. } = &next else {
                panic!("the step answers a transition");
            };
            let Val::Vector(outputs) = &fields[1] else {
                panic!("a transition's outputs are a vector");
            };
            events(&outputs.to_vec(), &mut out)?;
            state = fields[0].clone();
        }
        Ok(out)
    }

    /// A record grammar's passes as the csv grammar makes them: `csv`
    /// around the records, `record` around each, `val` closing with each
    /// cell's value, and the two empty cells' alternates in the group
    /// `empty`, one on an `elem`'s open pass (a cell before a comma) and
    /// one on its close (a cell at the record's end).
    fn record_passes(records: &[&[Option<&str>]], at_end: &[bool]) -> Vec<ParserEvent> {
        use crate::shared::PassAlt;
        let pass = |name: &str, alt: Option<PassAlt>, value: Option<Datum>| RulePass {
            name: name.into(),
            d: 0,
            i: 0,
            alt,
            tokens: vec![],
            forced: false,
            n: vec![],
            value,
        };
        let empty = |groups: &[&str]| {
            Some(PassAlt {
                p: "".into(),
                r: "".into(),
                b: 1,
                g: groups.iter().map(|g| Box::from(*g)).collect(),
            })
        };
        let mut passes = vec![ParserEvent::Open(pass("csv", None, None))];
        for (k, record) in records.iter().enumerate() {
            passes.push(ParserEvent::Open(pass("record", None, None)));
            for cell in record.iter() {
                passes.push(match cell {
                    Some(text) => {
                        ParserEvent::Close(pass("val", None, Some(Datum::String((*text).into()))))
                    }
                    None => ParserEvent::Open(pass("elem", empty(&["empty"]), None)),
                });
            }
            if at_end.get(k) == Some(&true) {
                passes.push(ParserEvent::Close(pass(
                    "elem",
                    empty(&["comma", "empty"]),
                    None,
                )));
            }
            passes.push(ParserEvent::Close(pass("record", None, None)));
        }
        passes.push(ParserEvent::Close(pass("csv", None, None)));
        passes.push(ParserEvent::End);
        passes
    }

    /// csv's part, as the library's text runs it over the passes the csv
    /// grammar makes: a header's names key each row's cells, an empty
    /// cell holds the fill, a short row takes the names it lacks with the
    /// fill, a long row names its extra cells by the prefix and their
    /// position, and without a header each record is an array.
    #[test]
    fn transduce_records_makes_a_documents_events_of_a_record_grammars_passes() {
        let rt = runtime(
            &format!(
                "{CSV_SPEC}def bare\n  record\n    entry :document \"csv\"\n    entry :record \"record\"\n    entry :cell \"val\"\n    entry :empty \"empty\"\n    entry :fill null\n\ndef export [input] (transduce-records csv-spec input)\n"
            ),
        );
        let records: &[&[Option<&str>]] = &[
            &[Some("a"), Some("b")],
            &[Some("1"), Some("2")],
            &[Some("3")],
            &[None, Some("4")],
            &[Some("5")],
            &[Some("6"), Some("7"), Some("8")],
        ];
        let at_end = [false, false, true, false, false, false];
        assert_eq!(
            records_scan(&rt, "csv-spec", record_passes(records, &at_end)).unwrap().join(" "),
            [
                r#"array-start"#,
                r#"object-start (key "a") (scalar "1") (key "b") (scalar "2") object-end"#,
                r#"object-start (key "a") (scalar "3") (key "b") (scalar "") object-end"#,
                r#"object-start (key "a") (scalar "") (key "b") (scalar "4") object-end"#,
                r#"object-start (key "a") (scalar "5") (key "b") (scalar "") object-end"#,
                r#"object-start (key "a") (scalar "6") (key "b") (scalar "7") (key "field~2") (scalar "8") object-end"#,
                r#"array-end"#,
            ]
            .join(" ")
        );
        assert_eq!(
            records_scan(&rt, "bare", record_passes(records, &at_end))
                .unwrap()
                .join(" "),
            [
                r#"array-start"#,
                r#"array-start (scalar "a") (scalar "b") array-end"#,
                r#"array-start (scalar "1") (scalar "2") array-end"#,
                r#"array-start (scalar "3") (scalar null) array-end"#,
                r#"array-start (scalar null) (scalar "4") array-end"#,
                r#"array-start (scalar "5") array-end"#,
                r#"array-start (scalar "6") (scalar "7") (scalar "8") array-end"#,
                r#"array-end"#,
            ]
            .join(" ")
        );
        // A document that makes no pass is the empty array.
        assert_eq!(
            records_scan(&rt, "csv-spec", vec![ParserEvent::End])
                .unwrap()
                .join(" "),
            "array-start array-end"
        );
        // Passes that end inside the document are refused.
        let mut cut = record_passes(&[&[Some("a")]], &[false]);
        cut.truncate(2);
        cut.push(ParserEvent::End);
        assert_eq!(
            records_scan(&rt, "csv-spec", cut).unwrap_err().code,
            Code::ProtocolOrderError
        );
    }

    /// The host pushes the parser's rule passes into the head
    /// `parser-events` is lowered to: each pass reaches the items below
    /// as `(open rule)` or `(close rule)`, a field the pass lacks left out
    /// of the rule, and the parse's end as the stream's end. A program
    /// that reads nothing of its input hands the end alone to the sink
    /// that writes its text.
    #[test]
    fn parser_events_are_items_pushed_into_the_head() {
        use std::sync::Mutex;

        struct Seen(Arc<Mutex<Vec<String>>>);
        impl ItemSink for Seen {
            fn item(&mut self, v: Val) -> Result<Flow, Fail> {
                self.0.lock().unwrap().push(format!("{v:?}"));
                Ok(Flow::Continue)
            }
            fn end(&mut self) -> Result<Flow, Fail> {
                self.0.lock().unwrap().push("end".to_string());
                Ok(Flow::Continue)
            }
        }
        impl Sink for Seen {
            fn event(&mut self, ev: JsonEvent<'_>) -> Result<Flow, Fail> {
                self.0.lock().unwrap().push(ev.to_owned().to_string());
                Ok(Flow::Continue)
            }
        }
        let event =
            |text: &str| ParserEvent::from_json(&serde_json::from_str(text).unwrap()).unwrap();
        let passes = [
            event(
                r##"["open",{"name":"val","d":1,"i":1,"alt":{"p":"","r":"","b":0,"g":["json"]},"tokens":[["#NR",2.5,"2.50",1,1]],"forced":false,"n":{"dmap":1}}]"##,
            ),
            event(
                r#"["close",{"name":"val","d":1,"i":1,"tokens":[],"forced":false,"n":{},"value":2.5}]"#,
            ),
            ParserEvent::End,
        ];
        let limits = Limits::default();
        let lowering = |rt| {
            Lowering::new(
                rt,
                &limits,
                Metrics::new(),
                Arc::new(Unlowered),
                Arc::new(Unlowered),
            )
        };
        let plan = Arc::new(Plan::ParserEvents {
            source: Arc::new(Plan::ParserInput),
        });

        let seen = Arc::new(Mutex::new(Vec::new()));
        let here = lowering(runtime("def export [input] (parser-events input)"));
        let events = here
            .items(&plan, Box::new(Seen(seen.clone())), false)
            .unwrap();
        // The parser events are read once.
        let again = here.items(&plan, Box::new(Seen(seen.clone())), false);
        assert_eq!(again.err().map(|f| f.code), Some(Code::StreamReused));
        let mut head = here.parser_head(events);
        for pass in &passes {
            head.event(pass).unwrap();
        }
        assert_eq!(
            *seen.lock().unwrap(),
            [
                r##"(open (record (entry :name "val") (entry :d 1) (entry :i 1) (entry :alt (record (entry :p "") (entry :r "") (entry :b 0) (entry :g ["json"]))) (entry :tokens [(record (entry :name "#NR") (entry :val 2.5) (entry :src "2.50") (entry :row 1) (entry :col 1))]) (entry :forced false) (entry :n (record (entry :dmap 1)))))"##,
                r#"(close (record (entry :name "val") (entry :d 1) (entry :i 1) (entry :tokens []) (entry :forced false) (entry :n (record)) (entry :value 2.5)))"#,
                "end",
            ]
        );
        // The JSON-events side of a parser program is never driven.
        let fail = ParserEventsOnly.event(JsonEvent::Null).unwrap_err();
        assert!(fail.message.starts_with("protocol_mismatch: "), "{fail}");

        let text = Arc::new(Mutex::new(Vec::new()));
        let nothing = lowering(runtime("def export [input] \"done\""));
        let mut head = nothing.parser_head(Box::new(Seen(text.clone())));
        for pass in &passes {
            head.event(pass).unwrap();
        }
        assert_eq!(*text.lock().unwrap(), ["end"]);
    }

    /// tabnas-json's rule passes for `src`, as a parser-events source
    /// makes them: each pass's own tokens (those its alternate gave back
    /// are the next rule's to consume), a number token's value with its
    /// source text as the lexeme, a close pass's scalar node as its
    /// value, then one end.
    fn json_passes(src: &str) -> Vec<ParserEvent> {
        use crate::shared::{PassAlt, PassToken};
        use tabnas::{RuleState, Value};
        let passes = Arc::new(Mutex::new(Vec::new()));
        let mut parser = tabnas_json::make();
        {
            let passes = passes.clone();
            parser.subscribe_rule_done(move |r, _, done| {
                let open = done.state == RuleState::Open;
                let all = if open { &r.o } else { &r.c };
                let b = done.alt.as_ref().map_or(0, |a| a.b);
                let tokens = all[..all.len().saturating_sub(b)]
                    .iter()
                    .map(|t| PassToken {
                        name: t.name.as_str().into(),
                        val: match (&t.val, t.name.as_str()) {
                            (Value::Number(n), "#NR") => Datum::Number {
                                value: *n,
                                lexeme: Some(t.src.as_str().into()),
                            },
                            (Value::Undefined, _) => Datum::Null,
                            (v, _) => Datum::from_tabnas(v),
                        },
                        src: t.src.as_str().into(),
                        row: t.site.ri as u64,
                        col: t.site.ci as u64,
                    })
                    .collect();
                let node = r.node.borrow();
                let value = match &*node {
                    _ if open => None,
                    Value::Object(_) | Value::Array(_) | Value::Undefined => None,
                    v => Some(Datum::from_tabnas(v)),
                };
                let mut n: Vec<(Box<str>, i64)> =
                    r.n.iter()
                        .map(|(k, v)| (k.as_str().into(), *v as i64))
                        .collect();
                n.sort();
                let pass = RulePass {
                    name: r.name.as_str().into(),
                    d: r.d as u64,
                    i: r.i as u64,
                    alt: done.alt.as_ref().map(|a| PassAlt {
                        p: a.p.as_str().into(),
                        r: a.r.as_str().into(),
                        b: a.b as u64,
                        g: a.g.iter().map(|g| g.as_str().into()).collect(),
                    }),
                    tokens,
                    forced: done.forced,
                    n,
                    value,
                };
                passes.lock().unwrap().push(if open {
                    ParserEvent::Open(pass)
                } else {
                    ParserEvent::Close(pass)
                });
            });
        }
        parser.parse(src).expect("the document is JSON");
        let mut out = passes.lock().unwrap().clone();
        out.push(ParserEvent::End);
        out
    }

    /// The scan a `transduce-tree` builds, run here by hand over
    /// tabnas-json's own rule passes: `step` applied to each item as
    /// transduce's scan-emit applies it, from `init`, and `finish` to the
    /// last state, since running the plan itself needs transduce's stages.
    /// The outputs, each as the value prints.
    fn tree_scan(
        rt: &Runtime,
        spec: &str,
        step: &str,
        finish: &str,
        init: Val,
        src: &str,
    ) -> Vec<String> {
        let spec = rt.def_value(Scope::Program, spec).unwrap().unwrap();
        let Some(Val::Fn(step)) = rt.def_value(Scope::Stdlib, step).unwrap() else {
            panic!("{step} is a function");
        };
        let Some(Val::Fn(finish)) = rt.def_value(Scope::Stdlib, finish).unwrap() else {
            panic!("{finish} is a function");
        };
        let at = SourceSpan::new(&Arc::from("t"), 0, 0);
        let mut state = init;
        let mut out = Vec::new();
        for ev in json_passes(src) {
            let item = match &ev {
                ParserEvent::Open(pass) => Val::tagged("open", vec![rule_record(pass)]),
                ParserEvent::Close(pass) => Val::tagged("close", vec![rule_record(pass)]),
                ParserEvent::End => {
                    let Val::Vector(tail) = rt.apply(&finish, vec![state.clone()], &at).unwrap()
                    else {
                        panic!("finish answers a vector");
                    };
                    out.extend(tail.iter().map(|v| format!("{v:?}")));
                    break;
                }
            };
            let next = rt
                .apply(&step, vec![spec.clone(), state.clone(), item], &at)
                .unwrap_or_else(|f| panic!("{src}: {f}"));
            let Val::Tagged { tag, fields } = &next else {
                panic!("the step answers a transition");
            };
            assert_eq!(&**tag, "transition");
            let Val::Vector(outputs) = &fields[1] else {
                panic!("a transition's outputs are a vector");
            };
            out.extend(outputs.iter().map(|v| format!("{v:?}")));
            state = fields[0].clone();
        }
        out
    }

    /// The json grammar's part, as the plan writes it: `transduce-tree`
    /// over tabnas-json's own rule passes yields the document's events,
    /// lexemes kept, in the order a walk of its value gives them, through
    /// transduce-step and through transduce-tree's own step alike.
    #[test]
    fn transduce_tree_makes_json_events_of_tabnas_jsons_rule_passes() {
        let rt = runtime(
            "def json-spec\n  record\n    entry :map (record (entry :container :object))\n    entry :list (record (entry :container :array))\n    entry :pair (record (entry :key 0))\n    entry :val (record (entry :scalar 0))\n\ndef export [input] (transduce-tree json-spec input)\n",
        );
        let tree = || Val::vector(vec![Val::vector(vec![]), Val::Bool(false)]);
        let stepped = |src: &str| {
            tree_scan(
                &rt,
                "json-spec",
                "transduce-step",
                "transduce-finish",
                tree(),
                src,
            )
        };
        let passed = |src: &str| {
            tree_scan(
                &rt,
                "json-spec",
                "transduce-pass",
                "transduce-done",
                Val::vector(vec![tree(), Val::vector(vec![])]),
                src,
            )
        };
        for run in [&stepped as &dyn Fn(&str) -> Vec<String>, &passed] {
            assert_eq!(
                run(r#"{"a":[1,2.50,"x",{"b":null}],"c":{},"d":[],"e":1e21,"f":true}"#).join(" "),
                r#"object-start (key "a") array-start (scalar 1) (scalar 2.50) (scalar "x") object-start (key "b") (scalar null) object-end array-end (key "c") object-start object-end (key "d") array-start array-end (key "e") (scalar 1e21) (key "f") (scalar true) object-end"#
            );
            assert_eq!(run("7").join(" "), "(scalar 7)");
            assert_eq!(run(r#""s""#).join(" "), r#"(scalar "s")"#);
            assert_eq!(run("[]").join(" "), "array-start array-end");
            assert_eq!(
                run("[[[{}]]]").join(" "),
                "array-start array-start array-start object-start object-end array-end array-end array-end"
            );
        }
    }

    /// transduce-tree's own step reads `:member` and `:text`: a member
    /// after the container its rule begins, and a `:text` rule's text, the
    /// `:src` of the tokens the passes between its open and its close
    /// consumed, first on its close; a `:text` rule closed inside another
    /// joins its text to the one around it, with its own passes' tokens.
    #[test]
    fn transduce_tree_reads_a_member_and_a_text() {
        let rt = runtime(
            "def values\n  record\n    entry :map (record (entry :container :object))\n    entry :pair (record (entry :key 0) (entry :text true))\n\ndef lists\n  record\n    entry :list (record (entry :container :object) (entry :member \"items\") (entry :text true))\n\ndef export [input] (transduce-tree values input)\n",
        );
        let init = || {
            Val::vector(vec![
                Val::vector(vec![Val::vector(vec![]), Val::Bool(false)]),
                Val::vector(vec![]),
            ])
        };
        let run = |spec: &str, src: &str| {
            tree_scan(&rt, spec, "transduce-pass", "transduce-done", init(), src).join(" ")
        };
        assert_eq!(
            run("values", r#"{"a":[1, 2],"b":"x","d":7}"#),
            r#"object-start (key "a") (scalar "[1,2]") (key "b") (scalar "\"x\"") (key "d") (scalar "7") object-end"#
        );
        assert_eq!(
            run("lists", r#"[1,[2,3],"x"]"#),
            r#"object-start (key "items") object-start (key "items") (scalar "2,3") object-end (scalar "1,[2,3],\"x\"") object-end"#
        );
        assert_eq!(
            run("lists", "[]"),
            r#"object-start (key "items") (scalar "") object-end"#
        );
    }
}
