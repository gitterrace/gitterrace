//! The per-commit proof that a smaller golden cell set loses nothing: every key the small set
//! shares with the full matrix must give the same body, up to chrome or equal-time order.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::oracle::{canonical, digest, golden_line, without_chrome};

/// Counts and classified differences of one proof run.
#[derive(Debug, Default)]
pub struct Proof {
    /// Answers in the full product run.
    pub full: usize,
    /// Answers in the golden cell set run.
    pub small: usize,
    /// Small-set keys the full run also has, and of those the byte-equal ones.
    pub shared: usize,
    pub equal: usize,
    /// Small-set keys outside the full product (the extra fixtures, such as the renamed repo).
    pub only_small: usize,
    pub chrome: Vec<String>,
    pub ties: Vec<String>,
    pub real: Vec<String>,
    /// The full run against an older full golden, when one was given.
    pub golden: Option<(usize, usize, Vec<String>)>,
}

impl Proof {
    /// Classifies one shared key: `full` and `small` are the two bodies.
    pub fn add(&mut self, key: &str, full: &str, small: &str) {
        self.shared += 1;
        if full == small {
            self.equal += 1;
        } else if canonical(full) == canonical(small) {
            self.ties.push(key.to_owned());
        } else if canonical(&without_chrome(key, full)) == canonical(&without_chrome(key, small)) {
            self.chrome.push(key.to_owned());
        } else {
            self.real.push(key.to_owned());
        }
    }

    /// Compares the full run to an older full golden (`exact canonical key` lines): matched
    /// exactly, matched up to ties, and the rest. A missing key counts as the rest.
    pub fn against_golden(&mut self, golden: &str, full: &BTreeMap<String, String>) {
        let (mut exact, mut tie, mut rest) = (0, 0, Vec::new());
        for (k, (e, c)) in golden
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(golden_line)
        {
            match full.get(k) {
                Some(v) if digest(v) == e => exact += 1,
                Some(v) if digest(&canonical(v)) == c => tie += 1,
                _ => rest.push(k.to_owned()),
            }
        }
        self.golden = Some((exact, tie, rest));
    }

    /// The proof file: counts first, then every non-equal key under its class.
    #[must_use]
    pub fn render(&self, head: &str) -> String {
        let mut t = String::new();
        let mut line = |s: String| writeln!(t, "{s}").expect("write string");
        line(head.to_owned());
        line(format!(
            "full answers {}, golden-set answers {}",
            self.full, self.small
        ));
        line(format!(
            "shared {}, equal {}, chrome-only {}, tie-only {}, real {}, golden-set only {}",
            self.shared,
            self.equal,
            self.chrome.len(),
            self.ties.len(),
            self.real.len(),
            self.only_small
        ));
        if let Some((exact, tie, rest)) = &self.golden {
            line(format!(
                "full run vs old full golden: exact {exact}, tie-only {tie}, other {}",
                rest.len()
            ));
            for k in rest.iter().take(50) {
                line(format!("  other {k}"));
            }
        }
        for (name, keys) in [
            ("chrome", &self.chrome),
            ("tie", &self.ties),
            ("real", &self.real),
        ] {
            for k in keys {
                line(format!("  {name} {k}"));
            }
        }
        t
    }
}
