//! `TableRows/1`: one schema, ordered finite rows, completion.
//!
//! The protocol is flat and budgeted: a row is a finite vector of cells,
//! already projected into schema order by the transducer, and a renderer
//! never sees a source path. Public columns carry a label; the source
//! binding (`BoundColumn`) stays on the transducer's side of the boundary.

use std::fmt;

use crate::shared::datum::Datum;
use crate::shared::error::Fail;
use crate::shared::selector::{Segment, Selector};
use crate::shared::sink::Flow;

/// One projected value.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Null,
    Bool(bool),
    Number {
        value: f64,
        lexeme: Option<Box<str>>,
    },
    String(Box<str>),
    /// The source had no value at the column's path. Not null, not the
    /// empty string, not zero: a policy maps or rejects it later.
    Missing,
}

impl Cell {
    /// From a retained value. A container is not a cell; it is
    /// serialized as compact JSON text, which is the lossy but
    /// unambiguous choice the standard binding makes and documents.
    pub fn from_datum(d: &Datum) -> Cell {
        match d {
            Datum::Null => Cell::Null,
            Datum::Bool(b) => Cell::Bool(*b),
            Datum::Number { value, lexeme } => Cell::Number {
                value: *value,
                lexeme: lexeme.clone(),
            },
            Datum::String(s) => Cell::String(s.clone()),
            Datum::Array(_) | Datum::Object(_) => Cell::String(d.to_string().into()),
        }
    }

    /// [`Cell::from_datum`] for a value the caller owns: the text moves
    /// instead of being copied.
    pub fn from_owned(d: Datum) -> Cell {
        match d {
            Datum::Null => Cell::Null,
            Datum::Bool(b) => Cell::Bool(b),
            Datum::Number { value, lexeme } => Cell::Number { value, lexeme },
            Datum::String(s) => Cell::String(s),
            Datum::Array(_) | Datum::Object(_) => Cell::String(d.to_string().into()),
        }
    }

    pub fn is_missing(&self) -> bool {
        matches!(self, Cell::Missing)
    }

    /// The bytes this cell retains, on the same basis as `Datum::byte_size`.
    pub fn byte_size(&self) -> usize {
        match self {
            Cell::Null | Cell::Bool(_) | Cell::Missing => crate::shared::limits::NODE_BYTES,
            Cell::Number { lexeme, .. } => {
                crate::shared::limits::NODE_BYTES + lexeme.as_ref().map_or(8, |l| l.len())
            }
            Cell::String(s) => crate::shared::limits::NODE_BYTES + s.len(),
        }
    }
}

impl fmt::Display for Cell {
    /// The cell as JSON text; `Missing` prints as `missing`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Cell::Null => f.write_str("null"),
            Cell::Bool(b) => write!(f, "{b}"),
            Cell::Number { value, lexeme } => match lexeme {
                Some(l) => f.write_str(l),
                None => write!(f, "{value}"),
            },
            Cell::String(s) => {
                let mut out = String::new();
                crate::shared::datum::write_json_string(s, &mut out);
                f.write_str(&out)
            }
            Cell::Missing => f.write_str("missing"),
        }
    }
}

/// What a renderer knows about a column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicColumn {
    pub label: Box<str>,
}

impl PublicColumn {
    pub fn new(label: impl Into<Box<str>>) -> PublicColumn {
        PublicColumn {
            label: label.into(),
        }
    }
}

/// One event of `TableRows/1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TableEvent<'a> {
    /// Exactly one, first.
    Schema(&'a [PublicColumn]),
    /// As many as there are rows, each exactly as wide as the schema.
    Row(&'a [Cell]),
    /// Exactly one, last, and only after the source validated to its end.
    End,
}

/// A consumer of `TableRows/1`.
pub trait TableSink {
    fn table_event(&mut self, ev: TableEvent<'_>) -> Result<Flow, Fail>;
}

impl<S: TableSink + ?Sized> TableSink for &mut S {
    fn table_event(&mut self, ev: TableEvent<'_>) -> Result<Flow, Fail> {
        (**self).table_event(ev)
    }
}

impl<S: TableSink + ?Sized> TableSink for Box<S> {
    fn table_event(&mut self, ev: TableEvent<'_>) -> Result<Flow, Fail> {
        (**self).table_event(ev)
    }
}

/// An owned recording of a table, for tests and small results.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub columns: Vec<PublicColumn>,
    pub rows: Vec<Vec<Cell>>,
    pub ended: bool,
}

impl TableSink for Table {
    fn table_event(&mut self, ev: TableEvent<'_>) -> Result<Flow, Fail> {
        match ev {
            TableEvent::Schema(c) => self.columns = c.to_vec(),
            TableEvent::Row(r) => self.rows.push(r.to_vec()),
            TableEvent::End => self.ended = true,
        }
        Ok(Flow::Continue)
    }
}

/// What to do when a row has no value at a column's path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MissingPolicy {
    /// Deliver [`Cell::Missing`]; the renderer's policy decides.
    #[default]
    Missing,
    /// Deliver `null`.
    Null,
    /// Fail the run with `MISSING_VALUE`.
    Error,
}

