//! The output folder: checked before an export, built beside it and put in place only when the
//! whole site is written, so a failed or capped export leaves the old site as it was.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use crate::export::write::write_leaf;

/// Written into every export folder; its presence says an export made the folder.
const MARKER: &str = ".gitterrace-export";

/// What is at `out` before an export, which decides how the finished site replaces it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Existing {
    /// Nothing, or an earlier export: the new site replaces it whole.
    Replace,
    /// An empty folder, or a foreign one written into under `--force`: the site's entries are
    /// moved in, beside whatever is there.
    Merge,
}

/// Where the site is built, beside the folder it will replace. Dropped without
/// [`Staging::finish`], it is removed, so a failed export leaves no half-built folder either.
pub(super) struct Staging {
    dir: PathBuf,
    done: bool,
}

impl Staging {
    /// A fresh folder beside `out`, on the same file system so the finished site moves into
    /// place by rename. Staging folders that killed runs left for this target go first.
    pub(super) fn new(out: &Path) -> anyhow::Result<Self> {
        let parent = match out.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        std::fs::create_dir_all(&parent)
            .with_context(|| format!("creating {}", parent.display()))?;
        let name = out
            .file_name()
            .map_or_else(|| "export".into(), |n| n.to_string_lossy().into_owned());
        remove_stale(&parent, &name, alive)?;
        let dir = parent.join(format!("{}{}", partial_prefix(&name), std::process::id()));
        if std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
            std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        }
        std::fs::create_dir(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Self { dir, done: false })
    }

    pub(super) fn path(&self) -> &Path {
        &self.dir
    }

    /// Writes the marker into the finished site and puts it at `out`. A replaced folder is
    /// swapped by rename, so `out` holds the old site or the new one, never a mix; a forced
    /// foreign folder gets the site's entries moved in beside its own files.
    pub(super) fn finish(
        mut self,
        repos: &[PathBuf],
        out: &Path,
        existing: Existing,
    ) -> anyhow::Result<()> {
        write_leaf(&self.dir.join(MARKER), marker_line(repos).as_bytes())?;
        match existing {
            Existing::Replace => {
                let old = old_path(&self.dir);
                let had = std::fs::symlink_metadata(out).is_ok();
                if had {
                    std::fs::rename(out, &old)
                        .with_context(|| format!("moving {} aside", out.display()))?;
                }
                std::fs::rename(&self.dir, out)
                    .with_context(|| format!("moving the site to {}", out.display()))?;
                self.done = true;
                if had {
                    std::fs::remove_dir_all(&old)
                        .with_context(|| format!("removing the old site {}", old.display()))?;
                }
            }
            Existing::Merge => {
                let entries = std::fs::read_dir(&self.dir)
                    .with_context(|| format!("reading {}", self.dir.display()))?;
                for entry in entries {
                    let from = entry?.path();
                    let Some(name) = from.file_name() else {
                        continue;
                    };
                    let to = out.join(name);
                    if std::fs::symlink_metadata(&to).is_ok() {
                        remove_entry(&to)?;
                    }
                    std::fs::rename(&from, &to)
                        .with_context(|| format!("moving {}", to.display()))?;
                }
            }
        }
        Ok(())
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.done && std::fs::symlink_metadata(&self.dir).is_ok() {
            // Best effort: the error that stopped the export is the one worth reporting.
            if std::fs::remove_dir_all(&self.dir).is_err() {}
        }
    }
}

/// Checks `out` before anything is written: an earlier export (with the marker) is replaced,
/// so no stale page survives; an empty folder is filled in place; a non-empty folder without
/// the marker is refused unless `force`. Nothing is changed here.
pub(super) fn check_out(repos: &[PathBuf], out: &Path, force: bool) -> anyhow::Result<Existing> {
    if std::fs::symlink_metadata(out).is_err() {
        return Ok(Existing::Replace);
    }
    let marked = out.join(MARKER).is_file();
    let empty = std::fs::read_dir(out)
        .with_context(|| format!("reading {}", out.display()))?
        .next()
        .is_none();
    if !marked && !empty && !force {
        bail!(
            "{} is not empty and was not made by an export (no {MARKER} file); \
             choose another folder or pass --force to write into it",
            out.display()
        );
    }
    if marked {
        // Replacing deletes, so it must never reach a served repository or the working folder.
        let root = out.canonicalize().context("resolving the output folder")?;
        let cwd = std::env::current_dir()
            .ok()
            .and_then(|d| d.canonicalize().ok());
        let inside = repos
            .iter()
            .filter_map(|r| r.canonicalize().ok())
            .chain(cwd)
            .any(|p| p.starts_with(&root));
        if inside {
            bail!(
                "{} holds a repository or the current folder. It cannot be cleared for a fresh export",
                out.display()
            );
        }
        return Ok(Existing::Replace);
    }
    Ok(Existing::Merge)
}

/// Name start of the staging folders for the target `name`; the process id follows.
fn partial_prefix(name: &str) -> String {
    format!(".{name}.partial-")
}

