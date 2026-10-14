//! Text output: the boundary every renderer writes its fragments to.
//!
//! A renderer produces many small fragments (a quote, a field, a comma) and
//! must never hold a whole document. [`TextOut`] is the boundary: a
//! fragment in, a failure out. [`JoinOut`] is a text output that also
//! knows logical items, the shape a join answers. The writers and the text
//! combinators that implement them (render's `WriteOut`, `Join`,
//! `ReplaceText`) are render's, and alchemy reaches them through
//! [`Renderers`](crate::shared::Renderers).

use crate::shared::error::Fail;

/// A consumer of text fragments.
///
/// Fragments arrive in order and are concatenated; where the boundaries
/// fall carries no meaning. `flush` pushes everything held so far to the
/// final destination, and a renderer calls it exactly once, at the end of
/// the protocol it renders, so that a document that failed half way is not
/// flushed as if it were whole.
pub trait TextOut {
    fn write_str(&mut self, s: &str) -> Result<(), Fail>;
    fn flush(&mut self) -> Result<(), Fail>;

    /// Whether any text has reached the final destination, so that a
    /// failure found now leaves partial output behind. A renderer asks this
    /// when it fails and reports `committed_output` from the answer, which
    /// is how a host knows to print `output: "partial"` rather than
    /// `"none"`. The default is the conservative answer for an output that
    /// cannot tell: whatever the renderer handed over may be out. render's
    /// `WriteOut` answers exactly, from the bytes its writer received; a
    /// fragment that is still buffered is not committed, and `into_inner`
    /// drops it rather than sending it after the fact.
    fn has_committed(&self) -> bool {
        true
    }

    /// Mark the end of a record: the text written so far ends with a
    /// whole record. A text that is a sequence of records marks the end of
    /// each one: a CSV row, a value directly inside a JSON document's root
    /// container, an item `concat-map` writes, or wherever a step of
    /// `scan-emit` answers `:record-end` among its outputs. render's
    /// `WriteOut` holds a record's text until its end is marked, so a run
    /// that fails part way leaves whole records behind and none half
    /// written. The default does nothing, which is right for an output
    /// that is the destination itself or holds the whole text. A
    /// combinator passes the mark on, after any text it holds that comes
    /// before it.
    fn record_end(&mut self) -> Result<(), Fail> {
        Ok(())
    }
}

impl<O: TextOut + ?Sized> TextOut for &mut O {
    fn write_str(&mut self, s: &str) -> Result<(), Fail> {
        (**self).write_str(s)
    }

    fn flush(&mut self) -> Result<(), Fail> {
        (**self).flush()
    }

    fn has_committed(&self) -> bool {
        (**self).has_committed()
    }

    fn record_end(&mut self) -> Result<(), Fail> {
        (**self).record_end()
    }
}

impl<O: TextOut + ?Sized> TextOut for Box<O> {
    fn write_str(&mut self, s: &str) -> Result<(), Fail> {
        (**self).write_str(s)
    }

    fn flush(&mut self) -> Result<(), Fail> {
        (**self).flush()
    }

    fn has_committed(&self) -> bool {
        (**self).has_committed()
    }

    fn record_end(&mut self) -> Result<(), Fail> {
        (**self).record_end()
    }
}

/// A [`TextOut`] that writes a separator between logical items: what
/// [`Renderers::join`](crate::shared::Renderers::join) answers.
///
/// An item is what lies between [`JoinOut::item_start`] and
/// [`JoinOut::item_end`]; it may be written in any number of fragments, or
/// in none, and an empty item is still an item. A fragment written outside
/// an item is an item of its own. The separator goes before every item but
/// the first, never between the fragments of one item.
pub trait JoinOut: TextOut {
    /// Begin an item: the separator is written now if an item came before.
    /// Starting an item inside an item is `PROTOCOL_ORDER_ERROR`.
    fn item_start(&mut self) -> Result<(), Fail>;

    /// End the current item. Ending when no item is open is
    /// `PROTOCOL_ORDER_ERROR`.
    fn item_end(&mut self) -> Result<(), Fail>;
}
