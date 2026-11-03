//! Line blame and its per-handle cache.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::error::git;
use crate::ident;
use crate::repo::{BLAME_OBJECT_CACHE_BYTES, Repo};
use crate::{BlameLine, Result};

/// Blame results kept per (commit id, path), bounded by approximate bytes.
pub(super) const BLAME_CACHE_BYTES: usize = 32 * 1024 * 1024;

pub(super) struct BlameCache {
    /// Approximate bytes kept before the oldest entries go.
    pub(super) limit: usize,
    map: HashMap<(gix::ObjectId, String), Arc<Vec<BlameLine>>>,
    /// Least recently used first.
    order: VecDeque<(gix::ObjectId, String)>,
    bytes: usize,
}

fn blame_size(lines: &[BlameLine]) -> usize {
    lines
        .iter()
        .map(|l| 64 + l.commit.len() + l.author.len() + l.text.len())
        .sum()
}

impl BlameCache {
    pub(super) fn with_limit(limit: usize) -> Self {
        Self {
            limit,
            map: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
        }
    }

    pub(super) fn get(&mut self, key: &(gix::ObjectId, String)) -> Option<Arc<Vec<BlameLine>>> {
        let hit = self.map.get(key)?.clone();
        if let Some(i) = self.order.iter().position(|k| k == key) {
            let k = self.order.remove(i)?;
            self.order.push_back(k);
        }
        Some(hit)
    }

    /// Drops the oldest entries until `room` more bytes fit under the limit.
    pub(super) fn shrink(&mut self, room: usize) {
        while self.bytes + room > self.limit {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(v) = self.map.remove(&old) {
                self.bytes -= blame_size(&v);
            }
        }
    }

    pub(super) fn put(&mut self, key: (gix::ObjectId, String), value: Arc<Vec<BlameLine>>) {
        let size = blame_size(&value);
        if size > self.limit || self.map.contains_key(&key) {
            return;
        }
        self.shrink(size);
        self.bytes += size;
        self.order.push_back(key.clone());
        self.map.insert(key, value);
    }
}

impl Repo {
    pub fn blame(&self, rev: &str, path: &str) -> Result<Vec<BlameLine>> {
        let mut repo = self.repo();
        // Blame touches every ancestor's trees once, so a big cache only adds resident memory.
        repo.object_cache_size(BLAME_OBJECT_CACHE_BYTES);
        let tip = self.resolve_id(&repo, rev)?;
        let path = path.trim_matches('/');
        let key = (tip, path.to_string());
        let id = Self::blob_id_at(&repo, tip, path)?;
        // Blame of a huge file costs as much as its diff against every ancestor. Checked before
        // the cache, because the text limit can change at runtime and is not part of the key.
        if repo.find_header(id).map_err(git)?.size() > self.text_limit() {
            return Ok(Vec::new());
        }
        if let Some(hit) = self.blame_cache.lock().ok().and_then(|mut c| c.get(&key)) {
            return Ok(hit.as_ref().clone());
        }
        let data = repo.find_object(id).map_err(git)?.detach().data;
        if crate::diff::is_binary(&data) {
            return Ok(Vec::new());
        }
        let Ok(text) = String::from_utf8(data) else {
            return Ok(Vec::new());
        };
        let outcome = blame_outcome(&repo, tip, path)?;
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
        if let Ok(mut c) = self.blame_cache.lock() {
            c.put(key, Arc::new(lines.clone()));
        }
        Ok(lines)
    }

    /// True when [`Repo::blame`] of `path` at `rev` would be served from the cache.
    pub fn blame_is_cached(&self, rev: &str, path: &str) -> bool {
        let repo = self.repo();
        let Ok(tip) = self.resolve_id(&repo, rev) else {
            return false;
        };
        let key = (tip, path.trim_matches('/').to_string());
        self.blame_cache
            .lock()
            .is_ok_and(|c| c.map.contains_key(&key))
    }

    /// Bytes the blame cache holds and may hold, so a prewarm can leave room for visitors.
    pub fn blame_cache_fill(&self) -> (usize, usize) {
        self.blame_cache
            .lock()
            .map_or((0, 0), |c| (c.bytes, c.limit))
    }
}

/// Blame of `path` at `tip` with the options `Repository::blame_file` uses by default. That
/// shortcut builds its diff cache through the repository's shared index snapshot, which keeps
/// the whole HEAD index alive for the life of the server; this one owns its index.
pub(crate) fn blame_outcome(
    repo: &gix::Repository,
    tip: gix::ObjectId,
    path: &str,
) -> Result<gix::blame::Outcome> {
    let mut cache = crate::diff::resource_cache(repo)?;
    gix::blame::file(
        &repo.objects,
        gix::blame::Start::Commit(tip),
        repo.commit_graph_if_enabled().map_err(git)?,
        &mut cache,
        path.into(),
        gix::blame::Options {
            diff_algorithm: repo.diff_algorithm().map_err(git)?,
            ranges: gix::blame::BlameRanges::default(),
            since: None,
            rewrites: None,
            debug_track_path: false,
        },
    )
    .map_err(git)
}

#[cfg(test)]
mod tests {
    /// Blame through the owned index is gix's own blame, entry for entry, on a history with
    /// moves, edits under `.gitattributes` rules and a merge. Red when an option differs from
    /// the shortcut's default (rewrites, since, ranges, algorithm) or attributes are dropped.
    #[test]
    fn the_owned_index_blame_is_gix_blame() -> crate::Result<()> {
        let dir = tempfile::tempdir().map_err(crate::error::git)?;
        let p = dir.path();
        let at = |args: &[&str]| gitterrace_testkit::git_at(p, "2024-01-02T12:00:00Z", args);
        let body = |tag: &str| (0..30).fold(String::new(), |b, l| b + &format!("{tag} {l}\n"));
        at(&["init", "-q", "-b", "main"]);
        std::fs::write(
            p.join(".gitattributes"),
            "*.txt text eol=crlf\n*.md diff=markdown\n",
        )
        .map_err(crate::error::git)?;
        std::fs::write(p.join("a.txt"), body("a")).map_err(crate::error::git)?;
        std::fs::write(p.join("b.md"), body("b")).map_err(crate::error::git)?;
        at(&["add", "-A"]);
        at(&["commit", "-qm", "one"]);
        at(&["mv", "a.txt", "moved.txt"]);
        std::fs::write(p.join("moved.txt"), body("a") + "tail\n").map_err(crate::error::git)?;
        at(&["commit", "-qam", "move and edit"]);
        at(&["checkout", "-q", "-b", "side", "HEAD~1"]);
        std::fs::write(p.join("b.md"), "side\n".to_owned() + &body("b"))
            .map_err(crate::error::git)?;
        at(&["commit", "-qam", "side edit"]);
        at(&["checkout", "-q", "main"]);
        at(&["merge", "-q", "--no-edit", "side"]);
        let repo = crate::Repo::discover(p)?;
        let gix = repo.repo();
        let tip = repo.resolve_id(&gix, "HEAD")?;
        for path in ["moved.txt", "b.md", ".gitattributes"] {
            let ours = super::blame_outcome(&gix, tip, path)?;
            let theirs = gix
                .blame_file(
                    path.into(),
                    tip,
                    gix::repository::blame_file::Options::default(),
                )
                .map_err(crate::error::git)?;
            assert_eq!(
                format!("{:?}", ours.entries),
                format!("{:?}", theirs.entries),
                "{path}"
            );
        }
        Ok(())
    }
}
