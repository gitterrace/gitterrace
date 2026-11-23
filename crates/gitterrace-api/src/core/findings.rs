//! The review basket. The reads are served on both transports. The writes are the web's alone:
//! this module offers them as plain methods and the API router and the MCP server never call
//! them. The text of a note is whatever the user typed in the page and is stored as plain text,
//! cut at [`gitterrace_index::MAX_NOTE`]: it is escaped wherever it is shown.

use gitterrace_actions::{kind_name, schema::KINDS};
use gitterrace_git::{Anchor, AnchorQuery};
use gitterrace_index::StoredFinding;
use gitterrace_types::AnchorBasis;
use gitterrace_types::api::{Finding, FindingAnchor, FindingLines, FindingsResponse, NewFinding};

use crate::core::ApiState;
use crate::error::ApiError;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// The kinds a finding may have: every kind but the basket's own.
fn is_finding_kind(k: &str) -> bool {
    KINDS.iter().any(|(kind, _)| kind_name(*kind) == k && k != "review")
}

fn anchored(f: StoredFinding, a: &Anchor) -> Finding {
    Finding {
        added: f.added,
        anchor: FindingAnchor {
            checked_against: if a.worktree { AnchorBasis::Worktree } else { AnchorBasis::Head },
            now_lines: a.now_lines.map(|(start, end)| FindingLines { end, start }),
            now_path: a.now_path.clone(),
            status: a.status,
        },
        id: f.id,
        kind: f.kind,
        lines: f.lines.map(|(start, end)| FindingLines { end, start }),
        note: f.note,
        path: f.path,
        query: f.query,
        rev: f.rev,
        sha: f.sha,
    }
}

impl ApiState {
    fn cap_entries(&self) -> usize {
        self.settings().review.findings_entries
    }

    /// Whether the basket can be kept: no newer build owns the cache and its folder exists.
    fn basket_persists(&self) -> bool {
        self.index.newer_owner().is_none() && self.index.dir().is_dir()
    }

    /// The basket, oldest first, each entry with where its code is now.
    pub fn findings(&self) -> Result<FindingsResponse, ApiError> {
        self.only_here("findings")?;
        let persisted = self.basket_persists();
        let stored = if persisted { self.index.findings_list()? } else { Vec::new() };
        Ok(FindingsResponse {
            cap_entries: self.cap_entries(),
            count: stored.len(),
            findings: self.with_anchors(stored)?,
            persisted,
        })
    }

    fn with_anchors(&self, stored: Vec<StoredFinding>) -> Result<Vec<Finding>, ApiError> {
        let queries: Vec<AnchorQuery> = stored
            .iter()
            .map(|f| AnchorQuery { sha: f.sha.clone(), path: f.path.clone(), lines: f.lines })
            .collect();
        let anchors = gitterrace_git::anchors_now(&self.repo, &queries)?;
        Ok(stored.into_iter().zip(&anchors).map(|(f, a)| anchored(f, a)).collect())
    }

    /// The entries named in `ids`, in the basket's order, for a `review` payload. An id that is
    /// not in the basket is a 400: the page asked for something the server never stored.
    pub fn findings_by_id(&self, ids: &[String]) -> Result<Vec<Finding>, ApiError> {
        self.only_here("findings")?;
        let stored = self.index.findings_list()?;
        if let Some(missing) = ids.iter().find(|i| !stored.iter().any(|f| f.id == **i)) {
            return Err(ApiError::BadRequest(format!("no finding {missing} in the basket")));
        }
        let picked = stored.into_iter().filter(|f| ids.contains(&f.id)).collect();
        self.with_anchors(picked)
    }

    /// Adds `new` to the basket and returns its id. Web only: the API and MCP never call it.
    /// The revision and the file are checked against the repository, the note is cut and
    /// stripped of control characters.
    pub fn finding_add(&self, new: &NewFinding) -> Result<String, ApiError> {
        self.only_here("findings")?;
        if !self.basket_persists() {
            return Err(ApiError::BadRequest(
                "the review basket cannot be kept: the cache folder cannot be written".to_owned(),
            ));
        }
        if !is_finding_kind(&new.kind) {
            return Err(ApiError::BadRequest(format!("unknown finding kind {:?}", new.kind)));
        }
        let rev = new.sha.as_deref().unwrap_or(&new.rev);
        let sha = self.repo.resolve(rev)?;
        let path = new.path.as_deref().map(|p| p.trim_matches('/')).filter(|p| !p.is_empty());
        if let Some(p) = path
            && (p.len() > 4096 || !self.repo.has(&sha, Some(p)))
        {
            return Err(ApiError::BadRequest("path not in the tree at this commit".to_owned()));
        }
        let lines = match new.lines {
            Some(l) if path.is_none() || l.start == 0 || l.end < l.start => {
                return Err(ApiError::BadRequest("lines need a path and start <= end".to_owned()));
            }
            Some(l) => Some((l.start, l.end)),
            None => None,
        };
        let query = new.query.as_deref().map(|q| q.chars().take(1024).collect::<String>());
        let rev: String = new.rev.chars().filter(|c| !c.is_control()).take(256).collect();
        let f = StoredFinding {
            id: String::new(),
            kind: new.kind.clone(),
            rev,
            sha,
            path: path.map(str::to_owned),
            lines,
            query,
            note: new.note.clone(),
            added: now(),
        }
        .sealed();
        let id = f.id.clone();
        self.index.findings_add(&f, self.cap_entries())?;
        Ok(id)
    }

    /// Removes the findings in `ids`, or empties the basket when `ids` is empty. Returns how
    /// many went. Web only.
    pub fn findings_remove(&self, ids: &[String]) -> Result<usize, ApiError> {
        self.only_here("findings")?;
        if self.index.newer_owner().is_some() {
            return Err(ApiError::BadRequest("the cache belongs to a newer gitterrace".to_owned()));
        }
        Ok(self.index.findings_remove(ids)?)
    }
}
