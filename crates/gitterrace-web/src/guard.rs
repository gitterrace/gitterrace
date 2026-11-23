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

/// The page policy, also written into an export's `_headers`.
///
/// Scripts are locked to this origin plus the hashes of the inline theme snippets in
/// `_base.html` and `repos.html`; the `inline_scripts_are_hashed` test fails when a template
/// changes without this. Styles allow `unsafe-inline` for one reason: mermaid writes its
/// diagram styles at runtime and cannot be given a hash. The app's own markup carries no
/// style attribute (`markup_owners` and `shared_markup_pages` tests), and a style cannot run code.
pub const CSP: &str = "default-src 'self'; img-src 'self' data:; media-src 'self'; \
    style-src 'self' 'unsafe-inline'; \
    script-src 'self' 'sha256-fYt56R17Pz3RaWQCK4ngD4kWxzR9lHjYqQRanR4kAxk=' \
    'sha256-Rbk1A+FIExFQ+koGhLey9hvtVuj90foK6DS83At6mms='; object-src 'none'; \
    frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

/// Whether the request declares a JSON body.
pub(crate) fn is_json(h: &HeaderMap) -> bool {
    h.get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))
}

/// The request's JSON body, bounded by `limit`: 413 past it, `malformed()` when it does not parse.
pub(crate) async fn json_body<T: serde::de::DeserializeOwned>(
    req: Request,
    limit: usize,
    malformed: impl FnOnce() -> Response,
) -> Result<T, Box<Response>> {
    let Ok(bytes) = axum::body::to_bytes(req.into_body(), limit).await else {
        return Err(Box::new((StatusCode::PAYLOAD_TOO_LARGE, "body too large\n").into_response()));
    };
    serde_json::from_slice(&bytes).map_err(|_| Box::new(malformed()))
}

