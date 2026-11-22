//! `target/site/`: the website (landing page, docs, changelog and legal pages), built from the
//! repo's own sources (Cargo.toml facts, clap, the registries, `docs/why.md`, the brand SVGs, the
//! screenshots and the vendored theme). A fact is never typed here, and the site is never
//! tracked, so what `check` guards is always a fresh build.

use std::fmt::Write as _;
use std::path::Path;

use crate::facts::Workspace;
use crate::out::Out;
use crate::readme_agents_md;
use crate::site::frame::{Body, Site, page};
use anyhow::{Context as _, Result};

mod bar;
mod changelog;
mod diagram;
pub mod disk;
mod docnav;
mod docs;
mod fit;
mod frame;
mod guards;
mod landing;
pub mod legal;
mod shots;

pub use landing::SHOTS;
pub(crate) use landing::sizes;
use shots::check_shots;

const FONTS: [(&str, &str); 3] = [
    ("fraunces", "fraunces-latin-wght-normal.woff2"),
    ("bricolage-grotesque", "bricolage-grotesque-latin-wght-normal.woff2"),
    ("jetbrains-mono", "jetbrains-mono-latin-wght-normal.woff2"),
];
/// Where `generate` writes the website, relative to the repo root.
pub const SITE_DIR: &str = "target/site";

/// The path of `rel` in the built site.
/// Every `attr="value"` of `html`, in order.
fn attrs<'a>(html: &'a str, attr: &str) -> Vec<&'a str> {
    let pat = format!(" {attr}=\"");
    html.split(&pat).skip(1).filter_map(|r| r.split_once('"').map(|v| v.0)).collect()
}

pub fn at(rel: &str) -> String {
    format!("{SITE_DIR}/{rel}")
}
const BRAND: [&str; 5] = ["logo-light", "logo-dark", "mark-light", "mark-dark", "mark"];
const TOKENS: &str = "crates/gitterrace-web/static/tokens.css";
/// The app's syntax colours, which the site's code blocks share.
const SYNTAX_CSS: &str = "crates/gitterrace-web/static/syntax.css";
/// The two mailboxes of the site: the legal pages name `SUPPORT_EMAIL` for contact and rights
/// requests, and `SECURITY_EMAIL` for vulnerability reports, which is also `security.txt`'s
/// first Contact. The legal sources are Markdown, so a test holds their addresses to these.
#[cfg(test)]
pub(crate) const SUPPORT_EMAIL: &str = "support@gitterrace.dev";
pub(crate) const SECURITY_EMAIL: &str = "security@gitterrace.dev";

/// The site's own rules, after the app's tokens in `site.css`.
const PAGE_CSS: &str = include_str!("../templates/site.css");

/// Whether `out` is a file of the built site rather than a tracked file.
pub fn is_site(out: &Out) -> bool {
    match out {
        Out::File { path, .. } | Out::Bytes { path, .. } => {
            path.strip_prefix(SITE_DIR).is_some_and(|p| p.starts_with('/'))
        }
        Out::Block { .. } => false,
    }
}

/// Writes the site files of `outs` under `dest`, into an emptied `dest/SITE_DIR`, so no file of
/// an earlier build survives. Returns that dir.
pub fn build(dest: &Path, outs: Vec<Out>) -> Result<std::path::PathBuf> {
    let site = dest.join(SITE_DIR);
    match std::fs::remove_dir_all(&site) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(e).with_context(|| format!("emptying {}", site.display()));
        }
        _ => {}
    }
    let outs: Vec<Out> = outs.into_iter().filter(is_site).collect();
    crate::out::apply(dest, outs, crate::out::Mode::Write)?;
    Ok(site)
}

