//! The spec of the library's `transduce-records`, for routers that run it
//! natively.
//!
//! `transduce-records spec input` (`stdlib/transduce.alc`) makes the JSON
//! events of a document of records, each a sequence of cells, of a
//! grammar's rule passes, as `spec` says. The library's text is the
//! specification. A host's routers may run it natively
//! ([`crate::shared::Routers::transduce_records`]), and the lowering asks
//! them to only when the program's spec has the standard shape
//! [`RecordsSpec`] holds: `:document`, `:record` and `:cell` strings, and
//! where they are given, `:empty` and `:unnamed` strings, `:header` a
//! boolean and `:fill` a scalar. A spec of any other shape runs the
//! library's text, which says what it does with it.
//!
//! What a native must do is what the text does, pass for pass. Its state
//! is the phase (before the document, the header, the rows, done), the
//! header's names once read, and how many cells of the current record
//! have been read:
//!
//! - An open pass of the `:document` rule begins the document's array,
//!   and then the header when the spec has one, the rows when it has not.
//! - An open pass of the `:record` rule, among the rows, begins a record:
//!   an object when a header names the columns, an array otherwise, and
//!   its cells count from none. The header's own record begins nothing.
//! - A close pass of the `:cell` rule with a value is a cell, and so is
//!   any other pass whose alternate is in the `:empty` group, an empty
//!   cell, which holds `:fill`; a close pass of the `:cell` rule with no
//!   value is no cell. Among the header's cells, a cell's name is kept
//!   ([`records_name`]). Among the rows, a cell is announced under the
//!   name at its position, or under `:unnamed` and the position counted
//!   from 0 when the header has no name there, and is yielded as a
//!   scalar; without a header it is yielded alone. Before the document
//!   and after it, a cell is nothing.
//! - A close pass of the `:record` rule ends the header, which yields
//!   nothing and begins the rows, or a row: an object first announces
//!   each name the record lacks, each holding `:fill`, then ends; an
//!   array ends.
//! - A close pass of the `:document` rule ends the document's array.
//! - The end of the passes yields the empty array when no document began,
//!   nothing more after the document ended, and otherwise refuses with
//!   `PROTOCOL_ORDER_ERROR`.
//!
//! The rules come first, in the text's order: for an open pass the
//! `:document` rule, then the `:record` rule, then the `:empty` group;
//! for a close pass the `:cell` rule, then the `:record` rule, then the
//! `:document` rule, then the group. A name the header repeats is
//! announced as often, as the record spells it: a tree's contract after
//! the transducer refuses such a document.
//!
//! The library's scan holds its state to `max_metadata_bytes`, as every
//! scan's is held: measured before the first pass and again whenever a
//! pass changes it. A native holds its own to
//! [`RecordsSpec::max_state_bytes`] at the same points, by the same
//! measure ([`records_state_bytes`]), and refuses past it with
//! [`records_state_refusal`], so the two refuse at the same pass. The
//! scan's other bound, `max_depth`, the state meets only below a depth of
//! three; there the two refuse with the same code and different limits.

use crate::shared::datum::{write_json, Datum};
use crate::shared::error::Fail;
use crate::shared::limits::NODE_BYTES;
use crate::shared::number::number_text;

/// A `transduce-records` spec of the standard shape.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordsSpec {
    /// The rule whose open pass begins the document's array and whose
    /// close pass ends it.
    pub document: Box<str>,
    /// The rule whose open pass begins a record and whose close pass ends
    /// it.
    pub record: Box<str>,
    /// The rule whose close pass's value is the record's next cell.
    pub cell: Box<str>,
    /// The group a pass's alternate is in when the pass is an empty cell.
    pub empty: Option<Box<str>>,
    /// What an empty cell holds, and a header cell a record lacks: a
    /// scalar.
    pub fill: Datum,
    /// Whether the first record's cells name the columns.
    pub header: bool,
    /// The prefix of the name of a cell past the header's.
    pub unnamed: Box<str>,
    /// The most bytes the state may hold, by [`records_state_bytes`]: the
    /// runtime's `max_metadata_bytes`, which the library's scan holds its
    /// state to. The lowering sets it when it hands the spec to a native;
    /// unbounded until then.
    pub max_state_bytes: usize,
}

impl RecordsSpec {
    /// A spec of the three rules, with no empty group, the empty string
    /// to fill with, no header and no prefix, as the library reads a spec
    /// that leaves those out.
    pub fn new(document: &str, record: &str, cell: &str) -> RecordsSpec {
        RecordsSpec {
            document: document.into(),
            record: record.into(),
            cell: cell.into(),
            empty: None,
            fill: Datum::String("".into()),
            header: false,
            unnamed: "".into(),
            max_state_bytes: usize::MAX,
        }
    }

