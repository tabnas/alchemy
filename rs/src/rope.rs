//! A string value: its text in one piece, or, once `string-join` makes
//! one of [`ROPE_MIN`] bytes or more, a rope: the chunks it was joined
//! from, kept in a persistent vector, and its length. A join onto a rope
//! copies at most [`CHUNK_MAX`] bytes whatever the rope's length, and the
//! rope it joined onto is unchanged and shares every chunk with the new
//! one, so a text a program builds a piece at a time (a `:text` rule's
//! text, a pass at a time) costs each piece its own bytes, not the text's
//! so far. A rope's length is known without reading its text; the text in
//! one piece is made the first time something reads it, and kept.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

use crate::pvec::{PVec, Summary};

/// The joined length from which `string-join` makes a rope rather than
/// one piece: a shorter text is copied whole at each join, which costs
/// less than its chunks would.
pub const ROPE_MIN: usize = 4096;

/// The most a rope's last chunk holds when a join adds to it: a piece
/// joins the last chunk, both copied into a new one, while the two fit;
/// otherwise the last chunk is complete and the piece begins the next.
pub const CHUNK_MAX: usize = 256;

/// A string value's text.
#[derive(Clone)]
pub struct Str(Repr);

#[derive(Clone)]
enum Repr {
    Flat(Arc<str>),
    Rope(Arc<Rope>),
}

struct Rope {
    /// The chunks before the last, in order.
    chunks: PVec<Arc<str>, Unsummed>,
    /// The last chunk, which a short piece joins.
    last: Arc<str>,
    len: usize,
    /// The text in one piece, once something has read it.
    flat: OnceLock<Arc<str>>,
}

/// A rope's chunks keep no sum: its length is kept whole.
#[derive(Clone, Copy, Default)]
pub struct Unsummed;

impl Summary for Unsummed {
    type Item = Arc<str>;

    fn of(_: &Arc<str>) -> Self {
        Unsummed
    }

    fn join(self, _: Self) -> Self {
        Unsummed
    }
}

