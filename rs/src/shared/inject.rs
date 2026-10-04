//! The interfaces a compiled program is lowered through.
//!
//! alchemy's runtime turns a plan into a chain of stages: a router for its
//! captures, the table transducer, a `scan-emit` operator, the source's
//! guard on the events a program makes, and the renderers, writers and
//! text combinators its output goes through. It constructs none of them
//! itself. The host passes a [`Routers`] and a [`Renderers`] in (to
//! `compile`, which keeps them with the program), and every stage is made
//! through them: transduce implements [`Routers`], render implements
//! [`Renderers`], and neither depends on the other or on the language.
//!
//! Each method mirrors one constructor of the crate that implements it,
//! parameter for parameter, and answers a boxed trait object, so both
//! traits are object safe and a program holds them as
//! `Arc<dyn Routers<V>>` and `Arc<dyn Renderers>`.

use std::io::Write;
use std::sync::Arc;

use crate::shared::csv::CsvOptions;
use crate::shared::datum::Duplicates;
use crate::shared::error::Fail;
use crate::shared::json::JsonOptions;
use crate::shared::limits::{AbortFlag, Limits, Metrics};
use crate::shared::route::{CaptureSpec, RouteSink};
use crate::shared::scan::Transition;
use crate::shared::sink::{Flow, Sink};
use crate::shared::table::{TableBinding, TableSink};
use crate::shared::text::{JoinOut, TextOut};

/// A `scan-emit` step: the state and one item in, the next state and the
/// items to emit for it out.
pub type ScanStep<V> = Box<dyn FnMut(V, V) -> Result<Transition<V, V>, Fail> + Send>;

/// A `scan-emit` finish: the final state in, the closing items out.
pub type ScanFinish<V> = Box<dyn FnOnce(V) -> Result<Vec<V>, Fail> + Send>;

/// Where a `scan-emit` sends each item it emits.
pub type ScanOut<V> = Box<dyn FnMut(V) -> Result<Flow, Fail> + Send>;

/// The `scan-emit` operator, as [`Routers::scan_emit`] answers it: fed one
/// item at a time, then finished exactly once, when the input completed
/// successfully, and never after a failure or a cancellation.
pub trait ScanEmitter<V>: Send {
    /// One input item. Its outputs go downstream before this returns.
    fn item(&mut self, item: V) -> Result<Flow, Fail>;

    /// The input completed: emit the closing items.
    fn finish(&mut self) -> Result<Flow, Fail>;
}

/// The routing stages a plan's streams run through, implemented by
/// transduce (`tabnas_transduce::routers()`).
///
/// `V` is the type of a `scan-emit`'s state, items and outputs: a
/// program's values (`tabnas_alchemy::value::Val`) for the routers a
/// program is compiled with.
pub trait Routers<V>: Send + Sync {
    /// transduce's `Router::new`: every capture of `specs` recognized in
    /// one pass over the events, each completed match handed to
    /// `downstream`. Two specs that may select overlapping scopes are
    /// refused (`CAPTURE_OVERLAP_UNSUPPORTED`) unless both observe.
    fn router(
        &self,
        specs: Vec<CaptureSpec>,
        limits: &Limits,
        duplicates: Duplicates,
        metrics: Arc<Metrics>,
        downstream: Box<dyn RouteSink + Send>,
    ) -> Result<Box<dyn Sink + Send>, Fail>;

    /// transduce's `TableFromJson::new`: the metadata-first table
    /// transducer, `JsonEvents/1` in and `TableRows/1` out to `sink`.
    fn table_from_json(
        &self,
        binding: TableBinding,
        limits: &Limits,
        duplicates: Duplicates,
        metrics: Arc<Metrics>,
        sink: Box<dyn TableSink + Send>,
    ) -> Result<Box<dyn Sink + Send>, Fail>;

    /// transduce's `ScanEmit::new`: the operator that owns the state and
    /// the iteration, over `step` and `finish`, emitting to `out`.
    fn scan_emit(
        &self,
        initial: V,
        step: ScanStep<V>,
        finish: ScanFinish<V>,
        out: ScanOut<V>,
    ) -> Box<dyn ScanEmitter<V>>;

    /// transduce's `Guarded::new`: the source limits (`max_depth`,
    /// `max_key_bytes`, `max_scalar_bytes`), the abort flag and the source
    /// metrics, on every event handed to `inner`.
    fn guarded(
        &self,
        inner: Box<dyn Sink + Send>,
        limits: &Limits,
        abort: AbortFlag,
        metrics: Arc<Metrics>,
    ) -> Box<dyn Sink + Send>;
}

/// The renderers, writers and text combinators a plan's output is written
/// through, implemented by render (`tabnas_render::renderers()`).
pub trait Renderers: Send + Sync {
    /// render's `JsonRenderer::new`: `JsonEvents/1` written to `out` as
    /// JSON text.
    fn json(&self, out: Box<dyn TextOut + Send>, options: JsonOptions) -> Box<dyn Sink + Send>;

    /// render's `CsvRenderer::new`: `TableRows/1` written to `out` as CSV,
    /// or `TARGET_VALUE_UNREPRESENTABLE` for a delimiter no CSV reader
    /// could take.
    fn csv(
        &self,
        out: Box<dyn TextOut + Send>,
        options: CsvOptions,
    ) -> Result<Box<dyn TableSink + Send>, Fail>;

    /// render's `RecordsToJson::new`: `TableRows/1` as `JsonEvents/1`, an
    /// array of objects keyed by label, handed to `sink`.
    fn records_to_json(&self, sink: Box<dyn Sink + Send>) -> Box<dyn TableSink + Send>;

    /// render's `Join::new`: `separator` written to `out` between the
    /// logical items written to the answer.
    fn join<'a>(
        &self,
        out: Box<dyn TextOut + Send + 'a>,
        separator: &str,
    ) -> Box<dyn JoinOut + Send + 'a>;

    /// render's `ReplaceText::new`: the text written to the answer reaches
    /// `out` with every occurrence of `from` replaced by `to`, however the
    /// fragments cut it.
    fn replace_text<'a>(
        &self,
        out: Box<dyn TextOut + Send + 'a>,
        from: &str,
        to: &str,
    ) -> Box<dyn TextOut + Send + 'a>;

    /// render's `WriteOut::new(writer).with_limits(limits)
    /// .with_metrics(metrics)`: fragments coalesced to a byte budget and
    /// written to `writer`, `max_output_bytes` enforced and `output_bytes`
    /// counted in `metrics`.
    fn write_out(
        &self,
        writer: Box<dyn Write + Send>,
        limits: &Limits,
        metrics: Arc<Metrics>,
    ) -> Box<dyn TextOut + Send>;
}
