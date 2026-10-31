//! Transport-neutral reads. Both the HTTP router and the MCP tools call these,
//! so the two surfaces cannot drift apart. Everything here is blocking.

use std::path::Path;
use std::sync::{Arc, Mutex};

use gitterrace_git::Repo;
use gitterrace_index::{CommitRecord, Index};
use serde_json::{Value, json};

use crate::error::ApiError;
use crate::{insights, json};

const DEFAULT_REV: &str = "HEAD";
const MAX_PAGE: usize = 500;
const MAX_BLOB_LINES: usize = 5000;
/// Lines alone do not bound an answer: one minified line can be megabytes.
const MAX_BLOB_BYTES: usize = 256 * 1024;
const MAX_BLAME_LINES: usize = 5000;

/// Shared handle on one repository and its insights cache.
#[derive(Clone)]
pub struct ApiState {
    pub(crate) repo: Arc<Repo>,
    pub(crate) index: Arc<Index>,
    /// Parsed records of the last request, keyed by (resolved tip, index generation).
    cache: Arc<Mutex<Option<RecordsCache>>>,
}

type RecordsCache = ((String, u64), Arc<Vec<CommitRecord>>);

impl ApiState {
    pub fn new(repo: Arc<Repo>, index: Arc<Index>) -> Self {
        Self {
            repo,
            index,
            cache: Arc::default(),
        }
    }

    /// Opens the repository containing `path` and its cache.
    pub fn open(path: &Path) -> Result<Self, ApiError> {
        let repo = Repo::discover(path)?;
        let index = Index::open(&repo)?;
        Ok(Self::new(Arc::new(repo), Arc::new(index)))
    }

    /// Brings the cache up to date with `HEAD`. Blocking; the web layer does this in the
    /// background, the MCP server does it once at start.
    pub fn refresh_index(&self) -> Result<(), ApiError> {
        self.index.update(&self.repo, DEFAULT_REV, &|_| {})?;
        Ok(())
    }

    /// Entity tag for every answer: HEAD, every ref's target and the cache format
    /// version. Requests may name any `rev`, so moving any branch or tag must
    /// invalidate, not only a new HEAD commit.
    pub fn etag(&self) -> Option<String> {
        use std::hash::{Hash, Hasher};
        let head = self.repo.resolve(DEFAULT_REV).ok()?;
        let mut refs: Vec<(String, String)> = self
            .repo
            .refs()
            .ok()?
            .into_iter()
            .map(|r| (r.name, r.target))
            .collect();
        refs.sort();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        refs.hash(&mut h);
        let version = std::fs::read_to_string(self.index.dir().join("VERSION"))
            .map_or_else(|_| "0".to_owned(), |v| v.trim().to_owned());
        Some(format!("\"{head}-{:016x}-{version}\"", h.finish()))
    }

    /// Records reachable from `rev`, shared between calls while the tip and the cache file are
    /// unchanged.
    pub(crate) fn records(&self, rev: &str) -> Result<Arc<Vec<CommitRecord>>, ApiError> {
        let tip = self.repo.resolve(rev)?;
        let key = (tip, self.index.generation());
        if let Ok(g) = self.cache.lock()
            && let Some((k, v)) = g.as_ref()
            && *k == key
        {
            return Ok(Arc::clone(v));
        }
        let recs = Arc::new(self.index.records(&self.repo, rev)?);
        if let Ok(mut g) = self.cache.lock() {
            *g = Some((key, Arc::clone(&recs)));
        }
        Ok(recs)
    }

    fn rev(rev: Option<&str>) -> &str {
        rev.filter(|r| !r.is_empty()).unwrap_or(DEFAULT_REV)
    }

    pub fn repo_info(&self) -> Result<Value, ApiError> {
        let head = self.repo.resolve(DEFAULT_REV).ok();
        let totals = self
            .records(DEFAULT_REV)
            .map(|r| gitterrace_index::totals(&r))?;
        Ok(json!({
            "name": self.repo.name(),
            "head": head,
            "head_branch": self.repo.head_branch(),
            "index_version": self.etag(),
            "totals": totals,
        }))
    }

    pub fn refs(&self) -> Result<Value, ApiError> {
        let refs: Vec<Value> = self.repo.refs()?.iter().map(json::ref_info).collect();
        Ok(json!({ "refs": refs }))
    }

