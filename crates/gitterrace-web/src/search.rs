//! Code search across the tree at a revision and commit search over history.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};

use askama::Template;
use axum::{
    extract::{Query, State},
    response::Html,
};
use gitterrace_git::{BlobReader, FileRef};
use regex::{Regex, RegexBuilder};
use serde::Deserialize;

use crate::{
    app::{App, blocking},
    error::Result,
    fmt,
    views::{self, Chrome, CommitRow},
};

/// Matching lines kept per search before the rest is reported as cut off.
const MAX_PER_FILE: usize = 100;
const MAX_COMMITS: usize = 200;
/// Files above this are skipped: they are generated or vendored far more often than read.
const MAX_LINE_CHARS: usize = 300;

#[derive(Deserialize, Default)]
pub(crate) struct SearchQuery {
    q: Option<String>,
    rev: Option<String>,
    kind: Option<String>,
    regex: Option<String>,
    case: Option<String>,
    glob: Option<String>,
}

pub(crate) struct Line {
    pub(crate) n: usize,
    pub(crate) html: String,
    pub(crate) hit: bool,
}

struct FileHits {
    pub(crate) path: String,
    pub(crate) href: String,
    pub(crate) count: usize,
    pub(crate) lines: Vec<Line>,
}

// The flags mirror the form's checkboxes; the template reads them directly.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent switches, one per option"
)]
#[derive(Template)]
#[template(path = "search.html")]
struct SearchPage {
    c: Chrome,
    q: String,
    rev: String,
    commits_mode: bool,
    regex: bool,
    case: bool,
    glob: String,
    error: Option<String>,
    searched: bool,
    ms: u128,
    scanned: usize,
    skipped: usize,
    matches: usize,
    summary: String,
    capped: bool,
    files: Vec<FileHits>,
    commits: Vec<CommitRow>,
}

pub(crate) async fn search(
    State(app): State<Arc<App>>,
    Query(q): Query<SearchQuery>,
) -> Result<Html<String>> {
    let p = blocking(&app, move |a| run(a, q)).await?;
    Ok(Html(p.render()?))
}

fn run(a: &App, q: SearchQuery) -> Result<SearchPage> {
    let rev = q
        .rev
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| "HEAD".to_owned());
    let text = q.q.unwrap_or_default();
    let commits_mode = q.kind.as_deref() == Some("commits");
    let (regex, case) = (q.regex.is_some(), q.case.is_some());
    let glob = q.glob.unwrap_or_default();
    let mut page = SearchPage {
        c: views::chrome_at(a, "search", &rev),
        searched: !text.is_empty(),
        q: text.clone(),
        rev: rev.clone(),
        commits_mode,
        regex,
        case,
        glob: glob.clone(),
        error: None,
        ms: 0,
        scanned: 0,
        skipped: 0,
        matches: 0,
        summary: String::new(),
        capped: false,
        files: Vec::new(),
        commits: Vec::new(),
    };
    if text.is_empty() {
        return Ok(page);
    }
    let start = Instant::now();
    let re = match build_regex(&text, regex, case) {
        Ok(r) => r,
        Err(e) => {
            page.error = Some(e);
            return Ok(page);
        }
    };
    if commits_mode {
        let now = fmt::now();
        let (rows, capped) = search_commits(a, &rev, &re)?;
        page.matches = rows.len();
        page.capped = capped;
        page.commits = rows.iter().map(|c| views::commit_row(c, now)).collect();
    } else {
        let globs = match glob_regex(&glob) {
            Ok(g) => g,
            Err(e) => {
                page.error = Some(e);
                return Ok(page);
            }
        };
        let out = search_code(a, &rev, &re, globs.as_ref())?;
        page.scanned = out.scanned;
        page.skipped = out.skipped;
        page.matches = out.matches;
        page.capped = out.capped;
        page.files = out.files;
    }
    page.ms = start.elapsed().as_millis();
    let plus = if page.capped { "+" } else { "" };
    page.summary = if commits_mode {
        format!(
            "{}{plus} {}",
            fmt::thousands(page.matches),
            fmt::noun(page.matches, "commit", "commits")
        )
    } else {
        format!(
            "{}{plus} {} in {}, {} scanned, {} skipped (binary or over 2 MiB)",
            fmt::thousands(page.matches),
            fmt::noun(page.matches, "match", "matches"),
            fmt::count(page.files.len(), "file", "files"),
            fmt::count(page.scanned, "file", "files"),
            page.skipped
        )
    };
    Ok(page)
}

