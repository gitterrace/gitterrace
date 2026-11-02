//! `site/index.html` and `site/assets/`: the one-page website, written from the repo's own
//! sources (Cargo.toml facts, clap, the registries, `docs/why.md`, the brand SVGs and the
//! vendored theme). A fact is never typed here, so `check` fails when the page is stale.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use pulldown_cmark::{Options, Parser, html};

use crate::facts::Workspace;
use crate::out::Out;
use crate::readme_agents_md;

const FONTS: [(&str, &str); 3] = [
    ("fraunces", "fraunces-latin-wght-normal.woff2"),
    (
        "bricolage-grotesque",
        "bricolage-grotesque-latin-wght-normal.woff2",
    ),
    ("jetbrains-mono", "jetbrains-mono-latin-wght-normal.woff2"),
];
const BRAND: [&str; 5] = ["logo-light", "logo-dark", "mark-light", "mark-dark", "mark"];
const TOKENS: &str = "crates/gitterrace-web/static/tokens.css";
/// RFC 9116 makes `Expires` mandatory in security.txt. Bump it with each release, at most a year
/// out; a passed date tells reporters the contact may be dead.
const SECURITY_TXT_EXPIRES: &str = "2027-10-31T23:00:00Z";

const PAGE_CSS: &str = "\
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:var(--text-md)/var(--leading) var(--sans)}
main{max-width:760px;margin:0 auto;padding:var(--space-8) var(--page-pad) var(--space-16)}
a{color:var(--accent-ink)}
h1,h2{font-family:var(--serif);line-height:var(--leading-tight);font-weight:var(--weight-semibold)}
h2{font-size:var(--text-xl);margin:var(--space-16) 0 var(--space-3)}
h3{font-size:var(--text-lg);margin:var(--space-8) 0 var(--space-2)}
header{padding:var(--space-8) 0 var(--space-4);position:relative}
.gh{position:absolute;top:var(--space-8);right:0;color:var(--muted);line-height:0}
.gh:hover,.gh:focus-visible{color:var(--fg)}
.gh svg{fill:currentColor}
header img{display:block;max-width:calc(100% - 40px);height:auto}
.tag{font-family:var(--serif);font-size:var(--text-lg);color:var(--muted);margin:var(--space-4) 0 0}
pre,code{font-family:var(--mono);font-size:var(--text-sm)}
pre{background:var(--soft);border:var(--border);border-radius:var(--radius);padding:var(--space-3) var(--space-4);overflow-x:auto}
:not(pre)>code{background:var(--soft);padding:1px var(--space-1);border-radius:var(--radius)}
.cmds{display:grid;grid-template-columns:max-content 1fr;gap:var(--space-1) var(--space-4);margin:0}.cmds dt code{background:none;padding:0;white-space:nowrap}.cmds dd{margin:0;color:var(--muted)}
@media (max-width:640px){.cmds{grid-template-columns:1fr}.cmds dd{margin-bottom:var(--space-2)}}
ul.features{list-style:none;padding:0;margin:0}
ul.features li{padding:var(--space-2) 0;border-top:var(--border)}
ul.features b{font-weight:var(--weight-semibold)}
.shots{display:grid;grid-template-columns:1fr 1fr;gap:var(--space-6) var(--space-4);margin:var(--space-8) 0}
@media (max-width:640px){.shots{grid-template-columns:1fr}}
figure{margin:0}
figure img,.lightbox img{display:block;width:100%;height:auto;border:var(--border);border-radius:var(--radius)}
.lightbox{display:none}
.lightbox:target{display:flex;position:fixed;inset:0;z-index:10;align-items:center;justify-content:center;padding:var(--space-8);background:color-mix(in srgb,var(--bg) 97%,transparent);cursor:zoom-out}
.lightbox:target picture{max-width:min(1280px,100%)}
.demo{margin-top:var(--space-6)}
figcaption{font-size:var(--text-xs);color:var(--muted);margin-top:var(--space-2);line-height:1.4}figcaption b{color:var(--fg);font-weight:var(--weight-semibold)}
footer{display:flex;flex-wrap:wrap;gap:var(--space-4);font-size:var(--text-sm)}
footer{margin-top:var(--space-16);padding-top:var(--space-4);border-top:var(--border);color:var(--faint)}
";

