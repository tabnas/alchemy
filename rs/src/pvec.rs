//! A persistent vector: the items in leaves of [`LEAF`] under a trie, and
//! the last up to [`LEAF`] of them in a tail, as Clojure's vector keeps
//! them. A push copies the tail, and once in [`LEAF`] pushes the path to
//! the leaf the full tail becomes, so it costs a bounded amount whatever
//! the vector's length, and the vector it pushed onto is unchanged and
//! shares every leaf with the new one. Every node keeps the [`Summary`] of
//! the items under it once something has asked for it, so a walk that sums
//! the items reads a leaf's sum instead of visiting its items again, and a
//! vector nothing sums costs nothing for it.

use std::fmt;
use std::sync::{Arc, OnceLock};

const BITS: u32 = 5;
/// The items in a leaf, and the most the tail holds.
pub const LEAF: usize = 1 << BITS;
const MASK: usize = LEAF - 1;

/// What a vector keeps about the items under each node.
pub trait Summary: Copy + Default + Send + Sync {
    type Item;
    /// One item's summary.
    fn of(item: &Self::Item) -> Self;
    /// Two summaries as one.
    fn join(self, other: Self) -> Self;
}

/// A vector whose every version shares what it has in common with the
/// others.
pub struct PVec<T, S> {
    /// The items before the tail, when there are any: whole leaves.
    trie: Option<Arc<Trie<T, S>>>,
    /// The last items, at least one unless the vector is empty, at most
    /// [`LEAF`].
    tail: Arc<[T]>,
}

struct Trie<T, S> {
    /// The items under the root, a multiple of [`LEAF`].
    len: usize,
    /// [`BITS`] for each level of branches above the leaves.
    shift: u32,
    root: Node<T, S>,
}

/// A node and, once asked for, the summary of the items under it, which
/// a copy of the node made after that keeps.
struct Node<T, S> {
    kind: Kind<T, S>,
    summary: OnceLock<S>,
}

enum Kind<T, S> {
    Branch(Arc<[Node<T, S>]>),
    Leaf(Arc<[T]>),
}

impl<T, S: Copy> Clone for Node<T, S> {
    fn clone(&self) -> Self {
        let kind = match &self.kind {
            Kind::Branch(children) => Kind::Branch(children.clone()),
            Kind::Leaf(items) => Kind::Leaf(items.clone()),
        };
        Node {
            kind,
            summary: self.summary.clone(),
        }
    }
}

impl<T, S> Clone for PVec<T, S> {
    fn clone(&self) -> Self {
        PVec {
            trie: self.trie.clone(),
            tail: self.tail.clone(),
        }
    }
}

impl<T: Clone, S: Summary<Item = T>> Node<T, S> {
    fn summary(&self) -> S {
        *self.summary.get_or_init(|| match &self.kind {
            Kind::Branch(children) => children
                .iter()
                .fold(S::default(), |sum, child| sum.join(child.summary())),
            Kind::Leaf(items) => items
                .iter()
                .fold(S::default(), |sum, item| sum.join(S::of(item))),
        })
    }

    fn leaf(items: Arc<[T]>) -> Self {
        Node {
            kind: Kind::Leaf(items),
            summary: OnceLock::new(),
        }
    }

    fn branch(children: Vec<Node<T, S>>) -> Self {
        Node {
            kind: Kind::Branch(children.into()),
            summary: OnceLock::new(),
        }
    }

    /// `leaf` under branches down from a node at `shift`.
    fn path(leaf: Node<T, S>, shift: u32) -> Self {
        if shift == 0 {
            leaf
        } else {
            Node::branch(vec![Node::path(leaf, shift - BITS)])
        }
    }

