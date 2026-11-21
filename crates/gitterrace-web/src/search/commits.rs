//! Commit search: from the index's message column when it is current, else by walking git.

use regex::Regex;

use crate::app::App;
use crate::error::Result;

/// The text a commit is matched on: its message, then the author as `name <email>`.
fn write_hay(out: &mut String, message: &str, name: &str, email: &str) {
    out.push_str(message);
    out.push('\n');
    out.push_str(name);
    out.push_str(" <");
    out.push_str(email);
    out.push('>');
}

pub(crate) fn search_commits(
    a: &App,
    rev: &str,
    re: &Regex,
) -> Result<(Vec<gitterrace_git::CommitSummary>, bool)> {
    let most = a.cfg().limits.search_commits;
    if let Some(hits) = search_indexed(a, rev, re, most)? {
        let rows = a.repo.search_summaries(rev, &hits.ids)?;
        return Ok((rows, hits.capped));
    }
    search_commits_counted(a, rev, re).map(|(rows, capped, _)| (rows, capped))
}

/// Commit search from the index's message column, the same hits in the same order as the
/// walk. `None` when the column is off or not current for `rev`'s commit: the walk answers.
/// An unreadable column starts the rebuild, logged and shown like every storage error, and
/// this one request walks git.
pub(crate) fn search_indexed(
    a: &App,
    rev: &str,
    re: &Regex,
    most: usize,
) -> Result<Option<gitterrace_index::messages::MessageHits>> {
    if !a.index.message_index() {
        return Ok(None);
    }
    let tip = a.repo.resolve(rev)?;
    let mailmap = a.repo.mailmap_at(rev)?;
    let author = |name: &[u8], email: &[u8]| mailmap.show(name, email);
    let is_match = |hay: &str| re.is_match(hay);
    let in_git = |id: &str| -> std::io::Result<bool> {
        let mut hit = false;
        gitterrace_git::scan_commits(
            &a.repo,
            id,
            |message, name, email| {
                let mut hay = String::new();
                write_hay(&mut hay, message, name, email);
                hit = re.is_match(&hay);
                false
            },
            |_, _| false,
        )
        .map_err(std::io::Error::other)?;
        Ok(hit)
    };
    let query = gitterrace_index::messages::MessageQuery {
        is_match: &is_match,
        author: &author,
        in_git: &in_git,
    };
    match a.index.search_messages(&tip, &query, most) {
        Ok(hits) => Ok(hits),
        Err(e) => match gitterrace_index::feed::SourceError::of(&e) {
            Some(s) => {
                a.unreadable(&s)?;
                Ok(None)
            }
            None => Err(e.into()),
        },
    }
}

/// Commit search by walking git, with the work the walk did, so a test can hold a miss to the
/// cost of the walk alone: only a matching commit has its summary built.
pub(crate) fn search_commits_counted(
    a: &App,
    rev: &str,
    re: &Regex,
) -> Result<(Vec<gitterrace_git::CommitSummary>, bool, gitterrace_git::CommitScan)> {
    let mut rows = Vec::new();
    let mut capped = false;
    let most = a.cfg().limits.search_commits;
    let mut hay = String::new();
    let scan = gitterrace_git::scan_commits(
        &a.repo,
        rev,
        |message, name, email| {
            hay.clear();
            write_hay(&mut hay, message, name, email);
            re.is_match(&hay)
        },
        |c, _| {
            if rows.len() >= most {
                capped = true;
                return false;
            }
            rows.push(c);
            true
        },
    )?;
    Ok((rows, capped, scan))
}
