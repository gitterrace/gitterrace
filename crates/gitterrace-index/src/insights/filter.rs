//! Which paths count: user excludes, generated files and ignore rules.

use std::sync::Mutex;

use gitterrace_git::IgnoreSet;

use super::Options;
use crate::insights::fasthash::FastMap;

/// Lock files and minified output: generated, not written.
const GENERATED: [&str; 10] = [
    "*.lock",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "npm-shrinkwrap.json",
    "go.sum",
    "*.min.js",
    "*.min.css",
    "*.map",
    "*.snap",
];

/// True for lock files and minified output. The one definition of "generated": the path
/// filter, the web pages and the API hotspots all use it.
pub fn is_generated(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    GENERATED
        .iter()
        .any(|p| fnmatch(p, path) || fnmatch(p, name))
}

/// Decides per path whether statistics leave it out.
pub struct PathFilter {
    patterns: Vec<String>,
    ignore: Option<IgnoreSet>,
    memo: Mutex<FastMap<String, bool>>,
    /// Answers by the address of a record's shared path. Records built by the loader share one
    /// `Arc<str>` per distinct path, so the per-file hot loops hash a pointer, not the string.
    /// The records outlive the filter, so an address never names a different path.
    by_addr: Mutex<FastMap<usize, bool>>,
}

impl PathFilter {
    pub fn new(opts: &Options, ignore: Option<IgnoreSet>) -> Self {
        let mut patterns = opts.exclude.clone();
        if !opts.include_generated {
            patterns.extend(GENERATED.iter().map(|s| (*s).to_owned()));
        }
        Self {
            patterns,
            ignore: if opts.respect_ignore { ignore } else { None },
            memo: Mutex::new(FastMap::default()),
            by_addr: Mutex::new(FastMap::default()),
        }
    }

    /// [`PathFilter::skips`] for a path shared between records, memoized by address.
    pub(crate) fn skips_shared(&self, path: &std::sync::Arc<str>) -> bool {
        let key = std::sync::Arc::as_ptr(path).cast::<u8>() as usize;
        if let Ok(m) = self.by_addr.lock()
            && let Some(hit) = m.get(&key)
        {
            return *hit;
        }
        let skip = self.skips(path);
        if let Ok(mut m) = self.by_addr.lock() {
            m.insert(key, skip);
        }
        skip
    }

    /// True when `path` is excluded, generated or ignored.
    pub fn skips(&self, path: &str) -> bool {
        if let Ok(memo) = self.memo.lock()
            && let Some(hit) = memo.get(path)
        {
            return *hit;
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        let skip = self
            .patterns
            .iter()
            .any(|p| fnmatch(p, path) || fnmatch(p, name))
            || self.ignore.as_ref().is_some_and(|i| i.is_ignored(path));
        if let Ok(mut memo) = self.memo.lock() {
            memo.insert(path.to_owned(), skip);
        }
        skip
    }
}

/// Shell-style match where `*` also crosses `/`, as Python's `fnmatch`: `*`, `?`, `[a-z]`, `[!x]`.
pub(crate) fn fnmatch(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.as_bytes(), text.as_bytes());
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        let step = if pi < p.len() {
            match p[pi] {
                b'*' => {
                    star = Some((pi, ti));
                    pi += 1;
                    continue;
                }
                b'?' => Some(1),
                b'[' => class(&p[pi..], t[ti]),
                c => (c == t[ti]).then_some(1),
            }
        } else {
            None
        };
        match step {
            Some(1) if p[pi] != b'[' => {
                pi += 1;
                ti += 1;
            }
            Some(len) => {
                pi += len;
                ti += 1;
            }
            None => match star {
                Some((sp, st)) => {
                    pi = sp + 1;
                    ti = st + 1;
                    star = Some((sp, st + 1));
                }
                None => return false,
            },
        }
    }
    p[pi..].iter().all(|c| *c == b'*')
}

/// Length of the `[...]` class at the start of `p` when it matches `c`.
fn class(p: &[u8], c: u8) -> Option<usize> {
    let close = p.iter().skip(2).position(|b| *b == b']')? + 2;
    let body = &p[1..close];
    let (neg, body) = match body.split_first() {
        Some((b'!', rest)) => (true, rest),
        _ => (false, body),
    };
    let mut hit = false;
    let mut i = 0;
    while i < body.len() {
        if i + 2 < body.len() && body[i + 1] == b'-' {
            hit |= (body[i]..=body[i + 2]).contains(&c);
            i += 3;
        } else {
            hit |= body[i] == c;
            i += 1;
        }
    }
    (hit != neg).then_some(close + 1)
}

#[cfg(test)]
mod tests {
    use super::{PathFilter, fnmatch};
    use crate::Options;

    #[test]
    fn fnmatch_matches_like_python() {
        assert!(fnmatch("*.lock", "Cargo.lock"));
        assert!(fnmatch("*.min.js", "a/b/app.min.js"));
        assert!(fnmatch("src/*", "src/a/b.rs"));
        assert!(fnmatch("a?c", "abc"));
        assert!(fnmatch("[a-c]x", "bx"));
        assert!(!fnmatch("[!a-c]x", "bx"));
        assert!(!fnmatch("*.lock", "lockfile"));
        assert!(fnmatch("*", ""));
    }

    #[test]
    fn generated_excluded_unless_included() {
        let mut o = Options::default();
        let f = PathFilter::new(&o, None);
        assert!(f.skips("Cargo.lock") && f.skips("web/pnpm-lock.yaml") && !f.skips("src/a.rs"));
        o.include_generated = true;
        o.exclude = vec!["vendor/*".into()];
        let f = PathFilter::new(&o, None);
        assert!(!f.skips("Cargo.lock") && f.skips("vendor/x/y.go"));
    }
}
