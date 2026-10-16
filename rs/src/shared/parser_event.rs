//! `ParserEvents/1`: the parser's rule passes, as a transducer part reads
//! them.
//!
//! A source over a tabnas parse (transduce's `ParserEventSource`) relays
//! each rule pass the engine reports to its `rule_done` subscribers as
//! one [`ParserEvent`]: an [`Open`](ParserEvent::Open) when a rule's open
//! pass has run, a [`Close`](ParserEvent::Close) when its close pass has,
//! and one [`End`](ParserEvent::End) after the last pass, only once the
//! engine has returned the parse's value, so a document is complete only
//! when it validated. A pass made inside a probe (the engine's
//! `RuleDone::probe`) is speculative, and the source drops it.
//!
//! Containers never travel in parser events: a rule's node is handed on
//! only as a scalar ([`RulePass::value`]), so a transducer that wants a
//! container emits its start and end from the passes that open and close
//! it, and nothing in the stream grows with the document.
//!
//! The fixture encoding ([`ParserEvent::to_json`]) follows the JSON
//! events' encoding: an array whose first element names the event,
//! `["open", {"name": "map", "d": 1, "i": 3, "alt": {"p": "pair", "r": "",
//! "b": 0, "g": ["map", "json"]}, "tokens": [["#OB", "{", "{", 1, 1]],
//! "forced": false, "n": {"pk": 0}}]`, and `["end"]`. A field that is
//! absent (`alt` when the state declared no alternates, `value` when the
//! node is not a scalar or the pass is an open) is left out, so `"value":
//! null` is always a null scalar.

use crate::shared::datum::Datum;
use crate::shared::error::Fail;
use crate::shared::sink::Flow;

/// One item of `ParserEvents/1`.
#[derive(Clone, Debug, PartialEq)]
pub enum ParserEvent {
    /// A rule's open pass completed.
    Open(RulePass),
    /// A rule's close pass completed.
    Close(RulePass),
    /// The parse returned its value. Exactly one, after the last pass.
    End,
}

/// What one rule pass reports.
#[derive(Clone, Debug, PartialEq)]
pub struct RulePass {
    /// The rule's name (`map`, `pair`, `val`).
    pub name: Box<str>,
    /// The rule's depth on the stack: one more than the rule that pushed
    /// it, the same as the rule it replaced.
    pub d: u64,
    /// The rule's instance number in the parse.
    pub i: u64,
    /// The alternate that matched; `None` when the state declared none.
    pub alt: Option<PassAlt>,
    /// The tokens the pass consumed.
    pub tokens: Vec<PassToken>,
    /// A close synthesized by recovery.
    pub forced: bool,
    /// The rule's named counters, by name.
    pub n: Vec<(Box<str>, i64)>,
    /// The node's value, when it is a scalar at a close pass.
    pub value: Option<Datum>,
}

/// The alternate a pass matched.
#[derive(Clone, Debug, PartialEq)]
pub struct PassAlt {
    /// The rule it pushed, or empty.
    pub p: Box<str>,
    /// The rule it replaced the current one with, or empty.
    pub r: Box<str>,
    /// How many tokens it gave back.
    pub b: u64,
    /// Its groups.
    pub g: Vec<Box<str>>,
}

/// A token a pass consumed.
#[derive(Clone, Debug, PartialEq)]
pub struct PassToken {
    /// The token's name (`#OB`, `#ST`, `#NR`).
    pub name: Box<str>,
    /// The token's value: a string's contents, a number's value.
    pub val: Datum,
    /// The source text the token was lexed from.
    pub src: Box<str>,
    pub row: u64,
    pub col: u64,
}

/// A consumer of `ParserEvents/1`: what a transducer part's head is, and
/// what a source pushes each rule pass into, synchronously, on the thread
/// that parses. As with [`crate::shared::Sink`], an `Err` aborts the run
/// and [`Flow::Stop`] asks the source to stop the parse.
pub trait ParserSink {
    fn event(&mut self, ev: &ParserEvent) -> Result<Flow, Fail>;
}

impl ParserSink for Vec<ParserEvent> {
    fn event(&mut self, ev: &ParserEvent) -> Result<Flow, Fail> {
        self.push(ev.clone());
        Ok(Flow::Continue)
    }
}

impl<S: ParserSink + ?Sized> ParserSink for &mut S {
    fn event(&mut self, ev: &ParserEvent) -> Result<Flow, Fail> {
        (**self).event(ev)
    }
}

impl<S: ParserSink + ?Sized> ParserSink for Box<S> {
    fn event(&mut self, ev: &ParserEvent) -> Result<Flow, Fail> {
        (**self).event(ev)
    }
}

