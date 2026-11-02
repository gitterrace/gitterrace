//! Markdown to HTML, with tables, task lists, footnotes and alerts.
//!
//! Raw HTML in the source is allowed through only as far as `ammonia` permits;
//! everything the parser or this module generates goes through the same
//! sanitizer, so the output never carries scripts, event handlers or styles.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Write as _;

use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html};

use crate::fmt::esc;

/// Where a markdown file lives, so relative links can be resolved.
pub(crate) struct Base {
    /// Revision as it should appear in URLs (already encoded).
    pub(crate) rev: String,
    /// Directory of the markdown file inside the repo, no slashes at the ends.
    pub(crate) dir: String,
}

impl Base {
    /// `rev` is the revision exactly as typed in the URL, `dir` the file's directory.
    pub(crate) fn new(rev: &str, dir: &str) -> Self {
        Self {
            rev: crate::fmt::enc_seg(rev),
            dir: dir.trim_matches('/').to_owned(),
        }
    }
}

/// Splits a leading `---` block from the document. Returns the block body and the rest.
/// An unterminated block is not front matter.
fn split_front_matter(src: &str) -> Option<(&str, &str)> {
    let rest = src
        .strip_prefix("---\n")
        .or_else(|| src.strip_prefix("---\r\n"))?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    None
}

/// Parses the simple `key: value` and `key:` + `- item` forms. None when any line
/// is something else, so the caller shows the block verbatim.
fn parse_front_matter(block: &str) -> Option<Vec<(String, String)>> {
    let mut rows: Vec<(String, Vec<String>)> = Vec::new();
    for line in block.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(item) = line.trim_start().strip_prefix("- ") {
            rows.last_mut()?.1.push(unquote(item.trim())?);
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            return None;
        }
        let (key, value) = line.split_once(':')?;
        let key = key.trim();
        if key.is_empty()
            || key.contains(|c: char| c.is_whitespace() || "&*!|>{}[]#'\"".contains(c))
        {
            return None;
        }
        let value = value.trim();
        if value.is_empty() {
            rows.push((key.to_owned(), Vec::new()));
        } else {
            rows.push((key.to_owned(), vec![unquote(value)?]));
        }
    }
    if rows.is_empty() {
        return None;
    }
    Some(rows.into_iter().map(|(k, v)| (k, v.join(", "))).collect())
}

/// Plain, or wrapped in one matching pair of quotes. Anchors, aliases, block
/// scalars and flow collections are not handled.
fn unquote(v: &str) -> Option<String> {
    for q in ['"', '\''] {
        if let Some(inner) = v.strip_prefix(q) {
            return inner.strip_suffix(q).map(str::to_owned);
        }
    }
    if v.starts_with(['&', '*', '!', '|', '>', '{', '[', '#']) {
        return None;
    }
    Some(v.to_owned())
}

/// HTML for the front matter block: a key/value table, or the raw text as code.
/// Built here and added after sanitizing, so every piece is escaped by hand.
fn front_matter_html(block: &str) -> String {
    parse_front_matter(block).map_or_else(
        || format!("<pre><code>{}</code></pre>\n", esc(block)),
        |rows| {
            let mut out = String::from("<table class=\"fm\">");
            for (k, v) in rows {
                let _ = write!(out, "<tr><th>{}</th><td>{}</td></tr>", esc(&k), esc(&v));
            }
            out.push_str("</table>\n");
            out
        },
    )
}

/// Heading slug as most Markdown hosts make it, so links written elsewhere still land: lowercase, keep letters, digits, `-` and `_`, spaces to `-`.
fn slugify(text: &str) -> String {
    text.trim()
        .chars()
        .filter_map(|c| {
            if c == ' ' {
                Some('-')
            } else if c == '-' || c == '_' || c.is_alphanumeric() {
                Some(c)
            } else {
                None
            }
        })
        .flat_map(char::to_lowercase)
        .collect()
}

fn unique_slug(seen: &mut HashMap<String, usize>, base: String) -> String {
    let n = seen.entry(base.clone()).or_insert(0);
    let slug = if *n == 0 { base } else { format!("{base}-{n}") };
    *n += 1;
    slug
}

