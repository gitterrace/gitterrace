use std::cmp::Reverse;
use std::collections::HashMap;
use std::path::Path;

use gix::bstr::ByteSlice;
use gix::revision::walk::Sorting;
use gix::traverse::commit::simple::CommitTimeOrder;

use crate::error::git;
use crate::ident;
use crate::{
    BlameLine, Blob, CommitDetail, CommitSummary, Error, RefInfo, RefKind, Result, TreeEntry,
    TreeEntryKind,
};

/// Decoded-object cache; trees and commits are re-read constantly by walks and diffs.
const OBJECT_CACHE_BYTES: usize = 128 * 1024 * 1024;
/// Same probe window as git for binary detection.
const BINARY_PROBE: usize = 8000;

/// A repository opened read-only.
pub struct Repo {
    inner: gix::ThreadSafeRepository,
}

impl Repo {
    /// Opens the repository containing `path` (discovers upward).
    pub fn discover(path: &Path) -> Result<Self> {
        let inner = gix::ThreadSafeRepository::discover(path)
            .map_err(|e| crate::Error::NotARepo(e.to_string()))?;
        Ok(Self { inner })
    }

    /// Path of the `.git` directory, where gitterrace keeps its cache.
    pub fn git_dir(&self) -> &Path {
        self.inner.git_dir()
    }

