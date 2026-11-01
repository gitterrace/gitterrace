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
header{padding:var(--space-8) 0 var(--space-4)}
header img{display:block;max-width:100%;height:auto}
.tag{font-family:var(--serif);font-size:var(--text-lg);color:var(--muted);margin:var(--space-4) 0 0}
pre,code{font-family:var(--mono);font-size:var(--text-sm)}
pre{background:var(--soft);border:var(--border);border-radius:var(--radius);padding:var(--space-3) var(--space-4);overflow-x:auto}
:not(pre)>code{background:var(--soft);padding:1px var(--space-1);border-radius:var(--radius)}
.cmds{display:grid;grid-template-columns:max-content 1fr;gap:var(--space-1) var(--space-4);margin:0}.cmds dt code{background:none;padding:0;white-space:nowrap}.cmds dd{margin:0;color:var(--muted)}
@media (max-width:640px){.cmds{grid-template-columns:1fr}.cmds dd{margin-bottom:var(--space-2)}}
ul.features{list-style:none;padding:0;margin:0}
ul.features li{padding:var(--space-2) 0;border-top:var(--border)}
ul.features b{font-weight:var(--weight-semibold)}
figure{margin:var(--space-6) 0}
.shot{display:flex;align-items:center;justify-content:center;min-height:140px;padding:var(--space-4);text-align:center;color:var(--faint);background:var(--soft);border:1px dashed var(--line-strong);border-radius:var(--radius);font-size:var(--text-sm)}
figcaption{font-size:var(--text-xs);color:var(--faint);margin-top:var(--space-1)}
nav.links,footer{display:flex;flex-wrap:wrap;gap:var(--space-4);font-size:var(--text-sm)}
footer{margin-top:var(--space-16);padding-top:var(--space-4);border-top:var(--border);color:var(--faint)}
";

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

    let repo = &pkg.repository;
    let home = pkg.homepage.trim_end_matches('/');
    let mut h = head(&pkg.name, &pkg.description, home);
    let _ = write!(
        h,
        "<header>\n<picture>\n<source media=\"(prefers-color-scheme: dark)\" srcset=\"assets/logo-dark.svg\">\n\
         <img src=\"assets/logo-light.svg\" alt=\"{n}\" width=\"540\" height=\"120\">\n</picture>\n\
         <p class=\"tag\">{tag}</p>\n{demo}</header>\n",
        n = esc(&pkg.name),
        tag = esc(&tagline(root)?),
        demo = if ws.demo.is_empty() {
            String::new()
        } else {
            format!(
                "<p class=\"demo\"><a href=\"{}\">Try the live demo</a>, no install needed.</p>\n",
                esc(&ws.demo)
            )
        },
    );
    let _ = write!(
        h,
        "<section>\n<h2>Install</h2>\n{}</section>\n",
        md(&readme_agents_md::install_md(ws))
    );
    let _ = write!(
        h,
        "<section>\n<h2>What it does</h2>\n<ul class=\"features\">\n{}</ul>\n<h3>Commands</h3>\n{}{}</section>\n",
        features(),
        commands(),
        shots()
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

fn features() -> String {
    let mut s = String::new();
    let line = |s: &mut String, head: &str, text: String| {
        let _ = writeln!(s, "<li><b>{head}</b> {text}</li>");
    };
    let tabs: Vec<String> = gitterrace_web::stats_tab_labels()
        .iter()
        .map(|t| esc(&t.to_lowercase()))
        .collect();
    line(&mut s, "Insights.", format!("{}.", sentence(&tabs)));
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
        &mut s,
        "Send to.",
        format!(
            "Hand a selection on with its context: {}.",
            sentence(&presets)
        ),
    );
    line(
        &mut s,
        "Actions.",
        gitterrace_actions::schema::ACTION_SUMMARY.to_owned(),
    );
    s
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

/// Placeholders only: the screenshots are the owner's.
fn shots() -> String {
    let mut s = String::new();
    for (alt, cap) in [
        (
            "Overview: the repository page with README, last commit, file list and key numbers.",
            "Overview",
        ),
        (
            "Send to window: a selected line with its context and the list of targets.",
            "Send to window",
        ),
        (
            "Insights: activity, heatmap and hotspots for a repository.",
            "Insights",
        ),
    ] {
        let _ = writeln!(
            s,
            "<figure>\n<div class=\"shot\" role=\"img\" aria-label=\"{a}\">Screenshot placeholder: {a}</div>\n<figcaption>{cap}</figcaption>\n</figure>",
            a = esc(alt)
        );
    }
    s
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

    #[test]
    fn redirect_target_is_the_installer_url() {
        let r = redirects("https://github.com/o/r", "r");
        assert!(r.contains(
            "/install.sh https://github.com/o/r/releases/latest/download/r-installer.sh 302"
        ));
    }
}
