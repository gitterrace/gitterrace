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

    check_shots(root)?;

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
const SITE_SHOT_DIR: &str = "site/assets/screenshots";
/// Pixel size of the lightbox WebP and of the tile thumbnail.
const FULL: (u32, u32) = (2560, 1600);
const THUMB: (u32, u32) = (1280, 800);

/// Checks the screenshots without copying them. `docs/screenshots/` holds exactly the PNG files
/// README.md links, each 2560x1600; `site/assets/screenshots/` holds exactly a full-size WebP at
/// 2560x1600 and a 1280x800 thumbnail for every entry in `SHOTS`. All of them come from
/// `make site-shots`; a missing, extra or wrongly sized file fails generation and names it.
fn check_shots(root: &Path) -> Result<()> {
    let readme = std::fs::read_to_string(root.join("README.md")).context("reading README.md")?;
    let pngs = readme_shots(&readme);
    let have = list(root, SHOT_DIR, "make site-shots")?;
    if let Some(f) = pngs.iter().find(|f| !have.contains(f)) {
        anyhow::bail!("{SHOT_DIR}/{f} is linked by README.md but does not exist (make site-shots)");
    }
    if let Some(f) = have.iter().find(|f| !pngs.contains(f)) {
        anyhow::bail!(
            "{SHOT_DIR}/{f} is not referenced by README.md: delete it (make site-shots keeps only the README's PNG files)"
        );
    }
    for f in &pngs {
        let got = shot_size(root, f)?;
        anyhow::ensure!(
            got == (2560, 1600),
            "{SHOT_DIR}/{f} is {}x{}, the README needs 2560x1600 (make site-shots)",
            got.0,
            got.1
        );
    }
    let mut want: Vec<(String, (u32, u32))> = Vec::new();
    for (n, ..) in SHOTS {
        for c in ["light", "dark"] {
            want.push((format!("{n}-{c}.webp"), FULL));
            want.push((format!("{n}-{c}.thumb.webp"), THUMB));
        }
    }
    let have = list(root, SITE_SHOT_DIR, "make site-shots")?;
    if let Some((f, _)) = want.iter().find(|(f, _)| !have.contains(f)) {
        anyhow::bail!("{SITE_SHOT_DIR}/{f} is missing (make site-shots)");
    }
    if let Some(f) = have.iter().find(|f| !want.iter().any(|(w, _)| w == *f)) {
        anyhow::bail!("{SITE_SHOT_DIR}/{f} is not referenced: delete it and run `make site-shots`");
    }
    for (f, size) in &want {
        let path = format!("{SITE_SHOT_DIR}/{f}");
        let got =
            webp_size(&std::fs::read(root.join(&path)).with_context(|| format!("reading {path}"))?)
                .with_context(|| format!("{path} is not a WebP (make site-shots)"))?;
        anyhow::ensure!(
            got == *size,
            "{path} is {}x{}, the site needs {}x{} (make site-shots)",
            got.0,
            got.1,
            size.0,
            size.1
        );
    }
    Ok(())
}

/// The `docs/screenshots/*.png` file names README.md links, sorted and deduplicated. The same
/// rule as `scripts/site-shots-run.mjs`, which copies exactly these.
fn readme_shots(readme: &str) -> Vec<String> {
    let key = "docs/screenshots/";
    let mut v: Vec<String> = readme
        .match_indices(key)
        .filter_map(|(i, _)| {
            let rest = &readme[i + key.len()..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))?;
            let f = rest[..end].trim_end_matches('.');
            Path::new(f)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("png"))
                .then(|| f.to_string())
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The file names in `dir`, sorted. A missing folder names the target that fills it.
fn list(root: &Path, dir: &str, fix: &str) -> Result<Vec<String>> {
    let mut v: Vec<String> = std::fs::read_dir(root.join(dir))
        .with_context(|| format!("reading {dir} (run `{fix}`)"))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| !f.starts_with('.'))
        .collect();
    v.sort();
    Ok(v)
}