    /// This branch, at `shift`, with `leaf` as the leaf of item `index`,
    /// the first after every item under it.
    fn with_leaf(&self, shift: u32, index: usize, leaf: Node<T, S>) -> Self {
        let Kind::Branch(children) = &self.kind else {
            unreachable!("a leaf has no room for another");
        };
        let slot = (index >> shift) & MASK;
        let mut kids = children.to_vec();
        if shift == BITS {
            kids.push(leaf);
        } else if slot < kids.len() {
            kids[slot] = kids[slot].with_leaf(shift - BITS, index, leaf);
        } else {
            kids.push(Node::path(leaf, shift - BITS));
        }
        Node::branch(kids)
    }

    /// This node, at `shift`, without its last leaf, the leaf of item
    /// `index`, and that leaf's items; `None` for a node left empty.
    fn without_last(&self, shift: u32, index: usize) -> (Option<Self>, Arc<[T]>) {
        match &self.kind {
            Kind::Leaf(items) => (None, items.clone()),
            Kind::Branch(children) => {
                let slot = (index >> shift) & MASK;
                let (child, items) = children[slot].without_last(shift - BITS, index);
                let mut kids = children[..slot].to_vec();
                kids.extend(child);
                let node = (!kids.is_empty()).then(|| Node::branch(kids));
                (node, items)
            }
        }
    }
}

impl<T, S> Trie<T, S> {
    /// The leaf holding item `index`, which is under the root.
    fn leaf(&self, index: usize) -> &[T] {
        let mut node = &self.root;
        let mut shift = self.shift;
        loop {
            match &node.kind {
                Kind::Leaf(items) => return items,
                Kind::Branch(children) => {
                    node = &children[(index >> shift) & MASK];
                    shift -= BITS;
                }
            }
        }
    }
}

impl<T: Clone, S: Summary<Item = T>> Trie<T, S> {
    fn with_leaf(&self, leaf: Node<T, S>) -> Self {
        let len = self.len + LEAF;
        if self.len == 1 << (self.shift + BITS) {
            // The root is full: a root above it and the new leaf's path.
            let root = Node::branch(vec![self.root.clone(), Node::path(leaf, self.shift)]);
            Trie {
                len,
                shift: self.shift + BITS,
                root,
            }
        } else {
            Trie {
                len,
                shift: self.shift,
                root: self.root.with_leaf(self.shift, self.len, leaf),
            }
        }
    }

    /// The trie without its last leaf, `None` when that was its only one,
    /// and the leaf's items.
    fn without_last(&self) -> (Option<Self>, Arc<[T]>) {
        let (root, items) = self.root.without_last(self.shift, self.len - LEAF);
        let Some(root) = root else {
            return (None, items);
        };
        // A root left with one child is that child, a level lower.
        let (root, shift) = match &root.kind {
            Kind::Branch(children) if children.len() == 1 => {
                (children[0].clone(), self.shift - BITS)
            }
            _ => (root, self.shift),
        };
        let trie = Trie {
            len: self.len - LEAF,
            shift,
            root,
        };
        (Some(trie), items)
    }
}

impl<T, S> PVec<T, S> {
    fn trie_len(&self) -> usize {
        self.trie.as_ref().map_or(0, |trie| trie.len)
    }
    /// Leaf `n` of the trie, or the tail after the last.
    fn chunk_at(&self, n: usize) -> &[T] {
        match self.trie.as_deref() {
            Some(trie) if n * LEAF < trie.len => trie.leaf(n * LEAF),
            _ => &self.tail,
        }
    }
}

impl<T: Clone, S: Summary<Item = T>> Default for PVec<T, S> {
    fn default() -> Self {
        PVec::new()
    }
}

impl<T: Clone, S: Summary<Item = T>> PVec<T, S> {
    pub fn new() -> Self {
        PVec {
            trie: None,
            tail: Arc::from(Vec::new()),
        }
    }

    pub fn len(&self) -> usize {
        self.trie_len() + self.tail.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tail.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        let before = self.trie_len();
        if index >= before {
            return self.tail.get(index - before);
        }
        let trie = self.trie.as_deref()?;
        trie.leaf(index).get(index & MASK)
    }

