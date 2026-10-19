//! A fast hasher for the evaluator's own tables.
//!
//! The evaluator looks a name up on every reference to a definition or a
//! native, and the standard library's SipHash, built to resist an
//! attacker's choice of keys, costs more than the lookups it serves. The
//! keys these tables hold are the names of a program's definitions and of
//! the natives, never the input's data, so there is no attacker's choice
//! to resist. This is the multiply-and-rotate hash rustc uses for its own
//! tables (FxHash), written out here rather than taken as a dependency.

use std::hash::{BuildHasherDefault, Hasher};

/// FxHash over the bytes written, a word at a time.
#[derive(Clone, Copy, Default)]
pub(crate) struct FxHasher {
    hash: u64,
}

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            let mut word = [0u8; 8];
            word.copy_from_slice(chunk);
            self.add(u64::from_le_bytes(word));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut word = [0u8; 8];
            word[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }

    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }

    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }

    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    fn finish(&self) -> u64 {
        self.hash
    }
}

/// The builder a table of names takes.
pub(crate) type FxBuild = BuildHasherDefault<FxHasher>;

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::hash::{BuildHasher, Hash};

    use super::*;

    fn hash(value: impl Hash) -> u64 {
        FxBuild::default().hash_one(value)
    }

    #[test]
    fn equal_names_hash_alike_and_a_table_keyed_by_them_works() {
        assert_eq!(hash("transition"), hash(String::from("transition")));
        assert_ne!(hash("a"), hash("b"));
        // A name longer than a word, and one that ends mid-word.
        assert_ne!(hash("object-start"), hash("object-start!"));
        let mut table: HashMap<&str, usize, FxBuild> = HashMap::default();
        for (i, name) in ["key", "scalar", "array-start", "array-end"]
            .iter()
            .enumerate()
        {
            table.insert(name, i);
        }
        assert_eq!(table.get("scalar"), Some(&1));
        assert_eq!(table.get("missing"), None);
    }
}