/// Width and height in pixels of a WebP, from its first chunk: lossy (VP8), lossless (VP8L)
/// or extended (VP8X). None when the header is not one of those.
fn webp_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 30 || &b[0..4] != b"RIFF" || &b[8..12] != b"WEBP" {
        return None;
    }
    let le16 = |i: usize| u32::from(b[i]) | u32::from(b[i + 1]) << 8;
    let le24 = |i: usize| le16(i) | u32::from(b[i + 2]) << 16;
    match &b[12..16] {
        b"VP8 " if b[23..26] == [0x9d, 0x01, 0x2a] => Some((le16(26) & 0x3fff, le16(28) & 0x3fff)),
        b"VP8L" if b[20] == 0x2f => {
            let bits = u32::from_le_bytes([b[21], b[22], b[23], b[24]]);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        b"VP8X" => Some((le24(24) + 1, le24(27) + 1)),
        _ => None,
    }
}

/// Width and height in pixels of a PNG, from its header.
fn shot_size(root: &Path, file: &str) -> Result<(u32, u32)> {
    let path = format!("{SHOT_DIR}/{file}");
    let b = std::fs::read(root.join(&path)).with_context(|| format!("reading {path}"))?;
    anyhow::ensure!(b.len() > 24 && &b[1..4] == b"PNG", "{path} is not a PNG");
    let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
    let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
    Ok((w, h))
}

