//! The links an export has queued, kept as fixed-size fingerprints rather than strings.
//!
//! A linux-sized export queues millions of paths, each about a hundred bytes as a string. A
//! 128-bit fingerprint (two FNV-1a 64 passes, forward and backward, folded with the length)
//! is 16 bytes, and two distinct paths sharing one is far below one in 10^20 for any export.

use std::collections::HashSet;

#[derive(Default)]
pub(crate) struct Seen(HashSet<u128>);

impl Seen {
    /// A set holding every path of `paths`.
    pub(crate) fn from_iter<'a>(paths: impl IntoIterator<Item = &'a String>) -> Self {
        Self(paths.into_iter().map(|p| fingerprint(p)).collect())
    }

    /// Records `path`, true when it was new.
    pub(crate) fn insert(&mut self, path: &str) -> bool {
        self.0.insert(fingerprint(path))
    }
}

fn fingerprint(path: &str) -> u128 {
    let b = path.as_bytes();
    let fwd = gitterrace_git::fnv1a64(b.iter().copied());
    let len = (b.len() as u64).to_le_bytes();
    let back = gitterrace_git::fnv1a64(b.iter().rev().copied().chain(len));
    (u128::from(fwd) << 64) | u128::from(back)
}

#[cfg(test)]
mod tests {
    use super::Seen;

    #[test]
    fn a_path_is_new_once_and_near_paths_stay_apart() {
        let mut s = Seen::default();
        assert!(s.insert("/commit/abc"));
        assert!(!s.insert("/commit/abc"));
        for p in ["/commit/abd", "/commit/ab", "/commit/abc/", "commit/abc", ""] {
            assert!(s.insert(p), "{p:?} collided");
        }
        let n = 200_000;
        let fresh = (0..n).filter(|i| s.insert(&format!("/blob/HEAD/d/{i}.rs"))).count();
        assert_eq!(fresh, n);
    }
}