    pub fn first(&self) -> Option<&T> {
        self.get(0)
    }

    pub fn last(&self) -> Option<&T> {
        self.tail.last()
    }

    /// The items, when they are all in the tail, as a vector of at most
    /// [`LEAF`] items has them.
    pub fn as_slice(&self) -> Option<&[T]> {
        self.trie.is_none().then_some(&*self.tail)
    }

    /// This vector with `item` after its last.
    pub fn push(&self, item: T) -> Self {
        if self.tail.len() < LEAF {
            let mut tail = Vec::with_capacity(self.tail.len() + 1);
            tail.extend_from_slice(&self.tail);
            tail.push(item);
            return PVec {
                trie: self.trie.clone(),
                tail: tail.into(),
            };
        }
        let leaf = Node::leaf(self.tail.clone());
        let trie = match self.trie.as_deref() {
            None => Trie {
                len: LEAF,
                shift: 0,
                root: leaf,
            },
            Some(trie) => trie.with_leaf(leaf),
        };
        PVec {
            trie: Some(Arc::new(trie)),
            tail: Arc::from(vec![item]),
        }
    }

    /// This vector without its last item; `None` when it is empty.
    pub fn pop(&self) -> Option<Self> {
        match self.tail.len() {
            0 => None,
            1 => Some(match self.trie.as_deref() {
                None => PVec::new(),
                Some(trie) => {
                    let (trie, tail) = trie.without_last();
                    PVec {
                        trie: trie.map(Arc::new),
                        tail,
                    }
                }
            }),
            n => Some(PVec {
                trie: self.trie.clone(),
                tail: Arc::from(&self.tail[..n - 1]),
            }),
        }
    }

    /// The summary of the items before the tail, which every node keeps;
    /// `None` when there are none.
    pub fn trie_summary(&self) -> Option<S> {
        self.trie.as_ref().map(|trie| trie.root.summary())
    }

    /// The last items, those [`PVec::trie_summary`] does not sum.
    pub fn tail(&self) -> &[T] {
        &self.tail
    }

    /// The summary of every item.
    pub fn summary(&self) -> S {
        self.tail
            .iter()
            .fold(self.trie_summary().unwrap_or_default(), |sum, item| {
                sum.join(S::of(item))
            })
    }

    pub fn iter(&self) -> Iter<'_, T, S> {
        Iter(match self.as_slice() {
            Some(items) => Items::Slice(items.iter()),
            None => Items::Chunked {
                items: self.chunks().flatten(),
                left: self.len(),
            },
        })
    }

    /// The items a leaf at a time, in order, the tail last: what a walk
    /// over every item reads as slices.
    pub fn chunks(&self) -> Chunks<'_, T, S> {
        Chunks {
            vec: self,
            front: 0,
            back: self.trie_len() / LEAF + 1,
        }
    }

    /// Whether the two are one vector, not merely equal.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.tail, &other.tail)
            && match (&self.trie, &other.trie) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }

    pub fn to_vec(&self) -> Vec<T> {
        self.iter().cloned().collect()
    }
}

impl<T: Clone, S: Summary<Item = T>> From<Vec<T>> for PVec<T, S> {
    /// The items moved into leaves, the last up to [`LEAF`] of them, full
    /// or not, the tail.
    fn from(items: Vec<T>) -> Self {
        if items.len() <= LEAF {
            return PVec {
                trie: None,
                tail: Arc::from(items),
            };
        }
        let in_tail = match items.len() % LEAF {
            0 => LEAF,
            rest => rest,
        };
        let leaves = (items.len() - in_tail) / LEAF;
        let mut items = items.into_iter();
        let mut vec = PVec::new();
        for _ in 0..leaves {
            let leaf = Node::leaf(items.by_ref().take(LEAF).collect());
            let trie = match vec.trie.as_deref() {
                None => Trie {
                    len: LEAF,
                    shift: 0,
                    root: leaf,
                },
                Some(trie) => trie.with_leaf(leaf),
            };
            vec.trie = Some(Arc::new(trie));
        }
        vec.tail = items.collect();
        vec
    }
}

