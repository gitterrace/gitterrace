//! What agents and search engines read besides the pages: `llms.txt` and `llms-full.txt`, the
//! sitemap with each page's `lastmod`, and the landing page's one JSON-LD `SoftwareApplication`.
//! Every value comes from a registry (the docs index through the twins, the workspace package,
//! git), never from text typed here.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result};
use gitterrace_git::{DAY, Repo, last_commits, ymd};
use serde_json::json;

use crate::facts::Package;
use crate::site::docs::without_front_matter;
use crate::site::frame::Site;
use crate::site::twins::{Twin, md_path};

/// The longest the history walk for one folder of sources may take.
const WALK: Duration = Duration::from_secs(300);

/// The date (`YYYY-MM-DD`, UTC) of the last commit to each of `sources`, by path. A file no
/// commit holds yet has no entry, so a page of a new file has no `lastmod`.
pub(super) fn lastmods(
    root: &Path,
    sources: &BTreeSet<String>,
) -> Result<BTreeMap<String, String>> {
    let repo = Repo::discover(root).context("opening the repository for the sitemap dates")?;
    let dirs: BTreeSet<&str> =
        sources.iter().map(|s| s.rsplit_once('/').map_or("", |d| d.0)).collect();
    let mut out = BTreeMap::new();
    for dir in dirs {
        let (found, complete) = last_commits(&repo, "HEAD", dir, WALK)
            .with_context(|| format!("last commits of {dir:?}"))?;
        anyhow::ensure!(complete, "the history walk of {dir:?} ran out of time");
        for c in found {
            let rel = if dir.is_empty() { c.name } else { format!("{dir}/{}", c.name) };
            if sources.contains(&rel) {
                out.insert(rel, ymd(c.time.div_euclid(DAY)));
            }
        }
    }
    Ok(out)
}

/// The source file each sitemap page is rendered from, by site path (`""` is the home page).
pub(super) fn sources(twins: &[Twin]) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> =
        twins.iter().map(|t| (t.path.clone(), t.source.clone())).collect();
    out.insert(String::new(), crate::site::landing::LANDING.to_owned());
    out.insert("docs/".to_owned(), crate::docs_index::INDEX.to_owned());
    out
}

/// `sitemap.xml`: the home page and every page of `paths`, each with the date of the last
/// commit to its source. No `changefreq` or `priority`, which search engines ignore.
pub(super) fn sitemap(
    home: &str,
    paths: &[String],
    sources: &BTreeMap<String, String>,
    dates: &BTreeMap<String, String>,
) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for p in std::iter::once(&String::new()).chain(paths) {
        let _ = write!(out, "<url><loc>{home}/{p}</loc>");
        if let Some(d) = sources.get(p).and_then(|s| dates.get(s)) {
            let _ = write!(out, "<lastmod>{d}</lastmod>");
        }
        out.push_str("</url>\n");
    }
    out.push_str("</urlset>\n");
    out
}

/// What both `llms` files open with: the name, the pitch and the version.
fn head(site: &Site, pitch: &str) -> String {
    format!("# {}\n\n> {pitch}\n\nVersion: {}\n", site.name, site.version)
}

/// `llms.txt`: the summary, then a link to every page of the sitemap: a document by its
/// markdown file, the home page and the docs index by the page.
pub(super) fn llms(site: &Site, pitch: &str, twins: &[Twin]) -> String {
    let home = site.home;
    let mut out = head(site, pitch);
    let _ = write!(
        out,
        "\n## Site\n\n- [Home]({home}/): {pitch}\n\
         - [Documentation]({home}/docs/): Every document, by what you need.\n\
         - [All documents in one file]({home}/llms-full.txt): The documents below as one text.\n"
    );
    let mut groups: Vec<&str> = Vec::new();
    for t in twins {
        if !groups.contains(&t.group.as_str()) {
            groups.push(&t.group);
        }
    }
    for g in groups {
        let _ = write!(out, "\n## {g}\n\n");
        for t in twins.iter().filter(|t| t.group == g) {
            let _ = writeln!(out, "- [{}]({home}/{}): {}", t.title, md_path(&t.path), t.what);
        }
    }
    out
}

/// `llms-full.txt`: the summary, then every document as markdown after the address of its
/// page. The changelog is left out: it grows with every release and `llms.txt` links it.
pub(super) fn llms_full(site: &Site, pitch: &str, twins: &[Twin]) -> String {
    let mut out = head(site, pitch);
    for t in twins.iter().filter(|t| t.path != crate::site::changelog::PATH) {
        let body = without_front_matter(&t.text).trim_end();
        let _ = write!(out, "\n---\n\nSource: {}/{}\n\n{body}\n", site.home, t.path);
    }
    out
}