/// `res` marked uncacheable: pages and replies that change under the reader.
pub(crate) fn no_store(mut res: Response) -> Response {
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

/// What the guard checks, fixed at startup.
pub(crate) struct Gate {
    bound: Option<IpAddr>,
    /// `server.is_public`.
    public: bool,
    /// `server.hosts`, lowercase.
    hosts: Vec<String>,
    /// `server.public_timeout_secs`: longest a public request may take before it answers 503.
    /// Blame or insights on a huge history can run longer; their blocking job finishes and
    /// fills the cache for a retry.
    timeout: Duration,
}

impl Gate {
    pub(crate) fn new(bound: Option<IpAddr>, cfg: &gitterrace_config::Config) -> Self {
        let s = &cfg.server;
        Self {
            bound,
            public: s.is_public,
            hosts: s.hosts.iter().map(|h| h.to_ascii_lowercase()).collect(),
            timeout: Duration::from_secs(s.public_timeout_secs),
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
    name.parse::<IpAddr>().is_ok_and(|n| ip.is_unspecified() || n == ip)
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
        return Some((StatusCode::FORBIDDEN, "forbidden: unrecognised Host header\n"));
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

/// Why a state-changing request from a page is refused before its body is read, shared by every
/// route that writes so they cannot drift apart. Fetch metadata must say same-origin (absent and
/// `none` are refused, unlike GETs), `Origin` must be this host, and the request must carry the
/// owner cookie. The first two stop other sites. They are headers, so another user of the machine
/// sets them freely: the cookie is the proof, and no page ever holds what makes it.
pub(crate) fn same_origin_write(
    h: &HeaderMap,
    owner: &crate::owner::Owner,
) -> Option<&'static str> {
    let get = |n: &str| h.get(n).and_then(|v| v.to_str().ok());
    if !get("sec-fetch-site").is_some_and(|v| v.eq_ignore_ascii_case("same-origin")) {
        return Some("forbidden: not a same-origin request\n");
    }
    let host = get("host").unwrap_or_default();
    let origin_ok = get("origin").is_some_and(|o| {
        o.strip_prefix("http://")
            .is_some_and(|rest| !host.is_empty() && rest.eq_ignore_ascii_case(host))
    });
    if !origin_ok {
        return Some("forbidden: foreign origin\n");
    }
    if !owner.proven(h) {
        return Some("forbidden: not the owner of this server, open it with `git terrace open`\n");
    }
    None
}

pub(crate) async fn guard(State(gate): State<Arc<Gate>>, req: Request, next: Next) -> Response {
    let mut res = match refusal(req.method(), req.headers(), &gate) {
        Some((code, why)) => (code, why).into_response(),
        None if gate.public => {
            tokio::time::timeout(gate.timeout, next.run(req)).await.unwrap_or_else(|_| {
                (StatusCode::SERVICE_UNAVAILABLE, "took too long, try again\n").into_response()
            })
        }
        None => next.run(req).await,
    };
    let h = res.headers_mut();
    h.insert("cross-origin-resource-policy", HeaderValue::from_static("same-origin"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    // The value an export's `_headers` sets, so both deployments leak the same referrer.
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("strict-origin-when-cross-origin"));
    // A route that knows its lifetime (hashed assets, raw with an ETag, no-store forms) says so.
    // Everything else is named by something that moves (HEAD, a branch, the worktree), and
    // even a page pinned to a commit shows refs and the live tree in its chrome, so the
    // browser revalidates instead of guessing. Local data stays out of shared caches.
    if !h.contains_key(header::CACHE_CONTROL) {
        let default = if gate.public { "no-cache" } else { "private, no-cache" };
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static(default));
    }
    let html = h.get(header::CONTENT_TYPE).is_some_and(|v| v.as_bytes().starts_with(b"text/html"));
    if html && !h.contains_key(header::CONTENT_SECURITY_POLICY) {
        h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    }
    res
}

#[cfg(test)]
mod tests {
    use super::{CSP, Gate, host_allowed};
    use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};

    /// A public server answers by method alone: a foreign Origin or cross-site fetch changes
    /// nothing for a read, and every write is refused before a body is read.
    #[test]
    fn public_mode_ignores_origin_and_refuses_every_write() {
        let g = gate(None, &["git.example.com"]);
        let mut h = HeaderMap::new();
        h.insert("host", HeaderValue::from_static("git.example.com"));
        h.insert("origin", HeaderValue::from_static("http://evil.test"));
        h.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert!(super::refusal(&Method::GET, &h, &g).is_none());
        for m in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
            let got = super::refusal(&m, &h, &g).map(|r| r.0);
            assert_eq!(got, Some(StatusCode::METHOD_NOT_ALLOWED), "{m}");
        }
    }

    /// Class: every `|safe` in a template is listed here with its count. A new one fails until
    /// its source is shown to come from an escaping builder and the entry is added.
    #[test]
    fn every_safe_template_site_is_inventoried() {
        const SITES: [(&str, usize); 19] = [
            ("_base.html", 1),
            ("blame.html", 1),
            ("blob.html", 8),
            ("commit.html", 4),
            ("commit_line.html", 1),
            ("compare.html", 1),
            ("dochead.html", 6),
            ("hotspots.html", 1),
            ("languages_body.html", 2),
            ("notebook.html", 2),
            ("overview.html", 3),
            ("pagehead.html", 2),
            ("refs.html", 1),
            ("search.html", 1),
            ("side.html", 1),
            ("stats.html", 2),
            ("toolbar.html", 1),
            ("tree.html", 1),
            ("worktree.html", 2),
        ];
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/templates");
        let mut found = Vec::new();
        for e in std::fs::read_dir(dir).expect("templates") {
            let path = e.expect("entry").path();
            let n = std::fs::read_to_string(&path).expect("read").matches("|safe").count();
            if n > 0 {
                found.push((path.file_name().expect("name").to_string_lossy().into_owned(), n));
            }
        }
        found.sort();
        let want: Vec<_> = SITES.iter().map(|(f, n)| ((*f).to_owned(), *n)).collect();
        assert_eq!(found, want, "a |safe changed: vet its source, then update SITES");
    }

    impl Gate {
        pub(crate) const fn timeout_for_test(&self) -> std::time::Duration {
            self.timeout
        }
    }

    /// Only an exported copy carries its fonts as `data:` URLs. The served app loads them
    /// from itself, so its policy never allows a `data:` font.
    #[test]
    fn served_policy_takes_no_data_fonts() {
        let fonts = CSP
            .split(';')
            .map(str::trim)
            .find(|d| d.starts_with("font-src"))
            .unwrap_or("default-src 'self'");
        assert!(!fonts.contains("data:"), "{CSP}");
        assert!(CSP.contains("default-src 'self';"), "{CSP}");
    }

    /// No page embeds a plugin, so the policy loads none.
    #[test]
    fn served_policy_loads_no_plugin_objects() {
        assert!(CSP.contains("object-src 'none'"), "{CSP}");
        assert!(!CSP.contains("object-src 'self'"), "{CSP}");
    }

    #[tokio::test]
    async fn every_response_carries_the_referrer_policy_and_html_the_csp() {
        let dir = tempfile::tempdir().expect("tmp");
        let f =
            gitterrace_testkit::Fixture::init(dir.path()).write("a.txt", "a\n").commit("feat: a");
        let router = crate::app_router(f.path()).expect("router");
        let base = gitterrace_testkit::base(&router).await;
        let head = |uri: String| {
            let router = router.clone();
            async move {
                let req = axum::http::Request::get(uri)
                    .header("host", "localhost")
                    .body(axum::body::Body::empty())
                    .expect("req");
                let res = tower::ServiceExt::oneshot(router, req).await.expect("res");
                (res.status().as_u16(), res.headers().clone())
            }
        };
        for uri in ["/".to_owned(), format!("{base}/raw/HEAD/a.txt"), "/nope".to_owned()] {
            let (_, h) = head(uri.clone()).await;
            let referrer = h.get("referrer-policy").map(|v| v.as_bytes().to_vec());
            assert_eq!(referrer.as_deref(), Some(&b"strict-origin-when-cross-origin"[..]), "{uri}");
        }
        let (status, h) = head(format!("{base}/tree/HEAD")).await;
        assert_eq!(status, 200);
        let csp = h.get("content-security-policy").expect("csp on a page");
        assert_eq!(csp, CSP, "the page policy, object-src 'none' included");
    }

    fn gate(bound: Option<&str>, hosts: &[&str]) -> Gate {
        Gate {
            bound: bound.map(|b| b.parse().expect("ip")),
            public: !hosts.is_empty(),
            hosts: hosts.iter().map(|h| (*h).to_owned()).collect(),
            timeout: std::time::Duration::from_secs(30),
        }
    }

    /// The reverse-proxy case: the configured name passes with or without a port and in any
    /// case; a sibling, a suffix trick or another name does not.
    #[test]
    fn proxy_hosts_are_exact() {
        let g = gate(None, &["git.example.com"]);
        for h in ["git.example.com", "GIT.example.com", "git.example.com:443", "localhost"] {
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
                assert!(CSP.contains(&format!("'sha256-{hash}'")), "unhashed: {body}");
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
        child.stdin.take().expect("stdin").write_all(s.as_bytes()).expect("write");
        let out = child.wait_with_output().expect("wait");
        String::from_utf8(out.stdout).expect("utf8").trim().to_owned()
    }
}