impl<T: Clone, S: Summary<Item = T>> std::ops::Index<usize> for PVec<T, S> {
    type Output = T;

    /// Item `index`; past the last item it panics, as a slice's does.
    fn index(&self, index: usize) -> &T {
        match self.get(index) {
            Some(item) => item,
            None => panic!(
                "index {index} is past the end of a vector of {} items",
                self.len()
            ),
        }
    }
}

impl<T: Clone, S: Summary<Item = T>> FromIterator<T> for PVec<T, S> {
    /// The items in a tail of their own when there are at most [`LEAF`]
    /// of them and the iterator knows how many, as most are: one
    /// allocation, as `Arc<[T]>` collects them.
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let iter = iter.into_iter();
        match iter.size_hint() {
            (n, Some(m)) if n == m && n <= LEAF => PVec {
                trie: None,
                tail: iter.collect(),
            },
            _ => PVec::from(iter.collect::<Vec<T>>()),
        }
    }
}

impl<T: Clone + PartialEq, S: Summary<Item = T>> PartialEq for PVec<T, S> {
    fn eq(&self, other: &Self) -> bool {
        if let (Some(a), Some(b)) = (self.as_slice(), other.as_slice()) {
            return a == b;
        }
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl<T: Clone + fmt::Debug, S: Summary<Item = T>> fmt::Debug for PVec<T, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<'v, T: Clone, S: Summary<Item = T>> IntoIterator for &'v PVec<T, S> {
    type Item = &'v T;
    type IntoIter = Iter<'v, T, S>;

    fn into_iter(self) -> Iter<'v, T, S> {
        self.iter()
    }
}

/// The leaves of a [`PVec`] in order, then its tail, each a slice.
pub struct Chunks<'v, T, S> {
    vec: &'v PVec<T, S>,
    /// The next chunk from the front, and the one after the next from
    /// the back, the tail numbered after the last leaf.
    front: usize,
    back: usize,
}

impl<'v, T, S> Iterator for Chunks<'v, T, S> {
    type Item = &'v [T];

    fn next(&mut self) -> Option<&'v [T]> {
        if self.front >= self.back {
            return None;
        }
        self.front += 1;
        Some(self.vec.chunk_at(self.front - 1))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.back - self.front;
        (n, Some(n))
    }
}

impl<T, S> DoubleEndedIterator for Chunks<'_, T, S> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front >= self.back {
            return None;
        }
        self.back -= 1;
        Some(self.vec.chunk_at(self.back))
    }
}

/// The items of a [`PVec`] in order: its chunks' items, each chunk read
/// as a slice.
pub struct Iter<'v, T, S>(Items<'v, T, S>);

enum Items<'v, T, S> {
    /// A vector all in its tail, as most are.
    Slice(std::slice::Iter<'v, T>),
    Chunked {
        items: std::iter::Flatten<Chunks<'v, T, S>>,
        /// The items not yet given from either end.
        left: usize,
    },
}

impl<'v, T, S> Iterator for Iter<'v, T, S> {
    type Item = &'v T;

    fn next(&mut self) -> Option<&'v T> {
        match &mut self.0 {
            Items::Slice(items) => items.next(),
            Items::Chunked { items, left } => {
                let item = items.next()?;
                *left -= 1;
                Some(item)
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.0 {
            Items::Slice(items) => items.size_hint(),
            Items::Chunked { left, .. } => (*left, Some(*left)),
        }
    }
}

impl<T, S> DoubleEndedIterator for Iter<'_, T, S> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            Items::Slice(items) => items.next_back(),
            Items::Chunked { items, left } => {
                let item = items.next_back()?;
                *left -= 1;
                Some(item)
            }
        }
    }
}