/// The landing page's JSON-LD `SoftwareApplication`, on one line and safe inside `<script>`.
/// Only facts the workspace states, and only the fields the page needs: no ratings, no FAQ or
/// how-to markup. Every byte counts against the page budget.
pub(super) fn json_ld(pkg: &Package, targets: &[String]) -> String {
    let mut systems = Vec::new();
    for (needle, name) in [("apple", "macOS"), ("linux", "Linux"), ("windows", "Windows")] {
        if targets.iter().any(|t| t.contains(needle)) {
            systems.push(name);
        }
    }
    let repo = pkg.repository.trim_end_matches('/');
    let doc = json!({
        "@context": "https://schema.org",
        "@type": "SoftwareApplication",
        "name": pkg.name,
        "description": pkg.description,
        "applicationCategory": "DeveloperApplication",
        "operatingSystem": systems.join(", "),
        "license": format!("https://spdx.org/licenses/{}.html", pkg.license),
        "offers": {"@type": "Offer", "price": "0"},
        "softwareVersion": pkg.version,
        "downloadUrl": format!("{repo}/releases/tag/v{}", pkg.version),
        "codeRepository": repo,
    });
    doc.to_string().replace('<', "\\u003c")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::site::twins::tests::twin;

    fn site() -> Site<'static> {
        Site {
            home: "https://h.example",
            repo: "https://github.com/o/r",
            name: "r",
            demo: "",
            license: "MIT",
            version: "1.2.3",
            version_href: "changelog/",
        }
    }

    #[test]
    fn llms_txt_links_every_twin_and_carries_the_version() {
        let twins = [twin("docs/x/", "Start"), twin("changelog/", "Project")];
        let txt = llms(&site(), "Pitch.", &twins);
        assert!(txt.contains("Version: 1.2.3"), "{txt}");
        assert!(txt.contains("(https://h.example/docs/x/index.md)"), "{txt}");
        assert!(txt.contains("(https://h.example/changelog/index.md)"), "{txt}");
        assert!(txt.contains("(https://h.example/docs/)"), "{txt}");
    }

    #[test]
    fn llms_full_holds_each_document_without_front_matter_and_skips_the_changelog() {
        let twins = [twin("docs/x/", "Start"), twin("changelog/", "Project")];
        let txt = llms_full(&site(), "Pitch.", &twins);
        assert!(txt.contains("Source: https://h.example/docs/x/\n\n# X\n\nBody."), "{txt}");
        assert!(!txt.contains("name: x"), "{txt}");
        assert!(!txt.contains("changelog/"), "{txt}");
    }

    #[test]
    fn the_sitemap_dates_each_page_from_its_source_without_changefreq_or_priority() {
        let twins = [twin("docs/x/", "Start")];
        let dates: BTreeMap<String, String> =
            [("docs/x.md".to_owned(), "2026-01-02".to_owned())].into();
        let paths = vec!["docs/x/".to_owned(), "changelog/".to_owned()];
        let xml = sitemap("https://h.example", &paths, &sources(&twins), &dates);
        let entry = "<loc>https://h.example/docs/x/</loc><lastmod>2026-01-02</lastmod>";
        assert!(xml.contains(entry), "{xml}");
        assert!(xml.contains("<loc>https://h.example/changelog/</loc></url>"), "{xml}");
        assert!(!xml.contains("changefreq") && !xml.contains("priority"), "{xml}");
    }

    #[test]
    fn the_json_ld_states_the_package_and_cannot_close_its_script() {
        let pkg = Package {
            name: "r".to_owned(),
            description: "A </script> tool.".to_owned(),
            rust_version: String::new(),
            version: "1.2.3".to_owned(),
            license: "MIT".to_owned(),
            repository: "https://github.com/o/r".to_owned(),
            homepage: "https://h.example/".to_owned(),
            bins: Vec::new(),
            deps: BTreeSet::new(),
        };
        let targets = ["aarch64-apple-darwin".to_owned(), "x86_64-pc-windows-msvc".to_owned()];
        let text = json_ld(&pkg, &targets);
        assert!(!text.contains("</script>"), "{text}");
        let v: serde_json::Value = serde_json::from_str(&text).expect("json");
        assert_eq!(v["@type"], "SoftwareApplication");
        assert_eq!(v["softwareVersion"], "1.2.3");
        assert_eq!(v["operatingSystem"], "macOS, Windows");
        assert_eq!(v["downloadUrl"], "https://github.com/o/r/releases/tag/v1.2.3");
        assert_eq!(v["offers"]["price"], "0");
        assert!(v.get("aggregateRating").is_none());
    }
}
