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
    let name = format!("launch-{port}.html");
    let page = format!(
        "<!doctype html><meta charset=utf-8><meta http-equiv=refresh content=\"0;url={href}\">\
        <a href=\"{href}\">Continue to gitterrace</a>\n",
        href = link.replace('&', "&amp;")
    );
    // Atomic and no-follow: a link planted at the name is refused, never written through.
    if gitterrace_git::write_atomic(dir, &name, page.as_bytes(), 0o600).is_err() {
        return plain;
    }
    format!("file://{}", path_query(&dir.join(name).to_string_lossy()))
}

/// `path` as a query value: every byte outside the unreserved set is escaped, so a `?` or `&`
/// in the link stays inside `to`.
pub(super) fn path_query(path: &str) -> String {
    let plain = |b: &u8| b.is_ascii_alphanumeric() || b"-._~/".contains(b);
    path.bytes()
        .map(|b| if plain(&b) { char::from(b).to_string() } else { format!("%{b:02X}") })
        .collect()
}
