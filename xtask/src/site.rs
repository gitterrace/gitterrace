//! `site/index.html` and `site/assets/`: the website (landing page, docs, changelog and legal
//! pages), written from the repo's own sources (Cargo.toml facts, clap, the registries,
//! `docs/why.md`, the brand SVGs and the vendored theme). A fact is never typed here, so `check`
//! fails when the page is stale.

use std::fmt::Write as _;
use std::path::Path;

use crate::facts::Workspace;
use crate::out::Out;
use crate::readme_agents_md;
use crate::site::frame::{Body, Site, page};
use anyhow::{Context as _, Result};

mod bar;
mod changelog;
pub mod disk;
mod docs;
mod fit;
mod frame;
mod landing;
pub mod legal;
mod shots;

pub use landing::SHOTS;
#[cfg(test)]
use landing::shot_html;
use landing::{
    COMPARED, MADE, SAFETY, check_facts, commands, features, gallery, install_tabs, md, section,
    site_links, split_why, why_body,
};
use shots::check_shots;
#[cfg(test)]
use shots::readme_shots;

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
/// The two mailboxes of the site: the legal pages name `SUPPORT_EMAIL` for contact and rights
/// requests, and `SECURITY_EMAIL` for vulnerability reports, which is also `security.txt`'s
/// first Contact. The legal sources are Markdown, so a test holds their addresses to these.
#[cfg(test)]
pub(crate) const SUPPORT_EMAIL: &str = "support@gitterrace.dev";
pub(crate) const SECURITY_EMAIL: &str = "security@gitterrace.dev";
const SECURITY_TXT_EXPIRES: &str = "2027-10-31T23:00:00Z";

/// The site's own rules, after the app's tokens in `site.css`.
const PAGE_CSS: &str = include_str!("../templates/site.css");

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
    let safety = section(&sections, SAFETY)?;
    check_facts(safety)?;

    let mut outs = Vec::new();
    outs.push(Out::file("site/assets/site.css", css(&read(TOKENS)?)?));
    for (dir, file) in FONTS {
        let path = format!("shared/schwarzwald/fonts/{dir}/{file}");
        let bytes = std::fs::read(root.join(&path)).with_context(|| format!("reading {path}"))?;
        outs.push(Out::bytes(format!("site/assets/fonts/{dir}.woff2"), bytes));
        outs.push(Out::file(
            format!("site/assets/fonts/{dir}-OFL.txt"),
            read(&format!("shared/schwarzwald/fonts/{dir}/OFL.txt"))?,
        ));
    }
    for name in BRAND {
        outs.push(Out::file(
            format!("site/assets/{name}.svg"),
            read(&format!("docs/brand/{name}.svg"))?,
        ));
    }

    check_shots(root)?;

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
    let pitch = pitch(root, &pkg.description)?;
    let mut h = String::new();
    let _ = write!(
        h,
        "<div class=\"hero\">\n<h1><img class=\"on-light\" src=\"assets/logo-light.svg\" \
         alt=\"{n}\" width=\"540\" height=\"120\" loading=\"lazy\">\n\
         <img class=\"on-dark\" src=\"assets/logo-dark.svg\" alt=\"\" width=\"540\" height=\"120\" \
         loading=\"lazy\"></h1>\n<p class=\"lead\">{tag}</p>\n</div>\n",
        n = esc(&pkg.name),
        tag = esc(&pitch),
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
        install_tabs(ws)
    );
    let _ = write!(
        h,
        "<section id=\"features\">\n<h2>What it does</h2>\n<ul class=\"features\">\n{}</ul>\n{}\
        {demo}<h3>Commands</h3>\n{}</section>\n",
        features(),
        gallery(),
        commands(),
    );
    let _ = write!(
        h,
        "<section>\n<h2>{SAFETY}</h2>\n{}</section>\n",
        md(&site_links(safety, repo))
    );
    let _ = write!(
        h,
        "<section>\n<h2>Why</h2>\n{}{}</section>\n",
        md(lead),
        md(&site_links(&why_body(&sections), repo))
    );
    let _ = write!(
        h,
        "<section>\n<h2>{COMPARED}</h2>\n{}</section>\n",
        md(section(&sections, COMPARED)?)
    );
    let _ = write!(
        h,
        "<section>\n<h2>{MADE}</h2>\n{}</section>\n",
        md(section(&sections, MADE)?)
    );
    outs.push(Out::file(
        "site/index.html",
        page(
            &site,
            &Body {
                desc: &pitch,
                inner: &h,
                scripts: vec![LIGHTBOX_JS.to_owned()],
                ..Body::default()
            },
        ),
    ));
    let (pages, paths) = docs::pages(root, &site)?;
    outs.extend(pages);
    outs.extend(host_files(&site, &paths));
    Ok(outs)
}