impl ParserEvent {
    /// The event's name in the encoding: `open`, `close` or `end`.
    pub fn name(&self) -> &'static str {
        match self {
            ParserEvent::Open(_) => "open",
            ParserEvent::Close(_) => "close",
            ParserEvent::End => "end",
        }
    }

    /// The pass an `Open` or a `Close` reports.
    pub fn pass(&self) -> Option<&RulePass> {
        match self {
            ParserEvent::Open(pass) | ParserEvent::Close(pass) => Some(pass),
            ParserEvent::End => None,
        }
    }

    /// The fixture encoding: `["open", {...}]`, `["close", {...}]` or
    /// `["end"]`.
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::{json, Map, Value};
        let Some(pass) = self.pass() else {
            return json!([self.name()]);
        };
        let mut rule = Map::new();
        rule.insert("name".into(), Value::from(&*pass.name));
        rule.insert("d".into(), Value::from(pass.d));
        rule.insert("i".into(), Value::from(pass.i));
        if let Some(alt) = &pass.alt {
            rule.insert(
                "alt".into(),
                json!({
                    "p": &*alt.p,
                    "r": &*alt.r,
                    "b": alt.b,
                    "g": alt.g.iter().map(|g| &**g).collect::<Vec<_>>(),
                }),
            );
        }
        rule.insert(
            "tokens".into(),
            Value::Array(
                pass.tokens
                    .iter()
                    .map(|t| json!([&*t.name, t.val.to_json(), &*t.src, t.row, t.col]))
                    .collect(),
            ),
        );
        rule.insert("forced".into(), Value::Bool(pass.forced));
        rule.insert(
            "n".into(),
            Value::Object(
                pass.n
                    .iter()
                    .map(|(k, v)| (k.to_string(), Value::from(*v)))
                    .collect(),
            ),
        );
        if let Some(value) = &pass.value {
            rule.insert("value".into(), value.to_json());
        }
        json!([self.name(), Value::Object(rule)])
    }

    /// An event from its fixture encoding, or what is wrong with it.
    pub fn from_json(value: &serde_json::Value) -> Result<ParserEvent, String> {
        use serde_json::Value;
        let items = value
            .as_array()
            .ok_or_else(|| format!("a parser event is an array, not {value}"))?;
        let name = items
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| format!("a parser event starts with its name: {value}"))?;
        match (name, items.len()) {
            ("end", 1) => return Ok(ParserEvent::End),
            ("open" | "close", 2) => {}
            _ => return Err(format!("not a parser event: {value}")),
        }
        let rule = items[1]
            .as_object()
            .ok_or_else(|| format!("a rule pass is an object: {value}"))?;
        let text = |key: &str| -> Result<Box<str>, String> {
            rule.get(key)
                .and_then(Value::as_str)
                .map(Box::from)
                .ok_or_else(|| format!("a rule pass's :{key} is a string: {value}"))
        };
        let count = |v: Option<&Value>, key: &str| -> Result<u64, String> {
            v.and_then(Value::as_u64)
                .ok_or_else(|| format!("a rule pass's :{key} is a count: {value}"))
        };
        let alt = match rule.get("alt") {
            None => None,
            Some(alt) => {
                let field = |key: &str| -> Result<Box<str>, String> {
                    alt.get(key)
                        .and_then(Value::as_str)
                        .map(Box::from)
                        .ok_or_else(|| format!("an alternate's :{key} is a string: {value}"))
                };
                let g = alt
                    .get("g")
                    .and_then(Value::as_array)
                    .ok_or_else(|| format!("an alternate's :g is a vector: {value}"))?
                    .iter()
                    .map(|g| g.as_str().map(Box::from))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| format!("an alternate's groups are strings: {value}"))?;
                Some(PassAlt {
                    p: field("p")?,
                    r: field("r")?,
                    b: count(alt.get("b"), "b")?,
                    g,
                })
            }
        };
        let tokens = rule
            .get("tokens")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("a rule pass's :tokens is a vector: {value}"))?
            .iter()
            .map(|t| match t.as_array().map(Vec::as_slice) {
                Some([name, val, src, row, col]) => Some(PassToken {
                    name: name.as_str()?.into(),
                    val: Datum::from_json(val),
                    src: src.as_str()?.into(),
                    row: row.as_u64()?,
                    col: col.as_u64()?,
                }),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| format!("a token is [name, val, src, row, col]: {value}"))?;
        let n = rule
            .get("n")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("a rule pass's :n is a record: {value}"))?
            .iter()
            .map(|(k, v)| v.as_i64().map(|v| (Box::from(k.as_str()), v)))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| format!("a rule pass's counters are integers: {value}"))?;
        let pass = RulePass {
            name: text("name")?,
            d: count(rule.get("d"), "d")?,
            i: count(rule.get("i"), "i")?,
            alt,
            tokens,
            forced: rule
                .get("forced")
                .and_then(Value::as_bool)
                .ok_or_else(|| format!("a rule pass's :forced is a boolean: {value}"))?,
            n,
            value: rule.get("value").map(Datum::from_json),
        };
        Ok(match name {
            "open" => ParserEvent::Open(pass),
            _ => ParserEvent::Close(pass),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_encoding_round_trips_and_leaves_absent_fields_out() {
        let open = r##"["open",{"name":"map","d":1,"i":3,"alt":{"p":"pair","r":"","b":0,"g":["map","json"]},"tokens":[["#OB","{","{",1,1]],"forced":false,"n":{"pk":0}}]"##;
        let close =
            r#"["close",{"name":"val","d":2,"i":4,"tokens":[],"forced":true,"n":{},"value":null}]"#;
        for text in [open, close, r#"["end"]"#] {
            let json: serde_json::Value = serde_json::from_str(text).unwrap();
            let event = ParserEvent::from_json(&json).unwrap();
            assert_eq!(event.to_json().to_string(), text);
        }
        let ParserEvent::Open(pass) =
            ParserEvent::from_json(&serde_json::from_str(open).unwrap()).unwrap()
        else {
            panic!("an open");
        };
        assert_eq!(pass.value, None, "an open pass has no value");
        assert_eq!(pass.alt.unwrap().p.as_ref(), "pair");
    }

    #[test]
    fn a_malformed_event_says_what_is_wrong() {
        for text in [
            r#"{"open":1}"#,
            r#"["start"]"#,
            r#"["open"]"#,
            r#"["close",{"name":"x","d":-1,"i":0,"tokens":[],"forced":false,"n":{}}]"#,
            r##"["close",{"name":"x","d":1,"i":0,"tokens":[["#ST"]],"forced":false,"n":{}}]"##,
        ] {
            let json: serde_json::Value = serde_json::from_str(text).unwrap();
            assert!(ParserEvent::from_json(&json).is_err(), "{text}");
        }
    }
}
