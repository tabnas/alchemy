//! The retained value type.
//!
//! What a capture materializes and what a projected cell holds. Distinct
//! from the engine's `Value` on purpose: a datum keeps a number's lexeme,
//! measures its own size against limits, and is owned by the transducer
//! rather than shared with a parse.

use std::fmt;

use indexmap::IndexMap;

use crate::shared::error::{Code, Fail};
use crate::shared::event::{JsonEvent, Number};
use crate::shared::limits::NODE_BYTES;
use crate::shared::selector::Segment;
use crate::shared::sink::{Flow, Sink};

/// A retained JSON-like value.
#[derive(Clone, Debug, PartialEq)]
pub enum Datum {
    Null,
    Bool(bool),
    Number {
        value: f64,
        lexeme: Option<Box<str>>,
    },
    String(Box<str>),
    Array(Vec<Datum>),
    /// Members in source order. A repeated member replaces the earlier one
    /// (last value wins) unless the builder's policy rejected it first.
    Object(IndexMap<Box<str>, Datum>),
}

impl Datum {
    /// Payload bytes plus [`NODE_BYTES`] per node: the measure limits use.
    pub fn byte_size(&self) -> usize {
        match self {
            Datum::Null | Datum::Bool(_) => NODE_BYTES,
            Datum::Number { lexeme, .. } => NODE_BYTES + lexeme.as_ref().map_or(8, |l| l.len()),
            Datum::String(s) => NODE_BYTES + s.len(),
            Datum::Array(items) => NODE_BYTES + items.iter().map(Datum::byte_size).sum::<usize>(),
            Datum::Object(members) => {
                NODE_BYTES
                    + members
                        .iter()
                        .map(|(k, v)| k.len() + v.byte_size())
                        .sum::<usize>()
            }
        }
    }

    /// The value at a concrete path below this one.
    pub fn get_path(&self, path: &[Segment]) -> Option<&Datum> {
        let mut here = self;
        for seg in path {
            here = match (seg, here) {
                (Segment::Key(k), Datum::Object(m)) => m.get(k.as_ref())?,
                (Segment::Index(i), Datum::Array(a)) => a.get(*i)?,
                _ => return None,
            };
        }
        Some(here)
    }

    /// The value at a concrete path below this one, moved out and replaced
    /// by `Null`. For a consumer that owns the datum and reads each path
    /// once: a later read of the same path, or of one below it, finds the
    /// `Null`, so the caller checks its paths are disjoint first.
    pub fn take_path(&mut self, path: &[Segment]) -> Option<Datum> {
        let mut here = self;
        for seg in path {
            here = match (seg, here) {
                (Segment::Key(k), Datum::Object(m)) => m.get_mut(k.as_ref())?,
                (Segment::Index(i), Datum::Array(a)) => a.get_mut(*i)?,
                _ => return None,
            };
        }
        Some(std::mem::replace(here, Datum::Null))
    }

