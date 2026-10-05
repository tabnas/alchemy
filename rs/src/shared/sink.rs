//! The push boundary between stages.
//!
//! A pipeline is a chain of sinks. The source calls the first sink once per
//! event, synchronously, on the thread that parses; each stage does its work
//! and calls the next. Nothing is queued between stages, so a slow writer at
//! the end slows the parser at the start: that is the backpressure, and it
//! costs no buffer. A stage that must stop early (a `take`) answers
//! [`Flow::Stop`], which the source turns into a cancelled parse.

use indexmap::IndexSet;

use crate::shared::error::{Code, Fail};
use crate::shared::event::{JsonEvent, OwnedJsonEvent};
use crate::shared::selector::{Path, Segment};

/// What a stage wants next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Keep sending.
    Continue,
    /// The stage has all it needs; the source should stop. Not an error:
    /// the source stops the parse, releases what it holds, and reports
    /// nothing further. Whether the rest of the input is validated first
    /// is the source's documented policy.
    Stop,
}

/// A consumer of `JsonEvents/1`.
pub trait Sink {
    /// One event. An `Err` aborts the run; the source stops the parse and
    /// the error reaches the caller unchanged.
    fn event(&mut self, ev: JsonEvent<'_>) -> Result<Flow, Fail>;
}

impl Sink for Vec<OwnedJsonEvent> {
    fn event(&mut self, ev: JsonEvent<'_>) -> Result<Flow, Fail> {
        self.push(ev.to_owned());
        Ok(Flow::Continue)
    }
}

impl<S: Sink + ?Sized> Sink for &mut S {
    fn event(&mut self, ev: JsonEvent<'_>) -> Result<Flow, Fail> {
        (**self).event(ev)
    }
}

impl<S: Sink + ?Sized> Sink for Box<S> {
    fn event(&mut self, ev: JsonEvent<'_>) -> Result<Flow, Fail> {
        (**self).event(ev)
    }
}

/// A sink made of a closure.
pub struct FnSink<F>(pub F);

impl<F> Sink for FnSink<F>
where
    F: FnMut(JsonEvent<'_>) -> Result<Flow, Fail>,
{
    fn event(&mut self, ev: JsonEvent<'_>) -> Result<Flow, Fail> {
        (self.0)(ev)
    }
}

/// A sink that counts events and drops them: the cheapest consumer, for
/// measuring a source on its own.
#[derive(Debug, Default)]
pub struct CountSink {
    pub events: u64,
}

impl Sink for CountSink {
    fn event(&mut self, _ev: JsonEvent<'_>) -> Result<Flow, Fail> {
        self.events += 1;
        Ok(Flow::Continue)
    }
}

/// A tree's events, held to their contract in front of a sink that takes
/// them as one (a render that writes a document from them): one root
/// value, and in each object a key and then its value, each key once. A
/// value walked from a parsed tree keeps it by construction. A parse
/// streamed as it proceeds may not: it hands on a member its grammar reads
/// twice (JSON's `{"a":1,"a":2}`, whose value keeps the last) where a tree
/// has one, and the rule-event adapter refuses most shapes it cannot
/// follow but not every one a grammar can produce. A repeated key in one
/// object is refused with `DUPLICATE_MEMBER`, and events no tree has (a
/// value where a key is due, a key outside an object, a close with nothing
/// open, a second root) with `STREAMABILITY_UNKNOWN`, each at the path of
/// the object concerned; the event is not passed on, and what to do then
/// is the host's (aless falls back once to the parsed value when nothing
/// has been written). Each open object keeps the keys it has had, dropped
/// when it closes, so the cost is a lookup per key and the keys of the
/// objects open at once, which a parse holds in its tree already.
pub struct TreeContract<S> {
    open: Vec<Open>,
    inner: S,
    /// A whole root value has passed.
    root_done: bool,
}

/// A container [`TreeContract`] has open.
enum Open {
    /// The keys the object has had, in order, the last of them the member
    /// whose value is due unless `key_due`; and whether a key is due next
    /// rather than a value.
    Object {
        keys: IndexSet<Box<str>>,
        key_due: bool,
    },
    /// The index of the element due next.
    Array { next: usize },
}

impl<S> TreeContract<S> {
    pub fn new(inner: S) -> TreeContract<S> {
        TreeContract {
            open: Vec::new(),
            inner,
            root_done: false,
        }
    }

    pub fn inner(&self) -> &S {
        &self.inner
    }

    pub fn into_inner(self) -> S {
        self.inner
    }

    /// The path of the value due next: the open containers, each by the
    /// member or element open in it, and in the innermost object, its
    /// last key when that member's value is due.
    fn path(&self) -> Path {
        let mut segments = Vec::with_capacity(self.open.len());
        let last = self.open.len().wrapping_sub(1);
        for (i, open) in self.open.iter().enumerate() {
            match open {
                Open::Object { keys, key_due } => {
                    // A container open inside this object is its last
                    // key's value, whatever `key_due` says: the member was
                    // counted as taken when its value opened.
                    if i != last || !*key_due {
                        if let Some(key) = keys.last() {
                            segments.push(Segment::Key(key.clone()));
                        }
                    }
                }
                Open::Array { next } => segments.push(Segment::Index(*next)),
            }
        }
        Path(segments)
    }

    /// The failure for events no tree has.
    fn not_a_tree(&self, what: &str) -> Fail {
        Fail::new(
            Code::StreamabilityUnknown,
            format!(
                "the stream holds {what}, which a tree's events never do, so it is not a tree's"
            ),
        )
        .at_path(self.path().to_string())
    }
}

impl<S: Sink> Sink for TreeContract<S> {
    fn event(&mut self, ev: JsonEvent<'_>) -> Result<Flow, Fail> {
        match &ev {
            JsonEvent::Key(key) => match self.open.last_mut() {
                Some(Open::Object { keys, key_due }) if *key_due => {
                    if !keys.insert((*key).into()) {
                        let mut path = self.path();
                        path.0.push(Segment::Key((*key).into()));
                        return Err(Fail::new(
                            Code::DuplicateMember,
                            format!(
                                "member {key:?} appears twice in one object, and a tree's events \
                                 hold each key once"
                            ),
                        )
                        .at_path(path.to_string()));
                    }
                    *key_due = false;
                }
                Some(Open::Object { .. }) => {
                    return Err(self.not_a_tree("a key where a value is due"));
                }
                _ => return Err(self.not_a_tree("a key outside an object")),
            },
            JsonEvent::ObjectEnd => match self.open.last() {
                Some(Open::Object { key_due: true, .. }) => {
                    self.open.pop();
                    self.closed();
                }
                Some(Open::Object { .. }) => {
                    return Err(self.not_a_tree("an object's end where a value is due"));
                }
                _ => return Err(self.not_a_tree("an object's end where none is due")),
            },
            JsonEvent::ArrayEnd => match self.open.last() {
                Some(Open::Array { .. }) => {
                    self.open.pop();
                    self.closed();
                }
                _ => return Err(self.not_a_tree("an array's end where none is due")),
            },
            JsonEvent::End => {
                if !self.open.is_empty() {
                    return Err(self.not_a_tree("its end inside an open container"));
                }
            }
            // A value: in an object, only once its key is in; at the root,
            // only once.
            value => {
                match self.open.last_mut() {
                    Some(Open::Object { key_due, .. }) => {
                        if *key_due {
                            return Err(self.not_a_tree("a value where a key is due"));
                        }
                        *key_due = true;
                    }
                    Some(Open::Array { .. }) => {}
                    None => {
                        if self.root_done {
                            return Err(self.not_a_tree("a second root value"));
                        }
                    }
                }
                match value {
                    JsonEvent::ObjectStart => self.open.push(Open::Object {
                        keys: IndexSet::new(),
                        key_due: true,
                    }),
                    JsonEvent::ArrayStart => self.open.push(Open::Array { next: 0 }),
                    _ => self.closed(),
                }
            }
        }
        self.inner.event(ev)
    }
}

impl<S> TreeContract<S> {
    /// A value is complete: the next in its array is due, or the root is.
    fn closed(&mut self) {
        match self.open.last_mut() {
            Some(Open::Array { next }) => *next += 1,
            Some(Open::Object { .. }) => {}
            None => self.root_done = true,
        }
    }
}

/// Replay a recording into a sink, stopping where the sink stops.
pub fn replay(events: &[OwnedJsonEvent], sink: &mut dyn Sink) -> Result<Flow, Fail> {
    for ev in events {
        if sink.event(ev.as_event())? == Flow::Stop {
            return Ok(Flow::Stop);
        }
    }
    Ok(Flow::Continue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vector_records_and_replays() {
        let mut rec: Vec<OwnedJsonEvent> = Vec::new();
        rec.event(JsonEvent::ArrayStart).unwrap();
        rec.event(JsonEvent::Bool(true)).unwrap();
        rec.event(JsonEvent::ArrayEnd).unwrap();
        rec.event(JsonEvent::End).unwrap();
        let mut count = CountSink::default();
        assert_eq!(replay(&rec, &mut count).unwrap(), Flow::Continue);
        assert_eq!(count.events, 4);
    }

    #[test]
    fn stop_ends_a_replay() {
        let rec = vec![
            OwnedJsonEvent::ArrayStart,
            OwnedJsonEvent::Null,
            OwnedJsonEvent::ArrayEnd,
            OwnedJsonEvent::End,
        ];
        let mut seen = 0;
        let mut stopper = FnSink(|_ev: JsonEvent<'_>| {
            seen += 1;
            Ok(if seen == 2 {
                Flow::Stop
            } else {
                Flow::Continue
            })
        });
        assert_eq!(replay(&rec, &mut stopper).unwrap(), Flow::Stop);
        assert_eq!(seen, 2);
    }

    fn events(json: &str) -> Vec<OwnedJsonEvent> {
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut rec = Vec::new();
        fn walk(v: &serde_json::Value, rec: &mut Vec<OwnedJsonEvent>) {
            use OwnedJsonEvent as E;
            match v {
                serde_json::Value::Null => rec.push(E::Null),
                serde_json::Value::Bool(b) => rec.push(E::Bool(*b)),
                serde_json::Value::Number(n) => rec.push(E::Number {
                    value: n.as_f64().unwrap(),
                    lexeme: None,
                }),
                serde_json::Value::String(s) => rec.push(E::String(s.as_str().into())),
                serde_json::Value::Array(items) => {
                    rec.push(E::ArrayStart);
                    items.iter().for_each(|i| walk(i, rec));
                    rec.push(E::ArrayEnd);
                }
                serde_json::Value::Object(members) => {
                    rec.push(E::ObjectStart);
                    for (k, v) in members {
                        rec.push(E::Key(k.as_str().into()));
                        walk(v, rec);
                    }
                    rec.push(E::ObjectEnd);
                }
            }
        }
        walk(&value, &mut rec);
        rec.push(OwnedJsonEvent::End);
        rec
    }

    /// A tree's events pass the contract unchanged, the same key in two
    /// objects included.
    #[test]
    fn a_trees_events_pass_the_tree_contract_unchanged() {
        let doc = r#"{"a":{"b":1,"a":2},"b":[{"a":3},{"a":4},[],{}],"c":[],"d":{}}"#;
        let mut guard = TreeContract::new(Vec::new());
        assert_eq!(replay(&events(doc), &mut guard).unwrap(), Flow::Continue);
        assert_eq!(guard.inner(), &events(doc));
        for root in ["1", "\"x\"", "null", "[]", "[1,[2,[3]]]"] {
            let mut guard = TreeContract::new(Vec::new());
            replay(&events(root), &mut guard).unwrap();
            assert_eq!(guard.into_inner(), events(root), "{root}");
        }
    }

    /// A key repeated in one object is `DUPLICATE_MEMBER` at the member's
    /// path, and the event that repeats it is not passed on.
    #[test]
    fn a_repeated_key_in_one_object_is_a_duplicate_member_at_its_path() {
        use OwnedJsonEvent as E;
        let key = |k: &str| E::Key(k.into());
        let stream = vec![
            E::ObjectStart,
            key("a"),
            E::Null,
            key("b"),
            E::ArrayStart,
            E::Null,
            E::ObjectStart,
            key("x"),
            E::Null,
            key("x"),
        ];
        let mut guard = TreeContract::new(Vec::new());
        let fail = replay(&stream, &mut guard).unwrap_err();
        assert_eq!(fail.code, Code::DuplicateMember, "{fail}");
        assert!(fail.message.starts_with("member \"x\""), "{fail}");
        assert_eq!(fail.path.as_deref(), Some(".b[1].x"));
        assert_eq!(guard.inner().len(), stream.len() - 1);
        let mut guard = TreeContract::new(Vec::new());
        let fail = replay(
            &[E::ObjectStart, key("a b"), E::Null, key("a b")],
            &mut guard,
        )
        .unwrap_err();
        assert_eq!(fail.path.as_deref(), Some(r#"."a b""#));
    }

    /// Events no tree has are `STREAMABILITY_UNKNOWN`, naming what was
    /// met and the path of the value due.
    #[test]
    fn events_no_tree_has_are_refused_as_not_a_tree() {
        use OwnedJsonEvent as E;
        let key = |k: &str| E::Key(k.into());
        for (bad, why, path) in [
            (
                vec![E::ObjectStart, E::Null],
                "a value where a key is due",
                ".",
            ),
            (
                vec![E::ObjectStart, key("a"), E::ObjectStart, E::ArrayStart],
                "a value where a key is due",
                ".a",
            ),
            (
                vec![E::ObjectStart, key("a"), key("b")],
                "a key where a value is due",
                ".a",
            ),
            (
                vec![E::ArrayStart, key("a")],
                "a key outside an object",
                "[0]",
            ),
            (vec![key("a")], "a key outside an object", "."),
            (
                vec![E::ObjectStart, key("a"), E::ObjectEnd],
                "an object's end where a value is due",
                ".a",
            ),
            (
                vec![E::ArrayStart, E::Null, E::ObjectEnd],
                "an object's end where none is due",
                "[1]",
            ),
            (vec![E::ObjectEnd], "an object's end where none is due", "."),
            (
                vec![E::ObjectStart, E::ArrayEnd],
                "an array's end where none is due",
                ".",
            ),
            (vec![E::Null, E::Null], "a second root value", "."),
            (
                vec![E::ArrayStart, E::ArrayEnd, E::ObjectStart],
                "a second root value",
                ".",
            ),
            (
                vec![E::ArrayStart, E::End],
                "its end inside an open container",
                "[0]",
            ),
        ] {
            let mut guard = TreeContract::new(Vec::new());
            let fail = replay(&bad, &mut guard).unwrap_err();
            assert_eq!(fail.code, Code::StreamabilityUnknown, "{bad:?}");
            assert!(fail.message.contains(why), "{bad:?}: {fail}");
            assert_eq!(fail.path.as_deref(), Some(path), "{bad:?}: {fail}");
            assert_eq!(guard.inner().len(), bad.len() - 1, "{bad:?}");
        }
    }
}
