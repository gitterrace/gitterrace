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

mod shots;

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
.gh{right:0;color:var(--muted)}
.gh:hover,.gh:focus-visible{color:var(--fg)}
.gh svg{fill:currentColor}
header img{max-width:calc(100% - 72px);height:auto}
header img.on-light{display:block}
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
figure img,.lightbox img{width:100%;height:auto;border:var(--border);border-radius:var(--radius)}
figure img.on-light,.lightbox img.on-light{display:block}
.lightbox{display:none}
.lightbox:target{display:flex;position:fixed;inset:0;z-index:10;align-items:center;justify-content:center;padding:var(--space-8);background:color-mix(in srgb,var(--bg) 97%,transparent);cursor:zoom-out}
.lightbox:target .shot{max-width:min(1280px,100%)}
.demo{margin-top:var(--space-6)}
figcaption{font-size:var(--text-xs);color:var(--muted);margin-top:var(--space-2);line-height:1.4}figcaption b{color:var(--fg);font-weight:var(--weight-semibold)}
footer{display:flex;flex-wrap:wrap;gap:var(--space-4);font-size:var(--text-sm)}
.gh,.theme{position:absolute;top:var(--space-8);line-height:0}
.theme{right:40px;width:24px;height:24px;padding:0;display:grid;place-items:center;background:none;border:0;color:var(--muted);cursor:pointer}
.theme:hover,.theme:focus-visible{color:var(--fg)}
.theme svg{width:24px;height:24px;overflow:visible;fill:none;stroke:currentColor;stroke-width:2;stroke-linecap:round;stroke-linejoin:round;display:none}
.theme svg *{vector-effect:non-scaling-stroke}
.theme .fill{fill:currentColor;stroke:none}
.theme .i-auto{display:block}
:root[data-theme=light] .theme .i-auto,:root[data-theme=dark] .theme .i-auto{display:none}
:root[data-theme=light] .theme .i-light,:root[data-theme=dark] .theme .i-dark{display:block}
.on-dark{display:none}
@media (prefers-color-scheme:dark){:root:not([data-theme=light]) img.on-light{display:none}:root:not([data-theme=light]) img.on-dark{display:block}}
:root[data-theme=dark] img.on-light{display:none}
:root[data-theme=dark] img.on-dark{display:block}
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
         <link rel=\"stylesheet\" href=\"assets/site.css\">\n<script>{THEME_INIT_JS}</script>\n</head>\n<body>\n<main>\n",
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

    check_shots(root)?;

    let repo = &pkg.repository;
    let home = pkg.homepage.trim_end_matches('/');
    let mut h = head(&pkg.name, &pkg.description, home);
    let _ = write!(
        h,
        "<header>\n<img class=\"on-light\" src=\"assets/logo-light.svg\" alt=\"{n}\" width=\"540\" height=\"120\" loading=\"lazy\">\n\
         <img class=\"on-dark\" src=\"assets/logo-dark.svg\" alt=\"\" width=\"540\" height=\"120\" loading=\"lazy\">\n\
         <p class=\"tag\">{tag}</p>\n{THEME_BUTTON}\n<a class=\"gh\" href=\"{r}\" aria-label=\"{n} on GitHub\" title=\"GitHub\"><svg viewBox=\"0 0 16 16\" width=\"24\" height=\"24\" aria-hidden=\"true\"><path d=\"{GH_MARK}\"/></svg></a>\n</header>\n",
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
        gallery(),
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
         <a href=\"{repo}/blob/main/LICENSE\">{lic}</a>\n<a href=\"{repo}/releases/tag/v{ver}\">v{ver}</a>\n</footer>\n\
         </main>\n<script>{THEME_TOGGLE_JS}</script>\n<script>{LIGHTBOX_JS}</script>\n</body>\n</html>\n",
        repo = esc(repo),
        lic = esc(&pkg.license),
        ver = esc(&pkg.version),
    );
    outs.push(Out::file("site/index.html", h));
    outs.extend(host_files(home, repo, &pkg.name));
    Ok(outs)
}

/// The one script on the page: Escape closes an open screenshot by going back to `#features`,
/// the same place a click on the overlay leads. Everything else works without it.
const LIGHTBOX_JS: &str = "addEventListener('keydown',function(e){if(e.key==='Escape'&&location.hash.indexOf('#shot-')===0)location.hash='features'})";

/// In the head, before first paint: a theme the visitor picked earlier (the app's own key, so
/// both remember one choice), else none and the system decides. No flash of the wrong scheme.
const THEME_INIT_JS: &str = "(function(){try{var t=localStorage.getItem('gitterrace-theme');if(t==='light'||t==='dark')document.documentElement.dataset.theme=t}catch(e){}})()";

/// The theme button cycles system, light and dark, as the app's does, and stores the choice.
const THEME_TOGGLE_JS: &str = "(function(){var r=document.documentElement,b=document.getElementById('theme'),o=['system','light','dark'];if(!b)return;function c(){return r.dataset.theme||'system'}function l(){b.setAttribute('aria-label','Theme: '+c());b.title='Theme: '+c()}l();b.addEventListener('click',function(){var n=o[(o.indexOf(c())+1)%3];if(n==='system')delete r.dataset.theme;else r.dataset.theme=n;try{if(n==='system')localStorage.removeItem('gitterrace-theme');else localStorage.setItem('gitterrace-theme',n)}catch(e){}l()})})()";

