//! The handoff to the browser opener: what its argument may hold, and how a path rides in a
//! query value.

use std::path::Path;

/// What the opener program gets as its argument. A launch link holds a live ticket, and the
/// argument of a running program is readable by every user of the machine (`ps` on macOS,
/// `/proc/PID/cmdline` on Linux), so the link never goes there: it is written to a page in the
/// 0600 state folder, and the opener gets that page. `plain` is for no launch link, or a page
/// that cannot be written: reading works, writing stays refused, and no ticket leaks.
pub(super) fn opener_arg(
    state: Option<&Path>,
    port: u16,
    launch: Option<String>,
    plain: String,
) -> String {
    let Some(link) = launch else { return plain };
    let Some(dir) = state else { return plain };
    handoff(dir, port, &link).unwrap_or(plain)
}

/// Writes the handoff page for `link` and returns its `file:` URL, or why it could not be
/// written. The error never holds the link.
pub(super) fn handoff(dir: &Path, port: u16, link: &str) -> Result<String, String> {
    let name = format!("launch-{port}.html");
    let page = format!(
        "<!doctype html><meta charset=utf-8><meta http-equiv=refresh content=\"0;url={href}\">\
        <a href=\"{href}\">Continue to gitterrace</a>\n",
        href = link.replace('&', "&amp;")
    );
    // Atomic and no-follow: a link planted at the name is refused, never written through.
    gitterrace_git::write_atomic(dir, &name, page.as_bytes(), 0o600)
        .map_err(|e| format!("cannot write {name} in the state folder: {e}"))?;
    Ok(file_url(&dir.join(name).to_string_lossy(), cfg!(windows)))
}

/// Opens the handoff page. When it cannot be written, the ticket-free front page opens and
/// `notice` says owner access was not established and why, so refused writes are explained.
pub(super) fn launch_open(
    state: Option<&Path>,
    addr: std::net::SocketAddr,
    link: &str,
    open: impl FnOnce(&str) -> anyhow::Result<()>,
    notice: impl FnOnce(&str),
) -> anyhow::Result<()> {
    let why = match state {
        Some(dir) => match handoff(dir, addr.port(), link) {
            Ok(page) => return open(&page),
            Err(why) => why,
        },
        None => "there is no state folder".to_owned(),
    };
    notice(&format!(
        "owner access was not established: {why}. Opening the read-only front page; \
         owner actions stay refused until `git terrace launch` succeeds."
    ));
    open(&format!("http://{addr}/"))
}

/// `path` as a `file:` URL the opener can load. A drive path `C:\a b` is `file:///C:/a%20b`, a
/// UNC path `\\host\share\x` is `file://host/share/x`, a Unix path `/a` is `file:///a`. Each
/// segment is percent-encoded, so `#`, `?` and `%` in a name stay in the path. `windows` is a
/// parameter so the rules run under test on every OS.
pub(super) fn file_url(path: &str, windows: bool) -> String {
    if !windows {
        return format!("file://{}", path_query(path));
    }
    let flat = path.replace('\\', "/");
    let flat = flat.strip_prefix("//?/").unwrap_or(&flat);
    if let Some(unc) = flat.strip_prefix("UNC/") {
        return format!("file://{}", path_query(unc));
    }
    if let Some(unc) = flat.strip_prefix("//") {
        return format!("file://{}", path_query(unc));
    }
    // The drive colon stays literal: `C%3A` is not a drive to a browser.
    match flat.split_once(':') {
        Some((drive, rest)) if drive.len() == 1 => {
            format!("file:///{drive}:{}", path_query(rest))
        }
        _ => format!("file:///{}", path_query(flat.trim_start_matches('/'))),
    }
}

/// The inverse of [`file_url`]: the path a `file:` URL names, for tests and callers that must
/// read what the opener is given.
#[cfg(test)]
pub(super) fn file_path(url: &str, windows: bool) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let mut raw = Vec::new();
    let mut bytes = rest.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hex = [bytes.next()?, bytes.next()?];
            raw.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            raw.push(b);
        }
    }
    let text = String::from_utf8(raw).ok()?;
    if !windows {
        return Some(text);
    }
    let back = text.replace('/', "\\");
    Some(back.strip_prefix('\\').map_or_else(|| format!("\\\\{back}"), str::to_owned))
}

/// `path` as a query value: every byte outside the unreserved set is escaped, so a `?` or `&`
/// in the link stays inside `to`.
pub(super) fn path_query(path: &str) -> String {
    let plain = |b: &u8| b.is_ascii_alphanumeric() || b"-._~/".contains(b);
    path.bytes()
        .map(|b| if plain(&b) { char::from(b).to_string() } else { format!("%{b:02X}") })
        .collect()
}
