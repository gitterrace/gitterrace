//! Transport-neutral reads. Both the HTTP router and the MCP tools call these,
//! so the two surfaces cannot drift apart. Everything here is blocking.

use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};
use gitterrace_git::Repo;
use gitterrace_index::{CommitRecord, Index};

use crate::error::ApiError;
use crate::{insights, json};

const DEFAULT_REV: &str = "HEAD";
const MAX_PAGE: usize = 500;
const MAX_BLOB_LINES: usize = 5000;

/// Shared handle on one repository and its stats cache.
#[derive(Clone)]
pub struct ApiState {
    pub(crate) repo: Arc<Repo>,
    pub(crate) index: Arc<Index>,
}

impl ApiState {
    pub fn new(repo: Arc<Repo>, index: Arc<Index>) -> Self {
        Self { repo, index }
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

    /// Entity tag for the current answer: head commit id plus cache format version.
    pub fn etag(&self) -> Option<String> {
        let head = self.repo.resolve(DEFAULT_REV).ok()?;
        let version = std::fs::read_to_string(self.index.dir().join("VERSION"))
            .map_or_else(|_| "0".to_owned(), |v| v.trim().to_owned());
        Some(format!("\"{head}-{version}\""))
    }

    fn records(&self, rev: &str) -> Result<Vec<CommitRecord>, ApiError> {
        Ok(self.index.records(&self.repo, rev)?)
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
                (
                    Some(slice.join("\n")),
                    Some(all.len()),
                    end < all.len() || start > 0,
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
        let lines: Vec<Value> = self
            .repo
            .blame(&id, path)?
            .iter()
            .map(json::blame_line)
            .collect();
        Ok(json!({ "rev": rev, "commit": id, "path": path, "lines": lines }))
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
        let total: usize = authors.iter().map(|a| a.commits).sum();
        let mut acc = 0;
        let mut bus_factor = 0;
        for a in &authors {
            if acc * 2 >= total {
                break;
            }
            acc += a.commits;
            bus_factor += 1;
        }
        Ok(json!({ "rev": rev, "bus_factor": bus_factor, "authors": authors }))
    }
}
