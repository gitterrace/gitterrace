//! The output folder: where it really is, what in it an export owns, and whether an export may
//! write there at all. Nothing here changes the disk; [`crate::export::swap`] does that.
//!
//! Ownership is recorded, not guessed: the marker file lists what an export put there. A
//! folder an export created whole is owned whole and replaced whole. A folder that already
//! held other files is owned only entry by entry, so later exports replace their own entries
//! and never touch anything else. A marker without that record (or one that cannot be read)
//! grants nothing.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

/// Written into every export folder; it records what the export there owns.
pub(super) const MARKER: &str = ".gitterrace-export";
/// Largest marker read. Not configurable: a real one lists the site's top-level names, a few
/// hundred bytes, and a larger file is cut off and so grants nothing.
const MARKER_MAX: u64 = 1 << 20;
const OWNS_ALL: &str = "owns *";
/// Marker line prefix naming one top-level entry the export put in a shared folder.
const OWNS: &str = "owns ";

/// The output folder as it really is: its parent resolved through every symlink, its own
/// name kept as given, so a link at the name itself is seen rather than followed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub(super) parent: PathBuf,
    pub(super) name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Owned {
    /// The folder is an earlier export's alone: the new site replaces it whole.
    All,
    /// These top-level entries of a shared folder; everything else stays.
    Entries(BTreeSet<String>),
}

impl Target {
    /// Resolves `out`. Its name must be a plain name, and the parent must resolve: the
    /// nearest existing ancestor is canonicalized and the missing rest appended, so a path that
    /// does not exist yet is still compared as the folder it will become.
    pub(super) fn resolve(out: &Path) -> anyhow::Result<Self> {
        let abs = std::path::absolute(out)
            .with_context(|| format!("resolving the output folder {}", out.display()))?;
        let name = match abs.file_name().and_then(|n| n.to_str()) {
            Some(n) if n != "." && n != ".." => n.to_owned(),
            _ => bail!(
                "{} does not end in a plain folder name. Name the output folder itself",
                out.display()
            ),
        };
        let parent = abs.parent().map_or_else(|| abs.clone(), Path::to_path_buf);
        Ok(Self { parent: real(&parent)?, name })
    }

    pub(super) fn path(&self) -> PathBuf {
        self.parent.join(&self.name)
    }
}

/// Checks the output before anything is written and says what the export owns there. Refused:
/// a symlink as the output, an output that is or holds a served repository (or, when it would
/// be replaced whole, the working folder), and a non-empty folder no export owns, unless
/// `force`. `None` means nothing is there yet.
pub(super) fn check_out(
    repos: &[PathBuf],
    target: &Target,
    force: bool,
) -> anyhow::Result<Option<Owned>> {
    let out = target.path();
    if holds_any(&out, repos)? {
        bail!(
            "{} holds a repository the export serves. Choose an output folder outside it",
            out.display()
        );
    }
    let meta = match std::fs::symlink_metadata(&out) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", out.display())),
    };
    if meta.file_type().is_symlink() {
        bail!("{} is a symlink. Name the folder it points to, or another folder", out.display());
    }
    if !meta.is_dir() {
        bail!("{} is not a folder", out.display());
    }
    if let Some(owned) = read_marker(&out)? {
        let cleared: Vec<PathBuf> = match &owned {
            Owned::All => vec![out.clone()],
            Owned::Entries(names) => names.iter().map(|n| out.join(n)).collect(),
        };
        if cleared.iter().any(|p| cwd_inside(p)) {
            bail!(
                "{} holds the current folder. It cannot be cleared for a fresh export",
                out.display()
            );
        }
        return Ok(Some(owned));
    }
    let empty = std::fs::read_dir(&out)
        .with_context(|| format!("reading {}", out.display()))?
        .next()
        .is_none();
    if !empty && !force {
        if std::fs::symlink_metadata(out.join(MARKER)).is_ok() {
            // An older version's marker records no ownership, so what else the folder holds
            // is unknown and none of it is cleared.
            bail!(
                "{} has a {MARKER} file that does not say what the export owns there, as an \
                 older version wrote it. Remove the folder and export again, or choose another",
                out.display()
            );
        }
        bail!(
            "{} is not empty and was not made by an export (no {MARKER} file); \
             choose another folder or pass --force to write into it",
            out.display()
        );
    }
    Ok(Some(Owned::Entries(BTreeSet::new())))
}

pub(super) fn marker(repos: &[PathBuf], owned: &Owned) -> String {
    let names: Vec<String> = repos
        .iter()
        .map(|r| {
            r.file_name()
                .map_or_else(|| r.display().to_string(), |n| n.to_string_lossy().into_owned())
        })
        .collect();
    let mut s = format!("gitterrace {} {}\n", env!("GITTERRACE_VERSION"), names.join(","));
    match owned {
        Owned::All => s.push_str(OWNS_ALL),
        Owned::Entries(names) => {
            let lines: Vec<String> = names.iter().map(|n| format!("{OWNS}{n}")).collect();
            s.push_str(&lines.join("\n"));
        }
    }
    s.push('\n');
    s
}