fn has_scheme(url: &str) -> bool {
    url.find([':', '/', '?', '#'])
        .is_some_and(|i| url.as_bytes()[i] == b':')
}

/// Resolves `target` against `dir`, collapsing `.` and `..`. Escaping the root clamps to it.
fn resolve_path(dir: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Rewrites a link or image target. Anchors, absolute URLs and schemes stay as they are.
fn rewrite_url(base: &Base, url: &str, image: bool) -> String {
    if url.is_empty() || url.starts_with('#') || url.starts_with("//") || has_scheme(url) {
        return url.to_owned();
    }
    let split = url.find(['?', '#']).unwrap_or(url.len());
    let (path, suffix) = url.split_at(split);
    let resolved = resolve_path(&base.dir, path);
    let kind = if image {
        "raw"
    } else if resolved.is_empty() || path.ends_with('/') {
        "tree"
    } else {
        "blob"
    };
    if resolved.is_empty() {
        crate::fmt::href(&format!("/{kind}/{}{suffix}", base.rev))
    } else {
        crate::fmt::href(&format!("/{kind}/{}/{resolved}{suffix}", base.rev))
    }
}

fn code_block(code: &str, lang: &str) -> String {
    let lang = lang.split([' ', ',', '{']).next().unwrap_or("");
    let body = crate::highlight::block(code, lang).unwrap_or_else(|| esc(code));
    let class = if lang.is_empty() {
        String::new()
    } else {
        format!(" class=\"language-{}\"", esc(lang))
    };
    format!("<pre><code{class}>{body}</code></pre>\n")
}

fn alert_open(kind: BlockQuoteKind) -> String {
    let (class, title) = match kind {
        BlockQuoteKind::Note => ("note", "Note"),
        BlockQuoteKind::Tip => ("tip", "Tip"),
        BlockQuoteKind::Important => ("important", "Important"),
        BlockQuoteKind::Warning => ("warning", "Warning"),
        BlockQuoteKind::Caution => ("caution", "Caution"),
    };
    format!(
        "<div class=\"markdown-alert markdown-alert-{class}\">\
         <p class=\"markdown-alert-title\">{title}</p>\n"
    )
}

const fn heading_level(level: pulldown_cmark::HeadingLevel) -> u8 {
    level as u8
}

/// Parser events with headings anchored, fences highlighted and alerts opened.
fn transform<'a>(parser: Parser<'a>) -> Vec<Event<'a>> {
    let mut out: Vec<Event<'a>> = Vec::new();
    let mut seen = HashMap::new();
    let mut heading: Option<(u8, Option<String>, Vec<Event<'a>>, String)> = None;
    let mut fence: Option<(String, String)> = None;

    for ev in parser {
        if let Some((lang, code)) = fence.as_mut() {
            match ev {
                Event::Text(t) => code.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let html = if lang == "mermaid" {
                        format!("<pre class=\"mermaid\">{}</pre>\n", esc(code))
                    } else {
                        code_block(code, lang)
                    };
                    out.push(Event::Html(html.into()));
                    fence = None;
                }
                _ => {}
            }
            continue;
        }
        match ev {
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.trim().to_owned(),
                    CodeBlockKind::Indented => String::new(),
                };
                fence = Some((lang, String::new()));
            }
            Event::Start(Tag::Heading { level, id, .. }) => {
                heading = Some((
                    heading_level(level),
                    id.map(|i| i.to_string()),
                    Vec::new(),
                    String::new(),
                ));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((lvl, id, events, text)) = heading.take() {
                    let slug = unique_slug(&mut seen, id.unwrap_or_else(|| slugify(&text)));
                    let slug = esc(&slug);
                    out.push(Event::Html(
                        format!(
                            "<h{lvl} id=\"{slug}\"><a class=\"anchor\" href=\"#{slug}\" \
                             aria-label=\"Permalink\"></a>"
                        )
                        .into(),
                    ));
                    out.extend(events);
                    out.push(Event::Html(format!("</h{lvl}>\n").into()));
                }
            }
            Event::Start(Tag::BlockQuote(Some(kind))) => {
                out.push(Event::Html(alert_open(kind).into()));
            }
            Event::End(TagEnd::BlockQuote(Some(_))) => {
                out.push(Event::Html("</div>\n".into()));
            }
            other => {
                if let Some((_, _, events, text)) = heading.as_mut() {
                    if let Event::Text(t) | Event::Code(t) = &other {
                        text.push_str(t);
                    }
                    events.push(other);
                } else {
                    out.push(other);
                }
            }
        }
    }
    out
}

