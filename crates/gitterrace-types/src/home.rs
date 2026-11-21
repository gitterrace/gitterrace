//! The home folder, read one way for every crate.

use std::path::{Path, PathBuf};

/// The home folder variable: `USERPROFILE` on Windows, `HOME` elsewhere.
pub fn home_var() -> Option<std::ffi::OsString> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
}

/// The home folder, `None` when its variable is unset.
pub fn home_dir() -> Option<PathBuf> {
    home_var().map(PathBuf::from)
}

/// `~`, `~/x` (and `~\x` on Windows) against `home`; anything else as written.
pub fn expand_tilde(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if cfg!(windows) && p.starts_with("~\\") => home.join(&p[2..]),
        None if p == "~" => home.to_path_buf(),
        None => PathBuf::from(p),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::expand_tilde;

    #[test]
    fn expand_tilde_resolves_only_a_leading_tilde_under_a_home() {
        let home = Path::new("/h/me");
        assert_eq!(expand_tilde("~/x/y", home), Path::new("/h/me/x/y"));
        assert_eq!(expand_tilde("~", home), home);
        assert_eq!(expand_tilde("/a/b", home), Path::new("/a/b"));
        assert_eq!(expand_tilde("~x", home), Path::new("~x"));
    }
}
