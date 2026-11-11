//! Highlighted file and blame HTML, and the memo that keeps highlighted files.

use std::fmt::Write as _;

use gitterrace_git::BlameLine;

use crate::fmt::{ago, esc, esc_text};
use crate::views::diff::CHUNK_LINES;
use crate::views::{Render, author_href};

/// Chunks highlighted before first paint of a big file; the rest is plain until `code_tail_html`.
const EAGER_CHUNKS: usize = 2;
/// Files up to this size are highlighted whole (about 20 ms): the fragment round trip is not worth
/// it.
const EAGER_BYTES: usize = 32 * 1024;

/// Highlighted files by content, at most `cap` bytes in all. A file over an eighth of the cap
/// would flush the rest for itself, so it is not kept. Full means start over: entries are
/// cheap to rebuild and an LRU would cost a list per hit.
#[derive(Default)]
pub(crate) struct CodeMemo {
    size: usize,
    map: std::collections::HashMap<u64, Code>,
}

/// A highlighted file: its line blocks, whether it was cut at the line cap, and the symbols
/// its grammar named while highlighting (empty for a language with its own scanner, which
/// the outline reads instead).
#[derive(Clone, Default)]
pub(crate) struct Code {
    pub(crate) html: String,
    pub(crate) cut: bool,
    pub(crate) symbols: Vec<crate::symbols::Symbol>,
}

impl CodeMemo {
    fn get(&self, key: u64) -> Option<Code> {
        self.map.get(&key).cloned()
    }

    fn put(&mut self, cap: usize, key: u64, v: &Code) {
        let len = v.html.len();
        if len > cap / 8 {
            return;
        }
        if self.size + len > cap {
            self.map.clear();
            self.size = 0;
        }
        self.size += len;
        self.map.insert(key, v.clone());
    }
}

static CODE_CACHE: std::sync::Mutex<Option<CodeMemo>> = std::sync::Mutex::new(None);

/// Keyed by the limits too: a reload that changes them must not serve the old rendering.
fn code_key(r: &Render, path: &str, text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (path, text, r.highlight_bytes, r.blob_lines).hash(&mut hasher);
    hasher.finish()
}

/// Puts `path` into `memo` as [`code_html`] puts it into the shared cache, and says whether
/// it is kept: the cache policy without the process-wide state other tests also write.
#[cfg(test)]
pub(crate) fn code_memo_put(memo: &mut CodeMemo, cap: usize, path: &str, text: &str) -> usize {
    let r = Render::test(cap);
    let built = code_html_uncached(&r, path, text, usize::MAX);
    memo.put(cap, code_key(&r, path, text), &built);
    built.html.len()
}

#[cfg(test)]
pub(crate) fn code_memo_has(memo: &CodeMemo, path: &str, text: &str) -> bool {
    memo.get(code_key(&Render::test(0), path, text)).is_some()
}

/// Drops every highlighted file and returns whether any was kept. The memo is shared by the
/// whole process, so the idle release empties it.
pub(crate) fn clear_code_memo() -> bool {
    CODE_CACHE
        .lock()
        .is_ok_and(|mut g| g.take().is_some_and(|m| !m.map.is_empty()))
}

/// True when `code_html` of `path` with `text` would be served from the memo.
#[cfg(test)]
pub(crate) fn code_is_cached(r: &Render, path: &str, text: &str) -> bool {
    code_cached(code_key(r, path, text)).is_some()
}

fn code_cached(key: u64) -> Option<Code> {
    let g = CODE_CACHE.lock().ok()?;
    g.as_ref()?.get(key)
}

/// Returns the line blocks and whether it was cut at the line cap.
/// One div per line: the number is a CSS counter, so it neither selects nor copies.
/// Highlighting costs ~0.7 ms/KB, so a blob is rendered once per content.
#[cfg(test)]
pub(crate) fn code_html(r: &Render, path: &str, text: &str) -> (String, bool) {
    let c = code(r, path, text);
    (c.html, c.cut)
}

