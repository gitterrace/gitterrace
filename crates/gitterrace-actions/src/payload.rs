//! The payload contract v1. The browser sends only `{action, kind, locator}`; everything the
//! script sees is read from git here, so a page cannot smuggle text the repo lacks and each
//! field has one source.

use std::collections::BTreeMap;

use gitterrace_git::{Repo, percent_encode_path};
use gitterrace_types::api::Finding;
use gitterrace_types::{AnchorStatus, LinkStyle};
use serde::{Deserialize, Serialize};

use crate::config::Kind;
use crate::link::styled;

pub const PAYLOAD_VERSION: u32 = 1;
/// Prefix of the per-action var env names; no payload variable starts with it.
const VAR_PREFIX: &str = "GITTERRACE_VAR_";
/// `text` caps: past either one the script gets the head and `text_truncated`.
/// Not configurable: payload v1 contract, scripts rely on these sizes.
pub const MAX_TEXT_BYTES: usize = crate::numbers::text_kib!() * 1024;
pub const MAX_TEXT_LINES: usize = crate::numbers::text_lines!();
const MAX_SUBJECT_BYTES: usize = 1024;
/// Env values are capped so no field can exhaust the argument space.
/// Not configurable: an OS bound on the argument and environment space.
const MAX_ENV_BYTES: usize = 4096;
/// Lines in the markdown snippet, the same cap the client-side copy uses.
const MARKDOWN_LINES: usize = 40;
/// Most findings one `review` payload carries, so the stdin and the markdown stay bounded.
/// Not configurable: a bound on what one action run is handed.
pub const MAX_REVIEW_FINDINGS: usize = 500;

