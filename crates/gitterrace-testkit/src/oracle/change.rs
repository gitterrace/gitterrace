//! The re-bless proof: the same cell set rendered by an old and a new commit, compared per cell.
//!
//! This answers "did the output change, and how", the question a re-bless must settle. The
//! completeness proof in `proof.rs` answers another one (does the pairwise set lose cells of
//! the full matrix at one commit) and cannot see a change between commits at all.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use crate::oracle::{canonical, normalize, without_chrome};

/// Per-cell classes of one old vs new comparison.
#[derive(Debug, Default)]
pub struct Change {
    /// Keys present on both sides, and of those the byte-equal ones.
    pub shared: usize,
    pub same: usize,
    /// Changed only by equal-time row order.
    pub ties: Vec<String>,
    /// Changed only inside the chrome (site header, overview sidebar), ties folded.
    pub chrome: Vec<String>,
    /// Changed anywhere else: key and the first differing line, old then new.
    pub real: Vec<(String, String, String)>,
    /// Keys on one side only.
    pub only_old: Vec<String>,
    pub only_new: Vec<String>,
}

impl Change {
    /// Classifies every key of two `key -> body` maps.
    #[must_use]
    pub fn of(old: &BTreeMap<String, String>, new: &BTreeMap<String, String>) -> Self {
        let mut c = Self::default();
        let mut keys: BTreeSet<&String> = old.keys().collect();
        keys.extend(new.keys());
        for k in keys {
            match (old.get(k), new.get(k)) {
                (Some(a), Some(b)) => c.add(k, a, b),
                (Some(_), None) => c.only_old.push(k.clone()),
                _ => c.only_new.push(k.clone()),
            }
        }
        c
    }

    /// Classifies one key present on both sides.
    pub fn add(&mut self, key: &str, old: &str, new: &str) {
        self.shared += 1;
        let (was, now) = (normalize(old), normalize(new));
        if was == now {
            self.same += 1;
        } else if canonical(&was) == canonical(&now) {
            self.ties.push(key.to_owned());
        } else {
            let (body_was, body_now) =
                (canonical(&without_chrome(key, &was)), canonical(&without_chrome(key, &now)));
            if body_was == body_now {
                self.chrome.push(key.to_owned());
            } else {
                let (left, right) = first_diff(&body_was, &body_now);
                self.real.push((key.to_owned(), left, right));
            }
        }
    }

    /// Real changes plus keys that appeared or vanished: what a re-bless must explain.
    #[must_use]
    pub const fn unexplained(&self) -> usize {
        self.real.len() + self.only_old.len() + self.only_new.len()
    }

    /// The report: one counts line, then every changed key under its class.
    #[must_use]
    pub fn render(&self, head: &str) -> String {
        let mut t = String::new();
        let mut line = |s: String| writeln!(t, "{s}").expect("write string");
        line(head.to_owned());
        line(format!(
            "change old->new: cells {}, unchanged {}, chrome-only {}, tie-only {}, REAL {}, \
             only-old {}, only-new {}",
            self.shared,
            self.same,
            self.chrome.len(),
            self.ties.len(),
            self.real.len(),
            self.only_old.len(),
            self.only_new.len()
        ));
        for (k, a, b) in &self.real {
            line(format!("  real {k}\n    - {a}\n    + {b}"));
        }
        for k in &self.only_old {
            line(format!("  only-old {k}"));
        }
        for k in &self.only_new {
            line(format!("  only-new {k}"));
        }
        for k in &self.chrome {
            line(format!("  chrome {k}"));
        }
        for k in &self.ties {
            line(format!("  tie {k}"));
        }
        t
    }
}

/// Reads a `TEST_ORACLE_DUMP` folder: one file per answer, the key on its first line.
#[must_use]
pub fn read_dump(dir: &Path) -> BTreeMap<String, String> {
    std::fs::read_dir(dir)
        .expect("dump dir")
        .filter_map(Result::ok)
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| t.split_once('\n').map(|(k, b)| (k.to_owned(), b.to_owned())))
        .collect()
}

/// The first line where two bodies differ, each cut to 160 characters.
fn first_diff(a: &str, b: &str) -> (String, String) {
    let cut = |s: Option<&str>| -> String {
        s.map_or_else(|| "(end)".to_owned(), |s| s.chars().take(160).collect())
    };
    let (mut lhs, mut rhs) = (a.lines(), b.lines());
    loop {
        match (lhs.next(), rhs.next()) {
            (None, None) => return ("(same lines)".into(), "(same lines)".into()),
            (left, right) if left != right => return (cut(left), cut(right)),
            _ => {}
        }
    }
}