    pub fn is_container(&self) -> bool {
        matches!(self, Datum::Array(_) | Datum::Object(_))
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Datum::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Datum]> {
        match self {
            Datum::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&IndexMap<Box<str>, Datum>> {
        match self {
            Datum::Object(m) => Some(m),
            _ => None,
        }
    }

    /// From an engine value. `Undefined` becomes `Null`, as the engine's
    /// own serialization has it; the metadata wrappers unwrap. With the
    /// `tabnas` feature, which the `language` feature turns on.
    #[cfg(feature = "tabnas")]
    pub fn from_tabnas(value: &tabnas::Value) -> Datum {
        match value {
            tabnas::Value::Undefined | tabnas::Value::Null => Datum::Null,
            tabnas::Value::Bool(b) => Datum::Bool(*b),
            tabnas::Value::Number(n) => Datum::Number {
                value: *n,
                lexeme: None,
            },
            tabnas::Value::String(s) => Datum::String(s.as_str().into()),
            tabnas::Value::Text(t) => Datum::String(t.string.as_str().into()),
            tabnas::Value::Array(a) => Datum::Array(a.iter().map(Datum::from_tabnas).collect()),
            tabnas::Value::ListRef(l) => {
                Datum::Array(l.value.iter().map(Datum::from_tabnas).collect())
            }
            tabnas::Value::Object(m) => Datum::Object(
                m.iter()
                    .map(|(k, v)| (k.as_str().into(), Datum::from_tabnas(v)))
                    .collect(),
            ),
            tabnas::Value::MapRef(m) => Datum::Object(
                m.value
                    .iter()
                    .map(|(k, v)| (k.as_str().into(), Datum::from_tabnas(v)))
                    .collect(),
            ),
        }
    }

    /// As a `serde_json` value, for oracles and tests. serde_json's number
    /// is an f64 (or an integer that fits), so a lexeme with more digits
    /// than that loses them here; [`Datum`]'s `Display` keeps them.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Datum::Null => serde_json::Value::Null,
            Datum::Bool(b) => serde_json::Value::Bool(*b),
            Datum::Number { value, lexeme } => lexeme
                .as_deref()
                .and_then(|l| l.parse::<serde_json::Number>().ok())
                .map(serde_json::Value::Number)
                .or_else(|| serde_json::Number::from_f64(*value).map(serde_json::Value::Number))
                .unwrap_or(serde_json::Value::Null),
            Datum::String(s) => serde_json::Value::String(s.to_string()),
            Datum::Array(a) => serde_json::Value::Array(a.iter().map(Datum::to_json).collect()),
            Datum::Object(m) => serde_json::Value::Object(
                m.iter()
                    .map(|(k, v)| (k.to_string(), v.to_json()))
                    .collect(),
            ),
        }
    }

    /// From a `serde_json` value, for tests.
    pub fn from_json(value: &serde_json::Value) -> Datum {
        match value {
            serde_json::Value::Null => Datum::Null,
            serde_json::Value::Bool(b) => Datum::Bool(*b),
            serde_json::Value::Number(n) => Datum::Number {
                value: n.as_f64().unwrap_or(f64::NAN),
                lexeme: Some(n.to_string().into()),
            },
            serde_json::Value::String(s) => Datum::String(s.as_str().into()),
            serde_json::Value::Array(a) => Datum::Array(a.iter().map(Datum::from_json).collect()),
            serde_json::Value::Object(m) => Datum::Object(
                m.iter()
                    .map(|(k, v)| (k.as_str().into(), Datum::from_json(v)))
                    .collect(),
            ),
        }
    }
}

impl fmt::Display for Datum {
    /// Compact JSON, keeping number lexemes.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        write_json(self, &mut out);
        f.write_str(&out)
    }
}

/// Append `s` as a JSON string literal, escaped as RFC 8259 requires:
/// `"` and `\\` escaped, control characters as `\\uXXXX` (with the short
/// forms for `\\b \\f \\n \\r \\t`), everything else as itself.
pub fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Append a number: its lexeme when known, else the shortest text that
/// reads back as the same f64. A non-finite value has no JSON form and is
/// written as `null`; renderers reject it before it gets here.
pub fn write_json_number(value: f64, lexeme: Option<&str>, out: &mut String) {
    match lexeme {
        Some(l) => out.push_str(l),
        None if value.is_finite() => out.push_str(&format!("{value}")),
        None => out.push_str("null"),
    }
}

/// Append a datum as compact JSON.
pub fn write_json(d: &Datum, out: &mut String) {
    match d {
        Datum::Null => out.push_str("null"),
        Datum::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Datum::Number { value, lexeme } => write_json_number(*value, lexeme.as_deref(), out),
        Datum::String(s) => write_json_string(s, out),
        Datum::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(item, out);
            }
            out.push(']');
        }
        Datum::Object(members) => {
            out.push('{');
            for (i, (k, v)) in members.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(k, out);
                out.push(':');
                write_json(v, out);
            }
            out.push('}');
        }
    }
}