/// One shot as a tile link plus a full-size view, for the caller's single `<figure>`. The view
/// is a `:target` overlay, since the site CSP runs no script; clicking it goes back to
/// `#features`. Its image is a decorative copy (empty alt) so the description is read once,
/// and `tabindex="-1"` lets the browser focus it when the fragment opens it.
fn shot_html(name: &str, alt: &str, cap: &str) -> String {
    // Every tile is the same size so the rows line up; check_shots holds the WebPs to it.
    // A tile shows only its thumbnail, so it never fetches the full-size file. Only the colour
    // scheme's source loads.
    let dark = " media=\"(prefers-color-scheme: dark)\"";
    let thumb = format!(
        "<picture><source{dark} srcset=\"assets/screenshots/{name}-dark.thumb.webp\"><img src=\"assets/screenshots/{name}-light.thumb.webp\" alt=\"{}\" width=\"1280\" height=\"800\" loading=\"lazy\" decoding=\"async\"></picture>",
        esc(alt)
    );
    // The lightbox loads only the full-size WebP for the colour scheme.
    let full = format!(
        "<picture><source{dark} srcset=\"assets/screenshots/{name}-dark.webp\"><img src=\"assets/screenshots/{name}-light.webp\" alt=\"\" width=\"1280\" height=\"800\" loading=\"lazy\" decoding=\"async\"></picture>"
    );
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
mod tests {
    use super::*;

    /// One figure per shot, a picture inside it, and a lightbox that is labelled, takes focus
    /// and does not announce its image twice.
    #[test]
    fn gallery_has_one_figure_per_shot_and_an_accessible_lightbox() {
        let g = gallery();
        assert!(!g.contains("<figure><figure"), "nested figure");
        assert_eq!(g.matches("<figure>").count(), SHOTS.len());
        assert_eq!(g.matches("<picture>").count(), SHOTS.len() * 2);
        assert_eq!(g.matches("tabindex=\"-1\"").count(), SHOTS.len());
        assert_eq!(g.matches("aria-label=\"Close ").count(), SHOTS.len());
    }

    /// A tile loads only its thumbnail, and the full-size file is left to the lightbox.
    #[test]
    fn tiles_never_reference_the_full_size_screenshot() {
        for (name, alt, cap) in SHOTS {
            let h = shot_html(name, alt, cap);
            let (tile, lightbox) = h.split_once("<a class=\"lightbox\"").expect("lightbox");
            for c in ["light", "dark"] {
                assert!(!tile.contains(&format!("screenshots/{name}-{c}.webp")));
                assert!(!h.contains(".png"), "shot {name} links a PNG");
                assert!(tile.contains(&format!("screenshots/{name}-{c}.thumb.webp")));
                assert!(
                    !lightbox.contains(".thumb."),
                    "lightbox {name} loads a thumbnail"
                );
            }
            assert!(lightbox.contains(&format!("screenshots/{name}-dark.webp")));
            assert!(tile.contains("width=\"1280\" height=\"800\" loading=\"lazy\""));
        }
    }

    #[test]
    fn redirect_target_is_the_installer_url() {
        let r = redirects("https://github.com/o/r", "r");
        assert!(r.contains(
            "/install.sh https://github.com/o/r/releases/latest/download/r-installer.sh 302"
        ));
    }

    /// A VP8X header of the given canvas size, enough for `webp_size`.
    fn webp(w: u32, h: u32) -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0".to_vec();
        b.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
        b.extend_from_slice(&(h - 1).to_le_bytes()[..3]);
        b
    }

    /// A PNG header of the given size, enough for `shot_size`.
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.push(0);
        b
    }

    #[test]
    fn readme_shots_are_the_linked_pngs() {
        let r = "<img src=\"docs/screenshots/b-light.png\" alt=\"x\">\n\
                 srcset=\"docs/screenshots/a-dark.png\" docs/screenshots/b-light.png docs/screenshots/x.svg";
        assert_eq!(readme_shots(r), ["a-dark.png", "b-light.png"]);
    }

    #[test]
    fn a_missing_unreferenced_or_wrongly_sized_screenshot_fails() {
        let dir = std::env::temp_dir().join(format!("gt-shots-{}", std::process::id()));
        let shots = dir.join(SHOT_DIR);
        let site = dir.join(SITE_SHOT_DIR);
        std::fs::create_dir_all(&shots).expect("mkdir");
        std::fs::create_dir_all(&site).expect("mkdir");
        std::fs::write(
            dir.join("README.md"),
            "<source srcset=\"docs/screenshots/overview-dark.png\">\n<img src=\"docs/screenshots/overview-light.png\">\n",
        )
        .expect("readme");
        let err = check_shots(&dir)
            .expect_err("empty folder fails")
            .to_string();
        assert!(
            err.contains("overview-dark.png is linked by README.md"),
            "{err}"
        );
        assert!(err.contains("make site-shots"), "{err}");
        for c in ["light", "dark"] {
            std::fs::write(shots.join(format!("overview-{c}.png")), png(2560, 1600))
                .expect("write");
        }
        // A PNG the README does not link fails, though the site has a shot of that name.
        std::fs::write(shots.join("blame-dark.png"), png(2560, 1600)).expect("write");
        let err = check_shots(&dir)
            .expect_err("unlinked png fails")
            .to_string();
        assert!(
            err.contains("blame-dark.png is not referenced by README.md"),
            "{err}"
        );
        assert!(err.contains("make site-shots"), "{err}");
        std::fs::remove_file(shots.join("blame-dark.png")).expect("rm");
        std::fs::write(shots.join("overview-dark.png"), png(1280, 800)).expect("write");
        let err = check_shots(&dir).expect_err("small png fails").to_string();
        assert!(err.contains("overview-dark.png is 1280x800"), "{err}");
        std::fs::write(shots.join("overview-dark.png"), png(2560, 1600)).expect("write");
        let err = check_shots(&dir).expect_err("no webp fails").to_string();
        assert!(err.contains("is missing (make site-shots)"), "{err}");
        for (n, ..) in SHOTS {
            for c in ["light", "dark"] {
                std::fs::write(site.join(format!("{n}-{c}.webp")), webp(2560, 1600))
                    .expect("write");
                std::fs::write(site.join(format!("{n}-{c}.thumb.webp")), webp(1280, 800))
                    .expect("write");
            }
        }
        check_shots(&dir).expect("complete set passes");
        std::fs::write(site.join("blame-dark.thumb.webp"), webp(1200, 750)).expect("write");
        let err = check_shots(&dir).expect_err("wrong size fails").to_string();
        assert!(err.contains("blame-dark.thumb.webp is 1200x750"), "{err}");
        assert!(err.contains("make site-shots"), "{err}");
        std::fs::write(site.join("blame-dark.thumb.webp"), webp(1280, 800)).expect("write");
        std::fs::write(site.join("blame-dark.png"), b"x").expect("write");
        let err = check_shots(&dir).expect_err("site png fails").to_string();
        assert!(err.contains("not referenced"), "{err}");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