impl<T, S> ExactSizeIterator for Iter<'_, T, S> {}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many items, their sum and the largest.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    struct Sum {
        count: u64,
        total: u64,
        max: u64,
    }

    impl Summary for Sum {
        type Item = u64;
        fn of(item: &u64) -> Self {
            Sum {
                count: 1,
                total: *item,
                max: *item,
            }
        }
        fn join(self, other: Self) -> Self {
            Sum {
                count: self.count + other.count,
                total: self.total + other.total,
                max: self.max.max(other.max),
            }
        }
    }

    type V = PVec<u64, Sum>;

    fn check(vec: &V, model: &[u64]) {
        assert_eq!(vec.len(), model.len());
        assert_eq!(vec.is_empty(), model.is_empty());
        assert_eq!(vec.to_vec(), model);
        assert_eq!(
            vec.iter().rev().copied().collect::<Vec<_>>(),
            model.iter().rev().copied().collect::<Vec<_>>()
        );
        for (i, item) in model.iter().enumerate() {
            assert_eq!(vec.get(i), Some(item), "item {i} of {}", model.len());
            assert_eq!(&vec[i], item);
        }
        assert_eq!(vec.get(model.len()), None);
        assert_eq!(vec.last(), model.last());
        assert_eq!(vec.first(), model.first());
        let want = model.iter().fold(Sum::default(), |s, i| s.join(Sum::of(i)));
        assert_eq!(vec.summary(), want, "the summary of {} items", model.len());
        assert_eq!(vec.as_slice().is_some(), model.len() <= LEAF);
        // The chunks are the items in order, every leaf full.
        let chunks: Vec<&[u64]> = vec.chunks().collect();
        assert_eq!(chunks.concat(), *model);
        assert!(chunks[..chunks.len() - 1].iter().all(|c| c.len() == LEAF));
    }

    #[test]
    fn pushes_and_pops_agree_with_a_vec_through_every_level() {
        // Past a root of leaves (1,024) and a root of those (32,768).
        let mut vec = V::new();
        let mut model = Vec::new();
        let mut versions = Vec::new();
        for i in 0..40_000u64 {
            vec = vec.push(i * 7 % 1_000);
            model.push(i * 7 % 1_000);
            if i % 997 == 0 || (31..34).contains(&(i % 1024)) {
                check(&vec, &model);
                versions.push((vec.clone(), model.clone()));
            }
        }
        check(&vec, &model);
        while let Some(shorter) = vec.pop() {
            model.pop();
            vec = shorter;
            if model.len() % 991 == 0 || (31..34).contains(&(model.len() % 1024)) {
                check(&vec, &model);
            }
        }
        assert!(model.is_empty() && vec.is_empty());
        assert!(vec.pop().is_none());
        // Every version kept is as it was: a push or a pop made a new one.
        for (version, model) in &versions {
            check(version, model);
        }
    }

    #[test]
    fn a_vec_built_at_once_is_the_one_pushed() {
        for n in [
            0usize, 1, 31, 32, 33, 64, 65, 1023, 1024, 1025, 1056, 2049, 33_000,
        ] {
            let model: Vec<u64> = (0..n as u64).collect();
            let built = V::from(model.clone());
            check(&built, &model);
            let pushed = model.iter().fold(V::new(), |v, i| v.push(*i));
            assert!(built == pushed, "{n}");
            // And either pops back to the empty vector.
            let mut v = built;
            for _ in 0..n {
                v = v.pop().unwrap();
            }
            assert!(v.is_empty());
        }
    }

    #[test]
    fn a_push_onto_an_old_version_leaves_the_new_one_alone() {
        let base: V = (0..100u64).collect();
        let a = base.push(1);
        let b = base.push(2);
        assert_eq!(a.last(), Some(&1));
        assert_eq!(b.last(), Some(&2));
        assert_eq!(base.len(), 100);
        assert!(a.pop().unwrap() == base && b.pop().unwrap() == base);
        assert!(!a.ptr_eq(&b) && base.ptr_eq(&base.clone()));
    }
}
