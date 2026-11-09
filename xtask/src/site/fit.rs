//! Every built page fitted to the repository's line width. The pages are committed source
//! files, held to 100 columns like the rest, so the frame runs [`fit`] over each page it
//! builds instead of each part wrapping itself.
//!
//! A line breaks only where a newline means what the character it replaces or precedes means:
//! at a space in text, at the space between two attributes, at a space inside a text attribute
//! (`alt`, `content`, `title`, `aria-label`), before a block tag, or between table tags. `<pre>`,
//! `<script>`, `<style>`, `<textarea>` and comments are kept as written. A run with no such
//! place (one long URL) is left whole: it is the only kind of line allowed to stay wider.

/// The widest line a page may have.
pub(crate) const WIDTH: usize = 100;

/// Elements whose content is kept byte for byte.
const RAW: [&str; 4] = ["pre", "script", "style", "textarea"];
/// Table parts, between which a newline is whitespace the table drops.
const TABLE: [&str; 6] = ["table", "thead", "tbody", "tr", "th", "td"];
/// Block-level tags: a newline before one renders as nothing.
const BLOCK: [&str; 32] = [
    "li",
    "p",
    "div",
    "section",
    "nav",
    "ol",
    "ul",
    "dl",
    "dt",
    "dd",
    "meta",
    "link",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "details",
    "summary",
    "figure",
    "figcaption",
    "footer",
    "header",
    "main",
    "blockquote",
    "table",
    "tr",
    "td",
    "th",
    "thead",
    "tbody",
];
/// Attributes whose value is prose, where a newline reads as the space it replaces.
const TEXT_ATTRS: [&str; 4] = ["alt", "content", "title", "aria-label"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Kept as written: a raw element with its content, or a comment.
    Raw,
    Tag,
    Text,
}

/// The tag's lowercase name and whether it closes.
fn name(tag: &str) -> (String, bool) {
    let rest = &tag[1..];
    let close = rest.starts_with('/');
    let rest = rest.trim_start_matches('/');
    let n = rest
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    (n, close)
}

/// `html` cut into raw runs, tags and text.
fn pieces(html: &str) -> Vec<(Kind, &str)> {
    let mut out = Vec::new();
    let mut rest = html;
    while !rest.is_empty() {
        if rest.starts_with("<!--") {
            let end = rest.find("-->").map_or(rest.len(), |e| e + 3);
            out.push((Kind::Raw, &rest[..end]));
            rest = &rest[end..];
        } else if rest.starts_with('<') {
            let mut quoted = false;
            let mut end = rest.len();
            for (i, c) in rest.char_indices() {
                match c {
                    '"' => quoted = !quoted,
                    '>' if !quoted => {
                        end = i + 1;
                        break;
                    }
                    _ => {}
                }
            }
            let (n, close) = name(&rest[..end]);
            if !close && RAW.contains(&n.as_str()) {
                let shut = format!("</{n}>");
                let end = rest[end..]
                    .find(&shut)
                    .map_or(rest.len(), |f| end + f + shut.len());
                out.push((Kind::Raw, &rest[..end]));
                rest = &rest[end..];
            } else {
                out.push((Kind::Tag, &rest[..end]));
                rest = &rest[end..];
            }
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            out.push((Kind::Text, &rest[..end]));
            rest = &rest[end..];
        }
    }
    out
}

/// `html` with the line breaks inside text turned back into spaces, and the newlines between
/// table tags dropped, so [`wrap`] can fill each line again. A blank line and a newline that
/// stands alone between two tags are kept.
fn join(html: &str) -> String {
    let ps = pieces(html);
    let mut out = String::with_capacity(html.len());
    let mut depth = 0usize;
    for (k, &(kind, s)) in ps.iter().enumerate() {
        match kind {
            Kind::Raw => out.push_str(s),
            Kind::Tag => {
                let (n, close) = name(s);
                if n == "table" {
                    depth = if close {
                        depth.saturating_sub(1)
                    } else {
                        depth + 1
                    };
                }
                out.push_str(s);
            }
            Kind::Text if s.trim_matches([' ', '\n']).is_empty() => {
                let before_part = ps.get(k + 1).is_some_and(|&(nk, ns)| {
                    nk == Kind::Tag && TABLE.contains(&name(ns).0.as_str())
                });
                if !(depth > 0 && s == "\n" && before_part) {
                    out.push_str(s);
                }
            }
            Kind::Text => {
                let chars: Vec<char> = s.chars().collect();
                for (j, &c) in chars.iter().enumerate() {
                    if c != '\n' {
                        out.push(c);
                        continue;
                    }
                    let prev = j.checked_sub(1).map(|p| chars[p]);
                    let next = chars.get(j + 1).copied();
                    match (prev, next) {
                        (None | Some('\n'), _) | (_, None | Some('\n')) => out.push(c),
                        (Some(' '), _) | (_, Some(' ')) => {}
                        _ => out.push(' '),
                    }
                }
            }
        }
    }
    out
}