/// Ids from the document live under this prefix so they cannot collide with page anchors.
const USER_ID: &str = "user-content-";
/// Class prefixes the renderer itself emits; anything else could restyle the page chrome.
const SAFE_CLASS: [&str; 6] = [
    "language-",
    "s-",
    "markdown-",
    "anchor",
    "mermaid",
    "footnote",
];

fn sanitizer(base: Base) -> ammonia::Builder<'static> {
    let mut b = ammonia::Builder::default();
    // `<picture>` with a `prefers-color-scheme` source is how READMEs ship a light and a
    // dark image, so it is kept.
    b.add_tags(["input", "picture", "source"])
        .add_tag_attributes("input", ["type", "checked", "disabled"])
        .add_tag_attributes("source", ["media", "srcset", "type"])
        .add_tag_attributes("th", ["style"])
        .add_tag_attributes("td", ["style"])
        .add_generic_attributes(["class", "id"])
        .url_relative(ammonia::UrlRelative::PassThrough)
        .attribute_filter(move |element, attr, value| match (element, attr) {
            ("a", "href") => {
                let url = rewrite_url(&base, value, false);
                Some(Cow::Owned(match url.strip_prefix('#') {
                    Some(frag) if !frag.is_empty() => format!("#{USER_ID}{frag}"),
                    _ => url,
                }))
            }
            (_, "id") => Some(Cow::Owned(format!("{USER_ID}{value}"))),
            (_, "class") => {
                let kept: Vec<&str> = value
                    .split_whitespace()
                    .filter(|c| SAFE_CLASS.iter().any(|p| c.starts_with(p)))
                    .collect();
                (!kept.is_empty()).then(|| Cow::Owned(kept.join(" ")))
            }
            ("img", "src") => Some(Cow::Owned(rewrite_url(&base, value, true))),
            // One candidate, as the README idiom writes it. A list with widths or densities
            // would need each URL rewritten, so it is dropped and the `<img>` shows instead.
            ("source", "srcset") => (!value.contains([',', ' ']))
                .then(|| Cow::Owned(rewrite_url(&base, value.trim(), true))),
            // Only colour-scheme queries: they are what the page's theme switch can honour.
            ("source", "media") => matches!(
                value.replace(' ', "").as_str(),
                "(prefers-color-scheme:dark)" | "(prefers-color-scheme:light)"
            )
            .then(|| Cow::Borrowed(value)),
            // Only the alignment the parser itself emits survives.
            (_, "style") => matches!(
                value,
                "text-align: left" | "text-align: right" | "text-align: center"
            )
            .then(|| Cow::Borrowed(value)),
            _ => Some(Cow::Borrowed(value)),
        });
    b
}

pub(crate) fn render_with(src: &str, base: Base) -> String {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM;
    let (head, body) = match split_front_matter(src) {
        Some((block, rest)) => (front_matter_html(block), rest),
        None => (String::new(), src),
    };
    let parser = Parser::new_ext(body, opts);
    let events = transform(parser);
    let mut raw = String::new();
    html::push_html(&mut raw, events.into_iter());
    head + &sanitizer(base).clean(&raw).to_string()
}

#[cfg(test)]
mod tests {
    fn render(src: &str) -> String {
        let base = Base {
            rev: "HEAD".to_owned(),
            dir: String::new(),
        };
        render_with(src, base)
    }

    use crate::markdown::{Base, render_with, resolve_path, rewrite_url, slugify};

    fn at(src: &str, dir: &str) -> String {
        let base = Base {
            rev: "abc123".to_owned(),
            dir: dir.to_owned(),
        };
        render_with(src, base)
    }