/// Emit a datum as `JsonEvents/1` (without the final `End`, so a datum can
/// stand in for any part of a document).
pub fn walk_datum(datum: &Datum, sink: &mut dyn Sink) -> Result<Flow, Fail> {
    macro_rules! send {
        ($ev:expr) => {
            if sink.event($ev)? == Flow::Stop {
                return Ok(Flow::Stop);
            }
        };
    }
    match datum {
        Datum::Null => send!(JsonEvent::Null),
        Datum::Bool(b) => send!(JsonEvent::Bool(*b)),
        Datum::Number { value, lexeme } => send!(JsonEvent::Number(Number {
            value: *value,
            lexeme: lexeme.as_deref(),
        })),
        Datum::String(s) => send!(JsonEvent::String(s)),
        Datum::Array(items) => {
            send!(JsonEvent::ArrayStart);
            for item in items {
                if walk_datum(item, sink)? == Flow::Stop {
                    return Ok(Flow::Stop);
                }
            }
            send!(JsonEvent::ArrayEnd);
        }
        Datum::Object(members) => {
            send!(JsonEvent::ObjectStart);
            for (k, v) in members {
                send!(JsonEvent::Key(k));
                if walk_datum(v, sink)? == Flow::Stop {
                    return Ok(Flow::Stop);
                }
            }
            send!(JsonEvent::ObjectEnd);
        }
    }
    Ok(Flow::Continue)
}

/// How a builder treats a repeated member name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Duplicates {
    /// Fail with [`Code::DuplicateMember`].
    Reject,
    /// The later value replaces the earlier one.
    LastWins,
    /// The earlier value stays.
    FirstWins,
}

/// Builds one [`Datum`] from the events of one value, under a byte limit.
///
/// Feed it every event from the value's first to its last; `finished()`
/// says when the value is complete. Bytes are counted as they arrive, and
/// the limit fails at the first byte over it rather than after the value
/// is whole. The builder knows nothing of where in a document its value
/// sits: a failure leaves `path` unset and the stage that placed the
/// builder adds it, so no path is built for the values that succeed.
#[derive(Debug)]
pub struct DatumBuilder {
    stack: Vec<Frame>,
    done: Option<Datum>,
    bytes: usize,
    limit: usize,
    limit_name: &'static str,
    duplicates: Duplicates,
}

#[derive(Debug)]
enum Frame {
    Array(Vec<Datum>),
    Object {
        members: IndexMap<Box<str>, Datum>,
        key: Option<Box<str>>,
    },
}

