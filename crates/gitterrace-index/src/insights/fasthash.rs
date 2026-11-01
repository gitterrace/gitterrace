//! A multiply-rotate hasher for the short path and id keys of the statistics passes. The std
//! default (SipHash) resists collision attacks, which these maps never face: their keys come
//! from the repository's own history and none outlives one computation.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

#[derive(Default, Clone, Copy)]
pub(crate) struct FastHasher(u64);

impl FastHasher {
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FastHasher {
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            let mut w = [0u8; 8];
            w.copy_from_slice(c);
            self.add(u64::from_le_bytes(w));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut w = [0u8; 8];
            w[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(w));
        }
    }

    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }

    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

pub(crate) type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;
pub(crate) type FastSet<K> = HashSet<K, BuildHasherDefault<FastHasher>>;

#[cfg(test)]
mod tests {
    use super::{FastMap, FastSet};

    #[test]
    fn keys_of_every_length_stay_distinct() {
        let mut m: FastMap<String, usize> = FastMap::default();
        for n in 0..40 {
            m.insert("a".repeat(n), n);
        }
        assert_eq!(m.len(), 40);
        assert_eq!(m["aaaaaaaaa"], 9);
        let s: FastSet<&str> = ["x/y", "x/z", "x"].into_iter().collect();
        assert_eq!(s.len(), 3);
    }
}