/// The source host's mark (16x16 path), the link to the source in the top right corner.
const GH_MARK: &str = "M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z";

/// The home page `<head>`: title, description, link previews and icons.
fn head(name: &str, desc: &str, home: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
         <meta name=\"color-scheme\" content=\"light dark\">\n\
         <title>{name}</title>\n<meta name=\"description\" content=\"{desc}\">\n\
         <link rel=\"canonical\" href=\"{home}/\">\n\
         <meta property=\"og:title\" content=\"{name}\">\n\
         <meta property=\"og:description\" content=\"{desc}\">\n\
         <meta property=\"og:url\" content=\"{home}/\">\n\
         <meta property=\"og:type\" content=\"website\">\n\
         <meta property=\"og:image\" content=\"{home}/assets/og.png\">\n\
         <meta property=\"og:image:width\" content=\"1200\">\n\
         <meta property=\"og:image:height\" content=\"630\">\n\
         <meta property=\"og:image:alt\" content=\"{name}\">\n\
         <meta name=\"twitter:card\" content=\"summary_large_image\">\n\
         <link rel=\"icon\" type=\"image/svg+xml\" href=\"assets/mark.svg\">\n\
         <link rel=\"apple-touch-icon\" href=\"assets/apple-touch-icon.png\">\n\
         <link rel=\"stylesheet\" href=\"assets/site.css\">\n</head>\n<body>\n<main>\n",
        name = esc(name),
        desc = esc(desc),
        home = esc(home),
    )
}

pub fn render(root: &Path, ws: &Workspace) -> Result<Vec<Out>> {
    let read = |rel: &str| -> Result<String> {
        std::fs::read_to_string(root.join(rel)).with_context(|| format!("reading {rel}"))
    };
    let pkg = ws
        .packages
        .get("gitterrace")
        .context("package gitterrace")?;
    let why = read("docs/why.md")?;
    let (lead, sections) = split_why(&why);

    let mut outs = Vec::new();
    outs.push(Out::file("site/assets/site.css", css(&read(TOKENS)?)?));
    for (dir, file) in FONTS {
        let path = format!("theme/schwarzwald/fonts/{dir}/{file}");
        let bytes = std::fs::read(root.join(&path)).with_context(|| format!("reading {path}"))?;
        outs.push(Out::bytes(format!("site/assets/fonts/{dir}.woff2"), bytes));
        outs.push(Out::file(
            format!("site/assets/fonts/{dir}-OFL.txt"),
            read(&format!("theme/schwarzwald/fonts/{dir}/OFL.txt"))?,
        ));
    }
    for name in BRAND {
        outs.push(Out::file(
            format!("site/assets/{name}.svg"),
            read(&format!("docs/brand/{name}.svg"))?,
        ));
    }

    outs.extend(shot_files(root)?);

    let repo = &pkg.repository;
    let home = pkg.homepage.trim_end_matches('/');
    let mut h = head(&pkg.name, &pkg.description, home);
    let _ = write!(
        h,
        "<header>\n<picture>\n<source media=\"(prefers-color-scheme: dark)\" srcset=\"assets/logo-dark.svg\">\n\
         <img src=\"assets/logo-light.svg\" alt=\"{n}\" width=\"540\" height=\"120\">\n</picture>\n\
         <p class=\"tag\">{tag}</p>\n<a class=\"gh\" href=\"{r}\" aria-label=\"{n} on GitHub\" title=\"GitHub\"><svg viewBox=\"0 0 16 16\" width=\"24\" height=\"24\" aria-hidden=\"true\"><path d=\"{GH_MARK}\"/></svg></a>\n</header>\n",
        r = esc(repo),
        n = esc(&pkg.name),
        tag = esc(&tagline(root)?),
    );
    let demo = if ws.demo.is_empty() {
        String::new()
    } else {
        format!(
            "<p class=\"demo\"><a href=\"{}\">Try the live demo</a>, no install needed.</p>\n",
            esc(&ws.demo)
        )
    };
    let _ = write!(
        h,
        "<section>\n<h2>Install</h2>\n{}</section>\n",
        md(&readme_agents_md::install_md(ws))
    );
    let _ = write!(
        h,
        "<section id=\"features\">\n<h2>What it does</h2>\n<ul class=\"features\">\n{}</ul>\n{}{demo}<h3>Commands</h3>\n{}</section>\n",
        features(),
        gallery(root)?,
        commands(),
    );
    let _ = write!(
        h,
        "<section>\n<h2>Why</h2>\n{}{}</section>\n",
        md(lead),
        md(&sections.0)
    );
    let _ = write!(
        h,
        "<section>\n<h2>How it is made</h2>\n{}</section>\n",
        md(&sections.1)
    );
    let _ = write!(
        h,
        "<footer>\n<a href=\"{repo}\">Repository</a>\n<a href=\"{repo}/tree/main/docs\">Docs</a>\n\
         <a href=\"{repo}/blob/main/LICENSE\">{lic} license</a>\n<span>Version {ver}</span>\n</footer>\n\
         </main>\n</body>\n</html>\n",
        repo = esc(repo),
        lic = esc(&pkg.license),
        ver = esc(&pkg.version),
    );
    outs.push(Out::file("site/index.html", h));
    outs.extend(host_files(home, repo, &pkg.name));
    Ok(outs)
}

