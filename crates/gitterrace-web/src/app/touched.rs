//! Which entries of a directory each commit touched, and the last commit of each entry.

use std::sync::Arc;
use std::time::Duration;

use gitterrace_git::LastCommit;

use crate::app::App;
use crate::error::Result;

impl App {
    /// Last commit per entry of `dir`, cached by the walk start and the directory.
    ///
    /// The key is the commit, not the directory's tree id: the same tree can sit in divergent
    /// histories with different last commits. A walk cut short by its budget is returned but not
    /// cached, so the next request walks again instead of serving the gaps forever.
    pub(crate) fn last_commits(&self, commit: &str, dir: &str) -> Result<Arc<Vec<LastCommit>>> {
        self.last_commits_within(commit, dir, Duration::from_secs(20))
    }

    pub(super) fn last_commits_within(
        &self,
        commit: &str,
        dir: &str,
        budget: Duration,
    ) -> Result<Arc<Vec<LastCommit>>> {
        let dir = dir.trim_matches('/');
        let key = format!("{commit}\0{dir}");
        if let Ok(g) = self.caches.last_commits.lock()
            && let Some(v) = g.get(&key)
        {
            return Ok(v);
        }
        // A ready index names the commits that changed anything under `dir`; the walk visits
        // only those instead of every commit in history.
        let among = self.accelerate(commit, |s| touched_names(&**s, dir))?;
        let (found, complete) =
            gitterrace_git::last_commits_among(&self.repo, commit, dir, among.as_deref(), budget)?;
        let v = Arc::new(found);
        if !complete {
            return Ok(v);
        }
        if let Ok(mut g) = self.caches.last_commits.lock() {
            g.put(key, Arc::clone(&v));
        }
        Ok(v)
    }
}

/// For each commit whose first-parent diff touches something under `dir`, newest first, the
/// names of the entries of `dir` it touches (old and new side of a rename).
pub(super) fn touched_names(
    src: &dyn gitterrace_index::feed::InsightsSource,
    dir: &str,
) -> Vec<(String, Vec<String>)> {
    use gitterrace_index::feed::CommitRange;
    let name = |p: &str| -> Option<String> {
        let rest = if dir.is_empty() {
            p
        } else {
            p.strip_prefix(dir)?.strip_prefix('/')?
        };
        Some(rest.split('/').next().unwrap_or(rest).to_owned())
    };
    let ranks: Vec<_> = src.commits_in(CommitRange::All).map(|c| c.rank).collect();
    let mut out: Vec<(String, Vec<String>)> = src
        .rows_for(ranks)
        .filter_map(|(rank, rows)| {
            let mut names: Vec<String> = rows
                .iter()
                .flat_map(|f| [Some(f.path), f.old])
                .flatten()
                .filter_map(|id| src.path(id))
                .filter_map(|p| name(&p))
                .collect();
            names.sort_unstable();
            names.dedup();
            let id = src.commit_id(rank)?.to_string();
            (!names.is_empty()).then_some((id, names))
        })
        .collect();
    out.reverse();
    out
}
