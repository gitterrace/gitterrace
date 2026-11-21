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

/// Characters of one result line shown; a longer line is clipped around its match. Not
/// configurable: a layout choice (one line in the result card), not a resource limit.
const MAX_LINE_CHARS: usize = 300;
/// Files scanned in parallel before their results are merged in order and the cap checked:
/// large enough to keep every worker busy, small enough that a capped query stops early.
const SCAN_CHUNK: usize = 256;
/// Longest query or glob a search takes, in bytes. A request controls both, and both are
/// copied into the memo key, the regex and the page, so a public server caps them before any
/// of that. Far past any query typed into the box.
pub(crate) const MAX_QUERY_BYTES: usize = 1024;

/// The message a search over [`MAX_QUERY_BYTES`] shows instead of results.
fn too_long(what: &str) -> String {
    format!("The {what} is longer than {MAX_QUERY_BYTES} bytes. Shorten it and search again.")
}

/// `s` cut to at most `MAX_QUERY_BYTES`, on a character boundary.
fn clip(s: &str) -> &str {
    let mut end = s.len().min(MAX_QUERY_BYTES);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Whether the query or glob of `q` is over the cap, so it is neither run nor memoized.
fn over_cap(q: &SearchQuery) -> bool {
    [&q.q, &q.glob].iter().any(|v| v.as_ref().is_some_and(|v| v.len() > MAX_QUERY_BYTES))
}

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
    pub(crate) generated: bool,
}

impl FileHits {
    /// The header count, matching lines as the summary counts them: "1 match", "2 matches".
    fn matches(&self) -> String {
        fmt::count(self.count, "match", "matches")
    }
}

// The flags mirror the form's checkboxes; the template reads them directly.
#[expect(clippy::struct_excessive_bools, reason = "independent switches, one per option")]
#[derive(Template)]
#[template(path = "search.html")]
struct SearchPage {
    c: Chrome,
    head: crate::pagehead::PageHead,
    q: String,
    rev: String,
    commits_mode: bool,
    /// Code/Commits, each a link to the same search with the other kind.
    kinds: crate::seg::Seg,
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
    nothing: Option<crate::empty::Empty>,
}

pub(crate) async fn search(
    State(app): State<Arc<App>>,
    Query(q): Query<SearchQuery>,
) -> Result<Html<String>> {
    if over_cap(&q) {
        return Ok(Html(blocking(&app, move |a| run(a, q)).await?.render()?));
    }
    let (key, q) = blocking(&app, move |a| Ok((memo_key(a, &q), q))).await?;
    if let Some(h) = app.memo_get(&key) {
        return Ok(Html(h));
    }
    // One scan per key: requests that arrive while it runs wait, then read its memo entry.
    let claim = app.render_lock(&key);
    let _held = claim.hold().await;
    if let Some(h) = app.memo_get(&key) {
        return Ok(Html(h));
    }
    let html = blocking(&app, move |a| run(a, q)).await?.render()?;
    app.memo_put(key, &html);
    Ok(Html(html))
}

/// Threads one code search scans with. The search holds one `blocking_jobs` slot, and its
/// readers stay within that setting, so `blocking_jobs` bounds the CPU a search takes.
pub(crate) fn scan_workers(cfg: &gitterrace_config::Config) -> usize {
    crate::app::job_slots(cfg).clamp(1, 8)
}

#[cfg(test)]
static SCANS: Mutex<Vec<std::path::PathBuf>> = Mutex::new(Vec::new());