impl Str {
    /// The text's length in bytes, which a rope keeps without reading its
    /// text.
    pub fn len(&self) -> usize {
        match &self.0 {
            Repr::Flat(text) => text.len(),
            Repr::Rope(rope) => rope.len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The text in one piece, shared.
    pub fn arc(&self) -> Arc<str> {
        match &self.0 {
            Repr::Flat(text) => text.clone(),
            Repr::Rope(rope) => rope.flat().clone(),
        }
    }

    /// The strings of `items` joined, `separator` between them: `len`
    /// bytes in all, which the caller has counted (and bounded). From
    /// [`ROPE_MIN`] bytes it is a rope, which shares the chunks of a rope
    /// it begins with and copies what it joins to them at most
    /// [`CHUNK_MAX`] bytes at a time.
    pub fn join(separator: &str, items: &[&Str], len: usize) -> Str {
        let Some((first, rest)) = items.split_first() else {
            return Str::from("");
        };
        if len < ROPE_MIN {
            let mut out = String::with_capacity(len);
            out.push_str(first);
            for item in rest {
                out.push_str(separator);
                out.push_str(item);
            }
            return Str::from(out);
        }
        let mut parts = Parts::of(first);
        let separator = (!separator.is_empty()).then(|| Arc::<str>::from(separator));
        for item in rest {
            if let Some(separator) = &separator {
                parts.add(separator);
            }
            match &item.0 {
                Repr::Flat(text) => parts.add(text),
                Repr::Rope(rope) => {
                    for chunk in &rope.chunks {
                        parts.add(chunk);
                    }
                    parts.add(&rope.last);
                }
            }
        }
        debug_assert_eq!(parts.len, len, "the length counted is the length joined");
        Str(Repr::Rope(Arc::new(Rope {
            chunks: parts.chunks,
            last: parts.last,
            len: parts.len,
            flat: OnceLock::new(),
        })))
    }

    /// Whether the text is held as a rope, for the tests.
    #[cfg(test)]
    fn is_rope(&self) -> bool {
        matches!(self.0, Repr::Rope(_))
    }
}

/// A rope being joined: its chunks, its last chunk and its length.
struct Parts {
    chunks: PVec<Arc<str>, Unsummed>,
    last: Arc<str>,
    len: usize,
}

impl Parts {
    /// A rope's parts, shared, or a text in one piece as the last chunk.
    fn of(text: &Str) -> Parts {
        match &text.0 {
            Repr::Flat(text) => Parts {
                chunks: PVec::new(),
                last: text.clone(),
                len: text.len(),
            },
            Repr::Rope(rope) => Parts {
                chunks: rope.chunks.clone(),
                last: rope.last.clone(),
                len: rope.len,
            },
        }
    }

    /// `piece` after the rest: joined to the last chunk when the two fit
    /// in [`CHUNK_MAX`], otherwise after it, as the new last chunk.
    fn add(&mut self, piece: &Arc<str>) {
        if piece.is_empty() {
            return;
        }
        self.len += piece.len();
        if self.last.len() + piece.len() <= CHUNK_MAX {
            let mut joined = String::with_capacity(self.last.len() + piece.len());
            joined.push_str(&self.last);
            joined.push_str(piece);
            self.last = Arc::from(joined);
            #[cfg(test)]
            COPIED.with(|copied| copied.set(copied.get() + self.last.len()));
        } else if self.last.is_empty() {
            self.last = piece.clone();
        } else {
            let last = std::mem::replace(&mut self.last, piece.clone());
            self.chunks = self.chunks.push(last);
        }
    }
}

#[cfg(test)]
thread_local! {
    /// The bytes the joins onto ropes have copied, for the tests.
    static COPIED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Rope {
    fn flat(&self) -> &Arc<str> {
        self.flat.get_or_init(|| {
            let mut text = String::with_capacity(self.len);
            for chunk in &self.chunks {
                text.push_str(chunk);
            }
            text.push_str(&self.last);
            Arc::from(text)
        })
    }
}

impl Deref for Str {
    type Target = str;

    fn deref(&self) -> &str {
        match &self.0 {
            Repr::Flat(text) => text,
            Repr::Rope(rope) => rope.flat(),
        }
    }
}

impl AsRef<str> for Str {
    fn as_ref(&self) -> &str {
        self
    }
}

impl From<&str> for Str {
    fn from(text: &str) -> Self {
        Str(Repr::Flat(Arc::from(text)))
    }
}

impl From<String> for Str {
    fn from(text: String) -> Self {
        Str(Repr::Flat(Arc::from(text)))
    }
}

impl From<Arc<str>> for Str {
    fn from(text: Arc<str>) -> Self {
        Str(Repr::Flat(text))
    }
}

impl PartialEq for Str {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && **self == **other
    }
}

impl Eq for Str {}

impl Hash for Str {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

impl fmt::Debug for Str {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl fmt::Display for Str {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn join(separator: &str, items: &[&Str]) -> Str {
        let len = items.iter().map(|item| item.len()).sum::<usize>()
            + separator.len() * items.len().saturating_sub(1);
        Str::join(separator, items, len)
    }

    #[test]
    fn a_short_join_is_one_piece_and_a_long_one_a_rope() {
        let a = Str::from("ab");
        let short = join(", ", &[&a, &a]);
        assert_eq!(&*short, "ab, ab");
        assert!(!short.is_rope());
        let half = Str::from("x".repeat(ROPE_MIN / 2));
        let long = join("-", &[&half, &half]);
        assert!(long.is_rope());
        assert_eq!(long.len(), ROPE_MIN + 1);
        assert_eq!(&*long, format!("{0}-{0}", "x".repeat(ROPE_MIN / 2)));
        assert_eq!(long, Str::from(&*long));
    }

    #[test]
    fn a_text_joined_a_piece_at_a_time_copies_each_piece_a_bounded_number_of_times() {
        // The library's transduce-write: the text so far, then a piece,
        // 100,000 times over. Copying the text so far each time is
        // quadratic in the pieces; a rope copies each piece's bytes into
        // at most CHUNK_MAX bytes of a last chunk.
        let piece = Str::from("token");
        let mut text = Str::from("");
        let mut model = String::new();
        COPIED.with(|copied| copied.set(0));
        for _ in 0..100_000 {
            let len = text.len() + piece.len();
            text = Str::join("", &[&text, &piece], len);
            model.push_str(&piece);
        }
        let copied = COPIED.with(|copied| copied.get());
        assert!(text.is_rope());
        assert_eq!(text.len(), model.len());
        // Each join copies the last chunk, at most CHUNK_MAX bytes.
        assert!(
            copied <= 100_000 * CHUNK_MAX,
            "{copied} bytes copied for {} joined",
            model.len()
        );
        assert_eq!(&*text, model);
        assert_eq!(text.arc().len(), model.len());
    }

    #[test]
    fn a_join_onto_a_rope_leaves_the_rope_as_it_was() {
        let base = join("", &[&Str::from("a".repeat(ROPE_MIN)), &Str::from("b")]);
        let left = join("", &[&base, &Str::from("L")]);
        let right = join("", &[&base, &Str::from("R")]);
        assert_eq!(base.len(), ROPE_MIN + 1);
        assert!(left.ends_with("bL") && right.ends_with("bR"));
        assert!(base.ends_with('b'));
        // A rope inside a join is read chunk by chunk, in order.
        let both = join("|", &[&left, &right]);
        assert_eq!(&*both, format!("{}|{}", &*left, &*right));
        assert_eq!(both.len(), 2 * (ROPE_MIN + 2) + 1);
    }

    #[test]
    fn strings_compare_and_hash_by_their_text() {
        use std::collections::hash_map::DefaultHasher;
        let hash = |s: &Str| {
            let mut h = DefaultHasher::new();
            s.hash(&mut h);
            h.finish()
        };
        let text = "y".repeat(ROPE_MIN + 10);
        let rope = join(
            "",
            &[&Str::from(&text[..ROPE_MIN]), &Str::from(&text[ROPE_MIN..])],
        );
        let flat = Str::from(text.as_str());
        assert!(rope.is_rope() && !flat.is_rope());
        assert_eq!(rope, flat);
        assert_eq!(hash(&rope), hash(&flat));
        assert_ne!(rope, Str::from(&text[1..]));
        assert_eq!(format!("{rope:?}"), format!("{text:?}"));
    }
}
