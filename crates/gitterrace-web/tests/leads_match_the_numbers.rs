//! Each insights page opens with one sentence stating its finding, and every number in that
//! sentence is one the page shows below it: the lead is computed from the page's own numbers,
//! so it cannot drift from them.

use gitterrace_testkit::{Fixture, page};

/// The lead sentence of `body`, or `None` when the page has none.
fn lead(body: &str) -> Option<String> {
    let start = body.find("data-lead>")? + "data-lead>".len();
    let end = body[start..].find("</p>")? + start;
    Some(body[start..end].to_owned())
}

/// The numbers written in `s`: digit runs with their thousands commas and decimals.
fn numbers(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() || (!cur.is_empty() && (c == ',' || c == '.')) {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(cur.trim_end_matches([',', '.']).to_owned());
            cur.clear();
        }
    }
    out
}

/// The text of `body` after the lead, with tags stripped, where the page shows its numbers.
fn shown(body: &str) -> String {
    let rest = body.split("data-lead>").nth(1).unwrap_or_default();
    let rest = rest.split_once("</p>").map_or("", |(_, r)| r);
    let mut out = String::new();
    let mut tag = false;
    for c in rest.chars() {
        match c {
            '<' => tag = true,
            '>' => {
                tag = false;
                out.push(' ');
            }
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    out
}

#[test]
fn numbers_are_read_with_commas_and_decimals() {
    assert_eq!(numbers("Rust is 62.0% of 66,500 lines."), ["62.0", "66,500"]);
    assert_eq!(numbers("3 people. 25 directories."), ["3", "25"]);
}

#[tokio::test]
async fn every_lead_states_numbers_the_page_shows() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(dir.path())
        .write("src/a.rs", "fn main() {}\n")
        .write("src/b.rs", "fn b() {}\nfn c() {}\n")
        .write("docs/x.md", "# x\n")
        .commit("feat: first")
        .write("src/a.rs", "fn main() {\n}\n")
        .commit("fix: second")
        .write("src/b.rs", "fn b() {}\n")
        .commit("refactor: third");
    let router = gitterrace_web::app_router(f.path()).expect("router");
    gitterrace_testkit::oracle::ready(&router).await;
    let mut fails = Vec::new();
    for p in
        ["/stats", "/stats/contributors", "/stats/hotspots", "/stats/ownership", "/stats/languages"]
    {
        let (status, body) = page(&router, p).await;
        assert_eq!(status, 200, "{p}");
        let Some(l) = lead(&body) else {
            fails.push(format!("{p}: no lead sentence"));
            continue;
        };
        let rest = shown(&body);
        let nums = numbers(&l);
        if nums.is_empty() {
            fails.push(format!("{p}: lead states no number: {l}"));
        }
        for n in nums {
            // A share is shown with its percent sign, a count as it is.
            let found = rest.contains(&format!("{n}%"))
                || rest
                    .split(|c: char| !(c.is_ascii_digit() || c == ',' || c == '.'))
                    .any(|w| w.trim_end_matches([',', '.']) == n);
            if !found {
                fails.push(format!("{p}: {n} in \"{l}\" is not shown on the page"));
            }
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}
