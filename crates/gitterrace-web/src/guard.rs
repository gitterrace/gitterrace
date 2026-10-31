//! The one middleware every request passes: Host allowlist (DNS rebinding), cross-site
//! refusal (any page the user visits can fire `<img src=http://127.0.0.1:...>`), and the
//! security headers on every response.

use std::net::IpAddr;

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Hashes of the inline theme snippets in `base.html` and `repos.html`; the
/// `inline_scripts_are_hashed` test fails when a template changes without this.
const CSP: &str = "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; \
    script-src 'self' 'sha256-f8yF5KiP4g1r0wpmUjDwQ50o+2qbnsXV/gZXuaQHLGo=' \
    'sha256-7Mrre5gu7kp7yI9IZqpAANWdNSEkrx/tOnHcVhDX2LE='; object-src 'none'; \
    frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

/// Host-header allowlist: loopback names plus, when bound elsewhere, the bound IP. A bind to
/// an unspecified address (`0.0.0.0`, `::`) accepts any IP literal: rebinding needs a name.
pub(crate) fn host_allowed(host: &str, bound: Option<IpAddr>) -> bool {
    let name = match host.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((ip, _)) => ip,
            None => return false,
        },
        None => host.rsplit_once(':').map_or(host, |(h, _)| h),
    };
    let name = name.to_ascii_lowercase();
    if matches!(name.as_str(), "localhost" | "127.0.0.1" | "::1") {
        return true;
    }
    let Some(ip) = bound else { return false };
    name.parse::<IpAddr>()
        .is_ok_and(|n| ip.is_unspecified() || n == ip)
}

/// Why a request is refused, if it is. Absent `Sec-Fetch-Site` (curl, MCP, old clients)
/// passes: browsers always send it, and they are the threat.
fn refusal(headers: &HeaderMap, bound: Option<IpAddr>) -> Option<&'static str> {
    let host_ok = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| host_allowed(h, bound));
    if !host_ok {
        return Some("forbidden: unrecognised Host header\n");
    }
    let cross = headers
        .get("sec-fetch-site")
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"cross-site"));
    cross.then_some("forbidden: cross-site request\n")
}

pub(crate) async fn guard(
    State(bound): State<Option<IpAddr>>,
    req: Request,
    next: Next,
) -> Response {
    let mut res = match refusal(req.headers(), bound) {
        Some(why) => (StatusCode::FORBIDDEN, why).into_response(),
        None => next.run(req).await,
    };
    let h = res.headers_mut();
    h.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    let html = h
        .get(header::CONTENT_TYPE)
        .is_some_and(|v| v.as_bytes().starts_with(b"text/html"));
    if html && !h.contains_key(header::CONTENT_SECURITY_POLICY) {
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        );
    }
    res
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::{CSP, host_allowed};

    #[test]
    fn host_allowlist() {
        let ip: std::net::IpAddr = "192.168.1.5".parse().expect("ip");
        let any: std::net::IpAddr = "0.0.0.0".parse().expect("ip");
        for h in [
            "localhost",
            "localhost:4242",
            "127.0.0.1:4242",
            "[::1]:4242",
            "LOCALHOST",
        ] {
            assert!(host_allowed(h, None), "{h}");
        }
        for h in ["evil.example:4242", "192.168.1.5:80", "[::1", ""] {
            assert!(!host_allowed(h, None), "{h}");
        }
        assert!(host_allowed("192.168.1.5:4242", Some(ip)));
        assert!(!host_allowed("10.0.0.1", Some(ip)));
        assert!(host_allowed("10.0.0.7:4242", Some(any)));
        assert!(!host_allowed("evil.example:4242", Some(any)));
    }

    /// Every inline `<script>` in a template must have its hash in the CSP, or the browser
    /// drops it silently.
    #[test]
    fn inline_scripts_are_hashed() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/templates");
        for e in std::fs::read_dir(dir).expect("templates") {
            let text = std::fs::read_to_string(e.expect("entry").path()).expect("read");
            for chunk in text.split("<script>").skip(1) {
                let body = chunk.split("</script>").next().expect("closed");
                let hash = sha256_b64(body);
                assert!(
                    CSP.contains(&format!("'sha256-{hash}'")),
                    "unhashed: {body}"
                );
            }
        }
    }

    fn sha256_b64(s: &str) -> String {
        use std::io::Write;
        let mut child = std::process::Command::new("sh")
            .args(["-c", "openssl dgst -sha256 -binary | base64"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("openssl");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(s.as_bytes())
            .expect("write");
        let out = child.wait_with_output().expect("wait");
        String::from_utf8(out.stdout)
            .expect("utf8")
            .trim()
            .to_owned()
    }
}
