//! `site/index.html` and `site/assets/`: the website (landing page, docs, changelog and legal
//! pages), written from the repo's own sources (Cargo.toml facts, clap, the registries,
//! `docs/why.md`, the brand SVGs and the vendored theme). A fact is never typed here, so `check`
//! fails when the page is stale.

use std::fmt::Write as _;
use std::path::Path;

use crate::facts::Workspace;
use crate::out::Out;
use crate::readme_agents_md;
use anyhow::{Context as _, Result};

mod bar;
mod changelog;
pub mod disk;
mod docs;
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

const PAGE_CSS: &str = "\
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:var(--text-md)/var(--leading) var(--sans)}
main{max-width:760px;margin:0 auto;padding:var(--space-8) var(--page-pad) var(--space-16)}
a{color:var(--accent-ink)}
:focus-visible{outline:2px solid var(--accent-ink);outline-offset:2px}
main:focus{outline:none}
.skip{position:absolute;left:var(--space-2);top:-100px;padding:var(--space-2) var(--space-3)}
.skip{background:var(--bg);border:var(--border);border-radius:var(--radius);z-index:20}
.skip:focus{top:var(--space-2)}
h1,h2{font-family:var(--serif);line-height:var(--leading-tight);font-weight:var(--weight-semibold)}
h2{font-size:var(--text-xl);margin:var(--space-16) 0 var(--space-3)}
h3{font-size:var(--text-lg);margin:var(--space-8) 0 var(--space-2)}
.top{color-scheme:dark;background:var(--header);border-bottom:var(--border)}
.bar{max-width:1200px;margin:0 auto;height:56px;padding:0 var(--space-6);display:flex;
align-items:center;gap:var(--space-6)}
.top a{color:var(--muted);font-size:14.5px;font-weight:500;text-decoration:none}
.top a:hover{color:var(--fg)}
.top .brand{display:flex;align-items:center;gap:var(--space-2);margin-right:auto;color:var(--fg);
font-size:18px;font-weight:var(--weight-semibold);letter-spacing:-.01em}
.top .brand img{width:24px;height:24px}
@media (max-width:640px){.bar{gap:var(--space-4);padding:0 var(--page-pad)}}
.gh{color:var(--muted)}
.gh:hover,.gh:focus-visible{color:var(--fg)}
.gh svg{fill:currentColor}
.hero{padding:var(--space-4) 0}
.hero h1{margin:0}
.hero img{max-width:100%;height:auto}
.hero img.on-light{display:block}
.tag{font-family:var(--serif);font-size:var(--text-lg);color:var(--muted);margin:var(--space-4) 0 0}
pre,code{font-family:var(--mono);font-size:var(--text-sm)}
pre{background:var(--soft);border:var(--border);border-radius:var(--radius);\
padding:var(--space-3) var(--space-4);overflow-x:auto}
:not(pre)>code{background:var(--soft);padding:1px var(--space-1);border-radius:var(--radius)}
:not(pre)>code{overflow-wrap:anywhere}
.cmds{display:grid;grid-template-columns:max-content 1fr;gap:var(--space-1) var(--space-4);\
margin:0}.cmds dt code{background:none;padding:0;white-space:nowrap}.cmds dd{margin:0;\
color:var(--muted)}
@media (max-width:640px){.cmds{grid-template-columns:1fr}.cmds dd{margin-bottom:var(--space-2)}}
main table{display:block;width:max-content;max-width:100%;overflow:auto;border-spacing:0}
main table{border-collapse:collapse;margin:0 0 var(--space-4)}
main th,main td{padding:6px 13px;border:1px solid var(--line)}
main td code,main th code{white-space:nowrap;overflow-wrap:normal}
main td:not(:has(> code:only-child))::before{content:'';display:block;width:16ch}
main th{font-weight:600;text-align:left}
main tr:nth-child(2n){background:var(--soft)}
ul.features{list-style:none;padding:0;margin:0}
ul.features li{padding:var(--space-2) 0;border-top:var(--border)}
ul.features b{font-weight:var(--weight-semibold)}
.shots{display:grid;grid-template-columns:1fr 1fr;gap:var(--space-6) var(--space-4);\
margin:var(--space-8) 0}
@media (max-width:640px){.shots{grid-template-columns:1fr}}
figure{margin:0}
figure img,.lightbox img{width:100%;height:auto;border:var(--border);border-radius:var(--radius)}
figure img.on-light,.lightbox img.on-light{display:block}
.lightbox{display:none}
.lightbox:target{display:flex;position:fixed;inset:0;z-index:10;align-items:center;\
justify-content:center;padding:var(--space-8);background:color-mix(in srgb,var(--bg) 97%,\
transparent);cursor:zoom-out}
.lightbox:target .shot{max-width:min(1280px,100%)}
.demo{margin-top:var(--space-6)}
figcaption{font-size:var(--text-xs);color:var(--muted);margin-top:var(--space-2);line-height:1.4}\
figcaption b{color:var(--fg);font-weight:var(--weight-semibold)}
footer{display:flex;flex-wrap:wrap;gap:var(--space-4);font-size:var(--text-sm)}
.gh,.theme{line-height:0}
.theme{width:24px;height:24px;padding:0;display:grid;place-items:center;\
background:none;border:0;color:var(--muted);cursor:pointer}
.theme:hover,.theme:focus-visible{color:var(--fg)}
.theme svg{width:24px;height:24px;overflow:visible;fill:none;stroke:currentColor;stroke-width:2;\
stroke-linecap:round;stroke-linejoin:round;display:none}
.theme svg *{vector-effect:non-scaling-stroke}
.theme .fill{fill:currentColor;stroke:none}
.theme .i-auto{display:block}
:root[data-theme=light] .theme .i-auto,:root[data-theme=dark] .theme .i-auto{display:none}
:root[data-theme=light] .theme .i-light,:root[data-theme=dark] .theme .i-dark{display:block}
.on-dark{display:none}
@media (prefers-color-scheme:dark){:root:not([data-theme=light]) img.on-light{display:none}\
:root:not([data-theme=light]) img.on-dark{display:block}}
:root[data-theme=dark] img.on-light{display:none}
:root[data-theme=dark] img.on-dark{display:block}
.tab-radio{position:absolute;opacity:0;pointer-events:none}
.tab-label{display:inline-block;padding:var(--space-1) var(--space-3);border:var(--border)}
.tab-label{border-bottom:0;border-radius:var(--radius) var(--radius) 0 0;color:var(--muted)}
.tab-label{cursor:pointer;font-family:var(--mono);font-size:var(--text-sm)}
.tab-radio:checked+.tab-label{color:var(--fg);background:var(--soft)}
.tab-radio:checked+.tab-label{position:relative;margin-bottom:-1px}
.tab-radio:checked+.tab-label{border-bottom:1px solid var(--soft)}
.tab-radio:focus-visible+.tab-label{outline:2px solid var(--accent-ink)}
.tabs pre{margin-top:0;border-top-left-radius:0}
.tab-panel{display:none}
#install-curl:checked~#panel-curl,#install-brew:checked~#panel-brew{display:block}
#install-cargo:checked~#panel-cargo{display:block}
footer{margin-top:var(--space-16);padding-top:var(--space-4);border-top:var(--border);\
color:var(--faint)}
.outline{display:none}
.outline .outline-q,.outline .outline-none{display:none}
.outline ol{list-style:none;margin:0;padding:0}
.outline h2{display:flex;align-items:center;gap:var(--space-3);margin:0 0 var(--space-2);
font:var(--weight-regular) var(--text-2xs)/1.4 var(--mono);text-transform:uppercase;
letter-spacing:.12em;color:var(--muted)}
.outline h2::after{content:\"\";flex:1;border-top:var(--border)}
.outline a{display:block;padding:3px var(--space-3);border-left:2px solid var(--line);
color:var(--muted);overflow-wrap:anywhere;font-size:var(--text-sm);text-decoration:none}
.outline a:hover{color:var(--fg)}
.outline a.on{color:var(--fg);font-weight:var(--weight-semibold);border-left-color:var(--accent)}
.outline li.d1>a{padding-left:var(--space-6)}
@media (min-width:1100px){.docgrid{display:grid;grid-template-columns:minmax(0,760px) 220px;
gap:var(--space-8);justify-content:center;padding:0 var(--page-pad)}
.docgrid>main{max-width:none;margin:0;padding-inline:0}
.outline{display:block;position:sticky;top:var(--space-4);align-self:start;
margin-top:var(--space-8);max-height:calc(100vh - 2*var(--space-4));overflow:auto}}
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
         <link rel=\"stylesheet\" href=\"assets/site.css\">\n<script>{THEME_INIT_JS}</script>\
         \n</head>\n",
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
    let mut h = head(&pkg.name, &pkg.description, home);
    h.push_str(&body_open(&pkg.name, repo, &ws.demo, false));
    let _ = write!(
        h,
        "<div class=\"hero\">\n<h1><img class=\"on-light\" src=\"assets/logo-light.svg\" \
         alt=\"{n}\" width=\"540\" height=\"120\" loading=\"lazy\">\n\
         <img class=\"on-dark\" src=\"assets/logo-dark.svg\" alt=\"\" width=\"540\" height=\"120\" \
         loading=\"lazy\"></h1>\n<p class=\"tag\">{tag}</p>\n</div>\n",
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
    landing_footer(root, &mut h, repo, pkg)?;
    outs.push(Out::file("site/index.html", h));
    let (pages, paths) = docs::pages(root, home, repo, &pkg.name, &ws.demo)?;
    outs.extend(pages);
    outs.extend(host_files(home, repo, &pkg.name, &ws.demo, &paths));
    Ok(outs)
}

/// The landing page's footer and closing tags: the shared footer plus the licence and the
/// version, which links to its changelog entry.
fn landing_footer(
    root: &Path,
    h: &mut String,
    repo: &str,
    pkg: &crate::facts::Package,
) -> Result<()> {
    let changelog_text =
        std::fs::read_to_string(root.join("CHANGELOG.md")).context("read CHANGELOG.md")?;
    let _ = write!(
        h,
        "{footer}\
         </main>\n<script>{THEME_TOGGLE_JS}</script>\n<script>{LIGHTBOX_JS}</script>\n</body>\
         \n</html>\n",
        footer = footer(
            repo,
            &format!(
                "<a href=\"{r}/blob/main/LICENSE\">{lic}</a>\n<a href=\"/{log}\">v{ver}</a>",
                r = esc(repo),
                lic = esc(&pkg.license),
                ver = esc(&pkg.version),
                log = changelog::version_href(&changelog_text, &pkg.version),
            ),
        ),
    );
    Ok(())
}

/// The footer of every page: the links all pages share, then `extra` (HTML, may be empty) for
/// what only this page has. One builder, so a page cannot drift from the others.
pub(crate) fn footer(repo: &str, extra: &str) -> String {
    let repo = esc(repo);
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!("{extra}\n")
    };
    let mut legal = String::new();
    for (rel, label) in crate::docs_index::LEGAL {
        let _ = writeln!(legal, "<a href=\"/{}\">{label}</a>", docs::legal_path(rel));
    }
    format!(
        "<footer>\n<a href=\"{repo}\">Repository</a>\n<a href=\"/docs/\">Docs</a>\n\
         <a href=\"/changelog/\">Changelog</a>\n{extra}{legal}</footer>\n"
    )
}

