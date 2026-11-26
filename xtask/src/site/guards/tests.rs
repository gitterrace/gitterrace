//! Each guard on a clean planted site, then with one fault planted, so every guard is seen red.

use super::*;
use std::fmt::Write as _;

const REPO: &str = "https://github.com/o/r";

fn words(n: usize) -> String {
    (0..n).fold(String::new(), |mut s, i| {
        let _ = write!(s, "w{i} ");
        s
    })
}

/// A page in the shared frame with `main` as its text.
fn page(main: &str) -> String {
    format!(
        "<title>T - r</title>\n<meta name=\"description\" content=\"What this page covers.\">\n\
         <header class=\"top\"></header><main id=\"main\">{main}</main><footer></footer>"
    )
}

/// A docs page: the docs list, its text, and next to `next` (empty for the last).
fn doc(next: &str, body: &str) -> String {
    let list = "<nav class=\"docnav\"><a href=\"/docs/a/\">A</a><a href=\"/docs/b/\">B</a></nav>";
    let pager = if next.is_empty() {
        "<nav class=\"pager\"></nav>".to_owned()
    } else {
        format!("<nav class=\"pager\"><a class=\"next\" href=\"/{next}\" rel=\"next\">N</a></nav>")
    };
    let menus = "<div class=\"menus\"><details><a href=\"/docs/a/\">A</a></details></div>\n";
    page(&format!("{menus}<h1>T</h1><p>Lead.</p><h2>S</h2><p>{body}</p>{pager}{list}"))
}

fn site() -> (BTreeMap<String, String>, Facts<'static>) {
    let mut pages = BTreeMap::new();
    let index = page(&format!(
        "<h1>Docs</h1><h2>G</h2><a href=\"/docs/a/\">A</a><a href=\"/docs/b/\">B</a><p>{}</p>",
        words(70)
    ));
    pages.insert("docs/index.html".to_owned(), index);
    pages.insert("docs/a/index.html".to_owned(), doc("docs/b/", &format!("alpha {}", words(70))));
    pages.insert("docs/b/index.html".to_owned(), doc("", &format!("beta {}", words(70))));
    pages.insert("404.html".to_owned(), page("<h1>Not found</h1>"));
    let facts = Facts {
        repo: REPO,
        published: ["docs/a.md".to_owned(), "docs/b.md".to_owned()].into(),
        docs: vec!["docs/a/".to_owned(), "docs/b/".to_owned()],
        budgets: Budgets {
            landing: 10_000,
            docs: 10_000,
            other: 10_000,
            req_landing: 10,
            req_docs: 10,
            req_other: 10,
        },
    };
    (pages, facts)
}

fn plant(edit: impl Fn(&mut BTreeMap<String, String>)) -> String {
    let (mut pages, facts) = site();
    edit(&mut pages);
    problems(&pages, &facts).join("\n")
}

fn edit_page(pages: &mut BTreeMap<String, String>, path: &str, from: &str, to: &str) {
    let html = pages.get_mut(path).expect(path);
    assert!(html.contains(from), "{from} not in {path}");
    *html = html.replacen(from, to, 1);
}