/// The files the static host reads beside the page: response headers, the 404 page, the
/// crawler files and the security contact. The CSP is tight because the page is static: no
/// script runs at all, so `script-src` is `'none'`, and a script tag added to the page fails
/// loudly in the browser instead of quietly shipping.
fn host_files(home: &str, repo: &str, name: &str) -> Vec<Out> {
    let headers = "\
# Generated by `cargo xtask generate` (xtask/src/site.rs). Do not edit.
/*
  Content-Security-Policy: default-src 'none'; img-src 'self'; style-src 'self'; font-src 'self'; script-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'
  X-Content-Type-Options: nosniff
  X-Frame-Options: DENY
  Referrer-Policy: strict-origin-when-cross-origin
  Strict-Transport-Security: max-age=63072000; includeSubDomains; preload
  Permissions-Policy: accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=()
  Cross-Origin-Opener-Policy: same-origin

/assets/fonts/*
  Cache-Control: public, max-age=31536000, immutable

/assets/screenshots/*
  Cache-Control: public, max-age=86400
";
    let not_found = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
         <meta name=\"color-scheme\" content=\"light dark\">\n\
         <meta name=\"robots\" content=\"noindex\">\n<title>Not found - {n}</title>\n\
         <link rel=\"icon\" type=\"image/svg+xml\" href=\"/assets/mark.svg\">\n\
         <link rel=\"stylesheet\" href=\"/assets/site.css\">\n</head>\n<body>\n<main>\n\
         <h1>Not found</h1>\n<p>There is no page here. <a href=\"/\">Back to {n}</a>.</p>\n\
         </main>\n</body>\n</html>\n",
        n = esc(name)
    );
    vec![
        Out::file("site/_headers", headers),
        Out::file("site/_redirects", redirects(repo, name)),
        Out::file("site/404.html", not_found),
        Out::file(
            "site/robots.txt",
            format!("User-agent: *\nAllow: /\nSitemap: {home}/sitemap.xml\n"),
        ),
        Out::file(
            "site/sitemap.xml",
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n\
                 <url><loc>{home}/</loc></url>\n</urlset>\n"
            ),
        ),
        Out::file(
            "site/.well-known/security.txt",
            format!(
                "Contact: {repo}/security/advisories/new\nPolicy: {repo}/blob/main/SECURITY.md\n\
                 Canonical: {home}/.well-known/security.txt\nExpires: {SECURITY_TXT_EXPIRES}\nPreferred-Languages: en\n"
            ),
        ),
    ]
}

/// `/install.sh` sends to the release's shell installer, so the one-liner never goes stale.
/// Workers static assets accept an external target in `_redirects`; 302 because "latest" moves.
fn redirects(repo: &str, name: &str) -> String {
    format!(
        "# Generated by `cargo xtask generate` (xtask/src/site.rs). Do not edit.\n/install.sh {} 302\n",
        readme_agents_md::installer_url(repo, name)
    )
}

/// `TAGLINE` from `scripts/theme-tagline-gen.py`, so the page, the README header and the logo agree.
fn tagline(root: &Path) -> Result<String> {
    let src = std::fs::read_to_string(root.join("scripts/theme-tagline-gen.py"))
        .context("reading scripts/theme-tagline-gen.py")?;
    src.lines()
        .find_map(|l| l.strip_prefix("TAGLINE = \""))
        .and_then(|l| l.strip_suffix('"'))
        .map(str::to_owned)
        .context("TAGLINE in scripts/theme-tagline-gen.py")
}

/// The generated block of the app's tokens, with the font paths pointed at `assets/fonts/`.
fn css(tokens: &str) -> Result<String> {
    let begin = tokens
        .find("/* BEGIN generated")
        .context("tokens.css begin marker")?;
    let end = tokens
        .find("/* END generated")
        .context("tokens.css end marker")?;
    let tail = tokens[end..]
        .find("/* roles")
        .context("tokens.css roles marker")?;
    let generated = &tokens[begin..end];
    let roles = &tokens[end + tail..];
    let body = format!("{generated}{roles}").replace("/static/fonts/", "fonts/");
    Ok(format!(
        "/* Generated by `cargo xtask generate` from the app's tokens.css. Do not edit. */\n{body}\n{PAGE_CSS}"
    ))
}