/// [`code_html`] with the symbols named in the same pass.
pub(crate) fn code(r: &Render, path: &str, text: &str) -> Code {
    let key = code_key(r, path, text);
    if let Some(hit) = code_cached(key) {
        return hit;
    }
    let built = code_html_uncached(r, path, text, usize::MAX);
    if let Ok(mut g) = CODE_CACHE.lock() {
        g.get_or_insert_with(CodeMemo::default)
            .put(r.cache_bytes, key, &built);
    }
    built
}

/// Like `code_html`, but a big uncached file highlights only its first chunks and marks the
/// block `data-hl` so the page fetches the rest from `code_tail_html` after first paint.
/// The symbols are those of the highlighted part: the lines a big file shows plain at first
/// name none until its tail is highlighted and kept.
pub(crate) fn code_first(r: &Render, path: &str, text: &str) -> Code {
    if text.len() <= EAGER_BYTES || code_cached(code_key(r, path, text)).is_some() {
        return code(r, path, text);
    }
    code_html_uncached(r, path, text, EAGER_CHUNKS * CHUNK_LINES)
}

/// [`code_first`] without the symbols.
#[cfg(test)]
pub(crate) fn code_html_first(r: &Render, path: &str, text: &str) -> (String, bool) {
    let c = code_first(r, path, text);
    (c.html, c.cut)
}

/// The chunks `code_first` left plain, highlighted. Empty when the file has no chunk
/// past the eager ones, which is also the answer for a direct request on a small file.
pub(crate) fn code_tail_html(r: &Render, path: &str, text: &str) -> String {
    let full = code(r, path, text).html;
    let marker = "<div class=\"ch\"";
    let end = full.len() - "</div>".len();
    let from = full
        .match_indices(marker)
        .nth(EAGER_CHUNKS)
        .map_or(end, |m| m.0);
    full[from..end].to_owned()
}

fn code_html_uncached(r: &Render, path: &str, text: &str, lit_lines: usize) -> Code {
    let first = text.lines().next().unwrap_or("");
    let mut hl = (text.len() <= r.highlight_bytes)
        .then(|| crate::highlight::for_file(path, first))
        .flatten()
        .map(|h| {
            if crate::symbols::scoped(path, first) {
                h.with_symbols()
            } else {
                h
            }
        });
    let mut symbols = Vec::new();
    let lazy = hl.is_some() && lit_lines != usize::MAX;
    let mut out = String::with_capacity(text.len() + text.len() / 8 + 64);
    // The number column is sized by the widest line number shown, floored at three digits.
    let digits = text
        .lines()
        .count()
        .min(r.blob_lines)
        .max(1)
        .to_string()
        .len()
        .max(3);
    let _ = write!(
        out,
        "<div class=\"code src\"{} data-digits=\"{digits}\">",
        if lazy { " data-hl" } else { "" }
    );
    let mut cut = false;
    let mut open = false;
    for (i, line) in text.lines().enumerate() {
        if i >= r.blob_lines {
            cut = true;
            break;
        }
        if i == lit_lines {
            symbols = hl.take().map(|mut h| h.take_symbols()).unwrap_or_default();
        }
        if i % CHUNK_LINES == 0 {
            if open {
                out.push_str("</div>");
            }
            out.push_str("<div class=\"ch\">");
            open = true;
        }
        let _ = write!(
            out,
            "<div id=\"L{n}\" class=\"l\" data-n=\"{n}\">{}</div>",
            hl.as_mut().map_or_else(|| esc_text(line), |h| h.line(line)),
            n = i + 1
        );
    }
    if open {
        out.push_str("</div>");
    }
    out.push_str("</div>");
    if let Some(mut h) = hl {
        symbols = h.take_symbols();
    }
    Code {
        html: out,
        cut,
        symbols,
    }
}

/// How recent a line is, 0 (this month) to 4 (older than six years): the class of its heat bar.
const fn heat_bucket(time: i64, now: i64) -> u8 {
    const DAY: i64 = 86_400;
    match (now - time) / DAY {
        d if d < 30 => 0,
        d if d < 365 => 1,
        d if d < 3 * 365 => 2,
        d if d < 6 * 365 => 3,
        _ => 4,
    }
}

