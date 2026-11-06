//! Which entries of a directory each commit touched.

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
