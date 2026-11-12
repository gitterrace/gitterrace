//! Length and syntax bounds on the `rev` and `path` a caller names, checked before any git read.

use gitterrace_git::REJECTED_REV;

use crate::error::ApiError;

/// Longest `rev` or `path` accepted, in bytes. Real ones are far shorter. Not configurable:
/// a longer name is never a real one.
const MAX_PATH_LEN: usize = 1_024;

/// `rev` unless it is too long or uses syntax in [`REJECTED_REV`].
pub(crate) fn checked(rev: &str) -> Result<&str, ApiError> {
    if rev.len() > MAX_PATH_LEN {
        return Err(too_long("revision"));
    }
    if REJECTED_REV.iter().any(|p| rev.contains(p)) {
        return Err(ApiError::BadRequest(format!(
            "unsupported revision syntax in {rev:?}: use a branch, tag, sha or HEAD~n"
        )));
    }
    Ok(rev)
}

/// `path` unless it is longer than [`MAX_PATH_LEN`].
pub(crate) fn checked_path(path: &str) -> Result<&str, ApiError> {
    if path.len() > MAX_PATH_LEN {
        return Err(too_long("path"));
    }
    Ok(path)
}

fn too_long(what: &str) -> ApiError {
    ApiError::BadRequest(format!("{what} is longer than {MAX_PATH_LEN} bytes"))
}

#[cfg(test)]
mod tests {
    use gitterrace_testkit::git;

    use super::*;
    use crate::ApiState;

    fn long() -> String {
        "a".repeat(MAX_PATH_LEN + 1)
    }

    fn state() -> (tempfile::TempDir, ApiState) {
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("a.txt"), "one\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "first"]);
        let s = ApiState::open(p).expect("open");
        (dir, s)
    }

    #[test]
    fn checked_rejects_rev_exceeding_max_len() {
        assert!(matches!(checked(&long()), Err(ApiError::BadRequest(_))));
        assert!(checked(&"a".repeat(MAX_PATH_LEN)).is_ok());
    }

    #[test]
    fn blob_rejects_path_exceeding_max_len() {
        let (_d, s) = state();
        let r = s.blob(None, &long(), None, None);
        assert!(matches!(r, Err(ApiError::BadRequest(_))));
    }

    #[test]
    fn blame_rejects_path_exceeding_max_len() {
        let (_d, s) = state();
        assert!(matches!(
            s.blame(None, &long()),
            Err(ApiError::BadRequest(_))
        ));
    }
}
