//! Code search across the tree at a revision and commit search over history.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
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
/// Files scanned in parallel before their results are merged in order and the cap checked:
/// large enough to keep every worker busy, small enough that a capped query stops early.
const SCAN_CHUNK: usize = 256;

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
    let (key, q) = blocking(&app, move |a| Ok((memo_key(a, &q), q))).await?;
    if let Some(h) = app.memo_get(&key) {
        return Ok(Html((*h).clone()));
    }
    let html = blocking(&app, move |a| run(a, q)).await?.render()?;
    app.memo_put(key, Arc::new(html.clone()));
    Ok(Html(html))
}

/// Key for one rendered search: the memo key moves with HEAD, the index and config, and the
/// query part pins the searched commit, so a repeat at the same tip skips the scan.
fn memo_key(a: &App, q: &SearchQuery) -> String {
    let rev = q.rev.as_deref().filter(|r| !r.is_empty()).unwrap_or("HEAD");
    let commit = a.repo.resolve(rev).unwrap_or_default();
    let query = format!(
        "{rev}\0{commit}\0{}\0{:?}\0{}\0{}\0{}",
        q.q.as_deref().unwrap_or_default(),
        q.kind,
        q.regex.is_some(),
        q.case.is_some(),
        q.glob.as_deref().unwrap_or_default(),
    );
    a.memo_key("search", Some(&query))
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
            "{}{plus} {} in {}, {} scanned, {} skipped (binary or over {})",
            fmt::thousands(page.matches),
            fmt::noun(page.matches, "match", "matches"),
            fmt::count(page.files.len(), "file", "files"),
            fmt::count(page.scanned, "file", "files"),
            page.skipped,
            fmt::size(a.repo.text_limit())
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

enum Scanned {
    Skipped,
    Miss,
    Hits(FileHits),
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
    let workers = std::thread::available_parallelism()
        .map_or(2, usize::from)
        .min(8);
    let rev_enc = fmt::enc_seg(rev);
    let mut readers: Vec<BlobReader> = (0..workers)
        .filter_map(|_| BlobReader::open(&a.repo).ok())
        .collect();
    let mut out = CodeOut {
        files: Vec::new(),
        matches: 0,
        scanned: 0,
        skipped: 0,
        capped: false,
    };
    // Chunks are scanned in parallel but merged in path order, so a capped result is always
    // the same first files: the page is memoized, and a race must not decide what it keeps.
    for chunk in list.chunks(SCAN_CHUNK) {
        let next = AtomicUsize::new(0);
        let slots: Vec<Mutex<Option<Scanned>>> = chunk.iter().map(|_| Mutex::new(None)).collect();
        std::thread::scope(|s| {
            for reader in &mut readers {
                let (next, slots) = (&next, &slots);
                let rev_enc = &rev_enc;
                s.spawn(move || {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let (Some(f), Some(slot)) = (chunk.get(i), slots.get(i)) else {
                            break;
                        };
                        let r = match reader.text(f.blob, a.repo.text_limit()) {
                            None => Scanned::Skipped,
                            Some(text) if re.is_match(&text) => {
                                Scanned::Hits(file_hits(&f.path, rev_enc, &text, re))
                            }
                            Some(_) => Scanned::Miss,
                        };
                        if let Ok(mut g) = slot.lock() {
                            *g = Some(r);
                        }
                    }
                });
            }
        });
        for slot in slots {
            match slot.into_inner().ok().flatten() {
                Some(Scanned::Skipped) | None => out.skipped += 1,
                Some(Scanned::Miss) => out.scanned += 1,
                Some(Scanned::Hits(mut hits)) => {
                    out.scanned += 1;
                    if hits.count == 0 {
                        continue;
                    }
                    hits.count = hits.count.min(max_matches - out.matches);
                    out.matches += hits.count;
                    out.files.push(hits);
                    if out.matches >= max_matches {
                        out.capped = true;
                        return Ok(out);
                    }
                }
            }
        }
    }
    Ok(out)
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
    use std::sync::Arc;

    use axum::extract::{Query, State};

    use crate::search::{
        SearchQuery, build_regex, file_hits, glob_regex, highlight, memo_key, search, search_code,
    };

    /// Guard: a repeat search at the same tip is served from the memo, not rescanned. The
    /// memo entry is swapped for a marker; a handler that scans again would not return it.
    #[tokio::test]
    async fn repeat_search_at_one_tip_is_memoized() {
        let dir = tempfile::tempdir().expect("tmp");
        let f = gitterrace_testkit::Fixture::init(dir.path())
            .write("a.txt", "needle\n")
            .commit("feat: a");
        let repo = gitterrace_git::Repo::discover(f.path()).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let env = crate::app::Env::new(gitterrace_config::Config::default());
        let app = Arc::new(crate::app::App::configured(repo, index, &env));
        let q = || SearchQuery {
            q: Some("needle".into()),
            ..SearchQuery::default()
        };
        let first = search(State(Arc::clone(&app)), Query(q()))
            .await
            .expect("page")
            .0;
        assert!(first.contains("1 match"), "{first}");
        app.memo_put(memo_key(&app, &q()), Arc::new("memo".into()));
        let again = search(State(Arc::clone(&app)), Query(q()))
            .await
            .expect("page")
            .0;
        assert_eq!(again, "memo", "the repeat must not scan again");
        let other = SearchQuery {
            case: Some("on".into()),
            ..q()
        };
        let other = search(State(app), Query(other)).await.expect("page").0;
        assert_ne!(other, "memo", "options are part of the key");
    }

    /// Guard: a capped code search keeps the first matching files in path order, whatever
    /// the thread timing, so the memoized page is the same page a rescan would give.
    #[test]
    fn capped_search_keeps_the_first_files_in_path_order() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut f = gitterrace_testkit::Fixture::init(dir.path());
        for i in 0..600 {
            f = f.write(&format!("f{i:04}.txt"), "needle\nneedle\n");
        }
        let f = f.commit("feat: many");
        let repo = gitterrace_git::Repo::discover(f.path()).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let mut cfg = gitterrace_config::Config::default();
        cfg.limits.search_matches = 501;
        let app = crate::app::App::configured(repo, index, &crate::app::Env::new(cfg));
        let re = build_regex("needle", false, false).expect("re");
        for _ in 0..5 {
            let out = search_code(&app, "HEAD", &re, None).expect("scan");
            let paths: Vec<&str> = out.files.iter().map(|h| h.path.as_str()).collect();
            let want: Vec<String> = (0..251).map(|i| format!("f{i:04}.txt")).collect();
            assert_eq!(paths, want, "the first files, in order");
            assert_eq!((out.matches, out.capped), (501, true));
            assert_eq!(
                out.files.last().map(|h| h.count),
                Some(1),
                "the last file is cut"
            );
            assert_eq!(
                out.scanned, 251,
                "files up to the one that hits the cap count as scanned"
            );
        }
    }

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