/// The theme button, the app's icons: half disc for system, sun, moon. Each viewBox is cropped
/// to its own ink plus half the stroke, so at 24 px every icon fills the box as the source-link
/// mark beside it does.
const THEME_BUTTON: &str = "<button class=\"theme\" type=\"button\" id=\"theme\" aria-label=\"Theme: system\" title=\"Theme: system\">\
<svg class=\"i-auto\" viewBox=\"3.2 3.2 17.6 17.6\" aria-hidden=\"true\"><circle cx=\"12\" cy=\"12\" r=\"8\"/><path d=\"M12 4a8 8 0 0 0 0 16z\" class=\"fill\"/></svg>\
<svg class=\"i-light\" viewBox=\"1.2 1.2 21.6 21.6\" aria-hidden=\"true\"><circle cx=\"12\" cy=\"12\" r=\"4\"/><path d=\"M12 2v2.5M12 19.5V22M2 12h2.5M19.5 12H22M4.9 4.9l1.8 1.8M17.3 17.3l1.8 1.8M4.9 19.1l1.8-1.8M17.3 6.7l1.8-1.8\"/></svg>\
<svg class=\"i-dark\" viewBox=\"3.85 3.2 16.95 16.95\" aria-hidden=\"true\"><path d=\"M20 14.5A8 8 0 0 1 9.5 4 8 8 0 1 0 20 14.5z\"/></svg></button>";

/// Every inline script on the page, so the CSP lists exactly these by hash.
const SCRIPTS: [&str; 3] = [THEME_INIT_JS, THEME_TOGGLE_JS, LIGHTBOX_JS];

/// The CSP source for [`SCRIPTS`]: the sha256 of each, so exactly those run and any other,
/// inline or loaded, fails loudly in the browser instead of quietly shipping.
fn script_hashes() -> String {
    use base64::Engine as _;
    use sha2::Digest as _;
    SCRIPTS
        .iter()
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
fn host_files(home: &str, repo: &str, name: &str) -> Vec<Out> {
    let headers = format!("\
# Generated by `cargo xtask generate` (xtask/src/site.rs). Do not edit.
/*
  Content-Security-Policy: default-src 'none'; img-src 'self'; style-src 'self'; font-src 'self'; script-src {script}; base-uri 'none'; form-action 'none'; frame-ancestors 'none'
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
",
        script = script_hashes()
    );
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
    // Kinds of destination, not product names: the starter file names apps that may not be out
    // yet, and the page should not lead with one of them.
    line(
        "Send to.",
        "Hand a selection on with its context: a mail draft, an issue or merge request, a task \
         or a note."
            .to_owned(),
    );
    line(
        "Actions.",
        gitterrace_actions::schema::ACTION_SUMMARY.to_owned(),
    );
    line(
        "Many repositories.",
        "Run <code>git terrace</code> in another repository and it joins the server already \
         running: one address, a switcher between them, and every link stays valid."
            .to_owned(),
    );
    let mut s = String::new();
    for (head, text) in rows {
        let _ = writeln!(s, "<li><b>{head}</b> {text}</li>");
    }
    s
}

/// The shots as a two-column gallery of equal tiles, each with its caption.
fn gallery() -> String {
    let mut s = String::from("<div class=\"shots\">\n");
    for (name, alt, cap) in SHOTS {
        let _ = writeln!(
            s,
            "<figure>{}<figcaption><b>{cap}.</b> {}</figcaption></figure>",
            shot_html(name, alt, cap),
            esc(alt.split_once(": ").map_or(alt, |(_, rest)| rest))
        );
    }
    s.push_str("</div>\n");
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

/// The screenshots, in gallery order (pairs side by side): file stem, alt text ("Caption: what
/// it shows"), caption. The site has a WebP pair per entry; the README shows the first, and
/// `docs/screenshots/` keeps only the PNG files the README links.
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
        "Send to: marked lines with their context, ready for mail, an issue, a merge request or your task list.",
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

/// One shot as a tile link plus a full-size view, for the caller's single `<figure>`. The view
/// is a `:target` overlay, so it works without script; clicking it (or Escape, through
/// [`LIGHTBOX_JS`]) goes back to `#features`. Its image is a decorative copy (empty alt) so the
/// description is read once, and `tabindex="-1"` lets the browser focus it when the fragment opens
/// it.
fn shot_html(name: &str, alt: &str, cap: &str) -> String {
    // Every tile is the same size so the rows line up; check_shots holds the WebPs to it.
    // A tile shows only its thumbnail, so it never fetches the full-size file. Light and dark
    // are two lazy images, one hidden by the theme (the toggle or the system): a hidden lazy
    // image is never fetched, so only the shown scheme loads. The dark copy is decorative.
    let pair = |kind: &str, a: &str| {
        format!(
            "<span class=\"shot\"><img class=\"on-light\" src=\"assets/screenshots/{name}-light{kind}.webp\" alt=\"{a}\" width=\"1280\" height=\"800\" loading=\"lazy\" decoding=\"async\">\
             <img class=\"on-dark\" src=\"assets/screenshots/{name}-dark{kind}.webp\" alt=\"\" width=\"1280\" height=\"800\" loading=\"lazy\" decoding=\"async\"></span>"
        )
    };
    let thumb = pair(".thumb", &esc(alt));
    // The lightbox loads only the full-size WebP for the colour scheme; its image is decorative.
    let full = pair("", "");
    format!(
        "<a href=\"#shot-{name}\">{}</a>\
         <a class=\"lightbox\" id=\"shot-{name}\" href=\"#features\" aria-label=\"Close {}\" tabindex=\"-1\">{}</a>",
        thumb,
        esc(cap),
        full
    )
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
mod tests;