/// The 404 page's text.
const NOT_FOUND: &str = "<h1>Not found</h1>\n<p>There is no page here. <a href=\"/\">Back to the \
home page</a>.</p>\n";

/// The one script on the page: Escape closes an open screenshot by going back to `#features`,
/// the same place a click on the overlay leads. Everything else works without it.
const LIGHTBOX_JS: &str = "addEventListener('keydown',\n\
function(e){if(e.key==='Escape'&&location.hash.indexOf('#shot-')===0)location.hash='features'})";

/// In the head, before first paint: a theme the visitor picked earlier (the app's own key, so
/// both remember one choice), else none and the system decides. No flash of the wrong scheme.
const THEME_INIT_JS: &str = "(function(){try{var t=localStorage.getItem('gitterrace-theme');\n\
if(t==='light'||t==='dark')document.documentElement.dataset.theme=t}catch(e){}})()";

/// The theme button cycles system, light and dark, as the app's does, and stores the choice.
const THEME_TOGGLE_JS: &str = "(function(){var r=document.documentElement,\n\
b=document.getElementById('theme'),o=['system','light','dark'];if(!b)return;\n\
function c(){return r.dataset.theme||'system'}\n\
function l(){b.setAttribute('aria-label','Theme: '+c());\n\
b.title='Theme: '+c()}l();b.addEventListener('click',function(){var n=o[(o.indexOf(c())+1)%3];\n\
if(n==='system')delete r.dataset.theme;else r.dataset.theme=n;\n\
try{if(n==='system')localStorage.removeItem('gitterrace-theme');else \n\
localStorage.setItem('gitterrace-theme',n)}catch(e){}l()})})()";

/// Every inline script on the page, so the CSP lists exactly these by hash.
const SCRIPTS: [&str; 3] = [THEME_INIT_JS, THEME_TOGGLE_JS, LIGHTBOX_JS];

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
    use base64::Engine as _;
    use sha2::Digest as _;
    SCRIPTS
        .iter()
        .map(|s| (*s).to_owned())
        .chain([outline_script()])
        .map(|s| {
            let sum = sha2::Sha256::digest(s.as_bytes());
            format!(
                "'sha256-{}'",
                base64::engine::general_purpose::STANDARD.encode(sum)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The files the static host reads beside the page: response headers, the 404 page, the
/// crawler files and the security contact. The CSP is tight because the page is static: the
/// only scripts allowed are [`SCRIPTS`], by their hashes.
fn host_files(site: &Site, pages: &[String]) -> Vec<Out> {
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

/assets/fonts/*
  Cache-Control: public, max-age=31536000, immutable

/assets/screenshots/*
  Cache-Control: public, max-age=86400
",
        script = script_hashes()
    );
    let not_found = page(
        site,
        &Body {
            title: "Not found",
            desc: "There is no page at this address.",
            path: "404.html",
            inner: NOT_FOUND,
            noindex: true,
            ..Body::default()
        },
    );
    vec![
        Out::file("site/_headers", &headers),
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
                 <url><loc>{home}/</loc></url>\n{pages}</urlset>\n"
            ),
        ),
        Out::file(
            "site/.well-known/security.txt",
            format!(
                "Contact: mailto:{SECURITY_EMAIL}\n\
                 Contact: {repo}/security/advisories/new\n\
                 Policy: {repo}/blob/main/SECURITY.md\n\
                 Canonical: {home}/.well-known/security.txt\n\
                 Expires: {SECURITY_TXT_EXPIRES}\n\
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
/// `TAGLINE` in `scripts/theme-tagline-gen.py` says something else.
fn pitch(root: &Path, desc: &str) -> Result<String> {
    let src = std::fs::read_to_string(root.join("scripts/theme-tagline-gen.py"))
        .context("reading scripts/theme-tagline-gen.py")?;
    let tag = src
        .lines()
        .find_map(|l| l.strip_prefix("TAGLINE = \""))
        .and_then(|l| l.strip_suffix('"'))
        .context("TAGLINE in scripts/theme-tagline-gen.py")?;
    anyhow::ensure!(
        tag == desc,
        "TAGLINE in scripts/theme-tagline-gen.py is not the package description {desc:?}: \
         set it and run the script to redraw the README header"
    );
    Ok(desc.to_owned())
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
        "/* Generated by `cargo xtask generate` from the app's tokens.css. Do not edit. \
        */\n{body}\n{PAGE_CSS}"
    ))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod built;
#[cfg(test)]
mod prose;
#[cfg(test)]
mod tests;