pub fn render(root: &Path, ws: &Workspace) -> Result<Vec<Out>> {
    let read = |rel: &str| -> Result<String> {
        std::fs::read_to_string(root.join(rel)).with_context(|| format!("reading {rel}"))
    };
    let pkg = ws.packages.get("gitterrace").context("package gitterrace")?;
    let mut outs = Vec::new();
    outs.push(Out::file(at("assets/site.css"), css(&read(TOKENS)?, &read(SYNTAX_CSS)?)?));
    for (dir, file) in FONTS {
        let path = format!("vendor/schwarzwald/kit/fonts/{dir}/{file}");
        let bytes = std::fs::read(root.join(&path)).with_context(|| format!("reading {path}"))?;
        outs.push(Out::bytes(at(&format!("assets/fonts/{dir}.woff2")), bytes));
        outs.push(Out::file(
            at(&format!("assets/fonts/{dir}-OFL.txt")),
            read(&format!("vendor/schwarzwald/kit/fonts/{dir}/OFL.txt"))?,
        ));
    }
    // Committed in docs/brand, so the built site links a file that exists. Under icons/, since
    // site/assets/*.png is the ignored, deploy-rendered link preview.
    let icon = "docs/brand/apple-touch-icon.png";
    let bytes = std::fs::read(root.join(icon)).with_context(|| format!("reading {icon}"))?;
    outs.push(Out::bytes(at("assets/icons/apple-touch-icon.png"), bytes));
    for name in BRAND {
        outs.push(Out::file(
            at(&format!("assets/{name}.svg")),
            read(&format!("docs/brand/{name}.svg"))?,
        ));
    }

    check_shots(root)?;
    outs.extend(shots::site_shots(root)?);

    let repo = &pkg.repository;
    let home = pkg.homepage.trim_end_matches('/');
    let log = read(docs::CHANGELOG)?;
    let version_href = changelog::version_href(&log, &pkg.version);
    let site = Site {
        home,
        repo,
        name: &pkg.name,
        demo: &ws.demo,
        license: &pkg.license,
        version: &pkg.version,
        version_href: &version_href,
    };
    let pitch = pitch(&pkg.description)?;
    let h = landing::body(ws, &pitch, &docs::Links::new(repo), &read)?;
    outs.push(Out::file(
        at("index.html"),
        page(&site, &Body { desc: &pitch, inner: &h, ..Body::default() }),
    ));
    let (pages, paths) = docs::pages(root, &site)?;
    outs.extend(pages);
    outs.extend(host_files(&site, &paths, &crate::security_expires::expires(&log)?));
    Ok(version_assets(outs))
}

/// Every `/assets/<file>` a built page or stylesheet names gets `?v=` and the first hex of
/// the file's sha256. A new build of a changed file is a new URL, so `_headers` can keep
/// `/assets/*` a year without a page ever meeting a stale picture, font or stylesheet. The
/// absolute link-preview URL (`og:image`) is left alone: crawlers fetch it once, by name.
fn version_assets(outs: Vec<Out>) -> Vec<Out> {
    // Stylesheets name fonts, so they are versioned first: a stylesheet's own hash then covers
    // the bytes it is served with, and its URL changes when a font it names does.
    let fonts = sums(&outs);
    let outs: Vec<Out> = outs
        .into_iter()
        .map(|o| match o {
            Out::File { path, body } if has_ext(&path, "css") => {
                let body = versioned(&body, &fonts);
                Out::File { path, body }
            }
            other => other,
        })
        .collect();
    let all = sums(&outs);
    outs.into_iter()
        .map(|o| match o {
            // A page is fitted again: a version can push a line past the width.
            Out::File { path, body } if has_ext(&path, "html") => {
                let body = fit::fit(&versioned(&body, &all));
                Out::File { path, body }
            }
            other => other,
        })
        .collect()
}

/// The first 12 hex digits of each `/assets/` file's sha256, by its path under `assets/`.
fn sums(outs: &[Out]) -> std::collections::BTreeMap<String, String> {
    use sha2::Digest as _;
    let mut sums = std::collections::BTreeMap::new();
    for o in outs {
        let (path, bytes) = match o {
            Out::File { path, body } => (path, body.as_bytes()),
            Out::Bytes { path, body } => (path, body.as_slice()),
            Out::Block { .. } => continue,
        };
        if let Some(rel) = path.strip_prefix(&format!("{SITE_DIR}/assets/")) {
            let sum = sha2::Sha256::digest(bytes);
            let hex = sum.iter().take(6).fold(String::new(), |mut h, b| {
                let _ = write!(h, "{b:02x}");
                h
            });
            sums.insert(rel.to_owned(), hex);
        }
    }
    sums
}

/// Whether `path` ends in `.ext`, in any case.
fn has_ext(path: &str, ext: &str) -> bool {
    Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// `body` with each `/assets/<file>` that `sums` knows followed by its `?v=`.
fn versioned(body: &str, sums: &std::collections::BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(body.len() + 256);
    let mut rest = body;
    while let Some(at) = rest.find("/assets/") {
        let start = at + "/assets/".len();
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || "-_./".contains(c)))
            .unwrap_or(rest.len());
        let file = &rest[..end];
        out.push_str(file);
        rest = &rest[end..];
        if let Some(v) = sums.get(file)
            && !rest.starts_with('?')
        {
            out.push_str("?v=");
            out.push_str(v);
        }
    }
    out.push_str(rest);
    out
}

/// The 404 page's text: where the reader likely meant to go.
const NOT_FOUND: &str = include_str!("../templates/not_found.html");