/// The text before the first `##`, the body of "What it is not" style sections, and the
/// "How it is made" section, from `docs/why.md`.
fn split_why(why: &str) -> (&str, (String, String)) {
    let mut parts = why.split("\n## ");
    let head = parts.next().unwrap_or_default();
    let lead = head.split_once('\n').map_or("", |(_, rest)| rest);
    let (mut why_body, mut made) = (String::new(), String::new());
    for p in parts {
        let (title, body) = p.split_once('\n').unwrap_or((p, ""));
        if title.starts_with("How it is made") {
            made.push_str(body);
        } else {
            let _ = write!(why_body, "\n### {title}\n{body}\n");
        }
    }
    (lead, (why_body, made))
}

/// One row per feature.
fn features() -> String {
    let mut rows: Vec<(&str, String)> = Vec::new();
    let mut line = |head: &'static str, text: String| rows.push((head, text));
    let tabs: Vec<String> = gitterrace_web::stats_tab_labels()
        .iter()
        .map(|t| esc(&t.to_lowercase()))
        .collect();
    line("Insights.", format!("{}.", sentence(&tabs)));
    let mut presets: Vec<String> = Vec::new();
    for e in crate::docs_md::starter_entries()
        .iter()
        .filter(|e| e.table == "send")
    {
        let label = esc(&e.label);
        if !presets.contains(&label) {
            presets.push(label);
        }
    }
    line(
        "Send to.",
        format!(
            "Hand a selection on with its context: {}.",
            sentence(&presets)
        ),
    );
    line(
        "Actions.",
        gitterrace_actions::schema::ACTION_SUMMARY.to_owned(),
    );
    let mut s = String::new();
    for (head, text) in rows {
        let _ = writeln!(s, "<li><b>{head}</b> {text}</li>");
    }
    s
}

/// The shots as a two-column gallery of equal tiles, each with its caption.
fn gallery(root: &Path) -> Result<String> {
    let mut s = String::from("<div class=\"shots\">\n");
    for (name, alt, cap) in SHOTS {
        let _ = writeln!(
            s,
            "<figure>{}<figcaption><b>{cap}.</b> {}</figcaption></figure>",
            shot_html(root, name, alt, cap)?,
            esc(alt.split_once(": ").map_or(alt, |(_, rest)| rest))
        );
    }
    s.push_str("</div>\n");
    Ok(s)
}