    #[test]
    fn renders_and_neutralizes_html() {
        let out = render("# Hi\n\n<script>alert(1)</script>\n\n<img src=x onerror=alert(1)>\n");
        assert!(out.contains("Hi</h1>"));
        assert!(!out.contains("<script"));
        assert!(!out.contains("onerror"));
        assert!(!out.contains("alert"));
    }

    #[test]
    fn javascript_urls_are_dropped() {
        let out = render("[x](javascript:alert(1))\n");
        assert!(!out.contains("javascript"));
    }

    #[test]
    fn drops_comments_after_front_matter() {
        let out = render("---\ntitle: x\n---\n\n<!-- note -->\n\n# Real\n");
        assert!(!out.contains("note"));
        assert!(out.contains("Real</h1>"));
        assert!(!out.contains("<h2"));
    }

    #[test]
    fn front_matter_becomes_a_table() {
        let out = render(
            "---\ntitle: Design contract\ndescription: \"The grammar: twelve rules\"\nname: 'q'\ntags:\n  - a\n  - b\n---\n# Doc\n",
        );
        assert!(out.contains("<table class=\"fm\">"), "{out}");
        assert!(
            out.contains("<th>title</th><td>Design contract</td>"),
            "{out}"
        );
        assert!(out.contains("<td>The grammar: twelve rules</td>"), "{out}");
        assert!(out.contains("<td>q</td>"), "{out}");
        assert!(out.contains("<td>a, b</td>"), "{out}");
        assert!(out.contains("Doc</h1>"));
    }

    #[test]
    fn front_matter_values_are_escaped_text() {
        let out = render("---\ntitle: <script>alert(1)</script> [x](http://e.com)\n---\nbody\n");
        assert!(!out.contains("<script"), "{out}");
        assert!(out.contains("&lt;script&gt;"), "{out}");
        assert!(!out.contains("<a "), "{out}");
    }

    #[test]
    fn nested_front_matter_falls_back_to_code() {
        let out = render("---\nauthor:\n  name: x\n  mail: y\n---\nbody\n");
        assert!(!out.contains("class=\"fm\""), "{out}");
        assert!(out.contains("<pre><code>"), "{out}");
        assert!(out.contains("  name: x"), "{out}");
    }

    #[test]
    fn no_or_unterminated_front_matter_is_plain_markdown() {
        let out = render("# T\n\ntext\n");
        assert!(!out.contains("fm"), "{out}");
        let out = render("---\ntitle: x\n\ntext\n");
        assert!(!out.contains("class=\"fm\""), "{out}");
        assert!(out.contains("<hr"), "{out}");
    }

    #[test]
    fn front_matter_handles_crlf() {
        let out = render("---\r\ntitle: Hi\r\n---\r\n# Doc\r\n");
        assert!(out.contains("<th>title</th><td>Hi</td>"), "{out}");
        assert!(out.contains("Doc</h1>"));
    }

    #[test]
    fn allows_safe_html() {
        let out = render(
            "<details><summary>More</summary>\n\nbody <kbd>Ctrl</kbd> H<sub>2</sub>O<br>\n\n</details>\n",
        );
        assert!(out.contains("<details>"));
        assert!(out.contains("<summary>More</summary>"));
        assert!(out.contains("<kbd>Ctrl</kbd>"));
        assert!(out.contains("<sub>2</sub>"));
    }

    #[test]
    fn hostile_class_and_id_are_neutralised() {
        let out = render(
            "<div class=\"ov-back\" id=\"L1\">x</div>\n\n<p class=\"language-rs ov-back\">y</p>\n",
        );
        assert!(!out.contains("ov-back"), "{out}");
        assert!(!out.contains("id=\"L1\""), "{out}");
        assert!(out.contains("id=\"user-content-L1\""), "{out}");
        assert!(out.contains("class=\"language-rs\""), "{out}");
    }

    #[test]
    fn heading_anchors_use_the_common_slug_rule() {
        assert_eq!(slugify("Hello, World! v1.2_x"), "hello-world-v12_x");
        let out = render("## Same\n\n## Same\n\n## `Code` & more\n");
        assert!(out.contains("id=\"user-content-same\""));
        assert!(out.contains("id=\"user-content-same-1\""));
        assert!(out.contains("id=\"user-content-code--more\""));
        assert!(out.contains("class=\"anchor\" href=\"#user-content-same\""));
    }

