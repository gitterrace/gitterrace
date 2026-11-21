//! The repositories one MCP server may read: a table fixed at start, opened lazily.
//!
//! A tool names a repository only by its table name. The name is looked up, never joined to a
//! path, so no argument reaches a folder outside the table.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use gitterrace_config::{Config, Feature};
use gitterrace_git::Repo;
use schemars::JsonSchema;
use serde::Serialize;

use crate::core::ApiState;
use crate::error::ApiError;

/// Repositories kept open at once. Each holds a git handle and its insights cache; past this the
/// least recently used is closed, so an agent walking many repositories stays bounded.
/// Not configurable: closing reopens on the next call, so the bound only trades speed for
/// memory and no answer depends on it.
const OPEN_MAX: usize = 8;

/// One repository in scope, as the caller found it.
#[derive(Debug, Clone)]
pub struct McpRepo {
    /// The name tools take as `repo`, the `/r/NAME` of the web server when it has one.
    pub name: String,
    pub path: PathBuf,
    /// Why it cannot be read, for a recorded folder that is gone or is not a repository.
    pub unavailable: Option<String>,
}

/// The newest commit of a repository in `list_repos`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct LastCommit {
    id: String,
    subject: String,
    /// Committer time, seconds since the Unix epoch.
    time: i64,
}

/// One row of `list_repos`.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub(crate) struct RepoRow {
    name: String,
    available: bool,
    error: Option<String>,
    head: Option<String>,
    head_branch: Option<String>,
    last_commit: Option<LastCommit>,
}

/// The `list_repos` answer.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct RepoList {
    /// Where the list comes from: the command line alone, or also the repositories served before.
    scope: String,
    repos: Vec<RepoRow>,
}

/// The table and the repositories open right now, most recently used last.
pub(crate) struct Repos {
    scope: String,
    list: Vec<McpRepo>,
    open: Mutex<Vec<(String, ApiState)>>,
    /// One gate per name, so two first calls on a cold repository open it once while other
    /// repositories stay free.
    gates: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// How many times a repository was opened from disk, for the single-flight test.
    opens: AtomicUsize,
}

impl Repos {
    pub(crate) fn new(scope: String, list: Vec<McpRepo>) -> Self {
        Self {
            scope,
            list,
            open: Mutex::new(Vec::new()),
            gates: Mutex::new(HashMap::new()),
            opens: AtomicUsize::new(0),
        }
    }

    /// A table of the one repository `state` reads, already open.
    pub(crate) fn single(state: ApiState) -> Self {
        let name = state.repo.name();
        let me = McpRepo { name: name.clone(), path: state.repo.root(), unavailable: None };
        let r = Self::new("one repository".to_owned(), vec![me]);
        r.lock().push((name, state));
        r
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<(String, ApiState)>> {
        self.open.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The table entry `repo` names: the only one when it is left out and there is one.
    pub(crate) fn pick(&self, repo: Option<&str>) -> Result<&McpRepo, ApiError> {
        let names = || {
            let all: Vec<&str> = self.list.iter().map(|r| r.name.as_str()).collect();
            all.join(", ")
        };
        let found = match repo {
            Some(n) => self.list.iter().find(|r| r.name == n).ok_or_else(|| {
                ApiError::BadRequest(format!(
                    "unknown repo {n:?}: call list_repos for the names in scope ({})",
                    names()
                ))
            })?,
            None if self.list.len() == 1 => &self.list[0],
            None => {
                return Err(ApiError::BadRequest(format!(
                    "several repositories are in scope, pass repo: one of {}",
                    names()
                )));
            }
        };
        if let Some(why) = &found.unavailable {
            return Err(ApiError::BadRequest(format!("repo {} is unavailable: {why}", found.name)));
        }
        Ok(found)
    }

    /// The open state of `entry`, opening it (and bringing its insights cache up to date) on
    /// first use. Blocking.
    #[expect(
        clippy::print_stderr,
        reason = "a stdio MCP server owns stdout for the protocol, so warnings go to stderr"
    )]
    pub(crate) fn state(&self, entry: &McpRepo, cfg: &Config) -> Result<ApiState, ApiError> {
        {
            let mut open = self.lock();
            if let Some(i) = open.iter().position(|(n, _)| *n == entry.name) {
                let hit = open.remove(i);
                let state = hit.1.clone();
                open.push(hit);
                return Ok(state);
            }
        }
        let gate = {
            let mut g = self.gates.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            Arc::clone(g.entry(entry.name.clone()).or_default())
        };
        let _one = gate.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, s)) = self.lock().iter().find(|(n, _)| *n == entry.name) {
            return Ok(s.clone());
        }
        self.opens.fetch_add(1, Ordering::SeqCst);
        let state = ApiState::open(&entry.path)?.configured(cfg);
        if let Err(e) = state.refresh_index() {
            eprintln!(
                "gitterrace: insights index of {} incomplete, serving anyway: {e}",
                entry.name
            );
        }
        let mut open = self.lock();
        open.push((entry.name.clone(), state.clone()));
        let over = open.len().saturating_sub(OPEN_MAX);
        open.drain(..over);
        Ok(state)
    }

    /// How many opens from disk happened so far.
    #[cfg(test)]
    pub(crate) fn opens(&self) -> usize {
        self.opens.load(Ordering::SeqCst)
    }

    /// Every repository in scope with its HEAD and newest commit, read from git alone: listing
    /// never builds an insights cache.
    pub(crate) fn list(&self, cfg: &Config) -> RepoList {
        let repos = self.list.iter().map(|r| row(r, cfg)).collect();
        RepoList { scope: self.scope.clone(), repos }
    }
}

/// A repository whose api switch is off is named, so an agent knows why it cannot read it, and
/// nothing else: no path, HEAD or subject.
fn row(r: &McpRepo, cfg: &Config) -> RepoRow {
    if !cfg.feature_on(Feature::Api, Some(&r.name)) {
        return RepoRow {
            name: r.name.clone(),
            error: Some("api off".to_owned()),
            ..RepoRow::default()
        };
    }
    let mut out =
        RepoRow { name: r.name.clone(), error: r.unavailable.clone(), ..RepoRow::default() };
    if out.error.is_some() {
        return out;
    }
    match Repo::discover(&r.path) {
        Ok(repo) => {
            out.available = true;
            out.head = repo.resolve("HEAD").ok();
            out.head_branch = repo.head_branch();
            out.last_commit = repo
                .log("HEAD", None, 0, 1)
                .ok()
                .and_then(|v| v.into_iter().next())
                .map(|c| LastCommit { id: c.id, subject: c.subject, time: c.committer.time });
        }
        Err(e) => out.error = Some(e.to_string()),
    }
    out
}
