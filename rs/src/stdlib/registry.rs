//! The native operators: every name a program may use that is not a
//! standard-library definition in `stdlib/*.alc`, each with its arity, its
//! kind (a function, a constant, a constructor), its implementation, and
//! the signature and effect the reference and the planner report.
//!
//! Every operator takes its data **last** (spec section 9.3). The
//! implementations are eager for values and build [`Plan`]s for streams
//! and texts (spec section 10.3): `map` over a vector maps now, `map` over
//! a stream answers a plan the runtime lowers once.
//!
//! Failures on data (a descriptor that is not an object, a path segment
//! that is not a string) are `INPUT_INVALID`: the data is at fault; a
//! value of the wrong kind where the program, not the data, chose it is
//! `DSL_TYPE_ERROR`.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use indexmap::IndexMap;

use crate::ast::SourceSpan;
use crate::fx::FxBuild;
use crate::interp::Runtime;
use crate::shared::{CaptureSpec, Code, Fail, Selector};
use crate::value::{type_error, Func, NonFinite, Partial, Plan, Seq, Str, Val, Vector, MISSING};

/// How many arguments an operator takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arity {
    Exact(usize),
    AtLeast(usize),
    /// From the first to the second, both included.
    Between(usize, usize),
}

impl Arity {
    pub fn exact(self) -> Option<usize> {
        match self {
            Arity::Exact(n) => Some(n),
            Arity::AtLeast(_) | Arity::Between(..) => None,
        }
    }

    pub fn accepts(self, n: usize) -> bool {
        match self {
            Arity::Exact(k) => n == k,
            Arity::AtLeast(k) => n >= k,
            Arity::Between(low, high) => (low..=high).contains(&n),
        }
    }
}

impl std::fmt::Display for Arity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Arity::Exact(n) => write!(f, "{n}"),
            Arity::AtLeast(n) => write!(f, "at least {n}"),
            Arity::Between(low, high) => write!(f, "{low} to {high}"),
        }
    }
}

/// What kind of name an operator is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Applied to arguments.
    Function,
    /// A value in its own right (`table-end`, `each-index`): the symbol
    /// denotes it, without a call.
    Constant,
    /// A function whose value is a tagged value with the operator's name
    /// as tag, so `(name p ...)` is also a pattern.
    Constructor,
}

/// The implementation of a native: the runtime (to apply functions), the
/// arguments, and the span of the form being evaluated, for a diagnostic.
pub type Call = fn(&Runtime, &[Val], &SourceSpan) -> Result<Val, Fail>;

/// One native operator.
pub struct Native {
    pub name: &'static str,
    pub arity: Arity,
    pub kind: Kind,
    pub call: Call,
    /// The signature as the reference prints it.
    pub signature: &'static str,
    /// The effect as the reference and the planner print it: what the
    /// operator retains, when its output is ready, what it consumes.
    pub effect: &'static str,
}

impl std::fmt::Debug for Native {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native {}", self.name)
    }
}

/// The native by name.
pub fn native(name: &str) -> Option<&'static Native> {
    static INDEX: OnceLock<HashMap<&'static str, &'static Native, FxBuild>> = OnceLock::new();
    INDEX
        .get_or_init(|| NATIVES.iter().map(|n| (n.name, n)).collect())
        .get(name)
        .copied()
}

/// Every native, in the order the reference lists them.
pub fn natives() -> &'static [Native] {
    NATIVES
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

fn input_invalid(message: impl std::fmt::Display) -> Fail {
    Fail::new(Code::InputInvalid, message.to_string())
}

/// Whether a value is data (what a document can hold), as opposed to a
/// function, a selector, a capture, a stream or a text.
fn is_data(v: &Val) -> bool {
    matches!(
        v,
        Val::Null | Val::Bool(_) | Val::Num { .. } | Val::Str(_) | Val::Vector(_) | Val::Record(_)
    ) || v.is_missing()
}

/// A string argument, a rope made one piece first, a step per
/// [`crate::rope::FLAT_STEP`] bytes, so that a native's read of a long
/// string is bounded by the fuel and the abort flag from its first byte,
/// whatever made it.
fn as_str<'a>(rt: &Runtime, op: &str, what: &str, v: &'a Val) -> Result<&'a Str, Fail> {
    match v {
        Val::Str(s) => {
            s.flat_with(|| rt.tick())?;
            Ok(s)
        }
        other => Err(type_error(format!(
            "{op}: {what} must be a string, not {}",
            other.kind()
        ))),
    }
}

fn as_keyword<'a>(op: &str, what: &str, v: &'a Val) -> Result<&'a Arc<str>, Fail> {
    match v {
        Val::Keyword(k) => Ok(k),
        other => Err(type_error(format!(
            "{op}: {what} must be a keyword, not {}",
            other.kind()
        ))),
    }
}

fn as_fn<'a>(op: &str, what: &str, v: &'a Val) -> Result<&'a Func, Fail> {
    match v {
        Val::Fn(f) => Ok(f),
        other => Err(type_error(format!(
            "{op}: {what} must be a function, not {}",
            other.kind()
        ))),
    }
}

fn as_selector<'a>(op: &str, what: &str, v: &'a Val) -> Result<&'a Arc<Selector>, Fail> {
    match v {
        Val::Selector(s) => Ok(s),
        other => Err(type_error(format!(
            "{op}: {what} must be a selector, not {}",
            other.kind()
        ))),
    }
}

fn as_stream<'a>(op: &str, v: &'a Val) -> Result<&'a Arc<Plan>, Fail> {
    match v {
        Val::Stream(p) => {
            not_parser_input(op, p)?;
            Ok(p)
        }
        other => Err(type_error(format!(
            "{op}: the data must be a stream, not {}",
            other.kind()
        ))),
    }
}

fn as_seq(op: &str, v: &Val) -> Result<Seq, Fail> {
    match v {
        Val::Vector(items) => Ok(Seq::Vector(items.clone())),
        Val::Stream(p) => {
            not_parser_input(op, p)?;
            Ok(Seq::Stream(p.clone()))
        }
        other => Err(type_error(format!(
            "{op}: the data must be a vector or a stream, not {}",
            other.kind()
        ))),
    }
}

/// The host's parser events are read by `parser-events` alone: anywhere
/// else they are refused as the plan is built. The checker refuses them
/// first wherever it sees them; a function read from data is what brings
/// them here.
fn not_parser_input(op: &str, p: &Plan) -> Result<(), Fail> {
    match p {
        Plan::ParserInput => Err(protocol_mismatch(format!(
            "{op}: the input is the parser's rule passes, which only parser-events reads"
        ))),
        _ => Ok(()),
    }
}

fn protocol_mismatch(message: impl std::fmt::Display) -> Fail {
    Fail::new(Code::DslTypeError, format!("protocol_mismatch: {message}"))
}

/// A string or a text: what the text algebra accepts as an item.
fn textlike(op: &str, v: &Val) -> Result<(), Fail> {
    match v {
        Val::Str(_) | Val::Text(_) => Ok(()),
        other => Err(type_error(format!(
            "{op}: expected a string or a text, not {}",
            other.kind()
        ))),
    }
}

/// The items of a vector the program built (its stack, its outputs), or
/// the type error that names the operator: what is not a vector here is
/// the program's mistake, not the data's (`as-vector` takes data).
fn as_items<'a>(op: &str, v: &'a Val) -> Result<&'a Vector, Fail> {
    match v {
        Val::Vector(items) => Ok(items),
        other => Err(type_error(format!(
            "{op}: the data must be a vector, not {}",
            other.kind()
        ))),
    }
}

/// A non-negative whole number that fits an index.
pub(crate) fn as_index(v: &Val) -> Option<usize> {
    match v {
        Val::Num { value, .. }
            if *value >= 0.0 && value.fract() == 0.0 && *value <= u32::MAX as f64 =>
        {
            Some(*value as usize)
        }
        _ => None,
    }
}

/// The field of the options record, or the type error that names it.
fn option(op: &str, options: &Val, key: &str) -> Result<Val, Fail> {
    match options {
        Val::Record(_) => {
            let v = options.field(key).unwrap_or_else(Val::missing);
            if v.is_missing() {
                return Err(type_error(format!("{op}: the options have no :{key}")));
            }
            Ok(v)
        }
        other => Err(type_error(format!(
            "{op}: the options must be a record, not {}",
            other.kind()
        ))),
    }
}

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

pub use crate::shared::number::{number_text, shortest_number};

// ---------------------------------------------------------------------------
// The implementations
// ---------------------------------------------------------------------------

fn get(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let key: &str = match &a[0] {
        Val::Keyword(k) => k,
        Val::Str(k) => k,
        other => {
            return Err(type_error(format!(
                "get: the key must be a keyword or a string, not {}",
                other.kind()
            )))
        }
    };
    get_field(key, &a[1])
}

/// `get key data`: a record's field, `missing` for an absent one or from
/// `missing`; `INPUT_INVALID` from other data, `type_mismatch` from what
/// is not data.
pub fn get_field(key: &str, data: &Val) -> Result<Val, Fail> {
    match data {
        Val::Record(_) => Ok(data.field(key).unwrap_or_else(Val::missing)),
        v if v.is_missing() => Ok(Val::missing()),
        v if is_data(v) => Err(input_invalid(format!(
            "get: {} has no member {key:?}; an object was expected",
            v.kind()
        ))),
        other => Err(type_error(format!(
            "get: a record was expected, not {}",
            other.kind()
        ))),
    }
}

fn get_path(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let selector = as_selector("get-path", "the path", &a[0])?;
    let segments = crate::value::selector_segments(selector).ok_or_else(|| {
        type_error(format!(
            "get-path: the path must name one location, not {selector}"
        ))
    })?;
    if !is_data(&a[1]) {
        return Err(type_error(format!(
            "get-path: the data must be a value, not {}",
            a[1].kind()
        )));
    }
    Ok(a[1].get_path(&segments))
}

fn as_path(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let Val::Vector(items) = &a[0] else {
        return Err(input_invalid(format!(
            "as-path: a path must be an array of segments, not {}",
            a[0].kind()
        )));
    };
    let mut selector = Selector::root();
    for item in items.iter() {
        selector = match item {
            Val::Str(s) => selector.property(&**s),
            v if as_index(v).is_some() => selector.index(as_index(v).unwrap_or(0)),
            other => {
                return Err(input_invalid(format!(
                    "as-path: a path segment must be a string or a non-negative integer, not {}",
                    other.kind()
                )))
            }
        };
    }
    Ok(Val::Selector(Arc::new(selector)))
}

fn as_vector(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    match &a[0] {
        Val::Vector(_) => Ok(a[0].clone()),
        other if is_data(other) => Err(input_invalid(format!(
            "as-vector: an array was expected, not {}",
            other.kind()
        ))),
        other => Err(type_error(format!(
            "as-vector: a vector was expected, not {}",
            other.kind()
        ))),
    }
}

fn record(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let mut fields: IndexMap<Arc<str>, Val> = IndexMap::with_capacity(a.len());
    for item in a {
        match item {
            Val::Tagged { tag, fields: kv } if &**tag == "entry" && kv.len() == 2 => {
                let key = as_keyword("record", "an entry's key", &kv[0])?;
                if fields.insert(key.clone(), kv[1].clone()).is_some() {
                    return Err(Fail::new(
                        Code::DslTypeError,
                        format!("duplicate_key: record has two entries for :{key}"),
                    ));
                }
            }
            other => {
                return Err(type_error(format!(
                    "record: every argument must be an entry, not {}",
                    other.kind()
                )))
            }
        }
    }
    Ok(Val::Record(Arc::new(fields)))
}

fn entry(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    as_keyword("entry", "the key", &a[0])?;
    Ok(Val::tagged("entry", a.to_vec()))
}

fn vector(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    if let Some(live) = a.iter().find(|v| v.is_live()) {
        return Err(type_error(format!(
            "vector: a vector cannot hold {}; a stream is used once, where it is",
            live.live_kind()
        )));
    }
    Ok(Val::vector(a.to_vec()))
}

/// `push item vector`: a new vector, the item last. A vector is
/// persistent ([`Vector`]), so the push costs a bounded amount whatever the
/// vector's length, and the vector it pushed onto is unchanged and shares
/// its items with the new one: a program accumulating a list a pass at a
/// time (a stack's markers, a game's moves) is linear in the passes. `pop`,
/// `top` and `count` below are bounded the same way.
fn push(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    if a[0].is_live() {
        return Err(type_error(format!(
            "push: a vector cannot hold {}; a stream is used once, where it is",
            a[0].live_kind()
        )));
    }
    let items = as_items("push", &a[1])?;
    Ok(Val::Vector(items.push(a[0].clone())))
}

/// `push-all items vector`: a new vector, the vector's items and then the
/// items', in their order: two vectors joined, as `push` appends one item.
/// Each item is pushed as `push` pushes it, a bounded cost whatever the
/// vector's length, so a join costs the items it appends, never the vector
/// it extends; a step for each, so the fuel and the host's abort flag bound
/// a long join. Where either is empty the join is the other, shared, after
/// the same steps. Neither vector can hold a stream or a live text, which
/// `vector` and `push` refuse, so neither does the join.
fn push_all(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let Val::Vector(items) = &a[0] else {
        return Err(type_error(format!(
            "push-all: the items must be a vector, not {}",
            a[0].kind()
        )));
    };
    let held = as_items("push-all", &a[1])?;
    if held.is_empty() {
        for _ in 0..items.len() {
            rt.tick()?;
        }
        return Ok(a[0].clone());
    }
    let mut out = held.clone();
    for item in items.iter() {
        rt.tick()?;
        out = out.push(item.clone());
    }
    Ok(Val::Vector(out))
}