/// In the head, before first paint: a theme the visitor picked earlier (the app's own key, so
/// both remember one choice), else none and the system decides. No flash of the wrong scheme.
const THEME_INIT_JS: &str = "(function(){try{var t=localStorage.getItem('gitterrace-theme');\n\
if(t==='light'||t==='dark')document.documentElement.dataset.theme=t}catch(e){}})()";

/// The theme button cycles system, light and dark, as the app's does, and stores the choice.
/// Its label names the scheme now and the next one a press picks.
/// A `<picture>` picks its source by media query alone, so each dark source's query follows
/// the choice: the system's, or always, or never.
const THEME_TOGGLE_JS: &str = "(function(){var r=document.documentElement,\n\
b=document.getElementById('theme'),o=['system','light','dark'];if(!b)return;\n\
function c(){return r.dataset.theme||'system'}\n\
function l(){var s='Theme: '+c()+', click for '+o[(o.indexOf(c())+1)%3];\n\
b.setAttribute('aria-label',s);b.title=s;\n\
var q={system:'(prefers-color-scheme: dark)',light:'not all',dark:'all'}[c()];\n\
document.querySelectorAll('picture source').forEach(function(s){s.media=q})}l();\n\
b.addEventListener('click',function(){var n=o[(o.indexOf(c())+1)%3];\n\
if(n==='system')delete r.dataset.theme;else r.dataset.theme=n;\n\
try{if(n==='system')localStorage.removeItem('gitterrace-theme');else \n\
localStorage.setItem('gitterrace-theme',n)}catch(e){}l()})})()";

/// The bar's Docs menu on a docs page: Escape closes it and puts focus back on its button, a
/// click outside or on a link closes it. It opens and closes without it, as native details.
/// "On this page" gets the same from the outline script.
pub(crate) const MENU_JS: &str = "document.querySelectorAll('details.docsmenu')\n\
.forEach(function(d){var s=d.querySelector('summary');d.addEventListener('keydown',function(e){\n\
if(e.key==='Escape'&&d.open){e.preventDefault();d.open=false;s.focus()}});\n\
d.addEventListener('click',function(e){if(e.target.closest('a'))d.open=false});\n\
document.addEventListener('click',function(e){if(d.open&&!d.contains(e.target))d.open=false})})";

/// Every inline script on the page, so the CSP lists exactly these by hash.
const SCRIPTS: [&str; 3] = [THEME_INIT_JS, THEME_TOGGLE_JS, MENU_JS];

/// The body of the outline's inline script on a docs page with a rail: the app's own script,
/// from a fresh line so its first line fits the page's line width. Its whole-line comments stay
/// in the app's file: every docs page carries this copy, so it is held to the page's length.
pub(crate) fn outline_script() -> String {
    let code: String = gitterrace_web::OUTLINE_JS
        .split_inclusive('\n')
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect();
    format!("\n{code}")
}

