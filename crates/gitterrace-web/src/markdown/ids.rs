//! Which document ids stay plain and which get the `user-content-` prefix.
//!
//! Headings and footnotes keep the id a user writes by hand (`bla.md#reference`), so links
//! work without script and in an export. An id that would collide with the page chrome, or
//! shadow a browser global through named access, is moved under the prefix instead. The same
//! function maps same-page hrefs, so a link and its target always agree.

/// The prefix a colliding id is moved under. `static/app.js` maps a plain hash to it.
pub(crate) const USER_ID: &str = "user-content-";

/// Every literal id the page chrome uses: templates, static scripts and Rust views. The
/// `chrome_ids_match_the_sources` test fails when this list and the sources disagree.
pub(crate) const CHROME_IDS: [&str; 55] = [
    "about",
    "activity-counts",
    "activity-hours",
    "activity-series",
    "actmark",
    "actnote",
    "actsel",
    "branches",
    "code-age",
    "commits",
    "coupled",
    "files-size",
    "formoff",
    "frequency",
    "gt-tip",
    "headstate",
    "hotspot-files",
    "hotspot-map",
    "keys-off",
    "livenote",
    "loc",
    "loc-data",
    "log-q",
    "log-q-error",
    "main",
    "patterns",
    "q",
    "range",
    "refbtn",
    "remotes",
    "releases",
    "repobtn",
    "repocount",
    "repolink",
    "repoq",
    "reposwitch",
    "searchbtn",
    "send-ctx",
    "send-head",
    "send-title",
    "seterrs",
    "seterrsum",
    "setform",
    "setgut",
    "setnote",
    "setplain",
    "setsave",
    "settext",
    "sizes",
    "stashes",
    "tags",
    "theme",
    "types",
    "worktrees",
    "wtchip",
];

/// Names that are dangerous whatever the templates hold: an element with such an id shadows
/// a `window` or `document` property through named access, or one of our script globals.
const NAMED_ACCESS: [&str; 26] = [
    "location",
    "top",
    "name",
    "opener",
    "parent",
    "self",
    "window",
    "document",
    "frames",
    "length",
    "history",
    "navigator",
    "localstorage",
    "sessionstorage",
    "origin",
    "closed",
    "frameelement",
    "body",
    "head",
    "forms",
    "cookie",
    "domain",
    "gtcopy",
    "gttoast",
    "gtoverlay",
    "gtsend",
];

/// Ids the code generates by pattern: `L12` and `L3-L9` line anchors, `f4` diff files.
fn generated(id: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if let Some(rest) = id.strip_prefix('L') {
        return match rest.split_once('-') {
            Some((a, b)) => digits(a) && digits(b.strip_prefix('L').unwrap_or(b)),
            None => digits(rest),
        };
    }
    id.strip_prefix('f').is_some_and(digits)
}

fn reserved(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    CHROME_IDS.contains(&id)
        || NAMED_ACCESS.contains(&lower.as_str())
        || generated(id)
        || id.starts_with(USER_ID)
}

/// The id a document's `id` ends up with in the page: plain unless it is reserved.
pub(crate) fn doc_id(id: &str) -> String {
    if reserved(id) {
        format!("{USER_ID}{id}")
    } else {
        id.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::*;

    fn walk(dir: &Path, ext: &str, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).expect("read_dir").flatten() {
            let p = e.path();
            if p.is_dir() && p.file_name().is_some_and(|n| n != "vendor") {
                walk(&p, ext, out);
            } else if p.extension().is_some_and(|x| x == ext) {
                let name = p.to_string_lossy();
                if !name.ends_with("tests.rs") && !name.contains("/markdown") {
                    out.push(std::fs::read_to_string(&p).expect("read"));
                }
            }
        }
    }

    #[test]
    fn chrome_ids_match_the_sources() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut texts = Vec::new();
        walk(&root.join("templates"), "html", &mut texts);
        walk(&root.join("static"), "js", &mut texts);
        walk(&root.join("src"), "rs", &mut texts);
        let re = regex::Regex::new(
            r#"(?:\bid=\\?"|getElementById\(\s*"|\.id\s*=\s*")([A-Za-z][\w-]*)\\?""#,
        )
        .expect("regex");
        let mut found = BTreeSet::new();
        for t in &texts {
            for c in re.captures_iter(t) {
                let id = &c[1];
                // `L` is getElementById("L" + n), covered by `generated`; the prefix is the
                // app.js lookup of a moved id.
                if id != "L" && id != USER_ID && !generated(id) {
                    found.insert(id.to_owned());
                }
            }
        }
        let listed: BTreeSet<String> = CHROME_IDS.iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(
            found, listed,
            "CHROME_IDS must list every chrome id in the sources"
        );
    }

    #[test]
    fn reserved_ids_get_the_prefix() {
        for id in CHROME_IDS.iter().chain(NAMED_ACCESS.iter()) {
            assert_eq!(doc_id(id), format!("{USER_ID}{id}"), "{id}");
        }
        for id in ["Location", "L12", "L3-L9", "L3-9", "f4", "user-content-x"] {
            assert!(doc_id(id).starts_with(USER_ID), "{id}");
        }
        for id in ["reference", "main-loop", "label", "fn1", "f", "Lx", "query"] {
            assert_eq!(doc_id(id), id, "{id}");
        }
    }
}