fn pop(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let items = as_items("pop", &a[0])?;
    let Some(rest) = items.pop() else {
        return Err(type_error(
            "pop: the vector is empty; there is no last item to remove",
        ));
    };
    Ok(Val::Vector(rest))
}

fn top(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let items = as_items("top", &a[0])?;
    items
        .last()
        .cloned()
        .ok_or_else(|| type_error("top: the vector is empty; there is no last item"))
}

fn count(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    Ok(Val::num(as_items("count", &a[0])?.len() as f64))
}

/// `keys record`: the record's keys as strings, in its order, which for
/// a captured object is the document's. A bounded operation over one
/// record, not a fold: the library's inferred table binding reads its
/// columns from the first row with it.
fn keys(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    match &a[0] {
        Val::Record(fields) => {
            // A step per key, so the host's abort flag stops a wide
            // record's copy as it stops any other long evaluation.
            let mut out = Vec::with_capacity(fields.len());
            for k in fields.keys() {
                rt.tick()?;
                out.push(Val::Str(k.clone().into()));
            }
            Ok(Val::vector(out))
        }
        other => Err(type_error(format!(
            "keys: the record must be a record, not {}",
            other.kind()
        ))),
    }
}

/// How many bytes of a string `length` counts per evaluation step.
const LENGTH_CHUNK: usize = 64 * 1024;

/// `length string`: how many characters the string holds, as a column
/// counts them (Unicode scalar values). A long string is counted a chunk
/// at a time, an evaluation step each, so the host's abort flag stops the
/// count as it stops any long evaluation.
fn length(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let s = as_str(rt, "length", "the string", &a[0])?;
    let mut count = 0usize;
    for chunk in s.as_bytes().chunks(LENGTH_CHUNK) {
        rt.tick()?;
        // A character begins at every byte that is not a UTF-8
        // continuation byte (`10xxxxxx`), wherever the chunk is cut.
        count += chunk.iter().filter(|&&b| b & 0xC0 != 0x80).count();
    }
    Ok(Val::num(count as f64))
}

/// `number string`: the number a string spells. A JSON number keeps the
/// text as its lexeme, so a renderer writes it as it was spelled; the three
/// non-finite numbers are spelled `Infinity`, `-Infinity` and `NaN`, as
/// JavaScript spells them. A format's reverse reads its numbers back with
/// it (XML's embedding writes a number as text). The string is read by
/// [`read_number`], as `is-number` reads it.
fn number(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let s = as_str(rt, "number", "the string", &a[0])?;
    read_number(rt, s)?.ok_or_else(|| input_invalid(format!("number: {s:?} spells no number")))
}

/// `is-number string`: whether `number` reads the string, without
/// failing: a JSON number, or `Infinity`, `-Infinity` or `NaN`. A program
/// cannot catch a failure, so a format's part tests a string with it
/// before it calls `number` (zon's render, a big integer's digits; XML's
/// reverse, a number's text). The two read with [`read_number`], so
/// `number` fails exactly where this answers false.
fn is_number(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let s = as_str(rt, "is-number", "the string", &a[0])?;
    Ok(Val::Bool(read_number(rt, s)?.is_some()))
}

/// How many characters of its input `number` and `is-number` read per
/// evaluation step.
pub const NUMBER_STEP: usize = 4096;

/// What [`read_number`] has read of a spelling.
#[derive(Clone, Copy)]
enum Spelled {
    Start,
    Minus,
    /// An integer part `0`, which no digit follows.
    Zero,
    /// An integer part of a digit from 1 to 9 and the digits after it.
    Integer,
    /// A decimal point, before the fraction's first digit.
    Point,
    Fraction,
    /// The exponent's `e` or `E`.
    E,
    /// The exponent's sign.
    Sign,
    Exponent,
    /// A word begun: the characters it still wants, and the number it
    /// spells.
    Word(&'static [u8], f64),
}

/// The number `number` answers for a string, or `None` where it fails: a
/// JSON number (`-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`), its
/// text kept as the lexeme while the value is finite, or `Infinity`,
/// `-Infinity` or `NaN`. The string is read from the front, an evaluation
/// step before the first of each [`NUMBER_STEP`] characters, and refused
/// at the first character no spelling goes on with, so the host's abort
/// flag stops the read of a long one, a number or not.
fn read_number(rt: &Runtime, s: &Str) -> Result<Option<Val>, Fail> {
    use Spelled::*;
    let mut at = Start;
    // A spelling is ASCII, so a byte read is a character read up to the
    // first that is not, where the reading ends.
    for (i, &c) in s.as_bytes().iter().enumerate() {
        if i % NUMBER_STEP == 0 {
            rt.tick()?;
        }
        at = match (at, c) {
            (Start, b'-') => Minus,
            (Start | Minus, b'0') => Zero,
            (Start | Minus, b'1'..=b'9') | (Integer, b'0'..=b'9') => Integer,
            (Zero | Integer, b'.') => Point,
            (Point | Fraction, b'0'..=b'9') => Fraction,
            (Zero | Integer | Fraction, b'e' | b'E') => E,
            (E, b'+' | b'-') => Sign,
            (E | Sign | Exponent, b'0'..=b'9') => Exponent,
            (Start, b'I') => Word(b"nfinity", f64::INFINITY),
            (Minus, b'I') => Word(b"nfinity", f64::NEG_INFINITY),
            (Start, b'N') => Word(b"aN", f64::NAN),
            (Word(word, value), c) if word.first() == Some(&c) => Word(&word[1..], value),
            _ => return Ok(None),
        };
    }
    let value = match at {
        // Every JSON number reads as an f64, one too large as an infinity.
        Zero | Integer | Fraction | Exponent => match s.parse::<f64>() {
            Ok(v) => v,
            Err(_) => return Ok(None),
        },
        Word([], value) => value,
        _ => return Ok(None),
    };
    let lexeme = if value.is_finite() {
        Some(s.arc())
    } else {
        None
    };
    Ok(Some(Val::Num { value, lexeme }))
}

/// `unquoted string`: the string a double-quoted form spells, the reverse
/// of `quoted`: a leading and a trailing quote around characters and
/// JSON's escapes (`\"`, `\\`, `\/`, `\b`, `\f`, `\n`, `\r`, `\t` and
/// `\uXXXX`, a surrogate pair as the one character it names). Any other
/// text is INPUT_INVALID: no quotes, a quote or a control character
/// unescaped inside, another escape, or a surrogate on its own. A format's
/// reverse reads back with it what its embedding wrote with `quoted`. The
/// input is read an evaluation step per [`UNQUOTED_STEP`] characters, as
/// `length` counts a long string in steps, so the host's abort flag stops
/// the read of a long one.
fn unquoted(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let s = as_str(rt, "unquoted", "the string", &a[0])?;
    match read_quoted(s, || rt.tick())? {
        Some(text) => Ok(Val::Str(text.into())),
        None => Err(input_invalid(format!(
            "unquoted: {s:?} is not a double-quoted string"
        ))),
    }
}

/// How many characters of its input `unquoted` reads per evaluation step.
pub const UNQUOTED_STEP: usize = 4096;

/// The string a double-quoted form spells, or `None`.
pub fn unquote(s: &str) -> Option<String> {
    match read_quoted(s, || Ok::<(), std::convert::Infallible>(())) {
        Ok(text) => text,
        Err(never) => match never {},
    }
}

/// A string's characters, read with a `step` before the first of each
/// [`UNQUOTED_STEP`] of them, so a string read to its end takes one step
/// per [`UNQUOTED_STEP`] characters or part of them.
struct Counted<'s, F> {
    chars: std::str::Chars<'s>,
    read: usize,
    step: F,
}

impl<E, F: FnMut() -> Result<(), E>> Counted<'_, F> {
    fn next(&mut self) -> Result<Option<char>, E> {
        if self.read % UNQUOTED_STEP == 0 && !self.chars.as_str().is_empty() {
            (self.step)()?;
        }
        let c = self.chars.next();
        if c.is_some() {
            self.read += 1;
        }
        Ok(c)
    }

    /// Four hex digits, either case, as the code unit they spell.
    fn hex4(&mut self) -> Result<Option<u32>, E> {
        let mut v = 0;
        for _ in 0..4 {
            match self.next()?.and_then(|c| c.to_digit(16)) {
                Some(d) => v = v * 16 + d,
                None => return Ok(None),
            }
        }
        Ok(Some(v))
    }
}

/// [`unquote`], read from the front with a `step` per [`UNQUOTED_STEP`]
/// characters: the opening quote, the characters and escapes, and a
/// closing quote that ends the input.
fn read_quoted<E>(s: &str, step: impl FnMut() -> Result<(), E>) -> Result<Option<String>, E> {
    let mut r = Counted {
        chars: s.chars(),
        read: 0,
        step,
    };
    if r.next()? != Some('"') {
        return Ok(None);
    }
    let mut out = String::with_capacity(s.len());
    loop {
        let Some(c) = r.next()? else {
            // No closing quote.
            return Ok(None);
        };
        match c {
            // The closing quote, which must end the input.
            '"' => return Ok(r.chars.as_str().is_empty().then_some(out)),
            c if (c as u32) < 0x20 => return Ok(None),
            '\\' => {
                let Some(escape) = r.next()? else {
                    return Ok(None);
                };
                match escape {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    '/' => out.push('/'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        let Some(high) = r.hex4()? else {
                            return Ok(None);
                        };
                        let code = match high {
                            0xD800..=0xDBFF => {
                                // A high surrogate names a character only
                                // with the low one after it, as a second
                                // escape.
                                if r.next()? != Some('\\') || r.next()? != Some('u') {
                                    return Ok(None);
                                }
                                let Some(low) = r.hex4()? else {
                                    return Ok(None);
                                };
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Ok(None);
                                }
                                0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
                            }
                            0xDC00..=0xDFFF => return Ok(None),
                            v => v,
                        };
                        match char::from_u32(code) {
                            Some(c) => out.push(c),
                            None => return Ok(None),
                        }
                    }
                    _ => return Ok(None),
                }
            }
            c => out.push(c),
        }
    }
}

/// `chars-within ranges string`: whether every character of the string
/// lies within one of the ranges, each a vector `[low high]` of code
/// points, both included. The empty string is within any ranges. A
/// format's part tests a string with it against the characters the format
/// can carry (XML's `Char` production, a name's characters) before it
/// writes the string, and chooses its convention when it cannot. The
/// ranges may come from data, so the work is counted in comparisons, an
/// evaluation step per [`CHARS_WITHIN_STEP`] of them and per as many
/// ranges read, and the host's abort flag stops a long test as it stops
/// any long evaluation.
fn chars_within(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let ranges = as_items("chars-within", &a[0])?;
    let mut bounds: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
    for (i, range) in ranges.iter().enumerate() {
        if i % CHARS_WITHIN_STEP == CHARS_WITHIN_STEP - 1 {
            rt.tick()?;
        }
        let pair = match range {
            Val::Vector(pair) if pair.len() == 2 => pair,
            other => {
                return Err(type_error(format!(
                    "chars-within: a range must be a vector [low high], not {}",
                    match other {
                        Val::Vector(v) => format!("a vector of {} items", v.len()),
                        other => other.kind().to_string(),
                    }
                )))
            }
        };
        let point = |v: &Val| match v {
            Val::Num { value, .. }
                if value.fract() == 0.0 && (0.0..=1_114_111.0).contains(value) =>
            {
                Ok(*value as u32)
            }
            Val::Num { value, lexeme } => Err(type_error(format!(
                "chars-within: a range's bound must be a code point from 0 to 1114111, not {}",
                number_text(*value, lexeme.as_deref()).unwrap_or_else(|_| value.to_string())
            ))),
            other => Err(type_error(format!(
                "chars-within: a range's bound must be a number, not {}",
                other.kind()
            ))),
        };
        let (low, high) = (point(&pair[0])?, point(&pair[1])?);
        if low > high {
            return Err(type_error(format!(
                "chars-within: a range must be a vector [low high] with low at most high, not [{low} {high}]"
            )));
        }
        bounds.push((low, high));
    }
    let s = as_str(rt, "chars-within", "the string", &a[1])?;
    // Comparisons made since the last step; a character costs at least
    // one, so an empty range vector still counts its characters.
    let mut work = 0usize;
    for c in s.chars() {
        let c = c as u32;
        let mut within = false;
        for &(low, high) in &bounds {
            work += 1;
            if work >= CHARS_WITHIN_STEP {
                rt.tick()?;
                work = 0;
            }
            if (low..=high).contains(&c) {
                within = true;
                break;
            }
        }
        if bounds.is_empty() {
            work += 1;
            if work >= CHARS_WITHIN_STEP {
                rt.tick()?;
                work = 0;
            }
        }
        if !within {
            return Ok(Val::Bool(false));
        }
    }
    Ok(Val::Bool(true))
}

/// How many comparisons, or ranges read, `chars-within` makes per
/// evaluation step.
pub const CHARS_WITHIN_STEP: usize = 4096;

