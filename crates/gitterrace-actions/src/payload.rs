//! The payload contract v1. The browser sends only `{action, kind, locator}`; everything the
//! script sees is read from git here, so a page cannot smuggle text the repo lacks and each
//! field has one source.

use std::collections::BTreeMap;

use gitterrace_git::{Repo, civil_from_days, percent_encode_path};
use serde::{Deserialize, Serialize};

use crate::config::Kind;
use crate::link::{LinkStyle, styled};

pub const PAYLOAD_VERSION: u32 = 1;
/// Prefix of the per-action var env names; no payload variable starts with it.
const VAR_PREFIX: &str = "GITTERRACE_VAR_";
/// `text` caps: past either one the script gets the head and `text_truncated`.
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_TEXT_LINES: usize = 400;
const MAX_SUBJECT_BYTES: usize = 1024;
/// Env values are capped so no field can exhaust the argument space.
const MAX_ENV_BYTES: usize = 4096;
/// Lines in the markdown snippet, the same cap the client-side copy uses.
const MARKDOWN_LINES: usize = 40;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lines {
    pub start: u32,
    pub end: u32,
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
    /// (for example the base URL of the default address) a `/go` permalink is rooted at; `style` picks the form.
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
            date: rfc3339(s.author.time),
        });
        let short = gitterrace_git::short_id(&sha);
        let id = repo.id();
        let permalink = styled(
            style,
            origin,
            id.as_deref(),
            &repo.name(),
            &sha,
            path.as_deref(),
            lines,
        );
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
        if let Some(p) = &self.path {
            put("GITTERRACE_PATH", p);
        }
        if let Some(l) = self.lines {
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

const fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Line => "line",
        Kind::File => "file",
        Kind::Commit => "commit",
        Kind::Hunk => "hunk",
        Kind::Blame => "blame",
        Kind::SearchHit => "search-hit",
        Kind::Insight => "insight",
        Kind::Page => "page",
    }
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

/// One line: control characters (newlines included) become spaces, then capped.
fn lossy_line(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    cap(&flat, max).0
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
        url = if l.start == l.end {
            format!("{url}#L{}", l.start)
        } else {
            format!("{url}#L{}-L{}", l.start, l.end)
        };
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

/// The same string the built-in "Copy as markdown" produces.
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
        let snippet = t
            .lines()
            .take(MARKDOWN_LINES)
            .collect::<Vec<_>>()
            .join("\n");
        // Longer than any backtick run inside, so repo text cannot close the fence.
        let longest = snippet.split(|c| c != '`').map(str::len).max().unwrap_or(0);
        let fence = "`".repeat(longest.max(2) + 1);
        md = format!("{md}{fence}{lang}\n{snippet}\n{fence}\n");
    }
    // The machine line comes last, so a reader finds it on the final line.
    format!("{md}{source}\n")
}

/// Seconds since the epoch as `YYYY-MM-DDTHH:MM:SSZ`.
fn rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
pub(crate) mod tests;