    #[test]
    fn relative_links_resolve_against_directory() {
        assert_eq!(resolve_path("docs/guide", "../a.md"), "docs/a.md");
        assert_eq!(resolve_path("docs", "/top.md"), "top.md");
        assert_eq!(resolve_path("", "../../x"), "x");
        let out = at(
            "[a](../a.md#frag) [d](sub/) [x](https://e.com/a) [h](#top)",
            "docs/guide",
        );
        assert!(out.contains("href=\"/blob/abc123/docs/a.md#frag\""));
        assert!(out.contains("href=\"/tree/abc123/docs/guide/sub\""));
        assert!(out.contains("href=\"https://e.com/a\""));
        assert!(out.contains("href=\"#user-content-top\""));
    }

    #[test]
    fn picture_keeps_its_colour_scheme_sources() {
        let out = at(
            "<picture>\n<source media=\"(prefers-color-scheme: dark)\" srcset=\"b/d.svg\">\n<img alt=\"x\" src=\"b/l.svg\">\n</picture>",
            "docs",
        );
        assert!(out.contains("<picture>"), "{out}");
        assert!(
            out.contains("media=\"(prefers-color-scheme: dark)\""),
            "{out}"
        );
        assert!(out.contains("srcset=\"/raw/abc123/docs/b/d.svg\""), "{out}");
        assert!(out.contains("src=\"/raw/abc123/docs/b/l.svg\""), "{out}");
        // Other queries and candidate lists are not passed through.
        let odd = at(
            "<picture><source media=\"print\" srcset=\"a.svg 1x, b.svg 2x\"><img src=\"l.svg\"></picture>",
            "",
        );
        assert!(!odd.contains("media=") && !odd.contains("srcset="), "{odd}");
    }

    #[test]
    fn relative_images_go_to_raw() {
        let out = at(
            "![logo](img/l.svg)\n\n<img src=\"../p.png\" alt=\"p\">",
            "docs",
        );
        assert!(out.contains("src=\"/raw/abc123/docs/img/l.svg\""));
        assert!(out.contains("src=\"/raw/abc123/p.png\""));
        let base = Base {
            rev: "r".into(),
            dir: String::new(),
        };
        assert_eq!(
            rewrite_url(&base, "https://x/y.png", true),
            "https://x/y.png"
        );
    }

    #[test]
    fn alerts_render_with_kind_classes() {
        let out = render("> [!WARNING]\n> careful\n");
        assert!(out.contains("markdown-alert markdown-alert-warning"));
        assert!(out.contains("markdown-alert-title\">Warning"));
        assert!(out.contains("careful"));
        assert!(!out.contains("[!WARNING]"));
    }

    #[test]
    fn gfm_tables_tasks_strike_footnotes_autolinks() {
        let out = render(
            "|a|b|\n|:-|-:|\n|1|2|\n\n- [x] done\n- [ ] todo\n\n~~gone~~\n\nnote[^1]\n\n[^1]: the note\n\nhttps://example.com\n",
        );
        assert!(out.contains("<table>"));
        assert!(out.contains("text-align: left"));
        assert!(out.contains("type=\"checkbox\""));
        assert!(out.contains("<del>gone</del>"));
        assert!(out.contains("footnote-reference"));
        assert!(out.contains("the note"));
    }

    #[test]
    fn mermaid_fence_becomes_pre() {
        let r = render_with(
            "```mermaid\ngraph TD; A-->B\n```\n",
            Base {
                rev: "r".into(),
                dir: String::new(),
            },
        );
        assert!(r.contains("<pre class=\"mermaid\">graph TD; A--&gt;B\n</pre>"));
    }

    #[test]
    fn code_fences_are_highlighted_with_classes() {
        let out = render("```rust\nfn main() {}\n```\n\n```nope\n<b>\n```\n");
        assert!(out.contains("class=\"language-rust\""));
        assert!(out.contains("class=\"s-"));
        assert!(!out.contains("style="));
        assert!(out.contains("&lt;b&gt;"));
    }
}