/// The run request body. Strict: an unknown field (say `command`) is a 400, never ignored.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub action: String,
    pub kind: Kind,
    pub locator: Locator,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Locator {
    /// A served repo's name; the web layer resolves it.
    pub repo: String,
    pub rev: String,
    pub path: Option<String>,
    pub lines: Option<Lines>,
    pub sha: Option<String>,
    pub query: Option<String>,
    /// Kind `review` only: ids in the server's own basket. The notes are read from there, so a
    /// page cannot send text of its own.
    #[serde(default)]
    pub findings: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lines {
    pub start: u32,
    pub end: u32,
}

impl Lines {
    /// `#L<a>` for one line, `#L<a>-L<b>` for a span.
    pub(crate) fn fragment(self) -> String {
        if self.start == self.end {
            format!("#L{}", self.start)
        } else {
            format!("#L{}-L{}", self.start, self.end)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepoInfo {
    pub name: String,
    pub root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommitInfo {
    pub subject: String,
    pub author: String,
    pub date: String,
}

/// What the script reads on stdin. Fields that do not apply are absent, never null.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Payload {
    pub v: u32,
    pub action: String,
    pub kind: Kind,
    pub repo: RepoInfo,
    pub rev: String,
    pub sha: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<Lines>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    pub permalink: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<CommitInfo>,
    pub markdown: String,
    /// The action's `[action.vars]`, set by the runner from `actions.toml` only. Added in v1
    /// without a version bump: readers ignore fields they do not know.
    pub vars: BTreeMap<String, String>,
    /// Kind `review` only: the basket entries, in order. Absent for every other kind, so their
    /// bytes are what v1 always sent.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<ReviewFinding>,
}

/// One entry of a `review` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewFinding {
    pub n: usize,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<Lines>,
    pub sha: String,
    pub permalink: String,
    pub note: String,
    /// What became of the code. Never says whether the finding was dealt with.
    pub status: AnchorStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_truncated: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
pub enum PayloadError {
    /// The locator is malformed or names nothing in the repo: a 400, not a 500.
    #[error("bad locator: {0}")]
    BadLocator(String),
    #[error(transparent)]
    Git(#[from] gitterrace_git::Error),
}

fn bad(why: impl Into<String>) -> PayloadError {
    PayloadError::BadLocator(why.into())
}

impl Payload {
    /// Builds the payload for `action` from git. `origin` is the page origin
    /// (for example the base URL of the default address) a `/go` permalink is rooted at; `style`
    /// picks the form.
    pub fn build(
        repo: &Repo,
        action: &str,
        kind: Kind,
        loc: &Locator,
        origin: &str,
        style: LinkStyle,
    ) -> Result<Self, PayloadError> {
        let rev = loc.sha.as_deref().unwrap_or(&loc.rev);
        if rev.is_empty()
            || rev.len() > 256
            || rev.starts_with('-')
            || rev.chars().any(char::is_control)
        {
            return Err(bad("rev"));
        }
        if kind == Kind::Review {
            return Err(bad("a review is built from the basket, not from a locator"));
        }
        if loc.findings.is_some() {
            return Err(bad("findings belong to the review kind"));
        }
        let sha = repo.resolve(rev).map_err(|_| bad("unknown rev"))?;
        let path = match loc.path.as_deref().map(|p| p.trim_matches('/')) {
            None | Some("") => None,
            Some(p) => {
                if p.split('/').any(|c| c.is_empty() || c == "." || c == "..")
                    || !repo.has(&sha, Some(p))
                {
                    return Err(bad("path not in the tree at this commit"));
                }
                Some(p.to_string())
            }
        };
        let lines = match loc.lines {
            Some(l) if l.start == 0 || l.end < l.start => return Err(bad("lines")),
            Some(_) if path.is_none() => return Err(bad("lines without a path")),
            l => l,
        };
        let needs_path = matches!(kind, Kind::Line | Kind::File | Kind::Blame | Kind::Hunk);
        if needs_path && path.is_none() {
            return Err(bad("this kind needs a path"));
        }
        if kind == Kind::Line && lines.is_none() {
            return Err(bad("line needs lines"));
        }

        let (text, truncated) = match (&path, lines) {
            (Some(p), Some(l)) => {
                let blob = repo.blob(&sha, p)?;
                let Some(all) = blob.text else {
                    return Err(bad("binary or oversized file has no lines"));
                };
                let (t, cut) = slice_lines(&all, l)?;
                (Some(t), Some(cut))
            }
            _ => (None, None),
        };

        let detail = repo.commit(&sha)?;
        let s = &detail.summary;
        let commit = Some(CommitInfo {
            subject: cap(&s.subject, MAX_SUBJECT_BYTES).0,
            author: cap(&s.author.name, MAX_SUBJECT_BYTES).0,
            date: gitterrace_git::iso_utc(s.author.time).unwrap_or_default(),
        });
        let short = gitterrace_git::short_id(&sha);
        let id = repo.id();
        let permalink =
            styled(style, origin, id.as_deref(), &repo.name(), &sha, path.as_deref(), lines);
        let title = title(kind, path.as_deref(), lines, short, &s.subject);
        let markdown = markdown(
            path.as_deref(),
            lines,
            short,
            &permalink,
            text.as_deref(),
            commit.as_ref(),
            &crate::source::source_line(&repo.name(), &sha, path.as_deref(), lines),
        );

        Ok(Self {
            v: PAYLOAD_VERSION,
            action: action.to_string(),
            kind,
            repo: RepoInfo {
                name: repo.name(),
                root: repo.root().to_string_lossy().into_owned(),
                id,
            },
            rev: lossy_line(&loc.rev, 256),
            sha,
            path,
            lines,
            text,
            text_truncated: truncated,
            query: loc.query.as_deref().map(|q| cap(q, MAX_SUBJECT_BYTES).0),
            permalink,
            title,
            commit,
            markdown,
            vars: BTreeMap::new(),
            findings: Vec::new(),
        })
    }

    /// The `review` payload of `findings`, the server's own basket entries with their anchors.
    /// Each entry's text is read from git at its commit, capped at the markdown line cap, so
    /// nothing a page sends reaches a script.
    pub fn build_review(
        repo: &Repo,
        action: &str,
        findings: &[Finding],
        origin: &str,
        style: LinkStyle,
    ) -> Result<Self, PayloadError> {
        let Some(first) = findings.first() else {
            return Err(bad("no findings to send"));
        };
        if findings.len() > MAX_REVIEW_FINDINGS {
            return Err(bad(format!("more than {MAX_REVIEW_FINDINGS} findings: send fewer")));
        }
        let id = repo.id();
        let name = repo.name();
        let mut entries = Vec::with_capacity(findings.len());
        for (i, f) in findings.iter().enumerate() {
            let lines = f.lines.map(|l| Lines { start: l.start, end: l.end });
            let permalink =
                styled(style, origin, id.as_deref(), &name, &f.sha, f.path.as_deref(), lines);
            let (text, truncated) = finding_text(repo, f, lines);
            entries.push(ReviewFinding {
                n: i + 1,
                kind: f.kind.clone(),
                path: f.path.clone(),
                lines,
                sha: f.sha.clone(),
                permalink,
                note: f.note.clone(),
                status: f.anchor.status,
                text,
                text_truncated: truncated,
            });
        }
        let short = gitterrace_git::short_id(&first.sha);
        let count = entries.len();
        let title = format!("Review: {count} finding{}", if count == 1 { "" } else { "s" });
        let markdown = review_markdown(&name, short, &entries);
        Ok(Self {
            v: PAYLOAD_VERSION,
            action: action.to_string(),
            kind: Kind::Review,
            repo: RepoInfo { name, root: repo.root().to_string_lossy().into_owned(), id },
            rev: lossy_line(&first.rev, 256),
            sha: first.sha.clone(),
            path: None,
            lines: None,
            text: None,
            text_truncated: None,
            query: None,
            permalink: entries[0].permalink.clone(),
            title,
            commit: None,
            markdown,
            vars: BTreeMap::new(),
            findings: entries,
        })
    }

    /// The `GITTERRACE_*` variables: newlines stripped, each capped, no text or subject.
    pub fn env(&self) -> BTreeMap<&'static str, String> {
        let mut e = BTreeMap::new();
        let mut put = |k, v: &str| {
            e.insert(k, lossy_line(v, MAX_ENV_BYTES));
        };
        put("GITTERRACE_PAYLOAD_VERSION", &self.v.to_string());
        put("GITTERRACE_ACTION", &self.action);
        put("GITTERRACE_KIND", kind_name(self.kind));
        put("GITTERRACE_REPO", &self.repo.name);
        put("GITTERRACE_ROOT", &self.repo.root);
        put("GITTERRACE_SHA", &self.sha);
        put("GITTERRACE_TITLE", &self.title);
        put("GITTERRACE_URL", &self.permalink);
        // A review has no top-level selection: the path and lines are the first finding's.
        let first = self.findings.first();
        if first.is_some() {
            put("GITTERRACE_FINDINGS", &self.findings.len().to_string());
        }
        if let Some(p) = self.path.as_ref().or_else(|| first.and_then(|f| f.path.as_ref())) {
            put("GITTERRACE_PATH", p);
        }
        if let Some(l) = self.lines.or_else(|| first.and_then(|f| f.lines)) {
            put("GITTERRACE_LINE_START", &l.start.to_string());
            put("GITTERRACE_LINE_END", &l.end.to_string());
        }
        e
    }

    /// The value a `{name}` placeholder in `command` expands to; `None` for an unknown name.
    /// A field the selection does not have (no path on a commit) expands to "".
    pub fn placeholder(&self, name: &str) -> Option<String> {
        let opt = |v: &Option<String>| v.clone().unwrap_or_default();
        let line = |f: fn(Lines) -> u32| self.lines.map(|l| f(l).to_string()).unwrap_or_default();
        Some(match name {
            "title" => self.title.clone(),
            "markdown" => self.markdown.clone(),
            "text" => opt(&self.text),
            "path" => opt(&self.path),
            "line_start" => line(|l| l.start),
            "line_end" => line(|l| l.end),
            "sha" => self.sha.clone(),
            "url" => self.permalink.clone(),
            "repo" => self.repo.name.clone(),
            "root" => self.repo.root.clone(),
            "kind" => kind_name(self.kind).to_string(),
            _ => self.vars.get(name.strip_prefix("var.")?)?.clone(),
        })
    }

    /// `GITTERRACE_VAR_<NAME>` for each var. Names are `[a-z0-9_]`, so the prefix keeps them
    /// out of the payload variables, `PATH` and `HOME`; values are passed verbatim.
    pub fn var_env(&self) -> BTreeMap<String, String> {
        self.vars
            .iter()
            .map(|(k, v)| (format!("{VAR_PREFIX}{}", k.to_ascii_uppercase()), v.clone()))
            .collect()
    }
}

/// The kebab-case name of `k`, as written in `actions.toml` and the payload.
pub const fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Line => "line",
        Kind::File => "file",
        Kind::Commit => "commit",
        Kind::Hunk => "hunk",
        Kind::Blame => "blame",
        Kind::SearchHit => "search-hit",
        Kind::Insight => "insight",
        Kind::Page => "page",
        Kind::Review => "review",
    }
}

/// The text of one finding's lines at its own commit, under the markdown line cap. `None` for a
/// finding without lines, or whose file is binary, too large or not at that commit: the entry
/// still goes, it just has no text.
fn finding_text(repo: &Repo, f: &Finding, lines: Option<Lines>) -> (Option<String>, Option<bool>) {
    let (Some(path), Some(l)) = (f.path.as_deref(), lines) else {
        return (None, None);
    };
    let Some(all) = repo.blob(&f.sha, path).ok().and_then(|b| b.text) else {
        return (None, None);
    };
    let Ok((t, cut)) = slice_lines(&all, l) else {
        return (None, None);
    };
    let total = t.lines().count();
    let kept: Vec<&str> = t.lines().take(MARKDOWN_LINES).collect();
    let cut = cut || total > kept.len();
    (Some(kept.join("\n") + "\n"), Some(cut))
}

/// What the numbered line of one finding calls it: the path and range, or the kind and commit.
fn finding_label(f: &ReviewFinding) -> String {
    match (&f.path, f.lines) {
        (Some(p), Some(l)) => format!("{p}{}", l.fragment()),
        (Some(p), None) => p.clone(),
        (None, _) => format!("{} {}", f.kind, gitterrace_git::short_id(&f.sha)),
    }
}

/// Appends `line` (the formatting is done by the caller, so no `format!` feeds `push_str`).
fn put(md: &mut String, line: &str) {
    md.push_str(line);
}

/// `**Review of N findings** @ <short>` and a numbered list: label, status and note, then the
/// permalink, the text when there is some, and the one `source:` line of that finding. A note
/// is flattened to one line so it cannot start a line of its own, and a fence is longer than
/// any backtick run in the text, as in v1.
fn review_markdown(repo: &str, short: &str, entries: &[ReviewFinding]) -> String {
    let n = entries.len();
    let mut md = format!("**Review of {n} finding{}** @ {short}\n", if n == 1 { "" } else { "s" });
    for f in entries {
        let note = flat(&f.note);
        put(&mut md, &format!("{}. **{}** [{}]", f.n, finding_label(f), f.status));
        if !note.trim().is_empty() {
            md.push_str(": ");
            md.push_str(note.trim());
        }
        put(&mut md, &format!("\n   {}\n", f.permalink));
        if let Some(t) = &f.text {
            let lang = f.path.as_deref().and_then(|p| p.rsplit_once('.')).map_or("", |(_, e)| e);
            let longest = t.split(|c| c != '`').map(str::len).max().unwrap_or(0);
            let fence = "`".repeat(longest.max(2) + 1);
            put(&mut md, &format!("   {fence}{}\n", flat(lang)));
            for line in t.lines() {
                md.push_str("   ");
                md.push_str(line);
                md.push('\n');
            }
            put(&mut md, &format!("   {fence}\n"));
        }
        let src = crate::source::source_line(repo, &f.sha, f.path.as_deref(), f.lines);
        put(&mut md, &format!("   {src}\n"));
    }
    md
}

/// Lines `start..=end` (1-based) under both caps. A range past the end of the file is an error.
fn slice_lines(all: &str, l: Lines) -> Result<(String, bool), PayloadError> {
    let total = all.lines().count();
    if l.start as usize > total {
        return Err(bad("lines past the end of the file"));
    }
    let end = (l.end as usize).min(total);
    let want = end + 1 - l.start as usize;
    let take = want.min(MAX_TEXT_LINES);
    let mut out = String::new();
    for line in all.lines().skip(l.start as usize - 1).take(take) {
        out.push_str(line);
        out.push('\n');
    }
    let (out, cut) = cap(&out, MAX_TEXT_BYTES);
    Ok((out, cut || take < want))
}

/// At most `max` bytes, cut on a char boundary.
fn cap(s: &str, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s.to_string(), false);
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

/// `s` with every control character (newlines included) turned into a space.
pub(crate) fn flat(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// One line: control characters (newlines included) become spaces, then capped.
fn lossy_line(s: &str, max: usize) -> String {
    cap(&flat(s), max).0
}

/// `/go/<id>/<sha>[/<path>][#L<a>[-L<b>]]`.
///
/// A repo without an id (no root commit) cannot
/// have a sha, so the name branch exists only for completeness. Shared with the API, which
/// passes an empty `origin` to get the path form.
pub fn permalink(
    origin: &str,
    id: Option<&str>,
    name: &str,
    sha: &str,
    path: Option<&str>,
    lines: Option<Lines>,
) -> String {
    let mut url = format!(
        "{}/go/{}/{sha}",
        origin.trim_end_matches('/'),
        percent_encode_path(id.unwrap_or(name))
    );
    if let Some(p) = path {
        url.push('/');
        url.push_str(&percent_encode_path(p));
    }
    if let Some(l) = lines {
        url.push_str(&l.fragment());
    }
    url
}

fn file_ref(path: &str, lines: Option<Lines>) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    match lines {
        Some(l) if l.start == l.end => format!("{base}:{}", l.start),
        Some(l) => format!("{base}:{}-{}", l.start, l.end),
        None => base.to_string(),
    }
}

fn title(
    kind: Kind,
    path: Option<&str>,
    lines: Option<Lines>,
    short: &str,
    subject: &str,
) -> String {
    let t = match (kind, path) {
        (Kind::Commit, _) | (_, None) => format!("{short} {subject}"),
        (_, Some(p)) => format!("{} @ {short}", file_ref(p, lines)),
    };
    lossy_line(&t, 200)
}

fn markdown(
    path: Option<&str>,
    lines: Option<Lines>,
    short: &str,
    link: &str,
    text: Option<&str>,
    commit: Option<&CommitInfo>,
    source: &str,
) -> String {
    let Some(p) = path else {
        let (subject, author) = commit.map_or(("", ""), |c| (&c.subject[..], &c.author[..]));
        return format!("**{subject}** @ {short}\n{author}\n{link}\n{source}\n");
    };
    let mut md = format!("**{}** @ {short}\n{link}\n", file_ref(p, lines));
    if let Some(t) = text {
        let lang = p.rsplit_once('.').map_or("", |(_, ext)| ext);
        let snippet = t.lines().take(MARKDOWN_LINES).collect::<Vec<_>>().join("\n");
        // Longer than any backtick run inside, so repo text cannot close the fence.
        let longest = snippet.split(|c| c != '`').map(str::len).max().unwrap_or(0);
        let fence = "`".repeat(longest.max(2) + 1);
        md = format!("{md}{fence}{lang}\n{snippet}\n{fence}\n");
    }
    // The machine line comes last, so a reader finds it on the final line.
    format!("{md}{source}\n")
}

#[cfg(test)]
pub(crate) mod tests;