/// `compare a b`: how two numbers are ordered, `:less`, `:equal` or
/// `:greater`, and `:unordered` when either is NaN; -0 and 0 are equal.
fn compare(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let number = |v: &Val, which: &str| match v {
        Val::Num { value, .. } => Ok(*value),
        other => Err(type_error(format!(
            "compare: the {which} must be a number, not {}",
            other.kind()
        ))),
    };
    let (x, y) = (number(&a[0], "first")?, number(&a[1], "second")?);
    Ok(Val::keyword(match x.partial_cmp(&y) {
        Some(std::cmp::Ordering::Less) => "less",
        Some(std::cmp::Ordering::Equal) => "equal",
        Some(std::cmp::Ordering::Greater) => "greater",
        None => "unordered",
    }))
}

/// The largest count `inc` answers, 2^53 - 1: below it a 64-bit float
/// holds every whole number, so the three runtimes count alike.
pub const INC_MAX: f64 = 9_007_199_254_740_991.0;

/// `inc count`: the count one more. Alchemy has no arithmetic; a part that
/// counts (PGN's moves) counts with this. The count is a whole number of
/// at least 0, as `repeat`'s is, and below [`INC_MAX`], so the answer is
/// exact; any other number is a type error, as `repeat`'s is.
fn inc(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    match &a[0] {
        Val::Num { value, .. } if *value >= 0.0 && value.fract() == 0.0 && *value < INC_MAX => {
            Ok(Val::num(*value + 1.0))
        }
        Val::Num { value, lexeme } => {
            let spelled = lexeme
                .as_deref()
                .map_or_else(|| value.to_string(), str::to_string);
            Err(type_error(format!(
                "inc: the count must be a whole number from 0 to 9007199254740990, not {spelled}"
            )))
        }
        other => Err(type_error(format!(
            "inc: the count must be a number, not {}",
            other.kind()
        ))),
    }
}

/// `number-class number`: `:finite`, `:infinity`, `:negative-infinity` or
/// `:nan`, so that a program writing a format with spellings for the
/// numbers JSON has none for (YAML's `.inf`, `-.inf` and `.nan`) can choose
/// them; `scalar-text` refuses those numbers, as JSON and CSV must.
fn number_class(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    match &a[0] {
        Val::Num { value, .. } => Ok(Val::keyword(if value.is_nan() {
            "nan"
        } else if *value == f64::INFINITY {
            "infinity"
        } else if *value == f64::NEG_INFINITY {
            "negative-infinity"
        } else {
            "finite"
        })),
        other => Err(type_error(format!(
            "number-class: the number must be a number, not {}",
            other.kind()
        ))),
    }
}

/// The kind of a value as a keyword, so that a program can tell a string
/// from a number, which no `match` pattern does: the words `Val::kind`
/// uses in messages, without their article.
fn kind(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    // The checker refuses a stream or a text where `kind` is named; this
    // is where `kind` passed as a function meets one (`map kind [(text
    // "x")]`), and a finite text is refused as a live one is.
    match kind_word(&a[0]) {
        Some(word) => Ok(Val::keyword(word)),
        None => Err(type_error(format!(
            "kind: {} cannot be asked; a stream or a text is used where it is, not inspected",
            a[0].kind()
        ))),
    }
}

/// The word `kind` answers for a retained value; none for a stream or a
/// text, live or finite, which `kind` refuses.
fn kind_word(v: &Val) -> Option<&'static str> {
    Some(match v {
        Val::Null => "null",
        Val::Bool(_) => "boolean",
        Val::Num { .. } => "number",
        Val::Str(_) => "string",
        Val::Keyword(_) => "keyword",
        Val::Vector(_) => "vector",
        Val::Record(_) => "record",
        Val::Fn(_) => "function",
        Val::Selector(_) => "selector",
        Val::CaptureSpec(_) => "capture",
        v @ Val::Tagged { .. } if v.is_missing() => "missing",
        Val::Tagged { .. } => "tagged",
        Val::Stream(_) | Val::Text(_) => return None,
    })
}

/// `equal a b`: whether two values are equal, as a `match` pattern compares
/// a constant (`Val`'s `==`): data structurally, numbers by value (NaN
/// equals nothing, -0 equals 0), a record to one with the same members in
/// any order, a tagged value to one of the same tag and fields; a function,
/// or a text a vector holds, equals nothing, itself included. A stream or
/// a text is refused, as `kind` refuses one.
///
/// The values are walked depth first, each item and member in its order,
/// one pair at a time: a pair of containers that agree in kind and size
/// schedules only its next pair, so the walk stops at the first pair that
/// differs, or at the first member of one record the other lacks, having
/// done nothing for the pairs after it. A step for each pair compared, so
/// the fuel and the host's abort flag bound the comparison of two long
/// values as they bound building them; TypeScript and Go walk the same
/// pairs in the same order, so all three take the same steps.
fn equal(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    for v in a {
        if kind_word(v).is_none() {
            return Err(type_error(format!(
                "equal: {} cannot be compared; a stream or a text is used where it is, not inspected",
                v.kind()
            )));
        }
    }
    let mut walk = vec![Walk::Pair(&a[0], &a[1])];
    while let Some(next) = walk.pop() {
        match next {
            Walk::Pair(x, y) => {
                rt.tick()?;
                match (x, y) {
                    (Val::Vector(xs), Val::Vector(ys)) => {
                        if xs.len() != ys.len() {
                            return Ok(Val::Bool(false));
                        }
                        walk.push(Walk::Items(xs, ys, 0));
                    }
                    (Val::Record(xs), Val::Record(ys)) => {
                        if xs.len() != ys.len() {
                            return Ok(Val::Bool(false));
                        }
                        walk.push(Walk::Members(xs, ys, 0));
                    }
                    (
                        Val::Tagged {
                            tag: xt,
                            fields: xf,
                        },
                        Val::Tagged {
                            tag: yt,
                            fields: yf,
                        },
                    ) => {
                        if xt != yt || xf.len() != yf.len() {
                            return Ok(Val::Bool(false));
                        }
                        walk.push(Walk::Fields(xf, yf, 0));
                    }
                    (x, y) => {
                        if x != y {
                            return Ok(Val::Bool(false));
                        }
                    }
                }
            }
            Walk::Items(xs, ys, i) => {
                if let (Some(x), Some(y)) = (xs.get(i), ys.get(i)) {
                    walk.push(Walk::Items(xs, ys, i + 1));
                    walk.push(Walk::Pair(x, y));
                }
            }
            Walk::Members(xs, ys, i) => {
                if let Some((k, x)) = xs.get_index(i) {
                    let Some(y) = ys.get(k) else {
                        return Ok(Val::Bool(false));
                    };
                    walk.push(Walk::Members(xs, ys, i + 1));
                    walk.push(Walk::Pair(x, y));
                }
            }
            Walk::Fields(xs, ys, i) => {
                if let (Some(x), Some(y)) = (xs.get(i), ys.get(i)) {
                    walk.push(Walk::Fields(xs, ys, i + 1));
                    walk.push(Walk::Pair(x, y));
                }
            }
        }
    }
    Ok(Val::Bool(true))
}

/// What `equal`'s walk does next: compare a pair, or schedule the pair at
/// an index of two vectors, two records or two tagged values' fields that
/// already agree in kind and size.
enum Walk<'a> {
    Pair(&'a Val, &'a Val),
    Items(&'a Vector, &'a Vector, usize),
    Members(
        &'a IndexMap<Arc<str>, Val>,
        &'a IndexMap<Arc<str>, Val>,
        usize,
    ),
    Fields(&'a [Val], &'a [Val], usize),
}

fn path(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let mut selector = Selector::root();
    for item in a {
        selector = match item {
            Val::Str(s) => selector.property(&**s),
            Val::Selector(s) => selector.compose(s),
            v if as_index(v).is_some() => selector.index(as_index(v).unwrap_or(0)),
            other => {
                return Err(type_error(format!(
                "path: a segment must be a string, a non-negative integer or a selector, not {}",
                other.kind()
            )))
            }
        };
    }
    Ok(Val::Selector(Arc::new(selector)))
}

fn root(_: &Runtime, _: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    Ok(Val::Selector(Arc::new(Selector::root())))
}

fn each_index(_: &Runtime, _: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    Ok(Val::Selector(Arc::new(Selector::root().each_index())))
}

fn each_member(_: &Runtime, _: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    Ok(Val::Selector(Arc::new(Selector::root().each_member())))
}

fn property(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let name = as_str(rt, "property", "the name", &a[0])?;
    Ok(Val::Selector(Arc::new(Selector::root().property(&**name))))
}

fn index(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let i = as_index(&a[0]).ok_or_else(|| {
        type_error(format!(
            "index: the position must be a non-negative integer, not {:?}",
            a[0]
        ))
    })?;
    Ok(Val::Selector(Arc::new(Selector::root().index(i))))
}

fn compose(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let first = as_selector("compose", "the first selector", &a[0])?;
    let second = as_selector("compose", "the second selector", &a[1])?;
    Ok(Val::Selector(Arc::new(
        Selector::clone(first).compose(second),
    )))
}

/// The `Limits` fields a capture may be bounded by, by the keyword that
/// names each; the byte count is the host's, read when the plan is
/// lowered ([`capture_budget`]).
pub const CAPTURE_LIMITS: &[&str] = &[
    "max_capture_bytes",
    "max_metadata_bytes",
    "max_record_bytes",
];

/// The bytes the host's `limits` give the capture limit `name`.
pub fn capture_budget(limits: &crate::shared::Limits, name: &str) -> Option<usize> {
    match name {
        "max_capture_bytes" => Some(limits.max_capture_bytes),
        "max_metadata_bytes" => Some(limits.max_metadata_bytes),
        "max_record_bytes" => Some(limits.max_record_bytes),
        _ => None,
    }
}

fn capture(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let tag = as_keyword("capture", "the tag", &a[0])?;
    let selector = as_selector("capture", "the selector", &a[1])?;
    let mut spec = CaptureSpec::materialize(tag.clone(), Selector::clone(selector));
    if let Some(limit) = a.get(2) {
        let limit = as_keyword("capture", "the limit", limit)?;
        let Some(name) = CAPTURE_LIMITS.iter().find(|n| **n == &**limit) else {
            return Err(type_error(format!(
                "capture: the limit must be one of :{}, not :{limit}",
                CAPTURE_LIMITS.join(", :")
            )));
        };
        let bytes = capture_budget(rt.limits(), name).unwrap_or(usize::MAX);
        spec = spec.budget(bytes, name);
    }
    Ok(Val::CaptureSpec(Arc::new(spec)))
}

