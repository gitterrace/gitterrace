//! The card of one file or directory, see [`ApiState::file_insight`].

use gitterrace_index::{PathCard, PathKind};
use gitterrace_types::api::{
    FileInsight, FileInsightAge, FileInsightChild, FileInsightChurn, FileInsightKind,
    FileInsightOwner, FileInsightPartner,
};

use crate::bounds::checked_path;
use crate::core::ApiState;
use crate::error::ApiError;
use crate::insights;

impl ApiState {
    /// The card of `path` at `rev`: churn rank and sparkline, owners, partners, age. Counted as
    /// the `hotspots`, `ownership` and coupling reports count under the same query, and equal
    /// to them narrowed to the path. A path the tree lacks answers an empty `missing` card.
    pub fn file_insight(
        &self,
        rev: Option<&str>,
        path: &str,
        params: &insights::Params,
    ) -> Result<FileInsight, ApiError> {
        let path = checked_path(path)?.trim_matches('/');
        let rev = Self::rev(rev)?;
        let tip = self.repo.resolve(rev)?;
        let scope = insights::scope(params, self.settings().defaults)?;
        let card = self.read(rev, |all| {
            let ins = self.insights(&tip, all, scope.options().clone());
            let counted = insights::counted(&ins, &scope, &**all);
            ins.path_card(&self.head_lines(&tip), &counted, path)
        })?;
        let median = (card.kind == PathKind::File).then(|| self.median_line_age(&tip, path));
        let (permalink, link) = self.links(&tip, (!path.is_empty()).then_some(path), None);
        Ok(card_response(card, median.flatten(), path, rev, (permalink, link)))
    }

    /// Median age in seconds of the lines of `path` now, by blame, against the newest commit
    /// time the blame names. `None` when blame cannot read the file.
    fn median_line_age(&self, tip: &str, path: &str) -> Option<i64> {
        let lines = self.repo.blame(tip, path).ok()?;
        let newest = lines.iter().map(|l| l.time).max()?;
        let mut ages: Vec<i64> = lines.iter().map(|l| newest.saturating_sub(l.time)).collect();
        ages.sort_unstable();
        ages.get(ages.len() / 2).copied()
    }
}

fn card_response(
    card: PathCard,
    median_line_age_secs: Option<i64>,
    path: &str,
    rev: &str,
    (permalink, link): (Option<String>, Option<String>),
) -> FileInsight {
    FileInsight {
        age: FileInsightAge { last_change: card.last_change, median_line_age_secs },
        authors_90d: card.authors_90d,
        bus_factor: card.bus_factor,
        churn: FileInsightChurn {
            changes: card.changes,
            of: card.of,
            lines_changed: card.lines_changed,
            rank: card.rank,
            series: card.series,
        },
        hot: card.hot,
        kind: match card.kind {
            PathKind::File => FileInsightKind::File,
            PathKind::Dir => FileInsightKind::Dir,
            PathKind::Missing => FileInsightKind::Missing,
        },
        link,
        owners: card
            .owners
            .into_iter()
            .map(|o| FileInsightOwner { name: o.name, share: o.share })
            .collect(),
        partners: card
            .partners
            .into_iter()
            .map(|p| FileInsightPartner { degree: p.degree, path: p.path, together: p.together })
            .collect(),
        path: path.to_owned(),
        permalink,
        paths: card
            .children
            .into_iter()
            .map(|c| FileInsightChild {
                changes: c.changes,
                hot: c.hot,
                path: c.path,
                rank: c.rank,
            })
            .collect(),
        rev: rev.to_owned(),
    }
}
