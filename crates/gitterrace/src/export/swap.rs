//! Putting a finished site in place, and undoing what a crash left half done.
//!
//! The site is built in a staging folder beside the output, on the same file system, then
//! moved in by rename. Whatever it replaces is first moved aside to `.<name>.partial-<pid>.old`
//! and removed only once the new site is in place: a failed rename moves it back, and after a
//! crash the next export ([`recover`]) puts it back if it is the only site left. Every move
//! goes through a handle on the real folder, so a path swapped for a symlink after the checks
//! is not followed.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use crate::export::out::{MARKER, Owned, Target, marker, plain_name, read_marker};
use crate::export::write::write_leaf;

/// A real folder, held open on unix so later moves cannot be redirected through a link.
struct Dir {
    path: PathBuf,
    #[cfg(unix)]
    fd: rustix::fd::OwnedFd,
}

/// What a name in a [`Dir`] is, never following a link.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Folder,
    Other,
}

/// Where the site is built. Dropped without [`Staging::publish`] it is removed, so a failed
/// export leaves no half-built folder.
pub(super) struct Staging {
    dir: Dir,
    target: Target,
    published: bool,
}

impl Dir {
    fn open(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use rustix::fs::{Mode, OFlags};
            let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            let fd = rustix::fs::open(path, flags, Mode::empty())?;
            Ok(Self { path: path.to_path_buf(), fd })
        }
        #[cfg(not(unix))]
        {
            let m = std::fs::symlink_metadata(path)?;
            if !m.is_dir() {
                return Err(io::Error::other("not a real folder"));
            }
            Ok(Self { path: path.to_path_buf() })
        }
    }

    fn kind(&self, name: &str) -> io::Result<Option<Kind>> {
        #[cfg(unix)]
        let found = rustix::fs::statat(&self.fd, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
            .map(|s| {
                rustix::fs::FileType::from_raw_mode(s.st_mode) == rustix::fs::FileType::Directory
            })
            .map_err(io::Error::from);
        #[cfg(not(unix))]
        let found = std::fs::symlink_metadata(self.path.join(name)).map(|m| m.is_dir());
        match found {
            Ok(true) => Ok(Some(Kind::Folder)),
            Ok(false) => Ok(Some(Kind::Other)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Moves `name` to `to/to_name`. With `fresh`, an entry already at the destination fails
    /// the move where the platform can tell, rather than being replaced.
    fn rename(&self, name: &str, to: &Self, to_name: &str, fresh: bool) -> anyhow::Result<()> {
        let done = rename_at(self, name, to, to_name, fresh);
        done.with_context(|| {
            format!(
                "moving {} to {}",
                self.path.join(name).display(),
                to.path.join(to_name).display()
            )
        })
    }

    /// Removes the entry `name`: a folder with all it holds, anything else as itself. A
    /// symlink is removed as a link, never followed.
    fn remove(&self, name: &str) -> anyhow::Result<()> {
        let path = self.path.join(name);
        let done = match self.kind(name)? {
            None => return Ok(()),
            Some(Kind::Folder) => std::fs::remove_dir_all(&path),
            Some(Kind::Other) => std::fs::remove_file(&path),
        };
        done.with_context(|| format!("removing {}", path.display()))
    }

    /// The names in this folder, the marker left out. A name that is not plain UTF-8 fails:
    /// a site never writes one.
    fn names(&self) -> anyhow::Result<BTreeSet<String>> {
        let mut out = BTreeSet::new();
        for entry in std::fs::read_dir(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?
        {
            let file = entry?.file_name();
            let Some(name) = file.to_str() else {
                bail!("{} holds a name that is not UTF-8", self.path.display());
            };
            if name != MARKER {
                out.insert(name.to_owned());
            }
        }
        Ok(out)
    }
}

#[cfg(all(unix, any(target_vendor = "apple", target_os = "linux", target_os = "android")))]
fn rename_at(from: &Dir, name: &str, to: &Dir, to_name: &str, fresh: bool) -> io::Result<()> {
    let flags =
        if fresh { rustix::fs::RenameFlags::NOREPLACE } else { rustix::fs::RenameFlags::empty() };
    Ok(rustix::fs::renameat_with(&from.fd, name, &to.fd, to_name, flags)?)
}

#[cfg(all(unix, not(any(target_vendor = "apple", target_os = "linux", target_os = "android"))))]
fn rename_at(from: &Dir, name: &str, to: &Dir, to_name: &str, fresh: bool) -> io::Result<()> {
    if fresh && to.kind(to_name)?.is_some() {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    Ok(rustix::fs::renameat(&from.fd, name, &to.fd, to_name)?)
}

#[cfg(not(unix))]
fn rename_at(from: &Dir, name: &str, to: &Dir, to_name: &str, fresh: bool) -> io::Result<()> {
    if fresh && to.kind(to_name)?.is_some() {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    std::fs::rename(from.path.join(name), to.path.join(to_name))
}

impl Staging {
    /// A fresh staging folder beside the output.
    pub(super) fn new(target: &Target) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&target.parent)
            .with_context(|| format!("creating {}", target.parent.display()))?;
        let parent = Dir::open(&target.parent)
            .with_context(|| format!("opening {}", target.parent.display()))?;
        let name = format!("{}{}", partial_prefix(&target.name), std::process::id());
        parent.remove(&name)?;
        let path = target.parent.join(&name);
        std::fs::create_dir(&path).with_context(|| format!("creating {}", path.display()))?;
        let dir = Dir::open(&path).with_context(|| format!("opening {}", path.display()))?;
        Ok(Self { dir, target: target.clone(), published: false })
    }

    pub(super) fn path(&self) -> &Path {
        &self.dir.path
    }

    /// Puts the finished site at the output. `owned` is what [`check_out`] found the export
    /// owns there: nothing yet or all of it, and the site replaces the folder whole; some
    /// entries of a shared folder, and only those are replaced. In a shared folder a name the
    /// export does not own is replaced only for a file under `force`, and a folder of someone
    /// else's never: the export fails first, with nothing changed.
    ///
    /// [`check_out`]: crate::export::out::check_out
    pub(super) fn publish(
        mut self,
        repos: &[PathBuf],
        owned: Option<Owned>,
        force: bool,
    ) -> anyhow::Result<()> {
        let parent = Dir::open(&self.target.parent)
            .with_context(|| format!("opening {}", self.target.parent.display()))?;
        // Pages are staged unsynced (`write_staged`): one flush here makes them durable
        // before any rename can expose them.
        #[cfg(unix)]
        rustix::fs::sync();
        match owned {
            None | Some(Owned::All) => {
                write_leaf(&self.dir.path.join(MARKER), marker(repos, &Owned::All).as_bytes())?;
                self.replace(&parent)?;
                self.published = true;
            }
            // The staging folder is left empty and goes on drop.
            Some(Owned::Entries(prev)) => self.merge(&parent, repos, &prev, force)?,
        }
        Ok(())
    }

    /// The whole-folder swap. A failed second rename puts the old site back.
    fn replace(&self, parent: &Dir) -> anyhow::Result<()> {
        let name = &self.target.name;
        let aside = aside_name(name);
        let had = parent.kind(name)?.is_some();
        if had {
            parent.rename(name, parent, &aside, true)?;
        }
        if let Err(e) = parent.rename(&staging_name(name), parent, name, true) {
            if had {
                parent.rename(&aside, parent, name, true).context("putting the old site back")?;
            }
            return Err(e);
        }
        if had {
            parent.remove(&aside).context("removing the replaced site")?;
        }
        Ok(())
    }

    /// Entry by entry into a shared folder: the export's own entries and the new ones go
    /// aside, the new ones come in, the marker records exactly what the export now owns.
    fn merge(
        &self,
        parent: &Dir,
        repos: &[PathBuf],
        prev: &BTreeSet<String>,
        force: bool,
    ) -> anyhow::Result<()> {
        let out = Dir::open(&self.target.path())
            .with_context(|| format!("opening {}", self.target.path().display()))?;
        let names = self.dir.names()?;
        for n in &names {
            if !plain_name(n) {
                bail!("the site holds the entry {n:?}, which is not a plain name");
            }
            if prev.contains(n) {
                continue;
            }
            match out.kind(n)? {
                Some(Kind::Folder) => bail!(
                    "{} is a folder not made by an export. It is left alone: move it away or \
                     choose another output folder",
                    out.path.join(n).display()
                ),
                Some(Kind::Other) if !force => bail!(
                    "{} was not made by an export; pass --force to replace it",
                    out.path.join(n).display()
                ),
                // Free, or under --force a file of someone else's, which it replaces.
                None | Some(Kind::Other) => {}
            }
        }
        if out.kind(MARKER)? == Some(Kind::Folder) {
            bail!("{} is a folder", out.path.join(MARKER).display());
        }
        let claim: BTreeSet<String> = prev.union(&names).cloned().collect();
        // Claimed before anything moves, so a crash part way never leaves an entry of ours
        // looking like someone else's: at worst the next export owns a name it then clears.
        let aside_name = aside_name(&self.target.name);
        parent.remove(&aside_name)?;
        let aside_path = self.target.parent.join(&aside_name);
        std::fs::create_dir(&aside_path)
            .with_context(|| format!("creating {}", aside_path.display()))?;
        let aside = Dir::open(&aside_path)?;
        let record = marker(repos, &Owned::Entries(claim.clone()));
        write_leaf(&aside.path.join(MARKER), record.as_bytes())?;
        write_leaf(&out.path.join(MARKER), record.as_bytes())?;
        let mut moved = Vec::new();
        let mut placed = Vec::new();
        let done = (|| {
            for n in &claim {
                if out.kind(n)?.is_some() {
                    out.rename(n, &aside, n, true)?;
                    moved.push(n);
                }
            }
            for n in &names {
                self.dir.rename(n, &out, n, true)?;
                placed.push(n);
            }
            anyhow::Ok(())
        })();
        if let Err(e) = done {
            for n in placed {
                out.rename(n, &self.dir, n, true).context("taking back a new entry")?;
            }
            for n in moved {
                aside.rename(n, &out, n, true).context("putting an old entry back")?;
            }
            write_leaf(
                &out.path.join(MARKER),
                marker(repos, &Owned::Entries(prev.clone())).as_bytes(),
            )?;
            parent.remove(&aside_name)?;
            return Err(e);
        }
        write_leaf(&out.path.join(MARKER), marker(repos, &Owned::Entries(names)).as_bytes())?;
        parent.remove(&aside_name).context("removing the replaced entries")
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.published {
            // Best effort: the error that stopped the export is the one worth reporting.
            if std::fs::remove_dir_all(&self.dir.path).is_err() {}
        }
    }
}

/// Undoes what crashed exports to `target` left beside it. A replaced site set aside is put
/// back when nothing is at the output, and removed only when a whole new site is there; the
/// entries set aside from a shared folder go back where nothing has taken their name. A
/// staging folder holds only pages being built and is removed. Only real folders named for
/// this target whose process is gone are touched, and an aside folder only with a marker
/// proving the export made it. Runs before anything else looks at the output.
pub(super) fn recover(target: &Target) -> anyhow::Result<()> {
    recover_with(target, alive)
}

fn recover_with(target: &Target, alive: fn(u32) -> bool) -> anyhow::Result<()> {
    let Ok(parent) = Dir::open(&target.parent) else {
        return Ok(());
    };
    let prefix = partial_prefix(&target.name);
    let mut found: Vec<(String, bool)> = Vec::new();
    for name in parent.names()? {
        let Some(rest) = name.strip_prefix(&prefix) else {
            continue;
        };
        let (pid, old) = rest.strip_suffix(".old").map_or((rest, false), |p| (p, true));
        let Ok(n) = pid.parse::<u32>() else {
            continue;
        };
        if n.to_string() == pid && !alive(n) && parent.kind(&name)? == Some(Kind::Folder) {
            found.push((name, old));
        }
    }
    for (name, old) in found {
        if old {
            put_back(&parent, &name, target)?;
        } else {
            parent.remove(&name)?;
        }
    }
    Ok(())
}

/// One aside folder `name` of a crashed export, put back or removed as [`recover`] says.
fn put_back(parent: &Dir, name: &str, target: &Target) -> anyhow::Result<()> {
    let path = target.parent.join(name);
    let Some(owned) = read_marker(&path)? else {
        return Ok(());
    };
    let out = &target.name;
    match owned {
        Owned::All => match parent.kind(out)? {
            None => parent.rename(name, parent, out, true),
            Some(Kind::Folder) if read_marker(&target.path())? == Some(Owned::All) => {
                parent.remove(name)
            }
            Some(_) => Ok(()),
        },
        Owned::Entries(_) => {
            let Ok(dest) = Dir::open(&target.path()) else {
                return Ok(());
            };
            let aside = Dir::open(&path)?;
            for n in aside.names()? {
                if dest.kind(&n)?.is_none() {
                    aside.rename(&n, &dest, &n, true)?;
                }
            }
            parent.remove(name)
        }
    }
}

/// Name start of the staging folders for the target `name`; the process id follows.
fn partial_prefix(name: &str) -> String {
    format!(".{name}.partial-")
}

fn staging_name(name: &str) -> String {
    format!("{}{}", partial_prefix(name), std::process::id())
}

/// Where what the site replaces waits until the new site is in place.
fn aside_name(name: &str) -> String {
    format!("{}.old", staging_name(name))
}

/// Whether process `pid` still runs. Our own counts as alive: its folder is in use.
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    let Some(p) = i32::try_from(pid).ok().and_then(rustix::process::Pid::from_raw) else {
        return false;
    };
    // EPERM means it exists under another user, which is alive for our purpose.
    !matches!(rustix::process::test_kill_process(p), Err(rustix::io::Errno::SRCH))
}

/// Without a cheap check every other folder is taken as in use and left alone.
#[cfg(not(unix))]
const fn alive(_pid: u32) -> bool {
    true
}

#[cfg(test)]
#[path = "swap_tests.rs"]
mod tests;