/// The CSP source for [`SCRIPTS`]: the sha256 of each, so exactly those run and any other,
/// inline or loaded, fails loudly in the browser instead of quietly shipping.
fn script_hashes() -> String {
    SCRIPTS
        .iter()
        .map(|s| (*s).to_owned())
        .chain([outline_script()])
        .map(|s| csp_hash(&s))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The sha256 CSP source of an inline script body.
fn csp_hash(body: &str) -> String {
    use base64::Engine as _;
    use sha2::Digest as _;
    let sum = sha2::Sha256::digest(body.as_bytes());
    format!("'sha256-{}'", base64::engine::general_purpose::STANDARD.encode(sum))
}

/// The files the static host reads beside the page: response headers, the 404 page, the
/// crawler files and the security contact. The CSP is tight because the page is static: the
/// only scripts allowed are [`SCRIPTS`], by their hashes.
fn host_files(site: &Site, pages: &[String], expires: &str) -> Vec<Out> {
    let (home, repo, name) = (site.home, site.repo, site.name);
    let pages = pages.iter().fold(String::new(), |mut s, p| {
        let _ = writeln!(s, "<url><loc>{home}/{p}</loc></url>");
        s
    });
    let headers = format!(
        "\
# Generated by `cargo xtask generate` (xtask/src/site.rs). Do not edit.
/*
  Content-Security-Policy: default-src 'none'; img-src 'self'; style-src 'self'; font-src \
  'self'; script-src {script}; base-uri 'none'; form-action 'none'; frame-ancestors 'none'
  X-Content-Type-Options: nosniff
  X-Frame-Options: DENY
  Referrer-Policy: strict-origin-when-cross-origin
  Strict-Transport-Security: max-age=63072000; includeSubDomains; preload
  Permissions-Policy: accelerometer=(), camera=(), geolocation=(), gyroscope=(), \
  magnetometer=(), microphone=(), payment=(), usb=()
  Cross-Origin-Opener-Policy: same-origin

/assets/*
  Cache-Control: public, max-age=31536000, immutable

/assets/og.png
  ! Cache-Control
  Cache-Control: no-cache
",
        script = script_hashes()
    );
    let not_found = page(
        site,
        &Body {
            title: "Not found",
            desc: "There is no page at this address: the home page, the docs and the changelog.",
            path: "404.html",
            inner: NOT_FOUND,
            noindex: true,
            ..Body::default()
        },
    );
    vec![
        Out::file(at("_headers"), &headers),
        Out::file(at("_redirects"), redirects(repo, name)),
        Out::file(at("404.html"), not_found),
        Out::file(
            at("robots.txt"),
            format!("User-agent: *\nAllow: /\nSitemap: {home}/sitemap.xml\n"),
        ),
        Out::file(
            at("sitemap.xml"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n\
                 <url><loc>{home}/</loc></url>\n{pages}</urlset>\n"
            ),
        ),
        Out::file(
            at(".well-known/security.txt"),
            format!(
                "Contact: mailto:{SECURITY_EMAIL}\n\
                 Contact: {repo}/security/advisories/new\n\
                 Policy: {repo}/blob/main/SECURITY.md\n\
                 Canonical: {home}/.well-known/security.txt\n\
                 Expires: {expires}\n\
                 Preferred-Languages: en\n"
            ),
        ),
    ]
}

/// `/install.sh` sends to the release's shell installer, so the one-liner never goes stale.
/// Workers static assets accept an external target in `_redirects`; 302 because "latest" moves.
fn redirects(repo: &str, name: &str) -> String {
    format!(
        "# Generated by `cargo xtask generate` (xtask/src/site.rs). Do not edit.\n/install.sh {} \
        302\n",
        readme_agents_md::installer_url(repo, name)
    )
}

/// The one pitch sentence: the package description, which the README, the landing page and the
/// meta description show. The README header draws it as outlined text, so generation fails when
/// `TAGLINE` in `xtask/src/tagline.rs` says something else.
fn pitch(desc: &str) -> Result<String> {
    crate::tagline::same_as(desc)?;
    Ok(desc.to_owned())
}

/// The app's syntax colours from `syntax.css` for the site's `pre` blocks: the rules after its
/// `/* syntax` comment, scoped to `pre`, with each `--md-*` alias resolved to the token it names
/// so the site needs no markdown variables. Light and dark come with the tokens.
fn syntax_css(markdown: &str) -> Result<String> {
    const SCOPE: &str = ":is(.markdown-body,.code,.diff) ";
    let vars = markdown
        .lines()
        .find(|l| l.starts_with(SCOPE.trim_end()) && l.contains("--md-k1:"))
        .context("syntax.css: no --md-* aliases")?;
    let aliases: Vec<(&str, &str)> = vars
        .split(['{', ';'])
        .filter_map(|d| d.split_once(':'))
        .filter(|(k, _)| k.starts_with("--md-"))
        .collect();
    let at = markdown.find("/* syntax").context("syntax.css: no syntax block")?;
    let mut out = String::from("/* syntax: generated from the app's syntax.css */\n");
    for line in markdown[at..].lines().skip(1) {
        if !line.contains("[class~=s-") {
            break;
        }
        let mut rule = line.replace(SCOPE, "pre ");
        for (k, v) in &aliases {
            rule = rule.replace(&format!("var({k})"), v);
        }
        if rule.contains("--md-") {
            anyhow::bail!("syntax.css: unresolved alias in {rule}");
        }
        out.push_str(&rule);
        out.push('\n');
    }
    Ok(out)
}

/// The generated block of the app's tokens, with the font paths pointed at `assets/fonts/`,
/// then the site's rules and the app's syntax colours.
fn css(tokens: &str, markdown: &str) -> Result<String> {
    let begin = tokens.find("/* BEGIN generated").context("tokens.css begin marker")?;
    let end = tokens.find("/* END generated").context("tokens.css end marker")?;
    let tail = tokens[end..].find("/* roles").context("tokens.css roles marker")?;
    let generated = &tokens[begin..end];
    let roles = &tokens[end + tail..];
    let body = format!("{generated}{roles}").replace("/static/fonts/", "/assets/fonts/");
    let all = format!("{body}\n{PAGE_CSS}{}", syntax_css(markdown)?);
    Ok(format!(
        "/* Generated by `cargo xtask generate` from the app's tokens.css. Do not edit. */\n{}",
        gitterrace_web::minify_css(&all)
    ))
}

#[cfg(test)]
mod built;
#[cfg(test)]
mod footer;
#[cfg(test)]
mod prose;
#[cfg(test)]
mod repo_links;
#[cfg(test)]
mod safety;
#[cfg(test)]
mod tests;
