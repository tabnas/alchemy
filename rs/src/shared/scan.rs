//! `scan-emit`'s step result: the next state and what to emit for it.
//!
//! A pure step function takes the state and one item and returns a
//! [`Transition`]; the operator that owns the state and the iteration is
//! transduce's `ScanEmit`, which alchemy reaches through
//! [`Routers::scan_emit`](crate::shared::Routers::scan_emit).

/// The result of one step: the next state and what to emit for it.
#[derive(Clone, Debug, PartialEq)]
pub struct Transition<S, O> {
    pub state: S,
    pub outputs: Vec<O>,
}

impl<S, O> Transition<S, O> {
    pub fn new(state: S, outputs: Vec<O>) -> Self {
        Transition { state, outputs }
    }

    /// A step that emits nothing.
    pub fn stay(state: S) -> Self {
        Transition {
            state,
            outputs: Vec::new(),
        }
    }

    /// A step that emits one item.
    pub fn emit(state: S, output: O) -> Self {
        Transition {
            state,
            outputs: vec![output],
        }
    }
}
