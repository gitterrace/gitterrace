//! The shared kit lives at `vendor/schwarzwald/kit`, synced whole and never edited here. The
//! product's own config and styles stay in `schwarzwald/`. No kit is at `schwarzwald/kit`, so
//! no tracked file outside the kit may name that path, written out or joined from segments.

#[cfg(test)]
mod tests {
    use gitterrace_testkit::git;

    /// Where the kit lives now.
    const KIT: &str = "vendor/schwarzwald/kit";

    /// Files that may quote the old path: the kit itself (it accepts both layouts), recorded
    /// output that names paths as data, this guard, whose cases plant the old path, and
    /// `_typos.toml`, which must carry the kit's typos base word for word (drift checks it), so
    /// it keeps the base's `schwarzwald/kit/**` exclude for products on the old layout.
    const SKIP: [&str; 5] = [
        "vendor/schwarzwald/kit/",
        "tests/bench/results/",
        "CHANGELOG.md",
        "xtask/src/kit_path.rs",
        "_typos.toml",
    ];

    /// Every `(file, line)` whose text names `schwarzwald/kit` other than as part of `KIT`.
    /// A path built from segments (`"schwarzwald" / "kit"`, `("schwarzwald", "kit")`) counts
    /// too: quotes, spaces and the joiners between segments are read as one slash.
    fn old_kit_refs<'a>(files: &[(&'a str, &'a str)]) -> Vec<(&'a str, usize)> {
        let mut out = Vec::new();
        for (path, text) in files {
            if SKIP.iter().any(|s| path.starts_with(s) || path == s) {
                continue;
            }
            for (i, line) in text.lines().enumerate() {
                let line = joined(line);
                let stale = line
                    .match_indices("schwarzwald/kit")
                    .any(|(at, _)| !line[..at].ends_with("vendor/"));
                if stale {
                    out.push((*path, i + 1));
                }
            }
        }
        out
    }

    /// `line` with every run of quotes, spaces, commas and slashes between two path segments
    /// read as one `/`, so a joined path reads like a written one.
    fn joined(line: &str) -> String {
        let mut out = String::with_capacity(line.len());
        let mut sep = false;
        for c in line.chars() {
            if matches!(c, '"' | '\'' | ' ' | ',' | '/') {
                sep = true;
            } else {
                if sep && !out.is_empty() {
                    out.push('/');
                }
                sep = false;
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn a_planted_old_kit_path_is_reported() {
        let files = [
            ("Makefile", "\tbash schwarzwald/kit/lint/run.sh\n"),
            (
                "a.py",
                "LINT = ROOT / \"schwarzwald\" / \"kit\" / \"lint\"\n",
            ),
            (
                "b.py",
                "LINT = ROOT / \"vendor\" / \"schwarzwald\" / \"kit\"\n",
            ),
            (
                "ruff.toml",
                "extend = \"vendor/schwarzwald/kit/configs/ruff-base.toml\"\n",
            ),
            ("vendor/schwarzwald/kit/hooks/x", "schwarzwald/kit\n"),
            ("CHANGELOG.md", "moved schwarzwald/kit\n"),
        ];
        assert_eq!(old_kit_refs(&files), [("Makefile", 1), ("a.py", 1)]);
    }

    #[test]
    fn no_tracked_file_names_the_kit_at_its_old_root() {
        let root = crate::repo_root();
        let tracked = git(&root, &["ls-files"]);
        let texts: Vec<(&str, String)> = tracked
            .lines()
            .filter_map(|p| std::fs::read_to_string(root.join(p)).ok().map(|t| (p, t)))
            .collect();
        let files: Vec<(&str, &str)> = texts.iter().map(|(p, t)| (*p, t.as_str())).collect();
        let bad = old_kit_refs(&files);
        assert!(
            bad.is_empty(),
            "name the kit at {KIT}: {}",
            bad.iter()
                .map(|(p, l)| format!("{p}:{l}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}
