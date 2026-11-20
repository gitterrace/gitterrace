//! The one memory budget of a repository's serving caches, `resources.cache_mb`, and every
//! cache's share of it. A cache that keeps bytes between requests takes its bound from
//! [`Config::cache_bytes`]; the shares sum to the whole budget, so the setting bounds all of
//! them together and no cache can sit outside it.

use crate::Config;

/// The parts `resources.cache_mb` is cut into.
pub const PARTS: usize = 16;

/// A serving cache with a share of `resources.cache_mb`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cache {
    /// Line blame per (commit, path), in the git handle.
    Blame,
    /// Highlighted code per file.
    Code,
    /// Rendered pages, the one cache the idle release keeps.
    Pages,
    /// File lists per tree.
    FileLists,
    /// Last commit per directory entry.
    LastCommits,
    /// `git log -- path` ids.
    PathLogs,
    /// Compare results per (base id, head id) read from git.
    Compares,
    /// Blocks of the index segment files, read by every insights page. Its source goes on
    /// the idle release, so it never counts against an idle server.
    Blocks,
}

/// Every cache and its parts of [`PARTS`]. Together they are the whole budget.
pub const SHARES: [(Cache, usize); 8] = [
    (Cache::Blame, 3),
    (Cache::Code, 3),
    (Cache::Pages, 2),
    (Cache::FileLists, 1),
    (Cache::LastCommits, 1),
    (Cache::PathLogs, 1),
    (Cache::Compares, 1),
    (Cache::Blocks, 4),
];

impl Cache {
    /// The cache's name in the configuration page.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Blame => "blame",
            Self::Code => "highlighted code",
            Self::Pages => "rendered pages",
            Self::FileLists => "file lists",
            Self::LastCommits => "last commits",
            Self::PathLogs => "path logs",
            Self::Compares => "compares",
            Self::Blocks => "index blocks",
        }
    }
}

/// The shares as one sentence for the configuration page, in [`SHARES`] order.
#[must_use]
pub fn shares_doc() -> String {
    let list: Vec<String> = SHARES.iter().map(|(c, p)| format!("{} {p}", c.label())).collect();
    format!("Parts of {PARTS}: {}.", list.join(", "))
}

impl Config {
    /// Bytes `cache` may keep: its share of `resources.cache_mb`.
    #[must_use]
    pub fn cache_bytes(&self, cache: Cache) -> usize {
        let parts = SHARES.iter().find(|(c, _)| *c == cache).map_or(0, |(_, p)| *p);
        (self.resources.cache_mb << 20) / PARTS * parts
    }
}

#[cfg(test)]
mod tests {
    use super::{Cache, Config, PARTS, SHARES};

    /// The shares are the whole budget: one more cache must take its bytes from another.
    #[test]
    fn shares_sum_to_the_budget() {
        assert_eq!(SHARES.iter().map(|(_, p)| p).sum::<usize>(), PARTS);
        let cfg = Config::default();
        let all: usize = SHARES.iter().map(|(c, _)| cfg.cache_bytes(*c)).sum();
        assert_eq!(all, cfg.resources.cache_mb << 20);
    }

    /// The configuration page names every cache with its parts, so a new share shows there.
    #[test]
    fn shares_doc_lists_every_share() {
        let doc = super::shares_doc();
        for (c, p) in SHARES {
            assert!(doc.contains(&format!("{} {p}", c.label())), "{c:?} missing");
        }
    }

    /// Every cache is named once, so none has two bounds.
    #[test]
    fn every_cache_has_one_share() {
        for (i, (c, _)) in SHARES.iter().enumerate() {
            assert!(SHARES[i + 1..].iter().all(|(d, _)| d != c), "{c:?} twice");
        }
        assert_eq!(Config::default().cache_bytes(Cache::Blocks), 8 << 20);
    }
}