/// Where the replaced site waits during the swap: the staging name plus `.old`, so a crash
/// between the two renames leaves a folder `remove_stale` recognizes. (`with_extension` would
/// swap the process id for `old` and lose the name it matches on.)
fn old_path(staging: &Path) -> PathBuf {
    let mut name = staging.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

/// Removes the staging folders a crashed export left beside `parent/name`: only real folders
/// named for this target with a process id that `alive` says is gone, plus the `.old` copy a
/// crash during the swap leaves. Anything else, a live run's folder included, stays.
fn remove_stale(parent: &Path, name: &str, alive: fn(u32) -> bool) -> anyhow::Result<()> {
    let prefix = partial_prefix(name);
    let entries =
        std::fs::read_dir(parent).with_context(|| format!("reading {}", parent.display()))?;
    for entry in entries {
        let entry = entry?;
        let file = entry.file_name();
        let Some(rest) = file.to_str().and_then(|f| f.strip_prefix(&prefix)) else {
            continue;
        };
        let pid = rest.strip_suffix(".old").unwrap_or(rest);
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if pid.to_string() != rest.trim_end_matches(".old") || alive(pid) {
            continue;
        }
        let path = entry.path();
        // A symlink by that name is not ours to follow or remove.
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("removing the stale {}", path.display()))?;
        }
    }
    Ok(())
}

/// Whether process `pid` still runs. Our own counts as alive: its folder is in use.
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    let Some(p) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return false;
    };
    // EPERM means it exists under another user, which is alive for our purpose.
    !matches!(
        rustix::process::test_kill_process(p),
        Err(rustix::io::Errno::SRCH)
    )
}

/// Without a cheap check every other folder is taken as in use and left alone.
#[cfg(not(unix))]
fn alive(pid: u32) -> bool {
    true
}

fn marker_line(repos: &[PathBuf]) -> String {
    let names: Vec<String> = repos
        .iter()
        .map(|r| {
            r.file_name().map_or_else(
                || r.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            )
        })
        .collect();
    format!(
        "gitterrace {} {}\n",
        env!("GITTERRACE_VERSION"),
        names.join(",")
    )
}

/// Removes one entry. A symlink is removed as a link, never followed.
fn remove_entry(path: &Path) -> anyhow::Result<()> {
    let kind = std::fs::symlink_metadata(path)?.file_type();
    let done = if kind.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    done.with_context(|| format!("removing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// Guard class: an export that deletes a folder it did not make. Beside the target, only
    /// staging folders of this target whose process is gone are removed: a live run's folder,
    /// another target's, an unparsable pid, a plain file and a symlink by a stale name all
    /// stay. Red when the cleanup matches loosely or ignores the pid.
    #[test]
    fn only_dead_staging_folders_of_this_target_are_removed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        let mk = |n: &str| std::fs::create_dir(p.join(n)).expect("mkdir");
        for n in [
            ".site.partial-111",
            ".site.partial-111.old",
            ".site.partial-222",
        ] {
            mk(n);
            std::fs::write(p.join(n).join("page"), "x").expect("write");
        }
        for n in [
            ".other.partial-111",
            ".site.partial-11x",
            ".site.partial-0111",
            "site",
        ] {
            mk(n);
        }
        std::fs::write(p.join(".site.partial-333"), "a file").expect("write");
        let outside = p.join("outside");
        mk("outside");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, p.join(".site.partial-444")).expect("symlink");
        super::remove_stale(p, "site", |pid| pid == 222).expect("cleanup");
        let mut left: Vec<String> = std::fs::read_dir(p)
            .expect("read")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        let mut want = vec![
            ".other.partial-111",
            ".site.partial-0111",
            ".site.partial-11x",
            ".site.partial-222",
            ".site.partial-333",
            "outside",
            "site",
        ];
        if cfg!(unix) {
            want.push(".site.partial-444");
            want.sort_unstable();
        }
        assert_eq!(left, want);
        assert!(Path::new(&p.join(".site.partial-222/page")).is_file());
    }

    /// Crash recovery of the swap: the old site set aside under the name `finish` uses is
    /// removed by the next export's cleanup once its process is gone, and not before.
    #[test]
    fn a_site_left_aside_by_a_crashed_swap_is_cleaned_up() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        let staging = p.join(format!("{}111", super::partial_prefix("site")));
        let old = super::old_path(&staging);
        assert_eq!(
            old.file_name().and_then(|n| n.to_str()),
            Some(".site.partial-111.old")
        );
        std::fs::create_dir(&old).expect("mkdir");
        std::fs::write(old.join("index.html"), "old site").expect("write");
        super::remove_stale(p, "site", |_| true).expect("cleanup while alive");
        assert!(old.is_dir(), "a running export's old site was removed");
        super::remove_stale(p, "site", |_| false).expect("cleanup after the crash");
        assert!(!old.exists(), "the old site of a crashed swap stayed");
    }

    #[cfg(unix)]
    #[test]
    fn this_process_is_alive_and_a_reaped_child_is_not() {
        assert!(super::alive(std::process::id()));
        let mut child = std::process::Command::new("true").spawn().expect("spawn");
        let pid = child.id();
        child.wait().expect("wait");
        assert!(!super::alive(pid), "reaped child {pid} still alive");
    }
}
