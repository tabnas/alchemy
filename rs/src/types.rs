//! The checker's types (spec sections 10.2 and 15; design brief 4.3).
//!
//! The types tell three things apart that the syntax does not: a
//! reusable value, a single-use resource, and a protocol. `Value` is any
//! data a document can hold; `Vector<T>` a finite retained collection;
//! `Stream<T>` an ordered single-use sequence, of which `TableEvents`
//! (`Stream<TableEvent>`, the `TableRows/1` protocol as tagged values) is
//! one; `JsonEvents` the single-use source; `Text` single-use incremental
//! text; `String` a finite retained string. `Unknown` is what inference
//! could not decide, and it is accepted everywhere: the checker is
//! conservative, reporting only what it can see is wrong. `Never` is the
//! type of `fail`.
//!
//! Compatibility is [`Type::accepts`]: whether a value of the actual type
//! may be given where the expected type is wanted. A mismatch between two
//! protocols (a `Text` for `JsonEvents`, a `Stream` for `TableEvents`) is
//! reported as `protocol_mismatch`, any other as `type_mismatch`.

use std::fmt;
use std::sync::Arc;

/// A type.
#[derive(Clone, Debug, PartialEq)]
pub enum Type {
    /// Not inferred; accepted everywhere.
    Unknown,
    /// The type of `fail`: accepted everywhere, since nothing comes back.
    Never,
    Null,
    Bool,
    Number,
    String,
    Keyword,
    /// Any data value (what a document holds, `missing` included).
    Value,
    Vector(Box<Type>),
    Record,
    Selector,
    CaptureSpec,
    /// A constructor's value, by the constructor's name (`transition`,
    /// `selected`, `schema`, `row`, `ready`, `no-schema`, `entry`).
    Tagged(Arc<str>),
    /// A `schema`, `row` or `table-end` value.
    TableEvent,
    /// A function: its parameters and its result.
    Fn(Vec<Type>, Box<Type>),
    /// A single-use stream of items.
    Stream(Box<Type>),
    /// The single-use source.
    JsonEvents,
    /// Single-use incremental text.
    Text,
}

impl Type {
    pub fn vector(item: Type) -> Type {
        Type::Vector(Box::new(item))
    }

    pub fn stream(item: Type) -> Type {
        Type::Stream(Box::new(item))
    }

    /// `Stream<TableEvent>`: the `TableRows/1` protocol.
    pub fn table_events() -> Type {
        Type::stream(Type::TableEvent)
    }

    pub fn tagged(name: &str) -> Type {
        Type::Tagged(Arc::from(name))
    }

    pub fn func(params: Vec<Type>, result: Type) -> Type {
        Type::Fn(params, Box::new(result))
    }

    /// A function of `n` parameters about which nothing more is known.
    pub fn func_of(n: usize) -> Type {
        Type::func(vec![Type::Unknown; n], Type::Unknown)
    }

    /// Whether a binding of this type is used at most once.
    pub fn is_affine(&self) -> bool {
        matches!(self, Type::Stream(_) | Type::JsonEvents | Type::Text)
    }

    /// Whether this is one of the protocols, so a mismatch is between
    /// protocols rather than between kinds of value.
    pub fn is_protocol(&self) -> bool {
        self.is_affine()
    }

    /// Whether this type is data: what a document can hold.
    pub fn is_data(&self) -> bool {
        match self {
            Type::Null
            | Type::Bool
            | Type::Number
            | Type::String
            | Type::Record
            | Type::Value
            | Type::Unknown
            | Type::Never => true,
            Type::Vector(item) => item.is_data(),
            Type::Tagged(tag) => &**tag == "missing",
            _ => false,
        }
    }

    /// Whether this is a stream or the source: what a vector can never
    /// hold. A text is affine too, but it may be finite (the library's
    /// `csv-row` joins the vector of texts a `map` answers), so a vector
    /// may hold one; the runtime refuses a live one where the vector is
    /// built, as it refuses one a `map` over a vector answers.
    pub fn is_stream_or_source(&self) -> bool {
        matches!(self, Type::Stream(_) | Type::JsonEvents)
    }

    /// Whether this type is a stream: `Stream<T>` in any shape.
    pub fn is_stream(&self) -> bool {
        matches!(self, Type::Stream(_))
    }

    /// Whether a text combinator takes a value of this type as an item: a
    /// string or a text, a data value that may be a string (the runtime
    /// refuses one that is not), or what it cannot see.
    pub fn is_textlike(&self) -> bool {
        matches!(
            self,
            Type::String | Type::Text | Type::Value | Type::Unknown | Type::Never
        )
    }

