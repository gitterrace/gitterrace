//! One shared component, one owner: each rule names the markup only its owner file may write,
//! and a scan over the sources fails where anything else writes it. A hand-written copy drifts
//! in spacing, escaping or behavior from the shared one (the period choices once glued
//! together, the refs page once put the commit id first). Every rule proves itself on its red
//! samples before its scan is trusted. The rendered side of the same rules is in
//! `shared_markup_pages.rs`.

use std::path::Path;

use gitterrace_testkit::{css, scan};

type Finder = fn(&str) -> Vec<String>;

/// A component rule: where to look, who may write the markup, how to find it, and samples
/// that must and must not be found.
struct Rule {
    name: &'static str,
    dirs: &'static [&'static str],
    /// File extensions scanned, every file when empty.
    exts: &'static [&'static str],
    owners: &'static [&'static str],
    find: Finder,
    red: &'static [&'static str],
    green: &'static [&'static str],
}

const RULES: &[Rule] = &[
    Rule {
        name: "author avatar, the _avatar.html macro",
        dirs: &["templates", "src", "static"],
        exts: &["html", "rs", "js"],
        owners: &["_avatar.html"],
        find: avatars,
        red: &[
            "<span class=\"av h{{ p.hue / 10 }}\">x</span>",
            "format!(\"<span class=\\\"av h{}\\\">\")",
        ],
        green: &["{% call avatars::avatar(p.hue, p.initial) %}{% endcall %}"],
    },
    Rule {
        name: "+a -d line delta, _delta.html and fmt.rs",
        dirs: &["templates", "src", "static"],
        exts: &["html", "rs", "js"],
        owners: &["_delta.html", "fmt.rs"],
        find: deltas,
        red: &[
            "<b class=\"add\">+{{ a }}</b> <b class=\"del\">-{{ d }}</b>",
            "\"<b class=\\\"add\\\">+{}</b> <b class=\\\"del\\\">-{}</b>\"",
        ],
        // Added and removed in their own cells are not a pair.
        green: &[
            "{% call deltas::delta(a, d) %}{% endcall %}",
            "<b class=\"add\">+1</b></td><td><b class=\"del\">-2</b>",
        ],
    },
    Rule {
        // The diff file header writes its change-kind badge with the rest of that header.
        name: "badge or tag label, the _badge.html macros",
        dirs: &["templates", "src"],
        exts: &[],
        owners: &["_badge.html", "diff.rs"],
        find: labels,
        red: &[
            r#"<span class="tag">new</span>"#,
            r#"<span class="badge set-src">x</span>"#,
            r#"<span class="tag {{ b }}">{{ b }}</span>"#,
            r#"format!("<span class=\"badge\">x</span>")"#,
        ],
        green: &[r#"<a class="tags">x</a><b class="sha">y</b>"#],
    },
    Rule {
        name: "row of headline figures, the _kpis.html macro",
        dirs: &["templates", "src"],
        exts: &[],
        owners: &["_kpis.html"],
        find: kpi_rows,
        red: &[
            r#"<div class="kpis"><div><b>3</b></div></div>"#,
            r#"format!("<div class=\"kpis\">{x}</div>")"#,
        ],
        green: &[r"{% call kp::kpis(kpis) %}{% endcall %}"],
    },
    Rule {
        name: "commit row, commit_line.rs",
        dirs: &["src", "templates", "static"],
        exts: &[],
        owners: &["commit_line.rs", "commit_line.html", "app.css"],
        find: commit_rows,
        red: &[
            "<li class=\"crow\"><div class=\"cmain\">",
            "\"<li class=\\\"crow\\\">\"",
        ],
        green: &["<li class=\"row\">"],
    },
    Rule {
        // compare_href applies the feature switch and the prefill; a branch row once linked
        // to a compare page the config had turned off.
        name: "compare link, compare::compare_href",
        dirs: &["src", "templates", "static"],
        exts: &[],
        owners: &["compare.rs"],
        find: compare_links,
        red: &["href(&format!(\"/compare?base={}&head={}\", a, b))"],
        green: &["crate::compare::compare_href(Some(b), h)"],
    },
    Rule {
        // The not-found page is a whole page with its own heading and several ways on.
        name: "empty state, empty.rs",
        dirs: &["src", "templates"],
        exts: &[],
        owners: &["empty.rs", "empty_state.html", "lost.html"],
        find: empties,
        red: &[
            "<p class=\"muted\">No commits.</p>",
            "<tr><td class=\"muted\">None</td></tr>",
            r#"<p class="empty">No history yet.</p>"#,
            "\"<p class=\\\"muted pad\\\">No textual changes.</p>\"",
        ],
        // A hint a script shows while filtering a menu starts hidden: not a page state.
        green: &[
            "<p class=\"muted\">Nothing else here</p>",
            "<span class=\"muted\">Notes</span>",
            "<p class=\"muted\" hidden>No matching heading</p>",
        ],
    },
    Rule {
        // The path macro's `seg` spans are path parts, not a switch. The scripts wire the
        // narrow select and the document views.
        name: "segmented control, seg.rs",
        dirs: &["src", "templates", "static"],
        exts: &[],
        owners: &[
            "seg.rs",
            "_path.html",
            "seg_control.html",
            "app.css",
            "buttons.css",
            "app.js",
            "markdown.js",
        ],
        find: switches,
        red: &["<div class=\"seg\"><span>Period</span><a href=\"a\" class=\"on\">Week</a>"],
        green: &["<div class=\"chips\"></div>"],
    },
    Rule {
        // A copy would drift in look and lose the outline button.
        name: "document card header, dochead.rs",
        dirs: &["src", "templates"],
        exts: &[],
        owners: &[
            "dochead.rs",
            "dochead.html",
            "outline.rs",
            "outline.html",
            "seg.rs",
            "seg_control.html",
        ],
        find: docheads,
        red: &[
            "<div class=\"dochead sticky\" data-dochead>",
            "<b class=\"outlinebtn\">",
        ],
        green: &["<div class=\"card\">"],
    },
    Rule {
        // The repositories home has no repository, so it builds its own bar.
        name: "top bar and full page, _base.html",
        dirs: &["templates"],
        exts: &["html"],
        owners: &["_base.html", "repos.html"],
        find: top_bars,
        red: &[
            "<header class=\"top\">",
            "<div class=\"topbar\">",
            "<!doctype html><html></html>",
        ],
        green: &["{% extends \"_base.html\" %}<!doctype html>"],
    },
    Rule {
        // `.meta` stays for real metadata (who, when, sha); a sentence takes the mono font and
        // sits jammed under the navigation.
        name: "page description, the .hint line",
        dirs: &["templates"],
        exts: &[],
        owners: &[],
        find: prose_metas,
        red: &["<p class=\"meta muted\">Linked worktrees recorded here.</p>"],
        green: &[
            "<p class=\"meta muted\">{% if a %}Tagged{% endif %}</p>",
            "<p class=\"hint\">Linked worktrees recorded here.</p>",
            "<p class=\"meta\"><b>Ann</b> 2 days ago</p>",
        ],
    },
    Rule {
        // Classes do the work, so a policy stricter than the app's own loses nothing. Every
        // template, including a branch no fixture reaches.
        name: "no style attribute in a template",
        dirs: &["templates"],
        exts: &[],
        owners: &[],
        find: style_attrs,
        red: &["<div style=\"x\">"],
        green: &["<div class=\"x\">"],
    },
    Rule {
        // Only the shared rule and controls that are not a form's submit button (segmented
        // control, menus, the repo switcher) may style a button.
        name: "form button, the shared button rule",
        dirs: &["static"],
        exts: &["css"],
        owners: &[],
        find: scoped_buttons,
        red: &[".cmp button{ background: red }", "form.x>button.go{ x: y }"],
        green: &["button{ x: y }button:hover{ x: y }.segs button{ x: y }"],
    },
];

/// Each needle `code` contains, once.
fn any_of(code: &str, needles: &[&str]) -> Vec<String> {
    needles
        .iter()
        .filter(|n| code.contains(**n))
        .map(|n| (*n).to_owned())
        .collect()
}

/// Each occurrence of each needle.
fn each_of(code: &str, needles: &[&str]) -> Vec<String> {
    needles
        .iter()
        .flat_map(|n| code.matches(n).map(str::to_owned))
        .collect()
}

fn avatars(code: &str) -> Vec<String> {
    each_of(
        code,
        &["class=\"av ", "class=\\\"av ", "class=\"av\"", "'av h"],
    )
}

fn deltas(code: &str) -> Vec<String> {
    each_of(
        code,
        &["</b> <b class=\"del\">", "</b> <b class=\\\"del\\\">"],
    )
}

fn kpi_rows(code: &str) -> Vec<String> {
    any_of(code, &["class=\"kpis\"", "class=\\\"kpis\\\""])
}

fn commit_rows(code: &str) -> Vec<String> {
    any_of(
        code,
        &[
            "class=\"crow",
            "class=\"cmain",
            "class=\\\"crow",
            "class=\\\"cmain",
        ],
    )
}

fn switches(code: &str) -> Vec<String> {
    any_of(
        code,
        &[
            "segctl",
            "class=\"segs",
            "segcap",
            "segsel",
            "data-seg-select",
            "class=\"seg\"",
            "class=\"seg ",
            "aria-pressed",
        ],
    )
}

fn docheads(code: &str) -> Vec<String> {
    any_of(
        code,
        &[
            "data-dochead",
            "class=\"dochead",
            "doctabs",
            "data-show=",
            "outlinebtn",
        ],
    )
}

fn top_bars(code: &str) -> Vec<String> {
    let mut bad = any_of(code, &["<header class=\"top\"", "class=\"topbar\""]);
    if code.contains("<!doctype") && !code.contains("{% extends") {
        bad.push("a full page that does not extend _base.html".to_owned());
    }
    bad
}

fn style_attrs(code: &str) -> Vec<String> {
    code.lines()
        .enumerate()
        .filter(|(_, l)| l.contains("style="))
        .map(|(n, _)| format!("line {}", n + 1))
        .collect()
}

fn compare_links(code: &str) -> Vec<String> {
    code.match_indices("/compare?")
        .map(|(i, _)| code[i.saturating_sub(30)..i + 9].to_owned())
        .collect()
}

/// An element whose class list starts with `badge` or `tag`.
fn labels(code: &str) -> Vec<String> {
    code.match_indices("class=")
        .filter_map(|(i, _)| {
            let rest = code[i + 6..].trim_start_matches(['"', '\\']);
            let first = rest.split(['"', '\\', ' ']).next().unwrap_or_default();
            ["badge", "tag", "badge{%", "tag{%"]
                .contains(&first)
                .then(|| code[i..].chars().take(60).collect())
        })
        .collect()
}

/// An element whose class names `empty`, or a muted line or cell that opens with "No" or reads
/// "None".
fn empties(code: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for (i, _) in code.match_indices("class=") {
        let rest = code[i + 6..].trim_start_matches(['"', '\\']);
        let class = rest.split(['"', '\\']).next().unwrap_or_default();
        let after = rest
            .split_once('>')
            .map_or("", |(_, a)| a)
            .trim_start_matches(['"', '#']);
        if rest
            .split('>')
            .next()
            .unwrap_or_default()
            .contains(" hidden")
        {
            continue;
        }
        let words: Vec<&str> = class.split_whitespace().collect();
        let says_none = after.starts_with("No ") || after.starts_with("None<");
        if words.contains(&"empty") || (words.contains(&"muted") && says_none) {
            bad.push(code[i..].chars().take(80).collect());
        }
    }
    bad
}

/// A `meta muted` paragraph whose text opens with a capital then a lower-case letter: a
/// sentence, not a template value or a name.
fn prose_metas(code: &str) -> Vec<String> {
    let open = "<p class=\"meta muted\">";
    code.match_indices(open)
        .filter(|(i, _)| {
            let mut chars = code[i + open.len()..].chars();
            let (a, b) = (chars.next().unwrap_or(' '), chars.next().unwrap_or(' '));
            a.is_ascii_uppercase() && b.is_ascii_lowercase()
        })
        .map(|(i, _)| code[i..].chars().take(80).collect())
        .collect()
}

/// Selectors that restyle a button through a container class.
fn scoped_buttons(css: &str) -> Vec<String> {
    let allowed = [".segs", "button.branch", "button.reponame", "button:"];
    let mut bad = Vec::new();
    for rule in css.split('}') {
        let Some((sel, _)) = rule.split_once('{') else {
            continue;
        };
        for s in sel.split(',').map(str::trim) {
            let words: Vec<&str> = s
                .split([' ', '>', '+', '~'])
                .filter(|w| !w.is_empty())
                .collect();
            let scoped = words.len() > 1 && words.last().is_some_and(|w| w.starts_with("button"));
            if scoped && !allowed.iter().any(|a| s.contains(a)) {
                bad.push(s.to_owned());
            }
        }
    }
    bad
}

#[test]
fn every_rule_finds_its_red_samples_and_passes_its_green_ones() {
    let mut fails = Vec::new();
    for r in RULES {
        for s in r.red {
            if (r.find)(s).is_empty() {
                fails.push(format!("{}: missed {s}", r.name));
            }
        }
        for s in r.green {
            if !(r.find)(s).is_empty() {
                fails.push(format!("{}: flagged {s}", r.name));
            }
        }
    }
    // Test modules may spell the markup back in asserts.
    assert_eq!(
        avatars(scan::product(
            "fn a() {}\n#[cfg(test)]\nmod t { \"class=\\\"av \" }"
        )),
        [] as [std::string::String; 0]
    );
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

#[test]
fn only_its_owner_writes_a_shared_component() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut bad = Vec::new();
    for r in RULES {
        for b in scan::outside_owners(root, r.dirs, r.exts, r.owners, r.find) {
            bad.push(format!("{}: {b}", r.name));
        }
    }
    assert!(
        bad.is_empty(),
        "written outside its owner:\n{}",
        bad.join("\n")
    );
}

/// A class selector in the static CSS that nothing emits is dead weight. Families built as
/// `format!("w{n}")` are checked by prefix; the markdown renderer names the footnote, alert and
/// heading classes.
#[test]
fn every_css_class_is_emitted_somewhere() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |dirs: &[&str], exts: &[&str]| -> String {
        scan::files_in(root, dirs, exts)
            .iter()
            .filter(|p| {
                let inside = p.strip_prefix(root).unwrap_or(p);
                !inside.components().any(|c| c.as_os_str() == "vendor")
            })
            .map(|p| std::fs::read_to_string(p).unwrap_or_default() + "\n")
            .collect()
    };
    let styles = read(&["static"], &["css"]);
    let code = read(&["templates"], &["html", "svg", "txt"])
        + &read(&["src"], &["rs"])
        + &read(&["static"], &["js"]);
    let ramps = ["w", "h", "lw", "kw", "m", "s", "c"];
    let rendered = [
        "footnote-definition",
        "markdown-alert-",
        "markdown-h",
        "footnote-definition-label",
    ];
    let dead = css::dead_classes(&styles, &code, &ramps, &rendered);
    assert!(
        dead.is_empty(),
        "classes in static CSS that nothing emits: {dead:?}"
    );
}