/// What the marker in `dir` says the export owns. `None` for no marker, one that is not a
/// regular file, or one without an ownership record (written before ownership was recorded,
/// so its folder may hold anything): such a folder is treated as foreign.
pub(super) fn read_marker(dir: &Path) -> anyhow::Result<Option<Owned>> {
    let path = dir.join(MARKER);
    match std::fs::symlink_metadata(&path) {
        Ok(m) if m.is_file() && m.len() <= MARKER_MAX => {}
        Ok(_) => return Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    }
    let mut text = String::new();
    std::fs::File::open(&path)
        .and_then(|f| f.take(MARKER_MAX).read_to_string(&mut text))
        .with_context(|| format!("reading {}", path.display()))?;
    Ok(parse_marker(&text))
}

fn parse_marker(text: &str) -> Option<Owned> {
    let mut lines = text.lines();
    if !lines.next()?.starts_with("gitterrace ") {
        return None;
    }
    let mut names = BTreeSet::new();
    let mut any = false;
    for line in lines {
        if line == OWNS_ALL {
            return Some(Owned::All);
        }
        let name = line.strip_prefix(OWNS)?;
        if !plain_name(name) {
            return None;
        }
        names.insert(name.to_owned());
        any = true;
    }
    any.then_some(Owned::Entries(names))
}

/// A name a site's top level may hold: one component, never the marker itself.
pub(super) fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name != MARKER
        && !name.contains(['/', '\\', ':', '\0'])
}

/// Whether `out`, resolved, is or contains any served repository: its folder or its git dir.
/// A repository whose location cannot be resolved fails the check rather than passing it.
fn holds_any(out: &Path, repos: &[PathBuf]) -> anyhow::Result<bool> {
    let out = real(out)?;
    for repo in repos {
        let mut places = vec![
            repo.canonicalize()
                .with_context(|| format!("resolving the repository {}", repo.display()))?,
        ];
        if let Ok(r) = gitterrace_git::Repo::discover(repo) {
            places.push(r.root());
            places.push(real(r.git_dir())?);
        }
        if places.iter().any(|p| p.starts_with(&out)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether the working folder is `path` or inside it. Unknown counts as inside.
fn cwd_inside(path: &Path) -> bool {
    let Ok(path) = real(path) else {
        return true;
    };
    std::env::current_dir()
        .ok()
        .and_then(|d| d.canonicalize().ok())
        .is_some_and(|d| d.starts_with(path))
}

/// `path` with its nearest existing ancestor canonicalized and the missing rest appended.
/// Only plain names may be missing: a `..` past a link would resolve differently later.
fn real(path: &Path) -> anyhow::Result<PathBuf> {
    let abs = std::path::absolute(path).with_context(|| format!("resolving {}", path.display()))?;
    let mut rest = Vec::new();
    let mut base = abs.as_path();
    loop {
        match base.canonicalize() {
            Ok(mut found) => {
                for part in rest.iter().rev() {
                    found.push(part);
                }
                return Ok(found);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let (Some(parent), Some(name)) = (base.parent(), base.file_name()) else {
                    bail!("cannot resolve {}", path.display());
                };
                rest.push(name.to_owned());
                base = parent;
            }
            Err(e) => return Err(e).with_context(|| format!("resolving {}", path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Owned, Target, marker, parse_marker};

    /// Ownership survives a round trip through the marker, and a marker without an ownership
    /// record, with a bad name or from something else grants nothing. Red when a bare marker
    /// makes a folder owned.
    #[test]
    fn only_a_recorded_ownership_marker_grants_ownership() {
        let names: BTreeSet<String> = ["index.html", "r"].map(String::from).into();
        for owned in [Owned::All, Owned::Entries(names)] {
            assert_eq!(parse_marker(&marker(&[], &owned)), Some(owned));
        }
        for text in [
            "gitterrace 0.1 repo\n",
            "gitterrace 0.1 repo\nowns ../x\n",
            "gitterrace 0.1 repo\nowns .gitterrace-export\n",
            "gitterrace 0.1 repo\nsomething\n",
            "other\nowns *\n",
            "",
        ] {
            assert_eq!(parse_marker(text), None, "{text:?}");
        }
    }

    /// A missing output resolves through its existing ancestors, and a name that is not one
    /// is refused.
    #[test]
    fn an_output_that_does_not_exist_yet_resolves_through_its_ancestors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().canonicalize().expect("real");
        let t = Target::resolve(&dir.path().join("a/b/site")).expect("resolve");
        assert_eq!(t.parent, real.join("a/b"));
        assert_eq!(t.name, "site");
        // On Windows `std::path::absolute` folds `a/..` away, so only Unix sees the name `..`.
        if cfg!(not(windows)) {
            assert!(Target::resolve(&dir.path().join("a/..")).is_err());
        }
    }
}
