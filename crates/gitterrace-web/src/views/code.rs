//! Highlighted file and blame HTML, and the memo that keeps highlighted files.

use std::fmt::Write as _;

use gitterrace_git::BlameLine;

use crate::fmt::{ago, esc, esc_text, sha_link};
use crate::views::diff::CHUNK_LINES;
use crate::views::{MAX_BLOB_LINES, author_href};

/// Chunks highlighted before first paint of a big file; the rest is plain until `code_tail_html`.
const EAGER_CHUNKS: usize = 2;
/// Files up to this size are highlighted whole (about 20 ms): the fragment round trip is not worth it.
const EAGER_BYTES: usize = 32 * 1024;

/// Highlighted files by content, at most `cap` bytes in all. A file over an eighth of the cap
/// would flush the rest for itself, so it is not kept. Full means start over: entries are
/// cheap to rebuild and an LRU would cost a list per hit.
#[derive(Default)]
pub(crate) struct CodeMemo {
    size: usize,
    map: std::collections::HashMap<u64, (String, bool)>,
}

impl CodeMemo {
    fn get(&self, key: u64) -> Option<(String, bool)> {
        self.map.get(&key).cloned()
    }

    fn put(&mut self, cap: usize, key: u64, v: &(String, bool)) {
        let len = v.0.len();
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

fn code_key(path: &str, text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (path, text).hash(&mut hasher);
    hasher.finish()
}

/// Puts `path` into `memo` as [`code_html`] puts it into the shared cache, and says whether
/// it is kept: the cache policy without the process-wide state other tests also write.
#[cfg(test)]
pub(crate) fn code_memo_put(memo: &mut CodeMemo, cap: usize, path: &str, text: &str) -> usize {
    let built = code_html_uncached(path, text, usize::MAX);
    memo.put(cap, code_key(path, text), &built);
    built.0.len()
}

#[cfg(test)]
pub(crate) fn code_memo_has(memo: &CodeMemo, path: &str, text: &str) -> bool {
    memo.get(code_key(path, text)).is_some()
}

fn code_cached(key: u64) -> Option<(String, bool)> {
    let g = CODE_CACHE.lock().ok()?;
    g.as_ref()?.get(key)
}

/// Returns the line blocks and whether it was cut at the line cap.
/// One div per line: the number is a CSS counter, so it neither selects nor copies.
/// Highlighting costs ~0.7 ms/KB, so a blob is rendered once per content.
pub(crate) fn code_html(cache_bytes: usize, path: &str, text: &str) -> (String, bool) {
    let key = code_key(path, text);
    if let Some(hit) = code_cached(key) {
        return hit;
    }
    let built = code_html_uncached(path, text, usize::MAX);
    if let Ok(mut g) = CODE_CACHE.lock() {
        g.get_or_insert_with(CodeMemo::default)
            .put(cache_bytes, key, &built);
    }
    built
}

/// Like `code_html`, but a big uncached file highlights only its first chunks and marks the
/// block `data-hl` so the page fetches the rest from `code_tail_html` after first paint.
pub(crate) fn code_html_first(cache_bytes: usize, path: &str, text: &str) -> (String, bool) {
    if text.len() <= EAGER_BYTES || code_cached(code_key(path, text)).is_some() {
        return code_html(cache_bytes, path, text);
    }
    code_html_uncached(path, text, EAGER_CHUNKS * CHUNK_LINES)
}

/// The chunks `code_html_first` left plain, highlighted. Empty when the file has no chunk
/// past the eager ones, which is also the answer for a direct request on a small file.
pub(crate) fn code_tail_html(cache_bytes: usize, path: &str, text: &str) -> String {
    let (full, _) = code_html(cache_bytes, path, text);
    let marker = "<div class=\"ch\"";
    let end = full.len() - "</div>".len();
    let from = full
        .match_indices(marker)
        .nth(EAGER_CHUNKS)
        .map_or(end, |m| m.0);
    full[from..end].to_owned()
}

fn code_html_uncached(path: &str, text: &str, lit_lines: usize) -> (String, bool) {
    let mut hl = (text.len() <= crate::highlight::HIGHLIGHT_LIMIT)
        .then(|| crate::highlight::for_file(path, text.lines().next().unwrap_or("")))
        .flatten();
    let lazy = hl.is_some() && lit_lines != usize::MAX;
    let mut out = String::with_capacity(text.len() + text.len() / 8 + 64);
    out.push_str(if lazy {
        "<div class=\"code src\" data-hl>"
    } else {
        "<div class=\"code src\">"
    });
    let mut cut = false;
    let mut open = false;
    for (i, line) in text.lines().enumerate() {
        if i >= MAX_BLOB_LINES {
            cut = true;
            break;
        }
        if i == lit_lines {
            hl = None;
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
    (out, cut)
}

pub(crate) fn blame_html(path: &str, lines: &[BlameLine], now: i64) -> String {
    let total: usize = lines.iter().map(|l| l.text.len()).sum();
    let mut hl = (total <= crate::highlight::HIGHLIGHT_LIMIT)
        .then(|| crate::highlight::for_file(path, lines.first().map_or("", |l| l.text.as_str())))
        .flatten();
    let mut out = String::from("<table class=\"code blame\">");
    let mut prev = "";
    for l in lines {
        let first = l.commit != prev;
        prev = &l.commit;
        let _ = write!(
            out,
            "<tr id=\"L{}\"{}>",
            l.line_no,
            if first { " class=\"run\"" } else { "" }
        );
        if first {
            let _ = write!(
                out,
                "<td class=\"who\"><div class=\"bl\">{}<a class=\"who\" href=\"{}\">{}</a><span class=\"muted\">{}</span></div></td>",
                sha_link(&l.commit),
                esc(&author_href(&l.author)),
                esc(&l.author),
                ago(l.time, now)
            );
        } else {
            out.push_str("<td class=\"who\"></td>");
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