fn route(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let Val::Vector(items) = &a[0] else {
        return Err(type_error(format!(
            "route: the captures must be a vector, not {}",
            a[0].kind()
        )));
    };
    let specs = items
        .iter()
        .map(|item| match item {
            Val::CaptureSpec(spec) => Ok(CaptureSpec::clone(spec)),
            other => Err(type_error(format!(
                "route: every capture must be a capture, not {}",
                other.kind()
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let source = as_stream("route", &a[1])?.clone();
    Ok(Val::Stream(Arc::new(Plan::Route { specs, source })))
}

fn select(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let selector = Selector::clone(as_selector("select", "the selector", &a[0])?);
    let source = as_stream("select", &a[1])?.clone();
    Ok(Val::Stream(Arc::new(Plan::Select { selector, source })))
}

fn scan_emit(_: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    let step = as_fn("scan-emit", "the step", &a[1])?.clone();
    let finish = as_fn("scan-emit", "the finish", &a[2])?.clone();
    let source = as_stream("scan-emit", &a[3])?.clone();
    Ok(Val::Stream(Arc::new(Plan::ScanEmit {
        init: a[0].clone(),
        step,
        finish,
        source,
        at: at.clone(),
    })))
}

fn transition(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    if !matches!(a[1], Val::Vector(_)) {
        return Err(type_error(format!(
            "transition: the outputs must be a vector, not {}",
            a[1].kind()
        )));
    }
    Ok(Val::tagged("transition", a.to_vec()))
}

fn partial(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let f = as_fn("partial", "the function", &a[0])?.clone();
    Ok(Val::Fn(Func::Partial(Arc::new(Partial {
        f,
        args: a[1..].to_vec(),
    }))))
}

fn map(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    let f = as_fn("map", "the function", &a[0])?;
    match as_seq("map", &a[1])? {
        Seq::Vector(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items.iter() {
                out.push(rt.apply(f, vec![item.clone()], at)?);
            }
            Ok(Val::vector(out))
        }
        Seq::Stream(source) => Ok(Val::Stream(Arc::new(Plan::Map {
            f: f.clone(),
            source,
            at: at.clone(),
        }))),
    }
}

/// The boolean an `if`, a `filter` or a `case` decides by; anything else
/// is a type error, since nothing is implicitly true or false.
pub fn truth(op: &str, v: &Val) -> Result<bool, Fail> {
    match v {
        Val::Bool(b) => Ok(*b),
        other => Err(type_error(format!(
            "{op}: a boolean was expected, not {}",
            other.kind()
        ))),
    }
}

fn filter(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    let f = as_fn("filter", "the predicate", &a[0])?;
    match as_seq("filter", &a[1])? {
        Seq::Vector(items) => {
            let mut out = Vec::new();
            for item in items.iter() {
                if truth("filter", &rt.apply(f, vec![item.clone()], at)?)? {
                    out.push(item.clone());
                }
            }
            Ok(Val::vector(out))
        }
        Seq::Stream(source) => Ok(Val::Stream(Arc::new(Plan::Filter {
            f: f.clone(),
            source,
            at: at.clone(),
        }))),
    }
}

fn concat_map(_: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    let f = as_fn("concat-map", "the function", &a[0])?.clone();
    let items = as_seq("concat-map", &a[1])?;
    Ok(Val::Text(Arc::new(Plan::ConcatMap {
        f,
        items,
        at: at.clone(),
    })))
}

fn join(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let sep = as_str(rt, "join", "the separator", &a[0])?.arc();
    let items = as_seq("join", &a[1])?;
    if let Seq::Vector(items) = &items {
        // `:record-end` is no item: it marks the end of a record.
        for item in items.iter().filter(|item| !item.is_record_end()) {
            textlike("join", item)?;
        }
    }
    Ok(Val::Text(Arc::new(Plan::Join { sep, items })))
}

fn concat(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    for item in a {
        textlike("concat", item)?;
    }
    if a.iter().filter(|v| v.is_live()).count() > 1 {
        return Err(Fail::new(
            Code::StreamReused,
            "reused: concat was given two live texts; the input is consumed once",
        ));
    }
    Ok(Val::Text(Arc::new(Plan::concat(a.to_vec()))))
}

fn text(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let s = as_str(rt, "text", "the argument", &a[0])?.arc();
    Ok(Val::Text(Arc::new(Plan::Lit(s))))
}

fn replace_text(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let from = as_str(rt, "replace-text", "the literal to find", &a[0])?.arc();
    let to = as_str(rt, "replace-text", "the replacement", &a[1])?.arc();
    textlike("replace-text", &a[2])?;
    Ok(Val::Text(Arc::new(Plan::Replace {
        from,
        to,
        source: a[2].clone(),
    })))
}

/// The text of a scalar cell under the options' policies: what the CSV
/// renderer writes for the same cell, so that the interpreted and the
/// native `csv` agree byte for byte.
fn scalar_text(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let options = &a[0];
    let cell = &a[1];
    let text: Arc<str> = match cell {
        Val::Null => as_str(
            rt,
            "scalar-text",
            ":null-text",
            &option("scalar-text", options, "null-text")?,
        )?
        .arc(),
        Val::Bool(true) => Arc::from("true"),
        Val::Bool(false) => Arc::from("false"),
        Val::Num { value, .. }
            if !value.is_finite() && non_finite(options)? != NonFinite::Reject =>
        {
            match non_finite(options)? {
                NonFinite::Null => as_str(
                    rt,
                    "scalar-text",
                    ":null-text",
                    &option("scalar-text", options, "null-text")?,
                )?
                .arc(),
                _ => Arc::from(NonFinite::word(*value)),
            }
        }
        Val::Num { value, lexeme } => Arc::from(number_text(*value, lexeme.as_deref())?),
        Val::Str(s) => s.arc(),
        v if v.is_missing() => match option("scalar-text", options, "missing")? {
            Val::Keyword(k) if &*k == "error" => {
                return Err(Fail::new(
                    Code::MissingValue,
                    "a cell has no value and the options map missing to :error",
                ))
            }
            Val::Str(t) => t.arc(),
            other => {
                return Err(type_error(format!(
                    "scalar-text: :missing must be :error or a string, not {}",
                    other.kind()
                )))
            }
        },
        Val::Vector(_) | Val::Record(_) => Arc::from(rt.json_text(cell)?),
        other => {
            return Err(type_error(format!(
                "scalar-text: a scalar was expected, not {}",
                other.kind()
            )))
        }
    };
    Ok(Val::Str(text.into()))
}

/// The `:non-finite` policy of an options record: `:reject` (the default,
/// when the record has none), `:null` or `:literal`.
pub fn non_finite(options: &Val) -> Result<NonFinite, Fail> {
    match options.field("non-finite") {
        None => Ok(NonFinite::Reject),
        Some(v) if v.is_missing() => Ok(NonFinite::Reject),
        Some(Val::Keyword(k)) => NonFinite::named(&k).ok_or_else(|| {
            type_error(format!(
                ":non-finite must be :reject, :null or :literal, not :{k}"
            ))
        }),
        Some(other) => Err(type_error(format!(
            ":non-finite must be :reject, :null or :literal, not {}",
            other.kind()
        ))),
    }
}

/// Whether a CSV options record lets a table of no columns through, to be
/// written as the empty document: its `:no-columns` is `:empty`, where
/// `:refuse`, the default when the record has none, refuses it.
pub fn no_columns_empty(options: &Val) -> Result<bool, Fail> {
    match options.field("no-columns") {
        None => Ok(false),
        Some(v) if v.is_missing() => Ok(false),
        Some(Val::Keyword(k)) => match &*k {
            "refuse" => Ok(false),
            "empty" => Ok(true),
            other => Err(type_error(format!(
                ":no-columns must be :refuse or :empty, not :{other}"
            ))),
        },
        Some(other) => Err(type_error(format!(
            ":no-columns must be :refuse or :empty, not {}",
            other.kind()
        ))),
    }
}

/// The double-quoted form of `s`: the JSON string form (RFC 8259's
/// escapes for the quote, the backslash and U+0000 to U+001F, the short
/// ones where they exist, `\u00xx` otherwise, in the render crate's
/// lowercase) with U+007F to U+009F, U+FFFE and U+FFFF escaped the same
/// way, since YAML's double-quoted style reads JSON's escapes but its
/// printable set excludes the C1 controls and those two noncharacters,
/// which XML's characters exclude too. Every other character is written
/// as itself.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(quoted_len(s));
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if escaped_as_code(c) => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Whether [`quote`] writes a character as `\uXXXX`: U+0000 to U+001F but
/// the five with short escapes, U+007F to U+009F, U+FFFE and U+FFFF.
fn escaped_as_code(c: char) -> bool {
    let c = c as u32;
    c < 0x20 || (0x7f..=0x9f).contains(&c) || c == 0xfffe || c == 0xffff
}

/// The length of [`quote`]'s result, counted before it is built: two
/// quotes, and each character as itself, as a two-byte escape, or as the
/// six bytes of `\u00XX`.
pub fn quoted_len(s: &str) -> usize {
    2 + s
        .chars()
        .map(|c| match c {
            '"' | '\\' | '\n' | '\t' | '\r' | '\u{8}' | '\u{c}' => 2,
            c if escaped_as_code(c) => 6,
            c => c.len_utf8(),
        })
        .sum::<usize>()
}

/// `quoted string`: the double-quoted form. The result is one scalar of
/// the output, up to six bytes per byte of the string, so it is held to
/// `max_scalar_bytes`, refused before it is built.
fn quoted(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let s = as_str(rt, "quoted", "the string", &a[0])?;
    let max = rt.limits().max_scalar_bytes;
    let len = quoted_len(s);
    if len > max {
        return Err(Fail::limit(
            "max_scalar_bytes",
            max as u64,
            format!("quoted: the quoted form is {len} bytes, more than {max}"),
        ));
    }
    Ok(Val::Str(quote(s).into()))
}

/// `string-join separator strings`: the strings of a vector joined into
/// one string, the separator between them, so that a program can build
/// one scalar from several: a table cell from the runs of a Markdown
/// cell, a failure message that names a key. The result is one scalar,
/// so it is held to `max_scalar_bytes`, refused before it is built. A
/// long result is a rope ([`Str::join`]), so a text joined a piece at a
/// time, the text so far and the next piece (the library's
/// `transduce-write`), costs each join the piece, not the text so far.
fn string_join(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let separator = as_str(rt, "string-join", "the separator", &a[0])?;
    let items = as_items("string-join", &a[1])?;
    let mut len = 0usize;
    for (i, item) in items.iter().enumerate() {
        let Val::Str(s) = item else {
            return Err(type_error(format!(
                "string-join: every item must be a string, not {}",
                item.kind()
            )));
        };
        len += s.len() + if i > 0 { separator.len() } else { 0 };
    }
    let max = rt.limits().max_scalar_bytes;
    if len > max {
        return Err(Fail::limit(
            "max_scalar_bytes",
            max as u64,
            format!("string-join: the joined string is {len} bytes, more than {max}"),
        ));
    }
    // Every item is a string: the count above refused any other.
    let strings = items.iter().filter_map(|item| match item {
        Val::Str(s) => Some(s),
        _ => None,
    });
    Ok(Val::Str(Str::join(separator, strings, len)))
}

/// `split separator string`: the strings between the separator's
/// occurrences in the string, found from the front, none overlapping
/// another: one more than it occurs, an empty one where two occurrences
/// meet or at either end, and the string itself where it does not occur.
/// The separator is a literal, not a pattern, and must not be empty. A
/// format's part, which cannot read a string a character at a time, reads
/// a dotted string's parts with it to check each one (semver's prerelease
/// and build identifiers). The string takes an evaluation step for each
/// [`SPLIT_STEP`] characters read, or part of them, as `unquoted` reads
/// its input, and each part one more, as `keys` takes one for each key, so
/// the fuel and the host's abort flag bound a long split; every part is a
/// piece of the string, so together they hold no more than it does.
fn split(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let separator = as_str(rt, "split", "the separator", &a[0])?;
    if separator.is_empty() {
        return Err(type_error(
            "split: the separator must be a non-empty string, not the empty string",
        ));
    }
    let s = as_str(rt, "split", "the string", &a[1])?;
    let separator_chars = separator.chars().count();
    // The characters read so far, and the steps taken for them.
    let (mut read, mut stepped) = (0usize, 0usize);
    let mut parts = Vec::new();
    for (i, part) in s.split(&**separator).enumerate() {
        read += part.chars().count() + if i > 0 { separator_chars } else { 0 };
        while stepped < read.div_ceil(SPLIT_STEP) {
            rt.tick()?;
            stepped += 1;
        }
        rt.tick()?;
        parts.push(Val::str(part));
    }
    Ok(Val::vector(parts))
}

/// How many characters of its input `split` reads per evaluation step.
pub const SPLIT_STEP: usize = 4096;

/// `repeat count string`: the string `count` times over. The result is one
/// scalar of the output (a line's indentation), so it is held to
/// `max_scalar_bytes`, refused before it is built.
fn repeat(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    // The count is judged before it is narrowed, so that any whole
    // non-negative count past the limit is the limit's refusal, and only
    // a negative or a fractional one is a type error.
    let count = match &a[0] {
        Val::Num { value, .. } if *value >= 0.0 && value.fract() == 0.0 => *value,
        Val::Num { value, lexeme } => {
            let spelled = lexeme
                .as_deref()
                .map_or_else(|| value.to_string(), str::to_string);
            return Err(type_error(format!(
                "repeat: the count must be a whole number of at least 0, not {spelled}"
            )));
        }
        other => {
            return Err(type_error(format!(
                "repeat: the count must be a number, not {}",
                other.kind()
            )))
        }
    };
    let s = as_str(rt, "repeat", "the string", &a[1])?;
    let max = rt.limits().max_scalar_bytes;
    if count * s.len() as f64 > max as f64 {
        return Err(Fail::limit(
            "max_scalar_bytes",
            max as u64,
            format!("repeat: {count} times {} bytes is more than {max}", s.len()),
        ));
    }
    // Within the limit, the count fits a usize, or the string is empty.
    let n = if s.is_empty() { 0 } else { count as usize };
    Ok(Val::Str(s.repeat(n).into()))
}

fn fail(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    // `(fail message)` is INPUT_INVALID, the code of a document the
    // program refuses; `(fail :code message)` names what the failure is: a
    // value the target cannot carry, a stream that breaks its protocol, or a
    // document a transducer part does not stream.
    let (code, message) = match a {
        [message] => (Code::InputInvalid, message),
        [code, message] => {
            let code = match code {
                Val::Keyword(k) => match &**k {
                    "invalid" => Code::InputInvalid,
                    "unrepresentable" => Code::TargetValueUnrepresentable,
                    "protocol-order" => Code::ProtocolOrderError,
                    "unstreamable" => Code::StreamabilityUnknown,
                    other => {
                        return Err(type_error(format!(
                            "the code of fail must be :invalid, :unrepresentable, :protocol-order or :unstreamable, not :{other}"
                        )))
                    }
                },
                other => {
                    return Err(type_error(format!(
                        "fail: the code must be a keyword, not {}",
                        other.kind()
                    )))
                }
            };
            (code, message)
        }
        _ => return Err(type_error("fail takes a message, or a code and a message")),
    };
    let message = as_str(rt, "fail", "the message", message)?;
    // STREAMABILITY_UNKNOWN leads with its finer code, as the checker's
    // own do: `unstreamable`, a transducer part's refusal of a document it
    // does not stream, which a host that reads the part reads whole.
    let text = if code == Code::StreamabilityUnknown {
        format!("unstreamable: {message}")
    } else {
        message.to_string()
    };
    Err(rt.fail_at(Fail::new(code, text), at))
}

fn as_events(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let source = as_stream("as-events", &a[0])?.clone();
    Ok(Val::Stream(Arc::new(Plan::AsEvents { source })))
}

/// The positions of a vector's items, as numbers: what the interpreted
/// inferred table labels an array row's cells by. A bounded operation
/// over one vector, like `keys` over one record.
fn indices(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    match &a[0] {
        Val::Vector(items) => {
            // A step per item, as `keys` takes one per key, so the host's
            // abort flag stops a long vector's count like any evaluation.
            let mut out = Vec::with_capacity(items.len());
            for i in 0..items.len() {
                rt.tick()?;
                out.push(Val::num(i as f64));
            }
            Ok(Val::vector(out))
        }
        other => Err(type_error(format!(
            "indices: a vector was expected, not {}",
            other.kind()
        ))),
    }
}

fn is_ready_value(v: &Val) -> bool {
    matches!(v, Val::Tagged { tag, fields } if &**tag == "ready" && fields.len() == 1)
}

fn is_ready(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    Ok(Val::Bool(is_ready_value(&a[0])))
}

fn require_columns(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    match &a[0] {
        Val::Tagged { tag, fields } if &**tag == "ready" && fields.len() == 1 => {
            Ok(fields[0].clone())
        }
        _ => Err(Fail::new(
            Code::InputOrderViolation,
            "a row began before the column metadata had completed; rows must follow their metadata",
        )),
    }
}

fn constructor(name: &'static str) -> impl Fn(&Runtime, &[Val], &SourceSpan) -> Result<Val, Fail> {
    move |_, a, _| Ok(Val::tagged(name, a.to_vec()))
}

/// `schema columns`: the table's one schema. A table has at most
/// `max_columns` columns, so a schema past it is refused where it is
/// built, whatever consumes it, since its events may reach no renderer
/// that would check them.
fn schema(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    if let Val::Vector(columns) = &a[0] {
        let max = rt.limits().max_columns;
        if columns.len() > max {
            return Err(Fail::limit(
                "max_columns",
                max as u64,
                format!(
                    "the schema declares {} columns, more than {max}",
                    columns.len()
                ),
            ));
        }
    }
    constructor("schema")(rt, a, at)
}

fn row(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    constructor("row")(rt, a, at)
}

fn ready(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    constructor("ready")(rt, a, at)
}

fn selected(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    constructor("selected")(rt, a, at)
}

fn table_end(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    constructor("table-end")(rt, a, at)
}

fn no_schema(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    constructor("no-schema")(rt, a, at)
}

fn missing(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    constructor(MISSING)(rt, a, at)
}

fn object_start(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    constructor("object-start")(rt, a, at)
}

fn object_end(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    constructor("object-end")(rt, a, at)
}

fn array_start(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    constructor("array-start")(rt, a, at)
}

fn array_end(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    constructor("array-end")(rt, a, at)
}

fn key(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    as_str(rt, "key", "the name", &a[0])?;
    Ok(Val::tagged("key", a.to_vec()))
}

fn scalar(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    match &a[0] {
        Val::Null | Val::Bool(_) | Val::Num { .. } | Val::Str(_) => {
            Ok(Val::tagged("scalar", a.to_vec()))
        }
        other => Err(type_error(format!(
            "scalar: the value must be null, a boolean, a number or a string, not {}",
            other.kind()
        ))),
    }
}

fn events(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let source = as_stream("events", &a[0])?.clone();
    Ok(Val::Stream(Arc::new(Plan::Events { source })))
}

/// `parser-events input`: the host's parser events, every rule pass an
/// item. Only the host's input is parser events: a JSON-events input, or
/// any stream a program made, is refused as the plan is built.
fn parser_events(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    match &a[0] {
        Val::Stream(source) if matches!(**source, Plan::ParserInput) => {
            Ok(Val::Stream(Arc::new(Plan::ParserEvents {
                source: source.clone(),
            })))
        }
        Val::Stream(other) => Err(protocol_mismatch(format!(
            "parser-events reads the parser's rule passes, the input a host declares as parser events, not {}",
            match &**other {
                Plan::Input => "JSON events",
                plan => crate::value::plan_name(plan),
            }
        ))),
        other => Err(type_error(format!(
            "parser-events: the data must be a stream, not {}",
            other.kind()
        ))),
    }
}

/// `(open rule)` and `(close rule)`: a rule pass, the rule a record.
fn rule_pass(name: &'static str) -> impl Fn(&Runtime, &[Val], &SourceSpan) -> Result<Val, Fail> {
    move |_, a, _| match &a[0] {
        Val::Record(_) => Ok(Val::tagged(name, a.to_vec())),
        other => Err(type_error(format!(
            "{name}: the rule must be a record, not {}",
            other.kind()
        ))),
    }
}

fn open(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    rule_pass("open")(rt, a, at)
}

fn close(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    rule_pass("close")(rt, a, at)
}

fn csv_table(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let source = as_stream("csv-table", &a[1])?.clone();
    Ok(Val::Stream(Arc::new(Plan::CsvTable {
        options: a[0].clone(),
        source,
    })))
}

fn json(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let (options, events) = match a {
        [events] => (None, events),
        [options, events] => (Some(options), events),
        _ => {
            return Err(type_error(
                "json takes events, or an options record and events",
            ))
        }
    };
    let non_finite = match options {
        None => NonFinite::Reject,
        Some(options) => json_options(options)?,
    };
    let source = as_stream("json", events)?.clone();
    Ok(Val::Text(Arc::new(Plan::Json { source, non_finite })))
}

/// The policy `json`'s options record names: it holds `:non-finite` and
/// nothing else, `:reject` or `:null`, since JSON has no other spelling.
fn json_options(options: &Val) -> Result<NonFinite, Fail> {
    let Val::Record(fields) = options else {
        return Err(type_error(format!(
            "json: the options must be a record, not {}",
            options.kind()
        )));
    };
    if let Some(other) = fields.keys().find(|k| &***k != "non-finite") {
        return Err(type_error(format!(
            "json: an option must be :non-finite, not :{other}"
        )));
    }
    match non_finite(options)? {
        NonFinite::Literal => Err(type_error(
            "json: :non-finite must be :reject or :null, not :literal",
        )),
        policy => Ok(policy),
    }
}

/// `tree-contract events`: the events unchanged, held to a tree's
/// contract where they pass.
fn tree_contract(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let source = as_stream("tree-contract", &a[0])?.clone();
    Ok(Val::Stream(Arc::new(Plan::TreeContract { source })))
}

fn records(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let source = as_stream("records", &a[0])?.clone();
    Ok(Val::Stream(Arc::new(Plan::Records { source })))
}

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

const fn f(
    name: &'static str,
    arity: Arity,
    call: Call,
    signature: &'static str,
    effect: &'static str,
) -> Native {
    Native {
        name,
        arity,
        kind: Kind::Function,
        call,
        signature,
        effect,
    }
}

const fn c(
    name: &'static str,
    call: Call,
    signature: &'static str,
    effect: &'static str,
) -> Native {
    Native {
        name,
        arity: Arity::Exact(0),
        kind: Kind::Constant,
        call,
        signature,
        effect,
    }
}

const fn k(
    name: &'static str,
    arity: Arity,
    call: Call,
    signature: &'static str,
    effect: &'static str,
) -> Native {
    Native {
        name,
        arity,
        kind: Kind::Constructor,
        call,
        signature,
        effect,
    }
}

use Arity::{AtLeast, Between, Exact};

static NATIVES: &[Native] = &[
    // Values and records.
    f("get", Exact(2), get, "get key data -> Value", "reads a retained record or object; missing for an absent member"),
    f("get-path", Exact(2), get_path, "get-path path data -> Value", "walks a retained value by one concrete path; missing where the path leaves it"),
    f("as-path", Exact(1), as_path, "as-path data -> Selector", "validates a data-supplied array of segments; never reads data as source"),
    f("as-vector", Exact(1), as_vector, "as-vector data -> Vector", "a captured array as a vector; INPUT_INVALID otherwise"),
    f("record", AtLeast(0), record, "record entry... -> Record", "a retained record; duplicate keys are an error"),
    k("entry", Exact(2), entry, "entry :key value -> Entry", "one record field"),
    f("vector", AtLeast(0), vector, "vector item... -> Vector", "a retained vector; it cannot hold a stream or a live text"),
    f("push", Exact(2), push, "push item vector -> Vector", "a new vector with the item appended; it cannot hold a stream or a live text"),
    f("push-all", Exact(2), push_all, "push-all items vector -> Vector", "a new vector with every item of items appended, in their order: two vectors joined, a step for each item appended"),
    f("pop", Exact(1), pop, "pop vector -> Vector", "the vector without its last item; an empty vector is a type error"),
    f("top", Exact(1), top, "top vector -> Value", "the last item; an empty vector is a type error"),
    f("count", Exact(1), count, "count vector -> Number", "how many items the vector holds"),
    f("keys", Exact(1), keys, "keys record -> Vector", "the record's keys as strings, in its order, which for a captured object is the document's"),
    f("length", Exact(1), length, "length string -> Number", "how many characters the string holds"),
    f("unquoted", Exact(1), unquoted, "unquoted string -> String", "the string a double-quoted form spells, the reverse of quoted: JSON's escapes (\\\", \\\\, \\/, \\b, \\f, \\n, \\r, \\t, \\uXXXX) read, a surrogate pair as the one character it names; INPUT_INVALID for any other text (no quotes, a quote or a control character unescaped inside, another escape, a surrogate on its own)"),
    f("chars-within", Exact(2), chars_within, "chars-within ranges string -> Bool", "whether every character of the string lies within one of the ranges, each a vector [low high] of code points, both included; true for the empty string. A part tests a string with it against what its format can carry (XML's Char production, a name's characters) and chooses its convention where it cannot"),
    f("number", Exact(1), number, "number string -> Number", "the number the string spells: a JSON number, its text kept as the lexeme, or Infinity, -Infinity or NaN; INPUT_INVALID for any other text"),
    f("is-number", Exact(1), is_number, "is-number string -> Bool", "whether number reads the string: a JSON number, or Infinity, -Infinity or NaN; false for any other text, where number fails. A program cannot catch a failure, so a part tests a string with it before it calls number"),
    f("compare", Exact(2), compare, "compare a b -> Keyword", "how two numbers are ordered: :less, :equal or :greater, and :unordered when either is NaN"),
    f("inc", Exact(1), inc, "inc count -> Number", "the count one more: a whole number from 0 to 9007199254740990, where every runtime counts exactly; any other number is a type error"),
    f("number-class", Exact(1), number_class, "number-class number -> Keyword", ":finite, :infinity, :negative-infinity or :nan"),
    f("kind", Exact(1), kind, "kind value -> Keyword", "the kind of a value as a keyword: :null, :boolean, :number, :string, :keyword, :vector, :record, :missing, :tagged, :function, :selector or :capture; a stream or a text cannot be asked"),
    f("equal", Exact(2), equal, "equal a b -> Bool", "whether two values are equal, as a match pattern compares a constant: structurally, numbers by value (NaN equals nothing), a record to one with the same members in any order, a function to nothing; a stream or a text cannot be asked; a step for each pair of values compared"),
    // Selectors.
    f("path", AtLeast(0), path, "path segment... -> Selector", "a selector from strings, indexes and selectors"),
    c("root", root, "root -> Selector", "the document"),
    c("each-index", each_index, "each-index -> Selector", "every element of an array"),
    c("each-member", each_member, "each-member -> Selector", "every member value of an object"),
    f("property", Exact(1), property, "property name -> Selector", "one member"),
    f("index", Exact(1), index, "index n -> Selector", "one element"),
    f("compose", Exact(2), compose, "compose outer inner -> Selector", "inner below every location outer names"),
    // Streams.
    f("capture", Between(2, 3), capture, "capture :tag selector [:limit] -> CaptureSpec", "materialize each selected scope under max_capture_bytes, or under the Limits field the keyword names (:max_metadata_bytes, :max_record_bytes), the host's value for it"),
    f("route", Exact(2), route, "route captures input -> Stream<Selected>", "one pass, a shared prefix matcher; retains one selected scope at a time; captures may not overlap"),
    f("select", Exact(2), select, "select selector input -> Stream<Value>", "route with one capture, delivering the values"),
    f("events", Exact(1), events, "events input -> Stream<Event>", "every event of JsonEvents as one item, as it arrives: the container events as constants, key and scalar with their one field; End ends the stream and is no item; nothing is retained between events"),
    f("indices", Exact(1), indices, "indices vector -> Vector<Number>", "the positions of the vector's items, 0 to one less than its count, as numbers: the labels the inferred table gives an array row's cells; a bounded operation over one vector"),
    f("as-events", Exact(1), as_events, "as-events items -> JsonEvents", "a stream of items the program built, each an event, as JsonEvents: what every taker of JSON events applies to such a stream, said by the program where the checker cannot type the items (an export, say); an item that is not an event fails where it arrives"),
    f("scan-emit", Exact(4), scan_emit, "scan-emit init step finish stream -> Stream<Output>", "retains its initial state and the state the step returns, measured when the stage is built and as the state changes, through every closure, partial and finite text it holds (a text's items and the function its concat-map applies included): at most max_metadata_bytes, no deeper than max_depth, reported in retained_bytes_high; ready after each item; finish runs once at the validated end"),
    k("transition", Exact(2), transition, "transition state outputs -> Transition", "one step's result: the next state and a vector of outputs; :record-end among them marks where a record of the text join writes ends (Record ends, below)"),
    f("partial", AtLeast(1), partial, "partial f arg... -> Fn", "f with its first arguments supplied"),
    f("map", Exact(2), map, "map f items -> Vector | Stream", "eager over a vector; per item over a stream, retaining nothing"),
    f("filter", Exact(2), filter, "filter predicate items -> Vector | Stream", "eager over a vector; per item over a stream"),
    // Text.
    f("concat-map", Exact(2), concat_map, "concat-map f items -> Text", "f answers a string or a text per item; each item's text is assembled whole, under max_output_bytes, and written as items arrive, so a failure leaves no half item; each item is a record of the text"),
    f("join", Exact(2), join, "join separator items -> Text", "the separator between items, never between the fragments of one; each item assembled as concat-map's is; :record-end among the items is no item and writes nothing, no separator either, but marks the end of a record"),
    f("concat", AtLeast(0), concat, "concat item... -> Text", "in order, without assembling the result"),
    f("text", Exact(1), text, "text string -> Text", "a string as a text"),
    f("replace-text", Exact(3), replace_text, "replace-text from to text -> Text", "a fixed literal replaced across fragment boundaries, a finite text's as a live one's; retains at most the literal's length"),
    f("scalar-text", Exact(2), scalar_text, "scalar-text options cell -> String", "a cell's text under the options' null, missing and non-finite policies: a string as it is, a number by its lexeme (one that is not finite refused, unless the options' :non-finite is :null, the null text, or :literal, the word Infinity, -Infinity or NaN), a boolean by its name, a vector or a record as its compact JSON text (number lexemes kept, quotes as JSON writes them) under max_scalar_bytes; the host's CSV renderer writes the same cell the same way"),
    f("quoted", Exact(1), quoted, "quoted string -> String", "the double-quoted form: a leading and a trailing quote, the quote and the backslash escaped by a backslash, U+0000 to U+001F as \\n, \\t, \\r, \\b, \\f or \\u00XX, and U+007F to U+009F, U+FFFE and U+FFFF as \\uXXXX (the JSON string form, which YAML's double-quoted style reads too, plus the C1 controls and the two noncharacters its printable set excludes, which XML's characters exclude too); refused past max_scalar_bytes, before it is built"),
    f("string-join", Exact(2), string_join, "string-join separator strings -> String", "the strings of a vector joined into one string, the separator between them; refused past max_scalar_bytes, before it is built"),
    f("split", Exact(2), split, "split separator string -> Vector", "the strings between the separator's occurrences in the string, found from the front, none overlapping another: one more than it occurs, an empty one where two occurrences meet or at either end, and the string itself where it does not occur (split \".\" \"a..b\" is [\"a\" \"\" \"b\"], split \".\" \"\" is [\"\"]); the separator is a literal, not a pattern, and must not be empty. A part reads a dotted string's parts with it to check each one"),
    f("repeat", Exact(2), repeat, "repeat count string -> String", "the string count times over; refused past max_scalar_bytes, before it is built"),
    f("fail", Between(1, 2), fail, "fail [code] message -> Never", "INPUT_INVALID with the message and the form's position; with a code first, :unrepresentable is TARGET_VALUE_UNREPRESENTABLE (a value the target cannot carry), :protocol-order is PROTOCOL_ORDER_ERROR (a stream that breaks its protocol), :unstreamable is STREAMABILITY_UNKNOWN, the finer code unstreamable leading the message (a document a transducer part does not stream from the rule passes, which a host that reads the part reads whole) and :invalid is INPUT_INVALID"),
    // The table protocol.
    f("is-ready", Exact(1), is_ready, "is-ready state -> Bool", "whether the state holds columns"),
    f("require-columns", Exact(1), require_columns, "require-columns state -> Vector<Column>", "the columns, or INPUT_ORDER_VIOLATION"),
    k("schema", Exact(1), schema, "schema columns -> TableEvent", "the table's one schema, of at most max_columns columns, refused where it is built past them"),
    k("row", Exact(1), row, "row cells -> TableEvent", "one row, as wide as the schema"),
    k("ready", Exact(1), ready, "ready columns -> State", "the state once the metadata is bound"),
    k("selected", Exact(2), selected, "selected :tag value -> Selected", "what route delivers: the capture's tag and its value"),
    c("table-end", table_end, "table-end -> TableEvent", "the table's end, after the source validated"),
    c("no-schema", no_schema, "no-schema -> State", "the state before the metadata"),
    c("missing", missing, "missing -> Value", "an absent member, distinct from null"),
    // The source's events, as `events` delivers them.
    c("object-start", object_start, "object-start -> Event", "an object begins"),
    c("object-end", object_end, "object-end -> Event", "an object ends"),
    c("array-start", array_start, "array-start -> Event", "an array begins"),
    c("array-end", array_end, "array-end -> Event", "an array ends"),
    k("key", Exact(1), key, "key name -> Event", "the name of the member whose value follows, inside an object"),
    k("scalar", Exact(1), scalar, "scalar value -> Event", "one scalar of the source: null, a boolean, a number with its lexeme, or a string"),
    // The parser's rule passes, as `parser-events` delivers them.
    f("parser-events", Exact(1), parser_events, "parser-events input -> Stream<ParserEvent>", "every rule pass of the parser as one item, as it arrives, when the host declares a parser-events input: (open rule) once a rule's open pass has run, (close rule) once its close pass has; the rule a record of :name, :d, :i, :alt, :tokens, :forced, :n and :value; the end of the parse ends the stream and is no item; nothing is retained between passes"),
    k("open", Exact(1), open, "open rule -> ParserEvent", "a rule's open pass completed: the rule a record"),
    k("close", Exact(1), close, "close rule -> ParserEvent", "a rule's close pass completed: the rule a record, its :value the node's when that is a scalar"),
    // Renderers and protocol adapters.
    f("json", Between(1, 2), json, "json [options] events -> Text", "JsonEvents, or a Stream<Event> a program built, as compact JSON text, event by event, with a final newline. A number that is not finite has no JSON form and is refused (TARGET_VALUE_UNREPRESENTABLE), unless an options record first says (entry :non-finite :null): then it is written as null. The record holds :non-finite and nothing else, :reject (the default) or :null"),
    f("records", Exact(1), records, "records table-events -> JsonEvents", "one object per row keyed by label; retains the labels"),
    f("tree-contract", Exact(1), tree_contract, "tree-contract events -> JsonEvents", "the events unchanged, held to a tree's contract as they pass: one root value, and in each object a key and then its value, each key once; a repeated key is DUPLICATE_MEMBER, and events no tree has (a value where a key is due, a key outside an object, an end with nothing open, a second root, the end inside a container) are STREAMABILITY_UNKNOWN (not_a_tree), each at the path concerned; retains the keys of each open object, dropped as it closes. A translation holds a transducer's events to it before a render that writes from a tree"),
    f("csv-table", Exact(2), csv_table, "csv-table options events -> TableEvents", "the events unchanged, validated as the CSV renderer validates them: one schema first, of at least one column and at most max_columns, labels strings, numbers or booleans; rows as wide as the schema; one table-end; a delimiter that holds the quote, a line break or NUL is refused before anything runs"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_native_is_found_by_name_and_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for n in natives() {
            assert!(seen.insert(n.name), "{} twice", n.name);
            assert!(std::ptr::eq(native(n.name).unwrap(), n));
            assert!(
                n.signature.starts_with(n.name),
                "{}: {}",
                n.name,
                n.signature
            );
        }
        assert!(native("nope").is_none());
    }

    /// `signature` and `effect` are what the reference prints: every
    /// native with a row of its own in `docs/language.md`'s table reads
    /// there as it does here (the row's code spans unquoted), so neither
    /// can change without the other.
    #[test]
    fn a_native_reads_as_its_reference_row() {
        let doc =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/language.md"))
                .expect("the reference");
        let spans = |cell: &str| -> Option<String> {
            let inner = cell.strip_prefix('`')?.strip_suffix('`')?;
            (!inner.contains('`')).then(|| inner.to_string())
        };
        let mut rows = std::collections::HashMap::new();
        for line in doc.lines() {
            let Some(inner) = line.strip_prefix("| ").and_then(|l| l.strip_suffix(" |")) else {
                continue;
            };
            let cells: Vec<&str> = inner.split(" | ").collect();
            if let [name, signature, effect] = cells[..] {
                if let (Some(name), Some(signature)) = (spans(name), spans(signature)) {
                    rows.insert(name, (signature, effect.replace('`', "")));
                }
            }
        }
        let mut compared = 0;
        for n in natives() {
            if let Some((signature, effect)) = rows.get(n.name) {
                assert_eq!(n.signature, signature, "the signature of {}", n.name);
                assert_eq!(n.effect, effect, "the effect of {}", n.name);
                compared += 1;
            }
        }
        // The natives the reference lists one to a row, so a table the
        // reader stops recognizing fails rather than comparing nothing.
        assert_eq!(compared, 56);
    }

    /// The JSON string form, with the C1 controls escaped as well, in the
    /// render crate's lowercase hex; everything else as itself.
    #[test]
    fn quoted_is_the_json_string_form_with_the_c1_controls_escaped() {
        assert_eq!(quote(""), "\"\"");
        assert_eq!(quote("plain"), "\"plain\"");
        assert_eq!(
            quote("q\" b\\ n\n r\r t\t bs\u{8} ff\u{c} nul\0 c1\u{1} us\u{1f}"),
            "\"q\\\" b\\\\ n\\n r\\r t\\t bs\\b ff\\f nul\\u0000 c1\\u0001 us\\u001f\""
        );
        assert_eq!(
            quote("del\u{7f} pad\u{80} apc\u{9f} nbsp\u{a0} é 日本 🚀 /"),
            "\"del\\u007f pad\\u0080 apc\\u009f nbsp\u{a0} é 日本 🚀 /\""
        );
        // What the render crate writes for the same string, where both
        // escape: the two agree on JSON's own escapes.
        let json = crate::shared::Datum::String("q\" \\ \n \u{1f} é".into()).to_string();
        assert_eq!(quote("q\" \\ \n \u{1f} é"), json);
        // The length counted before building is the length built.
        for s in [
            "",
            "plain",
            "q\" \\ \n \u{1f} é",
            "del\u{7f} pad\u{80} 日本 🚀",
            "nonchar\u{fffe}\u{ffff}\u{fffd}",
        ] {
            assert_eq!(quoted_len(s), quote(s).len(), "{s:?}");
        }
        // The two noncharacters YAML's printable set and XML's characters
        // exclude are escaped; U+FFFD, a character, is not.
        assert_eq!(
            quote("\u{fffe}\u{ffff}\u{fffd}"),
            "\"\\ufffe\\uffff\u{fffd}\""
        );
    }

    /// `chars-within` counts its work in comparisons, so ranges from data
    /// as many as they are take evaluation steps, and a bound on steps
    /// (the fuel, the abort flag) stops a test that has not yet read 4,096
    /// characters.
    #[test]
    fn chars_within_takes_a_step_per_comparisons() {
        // A runtime of its own for each case, with ten steps of fuel.
        let fueled = || {
            let rt = crate::lower::tests::runtime("def export [input] input");
            match std::sync::Arc::try_unwrap(rt) {
                Ok(rt) => std::sync::Arc::new(rt.with_fuel(Some(10))),
                Err(_) => unreachable!("the test holds the one runtime"),
            }
        };
        let span = SourceSpan::new(&Arc::from("t.alc"), 0, 0);
        let range = Val::vector(vec![Val::num(5.0), Val::num(5.0)]);
        let ranges = Val::vector(vec![range; 100_000]);
        let s = Val::str("a");
        let fail = chars_within(&fueled(), &[ranges, s], &span).unwrap_err();
        assert_eq!(
            fail.limit.as_ref().map(|l| l.name),
            Some("max_plan_steps"),
            "{fail}"
        );
        // A long string against a few ranges is counted the same way: two
        // ranges read take no step, and the comparisons alone, two for each
        // character, exhaust the fuel.
        let few = Val::vector(vec![
            Val::vector(vec![Val::num(0.0), Val::num(1.0)]),
            Val::vector(vec![Val::num(97.0), Val::num(97.0)]),
        ]);
        let long = Val::str(&"a".repeat(CHARS_WITHIN_STEP * 6));
        let fail = chars_within(&fueled(), &[few, long], &span).unwrap_err();
        assert_eq!(
            fail.limit.as_ref().map(|l| l.name),
            Some("max_plan_steps"),
            "{fail}"
        );
    }

    #[test]
    fn unquote_reads_back_what_quote_writes() {
        for s in [
            "",
            "plain",
            "q\" b\\ n\n r\r t\t bs\u{8} ff\u{c} nul\0 us\u{1f}",
            "del\u{7f} pad\u{80} apc\u{9f} nbsp\u{a0} é 日本 🚀 /",
            "\u{fffe}\u{ffff}",
        ] {
            assert_eq!(unquote(&quote(s)).as_deref(), Some(s), "{s:?}");
        }
        // JSON's other spellings: the solidus, uppercase hex, a surrogate
        // pair.
        assert_eq!(
            unquote(r#""\/\u00E9\ud83d\ude80""#).as_deref(),
            Some("/é🚀")
        );
        // Not a double-quoted form.
        for text in [
            "",
            "\"",
            "plain",
            "\"open",
            "\"in\"side\"",
            "\"raw\nline\"",
            "\"bad \\x escape\"",
            "\"short \\u12\"",
            "\"lone \\ud800\"",
            "\"low \\udc00 first\"",
            "\"pair \\ud800\\u0041\"",
        ] {
            assert_eq!(unquote(text), None, "{text:?}");
        }
    }

    /// `unquoted` reads its input an evaluation step per [`UNQUOTED_STEP`]
    /// characters as it goes, as `length` counts a long string in steps:
    /// ten steps of fuel read ten times that many characters and no more,
    /// whether or not the string turns out to be a quoted form, and a
    /// string refused at its start costs no more than its first step.
    #[test]
    fn unquoted_takes_a_step_per_characters_read() {
        // A runtime of its own for each case, with ten steps of fuel.
        let fueled = || {
            let rt = crate::lower::tests::runtime("def export [input] input");
            match std::sync::Arc::try_unwrap(rt) {
                Ok(rt) => std::sync::Arc::new(rt.with_fuel(Some(10))),
                Err(_) => unreachable!("the test holds the one runtime"),
            }
        };
        let span = SourceSpan::new(&Arc::from("t.alc"), 0, 0);
        let run = |text: String| unquoted(&fueled(), &[Val::Str(text.into())], &span);
        // A quoted form `n` characters long, its quotes included.
        let quoted = |n: usize, c: &str| format!("\"{}\"", c.repeat(n - 2));
        let limit = |fail: &Fail| fail.limit.as_ref().map(|l| l.name);
        // A character is a character, however many bytes it takes.
        for c in ["a", "\u{1f680}"] {
            assert_eq!(
                run(quoted(10 * UNQUOTED_STEP, c)).unwrap(),
                Val::str(&c.repeat(10 * UNQUOTED_STEP - 2)),
                "{c}"
            );
        }
        // One character more takes an eleventh step.
        let fail = run(quoted(10 * UNQUOTED_STEP + 1, "a")).unwrap_err();
        assert_eq!(limit(&fail), Some("max_plan_steps"), "{fail}");
        // A string with no closing quote is read as far before it is
        // refused.
        let fail = run(format!("\"{}", "a".repeat(10 * UNQUOTED_STEP))).unwrap_err();
        assert_eq!(limit(&fail), Some("max_plan_steps"), "{fail}");
        let fail = run(format!("x{}", "a".repeat(100 * UNQUOTED_STEP))).unwrap_err();
        assert_eq!(fail.code, Code::InputInvalid, "{fail}");
    }

    /// `is-number` answers whether `number` reads a string, and `number`
    /// fails exactly where it answers false; a string that is not one is
    /// the same type error to both.
    #[test]
    fn is_number_answers_whether_number_reads_the_string() {
        let rt = crate::lower::tests::runtime("");
        let at = SourceSpan::new(&Arc::from("t"), 0, 0);
        let numbers = [
            "0",
            "-0",
            "12",
            "1.5",
            "-1.5e10",
            "2E-3",
            "1e+5",
            "0e0",
            "1e5",
            "1e999",
            "12345678901234567890",
            "Infinity",
            "-Infinity",
            "NaN",
        ];
        let others = [
            "",
            "-",
            "01",
            "-01",
            "1.",
            ".5",
            "+1",
            "1e",
            "1e+",
            "1-2",
            " 1",
            "1 ",
            "0x10",
            "1_000",
            "1.5e",
            "-NaN",
            "nan",
            "infinity",
            "Inf",
            "Infinityx",
            "NaNN",
            "١",
        ];
        for (text, reads) in numbers
            .iter()
            .map(|t| (t, true))
            .chain(others.iter().map(|t| (t, false)))
        {
            let s = Val::str(text);
            assert_eq!(
                is_number(&rt, std::slice::from_ref(&s), &at).unwrap(),
                Val::Bool(reads),
                "{text:?}"
            );
            match number(&rt, &[s], &at) {
                Ok(_) => assert!(reads, "{text:?} is read"),
                Err(fail) => {
                    assert!(!reads, "{text:?}: {fail}");
                    assert_eq!(fail.code, Code::InputInvalid, "{fail}");
                }
            }
        }
        // A finite number keeps its text as the lexeme; the non-finite
        // ones have none.
        let read = |text: &str| match number(&rt, &[Val::str(text)], &at).unwrap() {
            Val::Num { value, lexeme } => (value, lexeme.map(|l| l.to_string())),
            other => panic!("{other:?}"),
        };
        assert_eq!(read("1.50"), (1.5, Some("1.50".to_string())));
        assert_eq!(read("-Infinity"), (f64::NEG_INFINITY, None));
        assert_eq!(read("1e999"), (f64::INFINITY, None));
        for (name, op) in [("is-number", is_number as Call), ("number", number)] {
            let fail = op(&rt, &[Val::num(1.0)], &at).unwrap_err();
            assert_eq!(fail.code, Code::DslTypeError, "{fail}");
            assert_eq!(
                fail.message,
                format!("type_mismatch: {name}: the string must be a string, not a number")
            );
        }
    }

    /// `number` and `is-number` read their input an evaluation step per
    /// [`NUMBER_STEP`] characters as they go, as `unquoted` reads its own:
    /// ten steps of fuel read ten times that many characters and no more,
    /// whether or not the string turns out to spell a number, and a string
    /// refused at its start costs no more than its first step.
    #[test]
    fn number_and_is_number_take_a_step_per_characters_read() {
        // A runtime of its own for each case, with ten steps of fuel.
        let fueled = || {
            let rt = crate::lower::tests::runtime("def export [input] input");
            match std::sync::Arc::try_unwrap(rt) {
                Ok(rt) => std::sync::Arc::new(rt.with_fuel(Some(10))),
                Err(_) => unreachable!("the test holds the one runtime"),
            }
        };
        let span = SourceSpan::new(&Arc::from("t.alc"), 0, 0);
        let is = |text: &str| is_number(&fueled(), &[Val::str(text)], &span);
        let read = |text: &str| number(&fueled(), &[Val::str(text)], &span);
        let limit = |fail: &Fail| fail.limit.as_ref().map(|l| l.name);
        // A number `n` characters long.
        let long = |n: usize| format!("0.{}", "5".repeat(n - 2));
        assert_eq!(is(&long(10 * NUMBER_STEP)).unwrap(), Val::Bool(true));
        let n = read(&long(10 * NUMBER_STEP)).unwrap();
        assert!(
            matches!(
                n,
                Val::Num {
                    lexeme: Some(_),
                    ..
                }
            ),
            "{n:?}"
        );
        // One character more takes an eleventh step, and so does a
        // character after as many that ends the spelling: the string is
        // read as far before it is refused.
        for text in [
            long(10 * NUMBER_STEP + 1),
            format!("{}x", long(10 * NUMBER_STEP)),
        ] {
            for fail in [is(&text).unwrap_err(), read(&text).unwrap_err()] {
                assert_eq!(limit(&fail), Some("max_plan_steps"), "{fail}");
            }
        }
        let early = format!("x{}", long(100 * NUMBER_STEP));
        assert_eq!(is(&early).unwrap(), Val::Bool(false));
        assert_eq!(read(&early).unwrap_err().code, Code::InputInvalid);
    }

    /// `split` answers the strings between a separator's occurrences,
    /// found from the front, none overlapping another: one more than it
    /// occurs, an empty one where two meet or at either end. The separator
    /// is a literal that must not be empty.
    #[test]
    fn split_answers_the_strings_between_the_separators() {
        let rt = crate::lower::tests::runtime("");
        let at = SourceSpan::new(&Arc::from("t"), 0, 0);
        let parts = |separator: &str, s: &str| -> Vec<String> {
            match split(&rt, &[Val::str(separator), Val::str(s)], &at).unwrap() {
                Val::Vector(items) => items
                    .iter()
                    .map(|v| match v {
                        Val::Str(s) => s.to_string(),
                        other => panic!("{other:?}"),
                    })
                    .collect(),
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(parts(".", "a..b"), ["a", "", "b"]);
        assert_eq!(parts(".", ""), [""]);
        assert_eq!(parts(".", "abc"), ["abc"]);
        assert_eq!(parts(".", ".a."), ["", "a", ""]);
        assert_eq!(parts("::", "a::b::c"), ["a", "b", "c"]);
        assert_eq!(parts("aa", "aaa"), ["", "a"]);
        assert_eq!(parts("→", "α→β→γ"), ["α", "β", "γ"]);
        for (args, message) in [
            (
                [Val::str(""), Val::str("a")],
                "the separator must be a non-empty string, not the empty string",
            ),
            (
                [Val::num(1.0), Val::str("a")],
                "the separator must be a string, not a number",
            ),
            (
                [Val::str("."), Val::num(1.0)],
                "the string must be a string, not a number",
            ),
        ] {
            let fail = split(&rt, &args, &at).unwrap_err();
            assert_eq!(fail.code, Code::DslTypeError, "{fail}");
            assert_eq!(fail.message, format!("type_mismatch: split: {message}"));
        }
    }

    /// `split` takes an evaluation step for each [`SPLIT_STEP`] characters
    /// it reads, or part of them, and one for each part: ten steps of fuel
    /// read nine times that many characters into one part, or split nine
    /// parts off eight separators, and no more.
    #[test]
    fn split_takes_a_step_per_characters_read_and_per_part() {
        // A runtime of its own for each case, with ten steps of fuel.
        let fueled = || {
            let rt = crate::lower::tests::runtime("def export [input] input");
            match std::sync::Arc::try_unwrap(rt) {
                Ok(rt) => std::sync::Arc::new(rt.with_fuel(Some(10))),
                Err(_) => unreachable!("the test holds the one runtime"),
            }
        };
        let span = SourceSpan::new(&Arc::from("t.alc"), 0, 0);
        let run = |s: String| split(&fueled(), &[Val::str("."), Val::str(&s)], &span);
        let limit = |fail: &Fail| fail.limit.as_ref().map(|l| l.name);
        // A character is a character, however many bytes it takes.
        for c in ["a", "\u{1f680}"] {
            assert!(run(c.repeat(9 * SPLIT_STEP)).is_ok(), "{c}");
            let fail = run(c.repeat(9 * SPLIT_STEP + 1)).unwrap_err();
            assert_eq!(limit(&fail), Some("max_plan_steps"), "{fail}");
        }
        assert!(run(".".repeat(8)).is_ok());
        let fail = run(".".repeat(9)).unwrap_err();
        assert_eq!(limit(&fail), Some("max_plan_steps"), "{fail}");
    }

    /// A runtime of its own with ten steps of fuel, for a native's steps.
    fn fueled() -> Arc<Runtime> {
        let rt = crate::lower::tests::runtime("def export [input] input");
        match Arc::try_unwrap(rt) {
            Ok(rt) => Arc::new(rt.with_fuel(Some(10))),
            Err(_) => unreachable!("the test holds the one runtime"),
        }
    }

    fn numbers(v: Val) -> Vec<f64> {
        match v {
            Val::Vector(items) => items
                .iter()
                .map(|v| match v {
                    Val::Num { value, .. } => *value,
                    other => panic!("{other:?}"),
                })
                .collect(),
            other => panic!("{other:?}"),
        }
    }

    fn nums(ns: &[f64]) -> Val {
        Val::vector(ns.iter().map(|n| Val::num(*n)).collect())
    }

    /// `push-all` joins two vectors, the vector's items first, and leaves
    /// both as they were; what is not a vector is a type error naming the
    /// argument.
    #[test]
    fn push_all_joins_two_vectors() {
        let rt = crate::lower::tests::runtime("");
        let at = SourceSpan::new(&Arc::from("t"), 0, 0);
        let join = |items: &[f64], vector: &[f64]| {
            numbers(push_all(&rt, &[nums(items), nums(vector)], &at).unwrap())
        };
        assert_eq!(join(&[3.0, 4.0], &[1.0, 2.0]), [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(join(&[], &[1.0]), [1.0]);
        assert_eq!(join(&[1.0], &[]), [1.0]);
        assert!(join(&[], &[]).is_empty());
        // Past a leaf of the persistent vector, from both sides.
        let long: Vec<f64> = (0..70).map(f64::from).collect();
        assert_eq!(join(&long[40..], &long[..40]), long);
        let (items, vector) = (nums(&[2.0]), nums(&[1.0]));
        push_all(&rt, &[items.clone(), vector.clone()], &at).unwrap();
        assert_eq!(numbers(items), [2.0]);
        assert_eq!(numbers(vector), [1.0]);
        for (args, message) in [
            (
                [Val::num(1.0), nums(&[])],
                "the items must be a vector, not a number",
            ),
            (
                [nums(&[]), Val::str("a")],
                "the data must be a vector, not a string",
            ),
        ] {
            let fail = push_all(&rt, &args, &at).unwrap_err();
            assert_eq!(fail.code, Code::DslTypeError, "{fail}");
            assert_eq!(fail.message, format!("type_mismatch: push-all: {message}"));
        }
    }

    /// `push-all` takes a step for each item it appends, and none for the
    /// vector it extends: ten steps of fuel append ten items and no more.
    #[test]
    fn push_all_takes_a_step_per_item_appended() {
        let at = SourceSpan::new(&Arc::from("t.alc"), 0, 0);
        let run = |n: u32, held: u32| {
            let items: Vec<f64> = (0..n).map(f64::from).collect();
            let vector: Vec<f64> = (0..held).map(f64::from).collect();
            push_all(&fueled(), &[nums(&items), nums(&vector)], &at)
        };
        assert!(run(10, 1000).is_ok());
        let fail = run(11, 0).unwrap_err();
        assert_eq!(
            fail.limit.as_ref().map(|l| l.name),
            Some("max_plan_steps"),
            "{fail}"
        );
    }

    /// `inc` answers the count one more, exactly up to 2^53 - 1, and
    /// refuses a count that is not a whole number of at least 0 below it,
    /// as `repeat` refuses its count.
    #[test]
    fn inc_counts_one_more_exactly() {
        let rt = crate::lower::tests::runtime("");
        let at = SourceSpan::new(&Arc::from("t"), 0, 0);
        let next = |v: Val| match inc(&rt, &[v], &at).unwrap() {
            Val::Num { value, lexeme } => {
                assert_eq!(lexeme, None);
                value
            }
            other => panic!("{other:?}"),
        };
        assert_eq!(next(Val::num(0.0)), 1.0);
        assert_eq!(next(Val::num(-0.0)), 1.0);
        assert_eq!(next(Val::num(41.0)), 42.0);
        assert_eq!(next(Val::num(INC_MAX - 1.0)), INC_MAX);
        let lexeme = Val::Num {
            value: 100.0,
            lexeme: Some(Arc::from("1e2")),
        };
        assert_eq!(next(lexeme), 101.0);
        let spelled = Val::Num {
            value: 0.5,
            lexeme: Some(Arc::from("5e-1")),
        };
        for (arg, message) in [
            (
                Val::num(INC_MAX),
                "the count must be a whole number from 0 to 9007199254740990, not 9007199254740991",
            ),
            (
                Val::num(-1.0),
                "the count must be a whole number from 0 to 9007199254740990, not -1",
            ),
            (
                Val::num(1.5),
                "the count must be a whole number from 0 to 9007199254740990, not 1.5",
            ),
            (
                spelled,
                "the count must be a whole number from 0 to 9007199254740990, not 5e-1",
            ),
            (
                Val::num(f64::INFINITY),
                "the count must be a whole number from 0 to 9007199254740990, not inf",
            ),
            (
                Val::num(f64::NAN),
                "the count must be a whole number from 0 to 9007199254740990, not NaN",
            ),
            (Val::str("1"), "the count must be a number, not a string"),
        ] {
            let fail = inc(&rt, &[arg], &at).unwrap_err();
            assert_eq!(fail.code, Code::DslTypeError, "{fail}");
            assert_eq!(fail.message, format!("type_mismatch: inc: {message}"));
        }
    }

    /// `equal` compares as a `match` pattern compares a constant: data
    /// structurally, numbers by value, a record's members in any order;
    /// a function equals nothing, and a stream or a text is refused.
    #[test]
    fn equal_compares_values_as_a_pattern_compares_a_constant() {
        let rt = crate::lower::tests::runtime("");
        let at = SourceSpan::new(&Arc::from("t"), 0, 0);
        let eq = |a: Val, b: Val| match equal(&rt, &[a, b], &at).unwrap() {
            Val::Bool(b) => b,
            other => panic!("{other:?}"),
        };
        let rec = |fields: &[(&str, Val)]| {
            Val::Record(Arc::new(
                fields
                    .iter()
                    .map(|(k, v)| (Arc::from(*k), v.clone()))
                    .collect(),
            ))
        };
        assert!(eq(Val::str("FEN"), Val::str("FEN")));
        assert!(!eq(Val::str("FEN"), Val::str("Fen")));
        assert!(!eq(Val::str("1"), Val::num(1.0)));
        assert!(eq(Val::Null, Val::Null));
        assert!(!eq(Val::Null, Val::missing()));
        assert!(eq(Val::missing(), Val::missing()));
        assert!(eq(Val::keyword("w"), Val::keyword("w")));
        assert!(!eq(Val::keyword("w"), Val::str("w")));
        assert!(eq(Val::Bool(true), Val::Bool(true)));
        // Numbers by value, whatever their lexemes.
        let spelled = Val::Num {
            value: 100.0,
            lexeme: Some(Arc::from("1e2")),
        };
        assert!(eq(spelled, Val::num(100.0)));
        assert!(eq(Val::num(-0.0), Val::num(0.0)));
        assert!(!eq(Val::num(f64::NAN), Val::num(f64::NAN)));
        // Vectors item by item; records member by member, in any order.
        assert!(eq(nums(&[1.0, 2.0]), nums(&[1.0, 2.0])));
        assert!(!eq(nums(&[1.0, 2.0]), nums(&[2.0, 1.0])));
        assert!(!eq(nums(&[1.0]), nums(&[1.0, 1.0])));
        let ab = rec(&[("a", Val::num(1.0)), ("b", nums(&[2.0]))]);
        let ba = rec(&[("b", nums(&[2.0])), ("a", Val::num(1.0))]);
        assert!(eq(ab.clone(), ba));
        assert!(!eq(
            ab.clone(),
            rec(&[("a", Val::num(1.0)), ("c", nums(&[2.0]))])
        ));
        assert!(!eq(ab, rec(&[("a", Val::num(1.0))])));
        // Tagged values by tag and fields.
        let key = |k: &str| Val::tagged("key", vec![Val::str(k)]);
        assert!(eq(key("a"), key("a")));
        assert!(!eq(key("a"), key("b")));
        assert!(!eq(key("a"), Val::tagged("scalar", vec![Val::str("a")])));
        let start = || Val::tagged("object-start", vec![]);
        assert!(eq(start(), start()));
        assert!(!eq(start(), Val::tagged("object-end", vec![])));
        let nested = || {
            Val::vector(vec![
                start(),
                Val::vector(vec![Val::tagged("array-end", vec![])]),
            ])
        };
        assert!(eq(nested(), nested()));
        // A function equals nothing, itself included.
        let f = Val::Fn(Func::Native(native("count").unwrap()));
        assert!(!eq(f.clone(), f));
        // A stream or a text is refused, either side.
        let t = text(&rt, &[Val::str("x")], &at).unwrap();
        for args in [[t.clone(), Val::str("x")], [Val::str("x"), t]] {
            let fail = equal(&rt, &args, &at).unwrap_err();
            assert_eq!(fail.code, Code::DslTypeError, "{fail}");
            assert_eq!(
                fail.message,
                "type_mismatch: equal: a text cannot be compared; a stream or a text is used where it is, not inspected"
            );
        }
    }

    /// `equal` takes a step for each pair of values it compares, depth
    /// first, and stops at the first that differs: ten steps of fuel
    /// compare two vectors of nine equal items, and two that differ at
    /// their first item of any length.
    #[test]
    fn equal_takes_a_step_per_pair_compared() {
        let at = SourceSpan::new(&Arc::from("t.alc"), 0, 0);
        let run = |a: &[f64], b: &[f64]| equal(&fueled(), &[nums(a), nums(b)], &at);
        let nine = [7.0; 9];
        assert!(matches!(run(&nine, &nine), Ok(Val::Bool(true))));
        let ten = [7.0; 10];
        let fail = run(&ten, &ten).unwrap_err();
        assert_eq!(
            fail.limit.as_ref().map(|l| l.name),
            Some("max_plan_steps"),
            "{fail}"
        );
        let mut differ = [7.0; 100];
        differ[0] = 8.0;
        assert!(matches!(run(&differ, &[7.0; 100]), Ok(Val::Bool(false))));
        // Two steps, the vectors and their first items, with fuel for two.
        let two = || {
            let rt = crate::lower::tests::runtime("def export [input] input");
            match Arc::try_unwrap(rt) {
                Ok(rt) => Arc::new(rt.with_fuel(Some(2))),
                Err(_) => unreachable!("the test holds the one runtime"),
            }
        };
        let wide = equal(&two(), &[nums(&differ), nums(&[7.0; 100])], &at);
        assert!(matches!(wide, Ok(Val::Bool(false))), "{wide:?}");
        // A member the other record lacks ends the walk where it reaches
        // that member: the records, a's vectors and their two items, four
        // steps, then b, which the other lacks.
        let rec = |last: &str| {
            Val::Record(Arc::new(
                [("a", nums(&[1.0, 2.0])), (last, Val::num(1.0))]
                    .iter()
                    .map(|(k, v)| (Arc::from(*k), v.clone()))
                    .collect(),
            ))
        };
        let rt = fueled();
        for _ in 0..2 {
            assert!(matches!(
                equal(&rt, &[rec("b"), rec("c")], &at),
                Ok(Val::Bool(false))
            ));
        }
        assert!(equal(&rt, &[rec("b"), rec("c")], &at).is_err());
        // Nested: the vector, its first item's vector and the two items
        // in it, then the second item: five steps.
        let nested = |x: f64| Val::vector(vec![nums(&[1.0, 2.0]), Val::num(x)]);
        let rt = fueled();
        assert!(matches!(
            equal(&rt, &[nested(3.0), nested(3.0)], &at),
            Ok(Val::Bool(true))
        ));
        assert!(matches!(
            equal(&rt, &[nested(3.0), nested(3.0)], &at),
            Ok(Val::Bool(true))
        ));
        assert!(equal(&rt, &[nested(3.0), nested(3.0)], &at).is_err());
    }

    /// `kind` names every retained value's kind by one keyword, the word
    /// the messages use.
    #[test]
    fn kind_names_a_value_by_one_keyword() {
        let kind_of = |v: Val| kind_word(&v).unwrap().to_string();
        assert_eq!(kind_of(Val::Null), "null");
        assert_eq!(kind_of(Val::Bool(true)), "boolean");
        assert_eq!(kind_of(Val::num(1.5)), "number");
        assert_eq!(kind_of(Val::str("s")), "string");
        assert_eq!(kind_of(Val::keyword("k")), "keyword");
        assert_eq!(kind_of(Val::vector(vec![])), "vector");
        assert_eq!(kind_of(Val::missing()), "missing");
        assert_eq!(kind_of(Val::tagged("key", vec![Val::str("a")])), "tagged");
    }

    /// NaN and the infinities, which no literal spells: `number-class`
    /// names each, and `compare` orders the infinities and leaves NaN
    /// unordered.
    #[test]
    fn number_class_and_compare_see_the_non_finite_numbers() {
        let rt = crate::lower::tests::runtime("");
        let at = SourceSpan::new(&Arc::from("t"), 0, 0);
        let word = |v: Val| match v {
            Val::Keyword(k) => k.to_string(),
            other => panic!("{other:?}"),
        };
        let class = |v: f64| word(number_class(&rt, &[Val::num(v)], &at).unwrap());
        assert_eq!(class(1.5), "finite");
        assert_eq!(class(-0.0), "finite");
        assert_eq!(class(f64::INFINITY), "infinity");
        assert_eq!(class(f64::NEG_INFINITY), "negative-infinity");
        assert_eq!(class(f64::NAN), "nan");
        let order = |x: f64, y: f64| word(compare(&rt, &[Val::num(x), Val::num(y)], &at).unwrap());
        assert_eq!(order(f64::NAN, 1.0), "unordered");
        assert_eq!(order(1.0, f64::NAN), "unordered");
        assert_eq!(order(-0.0, 0.0), "equal");
        assert_eq!(order(f64::NEG_INFINITY, -1e308), "less");
        assert_eq!(order(f64::INFINITY, f64::INFINITY), "equal");
    }

    /// `length` counts characters a chunk at a time, however a chunk cuts
    /// a character, and takes an evaluation step per chunk, so the host's
    /// abort flag stops the count of a long string.
    #[test]
    fn length_counts_characters_in_steps_the_abort_flag_reads() {
        let at = SourceSpan::new(&Arc::from("t"), 0, 0);
        let long = Val::str(&"héllo 日本".repeat(LENGTH_CHUNK / 3));
        let rt = crate::lower::tests::runtime("");
        assert_eq!(
            length(&rt, std::slice::from_ref(&long), &at).unwrap(),
            Val::num((8 * (LENGTH_CHUNK / 3)) as f64)
        );
        let flag = crate::shared::AbortFlag::new();
        let rt = crate::lower::tests::runtime_with_abort("", flag.clone());
        flag.abort();
        let huge = Val::str(&"k".repeat(LENGTH_CHUNK * 64));
        let fail = length(&rt, &[huge], &at).unwrap_err();
        assert_eq!(fail.code, Code::Aborted, "{fail}");
    }

    #[test]
    fn constants_take_no_arguments_and_constructors_tag_with_their_name() {
        for n in natives() {
            match n.kind {
                Kind::Constant => assert_eq!(n.arity, Exact(0), "{}", n.name),
                Kind::Constructor => {
                    assert!(n.arity.accepts(2) || n.arity.accepts(1), "{}", n.name)
                }
                Kind::Function => {}
            }
        }
    }

    #[test]
    fn numbers_print_as_the_renderers_print_them() {
        assert_eq!(shortest_number(1.0), "1");
        assert_eq!(shortest_number(50.25), "50.25");
        assert_eq!(shortest_number(1e20), "100000000000000000000");
        assert_eq!(shortest_number(1e21), "1e21");
        assert_eq!(shortest_number(1e-7), "1e-7");
        assert_eq!(shortest_number(-0.0), "-0");
        assert_eq!(number_text(1.5, Some("1.50")).unwrap(), "1.50");
        assert_eq!(
            number_text(1.0, Some("01")).unwrap_err().code,
            Code::InvalidNumber
        );
        assert_eq!(
            number_text(f64::INFINITY, Some("1e999")).unwrap_err().code,
            Code::TargetValueUnrepresentable
        );
        assert_eq!(
            number_text(f64::NAN, None).unwrap_err().code,
            Code::TargetValueUnrepresentable
        );
    }
}