/// What each heat bar's colour says, in words, for a screen reader: the bar is the only cue,
/// and the time beside it may be a date. Indexed by [`heat_bucket`].
const HEAT_WORDS: [&str; 5] = [
    "Changed in the last month",
    "Changed in the last year",
    "Changed 1 to 3 years ago",
    "Changed 3 to 6 years ago",
    "Changed over 6 years ago",
];

pub(crate) fn blame_html(r: &Render, path: &str, lines: &[BlameLine], now: i64) -> String {
    let total: usize = lines.iter().map(|l| l.text.len()).sum();
    let mut hl = (total <= r.highlight_bytes)
        .then(|| crate::highlight::for_file(path, lines.first().map_or("", |l| l.text.as_str())))
        .flatten();
    // The same caption and quiet header row as `_table.html` writes for a table the design
    // shows without a header row.
    let mut out = String::from(
        "<table class=\"code blame\"><caption class=\"vh\">Blame</caption>\
         <thead class=\"quiet\"><tr>\
         <th scope=\"col\"><span class=\"vh\">Commit</span></th>\
         <th scope=\"col\"><span class=\"vh\">Line</span></th>\
         <th scope=\"col\"><span class=\"vh\">Code</span></th></tr></thead>",
    );
    let mut prev = "";
    let mut heat = 0;
    for l in lines {
        let first = l.commit != prev;
        prev = &l.commit;
        if first {
            heat = heat_bucket(l.time, now);
        }
        let _ = write!(
            out,
            "<tr id=\"L{}\"{}>",
            l.line_no,
            if first { " class=\"run\"" } else { "" }
        );
        if first {
            let _ = write!(
                out,
                "<td class=\"who h{heat}\"><div class=\"bl\"><span class=\"vh\">{}. </span>{}\
                 <a class=\"who\" href=\"{}\">{}</a><span class=\"muted\">{}</span></div></td>",
                HEAT_WORDS[usize::from(heat)],
                crate::fmt::sha_file_link(&l.commit, path),
                esc(&author_href(&l.author)),
                esc(&l.author),
                ago(l.time, now)
            );
        } else {
            let _ = write!(out, "<td class=\"who h{heat}\"></td>");
        }
        let _ = write!(
            out,
            "<td class=\"n\"><a href=\"#L{n}\">{n}</a></td><td class=\"c\">{}</td></tr>",
            hl.as_mut()
                .map_or_else(|| esc(&l.text), |h| h.line(&l.text)),
            n = l.line_no
        );
    }
    out.push_str("</table>");
    out
}

#[cfg(test)]
mod tests {
    use super::heat_bucket;

    #[test]
    fn heat_runs_from_this_month_to_old() {
        let now = 10 * 365 * 86_400;
        let ago = |d: i64| now - d * 86_400;
        let got: Vec<u8> = [1, 100, 800, 2000, 4000]
            .into_iter()
            .map(|d| heat_bucket(ago(d), now))
            .collect();
        assert_eq!(got, [0, 1, 2, 3, 4]);
    }

    /// Each run of a blame says the age its heat bar shows, in words, once per run.
    #[test]
    fn each_blame_run_says_the_age_of_its_bar() {
        let now = 10 * 365 * 86_400;
        let line = |n: u32, commit: &str, days: i64| gitterrace_git::BlameLine {
            line_no: n,
            commit: commit.repeat(40),
            author: "A".into(),
            time: now - days * 86_400,
            text: "x".into(),
        };
        let lines = [
            line(1, "a", 3),
            line(2, "a", 3),
            line(3, "b", 800),
            line(4, "c", 4000),
        ];
        let html = super::blame_html(&crate::views::Render::fold(400), "a.txt", &lines, now);
        let said: Vec<&str> = html
            .split("<div class=\"bl\"><span class=\"vh\">")
            .skip(1)
            .map(|s| s.split(". </span>").next().unwrap_or_default())
            .collect();
        assert_eq!(
            said,
            [
                "Changed in the last month",
                "Changed 1 to 3 years ago",
                "Changed over 6 years ago"
            ],
            "{html}"
        );
        let problems = gitterrace_testkit::a11y::problems(&format!("<h1>x</h1>{html}"));
        assert!(problems.is_empty(), "{problems:?}");
    }
}