fn build_regex(q: &str, is_regex: bool, case: bool) -> std::result::Result<Regex, String> {
    let pat = if is_regex {
        q.to_owned()
    } else {
        regex::escape(q)
    };
    RegexBuilder::new(&pat)
        .case_insensitive(!case)
        .multi_line(true)
        .size_limit(8 * 1024 * 1024)
        .build()
        .map_err(|e| {
            e.to_string()
                .lines()
                .last()
                .unwrap_or("invalid pattern")
                .to_owned()
        })
}

/// Comma separated globs. Without a `/` a glob matches a file name at any depth.
fn glob_regex(glob: &str) -> std::result::Result<Option<Regex>, String> {
    let parts: Vec<&str> = glob
        .split(',')
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .collect();
    if parts.is_empty() {
        return Ok(None);
    }
    let mut alts = Vec::new();
    for g in parts {
        let g = if g.ends_with('/') {
            format!("{g}**")
        } else {
            g.to_owned()
        };
        let mut body = String::new();
        let mut chars = g.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '*' if chars.peek() == Some(&'*') => {
                    chars.next();
                    if chars.peek() == Some(&'/') {
                        chars.next();
                        body.push_str("(?:.*/)?");
                    } else {
                        body.push_str(".*");
                    }
                }
                '*' => body.push_str("[^/]*"),
                '?' => body.push_str("[^/]"),
                c => body.push_str(&regex::escape(&c.to_string())),
            }
        }
        alts.push(if g.contains('/') {
            format!("^{body}$")
        } else {
            format!("(?:^|/){body}$")
        });
    }
    Regex::new(&alts.join("|"))
        .map(Some)
        .map_err(|e| e.to_string())
}

struct CodeOut {
    files: Vec<FileHits>,
    matches: usize,
    scanned: usize,
    skipped: usize,
    capped: bool,
}

fn search_code(a: &App, rev: &str, re: &Regex, glob: Option<&Regex>) -> Result<CodeOut> {
    let id = a.repo.resolve(rev)?;
    let all = gitterrace_git::files(&a.repo, &id)?;
    let list: Vec<&FileRef> = all
        .iter()
        .filter(|f| glob.is_none_or(|g| g.is_match(&f.path)))
        .collect();
    let max_matches = a.cfg().limits.search_matches;
    let next = AtomicUsize::new(0);
    let total = AtomicUsize::new(0);
    let scanned = AtomicUsize::new(0);
    let skipped = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let found: Mutex<Vec<FileHits>> = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism()
        .map_or(2, usize::from)
        .min(8);
    let rev_enc = fmt::enc_seg(rev);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                let Ok(reader) = BlobReader::open(&a.repo) else {
                    return;
                };
                while !stop.load(Ordering::Relaxed) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(f) = list.get(i) else { break };
                    let Some(text) = reader.text(f.blob, a.repo.text_limit()) else {
                        skipped.fetch_add(1, Ordering::Relaxed);
                        continue;
                    };
                    scanned.fetch_add(1, Ordering::Relaxed);
                    if !re.is_match(&text) {
                        continue;
                    }
                    let mut hits = file_hits(&f.path, &rev_enc, &text, re);
                    if hits.count == 0 {
                        continue;
                    }
                    let room = max_matches.saturating_sub(total.load(Ordering::Relaxed));
                    if hits.count > room {
                        hits.count = room;
                    }
                    if total.fetch_add(hits.count, Ordering::Relaxed) + hits.count >= max_matches {
                        stop.store(true, Ordering::Relaxed);
                    }
                    if let Ok(mut g) = found.lock() {
                        g.push(hits);
                    }
                }
            });
        }
    });
    let mut files = found.into_inner().unwrap_or_default();
    files.sort_by(|x, y| x.path.cmp(&y.path));
    let matches = files.iter().map(|f| f.count).sum();
    Ok(CodeOut {
        files,
        matches,
        scanned: scanned.load(Ordering::Relaxed),
        skipped: skipped.load(Ordering::Relaxed),
        capped: stop.load(Ordering::Relaxed),
    })
}