/// Every page opens its body the same way: a skip link as the first Tab stop, the bar, then the
/// main landmark the link targets. `tabindex="-1"` lets the browser move focus there, not only
/// scroll. `grid` opens the two-column grid a docs page with an outline rail sits in.
pub(crate) fn body_open(name: &str, repo: &str, demo: &str, grid: bool) -> String {
    format!(
        "<body>\n<a class=\"skip\" href=\"#main\">Skip to content</a>\n{bar}{grid}\
         <main id=\"main\" tabindex=\"-1\">\n",
        bar = bar::bar(name, repo, demo),
        grid = if grid {
            "<div class=\"docgrid\">\n"
        } else {
            ""
        },
    )
}

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
fn host_files(home: &str, repo: &str, name: &str, demo: &str, pages: &[String]) -> Vec<Out> {
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
    let not_found = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
         <meta name=\"color-scheme\" content=\"light dark\">\n\
         <meta name=\"robots\" content=\"noindex\">\n<title>Not found - {n}</title>\n\
         <link rel=\"icon\" type=\"image/svg+xml\" href=\"/assets/mark.svg\">\n\
         <link rel=\"stylesheet\" href=\"/assets/site.css\">\n\
         <script>{THEME_INIT_JS}</script>\n</head>\n{open}\
         <h1>Not found</h1>\n<p>There is no page here. <a href=\"/\">Back to {n}</a>.</p>\n\
         {footer}</main>\n<script>{THEME_TOGGLE_JS}</script>\n</body>\n</html>\n",
        n = esc(name),
        open = body_open(name, repo, demo, false),
        footer = footer(repo, ""),
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

/// `TAGLINE` from `scripts/theme-tagline-gen.py`, so the page, the README header and the logo
/// agree.
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
