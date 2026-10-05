//! The types every tabnas transducer and renderer shares, and the two
//! interfaces a compiled program is lowered through.
//!
//! This unit depends on nothing else in the fleet: with
//! `default-features = false` it is the whole crate, and it is what
//! tabnas-transduce and tabnas-render build on.
//!
//! - the source protocol [`JsonEvent`] (`JsonEvents/1`) and the push
//!   boundary [`Sink`] ([`event`], [`sink`]);
//! - the table protocol [`TableEvent`] (`TableRows/1`), its cells,
//!   columns and bindings ([`table`]);
//! - [`Selector`]s and concrete [`Path`]s ([`selector`]), and the retained
//!   value [`Datum`] with its builder and JSON writer ([`datum`]);
//! - [`Limits`], [`Metrics`] and [`AbortFlag`] ([`limits`]), and the
//!   stable failure [`Code`]s every stage reports, carried by [`Fail`]
//!   ([`error`]);
//! - the types on either side of a router ([`route`], [`matcher`]) and of
//!   `scan-emit` ([`scan`]);
//! - the text boundary renderers write to ([`text`]) and the renderers'
//!   options ([`csv`], [`json`]);
//! - [`Routers`] and [`Renderers`] ([`inject`]): what alchemy's runtime
//!   makes its stages with, implemented by transduce and by render and
//!   passed in by the host.
//!
//! With the `tabnas` feature, which `language` turns on, `Fail::from_tabnas`
//! and `Datum::from_tabnas` convert the engine's own error and value.

pub mod csv;
pub mod datum;
pub mod error;
pub mod event;
pub mod inject;
pub mod json;
pub mod limits;
pub mod matcher;
pub mod route;
pub mod scan;
pub mod selector;
pub mod sink;
pub mod table;
pub mod text;

pub use csv::{CsvOptions, MissingText, Newline, Quoting};
pub use datum::{
    walk_datum, write_json, write_json_number, write_json_string, Datum, DatumBuilder, Duplicates,
};
pub use error::{Code, Fail, Limit};
pub use event::{JsonEvent, Number, OwnedJsonEvent};
pub use inject::{Renderers, Routers, ScanEmitter, ScanFinish, ScanOut, ScanStep};
pub use json::JsonOptions;
pub use limits::{AbortFlag, Limits, Metrics};
pub use matcher::CaptureId;
pub use route::{Budget, CaptureMode, CaptureSpec, FnRoute, RouteSink, Selected};
pub use scan::Transition;
pub use selector::{Path, Segment, Selector, Step};
pub use sink::{replay, CountSink, Flow, FnSink, Sink, TreeContract};
pub use table::{
    column_from_meta, BoundColumn, Cell, ColumnMapper, MissingPolicy, PublicColumn, Schema, Table,
    TableBinding, TableEvent, TableSink,
};
pub use text::{JoinOut, TextOut};