/// One row per command, the README block's rows, as a definition list so long text wraps
/// instead of running off the edge.
fn commands() -> String {
    let rows = readme_agents_md::commands_rows();
    let mut s = String::from("<dl class=\"cmds\">");
    for (a, b) in rows {
        let about = b.split(" [--").next().unwrap_or(&b);
        let _ = write!(
            s,
            "<dt><code>{}</code></dt><dd>{}</dd>",
            esc(&a),
            esc(about)
        );
    }
    s.push_str("</dl>\n");
    s
}

fn sentence(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{}, and {last}", head.join(", ")),
    }
}

/// The screenshots in `docs/screenshots/`, in gallery order (pairs side by side): file stem,
/// alt text ("Caption: what it shows"), caption. The site and the README both read this list,
/// so a name here without its PNG, or a PNG not listed here, fails generation.
pub const SHOTS: [(&str, &str, &str); 6] = [
    (
        "overview",
        "Browse: the repository page with README, last commit, file list and key numbers.",
        "Browse",
    ),
    (
        "commit",
        "Commit: the message, the changed files and a highlighted diff.",
        "Commit",
    ),
    (
        "blame",
        "Blame: who last changed each line, with the commit and its age beside the code.",
        "Blame",
    ),
    (
        "send",
        "Send to: marked lines with their context, ready for tendir, mail, an issue or a merge request.",
        "Send to",
    ),
    (
        "insights",
        "Hotspots: the files that change most, against how many people touch them.",
        "Hotspots",
    ),
    (
        "ownership",
        "Ownership: bus factor, solo and stale lines, and who holds each directory.",
        "Ownership",
    ),
];

const SHOT_DIR: &str = "docs/screenshots";

/// One `Out` per PNG, copied to `site/assets/screenshots/`. Fails on a missing file and on a PNG
/// in the folder that nothing references.
fn shot_files(root: &Path) -> Result<Vec<Out>> {
    let mut want: Vec<String> = SHOTS
        .iter()
        .flat_map(|(n, ..)| ["light", "dark"].map(|c| format!("{n}-{c}.png")))
        .collect();
    want.sort();
    let mut have: Vec<String> = std::fs::read_dir(root.join(SHOT_DIR))
        .with_context(|| format!("reading {SHOT_DIR} (run `make site-shots`)"))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| Path::new(f).extension().is_some_and(|e| e == "png"))
        .collect();
    have.sort();
    if let Some(f) = want.iter().find(|f| !have.contains(f)) {
        anyhow::bail!(
            "{SHOT_DIR}/{f} is named by the site and README but does not exist (make site-shots)"
        );
    }
    if let Some(f) = have.iter().find(|f| !want.contains(f)) {
        anyhow::bail!(
            "{SHOT_DIR}/{f} is not referenced: add it to SHOTS in xtask/src/site.rs or delete it"
        );
    }
    // A WebP made by `make site-render` is copied along when it sits next to its PNG.
    let webps = want.iter().map(|f| f.replace(".png", ".webp"));
    want.iter()
        .cloned()
        .chain(webps.filter(|f| root.join(SHOT_DIR).join(f).exists()))
        .map(|f| {
            let path = format!("{SHOT_DIR}/{f}");
            let bytes =
                std::fs::read(root.join(&path)).with_context(|| format!("reading {path}"))?;
            Ok(Out::bytes(format!("site/assets/screenshots/{f}"), bytes))
        })
        .collect()
}

/// Width and height in CSS pixels of a 2x PNG, from its header.
fn shot_size(root: &Path, file: &str) -> Result<(u32, u32)> {
    let path = format!("{SHOT_DIR}/{file}");
    let b = std::fs::read(root.join(&path)).with_context(|| format!("reading {path}"))?;
    anyhow::ensure!(b.len() > 24 && &b[1..4] == b"PNG", "{path} is not a PNG");
    let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
    let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
    Ok((w / 2, h / 2))
}

