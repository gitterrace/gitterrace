//! The sidebar's lists: linked worktrees, branches left behind and the language bar.

use std::collections::HashMap;

use gitterrace_config::Feature;

use crate::{
    app::App,
    fmt::{self, enc_seg, href},
    overview_side::{Lang, Loose, path_filter},
};

const TOP_LOOSE: usize = 3;
const TOP_LANGS: usize = 4;

/// Linked worktrees to list, none while `features.worktree_enabled` is off.
pub(super) fn worktree_count(a: &App) -> usize {
    if crate::features::on(Feature::Worktree) {
        a.repo.linked_worktrees().map_or(0, |w| w.len())
    } else {
        0
    }
}

/// Local branches with commits HEAD lacks, newest tip first, and how many there are.
pub(super) fn loose_ends(
    a: &App,
    head: &str,
    current: Option<&str>,
    now: i64,
) -> (Vec<Loose>, usize) {
    let times: HashMap<String, i64> = a
        .repo
        .refs()
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.kind == gitterrace_git::RefKind::Branch)
        .map(|r| (r.name, r.time))
        .collect();
    let mut open: Vec<_> = a
        .repo
        .branch_status(head)
        .unwrap_or_default()
        .into_iter()
        .filter(|b| !b.merged() && Some(b.name.as_str()) != current)
        .map(|b| (times.get(&b.name).copied().unwrap_or(0), b))
        .collect();
    open.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.name.cmp(&y.1.name)));
    let total = open.len();
    let base = current.unwrap_or("HEAD");
    let rows = open
        .into_iter()
        .take(TOP_LOOSE)
        .map(|(t, b)| Loose {
            href: if crate::features::on(Feature::Compare) {
                crate::compare::compare_href(Some(base), &b.name)
            } else {
                href(&format!("/log/{}", enc_seg(&b.name)))
            },
            tip: format!("{} ahead, {} behind", fmt::count(b.ahead, "commit", "commits"), b.behind),
            when: fmt::ago(t, now),
            name: b.name,
        })
        .collect();
    (rows, total)
}

/// The top languages by lines at the tip, with `Other` for the rest.
pub(super) fn langs(a: &App, head: &str) -> Vec<Lang> {
    // The insights filter (excludes, generated, ignored) and only named languages: a bar of
    // `.test` and `.prebuilt` says nothing about the code.
    let filter = path_filter(a, head);
    let mut by: HashMap<String, u64> = HashMap::new();
    for f in a.head_lines(head).iter().filter(|f| !filter.skips(&f.path)) {
        let l = gitterrace_index::language(&f.path);
        if l.starts_with('.') || l.starts_with('(') {
            continue;
        }
        *by.entry(l).or_default() += u64::from(f.lines);
    }
    let total: u64 = by.values().sum();
    if total == 0 {
        return Vec::new();
    }
    let mut v: Vec<_> = by.into_iter().filter(|(_, n)| *n > 0).collect();
    v.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    if v.len() > TOP_LANGS {
        let rest: u64 = v.drain(TOP_LANGS..).map(|(_, n)| n).sum();
        v.push(("Other".to_owned(), rest));
    }
    v.into_iter()
        .enumerate()
        .map(|(i, (name, n))| {
            #[expect(clippy::cast_precision_loss, reason = "counts far below 2^52")]
            let share = n as f64 * 100.0 / total as f64;
            Lang {
                pct: if share < 1.0 { "<1%".to_owned() } else { format!("{share:.0}%") },
                // Whole percent steps, at least 1 so a sliver stays visible.
                class: format!("c{} w{}", i + 1, (share.round() as u32).clamp(1, 100)),
                name,
            }
        })
        .collect()
}