    pub fn log(
        &self,
        rev: Option<&str>,
        path: Option<&str>,
        page: usize,
        per_page: usize,
    ) -> Result<Value, ApiError> {
        let rev = Self::rev(rev);
        let per_page = per_page.clamp(1, MAX_PAGE);
        let page = page.max(1);
        // One extra row tells us whether another page exists.
        let rows = self
            .repo
            .log(rev, path, (page - 1) * per_page, per_page + 1)?;
        let has_more = rows.len() > per_page;
        let commits: Vec<Value> = rows.iter().take(per_page).map(json::summary).collect();
        Ok(json!({
            "rev": rev, "path": path, "page": page, "per_page": per_page,
            "has_more": has_more, "commits": commits,
        }))
    }

    pub fn commit(
        &self,
        id: &str,
        diff: bool,
        max_lines: Option<usize>,
    ) -> Result<Value, ApiError> {
        let id = self.repo.resolve(id)?;
        let detail = self.repo.commit(&id)?;
        Ok(json::detail(&detail, diff, max_lines))
    }

    pub fn tree(&self, rev: Option<&str>, path: Option<&str>) -> Result<Value, ApiError> {
        let rev = Self::rev(rev);
        let id = self.repo.resolve(rev)?;
        let path = path.unwrap_or("");
        let entries: Vec<Value> = self
            .repo
            .tree(&id, path)?
            .iter()
            .map(json::tree_entry)
            .collect();
        Ok(json!({ "rev": rev, "commit": id, "path": path, "entries": entries }))
    }

    /// Text content of a file. Binary files report `binary: true` and no text.
    /// `start_line` is 1-based; at most `limit` lines are returned.
    pub fn blob(
        &self,
        rev: Option<&str>,
        path: &str,
        start_line: Option<usize>,
        limit: Option<usize>,
    ) -> Result<Value, ApiError> {
        let rev = Self::rev(rev);
        let id = self.repo.resolve(rev)?;
        let blob = self.repo.blob(&id, path)?;
        let (text, total_lines, truncated) = match &blob.text {
            Some(t) => {
                let all: Vec<&str> = t.lines().collect();
                let start = start_line.unwrap_or(1).max(1) - 1;
                let limit = limit.unwrap_or(MAX_BLOB_LINES).min(MAX_BLOB_LINES);
                let end = all.len().min(start.saturating_add(limit));
                let slice = all.get(start..end).unwrap_or(&[]);
                let mut text = slice.join("\n");
                let over = text.len() > MAX_BLOB_BYTES;
                if over {
                    let mut cut = MAX_BLOB_BYTES;
                    while !text.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    text.truncate(cut);
                }
                (
                    Some(text),
                    Some(all.len()),
                    over || end < all.len() || start > 0,
                )
            }
            None => (None, None, false),
        };
        Ok(json!({
            "rev": rev, "commit": id, "path": blob.path, "size": blob.size,
            "binary": blob.text.is_none(), "total_lines": total_lines,
            "truncated": truncated, "text": text,
        }))
    }

    pub fn blame(&self, rev: Option<&str>, path: &str) -> Result<Value, ApiError> {
        let rev = Self::rev(rev);
        let id = self.repo.resolve(rev)?;
        let all = self.repo.blame(&id, path)?;
        let lines: Vec<Value> = all
            .iter()
            .take(MAX_BLAME_LINES)
            .map(json::blame_line)
            .collect();
        Ok(json!({
            "rev": rev, "commit": id, "path": path,
            "truncated": all.len() > MAX_BLAME_LINES, "lines": lines,
        }))
    }

    /// One named aggregate over the cached records reachable from `rev`.
    pub fn insight(
        &self,
        name: &str,
        rev: Option<&str>,
        params: &insights::Params,
    ) -> Result<Value, ApiError> {
        let rev = Self::rev(rev);
        let records = self.records(rev)?;
        let data = insights::compute(name, &records, params)?;
        Ok(json!({ "rev": rev, "insight": name, "commits_indexed": records.len(), "data": data }))
    }

    /// Authors plus a commit-share bus factor: the fewest authors who made half the commits.
    pub fn contributors(&self, rev: Option<&str>) -> Result<Value, ApiError> {
        let rev = Self::rev(rev);
        let mut authors = gitterrace_index::authors(&self.records(rev)?);
        authors.sort_by_key(|a| std::cmp::Reverse(a.commits));
        let bus_factor = gitterrace_index::bus_factor(authors.iter().map(|a| a.commits as f64));
        Ok(json!({ "rev": rev, "bus_factor": bus_factor, "authors": authors }))
    }
}
