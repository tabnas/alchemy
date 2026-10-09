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
//! that is not a string) are `INPUT_INVALID`, the code the native table
//! transducer raises for the same shapes, so the interpreted and the
//! native standard library fail alike; a value of the wrong kind where
//! the program, not the data, chose it is `DSL_TYPE_ERROR`.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use indexmap::IndexMap;

use crate::ast::SourceSpan;
use crate::interp::Runtime;
use crate::lex::is_json_number;
use crate::shared::{CaptureSpec, Code, Fail, Selector};
use crate::value::{type_error, Func, NonFinite, Partial, Plan, Seq, Val, MISSING};

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
    static INDEX: OnceLock<HashMap<&'static str, &'static Native>> = OnceLock::new();
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

fn as_str<'a>(op: &str, what: &str, v: &'a Val) -> Result<&'a Arc<str>, Fail> {
    match v {
        Val::Str(s) => Ok(s),
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
        Val::Stream(p) => Ok(p),
        other => Err(type_error(format!(
            "{op}: the data must be a stream, not {}",
            other.kind()
        ))),
    }
}

fn as_seq(op: &str, v: &Val) -> Result<Seq, Fail> {
    match v {
        Val::Vector(items) => Ok(Seq::Vector(items.clone())),
        Val::Stream(p) => Ok(Seq::Stream(p.clone())),
        other => Err(type_error(format!(
            "{op}: the data must be a vector or a stream, not {}",
            other.kind()
        ))),
    }
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
fn as_items<'a>(op: &str, v: &'a Val) -> Result<&'a Arc<[Val]>, Fail> {
    match v {
        Val::Vector(items) => Ok(items),
        other => Err(type_error(format!(
            "{op}: the data must be a vector, not {}",
            other.kind()
        ))),
    }
}