    /// Whether a value of type `actual` may be given where `self` is
    /// expected.
    pub fn accepts(&self, actual: &Type) -> bool {
        match (self, actual) {
            (Type::Unknown, _) | (_, Type::Unknown) | (_, Type::Never) => true,
            (Type::Value, actual) => actual.is_data(),
            (Type::Vector(a), Type::Vector(b)) => a.accepts(b),
            (Type::Stream(a), Type::Stream(b)) => a.accepts(b),
            (Type::TableEvent, Type::Tagged(tag)) => {
                matches!(&**tag, "schema" | "row" | "table-end")
            }
            (Type::Fn(ps, r), Type::Fn(qs, s)) => ps.len() == qs.len() && r.accepts(s),
            (a, b) => a == b,
        }
    }

    /// The type of a value that is one of two: the same type when they
    /// agree, a text when one is a text and the other a string (a string
    /// lifts to a text), else unknown.
    pub fn join(a: &Type, b: &Type) -> Type {
        match (a, b) {
            (Type::Never, other) | (other, Type::Never) => other.clone(),
            (a, b) if a == b => a.clone(),
            (Type::Text, Type::String) | (Type::String, Type::Text) => Type::Text,
            (Type::Vector(a), Type::Vector(b)) => Type::vector(Type::join(a, b)),
            (a, b) if a.is_data() && b.is_data() => Type::Value,
            _ => Type::Unknown,
        }
    }

    /// The type of the items of a `Vector` or `Stream`.
    pub fn item(&self) -> Option<&Type> {
        match self {
            Type::Vector(t) | Type::Stream(t) => Some(t),
            _ => None,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Unknown => f.write_str("Unknown"),
            Type::Never => f.write_str("Never"),
            Type::Null => f.write_str("Null"),
            Type::Bool => f.write_str("Bool"),
            Type::Number => f.write_str("Number"),
            Type::String => f.write_str("String"),
            Type::Keyword => f.write_str("Keyword"),
            Type::Value => f.write_str("Value"),
            Type::Vector(t) => write!(f, "Vector<{t}>"),
            Type::Record => f.write_str("Record"),
            Type::Selector => f.write_str("Selector"),
            Type::CaptureSpec => f.write_str("CaptureSpec"),
            Type::Tagged(tag) => f.write_str(tag),
            Type::TableEvent => f.write_str("TableEvent"),
            Type::Fn(params, result) => {
                f.write_str("Fn(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, " -> {result})")
            }
            Type::Stream(t) if **t == Type::TableEvent => f.write_str("TableEvents"),
            Type::Stream(t) => write!(f, "Stream<{t}>"),
            Type::JsonEvents => f.write_str("JsonEvents"),
            Type::Text => f.write_str("Text"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_and_never_are_accepted_everywhere() {
        for t in [
            Type::Text,
            Type::JsonEvents,
            Type::Record,
            Type::table_events(),
        ] {
            assert!(t.accepts(&Type::Unknown));
            assert!(t.accepts(&Type::Never));
            assert!(Type::Unknown.accepts(&t));
        }
    }

    #[test]
    fn value_accepts_data_and_nothing_else() {
        assert!(Type::Value.accepts(&Type::Number));
        assert!(Type::Value.accepts(&Type::vector(Type::String)));
        assert!(Type::Value.accepts(&Type::tagged("missing")));
        assert!(!Type::Value.accepts(&Type::Selector));
        assert!(!Type::Value.accepts(&Type::Text));
        assert!(!Type::Value.accepts(&Type::func_of(1)));
    }

    #[test]
    fn streams_and_table_events() {
        let table = Type::table_events();
        assert!(table.accepts(&Type::stream(Type::Unknown)));
        assert!(table.accepts(&Type::stream(Type::tagged("row"))));
        assert!(!table.accepts(&Type::stream(Type::tagged("selected"))));
        assert!(!table.accepts(&Type::JsonEvents));
        assert!(!Type::JsonEvents.accepts(&table));
        assert!(table.is_affine() && Type::Text.is_affine() && !Type::String.is_affine());
        assert_eq!(table.to_string(), "TableEvents");
        assert_eq!(Type::stream(Type::Value).to_string(), "Stream<Value>");
        assert_eq!(
            Type::func(vec![Type::Record, Type::JsonEvents], table).to_string(),
            "Fn(Record JsonEvents -> TableEvents)"
        );
    }

    #[test]
    fn functions_by_arity_and_result() {
        let f = Type::func(vec![Type::Unknown, Type::Unknown], Type::Text);
        assert!(f.accepts(&Type::func(vec![Type::Record, Type::Value], Type::Text)));
        assert!(!f.accepts(&Type::func_of(1)));
        assert!(Type::func_of(2).accepts(&f));
    }

    #[test]
    fn joins() {
        assert_eq!(Type::join(&Type::Never, &Type::Text), Type::Text);
        assert_eq!(Type::join(&Type::Text, &Type::String), Type::Text);
        assert_eq!(Type::join(&Type::Number, &Type::String), Type::Value);
        assert_eq!(Type::join(&Type::Text, &Type::Record), Type::Unknown);
        assert_eq!(
            Type::join(&Type::vector(Type::Number), &Type::vector(Type::Null)),
            Type::vector(Type::Value)
        );
    }
}
