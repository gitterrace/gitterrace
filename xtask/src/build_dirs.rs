//! Build output is never tracked: a tracked copy goes stale, and the gate then checks the copy
//! instead of what a fresh build makes. Every dir a build writes is listed in `BUILD_DIRS`.

#[cfg(test)]
mod tests {
    use gitterrace_testkit::{git, try_git};

    /// The dirs a build writes, relative to the repo root: cargo's, and the site's.
    const BUILD_DIRS: [&str; 2] = ["target", crate::site::SITE_DIR];

    /// The tracked paths under any of `dirs`.
    fn tracked_under<'a>(tracked: &'a str, dirs: &[&str]) -> Vec<&'a str> {
        tracked.lines().filter(|p| dirs.iter().any(|d| p.starts_with(&format!("{d}/")))).collect()
    }

    #[test]
    fn a_planted_tracked_build_file_is_reported() {
        let tracked = "src/a.rs\ntarget/debug/x\nsite-src/b\nout/c.html\n";
        assert_eq!(tracked_under(tracked, &["target", "out"]), ["target/debug/x", "out/c.html"]);
    }

    #[test]
    fn no_tracked_file_lives_in_a_build_dir_and_each_is_ignored() {
        let root = crate::repo_root();
        let tracked = git(&root, &["ls-files"]);
        let bad = tracked_under(&tracked, &BUILD_DIRS);
        assert!(bad.is_empty(), "tracked build output: {}", bad.join(", "));
        for dir in BUILD_DIRS {
            let probe = format!("{dir}/x");
            let ignored = try_git(&root, &["check-ignore", "-q", "--no-index", &probe]).is_ok();
            assert!(ignored, "{dir}/ is a build dir but .gitignore does not cover it");
        }
    }

    /// The host serves what the build writes, not a tracked copy of it.
    #[test]
    fn wrangler_serves_the_site_build_dir() {
        let toml = std::fs::read_to_string(crate::repo_root().join("wrangler.toml"))
            .expect("wrangler.toml");
        let want = format!("directory = \"./{}\"", crate::site::SITE_DIR);
        assert!(toml.lines().any(|l| l.trim() == want), "wrangler.toml lacks {want}");
    }
}