#[test]
fn a_clean_planted_site_passes_every_guard() {
    let (pages, facts) = site();
    let bad = problems(&pages, &facts);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn guard_1_a_link_to_the_markdown_of_a_published_page_is_red() {
    let bad = plant(|p| {
        let links = format!(
            "<h2>S</h2><a href=\"{REPO}/blob/main/docs/b.md#x\">b</a>\
             <a href=\"{REPO}/edit/main/docs/a.md\">edit</a>\
             <a href=\"{REPO}/blob/main/CONTRIBUTING.md\">ok</a>\
             <a class=\"edit\" href=\"{REPO}/blob/main/docs/a.md\">View source</a>"
        );
        edit_page(p, "docs/a/index.html", "<h2>S</h2>", &links);
    });
    assert!(bad.contains("blob/main/docs/b.md#x links the markdown"), "{bad}");
    assert_eq!(bad.matches("links the markdown").count(), 1, "{bad}");
}

#[test]
fn guard_3c_an_asset_link_without_its_version_is_red() {
    let bad = plant(|p| {
        let a = "<h2>S</h2><img src=\"/assets/x.svg\" alt=\"\">";
        edit_page(p, "docs/a/index.html", "<h2>S</h2>", a);
    });
    assert!(bad.contains("/assets/x.svg has no ?v="), "{bad}");
    let ok = plant(|p| {
        let a = "<h2>S</h2><img src=\"/assets/x.svg?v=1\" alt=\"\">";
        edit_page(p, "docs/a/index.html", "<h2>S</h2>", a);
    });
    assert!(!ok.contains("has no ?v="), "{ok}");
}

#[test]
fn guard_3b_a_landing_sentence_said_twice_or_elsewhere_is_red() {
    let line = "This sentence is long enough to count, and the landing page says it.";
    let twice = plant(|p| {
        let html = page(&format!(
            "<h1>Home</h1><h2>S</h2><p>{}.</p><p>{line}</p><p>Other words. {line}</p>",
            words(70)
        ));
        p.insert("index.html".to_owned(), html);
    });
    assert!(twice.contains("index.html: says twice"), "{twice}");
    let elsewhere = plant(|p| {
        let html = page(&format!("<h1>Home</h1><h2>S</h2><p>{}. {line}</p>", words(70)));
        p.insert("index.html".to_owned(), html);
        let b = format!("<h2>S</h2><p>Some lead in. {line} And more.</p>");
        edit_page(p, "docs/b/index.html", "<h2>S</h2>", &b);
    });
    assert!(elsewhere.contains("another page also says"), "{elsewhere}");
    let once = plant(|p| {
        let html = page(&format!("<h1>Home</h1><h2>S</h2><p>{}. {line}</p>", words(70)));
        p.insert("index.html".to_owned(), html);
    });
    assert!(!once.contains("index.html:"), "{once}");
}

#[test]
fn guard_3_prose_two_pages_carry_is_red_unless_quoted() {
    let shared = "This sentence is long enough to count as prose that two pages repeat.";
    let bad = plant(|p| {
        let (a, b) = (format!("<h2>S</h2><p>{shared}</p>"), format!("<h2>S</h2><li>{shared}</li>"));
        edit_page(p, "docs/a/index.html", "<h2>S</h2>", &a);
        edit_page(p, "docs/b/index.html", "<h2>S</h2>", &b);
    });
    assert!(bad.contains("repeats prose of docs/a/index.html"), "{bad}");
    let quoted = plant(|p| {
        let a = format!("<h2>S</h2><p>{shared}</p>");
        edit_page(p, "docs/a/index.html", "<h2>S</h2>", &a);
        edit_page(
            p,
            "docs/b/index.html",
            "<h2>S</h2>",
            &format!("<h2>S</h2><div data-from=\"docs/a/\"><p>{shared}</p></div>"),
        );
    });
    assert!(!quoted.contains("repeats prose"), "{quoted}");
    let code = plant(|p| {
        let a = format!("<h2>S</h2><pre><code>{shared}</code></pre>");
        edit_page(p, "docs/a/index.html", "<h2>S</h2>", &a);
        edit_page(p, "docs/b/index.html", "<h2>S</h2>", &a);
    });
    assert!(!code.contains("repeats prose"), "{code}");
}

#[test]
fn guard_4_a_page_without_text_or_a_section_is_red() {
    let bad = plant(|p| {
        p.insert("changelog/index.html".to_owned(), page("<h1>Changelog</h1><p>Nothing yet.</p>"));
    });
    assert!(bad.contains("changelog/index.html: 3 words and 0 h2"), "{bad}");
}

#[test]
fn guard_6_a_diagram_shown_as_source_is_red() {
    let bad = plant(|p| {
        edit_page(
            p,
            "docs/a/index.html",
            "<h2>S</h2>",
            "<h2>S</h2><pre><code class=\"language-mermaid\">flowchart TD</code></pre>",
        );
    });
    assert!(bad.contains("a mermaid block shows as source"), "{bad}");
}

#[test]
fn guard_7_a_page_without_title_description_or_frame_is_red() {
    let bad = plant(|p| {
        edit_page(p, "404.html", "<title>T - r</title>", "<title></title>");
        edit_page(p, "404.html", "What this page covers.", "");
        edit_page(p, "docs/b/index.html", "<footer></footer>", "");
    });
    assert!(bad.contains("404.html: no title"), "{bad}");
    assert!(bad.contains("404.html: no meta description"), "{bad}");
    assert!(bad.contains("docs/b/index.html: not exactly one <footer>"), "{bad}");
}

#[test]
fn guard_8_a_page_left_out_of_the_index_the_list_or_the_chain_is_red() {
    let stray = plant(|p| {
        p.insert("docs/c/index.html".to_owned(), doc("", &words(70)));
    });
    let want = "docs/c/: a docs page the docs index does not list";
    assert!(stray.contains(want), "{stray}");
    let unlinked = plant(|p| {
        edit_page(p, "docs/index.html", "<a href=\"/docs/b/\">B</a>", "");
    });
    assert!(unlinked.contains("docs/index.html: does not link /docs/b/"), "{unlinked}");
    let unlisted = plant(|p| {
        edit_page(p, "docs/a/index.html", "<a href=\"/docs/b/\">B</a></nav>", "</nav>");
    });
    assert!(unlisted.contains("docs/a/: the docs list leaves out /docs/b/"), "{unlisted}");
    let broken = plant(|p| {
        let next = "<a class=\"next\" href=\"/docs/b/\"";
        edit_page(p, "docs/a/index.html", next, "<a class=\"x\"");
    });
    assert!(broken.contains("the previous and next chain"), "{broken}");
}

/// The class: a docs page that opens with anything but its title and a lead paragraph. The
/// second defect it catches: a page that jumps from the title straight to a section.
#[test]
fn guard_11_a_docs_page_without_title_first_and_a_lead_is_red() {
    // own_main cuts the menus up to the title, so the plant goes before the menus, where a
    // rendered front matter block lands.
    let raw = plant(|p| {
        edit_page(
            p,
            "docs/a/index.html",
            "<div class=\"menus\">",
            "<hr /><h2>name: x</h2><div class=\"menus\">",
        );
    });
    assert!(raw.contains("docs/a/index.html: something comes before the title"), "{raw}");
    let bare = plant(|p| {
        edit_page(p, "docs/b/index.html", "<p>Lead.</p>", "");
    });
    assert!(bare.contains("docs/b/index.html: no lead paragraph right after the title"), "{bare}");
}

/// The class: a page that fetches one more file than its kind may, whatever its size. The second
/// defect it catches: an image added without `loading="lazy"`, which the byte budget misses when
/// the file is not inlined.
#[test]
fn a_page_over_its_request_budget_is_reported() {
    let got = plant(|p| {
        let links = "<link rel=\"stylesheet\" href=\"/a.css\">".repeat(11);
        edit_page(p, "docs/a/index.html", "</main>", &format!("{links}</main>"));
    });
    assert!(got.contains("docs/a/index.html") && got.contains("docs budget"), "{got}");
    let img = plant(|p| {
        let imgs = "<img src=\"/x.webp\" alt=\"\">".repeat(11);
        edit_page(p, "docs/a/index.html", "</main>", &format!("{imgs}</main>"));
    });
    assert!(img.contains("requests"), "{img}");
    let lazy = plant(|p| {
        let imgs = "<img src=\"/x.webp\" alt=\"\" loading=\"lazy\">".repeat(11);
        edit_page(p, "docs/a/index.html", "</main>", &format!("{imgs}</main>"));
    });
    assert!(!lazy.contains("requests"), "{lazy}");
}

/// The class: a page that grows past its kind, such as one shipping an unminified inline SVG.
#[test]
fn a_page_over_its_byte_budget_is_reported() {
    let got = plant(|p| {
        edit_page(
            p,
            "docs/a/index.html",
            "</main>",
            &format!("<svg>{}</svg></main>", "9".repeat(10_000)),
        );
    });
    assert!(got.contains("docs/a/index.html") && got.contains("docs budget"), "{got}");
    assert!(!plant(|_| {}).contains("budget"));
}

/// The class: any markup left out of the weight. The JSON-LD block and the alternate link are
/// sent on every load, so a page is measured with them, and one they push over is reported.
#[test]
fn the_weight_of_a_page_counts_its_json_ld_and_alternate_link() {
    let head = "<link rel=\"alternate\" type=\"text/markdown\" href=\"/docs/a/index.md\">\n\
                <script type=\"application/ld+json\">{}</script>\n";
    let (pages, _) = site();
    let html = pages["docs/a/index.html"].replacen("<title>", &format!("{head}<title>"), 1);
    assert_eq!(weight(&html), html.len());
    let twice = format!("{head}{html}");
    assert_eq!(weight(&twice), twice.len());
    let room = 10_000 - pages["docs/a/index.html"].len();
    let fill = format!("<i>{}</i></main>", "9".repeat(room - head.len() / 2));
    let got = plant(|p| {
        edit_page(p, "docs/a/index.html", "</main>", &fill);
    });
    assert!(!got.contains("budget"), "{got}");
    let got = plant(|p| {
        edit_page(p, "docs/a/index.html", "</main>", &fill);
        edit_page(p, "docs/a/index.html", "<title>", &format!("{head}<title>"));
    });
    assert!(got.contains("docs budget"), "{got}");
}

#[test]
fn budgets_are_read_from_the_registry_counts() {
    let b = Budgets::parse(concat!(
        "site_bytes_landing = 3\nsite_bytes_docs = 2\nsite_bytes_other = 1\n",
        "site_requests_landing = 6\nsite_requests_docs = 5\nsite_requests_other = 4\n"
    ));
    assert_eq!(
        b.map(|b| (b.landing, b.docs, b.other, b.req_landing, b.req_docs, b.req_other)),
        Some((3, 2, 1, 6, 5, 4))
    );
    assert!(Budgets::parse("site_bytes_docs = 2").is_none());
}