/// One shot as a tile link plus a full-size view, for the caller's single `<figure>`. The view
/// is a `:target` overlay, since the site CSP runs no script; clicking it goes back to
/// `#features`. Its image is a decorative copy (empty alt) so the description is read once,
/// and `tabindex="-1"` lets the browser focus it when the fragment opens it.
fn shot_html(root: &Path, name: &str, alt: &str, cap: &str) -> Result<String> {
    // Every tile is the same size so the rows line up; a different size fails generation.
    for c in ["light", "dark"] {
        let got = shot_size(root, &format!("{name}-{c}.png"))?;
        anyhow::ensure!(
            got == (1280, 800),
            "{SHOT_DIR}/{name}-{c}.png is {}x{} CSS px, the site needs 1280x800 (a 2560x1600 PNG)",
            got.0,
            got.1
        );
    }
    // A WebP next to the PNG is offered first; only the source matching the colour scheme loads.
    let source = |c: &str, media: &str| {
        let base = format!("assets/screenshots/{name}-{c}");
        let webp = root
            .join(SHOT_DIR)
            .join(format!("{name}-{c}.webp"))
            .exists();
        let mut s = String::new();
        if webp {
            let _ = write!(
                s,
                "<source{media} type=\"image/webp\" srcset=\"{base}.webp\">"
            );
        }
        let _ = write!(s, "<source{media} srcset=\"{base}.png\">");
        s
    };
    let pic = |alt: &str| {
        format!(
            "<picture>{}{}<img src=\"assets/screenshots/{name}-light.png\" alt=\"{}\" width=\"1280\" height=\"800\" loading=\"lazy\"></picture>",
            source("dark", " media=\"(prefers-color-scheme: dark)\""),
            source("light", ""),
            esc(alt)
        )
    };
    Ok(format!(
        "<a href=\"#shot-{name}\">{}</a>\
         <a class=\"lightbox\" id=\"shot-{name}\" href=\"#features\" aria-label=\"Close {}\" tabindex=\"-1\">{}</a>",
        pic(alt),
        esc(cap),
        pic("")
    ))
}

fn md(text: &str) -> String {
    let mut out = String::new();
    html::push_html(&mut out, Parser::new_ext(text, Options::empty()));
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One figure per shot, a picture inside it, and a lightbox that is labelled, takes focus
    /// and does not announce its image twice.
    #[test]
    fn gallery_has_one_figure_per_shot_and_an_accessible_lightbox() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let g = gallery(&root).expect("gallery");
        assert!(!g.contains("<figure><figure"), "nested figure");
        assert_eq!(g.matches("<figure>").count(), SHOTS.len());
        assert_eq!(g.matches("<picture>").count(), SHOTS.len() * 2);
        assert_eq!(g.matches("tabindex=\"-1\"").count(), SHOTS.len());
        assert_eq!(g.matches("aria-label=\"Close ").count(), SHOTS.len());
    }

    #[test]
    fn redirect_target_is_the_installer_url() {
        let r = redirects("https://github.com/o/r", "r");
        assert!(r.contains(
            "/install.sh https://github.com/o/r/releases/latest/download/r-installer.sh 302"
        ));
    }

    #[test]
    fn a_missing_or_unreferenced_screenshot_fails() {
        let dir = std::env::temp_dir().join(format!("gt-shots-{}", std::process::id()));
        let shots = dir.join(SHOT_DIR);
        std::fs::create_dir_all(&shots).expect("mkdir");
        let err = shot_files(&dir)
            .err()
            .expect("empty folder fails")
            .to_string();
        assert!(err.contains("does not exist"), "{err}");
        for (n, ..) in SHOTS {
            for c in ["light", "dark"] {
                std::fs::write(shots.join(format!("{n}-{c}.png")), b"x").expect("write");
            }
        }
        assert!(shot_files(&dir).is_ok());
        std::fs::write(shots.join("stray.png"), b"x").expect("write");
        let err = shot_files(&dir).err().expect("stray fails").to_string();
        assert!(err.contains("not referenced"), "{err}");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
