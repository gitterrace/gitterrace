//! The credit line at the foot of every exported page: whose project this is, under which
//! license, and where it comes from. All of it is read from the exported commit and the
//! repository config, and every value is escaped before it reaches the page.

use gitterrace_git::{Repo, TreeEntryKind};

/// Largest license file read; a real license text is far smaller.
const MAX_LICENSE: usize = 256 * 1024;
/// How much of the text names the license; the GPL texts mention others further down.
const TITLE_CHARS: usize = 400;

/// What is known about the exported project.
pub(super) struct Credit {
    name: String,
    license: Option<License>,
    origin: Option<String>,
}

/// The license file and, when its text is recognised, the short name.
struct License {
    file: String,
    id: Option<&'static str>,
}

impl License {
    /// The short name, or a link to the file when the text is not a common license.
    fn html(&self, prefix: &str) -> String {
        let esc = gitterrace_web::escape_html;
        if let Some(id) = self.id {
            return format!("{} license", esc(id));
        }
        let href = format!(
            "{prefix}/blob/HEAD/{}",
            gitterrace_git::percent_encode_path(&self.file)
        );
        format!("<a href=\"{}\">license</a>", esc(&href))
    }
}

impl Credit {
    /// Reads the license file at `head` and the `origin` remote of `repo`. `origin_override`
    /// (a project URL given on the command line) wins over the remote.
    pub(super) fn read(repo: &Repo, head: &str, name: &str, origin_override: Option<&str>) -> Self {
        let from_config = std::fs::read_to_string(repo.git_dir().join("config"))
            .ok()
            .and_then(|c| origin_url(&c));
        let origin = origin_override.and_then(public_https).or(from_config);
        Self {
            name: name.to_owned(),
            license: find_license(repo, head),
            origin,
        }
    }

    /// The footer element to place at the end of a page of the site under `prefix`.
    pub(super) fn footer(&self, prefix: &str) -> String {
        let esc = gitterrace_web::escape_html;
        let mut parts = vec![format!("Source: {}", esc(&self.name))];
        if let Some(l) = &self.license {
            parts.push(l.html(prefix));
        }
        if let Some(url) = &self.origin {
            let shown = url.strip_prefix("https://").unwrap_or(url);
            parts.push(format!("<a href=\"{}\">{}</a>", esc(url), esc(shown)));
        }
        format!(
            "<footer class=\"snapshot credit\" role=\"contentinfo\"><div class=\"wrap\">{}</div></footer>",
            parts.join(", ")
        )
    }
}

/// Puts `footer` before the last `</body>`; a page without one is returned unchanged.
pub(super) fn add_footer(html: &[u8], footer: &str) -> Vec<u8> {
    let page = String::from_utf8_lossy(html);
    let Some(at) = page.rfind("</body>") else {
        return html.to_vec();
    };
    format!("{}{footer}{}", &page[..at], &page[at..]).into_bytes()
}

fn find_license(repo: &Repo, head: &str) -> Option<License> {
    let mut names: Vec<String> = repo
        .tree(head, "")
        .ok()?
        .into_iter()
        .filter(|e| e.kind == TreeEntryKind::File && is_license_name(&e.name))
        .map(|e| e.name)
        .collect();
    names.sort();
    let file = names.into_iter().next()?;
    let id = repo
        .blob_bytes(head, &file)
        .ok()
        .filter(|b| b.len() <= MAX_LICENSE)
        .and_then(|b| detect(&String::from_utf8_lossy(&b)));
    Some(License { file, id })
}

fn is_license_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["license", "licence", "copying"]
        .iter()
        .any(|stem| lower == *stem || lower.starts_with(&format!("{stem}.")))
}

/// Names a license by marker phrases in its text; `None` when it is none of the common ones.
fn detect(text: &str) -> Option<&'static str> {
    let flat = text
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let head: String = flat.chars().take(TITLE_CHARS).collect();
    if head.contains("lesser general public license") {
        return Some(if head.contains("version 3") {
            "LGPL-3.0"
        } else if head.contains("version 2.1") {
            "LGPL-2.1"
        } else {
            "LGPL"
        });
    }
    if head.contains("general public license") {
        if head.contains("version 3") {
            return Some("GPL-3.0");
        }
        if head.contains("version 2") {
            return Some("GPL-2.0");
        }
    }
    if head.contains("apache license") && head.contains("version 2.0") {
        return Some("Apache-2.0");
    }
    if head.contains("mozilla public license") && head.contains("2.0") {
        return Some("MPL-2.0");
    }
    if flat.contains("free and unencumbered software released into the public domain") {
        return Some("Unlicense");
    }
    if flat.contains("permission is hereby granted, free of charge") {
        return Some("MIT");
    }
    if flat.contains(
        "permission to use, copy, modify, and/or distribute this software for any purpose",
    ) {
        return Some("ISC");
    }
    if flat.contains("redistribution and use in source and binary forms") {
        let three = flat.contains("neither the name") || flat.contains("endorse or promote");
        return Some(if three {
            "BSD-3-Clause"
        } else {
            "BSD-2-Clause"
        });
    }
    None
}

/// The `url` of `[remote "origin"]` in a git config, made public and https, or `None`.
fn origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            let inner = line.trim_start_matches('[').split(']').next().unwrap_or("");
            let mut words = inner.splitn(2, char::is_whitespace);
            let section = words.next().unwrap_or("");
            let sub = words.next().unwrap_or("").trim();
            in_origin = section.eq_ignore_ascii_case("remote") && sub == "\"origin\"";
        } else if in_origin
            && let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("url")
        {
            return public_https(value.trim().trim_matches('"'));
        }
    }
    None
}

/// An `https://`, `ssh://` or `user@host:path` remote as a plain `https://host/path` link,
/// without credentials, port or `.git`. Local paths, other schemes and hosts that are not
/// public names (no dot, an IP address, a local suffix) give `None`.
fn public_https(url: &str) -> Option<String> {
    let (host, path) = if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("ssh://"))
    {
        let (auth, path) = rest.split_once('/')?;
        let host = auth.rsplit_once('@').map_or(auth, |(_, h)| h);
        (host.split(':').next()?, path)
    } else if url.contains("://") || url.starts_with(['/', '.', '~']) {
        return None;
    } else {
        // scp-like: user@host:owner/repo
        let (auth, path) = url.split_once(':')?;
        (auth.rsplit_once('@').map_or(auth, |(_, h)| h), path)
    };
    let host = host.to_ascii_lowercase();
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let host_ok = host.contains('.')
        && !host.starts_with(['.', '-'])
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && !host.chars().all(|c| c.is_ascii_digit() || c == '.')
        && !["local", "localhost", "internal", "lan", "home", "corp"]
            .iter()
            .any(|s| host.rsplit('.').next() == Some(s));
    let path_ok = !path.is_empty()
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._~-/".contains(c));
    (host_ok && path_ok).then(|| format!("https://{host}/{path}"))
}

#[cfg(test)]
mod tests;
