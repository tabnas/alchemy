//! The JSON profile: the options render's `JsonRenderer` takes, which
//! alchemy hands to [`Renderers::json`](crate::shared::Renderers::json).

/// The JSON profile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JsonOptions {
    /// Spaces per nesting level, with a newline before every item and
    /// every closing bracket of a non-empty container. `None` or `Some(0)`
    /// is compact: no whitespace at all.
    pub indent: Option<usize>,
    /// Write a newline after the root value, at `End`.
    pub trailing_newline: bool,
}