/// A place a line may break: the index in the output, and whether the character there (a
/// space) is replaced or the newline goes in before it.
type Break = (usize, bool);

struct Lines {
    out: Vec<char>,
    col: usize,
    /// The last place to break, outside an attribute value.
    cand: Option<Break>,
    /// The last space inside a text attribute value, used only when there is no `cand`.
    low: Option<Break>,
}

impl Lines {
    const fn newline(&mut self) {
        self.col = 0;
        self.cand = None;
        self.low = None;
    }

    fn push(&mut self, c: char) {
        self.out.push(c);
        if c == '\n' {
            self.newline();
            return;
        }
        self.col += 1;
        if self.col > WIDTH
            && let Some((at, replace)) = self.cand.or(self.low)
        {
            if replace {
                self.out[at] = '\n';
            } else {
                self.out.insert(at, '\n');
            }
            self.col = self.out.len() - at - 1;
            self.cand = None;
            self.low = None;
        }
    }

    /// A space that may become a newline. One past the width breaks at once; `inside` marks a
    /// space inside a text attribute value.
    fn space(&mut self, inside: bool) {
        self.out.push(' ');
        self.col += 1;
        let at = self.out.len() - 1;
        if self.col > WIDTH {
            self.out[at] = '\n';
            self.newline();
        } else if inside {
            self.low = Some((at, true));
        } else {
            self.cand = Some((at, true));
        }
    }

    fn after_space(&self) -> bool {
        matches!(self.out.last(), None | Some(' ' | '\n'))
    }
}

/// `html` broken into lines at most [`WIDTH`] wide at the places the module names.
fn wrap(html: &str) -> String {
    let mut l = Lines {
        out: Vec::with_capacity(html.len() + html.len() / 50),
        col: 0,
        cand: None,
        low: None,
    };
    let mut depth = 0usize;
    for (kind, s) in pieces(html) {
        match kind {
            Kind::Raw => {
                if !s.starts_with("<!--") && l.col > 0 {
                    l.cand = Some((l.out.len(), false));
                }
                s.chars().for_each(|c| l.push(c));
            }
            Kind::Text => {
                for c in s.chars() {
                    if c == ' ' && !l.after_space() {
                        l.space(false);
                    } else {
                        l.push(c);
                    }
                }
            }
            Kind::Tag => {
                let (n, close) = name(s);
                if n == "table" && !close {
                    depth += 1;
                }
                let block = !close && BLOCK.contains(&n.as_str());
                if l.col > 0 && (block || (depth > 0 && TABLE.contains(&n.as_str()))) {
                    l.cand = Some((l.out.len(), false));
                }
                if n == "table" && close {
                    depth = depth.saturating_sub(1);
                }
                tag(&mut l, s);
            }
        }
    }
    l.out.into_iter().collect()
}

/// One tag into `l`: a space between attributes may break, as may one inside a text value.
/// Between attributes the newline goes in before the space, so every attribute keeps the
/// space in front of it that a scan for ` href="` looks for. The space after the tag name is
/// no break: a line that starts with an attribute alone reads as a stray.
fn tag(l: &mut Lines, s: &str) {
    let (mut quoted, mut attr, mut word, mut value) = (false, String::new(), String::new(), "");
    let mut first = true;
    for c in s.chars() {
        if c == '"' {
            if !quoted {
                value = TEXT_ATTRS
                    .iter()
                    .find(|a| **a == attr)
                    .copied()
                    .unwrap_or("");
            }
            quoted = !quoted;
            l.push(c);
            continue;
        }
        if !quoted {
            match c {
                '=' => attr = word.to_ascii_lowercase(),
                ' ' | '\n' | '<' | '>' | '/' => word.clear(),
                _ => word.push(c),
            }
        }
        if c == ' ' && !l.after_space() && quoted && !value.is_empty() {
            l.space(true);
            continue;
        }
        if c == ' ' && !l.after_space() && !quoted {
            if !first {
                l.cand = Some((l.out.len(), false));
            }
            first = false;
        }
        l.push(c);
    }
}