/// Code scans run for the repository at `git_dir`, for the single-flight test.
#[cfg(test)]
pub(crate) fn scans_of(git_dir: &std::path::Path) -> usize {
    SCANS.lock().map_or(0, |g| g.iter().filter(|p| *p == git_dir).count())
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

/// Code/Commits as the page's one switch: each choice is this search with the other kind.
fn kinds_seg(
    text: &str,
    rev: &str,
    glob: &str,
    (regex, case): (bool, bool),
    commits: bool,
) -> crate::seg::Seg {
    use crate::seg::{Choice, Seg};
    let link = |kind: &str| {
        let mut u = format!(
            "/search?q={}&kind={kind}&rev={}&glob={}",
            fmt::enc_seg(text),
            fmt::enc_seg(rev),
            fmt::enc_seg(glob)
        );
        if regex {
            u.push_str("&regex=1");
        }
        if case {
            u.push_str("&case=1");
        }
        fmt::href(&u)
    };
    Seg::links(
        "Search in",
        vec![
            Choice::new("Code", link("code"), !commits),
            Choice::new("Commits", link("commits"), commits),
        ],
    )
    .no_caption()
}

/// The query and file filter cut to [`MAX_QUERY_BYTES`], with the message when either was
/// longer, the query first.
fn capped(text: &str, glob: &str) -> (String, String, Option<String>) {
    let error = if text.len() > MAX_QUERY_BYTES {
        Some(too_long("query"))
    } else if glob.len() > MAX_QUERY_BYTES {
        Some(too_long("file filter"))
    } else {
        None
    };
    (clip(text).to_owned(), clip(glob).to_owned(), error)
}

fn run(a: &App, q: SearchQuery) -> Result<SearchPage> {
    #[cfg(test)]
    if let Ok(mut g) = SCANS.lock() {
        g.push(a.repo.git_dir().to_owned());
    }
    let rev = q.rev.filter(|r| !r.is_empty());
    let rev = rev.unwrap_or_else(|| "HEAD".to_owned());
    let commits_mode = q.kind.as_deref() == Some("commits");
    let (regex, case) = (q.regex.is_some(), q.case.is_some());
    let (text, glob, cap_error) =
        capped(q.q.as_deref().unwrap_or_default(), q.glob.as_deref().unwrap_or_default());
    let mut page = SearchPage {
        c: views::chrome_at(a, "search", &rev),
        head: search_head(&rev),
        searched: !text.is_empty(),
        q: text.clone(),
        rev: rev.clone(),
        commits_mode,
        kinds: kinds_seg(&text, &rev, &glob, (regex, case), commits_mode),
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
        nothing: None,
    };
    if let Some(e) = cap_error {
        page.error = Some(e);
        return Ok(page);
    }
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
        page.files = generated::generated_last(a, &rev, out.files)?;
    }
    page.ms = start.elapsed().as_millis();
    if page.matches == 0 {
        let e = crate::empty::Empty::new("No results.");
        // The other kind of search is the next place to look.
        let (label, kind) = if commits_mode {
            ("Search the code instead", "code")
        } else {
            ("Search commit messages instead", "commits")
        };
        let next =
            format!("/search?q={}&kind={kind}&rev={}", fmt::enc_seg(&text), fmt::enc_seg(&rev));
        page.nothing = Some(e.step(label, fmt::href(&next)));
    }
    page.summary = summary(&page, commits_mode, a.repo.text_limit());
    page.head.tagline =
        crate::pagehead::Tagline::of(&page.summary).plain(&format!(" in {} ms", page.ms));
    Ok(page)
}

/// What a search found, for the head's tagline: the matches, and for code the files read.
fn summary(page: &SearchPage, commits_mode: bool, text_limit: u64) -> String {
    let plus = if page.capped { "+" } else { "" };
    if commits_mode {
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
            fmt::size(text_limit)
        )
    }
}

/// The search page's head, before a query: what it searches.
fn search_head(rev: &str) -> crate::pagehead::PageHead {
    let said = format!("Search the code and commit messages at {rev}");
    crate::pagehead::PageHead::new("Search", crate::pagehead::Tagline::of(&said))
}

/// Compiles `pat` under the size limit every search pattern shares.
fn compile(
    pat: &str,
    tune: impl FnOnce(&mut RegexBuilder),
) -> std::result::Result<Regex, regex::Error> {
    let mut b = RegexBuilder::new(pat);
    b.size_limit(8 * 1024 * 1024);
    tune(&mut b);
    b.build()
}

/// The error is the last line of the regex engine's complaint, which is the part a reader
/// needs.
fn build_regex(q: &str, is_regex: bool, case: bool) -> std::result::Result<Regex, String> {
    let pat = if is_regex { q.to_owned() } else { regex::escape(q) };
    compile(&pat, |b| {
        b.case_insensitive(!case).multi_line(true);
    })
    .map_err(|e| e.to_string().lines().last().unwrap_or("invalid pattern").to_owned())
}

/// Comma separated globs. Without a `/` a glob matches a file name at any depth.
fn glob_regex(glob: &str) -> std::result::Result<Option<Regex>, String> {
    let parts: Vec<&str> = glob.split(',').map(str::trim).filter(|g| !g.is_empty()).collect();
    if parts.is_empty() {
        return Ok(None);
    }
    let mut alts = Vec::new();
    for g in parts {
        let g = if g.ends_with('/') { format!("{g}**") } else { g.to_owned() };
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
        alts.push(if g.contains('/') { format!("^{body}$") } else { format!("(?:^|/){body}$") });
    }
    compile(&alts.join("|"), |_| {}).map(Some).map_err(|e| e.to_string())
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
    let list: Vec<&FileRef> =
        all.iter().filter(|f| glob.is_none_or(|g| g.is_match(&f.path))).collect();
    let max_matches = a.cfg().limits.search_matches;
    let per_file = a.cfg().limits.search_file_lines;
    let workers = scan_workers(&a.cfg());
    let rev_enc = fmt::enc_seg(rev);
    let mut readers: Vec<BlobReader> =
        (0..workers).filter_map(|_| BlobReader::open(&a.repo).ok()).collect();
    let mut out = CodeOut { files: Vec::new(), matches: 0, scanned: 0, skipped: 0, capped: false };
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
                                Scanned::Hits(file_hits(&f.path, rev_enc, &text, re, per_file))
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
fn file_hits(path: &str, rev_enc: &str, text: &str, re: &Regex, per_file: usize) -> FileHits {
    let lines: Vec<&str> = text.lines().collect();
    let mut hit_idx = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if re.is_match(l) {
            hit_idx.push(i);
            if hit_idx.len() >= per_file {
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
            out.push(Line { n: 0, html: String::new(), hit: false });
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
        generated: false,
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

pub(crate) mod commits;
mod generated;
use commits::search_commits;

#[cfg(test)]
mod message_tests;
#[cfg(test)]
mod tests;
