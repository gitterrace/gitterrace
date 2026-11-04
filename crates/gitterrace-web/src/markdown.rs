//! Markdown to HTML, with tables, task lists, footnotes and alerts.
//!
//! Raw HTML in the source is allowed through only as far as `ammonia` permits;
//! everything the parser or this module generates goes through the same
//! sanitizer, so the output never carries scripts, event handlers or styles.

#[cfg(test)]
mod anchor_tests;
mod base;
mod front;
mod ids;
mod lines;
mod outline;

use std::borrow::Cow;
use std::collections::HashMap;

use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html};

use crate::fmt::esc;
pub(crate) use crate::markdown::base::Base;
use crate::markdown::front::{front_matter_html, split_front_matter};
use crate::markdown::lines::{LineMap, mark, sentinel};
pub(crate) use crate::markdown::outline::{OutlineItem, outline_menu, outline_rail};

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

fn code_block(code: &str, lang: &str, span: &str, most: usize) -> String {
    let lang = lang.split([' ', ',', '{']).next().unwrap_or("");
    let body = crate::highlight::block(code, lang, most).unwrap_or_else(|| esc(code));
    let class = if lang.is_empty() {
        String::new()
    } else {
        format!(" class=\"language-{}\"", esc(lang))
    };
    format!("{span}<pre><code{class}>{body}</code></pre>\n")
}

fn alert_open(kind: BlockQuoteKind, span: &str) -> String {
    let (class, title) = match kind {
        BlockQuoteKind::Note => ("note", "Note"),
        BlockQuoteKind::Tip => ("tip", "Tip"),
        BlockQuoteKind::Important => ("important", "Important"),
        BlockQuoteKind::Warning => ("warning", "Warning"),
        BlockQuoteKind::Caution => ("caution", "Caution"),
    };
    format!(
        "{span}<div class=\"markdown-alert markdown-alert-{class}\">\
         <p class=\"markdown-alert-title\">{title}</p>\n"
    )
}

const fn heading_level(level: pulldown_cmark::HeadingLevel) -> u8 {
    level as u8
}

/// A heading being collected: level, explicit id, inner events, plain text, line span.
type Heading<'a> = (u8, Option<String>, Vec<Event<'a>>, String, String);

