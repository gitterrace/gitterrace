//! The index page's read of one repository: newest commit and the sparkline, memoised by HEAD.

use std::sync::Arc;

use gitterrace_git::{CommitSummary, DAY, Repo};

use crate::{multi::Served, spark};

/// The index sparkline's longest window, half a year.
/// Not configurable: a chart definition (the span a sparkline shows), not a resource limit.
pub(super) const SPARK_CAP: i64 = 26 * spark::WEEK;

/// What the index shows for one repository: its newest commit and its recent commits,
/// bucketed to fit its age.
#[derive(Default)]
pub(super) struct Recent {
    pub(super) last: Option<CommitSummary>,
    pub(super) series: spark::Series,
    /// Commits since the local midnight of the newest commit's zone.
    pub(super) today: u32,
    /// The end of the local day `today` counts, past which the read is stale.
    pub(super) day_end: i64,
}

impl Served {
    /// The newest commit and the sparkline, one discover and one bounded walk per HEAD. The
    /// walk stops at the window's cap, so it counts every commit in it however busy the
    /// repository is. The repository's start comes from the same walk: when it dropped no
    /// parent the whole history is inside the cap, else the window is capped.
    pub(super) fn recent(&self, now: i64) -> Arc<Recent> {
        let Ok(repo) = Repo::discover(&self.path) else {
            return Arc::default();
        };
        let Ok(head) = repo.resolve("HEAD") else {
            return Arc::default();
        };
        if let Ok(g) = self.recent.lock()
            && let Some((at, r)) = g.as_ref()
            && *at == head
            && now < r.day_end
        {
            return Arc::clone(r);
        }
        let since = now - SPARK_CAP;
        let (times, first) = repo.commit_times_since(&head, since).unwrap_or_default();
        // HEAD resolved, so there is history: when it reaches past the cap, cap the window.
        let first = first.or(Some(since - 1));
        let last = repo
            .log(&head, None, 0, 1)
            .ok()
            .and_then(|l| l.into_iter().next());
        // Days end at local midnight of the newest commit's zone, as on the sidebar.
        let offset = last.as_ref().map_or(0, |c| c.author.offset);
        let day_end = spark::End::local(now, offset).day_end;
        let today =
            u32::try_from(times.iter().filter(|t| **t >= day_end - DAY).count()).unwrap_or(0);
        let r = Arc::new(Recent {
            today,
            day_end,
            series: spark::series(times, first, spark::End::local(now, offset), SPARK_CAP),
            last,
        });
        if let Ok(mut g) = self.recent.lock() {
            *g = Some((head, Arc::clone(&r)));
        }
        r
    }
}
