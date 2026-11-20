//! One page of the log under a filter: from the index when it is ready, else by a bounded
//! walk of git.
//!
//! With the index, the author postings and the commit columns answer every keyword without
//! reading git; message words read the message column, or git for the candidates only. The walk
//! reads at most [`WALK_CAP`] commits and says so when it stopped there.

use gitterrace_git::CommitSummary;
use gitterrace_index::feed::{CommitRange, IdentId, InsightsSource, Posting, Rank};
use gitterrace_index::query::{Commit, Matcher, by};

use crate::app::App;
use crate::error::Result;
use crate::routes::PAGE;

/// Commits per read of the walk.
const CHUNK: usize = 500;
/// Commits the walk reads before it gives up with what it has. Not configurable: it only bounds
/// a log served before the index is built, which then answers every filter without a walk.
pub(crate) const WALK_CAP: usize = 2_000;

/// One page of a filtered log.
#[derive(Debug, Default)]
pub(crate) struct LogPage {
    pub(crate) list: Vec<CommitSummary>,
    pub(crate) more: bool,
    /// The walk stopped at [`WALK_CAP`] commits: older matches may exist.
    pub(crate) capped: bool,
    pub(crate) work: Work,
}

/// What answering the page cost: the counted-work bound a test holds it to.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Work {
    pub(crate) rows: usize,
    pub(crate) walked: usize,
}

/// Page `pg` of the commits from `id` that `m` keeps, newest first.
pub(crate) fn page(a: &App, id: &str, m: &Matcher, pg: usize) -> Result<LogPage> {
    let from = pg.saturating_mul(PAGE);
    if let Some((ranks, rows)) = a.accelerate(id, |s| indexed(&**s, m))? {
        let ids: Vec<String> = a
            .accelerate(id, |s| {
                ranks.iter().rev().filter_map(|r| s.commit_id(*r).map(|x| x.to_string())).collect()
            })?
            .unwrap_or_default();
        let (ids, walked) = with_words(a, m, ids, from + PAGE + 1)?;
        let more = ids.len() > from + PAGE;
        let slice: Vec<String> = ids.into_iter().skip(from).take(PAGE).collect();
        return Ok(LogPage {
            list: a.repo.summaries(&slice)?,
            more,
            capped: false,
            work: Work { rows, walked },
        });
    }
    walk(a, id, m, from)
}

/// The ranks `m` keeps by its column and path terms, oldest first, and the rows read.
/// An author term starts from its postings, so no other commit is looked at.
pub(crate) fn indexed(src: &dyn InsightsSource, m: &Matcher) -> (Vec<Rank>, usize) {
    let now = gitterrace_index::feed::reference_time(src);
    let cand: Vec<Rank> = m.authors().map_or_else(
        || {
            src.commits_in(CommitRange::All)
                .filter(|c| m.commit_row(src, c, now))
                .map(|c| c.rank)
                .collect()
        },
        |who| {
            by_authors(src, who)
                .into_iter()
                .filter(|r| src.commit(*r).is_some_and(|c| m.commit_row(src, &c, now)))
                .collect()
        },
    );
    if !m.has_path() {
        return (cand, 0);
    }
    let mut rows = 0;
    let kept = src
        .rows_for(cand)
        .filter(|(_, fs)| {
            rows += fs.len();
            fs.iter().any(|f| src.path(f.path).is_some_and(|n| m.row(&n)))
        })
        .map(|(r, _)| r)
        .collect();
    (kept, rows)
}

/// The ranks of the commits by any of `who`, oldest first, from the author postings.
fn by_authors(src: &dyn InsightsSource, who: &[String]) -> Vec<Rank> {
    let mut ranks: Vec<Rank> = (0..src.idents())
        .filter_map(|i| {
            let id = IdentId(u32::try_from(i).ok()?);
            let (name, email) = src.ident(id)?;
            who.iter().any(|w| by(w, &name, &email)).then_some(id)
        })
        .flat_map(|id| src.postings(Posting::Author(id)).map(|h| h.rank).collect::<Vec<_>>())
        .collect();
    ranks.sort_unstable();
    ranks.dedup();
    ranks
}

/// `ids` (newest first) narrowed by the message words, read from git for these commits only,
/// up to `want` hits.
fn with_words(a: &App, m: &Matcher, ids: Vec<String>, want: usize) -> Result<(Vec<String>, usize)> {
    if m.words().is_empty() {
        return Ok((ids, 0));
    }
    let mut out = Vec::new();
    let mut read = 0;
    for chunk in ids.chunks(CHUNK) {
        let texts = a.repo.messages(chunk)?;
        read += chunk.len();
        out.extend(chunk.iter().zip(texts).filter(|(_, t)| m.message(t)).map(|(id, _)| id.clone()));
        if out.len() >= want {
            break;
        }
    }
    Ok((out, read))
}

/// The git walk from `id`: the first path pushed into the walk, every other term tested on
/// each commit, at most [`WALK_CAP`] commits read.
fn walk(a: &App, id: &str, m: &Matcher, from: usize) -> Result<LogPage> {
    let want = from + PAGE + 1;
    let mut hits = Vec::new();
    let mut walked = 0;
    let mut capped = true;
    let path = m.single_path();
    while walked < WALK_CAP {
        let chunk = a.repo.log(id, path, walked, CHUNK)?;
        let n = chunk.len();
        walked += n;
        let texts = if m.words().is_empty() {
            Vec::new()
        } else {
            let ids: Vec<String> = chunk.iter().map(|c| c.id.clone()).collect();
            a.repo.messages(&ids)?
        };
        for (i, c) in chunk.into_iter().enumerate() {
            if keeps(a, m, &c, texts.get(i).map(String::as_str))? {
                hits.push(c);
            }
        }
        if hits.len() >= want || n < CHUNK {
            capped = false;
            break;
        }
    }
    let mut list: Vec<_> = hits.into_iter().skip(from).collect();
    let more = list.len() > PAGE;
    list.truncate(PAGE);
    Ok(LogPage { capped: capped && !more, list, more, work: Work { rows: 0, walked } })
}

/// Whether the walked commit `c` passes `m`. Paths beyond the one the walk filters by read
/// the commit's changed files.
fn keeps(a: &App, m: &Matcher, c: &CommitSummary, text: Option<&str>) -> Result<bool> {
    let (kind, scope) = gitterrace_index::parse_subject(&c.subject);
    let cols = Commit {
        kind: kind.as_deref(),
        scope: scope.as_deref(),
        name: &c.author.name,
        email: &c.author.email,
        time: c.author.time,
        merge: c.parents.len() > 1,
    };
    // A walk has no newest-commit clock of its own: a period counts back from now.
    if !m.commit(&cols, crate::fmt::now()) || !text.is_none_or(|t| m.message(t)) {
        return Ok(false);
    }
    // The walk filtered by the one path already; any other path term reads the files.
    if !m.has_path() || m.single_path().is_some() {
        return Ok(true);
    }
    let files = a.repo.commit(&c.id)?.files;
    Ok(files.iter().any(|f| m.row(&f.path)))
}
