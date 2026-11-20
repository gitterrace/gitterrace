//! The rendered side of `markup_owners.rs`: pages built from the shared components show them,
//! and none shows a hand-shaped copy (a commit in a table row, a switch outside the segmented
//! control, a document card without the shared header, a style attribute). One router over one
//! shared repository serves every page here.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::{Shared, oracle, page, wait};

/// A table row that holds a commit id link next to a subject: a commit row shaped as a table.
fn table_commits(body: &str) -> usize {
    body.split("<tr")
        .skip(1)
        .filter(|r| {
            let r = r.split("</tr>").next().unwrap_or_default();
            r.contains("class=\"sha\"") && r.contains("class=\"subj\"")
        })
        .count()
}

/// A run of two or more sibling links where one is marked `class="on"` and none sits in the
/// shared control: a hand-written switch, whatever its class is called. The tab row and the
/// option toggles (each flips one setting) are not switches.
fn loose_switches(body: &str) -> Vec<String> {
    let mut rest = body;
    let mut page = String::new();
    while let Some(i) = rest.find("<nav") {
        page.push_str(&rest[..i]);
        rest = rest[i..]
            .split_once("</nav>")
            .map_or("", |(_, after)| after);
    }
    page.push_str(rest);
    let mut bad = Vec::new();
    for div in page.split("<div").skip(1) {
        let open = div.split('>').next().unwrap_or_default();
        if open.contains("segctl") || open.contains("opts") {
            continue;
        }
        let inner = div.split("</div>").next().unwrap_or_default();
        if inner.contains("<nav") || inner.contains("segctl") {
            continue;
        }
        let links = inner.matches("<a ").count();
        let on = inner.matches(" class=\"on\"").count();
        if links >= 2 && on == 1 && !inner.contains("<table") && !inner.contains("<li") {
            bad.push(div.chars().take(160).collect());
        }
    }
    bad
}

/// Cards of `body` that are wrong: a document card whose first child is not the shared header,
/// a card with two, or a card whose first child is classed like a header but is not one.
fn bad_cards(body: &str) -> Vec<String> {
    let mut starts: Vec<usize> = body
        .match_indices("<div class=\"card")
        .chain(body.match_indices("<section class=\"card"))
        .map(|(i, _)| i)
        .collect();
    starts.sort_unstable();
    let mut bad = Vec::new();
    for (n, &s) in starts.iter().enumerate() {
        let card = &body[s..starts.get(n + 1).copied().unwrap_or(body.len())];
        let Some(open_end) = card.find('>') else {
            continue;
        };
        let first = card[open_end + 1..]
            .trim_start()
            .split('>')
            .next()
            .unwrap_or_default();
        let is_doc = card.contains("class=\"markdown-body") || card.contains("data-md-view");
        let headlike = first.starts_with("<div")
            && first.split("class=\"").nth(1).is_some_and(|c| {
                let c = c.split('"').next().unwrap_or_default();
                c.split(' ')
                    .any(|w| w.ends_with("-h") || w.contains("head") || w == "tabs")
            });
        let shared = first.contains("data-dochead");
        if (is_doc && !shared) || card.matches("data-dochead").count() > 1 || (headlike && !shared)
        {
            bad.push(card.chars().take(160).collect());
        }
    }
    bad
}

#[test]
fn hand_shaped_markup_is_caught() {
    let table = "<tr><td>v1</td><td><span class=\"subj\">x</span></td>\
                 <td><a class=\"sha\" href=\"/commit/1\">1</a></td></tr>";
    assert_eq!(table_commits(table), 1);
    let metric =
        "<div class=\"pick\"><a href=\"a\" class=\"on\">Commits</a><a href=\"b\">Lines</a></div>";
    assert_eq!(loose_switches(metric).len(), 1);
    let shared = "<div class=\"segctl\" role=\"group\">\
                  <span class=\"segs\"><a class=\"on\">a</a><a>b</a></span></div>";
    assert_eq!(loose_switches(shared), [] as [std::string::String; 0]);
    let readme = "<div class=\"card readme\"><div class=\"readme-h\">README</div><div \
                  class=\"markdown-body\"></div></div>";
    assert_eq!(bad_cards(readme).len(), 1);
    let pre = "<div class=\"card\"><div class=\"doc-head\">x</div><pre></pre></div>";
    assert_eq!(bad_cards(pre).len(), 1);
    let good = "<div class=\"card x\"><div class=\"dochead sticky\" data-dochead></div><div \
                class=\"markdown-body\"></div></div>";
    assert_eq!(bad_cards(good), [] as [std::string::String; 0]);
}