    /// Display name: the worktree folder name.
    pub fn name(&self) -> String {
        let root = self
            .inner
            .work_dir()
            .unwrap_or_else(|| self.inner.git_dir());
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let dir = if root.file_name().is_some_and(|n| n == ".git") {
            root.parent().map(Path::to_path_buf).unwrap_or(root)
        } else {
            root
        };
        dir.file_name().map_or_else(
            || "repository".to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    /// Resolves a revision (branch, tag, sha, `HEAD`) to a full commit id.
    pub fn resolve(&self, rev: &str) -> Result<String> {
        Ok(Self::resolve_id(&self.repo(), rev)?.to_string())
    }

    /// Commits reachable from `rev`, newest first, optionally limited to `path`.
    pub fn log(
        &self,
        rev: &str,
        path: Option<&str>,
        skip: usize,
        limit: usize,
    ) -> Result<Vec<CommitSummary>> {
        let repo = self.repo();
        let tip = Self::resolve_id(&repo, rev)?;
        let path = path.map(|p| p.trim_matches('/')).filter(|p| !p.is_empty());
        let walk = repo
            .rev_walk([tip])
            .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
            .all()
            .map_err(git)?;
        let mailmap = ident::mailmap(&repo, tip);
        let mut out = Vec::new();
        let mut seen = 0;
        for info in walk {
            if out.len() >= limit {
                break;
            }
            let info = info.map_err(git)?;
            let commit = info.object().map_err(git)?;
            if let Some(path) = path
                && !touches_path(&repo, &commit, path)?
            {
                continue;
            }
            if seen < skip {
                seen += 1;
                continue;
            }
            out.push(summary(&commit, &mailmap)?);
        }
        Ok(out)
    }

    /// Commit ids reachable from `rev`, oldest first. Used by the indexer.
    pub fn rev_list(&self, rev: &str) -> Result<Vec<String>> {
        let repo = self.repo();
        let tip = Self::resolve_id(&repo, rev)?;
        let walk = repo
            .rev_walk([tip])
            .sorting(Sorting::ByCommitTime(CommitTimeOrder::NewestFirst))
            .all()
            .map_err(git)?;
        let mut ids = Vec::new();
        for info in walk {
            ids.push(info.map_err(git)?.id.to_string());
        }
        ids.reverse();
        Ok(ids)
    }

    /// One commit with its diff against the first parent.
    pub fn commit(&self, id: &str) -> Result<CommitDetail> {
        let repo = self.repo();
        let oid = Self::resolve_id(&repo, id)?;
        let commit = repo.find_commit(oid).map_err(git)?;
        let mailmap = ident::mailmap(&repo, oid);
        let message = commit
            .message_raw()
            .map_err(git)?
            .to_str_lossy()
            .into_owned();
        let body = message
            .split_once('\n')
            .map(|(_, rest)| rest.trim().to_string())
            .unwrap_or_default();
        Ok(CommitDetail {
            summary: summary(&commit, &mailmap)?,
            body,
            files: crate::diff::changes(&repo, &commit, true)?,
        })
    }

    /// Per-file line counts against the first parent, without hunk text.
    /// The indexer calls this once per commit, so it must stay cheap.
    pub fn numstat(&self, id: &str) -> Result<Vec<crate::FileDiff>> {
        let repo = self.repo();
        let oid = Self::resolve_id(&repo, id)?;
        let commit = repo.find_commit(oid).map_err(git)?;
        crate::diff::changes(&repo, &commit, false)
    }

    /// [`Repo::numstat`] for many commits, spread over all cores. Results are in the order of
    /// `ids`. Prefer this to a loop over `numstat`: the work is CPU bound and independent.
    pub fn numstat_many(&self, ids: &[String]) -> Result<Vec<Vec<crate::FileDiff>>> {
        let workers = std::thread::available_parallelism().map_or(1, usize::from);
        let per = ids.len().div_ceil(workers).max(1);
        std::thread::scope(|scope| {
            let handles: Vec<_> = ids
                .chunks(per)
                .map(|chunk| {
                    scope.spawn(move || {
                        let repo = self.repo();
                        chunk
                            .iter()
                            .map(|id| {
                                let oid = Self::resolve_id(&repo, id)?;
                                let commit = repo.find_commit(oid).map_err(git)?;
                                crate::diff::changes(&repo, &commit, false)
                            })
                            .collect::<Result<Vec<_>>>()
                    })
                })
                .collect();
            let mut out = Vec::with_capacity(ids.len());
            for handle in handles {
                let part = handle
                    .join()
                    .map_err(|_| Error::Git("numstat worker panicked".to_string()))??;
                out.extend(part);
            }
            Ok(out)
        })
    }

    /// Entries of the directory at `path` in `rev`, dirs first then by name.
    pub fn tree(&self, rev: &str, path: &str) -> Result<Vec<TreeEntry>> {
        let repo = self.repo();
        let tip = Self::resolve_id(&repo, rev)?;
        let root = repo.find_commit(tip).map_err(git)?.tree().map_err(git)?;
        let path = path.trim_matches('/');
        let tree = if path.is_empty() {
            root
        } else {
            let entry = root
                .lookup_entry_by_path(path)
                .map_err(git)?
                .ok_or_else(|| Error::PathNotFound(path.to_string()))?;
            if !entry.mode().is_tree() {
                return Err(Error::PathNotFound(path.to_string()));
            }
            entry.object().map_err(git)?.into_tree()
        };
        let mut out = Vec::new();
        for entry in tree.iter() {
            let entry = entry.map_err(git)?;
            let mode = entry.mode();
            let kind = if mode.is_tree() {
                TreeEntryKind::Dir
            } else if mode.is_commit() {
                TreeEntryKind::Submodule
            } else if mode.is_link() {
                TreeEntryKind::Symlink
            } else {
                TreeEntryKind::File
            };
            let size = match kind {
                TreeEntryKind::File | TreeEntryKind::Symlink => {
                    repo.find_header(entry.oid()).ok().map(|h| h.size())
                }
                _ => None,
            };
            out.push(TreeEntry {
                name: entry.filename().to_str_lossy().into_owned(),
                kind,
                size,
            });
        }
        out.sort_by(|a, b| {
            let dir = |e: &TreeEntry| e.kind != TreeEntryKind::Dir;
            dir(a).cmp(&dir(b)).then_with(|| a.name.cmp(&b.name))
        });
        Ok(out)
    }

    pub fn blob(&self, rev: &str, path: &str) -> Result<Blob> {
        let repo = self.repo();
        let tip = Self::resolve_id(&repo, rev)?;
        let path = path.trim_matches('/');
        let entry = repo
            .find_commit(tip)
            .map_err(git)?
            .tree()
            .map_err(git)?
            .lookup_entry_by_path(path)
            .map_err(git)?
            .filter(|e| !e.mode().is_tree() && !e.mode().is_commit())
            .ok_or_else(|| Error::PathNotFound(path.to_string()))?;
        let data = entry.object().map_err(git)?.detach().data;
        let binary = data[..data.len().min(BINARY_PROBE)].contains(&0);
        let text = if binary {
            None
        } else {
            String::from_utf8(data.clone()).ok()
        };
        Ok(Blob {
            path: path.to_string(),
            size: data.len() as u64,
            text,
        })
    }

    pub fn blame(&self, rev: &str, path: &str) -> Result<Vec<BlameLine>> {
        let repo = self.repo();
        let tip = Self::resolve_id(&repo, rev)?;
        let path = path.trim_matches('/');
        let content = self.blob(rev, path)?;
        let Some(text) = content.text else {
            return Ok(Vec::new());
        };
        let outcome = repo
            .blame_file(
                path.into(),
                tip,
                gix::repository::blame_file::Options::default(),
            )
            .map_err(git)?;
        let mailmap = ident::mailmap(&repo, tip);
        let mut commits: HashMap<gix::ObjectId, (String, i64)> = HashMap::new();
        let mut lines: Vec<BlameLine> = Vec::new();
        let source: Vec<&str> = text.split_inclusive('\n').collect();
        for entry in &outcome.entries {
            let (author, time) = if let Some(known) = commits.get(&entry.commit_id) {
                known.clone()
            } else {
                let commit = repo.find_commit(entry.commit_id).map_err(git)?;
                let sig = ident::signature(&mailmap, commit.author().map_err(git)?);
                let value = (sig.name, sig.time);
                commits.insert(entry.commit_id, value.clone());
                value
            };
            for offset in 0..entry.len.get() {
                let line_no = entry.start_in_blamed_file + offset;
                let raw = source.get(line_no as usize).copied().unwrap_or_default();
                lines.push(BlameLine {
                    line_no: line_no + 1,
                    commit: entry.commit_id.to_string(),
                    author: author.clone(),
                    time,
                    text: raw.trim_end_matches(['\n', '\r']).to_string(),
                });
            }
        }
        lines.sort_by_key(|l| l.line_no);
        Ok(lines)
    }

    /// Branches, tags and remote branches, newest first.
    pub fn refs(&self) -> Result<Vec<RefInfo>> {
        let repo = self.repo();
        let platform = repo.references().map_err(git)?;
        let mut out = Vec::new();
        for reference in platform.all().map_err(git)? {
            let mut reference = reference.map_err(git)?;
            let name = reference.name().as_bstr().to_str_lossy().into_owned();
            let (kind, short) = if let Some(s) = name.strip_prefix("refs/heads/") {
                (RefKind::Branch, s)
            } else if let Some(s) = name.strip_prefix("refs/tags/") {
                (RefKind::Tag, s)
            } else if let Some(s) = name.strip_prefix("refs/remotes/") {
                (RefKind::Remote, s)
            } else {
                continue;
            };
            if short.ends_with("/HEAD") {
                continue;
            }
            let Ok(commit) = reference.peel_to_commit() else {
                continue;
            };
            let time = commit.time().map_or(0, |t| t.seconds);
            out.push(RefInfo {
                name: short.to_string(),
                kind,
                target: commit.id.to_string(),
                time,
            });
        }
        out.sort_by_key(|r| Reverse(r.time));
        Ok(out)
    }

    /// Name of the branch HEAD points at, or `None` when detached.
    pub fn head_branch(&self) -> Option<String> {
        let name = self.repo().head_name().ok()??;
        Some(name.shorten().to_str_lossy().into_owned())
    }

    fn repo(&self) -> gix::Repository {
        let mut repo = self.inner.to_thread_local();
        repo.object_cache_size_if_unset(OBJECT_CACHE_BYTES);
        repo
    }

    fn resolve_id(repo: &gix::Repository, rev: &str) -> Result<gix::ObjectId> {
        let not_found = || Error::RevNotFound(rev.to_string());
        let id = repo.rev_parse_single(rev).map_err(|_| not_found())?;
        let commit = id
            .object()
            .map_err(git)?
            .peel_to_commit()
            .map_err(|_| not_found())?;
        Ok(commit.id)
    }
}

fn summary(commit: &gix::Commit<'_>, mailmap: &gix::mailmap::Snapshot) -> Result<CommitSummary> {
    let message = commit.message_raw().map_err(git)?;
    let subject = message
        .lines()
        .next()
        .unwrap_or_default()
        .to_str_lossy()
        .trim()
        .to_string();
    Ok(CommitSummary {
        id: commit.id.to_string(),
        parents: commit.parent_ids().map(|p| p.to_string()).collect(),
        author: ident::signature(mailmap, commit.author().map_err(git)?),
        committer: ident::signature(mailmap, commit.committer().map_err(git)?),
        subject,
    })
}

/// History simplification as `git log -- path`: a commit counts when the path's object differs
/// from every parent (a root counts when the path exists).
fn touches_path(repo: &gix::Repository, commit: &gix::Commit<'_>, path: &str) -> Result<bool> {
    let here = path_id(commit, path)?;
    let mut parents = commit.parent_ids().peekable();
    if parents.peek().is_none() {
        return Ok(here.is_some());
    }
    for parent in parents {
        let parent = repo.find_commit(parent).map_err(git)?;
        if path_id(&parent, path)? == here {
            return Ok(false);
        }
    }
    Ok(true)
}

fn path_id(commit: &gix::Commit<'_>, path: &str) -> Result<Option<gix::ObjectId>> {
    let tree = commit.tree().map_err(git)?;
    let entry = tree.lookup_entry_by_path(path).map_err(git)?;
    Ok(entry.map(|e| e.object_id()))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::process::Command;

    use crate::{ChangeKind, LineKind, Repo};

    fn git(dir: &std::path::Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_DATE", "2024-01-01T00:00:00+00:00")
            .env("GIT_COMMITTER_DATE", "2024-01-01T00:00:00+00:00")
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn write(dir: &std::path::Path, path: &str, body: &str) {
        let full = dir.join(path);
        fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
        fs::write(full, body).expect("write");
    }

    fn commit(dir: &std::path::Path, msg: &str) {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", msg]);
    }

    fn numbered(n: usize) -> String {
        (1..=n).fold(String::new(), |acc, i| acc + &format!("line {i}\n"))
    }

    fn fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        let d = tmp.path();
        git(d, &["init", "-q", "-b", "main"]);
        git(d, &["config", "user.name", "Old Name"]);
        git(d, &["config", "user.email", "old@example.com"]);
        write(
            d,
            ".mailmap",
            "New Name <new@example.com> <old@example.com>\n",
        );
        write(d, "a.txt", &numbered(40));
        write(d, "src/b.txt", "one\ntwo\n");
        commit(d, "feat: first\n\nbody text\nmore body");
        let mut edited = numbered(40)
            .replace("line 5\n", "five\n")
            .replace("line 30\n", "thirty\n");
        edited.push_str("tail\n");
        write(d, "a.txt", &edited);
        write(d, "bin.dat", "\0\u{1}\u{2}binary");
        commit(d, "fix: edit a, add binary");
        fs::rename(d.join("src/b.txt"), d.join("src/c.txt")).expect("rename");
        commit(d, "refactor: rename b to c");
        fs::remove_file(d.join("a.txt")).expect("rm");
        write(d, "src/c.txt", "one\ntwo\nthree\n");
        commit(d, "chore: drop a, grow c");
        tmp
    }

    /// Parses `git log --numstat -z -M` into id to path to `(add, del, old_path)`.
    #[allow(clippy::type_complexity)]
    fn reference(
        dir: &std::path::Path,
    ) -> BTreeMap<String, BTreeMap<String, (u32, u32, Option<String>)>> {
        let raw = git(
            dir,
            &[
                "log",
                "--numstat",
                "-z",
                "-M",
                "--format=%x01%H%x00",
                "--topo-order",
            ],
        );
        let mut out = BTreeMap::new();
        for chunk in raw.split('\u{1}').filter(|c| !c.is_empty()) {
            let mut parts = chunk.split('\0');
            let id = parts.next().expect("id").to_string();
            let mut files = BTreeMap::new();
            while let Some(head) = parts.next() {
                let head = head.trim_start_matches('\n');
                if head.is_empty() {
                    continue;
                }
                let mut f = head.split('\t');
                let (a, d, p) = (
                    f.next().expect("a"),
                    f.next().expect("d"),
                    f.next().expect("p"),
                );
                let (path, old) = if p.is_empty() {
                    let old = parts.next().expect("old").to_string();
                    (parts.next().expect("new").to_string(), Some(old))
                } else {
                    (p.to_string(), None)
                };
                let num = |s: &str| s.parse().unwrap_or(0);
                files.insert(path, (num(a), num(d), old));
            }
            out.insert(id, files);
        }
        out
    }

    #[test]
    fn numstat_matches_git() {
        let tmp = fixture();
        let repo = Repo::discover(tmp.path()).expect("open");
        let want = reference(tmp.path());
        assert_eq!(want.len(), 4);
        for id in repo.rev_list("HEAD").expect("rev_list") {
            let got: BTreeMap<_, _> = repo
                .numstat(&id)
                .expect("numstat")
                .into_iter()
                .map(|f| (f.path, (f.additions, f.deletions, f.old_path)))
                .collect();
            assert_eq!(got, want[&id], "commit {id}");
        }
    }

    #[test]
    fn commit_detail_has_hunks_kinds_and_body() {
        let tmp = fixture();
        let repo = Repo::discover(tmp.path()).expect("open");
        let log = repo.log("HEAD", None, 0, 10).expect("log");
        assert_eq!(log.len(), 4);
        let second = &log[2];
        assert_eq!(second.subject, "fix: edit a, add binary");
        let detail = repo.commit(&second.id).expect("commit");
        let a = detail
            .files
            .iter()
            .find(|f| f.path == "a.txt")
            .expect("a.txt");
        assert_eq!(
            (a.kind, a.additions, a.deletions),
            (ChangeKind::Modified, 3, 2)
        );
        assert_eq!(a.hunks.len(), 3);
        let first = &a.hunks[0];
        assert_eq!(first.header, "@@ -2,7 +2,7 @@");
        let removed = first
            .lines
            .iter()
            .find(|l| l.kind == LineKind::Removed)
            .expect("removed");
        assert_eq!(
            (removed.old_no, removed.new_no, removed.text.as_str()),
            (Some(5), None, "line 5")
        );
        let bin = detail
            .files
            .iter()
            .find(|f| f.path == "bin.dat")
            .expect("bin");
        assert!(bin.binary && bin.hunks.is_empty() && bin.kind == ChangeKind::Added);

        let root = repo.commit(&log[3].id).expect("root");
        assert_eq!(root.body, "body text\nmore body");
        assert!(root.files.iter().all(|f| f.kind == ChangeKind::Added));

        let rename = repo.commit(&log[1].id).expect("rename");
        assert_eq!(rename.files.len(), 1);
        assert_eq!(rename.files[0].kind, ChangeKind::Renamed);
        assert_eq!(rename.files[0].old_path.as_deref(), Some("src/b.txt"));
    }

    #[test]
    fn log_orders_filters_and_pages() {
        let tmp = fixture();
        let repo = Repo::discover(tmp.path()).expect("open");
        let all = repo.log("main", None, 0, 100).expect("log");
        assert_eq!(all[0].subject, "chore: drop a, grow c");
        let paged = repo.log("main", None, 1, 2).expect("log");
        assert_eq!(paged.len(), 2);
        assert_eq!(paged[0].id, all[1].id);
        let on_a = repo.log("HEAD", Some("a.txt"), 0, 100).expect("log");
        let subjects: Vec<_> = on_a.iter().map(|c| c.subject.as_str()).collect();
        assert_eq!(
            subjects,
            [
                "chore: drop a, grow c",
                "fix: edit a, add binary",
                "feat: first"
            ]
        );
        let oldest_first = repo.rev_list("HEAD").expect("rev_list");
        assert_eq!(oldest_first.last(), Some(&all[0].id));
        assert!(matches!(
            repo.resolve("nope"),
            Err(crate::Error::RevNotFound(_))
        ));
    }

    #[test]
    fn mailmap_is_applied() {
        let tmp = fixture();
        let repo = Repo::discover(tmp.path()).expect("open");
        let log = repo.log("HEAD", None, 0, 1).expect("log");
        assert_eq!(log[0].author.name, "New Name");
        assert_eq!(log[0].author.email, "new@example.com");
    }

    #[test]
    fn tree_blob_blame_refs() {
        let tmp = fixture();
        let d = tmp.path();
        git(d, &["tag", "v1"]);
        let repo = Repo::discover(d).expect("open");
        let first = repo.log("HEAD", None, 0, 10).expect("log")[3].id.clone();
        let entries = repo.tree(&first, "").expect("tree");
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["src", ".mailmap", "a.txt"]);
        assert_eq!(repo.tree("HEAD", "src").expect("src").len(), 1);
        assert!(repo.tree("HEAD", "missing").is_err());
        let blob = repo.blob(&first, "src/b.txt").expect("blob");
        assert_eq!(blob.text.as_deref(), Some("one\ntwo\n"));
        assert!(repo.blob("HEAD", "src/c.txt").is_ok());
        let second = repo.log("HEAD", None, 0, 10).expect("log")[2].id.clone();
        assert!(repo.blob(&second, "bin.dat").expect("bin").text.is_none());

        let blame = repo.blame(&second, "a.txt").expect("blame");
        assert_eq!(blame.len(), 41);
        assert_eq!(blame[4].text, "five");
        assert_ne!(blame[4].commit, blame[0].commit);
        assert_eq!(blame[0].author, "New Name");

        assert_eq!(repo.head_branch().as_deref(), Some("main"));
        let refs = repo.refs().expect("refs");
        let kinds: Vec<_> = refs.iter().map(|r| (r.name.as_str(), r.kind)).collect();
        assert!(kinds.contains(&("main", crate::RefKind::Branch)));
        assert!(kinds.contains(&("v1", crate::RefKind::Tag)));
        assert!(!repo.name().is_empty());
    }

    /// Run with `GITTERRACE_BENCH_REPO=<path> cargo test -p gitterrace-git --release bench -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a large real repository"]
    fn bench_real_repo() {
        let Ok(path) = std::env::var("GITTERRACE_BENCH_REPO") else {
            return;
        };
        let repo = Repo::discover(std::path::Path::new(&path)).expect("open");
        let t = std::time::Instant::now();
        let first = repo.log("HEAD", None, 0, 50).expect("log");
        println!("log 50: {:?} ({} commits)", t.elapsed(), first.len());
        let t = std::time::Instant::now();
        let ids = repo.rev_list("HEAD").expect("rev_list");
        println!("rev_list: {:?} ({} commits)", t.elapsed(), ids.len());
        let t = std::time::Instant::now();
        let mut files = 0;
        for chunk in ids.chunks(2000) {
            files += repo
                .numstat_many(chunk)
                .expect("numstat")
                .iter()
                .map(Vec::len)
                .sum::<usize>();
        }
        println!("numstat all: {:?} ({files} file changes)", t.elapsed());
    }
}