/// Hit lines of one file, each with one line of context before and after.
fn file_hits(path: &str, rev_enc: &str, text: &str, re: &Regex) -> FileHits {
    let lines: Vec<&str> = text.lines().collect();
    let mut hit_idx = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if re.is_match(l) {
            hit_idx.push(i);
            if hit_idx.len() >= MAX_PER_FILE {
                break;
            }
        }
    }
    let mut out = Vec::new();
    let mut last: Option<usize> = None;
    for &h in &hit_idx {
        let from = h.saturating_sub(1).max(last.map_or(0, |l| l + 1));
        let to = (h + 1).min(lines.len().saturating_sub(1));
        if let Some(l) = last
            && from > l + 1
        {
            out.push(Line {
                n: 0,
                html: String::new(),
                hit: false,
            });
        }
        for (i, line) in lines.iter().enumerate().take(to + 1).skip(from) {
            let is_hit = i == h;
            out.push(Line {
                n: i + 1,
                html: if is_hit {
                    highlight(line, re)
                } else {
                    fmt::esc(&line.chars().take(MAX_LINE_CHARS).collect::<String>())
                },
                hit: is_hit,
            });
        }
        last = Some(to);
    }
    FileHits {
        href: fmt::href(&format!("/blob/{rev_enc}/{}", fmt::enc_path(path))),
        path: path.to_owned(),
        count: hit_idx.len(),
        lines: out,
    }
}

/// Escaped line with matches wrapped in `<mark>`.
pub(crate) fn highlight(line: &str, re: &Regex) -> String {
    let first = re.find(line).map_or(0..0, |m| m.range());
    let long = line.chars().count() > MAX_LINE_CHARS;
    let (view, shift) = if long {
        let start_chars = line[..first.start].chars().count().saturating_sub(40);
        let byte_start = line.char_indices().nth(start_chars).map_or(0, |(b, _)| b);
        let end = line[byte_start..]
            .char_indices()
            .nth(MAX_LINE_CHARS)
            .map_or(line.len(), |(b, _)| byte_start + b);
        (&line[byte_start..end], byte_start)
    } else {
        (line, 0)
    };
    let mut out = String::new();
    if shift > 0 {
        out.push_str("...");
    }
    let mut at = 0;
    for m in re.find_iter(view) {
        if m.start() == m.end() {
            continue;
        }
        out.push_str(&fmt::esc(&view[at..m.start()]));
        out.push_str("<mark>");
        out.push_str(&fmt::esc(m.as_str()));
        out.push_str("</mark>");
        at = m.end();
    }
    out.push_str(&fmt::esc(&view[at..]));
    if shift + view.len() < line.len() {
        out.push_str("...");
    }
    out
}

fn search_commits(
    a: &App,
    rev: &str,
    re: &Regex,
) -> Result<(Vec<gitterrace_git::CommitSummary>, bool)> {
    let mut rows = Vec::new();
    let mut capped = false;
    gitterrace_git::scan_commits(&a.repo, rev, |c, message| {
        let hay = format!("{message}\n{} <{}>", c.author.name, c.author.email);
        if re.is_match(&hay) {
            if rows.len() >= MAX_COMMITS {
                capped = true;
                return false;
            }
            rows.push(c.clone());
        }
        true
    })?;
    Ok((rows, capped))
}

#[cfg(test)]
mod tests {
    use crate::search::{build_regex, file_hits, glob_regex, highlight};

    #[test]
    fn literal_is_escaped_and_case_folds() {
        let re = build_regex("a.b", false, false).expect("re");
        assert!(re.is_match("xA.Bx"));
        assert!(!re.is_match("axb"));
        let re = build_regex("a.b", false, true).expect("re");
        assert!(!re.is_match("A.B"));
        assert!(build_regex("(", true, false).is_err());
    }

    #[test]
    fn globs_match_names_and_paths() {
        let g = glob_regex("*.rs, docs/").expect("g").expect("some");
        assert!(g.is_match("crates/a/src/x.rs"));
        assert!(g.is_match("docs/a/b.md"));
        assert!(!g.is_match("x.md"));
        let g = glob_regex("src/**/*.rs").expect("g").expect("some");
        assert!(g.is_match("src/a/b.rs") && g.is_match("src/b.rs"));
        assert!(glob_regex("").expect("g").is_none());
    }

    #[test]
    fn highlights_escape_and_mark() {
        let re = build_regex("b", false, false).expect("re");
        assert_eq!(
            highlight("a<b>b", &re),
            "a&lt;<mark>b</mark>&gt;<mark>b</mark>"
        );
    }

    #[test]
    fn hits_carry_context_and_gaps() {
        let re = build_regex("x", false, true).expect("re");
        let text = "x\n.\n.\n.\n.\nx\n.\n";
        let h = file_hits("a", "HEAD", text, &re);
        assert_eq!(h.count, 2);
        let ns: Vec<usize> = h.lines.iter().map(|l| l.n).collect();
        assert_eq!(ns, vec![1, 2, 0, 5, 6, 7]);
    }
}