/// A non-negative whole number that fits an index.
fn as_index(v: &Val) -> Option<usize> {
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

/// The magnitudes written positionally, as the renderers write them:
/// from `1e-6` up to, not including, `1e21`.
const POSITIONAL_MIN: f64 = 1e-6;
const POSITIONAL_MAX: f64 = 1e21;

/// The shortest text that reads back as `value`, laid out as the render
/// crate lays it out (`number::write_value` there): positional within the
/// JavaScript range, exponent form outside it. The two must agree byte for
/// byte, which the differential test checks on every number without a
/// lexeme.
pub fn shortest_number(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude == 0.0 || (POSITIONAL_MIN..POSITIONAL_MAX).contains(&magnitude) {
        format!("{value}")
    } else {
        format!("{value:e}")
    }
}

/// The text of a number as a renderer writes it, with the renderer's
/// checks: a lexeme that is not a JSON number is `INVALID_NUMBER`, a
/// non-finite value `TARGET_VALUE_UNREPRESENTABLE`.
pub fn number_text(value: f64, lexeme: Option<&str>) -> Result<String, Fail> {
    if let Some(l) = lexeme {
        if !is_json_number(l) {
            return Err(Fail::new(
                Code::InvalidNumber,
                format!("{l:?} is not a JSON number"),
            ));
        }
    }
    if !value.is_finite() {
        let message = match lexeme {
            Some(l) => format!("{l:?} is {value} as a number, which has no representation"),
            None => format!("{value} has no representation as a number"),
        };
        return Err(Fail::new(Code::TargetValueUnrepresentable, message));
    }
    Ok(match lexeme {
        Some(l) => l.to_string(),
        None => shortest_number(value),
    })
}

// ---------------------------------------------------------------------------
// The implementations
// ---------------------------------------------------------------------------

fn get(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let key: &str = match &a[0] {
        Val::Keyword(k) | Val::Str(k) => k,
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
/// is not data. The native table binds a column record by the same rule,
/// so a column function that answers something other than a record fails
/// alike both ways.
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

/// `push item vector`: a new vector, the item last. Bounded by the vector's
/// length, as the three below are: a stack of markers is as long as the
/// document's nesting, never its length.
fn push(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    if a[0].is_live() {
        return Err(type_error(format!(
            "push: a vector cannot hold {}; a stream is used once, where it is",
            a[0].live_kind()
        )));
    }
    let items = as_items("push", &a[1])?;
    let mut out = Vec::with_capacity(items.len() + 1);
    out.extend_from_slice(items);
    out.push(a[0].clone());
    Ok(Val::vector(out))
}

fn pop(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let items = as_items("pop", &a[0])?;
    let Some(last) = items.len().checked_sub(1) else {
        return Err(type_error(
            "pop: the vector is empty; there is no last item to remove",
        ));
    };
    Ok(Val::vector(items[..last].to_vec()))
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
                out.push(Val::Str(k.clone()));
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
    let s = as_str("length", "the string", &a[0])?;
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
/// it (XML's embedding writes a number as text).
fn number(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let s = as_str("number", "the string", &a[0])?;
    let value = match &**s {
        "Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        "NaN" => f64::NAN,
        text if is_json_number(text) => match text.parse::<f64>() {
            Ok(v) => v,
            Err(_) => return Err(input_invalid(format!("number: {text:?} spells no number"))),
        },
        text => return Err(input_invalid(format!("number: {text:?} spells no number"))),
    };
    let lexeme = if value.is_finite() {
        Some(s.clone())
    } else {
        None
    };
    Ok(Val::Num { value, lexeme })
}

/// `unquoted string`: the string a double-quoted form spells, the reverse
/// of `quoted`: a leading and a trailing quote around characters and
/// JSON's escapes (`\"`, `\\`, `\/`, `\b`, `\f`, `\n`, `\r`, `\t` and
/// `\uXXXX`, a surrogate pair as the one character it names). Any other
/// text is INPUT_INVALID: no quotes, a quote or a control character
/// unescaped inside, another escape, or a surrogate on its own. A format's
/// reverse reads back with it what its embedding wrote with `quoted`.
fn unquoted(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let s = as_str("unquoted", "the string", &a[0])?;
    match unquote(s) {
        Some(text) => {
            rt.tick()?;
            Ok(Val::Str(Arc::from(text)))
        }
        None => Err(input_invalid(format!(
            "unquoted: {s:?} is not a double-quoted string"
        ))),
    }
}

/// The string a double-quoted form spells, or `None`.
pub fn unquote(s: &str) -> Option<String> {
    let inner = s.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    let hex4 = |chars: &mut std::str::Chars<'_>| -> Option<u32> {
        let mut v = 0;
        for _ in 0..4 {
            v = v * 16 + chars.next()?.to_digit(16)?;
        }
        Some(v)
    };
    while let Some(c) = chars.next() {
        match c {
            '"' => return None,
            c if (c as u32) < 0x20 => return None,
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let high = hex4(&mut chars)?;
                    let code = match high {
                        0xD800..=0xDBFF => {
                            if chars.next()? != '\\' || chars.next()? != 'u' {
                                return None;
                            }
                            let low = hex4(&mut chars)?;
                            if !(0xDC00..=0xDFFF).contains(&low) {
                                return None;
                            }
                            0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
                        }
                        0xDC00..=0xDFFF => return None,
                        v => v,
                    };
                    out.push(char::from_u32(code)?);
                }
                _ => return None,
            },
            c => out.push(c),
        }
    }
    Some(out)
}

/// `chars-within ranges string`: whether every character of the string
/// lies within one of the ranges, each a vector `[low high]` of code
/// points, both included. The empty string is within any ranges. A
/// format's part tests a string with it against the characters the format
/// can carry (XML's `Char` production, a name's characters) before it
/// writes the string, and chooses its convention when it cannot.
fn chars_within(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let ranges = as_items("chars-within", &a[0])?;
    let mut bounds: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
    for range in ranges.iter() {
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
    let s = as_str("chars-within", "the string", &a[1])?;
    for (i, c) in s.chars().enumerate() {
        if i % 4096 == 4095 {
            rt.tick()?;
        }
        let c = c as u32;
        if !bounds.iter().any(|&(low, high)| (low..=high).contains(&c)) {
            return Ok(Val::Bool(false));
        }
    }
    Ok(Val::Bool(true))
}

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

fn property(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let name = as_str("property", "the name", &a[0])?;
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

fn join(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let sep = as_str("join", "the separator", &a[0])?.clone();
    let items = as_seq("join", &a[1])?;
    if let Seq::Vector(items) = &items {
        for item in items.iter() {
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

fn text(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let s = as_str("text", "the argument", &a[0])?.clone();
    Ok(Val::Text(Arc::new(Plan::Lit(s))))
}

fn replace_text(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let from = as_str("replace-text", "the literal to find", &a[0])?.clone();
    let to = as_str("replace-text", "the replacement", &a[1])?.clone();
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
            "scalar-text",
            ":null-text",
            &option("scalar-text", options, "null-text")?,
        )?
        .clone(),
        Val::Bool(true) => Arc::from("true"),
        Val::Bool(false) => Arc::from("false"),
        Val::Num { value, .. }
            if !value.is_finite() && non_finite(options)? != NonFinite::Reject =>
        {
            match non_finite(options)? {
                NonFinite::Null => as_str(
                    "scalar-text",
                    ":null-text",
                    &option("scalar-text", options, "null-text")?,
                )?
                .clone(),
                _ => Arc::from(NonFinite::word(*value)),
            }
        }
        Val::Num { value, lexeme } => Arc::from(number_text(*value, lexeme.as_deref())?),
        Val::Str(s) => s.clone(),
        v if v.is_missing() => match option("scalar-text", options, "missing")? {
            Val::Keyword(k) if &*k == "error" => {
                return Err(Fail::new(
                    Code::MissingValue,
                    "a cell has no value and the options map missing to :error",
                ))
            }
            Val::Str(t) => t,
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
    Ok(Val::Str(text))
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
    let s = as_str("quoted", "the string", &a[0])?;
    let max = rt.limits().max_scalar_bytes;
    let len = quoted_len(s);
    if len > max {
        return Err(Fail::limit(
            "max_scalar_bytes",
            max as u64,
            format!("quoted: the quoted form is {len} bytes, more than {max}"),
        ));
    }
    Ok(Val::Str(Arc::from(quote(s))))
}

/// `string-join separator strings`: the strings of a vector joined into
/// one string, the separator between them, so that a program can build
/// one scalar from several: a table cell from the runs of a Markdown
/// cell, a failure message that names a key. The result is one scalar,
/// so it is held to `max_scalar_bytes`, refused before it is built.
fn string_join(rt: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    let separator = as_str("string-join", "the separator", &a[0])?;
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
    let mut out = String::with_capacity(len);
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(separator);
        }
        if let Val::Str(s) = item {
            out.push_str(s);
        }
    }
    Ok(Val::Str(Arc::from(out)))
}

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
    let s = as_str("repeat", "the string", &a[1])?;
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
    Ok(Val::Str(Arc::from(s.repeat(n))))
}

fn fail(rt: &Runtime, a: &[Val], at: &SourceSpan) -> Result<Val, Fail> {
    // `(fail message)` is INPUT_INVALID, the code of a document the
    // program refuses; `(fail :code message)` names what the failure is: a
    // value the target cannot carry, or a stream that breaks its protocol.
    let (code, message) = match a {
        [message] => (Code::InputInvalid, message),
        [code, message] => {
            let code = match code {
                Val::Keyword(k) => match &**k {
                    "invalid" => Code::InputInvalid,
                    "unrepresentable" => Code::TargetValueUnrepresentable,
                    "protocol-order" => Code::ProtocolOrderError,
                    other => {
                        return Err(type_error(format!(
                            "the code of fail must be :invalid, :unrepresentable or :protocol-order, not :{other}"
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
    let message = as_str("fail", "the message", message)?;
    Err(rt.fail_at(Fail::new(code, message.to_string()), at))
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
/// built, whatever consumes it: the native table binds no more, and the
/// library's twin of it, whose events may reach no renderer that would
/// check them, is held to the same count.
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

fn key(_: &Runtime, a: &[Val], _: &SourceSpan) -> Result<Val, Fail> {
    as_str("key", "the name", &a[0])?;
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
    f("pop", Exact(1), pop, "pop vector -> Vector", "the vector without its last item; an empty vector is a type error"),
    f("top", Exact(1), top, "top vector -> Value", "the last item; an empty vector is a type error"),
    f("count", Exact(1), count, "count vector -> Number", "how many items the vector holds"),
    f("keys", Exact(1), keys, "keys record -> Vector", "the record's keys as strings, in its order, which for a captured object is the document's"),
    f("length", Exact(1), length, "length string -> Number", "how many characters the string holds"),
    f("unquoted", Exact(1), unquoted, "unquoted string -> String", "the string a double-quoted form spells, the reverse of quoted: JSON's escapes (\\\", \\\\, \\/, \\b, \\f, \\n, \\r, \\t, \\uXXXX) read, a surrogate pair as the one character it names; INPUT_INVALID for any other text (no quotes, a quote or a control character unescaped inside, another escape, a surrogate on its own)"),
    f("chars-within", Exact(2), chars_within, "chars-within ranges string -> Bool", "whether every character of the string lies within one of the ranges, each a vector [low high] of code points, both included; true for the empty string. A part tests a string with it against what its format can carry (XML's Char production, a name's characters) and chooses its convention where it cannot"),
    f("number", Exact(1), number, "number string -> Number", "the number the string spells: a JSON number, its text kept as the lexeme, or Infinity, -Infinity or NaN; INPUT_INVALID for any other text"),
    f("compare", Exact(2), compare, "compare a b -> Keyword", "how two numbers are ordered: :less, :equal or :greater, and :unordered when either is NaN"),
    f("number-class", Exact(1), number_class, "number-class number -> Keyword", ":finite, :infinity, :negative-infinity or :nan"),
    f("kind", Exact(1), kind, "kind value -> Keyword", "the kind of a value as a keyword: :null, :boolean, :number, :string, :keyword, :vector, :record, :missing, :tagged, :function, :selector or :capture; a stream or a text cannot be asked"),
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
    k("transition", Exact(2), transition, "transition state outputs -> Transition", "one step's result: the next state and a vector of outputs"),
    f("partial", AtLeast(1), partial, "partial f arg... -> Fn", "f with its first arguments supplied"),
    f("map", Exact(2), map, "map f items -> Vector | Stream", "eager over a vector; per item over a stream, retaining nothing"),
    f("filter", Exact(2), filter, "filter predicate items -> Vector | Stream", "eager over a vector; per item over a stream"),
    // Text.
    f("concat-map", Exact(2), concat_map, "concat-map f items -> Text", "f answers a string or a text per item; each item's text is assembled whole, under max_output_bytes, and written as items arrive, so a failure leaves no half item"),
    f("join", Exact(2), join, "join separator items -> Text", "the separator between items, never between the fragments of one; each item assembled as concat-map's is"),
    f("concat", AtLeast(0), concat, "concat item... -> Text", "in order, without assembling the result"),
    f("text", Exact(1), text, "text string -> Text", "a string as a text"),
    f("replace-text", Exact(3), replace_text, "replace-text from to text -> Text", "a fixed literal replaced across fragment boundaries, a finite text's as a live one's; retains at most the literal's length"),
    f("scalar-text", Exact(2), scalar_text, "scalar-text options cell -> String", "a cell's text under the options' null, missing and non-finite policies: a string as it is, a number by its lexeme (one that is not finite refused, unless the options' :non-finite is :null, the null text, or :literal, the word Infinity, -Infinity or NaN), a boolean by its name, a vector or a record as its compact JSON text (number lexemes kept, quotes as JSON writes them) under max_scalar_bytes; the native renderer writes the same cell the same way"),
    f("quoted", Exact(1), quoted, "quoted string -> String", "the double-quoted form: a leading and a trailing quote, the quote and the backslash escaped by a backslash, U+0000 to U+001F as \\n, \\t, \\r, \\b, \\f or \\u00XX, and U+007F to U+009F, U+FFFE and U+FFFF as \\uXXXX (the JSON string form, which YAML's double-quoted style reads too, plus the C1 controls and the two noncharacters its printable set excludes, which XML's characters exclude too); refused past max_scalar_bytes, before it is built"),
    f("string-join", Exact(2), string_join, "string-join separator strings -> String", "the strings of a vector joined into one string, the separator between them; refused past max_scalar_bytes, before it is built"),
    f("repeat", Exact(2), repeat, "repeat count string -> String", "the string count times over; refused past max_scalar_bytes, before it is built"),
    f("fail", Between(1, 2), fail, "fail [code] message -> Never", "INPUT_INVALID with the message and the form's position; with a code first, :unrepresentable is TARGET_VALUE_UNREPRESENTABLE (a value the target cannot carry), :protocol-order is PROTOCOL_ORDER_ERROR (a stream that breaks its protocol) and :invalid is INPUT_INVALID"),
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
    // Renderers and protocol adapters.
    f("json", Between(1, 2), json, "json [options] events -> Text", "JsonEvents, or a Stream<Event> a program built, as compact JSON text, event by event, with a final newline. A number that is not finite has no JSON form and is refused (TARGET_VALUE_UNREPRESENTABLE), unless an options record first says (entry :non-finite :null): then it is written as null. The record holds :non-finite and nothing else, :reject (the default) or :null"),
    f("records", Exact(1), records, "records table-events -> JsonEvents", "one object per row keyed by label; retains the labels"),
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
        assert_eq!(compared, 47);
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
        let rt = crate::lower::tests::runtime("", true);
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
        let rt = crate::lower::tests::runtime("", true);
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
