//! A cache bounded by the bytes its entries hold, not by their count: one file list of a large
//! tree weighs as much as thousands of small ones, and a count cap lets a visitor who asks for
//! many distinct commits pin all of them.

use std::collections::{HashMap, VecDeque};

/// Approximate heap bytes a cached value keeps alive.
pub(crate) trait Weight {
    fn weight(&self) -> usize;
}

impl Weight for String {
    fn weight(&self) -> usize {
        self.len()
    }
}

impl<T: Weight> Weight for Vec<T> {
    fn weight(&self) -> usize {
        self.iter()
            .map(|t| t.weight() + std::mem::size_of::<T>())
            .sum()
    }
}

impl<T: Weight> Weight for std::sync::Arc<T> {
    fn weight(&self) -> usize {
        (**self).weight()
    }
}

impl<A: Weight, B: Weight> Weight for (A, B) {
    fn weight(&self) -> usize {
        self.0.weight() + self.1.weight()
    }
}

impl Weight for std::time::Instant {
    fn weight(&self) -> usize {
        0
    }
}

impl Weight for gitterrace_git::LastCommit {
    fn weight(&self) -> usize {
        self.name.len() + self.id.len() + self.subject.len()
    }
}

/// Entries by key, evicted oldest first once their weight passes `budget`. A miss is a rebuild,
/// so evicting a few entries early costs only time, never correctness.
pub(crate) struct Bounded<V> {
    budget: usize,
    used: usize,
    map: HashMap<String, (V, usize)>,
    order: VecDeque<String>,
}

impl<V: Clone + Weight> Bounded<V> {
    pub(crate) fn new(budget: usize) -> Self {
        Self {
            budget,
            used: 0,
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub(crate) fn get(&self, key: &str) -> Option<V> {
        self.map.get(key).map(|(v, _)| v.clone())
    }

    /// Stores `v`, evicting the oldest entries to make room. A value larger than an eighth of
    /// the budget is not kept: it would flush most of the others for one entry.
    pub(crate) fn put(&mut self, key: String, v: V) {
        let w = v.weight() + key.len();
        self.remove(&key);
        if w > self.budget / 8 {
            return;
        }
        while self.used + w > self.budget {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some((_, ow)) = self.map.remove(&old) {
                self.used -= ow;
            }
        }
        self.used += w;
        self.order.push_back(key.clone());
        self.map.insert(key, (v, w));
    }

    fn remove(&mut self, key: &str) {
        if let Some((_, w)) = self.map.remove(key) {
            self.used -= w;
            self.order.retain(|k| k != key);
        }
    }

    /// Moves the budget, evicting the oldest entries when it shrank.
    pub(crate) fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
        while self.used > self.budget {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some((_, ow)) = self.map.remove(&old) {
                self.used -= ow;
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.used = 0;
    }

    #[cfg(test)]
    pub(crate) const fn used(&self) -> usize {
        self.used
    }
}

#[cfg(test)]
mod tests {
    use super::Bounded;

    #[test]
    fn evicts_oldest_first_by_bytes_not_count() {
        // Entries of 10 bytes in a budget of 80: eight fit, the ninth evicts the oldest.
        let mut c: Bounded<String> = Bounded::new(80);
        for k in ["a", "b", "c", "d", "e", "f", "g", "h"] {
            c.put(k.into(), "x".repeat(9));
        }
        assert!(c.get("a").is_some() && c.get("h").is_some());
        c.put("i".into(), "x".repeat(9));
        assert!(c.get("a").is_none(), "the oldest entry goes first");
        assert!(c.get("b").is_some() && c.get("i").is_some());
        assert!(c.used() <= 80);
    }

    #[test]
    fn an_entry_over_an_eighth_of_the_budget_is_not_kept_and_flushes_nothing() {
        let mut c: Bounded<String> = Bounded::new(80);
        c.put("a".into(), "x".repeat(9));
        c.put("big".into(), "x".repeat(11));
        assert!(c.get("big").is_none());
        assert!(c.get("a").is_some());
    }

    #[test]
    fn a_smaller_budget_evicts_down_to_it() {
        let mut c: Bounded<String> = Bounded::new(80);
        for k in ["a", "b", "c", "d"] {
            c.put(k.into(), "x".repeat(9));
        }
        c.set_budget(20);
        assert!(c.used() <= 20);
        assert!(c.get("a").is_none() && c.get("d").is_some());
    }

    #[test]
    fn replacing_a_key_does_not_count_it_twice() {
        let mut c: Bounded<String> = Bounded::new(80);
        for _ in 0..10 {
            c.put("a".into(), "x".repeat(9));
        }
        assert_eq!(c.used(), 10);
        c.clear();
        assert_eq!(c.used(), 0);
    }
}
