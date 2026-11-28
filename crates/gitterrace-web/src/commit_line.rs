//! The one commit row: avatar, subject, author and time, then the commit id, in that order on
//! every list of commits (the log, search, file history, compare, commit types) and on the refs
//! and tag pages, where a ref name leads the row. Markup in `commit_line.html`. No template or
//! other module writes the row by hand, so a commit reads the same wherever it is listed.

use askama::Template;
use askama::filters::HtmlSafe;
use gitterrace_git::CommitSummary;

use crate::fmt::{ago_at, href, sha_link};
use crate::views::{author_href, hue, initials};

/// The ref that leads a row on the refs and tag pages: its name as a link and its badges.
#[derive(Debug, Clone, Default)]
pub(crate) struct Lead {
    pub(crate) name: String,
    pub(crate) href: String,
    pub(crate) badges: Vec<&'static str>,
}

/// A number at the end of a row (a branch's ahead and behind), linked when `href` is set.
#[derive(Debug, Clone)]
pub(crate) struct Count {
    pub(crate) label: &'static str,
    pub(crate) value: String,
    pub(crate) href: String,
    pub(crate) tip: &'static str,
}

#[derive(Debug, Clone, Default, Template)]
#[template(path = "commit_line.html")]
pub(crate) struct CommitRow {
    /// The commit id as its link, finished HTML from `fmt::sha_link`.
    pub(crate) sha: String,
    pub(crate) initial: String,
    pub(crate) hue: u32,
    pub(crate) href: String,
    pub(crate) subject: String,
    pub(crate) author: String,
    /// Commit list narrowed to this author, matched by (mailmapped) email.
    pub(crate) author_href: String,
    pub(crate) when: crate::when::When,
    pub(crate) lead: Option<Lead>,
    /// Muted text after the time, such as the lines a file history row changed.
    pub(crate) note: String,
    pub(crate) counts: Vec<Count>,
    /// The Graph view's cell; `None` in every other list.
    pub(crate) graph: Option<crate::graph_view::Cell>,
    /// Refs pointing at the commit, shown as chips in the Graph view: `(name, is HEAD's)`.
    pub(crate) refs: Vec<(String, bool)>,
}

impl CommitRow {
    pub(crate) fn of(c: &CommitSummary, now: i64) -> Self {
        Self {
            sha: sha_link(&c.id),
            initial: initials(&c.author.name),
            hue: hue(&c.author.name),
            href: href(&format!("/commit/{}", c.id)),
            subject: c.subject.clone(),
            author: c.author.name.clone(),
            author_href: author_href(&c.author.email),
            when: ago_at(c.author.time, c.author.offset, now),
            ..Self::default()
        }
    }

    /// A label's name cut after each slash and underscore, the break points a browser lacks for
    /// `_`: the template puts a `<wbr>` between the parts so a word is never split mid-way.
    #[expect(clippy::unused_self, reason = "a template reaches it as a method of the row")]
    fn label_parts<'a>(&self, name: &'a str) -> Vec<&'a str> {
        name.split_inclusive(['/', '_']).collect()
    }

    /// Puts `lead` before the commit, for a row that lists a ref.
    #[must_use]
    pub(crate) fn led_by(mut self, lead: Lead) -> Self {
        self.lead = Some(lead);
        self
    }

    #[must_use]
    pub(crate) fn note(mut self, note: String) -> Self {
        self.note = note;
        self
    }
}

impl HtmlSafe for CommitRow {}

#[cfg(test)]
mod tests {
    use gitterrace_git::Signature;

    use super::*;

    fn summary() -> CommitSummary {
        let who = Signature {
            name: "Ada <x>".to_owned(),
            email: "ada@example.com".to_owned(),
            time: 1_700_000_000,
            offset: 0,
        };
        CommitSummary {
            id: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            parents: Vec::new(),
            author: who.clone(),
            committer: who,
            subject: "fix: a < b".to_owned(),
        }
    }

    #[test]
    fn a_row_shows_avatar_subject_author_time_then_id() {
        let s = CommitRow::of(&summary(), 1_700_000_100).to_string();
        let order =
            ["class=\"av ", "class=\"subj\"", "class=\"who\"", "datetime=", "class=\"sha\""];
        let at: Vec<usize> = order.iter().map(|m| s.find(m).expect(m)).collect();
        assert!(at.windows(2).all(|w| w[0] < w[1]), "{s}");
        assert!(!s.contains("a < b") && !s.contains("Ada <x>"), "{s}");
    }

    #[test]
    fn a_ref_leads_and_counts_close_the_row() {
        let mut r = CommitRow::of(&summary(), 0).led_by(Lead {
            name: "main".to_owned(),
            href: "/log/main".to_owned(),
            badges: vec!["default"],
        });
        r.counts.push(Count {
            label: "ahead",
            value: "3".to_owned(),
            href: "/compare?base=a&head=b".to_owned(),
            tip: "Commits ahead",
        });
        let s = r.to_string();
        let lead = s.find("class=\"ref\"").expect("lead");
        assert!(lead < s.find("class=\"av ").expect("avatar"), "{s}");
        assert!(s.contains("<span class=\"tag default\">default</span>"), "{s}");
        assert!(s.find("class=\"cnums\"").expect("counts") > s.find("class=\"sha\"").expect("id"));
    }

    #[test]
    fn a_graph_label_may_break_only_after_a_slash_or_underscore() {
        let rows = vec![("a".to_owned(), Vec::new())];
        let placed = crate::graph_view::place(&rows, 0, 1);
        let mut r = CommitRow::of(&summary(), 0);
        r.graph = Some(crate::graph_view::cell(&placed[0], None, 1, false));
        r.refs.push(("feature/long_name-x".to_owned(), false));
        let s = r.to_string();
        assert!(s.contains(">feature/<wbr>long_<wbr>name-x</span>"), "{s}");
        assert!(s.contains("title=\"feature/long_name-x\""), "{s}");
    }
}