/// Parser events with headings anchored, fences highlighted, alerts opened and every block
/// marked with its source lines.
fn transform<'a>(
    body: &'a str,
    parser: Parser<'a>,
    map: &LineMap,
    most: usize,
) -> (Vec<Event<'a>>, Vec<OutlineItem>) {
    let mut out: Vec<Event<'a>> = Vec::new();
    let mut items: Vec<OutlineItem> = Vec::new();
    let mut seen = HashMap::new();
    let mut heading: Option<Heading<'a>> = None;
    let mut fence: Option<(String, String, String)> = None;

    for (ev, range) in parser.into_offset_iter() {
        let span = || map.span(body, &range);
        if let Some((lang, code, at)) = fence.as_mut() {
            match ev {
                Event::Text(t) => code.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let html = if lang == "mermaid" {
                        format!("{}<pre class=\"mermaid\">{}</pre>\n", mark(at), esc(code))
                    } else {
                        code_block(code, lang, &mark(at), most)
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
                fence = Some((lang, String::new(), span()));
            }
            Event::Start(Tag::Heading { level, id, .. }) => {
                heading = Some((
                    heading_level(level),
                    id.map(|i| i.to_string()),
                    Vec::new(),
                    String::new(),
                    span(),
                ));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((lvl, id, events, text, at)) = heading.take() {
                    let slug = unique_slug(&mut seen, id.unwrap_or_else(|| slugify(&text)));
                    items.push(OutlineItem {
                        level: lvl,
                        id: slug.clone(),
                        text: text.trim().to_owned(),
                    });
                    let slug = esc(&slug);
                    let at = mark(&at);
                    out.push(Event::Html(
                        format!(
                            "{at}<h{lvl} id=\"{slug}\"><a class=\"anchor\" href=\"#{slug}\" \
                             aria-label=\"Permalink\"></a>"
                        )
                        .into(),
                    ));
                    out.extend(events);
                    out.push(Event::Html(format!("</h{lvl}>\n").into()));
                }
            }
            Event::Start(Tag::BlockQuote(Some(kind))) => {
                out.push(Event::Html(alert_open(kind, &mark(&span())).into()));
            }
            Event::End(TagEnd::BlockQuote(Some(_))) => {
                out.push(Event::Html("</div>\n".into()));
            }
            // Tags the writer emits: the span rides in a sentinel right before the start tag.
            Event::Start(Tag::Paragraph | Tag::Item | Tag::Table(_) | Tag::BlockQuote(None))
                if heading.is_none() =>
            {
                out.push(sentinel(&span()));
                out.push(ev);
            }
            other => {
                if let Some((_, _, events, text, _)) = heading.as_mut() {
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
    (out, items)
}

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
                    Some(frag) if !frag.is_empty() => format!("#{}", ids::doc_id(frag)),
                    _ => url,
                }))
            }
            (_, "id") => Some(Cow::Owned(ids::doc_id(value))),
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

/// A rendered document with the outline of its headings.
pub(crate) struct Doc {
    pub(crate) html: String,
    pub(crate) outline: Vec<OutlineItem>,
}

#[cfg(test)]
pub(crate) fn render_with(src: &str, base: Base) -> String {
    render_doc(src, base).html
}

pub(crate) fn render_doc(src: &str, base: Base) -> Doc {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM;
    // The sentinel characters cannot come from the file, so no span is forged through them.
    let src = &*lines::neutralize(src);
    let (head, body) = match split_front_matter(src) {
        Some((block, rest)) => (front_matter_html(block), rest),
        None => (String::new(), src),
    };
    // Spans count lines of the whole file, so the front matter's lines come first.
    let map = LineMap::new(body, src[..src.len() - body.len()].matches('\n').count());
    let parser = Parser::new_ext(body, opts);
    let (events, outline) = transform(body, parser, &map, base.highlight_bytes);
    let mut raw = String::new();
    html::push_html(&mut raw, events.into_iter());
    // Spans are added after sanitizing, which drops every data-line the file wrote.
    Doc {
        html: head + &lines::place(&sanitizer(base).clean(&raw).to_string()),
        outline,
    }
}

#[cfg(test)]
mod tests {
    fn render(src: &str) -> String {
        let base = Base {
            rev: "HEAD".to_owned(),
            dir: String::new(),
            highlight_bytes: crate::highlight::HIGHLIGHT_BYTES,
        };
        render_with(src, base)
    }

    use crate::markdown::{Base, render_with, resolve_path, rewrite_url, slugify};

    fn at(src: &str, dir: &str) -> String {
        let base = Base {
            rev: "abc123".to_owned(),
            dir: dir.to_owned(),
            highlight_bytes: crate::highlight::HIGHLIGHT_BYTES,
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
        assert!(out.contains("id=\"same\""));
        assert!(out.contains("id=\"same-1\""));
        assert!(out.contains("id=\"code--more\""));
        assert!(out.contains("class=\"anchor\" href=\"#same\""));
    }

    #[test]
    fn relative_links_resolve_against_directory() {
        assert_eq!(resolve_path("docs/guide", "../a.md"), "docs/a.md");
        assert_eq!(resolve_path("docs", "/top.md"), "top.md");
        assert_eq!(resolve_path("", "../../x"), "x");
        let out = at(
            "[a](../a.md#frag) [d](sub/) [x](https://e.com/a) [h](#intro)",
            "docs/guide",
        );
        assert!(out.contains("href=\"/blob/abc123/docs/a.md#frag\""));
        assert!(out.contains("href=\"/tree/abc123/docs/guide/sub\""));
        assert!(out.contains("href=\"https://e.com/a\""));
        assert!(out.contains("href=\"#intro\""));
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
            highlight_bytes: crate::highlight::HIGHLIGHT_BYTES,
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
        assert!(out.contains("<table data-line=\"1-3\">"), "{out}");
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
                highlight_bytes: crate::highlight::HIGHLIGHT_BYTES,
            },
        );
        assert!(
            r.contains("<pre class=\"mermaid\" data-line=\"1-3\">graph TD; A--&gt;B\n</pre>"),
            "{r}"
        );
    }

    #[test]
    fn code_fences_are_highlighted_with_classes() {
        let out = render("```rust\nfn main() {}\n```\n\n```nope\n<b>\n```\n");
        assert!(out.contains("class=\"language-rust\""));
        assert!(out.contains("class=\"s-"));
        assert!(!out.contains("style="));
        assert!(out.contains("&lt;b&gt;"));
    }

    #[test]
    fn outline_lists_headings_in_order_with_ids_and_levels() {
        let doc = crate::markdown::render_doc(
            "# Title\n\n## Install\n\n### From source\n\n## Usage\n\n## Usage\n",
            Base::new("HEAD", ""),
        );
        let rail = crate::markdown::outline_rail(&doc.outline);
        let at = |needle: &str| {
            rail.find(needle)
                .unwrap_or_else(|| panic!("{needle} in {rail}"))
        };
        assert!(at("#install") < at("#from-source"));
        assert!(at("#from-source") < at("#usage\""));
        assert!(rail.contains("<li class=\"d1\"><a href=\"#from-source\""));
        assert!(rail.contains("<li class=\"d0\"><a href=\"#install\""));
        assert!(rail.contains("#usage-1"));
        assert!(!rail.contains("Title"), "a lone h1 is not listed");
        assert!(doc.html.contains("id=\"usage-1\""));
    }

    #[test]
    fn outline_is_hidden_below_three_headings() {
        let doc = crate::markdown::render_doc("## A\n\n## B\n\n#### C\n", Base::new("HEAD", ""));
        assert!(crate::markdown::outline_rail(&doc.outline).is_empty());
        assert!(crate::markdown::outline_menu(&doc.outline, false).is_empty());
    }

    #[test]
    fn outline_lists_h1_only_when_several_and_escapes_text() {
        let doc = crate::markdown::render_doc(
            "# One\n\n# Two & <i>x</i>\n\n## Three\n",
            Base::new("HEAD", ""),
        );
        let menu = crate::markdown::outline_menu(&doc.outline, false);
        assert!(menu.contains(">One</a>") && menu.contains(">Two &amp; x</a>"));
        assert!(!menu.contains("<i>"));
    }
}