/// A built page fitted to [`WIDTH`]. Fitting a fitted page changes nothing.
pub(crate) fn fit(html: &str) -> String {
    wrap(&join(html))
}

/// The lines of `html` wider than [`WIDTH`] outside `<pre>`, `<script>`, `<style>` and
/// `<textarea>` that no unbreakable token explains. A line wider because it holds a token of
/// at least half the width with no space in it (a single URL) is not counted.
#[cfg(test)]
pub(crate) fn breakable_wide_lines(html: &str) -> Vec<String> {
    let mut bad = Vec::new();
    let mut raw = false;
    for line in html.lines() {
        let opens = RAW.iter().any(|r| line.contains(&format!("<{r}")));
        let shuts = RAW.iter().any(|r| line.contains(&format!("</{r}>")));
        let in_raw = raw || opens;
        if !in_raw && line.chars().count() > WIDTH && !has_token(line) {
            bad.push(line.to_owned());
        }
        raw = (raw || opens) && !shuts;
    }
    bad
}

/// Whether `line` holds a run without a space of at least half [`WIDTH`].
#[cfg(test)]
fn has_token(line: &str) -> bool {
    line.split(' ').any(|t| t.chars().count() >= WIDTH / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn widest(s: &str) -> usize {
        s.lines().map(|l| l.chars().count()).max().unwrap_or(0)
    }

    #[test]
    fn long_text_wraps_at_word_boundaries_and_reads_the_same() {
        let words = "word ".repeat(60);
        let html = format!("<p>{}</p>\n", words.trim_end());
        let out = fit(&html);
        assert!(widest(&out) <= WIDTH, "{out}");
        assert_eq!(out.replace('\n', " "), html.replace('\n', " "));
        assert_eq!(fit(&out), out, "fitting twice changes nothing");
    }

    #[test]
    fn a_long_tag_breaks_between_attributes_and_in_a_text_value() {
        let url = format!("https://example.org/{}", "a/".repeat(30));
        let html = format!("<a class=\"edit\" href=\"{url}\">Edit on GitHub</a>\n");
        let out = fit(&html);
        assert!(out.starts_with("<a class=\"edit\"\n href="), "{out}");
        let desc = "one two three ".repeat(10);
        let meta = format!(
            "<meta name=\"description\" content=\"{}\">\n",
            desc.trim_end()
        );
        let out = fit(&meta);
        assert!(widest(&out) <= WIDTH, "{out}");
        assert_eq!(
            out.replace("\n ", " ").replace('\n', " "),
            meta.replace('\n', " ")
        );
    }

    #[test]
    fn pre_script_and_comments_are_kept_as_written() {
        let long = "x ".repeat(80);
        for html in [
            format!("<pre><code>{long}\n  indented</code></pre>\n"),
            format!("<script>{long}</script>\n"),
            format!("<!-- {long} -->\n"),
        ] {
            assert_eq!(fit(&html), html);
        }
    }

    #[test]
    fn a_table_packs_its_rows_and_an_unbreakable_url_stays_whole() {
        let rows = "<tr><td>a</td><td>b</td></tr>\n".repeat(20);
        let html = format!("<table><tbody>\n{rows}</tbody></table>\n");
        let out = fit(&html);
        assert!(out.lines().count() < 20, "{out}");
        assert!(widest(&out) <= WIDTH, "{out}");
        let url = format!("<a href=\"https://example.org/{}\">", "a".repeat(120));
        assert_eq!(fit(&url), url);
    }

    /// The guard's own red case: a wide prose line is caught, a wide single URL is not.
    #[test]
    fn the_wide_line_check_sees_a_breakable_line_and_spares_a_token() {
        let prose = format!("<p>{}</p>", "word ".repeat(30));
        assert_eq!(breakable_wide_lines(&prose).len(), 1);
        let url = format!("<a href=\"https://example.org/{}\">x</a>", "a".repeat(120));
        assert!(breakable_wide_lines(&url).is_empty());
        assert!(breakable_wide_lines(&fit(&prose)).is_empty());
    }
}