    /// The spec with its state held to `max` bytes.
    pub fn with_max_state_bytes(mut self, max: usize) -> RecordsSpec {
        self.max_state_bytes = max;
        self
    }

    /// The spec with the empty cells in `group`.
    pub fn with_empty(mut self, group: &str) -> RecordsSpec {
        self.empty = Some(group.into());
        self
    }

    /// The spec with the first record naming the columns, and `unnamed`
    /// the prefix of a name past them.
    pub fn with_header(mut self, unnamed: &str) -> RecordsSpec {
        self.header = true;
        self.unnamed = unnamed.into();
        self
    }

    /// The spec with `fill` for an empty cell.
    pub fn with_fill(mut self, fill: Datum) -> RecordsSpec {
        self.fill = fill;
        self
    }
}

/// The phase a native's state is in, as the library's state names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordsPhase {
    Before,
    Header,
    Row,
    Done,
}

impl RecordsPhase {
    /// The keyword the library's state holds for the phase.
    pub fn word(self) -> &'static str {
        match self {
            RecordsPhase::Before => "before",
            RecordsPhase::Header => "header",
            RecordsPhase::Row => "row",
            RecordsPhase::Done => "done",
        }
    }
}

/// The bytes the library's state `[phase names cells]` holds, by the
/// measure every scan's state is held to: [`NODE_BYTES`] for each node,
/// the vector, the phase, the names' vector and each name, the cells'
/// vector and each cell's marker, and the bytes of the phase's keyword
/// and of each name.
pub fn records_state_bytes<'n>(
    phase: RecordsPhase,
    names: impl IntoIterator<Item = &'n str>,
    cells: usize,
) -> u64 {
    let node = NODE_BYTES as u64;
    let mut bytes = node * 4 + phase.word().len() as u64 + node * cells as u64;
    for name in names {
        bytes = bytes.saturating_add(node + name.len() as u64);
    }
    bytes
}

/// The library's refusal of a state past
/// [`RecordsSpec::max_state_bytes`]: the scan's own.
pub fn records_state_refusal(max: usize) -> Fail {
    Fail::limit(
        "max_metadata_bytes",
        max as u64,
        format!("the scan-emit state holds more than {max} bytes"),
    )
}

/// The name a header cell gives its column, as the library's
/// `transduce-records-name` writes it: a string as it is, null and a
/// boolean as the words `null`, `true` and `false`, a number as
/// `scalar-text` writes it (its lexeme, or the shortest text that reads
/// back; a non-finite one has none, `TARGET_VALUE_UNREPRESENTABLE`), and
/// a container as its compact JSON text, which no pass's value is, since
/// a pass carries a scalar.
pub fn records_name(value: &Datum) -> Result<String, Fail> {
    match value {
        Datum::String(s) => Ok(s.to_string()),
        Datum::Null => Ok("null".into()),
        Datum::Bool(true) => Ok("true".into()),
        Datum::Bool(false) => Ok("false".into()),
        Datum::Number { value, lexeme } => number_text(*value, lexeme.as_deref()),
        Datum::Array(_) | Datum::Object(_) => {
            let mut text = String::new();
            write_json(value, &mut text);
            Ok(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_is_measured_as_the_scan_measures_it() {
        // [:before [] []]: four nodes, and the six bytes of `before`.
        assert_eq!(records_state_bytes(RecordsPhase::Before, [], 0), 70);
        // [:row ["a" "bc"] [true]]: seven nodes, three bytes of `row`
        // and three of the names.
        assert_eq!(
            records_state_bytes(RecordsPhase::Row, ["a", "bc"], 1),
            7 * 16 + 3 + 3
        );
    }

    #[test]
    fn a_header_cell_is_named_by_its_text() {
        assert_eq!(records_name(&Datum::String("id".into())).unwrap(), "id");
        assert_eq!(records_name(&Datum::Null).unwrap(), "null");
        assert_eq!(records_name(&Datum::Bool(false)).unwrap(), "false");
        let n = Datum::Number {
            value: 1.5,
            lexeme: Some("1.50".into()),
        };
        assert_eq!(records_name(&n).unwrap(), "1.50");
        let n = Datum::Number {
            value: 1e21,
            lexeme: None,
        };
        assert_eq!(records_name(&n).unwrap(), "1e21");
    }
}