/// A column as the transducer binds it: the public label, and the source
/// path projected from each row. Never crosses into a renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundColumn {
    pub label: Box<str>,
    pub source: Vec<Segment>,
    pub missing: MissingPolicy,
}

impl BoundColumn {
    pub fn new(label: impl Into<Box<str>>, source: Vec<Segment>) -> BoundColumn {
        BoundColumn {
            label: label.into(),
            source,
            missing: MissingPolicy::Missing,
        }
    }

    pub fn public(&self) -> PublicColumn {
        PublicColumn {
            label: self.label.clone(),
        }
    }
}

/// Maps one metadata descriptor to a bound column.
pub type ColumnMapper = Box<dyn Fn(&Datum) -> Result<BoundColumn, Fail> + Send + Sync>;

/// Where a table's columns come from.
pub enum Schema {
    /// Declared by the caller; no metadata is read from the source.
    Static(Vec<BoundColumn>),
    /// Selected from the source and mapped one descriptor at a time. The
    /// metadata must complete before the first row begins.
    FromMetadata {
        columns: Selector,
        column: ColumnMapper,
    },
    /// The first row's member names, in its order. Data-dependent, and
    /// documented as such: a later row's extra members are dropped, its
    /// absent ones are `Missing`.
    Infer,
}

impl fmt::Debug for Schema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Schema::Static(c) => f.debug_tuple("Static").field(c).finish(),
            Schema::FromMetadata { columns, .. } => f
                .debug_struct("FromMetadata")
                .field("columns", columns)
                .finish_non_exhaustive(),
            Schema::Infer => f.write_str("Infer"),
        }
    }
}

/// A table transducer's source binding.
#[derive(Debug)]
pub struct TableBinding {
    pub schema: Schema,
    /// Each location this names is one row.
    pub rows: Selector,
}

/// The standard mapping from a metadata descriptor to a column: the
/// spec's `column-from-meta`, reading `title` and a `path` of segments.
pub fn column_from_meta(meta: &Datum) -> Result<BoundColumn, Fail> {
    let obj = meta
        .as_object()
        .ok_or_else(|| Fail::input("a column descriptor is not an object"))?;
    let label = obj
        .get("title")
        .and_then(Datum::as_str)
        .ok_or_else(|| Fail::input("a column descriptor has no string \"title\""))?;
    let path = obj
        .get("path")
        .and_then(Datum::as_array)
        .ok_or_else(|| Fail::input(format!("column {label:?} has no \"path\" array")))?;
    let source = path
        .iter()
        .map(|seg| match seg {
            Datum::String(s) => Ok(Segment::Key(s.clone())),
            Datum::Number { value, .. }
                if *value >= 0.0 && value.fract() == 0.0 && *value <= u32::MAX as f64 =>
            {
                Ok(Segment::Index(*value as usize))
            }
            other => Err(Fail::input(format!(
                "column {label:?} has a path segment that is neither a string nor a non-negative integer: {other}"
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BoundColumn::new(label, source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_from_meta_reads_title_and_path() {
        let meta = Datum::from_json(
            &serde_json::json!({"title": "Balance", "path": ["account", "balance"]}),
        );
        let c = column_from_meta(&meta).unwrap();
        assert_eq!(&*c.label, "Balance");
        assert_eq!(
            c.source,
            vec![Segment::key("account"), Segment::key("balance")]
        );
        let meta = Datum::from_json(&serde_json::json!({"title": "First", "path": ["tags", 0]}));
        assert_eq!(
            column_from_meta(&meta).unwrap().source[1],
            Segment::Index(0)
        );
        let bad = Datum::from_json(&serde_json::json!({"title": "x", "path": ["a", -1]}));
        assert_eq!(
            column_from_meta(&bad).unwrap_err().code,
            crate::shared::Code::InputInvalid
        );
        let bad = Datum::from_json(&serde_json::json!({"path": ["a"]}));
        assert!(column_from_meta(&bad).is_err());
    }

    #[test]
    fn cells_print_as_json() {
        assert_eq!(
            Cell::from_datum(&Datum::from_json(&serde_json::json!(50.25))).to_string(),
            "50.25"
        );
        assert_eq!(
            Cell::from_datum(&Datum::from_json(&serde_json::json!([1, 2]))).to_string(),
            "\"[1,2]\""
        );
        assert_eq!(Cell::Missing.to_string(), "missing");
        assert_eq!(Cell::from_datum(&Datum::Null), Cell::Null);
    }

    #[test]
    fn a_table_records() {
        let mut t = Table::default();
        let cols = [PublicColumn::new("a")];
        t.table_event(TableEvent::Schema(&cols)).unwrap();
        t.table_event(TableEvent::Row(&[Cell::Bool(true)])).unwrap();
        t.table_event(TableEvent::End).unwrap();
        assert_eq!(t.columns.len(), 1);
        assert_eq!(t.rows, vec![vec![Cell::Bool(true)]]);
        assert!(t.ended);
    }
}