impl DatumBuilder {
    /// A builder whose limit failure names `limit_name` (a `Limits` field).
    pub fn new(limit: usize, limit_name: &'static str, duplicates: Duplicates) -> Self {
        DatumBuilder {
            stack: Vec::new(),
            done: None,
            bytes: 0,
            limit,
            limit_name,
            duplicates,
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn finished(&self) -> bool {
        self.done.is_some()
    }

    /// The value, once [`finished`](Self::finished). Probing an unfinished
    /// builder returns `None` and leaves the charge for the partial value in
    /// place: the bytes it holds are still held.
    pub fn take(&mut self) -> Option<Datum> {
        let done = self.done.take();
        if done.is_some() {
            self.bytes = 0;
        }
        done
    }

    fn charge(&mut self, n: usize) -> Result<(), Fail> {
        self.bytes += n;
        if self.bytes > self.limit {
            return Err(Fail::limit(
                self.limit_name,
                self.limit as u64,
                format!("a value is larger than {} bytes", self.limit),
            ));
        }
        Ok(())
    }

    fn place(&mut self, value: Datum) -> Result<(), Fail> {
        match self.stack.last_mut() {
            None => self.done = Some(value),
            Some(Frame::Array(items)) => items.push(value),
            Some(Frame::Object { members, key }) => {
                let key = key.take().ok_or_else(|| {
                    Fail::protocol("a value arrived inside an object without a key")
                })?;
                // A repeated member was charged in full as its events arrived,
                // which is right: for a moment both were held. Whichever one
                // goes now gives its bytes back, so an object that keeps
                // repeating a small key does not grow towards the limit while
                // the value it holds stays the same size. A completed value's
                // `byte_size` is exactly what its events were charged.
                let mut released = 0;
                if let Some(existing) = members.get(&key) {
                    match self.duplicates {
                        Duplicates::Reject => {
                            return Err(Fail::new(
                                Code::DuplicateMember,
                                format!("member {key:?} appears twice"),
                            ));
                        }
                        Duplicates::FirstWins => {
                            self.bytes = self.bytes.saturating_sub(key.len() + value.byte_size());
                            return Ok(());
                        }
                        Duplicates::LastWins => released = key.len() + existing.byte_size(),
                    }
                }
                members.insert(key, value);
                self.bytes = self.bytes.saturating_sub(released);
            }
        }
        Ok(())
    }

    /// One event of the value being built.
    pub fn event(&mut self, ev: JsonEvent<'_>) -> Result<(), Fail> {
        if self.done.is_some() {
            return Err(Fail::protocol(
                "an event arrived after the value was complete",
            ));
        }
        match ev {
            JsonEvent::ObjectStart => {
                self.charge(NODE_BYTES)?;
                self.stack.push(Frame::Object {
                    members: IndexMap::new(),
                    key: None,
                });
            }
            JsonEvent::ArrayStart => {
                self.charge(NODE_BYTES)?;
                self.stack.push(Frame::Array(Vec::new()));
            }
            JsonEvent::Key(k) => {
                self.charge(k.len())?;
                match self.stack.last_mut() {
                    Some(Frame::Object { key, .. }) if key.is_none() => *key = Some(k.into()),
                    _ => return Err(Fail::protocol("a key arrived where no member was expected")),
                }
            }
            JsonEvent::ObjectEnd => match self.stack.pop() {
                Some(Frame::Object { members, key: None }) => self.place(Datum::Object(members))?,
                Some(Frame::Object { key: Some(_), .. }) => {
                    return Err(Fail::protocol(
                        "an object ended after a key without its value",
                    ))
                }
                _ => return Err(Fail::protocol("an object ended that had not started")),
            },
            JsonEvent::ArrayEnd => match self.stack.pop() {
                Some(Frame::Array(items)) => self.place(Datum::Array(items))?,
                _ => return Err(Fail::protocol("an array ended that had not started")),
            },
            JsonEvent::Null => {
                self.charge(NODE_BYTES)?;
                self.place(Datum::Null)?;
            }
            JsonEvent::Bool(b) => {
                self.charge(NODE_BYTES)?;
                self.place(Datum::Bool(b))?;
            }
            JsonEvent::Number(n) => {
                self.charge(NODE_BYTES + n.lexeme.map_or(8, str::len))?;
                self.place(Datum::Number {
                    value: n.value,
                    lexeme: n.lexeme.map(Into::into),
                })?;
            }
            JsonEvent::String(s) => {
                self.charge(NODE_BYTES + s.len())?;
                self.place(Datum::String(s.into()))?;
            }
            JsonEvent::End => {
                return Err(Fail::protocol(
                    "the document ended inside a value being captured",
                ))
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::event::OwnedJsonEvent;

    fn build(events: &[OwnedJsonEvent], limit: usize) -> Result<Datum, Fail> {
        let mut b = DatumBuilder::new(limit, "max_capture_bytes", Duplicates::Reject);
        for ev in events {
            b.event(ev.as_event())?;
        }
        assert!(b.finished());
        Ok(b.take().unwrap())
    }

    #[test]
    fn walk_and_build_round_trip() {
        let src = serde_json::json!({"a": [1, "x", null, true], "b": {"c": 2.5}});
        let d = Datum::from_json(&src);
        let mut rec: Vec<OwnedJsonEvent> = Vec::new();
        walk_datum(&d, &mut rec).unwrap();
        let back = build(&rec, usize::MAX).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.to_json(), src);
        assert_eq!(back.to_string(), r#"{"a":[1,"x",null,true],"b":{"c":2.5}}"#);
    }

    #[test]
    fn lexemes_survive() {
        let d = Datum::from_json(&serde_json::json!({"n": 50.25}));
        assert_eq!(
            d.get_path(&[Segment::Key("n".into())]).unwrap().to_string(),
            "50.25"
        );
        let big = Datum::Number {
            value: 1.2345678901234568e29,
            lexeme: Some("123456789012345678901234567890".into()),
        };
        assert_eq!(big.to_string(), "123456789012345678901234567890");
        assert_eq!(
            Datum::Number {
                value: 72.0,
                lexeme: None
            }
            .to_string(),
            "72"
        );
    }

    #[test]
    fn strings_escape_as_rfc_8259() {
        let d = Datum::String("a\"b\\c\n\u{1}\u{7f}\u{e9}".into());
        assert_eq!(d.to_string(), "\"a\\\"b\\\\c\\n\\u0001\u{7f}\u{e9}\"");
        let back: serde_json::Value = serde_json::from_str(&d.to_string()).unwrap();
        assert_eq!(
            back,
            serde_json::Value::String("a\"b\\c\n\u{1}\u{7f}\u{e9}".into())
        );
    }

    #[test]
    fn get_path() {
        let d = Datum::from_json(&serde_json::json!({"a": [{"b": 1}]}));
        let p = [
            Segment::Key("a".into()),
            Segment::Index(0),
            Segment::Key("b".into()),
        ];
        assert_eq!(d.get_path(&p).unwrap().to_string(), "1");
        assert!(d.get_path(&[Segment::Key("z".into())]).is_none());
        assert!(d.get_path(&[Segment::Index(0)]).is_none());
        assert!(d.get_path(&[]).is_some());
    }

    #[test]
    fn take_path_moves_the_value_out_and_leaves_null() {
        let mut d = Datum::from_json(&serde_json::json!({"a": [{"b": "x"}], "c": 2}));
        let p = [
            Segment::Key("a".into()),
            Segment::Index(0),
            Segment::Key("b".into()),
        ];
        assert_eq!(d.take_path(&p), Some(Datum::String("x".into())));
        assert_eq!(d.get_path(&p), Some(&Datum::Null));
        assert!(d.take_path(&[Segment::Key("z".into())]).is_none());
        assert_eq!(d.to_string(), r#"{"a":[{"b":null}],"c":2}"#);
        assert_eq!(
            d.take_path(&[]).unwrap().to_string(),
            r#"{"a":[{"b":null}],"c":2}"#
        );
        assert_eq!(d, Datum::Null);
    }

    #[test]
    fn size_and_limit() {
        let d = Datum::from_json(&serde_json::json!(["abcd", "ef"]));
        assert_eq!(d.byte_size(), NODE_BYTES * 3 + 6);
        let mut rec: Vec<OwnedJsonEvent> = Vec::new();
        walk_datum(&d, &mut rec).unwrap();
        let err = build(&rec, NODE_BYTES * 2 + 5).unwrap_err();
        assert_eq!(err.code, Code::ResourceLimitExceeded);
        assert_eq!(err.limit.as_ref().unwrap().name, "max_capture_bytes");
    }

    #[test]
    fn duplicates_policy() {
        let events = [
            OwnedJsonEvent::ObjectStart,
            OwnedJsonEvent::Key("a".into()),
            OwnedJsonEvent::Bool(true),
            OwnedJsonEvent::Key("a".into()),
            OwnedJsonEvent::Bool(false),
            OwnedJsonEvent::ObjectEnd,
        ];
        let run = |policy| {
            let mut b = DatumBuilder::new(usize::MAX, "max_capture_bytes", policy);
            for ev in &events {
                b.event(ev.as_event())?;
            }
            Ok::<Datum, Fail>(b.take().unwrap())
        };
        assert_eq!(
            run(Duplicates::Reject).unwrap_err().code,
            Code::DuplicateMember
        );
        assert_eq!(
            run(Duplicates::LastWins).unwrap().to_string(),
            r#"{"a":false}"#
        );
        assert_eq!(
            run(Duplicates::FirstWins).unwrap().to_string(),
            r#"{"a":true}"#
        );
    }

    #[test]
    fn probing_an_unfinished_builder_keeps_its_charge() {
        let mut b = DatumBuilder::new(usize::MAX, "max_capture_bytes", Duplicates::Reject);
        b.event(JsonEvent::ArrayStart).unwrap();
        b.event(JsonEvent::String("abcd")).unwrap();
        let held = b.bytes();
        assert!(b.take().is_none());
        assert_eq!(b.bytes(), held, "a None from take() releases nothing");
        b.event(JsonEvent::ArrayEnd).unwrap();
        assert!(b.take().is_some());
        assert_eq!(b.bytes(), 0);
    }

    #[test]
    fn a_repeated_member_does_not_grow_the_charge() {
        for policy in [Duplicates::FirstWins, Duplicates::LastWins] {
            let mut b = DatumBuilder::new(usize::MAX, "max_capture_bytes", policy);
            b.event(JsonEvent::ObjectStart).unwrap();
            b.event(JsonEvent::Key("k")).unwrap();
            b.event(JsonEvent::String("first")).unwrap();
            let once = b.bytes();
            for _ in 0..100 {
                b.event(JsonEvent::Key("k")).unwrap();
                b.event(JsonEvent::String("again")).unwrap();
            }
            assert_eq!(
                b.bytes(),
                once,
                "{policy:?}: the charge is the object's size"
            );
            b.event(JsonEvent::ObjectEnd).unwrap();
            let d = b.take().unwrap();
            assert_eq!(d.byte_size(), once);
        }
    }

    #[test]
    fn a_repeated_member_still_trips_the_limit_while_both_are_held() {
        // Both values are held for a moment, and that moment is what the
        // limit protects: the second value is charged before the first is
        // released.
        let mut b = DatumBuilder::new(
            NODE_BYTES * 2 + 1 + 5 + 3,
            "max_capture_bytes",
            Duplicates::LastWins,
        );
        b.event(JsonEvent::ObjectStart).unwrap();
        b.event(JsonEvent::Key("k")).unwrap();
        b.event(JsonEvent::String("first")).unwrap();
        b.event(JsonEvent::Key("k")).unwrap();
        assert_eq!(
            b.event(JsonEvent::String("second")).unwrap_err().code,
            Code::ResourceLimitExceeded
        );
    }

    #[test]
    fn protocol_errors() {
        let mut b = DatumBuilder::new(usize::MAX, "max_capture_bytes", Duplicates::Reject);
        assert_eq!(
            b.event(JsonEvent::ObjectEnd).unwrap_err().code,
            Code::ProtocolOrderError
        );
        let mut b = DatumBuilder::new(usize::MAX, "max_capture_bytes", Duplicates::Reject);
        b.event(JsonEvent::ObjectStart).unwrap();
        assert_eq!(
            b.event(JsonEvent::Null).unwrap_err().code,
            Code::ProtocolOrderError
        );
        let mut b = DatumBuilder::new(usize::MAX, "max_capture_bytes", Duplicates::Reject);
        b.event(JsonEvent::ArrayStart).unwrap();
        assert_eq!(
            b.event(JsonEvent::End).unwrap_err().code,
            Code::ProtocolOrderError
        );
    }

    /// Read with the JSON grammar, which the `language` feature brings.
    #[cfg(feature = "language")]
    #[test]
    fn from_tabnas_unwraps() {
        let v = tabnas_json::parse(r#"{"a":[1,2],"b":"x","c":null}"#).unwrap();
        let d = Datum::from_tabnas(&v);
        assert_eq!(d.to_string(), r#"{"a":[1,2],"b":"x","c":null}"#);
    }
}
