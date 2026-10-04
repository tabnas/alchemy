//! The matcher's capture identifier: the one part of transduce's
//! shared-prefix matcher that crosses into the types every stage shares.

/// Which selector matched: its position in the slice given to
/// transduce's `Matcher::new`.
pub type CaptureId = usize;
