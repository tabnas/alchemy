//! Captures as data: what a router is asked to recognize and what it
//! delivers.
//!
//! A router (transduce's `Router`, which alchemy reaches through
//! [`Routers::router`](crate::shared::Routers::router)) feeds every event
//! through one matcher for all of its [`CaptureSpec`]s and hands each
//! completed match to a [`RouteSink`]. A `Materialize` capture is built into
//! a [`Datum`] under a byte budget and delivered when its value completes;
//! an `Observe` capture delivers only the path, at the value's end, and
//! retains nothing. These are the types on either side of it; the router
//! itself is transduce's.

use std::sync::Arc;

use crate::shared::datum::Datum;
use crate::shared::error::Fail;
use crate::shared::matcher::CaptureId;
use crate::shared::selector::{Path, Selector};
use crate::shared::sink::Flow;

/// What a capture keeps of its match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMode {
    /// Build the value and deliver it whole.
    Materialize,
    /// Deliver only that the value occurred, and where, when it ends.
    Observe,
}

/// The byte budget one materialized capture may not exceed, named after
/// the `Limits` field the failure reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub bytes: usize,
    pub name: &'static str,
}

/// One capture: a tag for the consumer, the selector to match, the mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureSpec {
    /// Shared with every `Selected` delivered for this capture, so a
    /// delivery costs a reference count and no allocation.
    pub tag: Arc<str>,
    pub selector: Selector,
    pub mode: CaptureMode,
    /// The budget for a `Materialize` capture; `None` takes the router's
    /// `max_capture_bytes`. A stage with a more specific limit (a table's
    /// rows under `max_record_bytes`) sets it so the failure names that.
    pub budget: Option<Budget>,
}

impl CaptureSpec {
    pub fn new(tag: impl Into<Arc<str>>, selector: Selector, mode: CaptureMode) -> CaptureSpec {
        CaptureSpec {
            tag: tag.into(),
            selector,
            mode,
            budget: None,
        }
    }

    pub fn materialize(tag: impl Into<Arc<str>>, selector: Selector) -> CaptureSpec {
        CaptureSpec::new(tag, selector, CaptureMode::Materialize)
    }

    pub fn observe(tag: impl Into<Arc<str>>, selector: Selector) -> CaptureSpec {
        CaptureSpec::new(tag, selector, CaptureMode::Observe)
    }

    pub fn budget(mut self, bytes: usize, name: &'static str) -> CaptureSpec {
        self.budget = Some(Budget { bytes, name });
        self
    }
}

/// One completed match.
#[derive(Clone, Debug, PartialEq)]
pub struct Selected {
    /// The spec's position in the router's list, for dispatch without a
    /// string comparison.
    pub id: CaptureId,
    pub tag: Arc<str>,
    pub path: Path,
    /// The value for a `Materialize` capture; `None` for `Observe`.
    pub value: Option<Datum>,
}

/// The consumer of a router's matches.
pub trait RouteSink {
    /// A capture's value is beginning at `path`'s position. Nothing has
    /// been retained for it yet, so a consumer that knows the value is
    /// out of order can refuse it here at no cost; the router adds the
    /// path to a failure that has none.
    fn began(&mut self, id: CaptureId, tag: &str) -> Result<(), Fail> {
        let _ = (id, tag);
        Ok(())
    }

    fn selected(&mut self, selected: Selected) -> Result<Flow, Fail>;

    /// The document ended, validated. Exactly once, after the last match.
    fn end(&mut self) -> Result<Flow, Fail>;
}

impl RouteSink for Vec<Selected> {
    fn selected(&mut self, selected: Selected) -> Result<Flow, Fail> {
        self.push(selected);
        Ok(Flow::Continue)
    }

    fn end(&mut self) -> Result<Flow, Fail> {
        Ok(Flow::Continue)
    }
}

/// A route sink made of a closure; `end` is a no-op.
pub struct FnRoute<F>(pub F);

impl<F> RouteSink for FnRoute<F>
where
    F: FnMut(Selected) -> Result<Flow, Fail>,
{
    fn selected(&mut self, selected: Selected) -> Result<Flow, Fail> {
        (self.0)(selected)
    }

    fn end(&mut self) -> Result<Flow, Fail> {
        Ok(Flow::Continue)
    }
}
