//! Selectors: reusable descriptions of where in a document to look.
//!
//! A selector is data, never code: it is built from constructors or from
//! validated path segments (`as-path`), and a matcher interprets it. A
//! concrete [`Path`] names one location; a [`Selector`] may name many
//! (`EachIndex`, `EachMember`). Both print in jq syntax, which is what the
//! rest of the fleet (aless included) prints and accepts.

use std::fmt;

/// One step of a concrete path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Segment {
    Key(Box<str>),
    Index(usize),
}

impl Segment {
    pub fn key(k: impl Into<Box<str>>) -> Segment {
        Segment::Key(k.into())
    }
}

/// A concrete location in a document.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Path(pub Vec<Segment>);

impl Path {
    pub fn root() -> Path {
        Path(Vec::new())
    }

    pub fn push(&mut self, seg: Segment) {
        self.0.push(seg);
    }

    pub fn pop(&mut self) -> Option<Segment> {
        self.0.pop()
    }

    pub fn depth(&self) -> usize {
        self.0.len()
    }

    pub fn segments(&self) -> &[Segment] {
        &self.0
    }
}

/// Write one key as jq does: bare when it is an identifier, quoted otherwise.
pub fn write_key(f: &mut fmt::Formatter<'_>, key: &str) -> fmt::Result {
    let bare = !key.is_empty()
        && key
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if bare {
        write!(f, ".{key}")
    } else {
        write!(f, ".{}", serde_json::Value::String(key.to_string()))
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str(".");
        }
        for seg in &self.0 {
            match seg {
                Segment::Key(k) => write_key(f, k)?,
                Segment::Index(i) => write!(f, "[{i}]")?,
            }
        }
        Ok(())
    }
}

/// One step of a selector.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    /// The member with this name, inside an object.
    Property(Box<str>),
    /// The element at this position, inside an array.
    Index(usize),
    /// Every element of an array.
    EachIndex,
    /// Every member value of an object.
    EachMember,
}

/// A description of locations: the root, narrowed step by step.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Selector(pub Vec<Step>);

impl Selector {
    /// The document itself.
    pub fn root() -> Selector {
        Selector(Vec::new())
    }

    pub fn property(mut self, name: impl Into<Box<str>>) -> Selector {
        self.0.push(Step::Property(name.into()));
        self
    }

    pub fn index(mut self, i: usize) -> Selector {
        self.0.push(Step::Index(i));
        self
    }

    pub fn each_index(mut self) -> Selector {
        self.0.push(Step::EachIndex);
        self
    }

    pub fn each_member(mut self) -> Selector {
        self.0.push(Step::EachMember);
        self
    }

    /// `self`, then `other` below every location `self` names.
    pub fn compose(mut self, other: &Selector) -> Selector {
        self.0.extend(other.0.iter().cloned());
        self
    }

    /// A selector naming exactly one location: `as-path` over data.
    pub fn from_segments(segments: &[Segment]) -> Selector {
        Selector(
            segments
                .iter()
                .map(|s| match s {
                    Segment::Key(k) => Step::Property(k.clone()),
                    Segment::Index(i) => Step::Index(*i),
                })
                .collect(),
        )
    }

    pub fn steps(&self) -> &[Step] {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether the selector can name more than one location.
    pub fn is_multi(&self) -> bool {
        self.0
            .iter()
            .any(|s| matches!(s, Step::EachIndex | Step::EachMember))
    }

    /// Whether this selector names a location strictly inside a location
    /// `other` names, or the same one: the test a router uses to refuse
    /// overlapping captures.
    pub fn may_overlap(&self, other: &Selector) -> bool {
        let (short, long) = if self.0.len() <= other.0.len() {
            (self, other)
        } else {
            (other, self)
        };
        short
            .0
            .iter()
            .zip(long.0.iter())
            .all(|(a, b)| step_may_match_same(a, b))
    }
}

fn step_may_match_same(a: &Step, b: &Step) -> bool {
    match (a, b) {
        (Step::Property(x), Step::Property(y)) => x == y,
        (Step::Property(_), Step::EachMember) | (Step::EachMember, Step::Property(_)) => true,
        (Step::EachMember, Step::EachMember) => true,
        (Step::Index(x), Step::Index(y)) => x == y,
        (Step::Index(_), Step::EachIndex) | (Step::EachIndex, Step::Index(_)) => true,
        (Step::EachIndex, Step::EachIndex) => true,
        _ => false,
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str(".");
        }
        for step in &self.0 {
            match step {
                Step::Property(k) => write_key(f, k)?,
                Step::Index(i) => write!(f, "[{i}]")?,
                Step::EachIndex => f.write_str("[*]")?,
                Step::EachMember => f.write_str("[]")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_jq() {
        let s = Selector::root()
            .property("response")
            .property("odd key")
            .index(3)
            .each_index()
            .each_member();
        assert_eq!(s.to_string(), ".response.\"odd key\"[3][*][]");
        assert_eq!(Selector::root().to_string(), ".");
        let p = Path(vec![
            Segment::key("a"),
            Segment::Index(0),
            Segment::key("b-c"),
        ]);
        assert_eq!(p.to_string(), ".a[0].\"b-c\"");
        assert_eq!(Path::root().to_string(), ".");
    }

    #[test]
    fn from_segments_is_single() {
        let s = Selector::from_segments(&[Segment::key("account"), Segment::key("balance")]);
        assert_eq!(s.to_string(), ".account.balance");
        assert!(!s.is_multi());
        assert!(Selector::root().each_index().is_multi());
    }

    #[test]
    fn overlap() {
        let rows = Selector::root().property("records").each_index();
        let meta = Selector::root().property("metadata");
        let inner = Selector::root().property("records").index(2).property("x");
        assert!(!rows.may_overlap(&meta));
        assert!(rows.may_overlap(&inner));
        assert!(inner.may_overlap(&rows));
        assert!(rows.may_overlap(&rows));
        assert!(Selector::root().may_overlap(&meta));
        assert!(Selector::root().each_member().may_overlap(&meta));
        assert!(!Selector::root().each_index().may_overlap(&meta));
    }

    #[test]
    fn compose_appends() {
        let a = Selector::root().property("a");
        let b = Selector::root().each_index();
        assert_eq!(a.compose(&b).to_string(), ".a[*]");
    }
}