/// The shared docs repository's router, once its index has the commits the pages list.
async fn ready_router() -> axum::Router {
    let router = gitterrace_web::app_router(Shared::Docs.path()).expect("router");
    oracle::ready(&router).await;
    wait::page_until(&router, "/search?q=first&kind=commits", |b| {
        b.contains("class=\"crow\"")
    })
    .await;
    router
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_empty_states_and_switches_render_the_shared_components() {
    let router = ready_router().await;
    let mut fails = Vec::new();
    let get = async |p: &str| {
        let (status, body) = page(&router, p).await;
        assert_eq!(status, 200, "{p}");
        body
    };

    // Every list of commits, and the refs, renders the one commit row.
    for (p, want) in [
        ("/log/HEAD", 2),
        ("/history/src/a.rs", 2),
        ("/refs", 2),
        ("/tag/v1", 1),
        ("/compare?base=v1&head=HEAD", 1),
    ] {
        let body = get(p).await;
        let rows = body.matches("<li class=\"crow\">").count();
        if rows < want {
            fails.push(format!("{p}: {rows} shared rows, want {want}"));
        }
        if table_commits(&body) > 0 {
            fails.push(format!("{p}: a commit in a table row"));
        }
    }

    // A page with nothing to show offers the next step.
    for p in [
        "/log/HEAD?q=path:nope",
        "/history/nope.rs",
        "/search?q=zzzqqq",
        "/stats?q=author:nobody%40example.com",
    ] {
        let body = get(p).await;
        let Some(state) = body.split("<div class=\"empty\"").nth(1) else {
            fails.push(format!("{p}: no shared empty state"));
            continue;
        };
        let state = state.split("</div>").next().unwrap_or_default();
        if !state.contains("class=\"next\"><a href=") {
            fails.push(format!("{p}: no next step in {state}"));
        }
    }

    // Switches are the shared segmented control.
    for (p, want) in [
        ("/stats", 1),
        ("/stats/contributors", 2),
        ("/stats/contributors?metric=added", 2),
        ("/stats/hotspots", 1),
        ("/stats/ownership", 1),
        ("/stats/languages", 3),
        ("/blob/HEAD/README.md", 1),
    ] {
        let body = get(p).await;
        let got = body.matches("class=\"segctl\"").count();
        if got < want {
            fails.push(format!("{p}: {got} shared controls, want {want}"));
        }
        fails.extend(
            loose_switches(&body)
                .into_iter()
                .map(|b| format!("{p}: {b}")),
        );
    }

    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn document_cards_and_styled_pages_render_the_shared_components() {
    let router = ready_router().await;
    let mut fails = Vec::new();
    let get = async |p: &str| {
        let (status, body) = page(&router, p).await;
        assert_eq!(status, 200, "{p}");
        body
    };

    // Document cards open with the shared header.
    let mut heads = 0;
    for p in [
        "/",
        "/?doc=license",
        "/?doc=contributing",
        "/tree/HEAD",
        "/tree/HEAD/docs",
        "/blob/HEAD/README.md",
        "/blob/HEAD/CONTRIBUTING.md",
        "/blob/HEAD/src/a.rs",
        "/log/HEAD",
    ] {
        let body = get(p).await;
        heads += body.matches("data-dochead").count();
        fails.extend(bad_cards(&body).into_iter().map(|b| format!("{p}: {b}")));
    }
    if heads < 7 {
        fails.push(format!("only {heads} shared document headers"));
    }

    // No style attribute on the pages that once carried one.
    for p in [
        "/",
        "/log/HEAD",
        "/blob/HEAD/long.txt",
        "/stats/hotspots",
        "/stats/languages",
        "/stats/contributors",
        "/stats/contributors?metric=added",
        "/stats/contributors?metric=net",
    ] {
        let body = get(p).await;
        if p.contains("metric=") && !body.contains("class=\"tsb\"") {
            fails.push(format!("{p}: no bars"));
        }
        if body.contains("style=\"") {
            fails.push(format!("{p}: an inline style"));
        }
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}
