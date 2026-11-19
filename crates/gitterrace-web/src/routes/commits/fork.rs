//! The fork link on a branch's log.

use crate::app::App;
use crate::pagehead::Action;

/// The "since it left <default branch>" link of a branch's log: the compare page from the
/// commit the branch forked at to its tip. `None` for the default branch itself, for a rev that
/// is no local branch, and when the histories share nothing.
pub(super) fn since_left(a: &App, rev: &str, id: &str) -> Option<Action> {
    let default = a.repo.head_branch().filter(|d| d != rev)?;
    let is_branch = a
        .repo
        .refs()
        .ok()?
        .iter()
        .any(|r| r.kind == gitterrace_git::RefKind::Branch && r.name == rev);
    let base = a
        .repo
        .merge_base(&default, id)
        .ok()
        .flatten()
        .filter(|_| is_branch)?;
    let href = crate::compare::compare_href(Some(&base), rev);
    (!href.is_empty()).then_some(Action {
        label: "Since it left the default branch",
        href,
    })
}
