//! The owner proof over a real loopback listener: what a tokenless local client can learn from
//! the challenge never opens a write, and the launch flow of the owner still does.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// What the client of the test got back.
struct Answer {
    status: u16,
    head: String,
    body: String,
}

/// One request on a fresh connection. `extra` are the headers after `Host`.
async fn request(
    addr: std::net::SocketAddr,
    method: &str,
    uri: &str,
    extra: &[(&str, &str)],
    body: &str,
) -> Answer {
    let mut s = TcpStream::connect(addr).await.expect("connect");
    let lines: Vec<String> = extra.iter().map(|(k, v)| format!("{k}: {v}")).collect();
    let headers = lines.iter().fold(String::new(), |acc, l| acc + l + "\r\n");
    let text = format!(
        "{method} {uri} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n{headers}\
Content-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(text.as_bytes()).await.expect("write");
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.expect("read");
    let raw = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head.get(9..12).and_then(|c| c.parse().ok()).unwrap_or(0);
    Answer { status, head: head.to_owned(), body: body.to_owned() }
}

/// The value of a JSON string field, `""` when absent.
fn field(json: &str, key: &str) -> String {
    json.split(&format!("\"{key}\":\""))
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap_or("")
        .into()
}

fn config_with_everything() -> gitterrace_config::Config {
    let on = gitterrace_config::Feature::ALL.map(|f| [f.key(), " = true\n"].concat()).concat();
    gitterrace_config::parse(&format!("[features]\n{on}")).expect("parse").config
}

/// The headers a forging local client sets: every one a non-browser may choose freely.
fn forged(addr: std::net::SocketAddr, cookie: &str) -> Vec<(&'static str, String)> {
    vec![
        ("Sec-Fetch-Site", "same-origin".into()),
        ("Origin", format!("http://{addr}")),
        ("Content-Type", "application/json".into()),
        ("Cookie", cookie.into()),
    ]
}

/// Class: nothing a client without the secret file can fetch opens a write. The challenge
/// answers any nonce, so the answers to "session" and "cookie-name" (the values the old
/// derivation made of them) and to every other nonce are tried as cookie name and value, on
/// every write route, over a real listener. A second defect it catches: a write route that
/// forgets the owner check, since each of them is driven here without the cookie.
#[tokio::test(flavor = "multi_thread")]
async fn challenge_answers_open_no_write_and_the_launch_flow_still_does() {
    let tmp = tempfile::tempdir().expect("tmp");
    let root = tmp.path().canonicalize().expect("canon");
    let repo = gitterrace_testkit::Fixture::init(&root.join("r"))
        .write("a.txt", "a\n")
        .commit("feat: one")
        .path()
        .to_owned();
    let settings = root.join("settings");
    std::fs::create_dir_all(&settings).expect("dir");
    std::fs::write(settings.join("config.toml"), "").expect("w");
    let actions = tempfile::tempdir().expect("tmp");
    let body = "version = 1\n[[action]]\nid = \"echo\"\nlabel = \"Echo\"\n\
        kinds = [\"line\", \"commit\", \"page\", \"file\"]\nshell = true\n\
        command = \"echo hi\"\ntimeout = \"5s\"\n";
    std::fs::write(actions.path().join("actions.toml"), body).expect("write");
    let state = root.join("state");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let watched = crate::Watched {
        file: settings.join("config.toml"),
        load: Box::new(|| Ok(config_with_everything())),
    };
    let serving = crate::serve_on(
        listener,
        vec![repo],
        Some(actions.path().join("actions.toml")),
        config_with_everything(),
        Some(watched),
        Some(state.clone()),
        &[],
    );
    tokio::spawn(async move { drop(serving.await) });
    let port = addr.port();
    gitterrace_testkit::wait::until("the owner secret is published", || async {
        crate::add_token(&state, port).is_ok()
    })
    .await;

    // The attacker holds no file: only what the challenge route says.
    let mut answers = Vec::new();
    for nonce in ["session", "cookie-name", "challenge", "gt-owner-", "x", "0123456789abcdef"] {
        let got = request(addr, "GET", &format!("/_repos?nonce={nonce}"), &[], "").await;
        assert_eq!(got.status, 200, "the challenge answers {nonce:?}: {}", got.head);
        answers.push(field(&got.body, "proof"));
    }
    assert!(answers.iter().all(|a| a.len() == 64), "{answers:?}");
    let writes: [(&str, &str, &str); 6] = [
        ("PUT", "/_settings/file", r#"{"f":"actions","text":"","stamp":"none"}"#),
        ("POST", crate::actions::RUN_PATH, r#"{"action":"echo","kind":"page"}"#),
        ("POST", "/r/r/_checkpoint", r#"{"scope":"all"}"#),
        ("POST", "/r/r/_viewed", r#"{"path":"a.txt","patch":"p","viewed":true}"#),
        ("POST", "/r/r/_findings", r#"{"note":"x"}"#),
        ("DELETE", "/r/r/_findings", r#"{"id":"x"}"#),
    ];
    // Every pairing of an answer as cookie name part and as cookie value.
    for name in &answers {
        for value in &answers {
            let cookie = format!("gt-owner-{}={value}", &name[..8]);
            for (m, uri, body) in writes {
                let h = forged(addr, &cookie);
                let h: Vec<_> = h.iter().map(|(k, v)| (*k, v.as_str())).collect();
                let got = request(addr, m, uri, &h, body).await;
                assert_eq!(got.status, 403, "{m} {uri} with {cookie}: {}", got.body);
            }
        }
    }
    let after = std::fs::read_to_string(settings.join("config.toml")).expect("r");
    assert_eq!(after, "", "a forged request wrote the config");

    // The owner reads the secret file, asks for a ticket, redeems it and writes.
    let secret = crate::add_token(&state, port).expect("secret");
    let ticket =
        request(addr, "POST", "/_launch", &[(crate::add::TOKEN_HEADER, &secret)], "").await;
    assert_eq!(ticket.status, 200, "{}", ticket.body);
    let link = format!("/_launch?t={}&to=/", field(&ticket.body, "ticket"));
    let redeemed = request(addr, "GET", &link, &[], "").await;
    assert_eq!(redeemed.status, 303, "{}", redeemed.head);
    let cookie = redeemed
        .head
        .lines()
        .find_map(|l| l.strip_prefix("set-cookie: "))
        .and_then(|v| v.split(';').next())
        .expect("the owner cookie")
        .to_owned();
    let h = forged(addr, &cookie);
    let h: Vec<_> = h.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let ok = request(addr, "POST", "/r/r/_checkpoint", &h, r#"{"scope":"all"}"#).await;
    assert_eq!(ok.status, 200, "the owner's write: {}", ok.body);
    let again = request(addr, "GET", &link, &[], "").await;
    assert_eq!(again.status, 403, "a ticket works once");
}
