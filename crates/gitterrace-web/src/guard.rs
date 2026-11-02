//! The one middleware every request passes: Host allowlist (DNS rebinding), cross-site
//! refusal (any page the user visits can fire `<img src=http://127.0.0.1:...>`), and the
//! security headers on every response. A public server also refuses every method but GET and
//! HEAD before routing and bounds each request's time.

use std::{net::IpAddr, sync::Arc, time::Duration};

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// Hashes of the inline theme snippets in `_base.html` and `repos.html`; the
/// `inline_scripts_are_hashed` test fails when a template changes without this.
const CSP: &str = "default-src 'self'; img-src 'self' data:; media-src 'self'; style-src 'self' 'unsafe-inline'; \
    script-src 'self' 'sha256-fYt56R17Pz3RaWQCK4ngD4kWxzR9lHjYqQRanR4kAxk=' \
    'sha256-Rbk1A+FIExFQ+koGhLey9hvtVuj90foK6DS83At6mms='; object-src 'self'; \
    frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

/// Longest a public request may take before it answers 503. Blame or insights on a huge
/// history can run longer; their blocking job finishes and fills the cache for a retry.
const PUBLIC_TIMEOUT: Duration = Duration::from_secs(30);

/// What the guard checks, fixed at startup.
pub(crate) struct Gate {
    /// The bound address when it is not loopback.
    bound: Option<IpAddr>,
    /// `server.public`.
    public: bool,
    /// `server.hosts`, lowercase.
    hosts: Vec<String>,
}

impl Gate {
    pub(crate) fn new(bound: Option<IpAddr>, cfg: &gitterrace_config::Config) -> Self {
        let s = &cfg.server;
        Self {
            bound,
            public: s.public,
            hosts: s.hosts.iter().map(|h| h.to_ascii_lowercase()).collect(),
        }
    }
}

/// The name part of a Host header, lowercase: `[v6]` unbracketed, a numeric port dropped.
fn host_name(host: &str) -> Option<String> {
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split_once(']')?.0,
        None => match host.rsplit_once(':') {
            Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
            Some(_) => return None,
            None => host,
        },
    };
    Some(name.to_ascii_lowercase())
}

/// Host-header allowlist: loopback names, `server.hosts` exactly, and, when bound elsewhere, the
/// bound IP. A bind to an unspecified address (`0.0.0.0`, `::`) accepts any IP literal:
/// rebinding needs a name.
fn host_allowed(host: &str, gate: &Gate) -> bool {
    let Some(name) = host_name(host) else {
        return false;
    };
    if matches!(name.as_str(), "localhost" | "127.0.0.1" | "::1") || gate.hosts.contains(&name) {
        return true;
    }
    let Some(ip) = gate.bound else { return false };
    name.parse::<IpAddr>()
        .is_ok_and(|n| ip.is_unspecified() || n == ip)
}

/// Why a request is refused, if it is. Absent `Sec-Fetch-Site` (curl, MCP, old clients)
/// passes: browsers always send it, and they are the threat.
fn refusal(
    method: &Method,
    headers: &HeaderMap,
    gate: &Gate,
) -> Option<(StatusCode, &'static str)> {
    let host_ok = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| host_allowed(h, gate));
    if !host_ok {
        return Some((
            StatusCode::FORBIDDEN,
            "forbidden: unrecognised Host header\n",
        ));
    }
    if gate.public {
        // Nothing public writes, so no other method is routed and no body is read. A link to
        // the site from anywhere is the point, so cross-site navigation passes.
        return (method != Method::GET && method != Method::HEAD)
            .then_some((StatusCode::METHOD_NOT_ALLOWED, "read-only\n"));
    }
    // Another port on localhost is same-site, so only same-origin and a direct visit pass.
    let foreign = headers.get("sec-fetch-site").is_some_and(|v| {
        !(v.as_bytes().eq_ignore_ascii_case(b"same-origin")
            || v.as_bytes().eq_ignore_ascii_case(b"none"))
    });
    foreign.then_some((StatusCode::FORBIDDEN, "forbidden: cross-site request\n"))
}

pub(crate) async fn guard(State(gate): State<Arc<Gate>>, req: Request, next: Next) -> Response {
    let mut res = match refusal(req.method(), req.headers(), &gate) {
        Some((code, why)) => (code, why).into_response(),
        None if gate.public => tokio::time::timeout(PUBLIC_TIMEOUT, next.run(req))
            .await
            .unwrap_or_else(|_| {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "took too long, try again\n",
                )
                    .into_response()
            }),
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
mod tests {
    use super::{CSP, Gate, host_allowed};

    fn gate(bound: Option<&str>, hosts: &[&str]) -> Gate {
        Gate {
            bound: bound.map(|b| b.parse().expect("ip")),
            public: !hosts.is_empty(),
            hosts: hosts.iter().map(|h| (*h).to_owned()).collect(),
        }
    }

    /// The reverse-proxy case: the configured name passes with or without a port and in any
    /// case; a sibling, a suffix trick or another name does not.
    #[test]
    fn proxy_hosts_are_exact() {
        let g = gate(None, &["git.example.com"]);
        for h in [
            "git.example.com",
            "GIT.example.com",
            "git.example.com:443",
            "localhost",
        ] {
            assert!(host_allowed(h, &g), "{h}");
        }
        for h in [
            "evil.example.com",
            "git.example.com.evil.example",
            "xgit.example.com",
            "example.com",
            "git.example.com:",
            "git.example.com:443x",
            "",
        ] {
            assert!(!host_allowed(h, &g), "{h}");
        }
        assert!(!host_allowed("git.example.com", &gate(None, &[])));
    }

    #[test]
    fn host_allowlist() {
        use gitterrace_testkit::{DEFAULT_ADDR, on_port};
        let none = gate(None, &[]);
        let ip = gate(Some("192.168.1.5"), &[]);
        let any = gate(Some("0.0.0.0"), &[]);
        let port = gitterrace_testkit::port();
        for h in [
            "localhost".to_owned(),
            on_port("localhost"),
            DEFAULT_ADDR.to_owned(),
            format!("[::1]:{port}"),
            "LOCALHOST".to_owned(),
        ] {
            assert!(host_allowed(&h, &none), "{h}");
        }
        for h in [
            on_port("evil.example"),
            "192.168.1.5:80".to_owned(),
            "[::1".to_owned(),
            String::new(),
            format!("{DEFAULT_ADDR}@evil"),
            "localhost:".to_owned(),
            "localhost:80a".to_owned(),
        ] {
            assert!(!host_allowed(&h, &none), "{h}");
        }
        assert!(host_allowed(&on_port("192.168.1.5"), &ip));
        assert!(!host_allowed("10.0.0.1", &ip));
        assert!(host_allowed(&on_port("10.0.0.7"), &any));
        assert!(!host_allowed(&on_port("evil.example"), &any));
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
