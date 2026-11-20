//! Resolving argv[0] to an absolute executable at load time.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};

pub(super) fn expand_tilde(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if cfg!(windows) && p.starts_with("~\\") => home.join(&p[2..]),
        None if p == "~" => home.to_path_buf(),
        None => PathBuf::from(p),
    }
}

/// argv[0] as an absolute executable: tilde-expanded, or looked up in `PATH` when it has no
/// slash. A relative path with a slash is refused: it would resolve against the cwd.
pub(super) fn resolve_program(prog: &str, home: &Path) -> Option<String> {
    let exec = |p: &Path| is_executable(p);
    let found = if prog.contains('/') || prog.contains(MAIN_SEPARATOR) || prog.starts_with('~') {
        Some(expand_tilde(prog, home)).filter(|p| p.is_absolute() && exec(p))
    } else {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .filter(|d| d.is_absolute())
            .map(|d| d.join(prog))
            .find(|p| exec(p))
    };
    found.map(|p| p.to_string_lossy().into_owned())
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Windows decides by extension, as `CreateProcess` and `cmd` do.
#[cfg(windows)]
fn is_executable(p: &Path) -> bool {
    let ext_ok = |p: &Path| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| ["exe", "cmd", "bat", "com"].iter().any(|x| e.eq_ignore_ascii_case(x)))
    };
    std::fs::metadata(p).is_ok_and(|m| m.is_file()) && ext_ok(p)
}
