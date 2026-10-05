//! `JsonEvents/1`: the source protocol every transducer consumes.
//!
//! A source (a tabnas parse, a parsed value, a line-delimited reader) emits
//! the events of one document in order: container boundaries, keys, whole
//! scalars, and one [`JsonEvent::End`] after the root value. Events borrow
//! from the source for the duration of one [`crate::shared::Sink::event`] call, so
//! moving them costs nothing; a stage that keeps anything copies it, and
//! says so in its retention contract.
//!
//! Scalars arrive whole because the tabnas lexer produces whole tokens. A
//! later `JsonEvents/2` may chunk strings; it will be a separately named
//! protocol, never a change to this one.

use std::fmt;

/// A number as the source spelled it and as a machine value.
///
/// `lexeme` is the source text when the source could hand it over (the
/// rule-event adapter reads it off the token; the JSON Lines source likewise),
/// else `None`, and the renderer falls back to the shortest round-trip form
/// of `value`. Keeping both is how `50.25` stays `50.25` and how a number
/// beyond f64's exact range keeps its digits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Number<'a> {
    pub value: f64,
    pub lexeme: Option<&'a str>,
}

impl<'a> Number<'a> {
    pub fn new(value: f64) -> Self {
        Number {
            value,
            lexeme: None,
        }
    }

    pub fn with_lexeme(value: f64, lexeme: &'a str) -> Self {
        Number {
            value,
            lexeme: Some(lexeme),
        }
    }
}

/// One event of `JsonEvents/1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JsonEvent<'a> {
    ObjectStart,
    ObjectEnd,
    ArrayStart,
    ArrayEnd,
    /// The name of the member whose value follows, inside an object.
    Key(&'a str),
    Null,
    Bool(bool),
    Number(Number<'a>),
    String(&'a str),
    /// The document is complete. Exactly one, after the root value, and
    /// only after the whole source has been validated.
    End,
}

impl JsonEvent<'_> {
    /// An owned copy, for recorders and tests.
    pub fn to_owned(&self) -> OwnedJsonEvent {
        match *self {
            JsonEvent::ObjectStart => OwnedJsonEvent::ObjectStart,
            JsonEvent::ObjectEnd => OwnedJsonEvent::ObjectEnd,
            JsonEvent::ArrayStart => OwnedJsonEvent::ArrayStart,
            JsonEvent::ArrayEnd => OwnedJsonEvent::ArrayEnd,
            JsonEvent::Key(k) => OwnedJsonEvent::Key(k.into()),
            JsonEvent::Null => OwnedJsonEvent::Null,
            JsonEvent::Bool(b) => OwnedJsonEvent::Bool(b),
            JsonEvent::Number(n) => OwnedJsonEvent::Number {
                value: n.value,
                lexeme: n.lexeme.map(Into::into),
            },
            JsonEvent::String(s) => OwnedJsonEvent::String(s.into()),
            JsonEvent::End => OwnedJsonEvent::End,
        }
    }

    /// Whether this event opens a container.
    pub fn is_start(&self) -> bool {
        matches!(self, JsonEvent::ObjectStart | JsonEvent::ArrayStart)
    }

    /// Whether this event closes a container.
    pub fn is_end(&self) -> bool {
        matches!(self, JsonEvent::ObjectEnd | JsonEvent::ArrayEnd)
    }

    /// Whether this event is a whole scalar value.
    pub fn is_scalar(&self) -> bool {
        matches!(
            self,
            JsonEvent::Null | JsonEvent::Bool(_) | JsonEvent::Number(_) | JsonEvent::String(_)
        )
    }
}

/// [`JsonEvent`] with its text owned: what a recorder keeps.
#[derive(Clone, Debug, PartialEq)]
pub enum OwnedJsonEvent {
    ObjectStart,
    ObjectEnd,
    ArrayStart,
    ArrayEnd,
    Key(Box<str>),
    Null,
    Bool(bool),
    Number {
        value: f64,
        lexeme: Option<Box<str>>,
    },
    String(Box<str>),
    End,
}

impl OwnedJsonEvent {
    /// The borrowed view, to replay a recording into a sink.
    pub fn as_event(&self) -> JsonEvent<'_> {
        match self {
            OwnedJsonEvent::ObjectStart => JsonEvent::ObjectStart,
            OwnedJsonEvent::ObjectEnd => JsonEvent::ObjectEnd,
            OwnedJsonEvent::ArrayStart => JsonEvent::ArrayStart,
            OwnedJsonEvent::ArrayEnd => JsonEvent::ArrayEnd,
            OwnedJsonEvent::Key(k) => JsonEvent::Key(k),
            OwnedJsonEvent::Null => JsonEvent::Null,
            OwnedJsonEvent::Bool(b) => JsonEvent::Bool(*b),
            OwnedJsonEvent::Number { value, lexeme } => JsonEvent::Number(Number {
                value: *value,
                lexeme: lexeme.as_deref(),
            }),
            OwnedJsonEvent::String(s) => JsonEvent::String(s),
            OwnedJsonEvent::End => JsonEvent::End,
        }
    }
}

impl fmt::Display for OwnedJsonEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OwnedJsonEvent::ObjectStart => f.write_str("{"),
            OwnedJsonEvent::ObjectEnd => f.write_str("}"),
            OwnedJsonEvent::ArrayStart => f.write_str("["),
            OwnedJsonEvent::ArrayEnd => f.write_str("]"),
            OwnedJsonEvent::Key(k) => write!(f, "key {k:?}"),
            OwnedJsonEvent::Null => f.write_str("null"),
            OwnedJsonEvent::Bool(b) => write!(f, "{b}"),
            OwnedJsonEvent::Number { value, lexeme } => match lexeme {
                Some(l) => write!(f, "{l}"),
                None => write!(f, "{value}"),
            },
            OwnedJsonEvent::String(s) => write!(f, "{s:?}"),
            OwnedJsonEvent::End => f.write_str("end"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_round_trips_through_borrowed() {
        let events = [
            JsonEvent::ObjectStart,
            JsonEvent::Key("a"),
            JsonEvent::Number(Number::with_lexeme(1.0, "1.0")),
            JsonEvent::Key("b"),
            JsonEvent::String("x"),
            JsonEvent::ObjectEnd,
            JsonEvent::End,
        ];
        for ev in events {
            let owned = ev.to_owned();
            assert_eq!(owned.as_event(), ev);
        }
    }

    #[test]
    fn classification() {
        assert!(JsonEvent::ArrayStart.is_start());
        assert!(JsonEvent::ObjectEnd.is_end());
        assert!(JsonEvent::Null.is_scalar());
        assert!(!JsonEvent::Key("k").is_scalar());
        assert!(!JsonEvent::End.is_scalar());
    }
}
