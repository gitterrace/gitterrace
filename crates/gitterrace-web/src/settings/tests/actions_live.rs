//! Saving an action through the settings route reaches the next page's action menu.

use std::path::Path;

use crate::actions::actions_tests::write_config;
use crate::app::Env;

const ONE: &str = r#"version = 1
[[action]]
id = "first"
label = "First"
kinds = ["page"]
command = ["/usr/bin/true"]
"#;
const TWO: &str = r#"version = 1
[[action]]
id = "first"
label = "First"
kinds = ["page"]
command = ["/usr/bin/true"]
[[action]]
id = "second"
label = "Second"
kinds = ["page"]
command = ["/usr/bin/true"]
"#;

fn server(root: &Path, dir: &Path, bound: Option<std::net::IpAddr>) -> axum::Router {
    let mut env = Env::new(gitterrace_config::Config::default());
    env.settings_dir = Some(dir.to_path_buf());
    let file = dir.join("actions.toml");
    crate::build_server(
        vec![super::repo(root)],
        bound,
        Some(&file),
        &env,
        None,
        None,
    )
    .expect("router")
    .0
}

async fn page(r: &axum::Router) -> String {
    super::send(r, super::Req::new("GET", "/r/r/", None))
        .await
        .1
}

async fn save(r: &axum::Router, dir: &Path, text: &str) -> serde_json::Value {
    let file = dir.join("actions.toml");
    let stamp = gitterrace_config::Stamp::tag(gitterrace_config::Stamp::of(&file).ok().flatten());
    let token = super::token_of(r);
    let mut q = super::Req::new("PUT", "/_settings/file", Some(&token));
    q.body = serde_json::json!({ "f": "actions", "text": text, "stamp": stamp }).to_string();
    let (s, reply) = super::send(r, q).await;
    assert_eq!(s, 200, "{reply}");
    serde_json::from_str(&reply).expect("json")
}

fn dir() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().expect("t");
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).expect("chmod");
    d
}

#[tokio::test(flavor = "multi_thread")]
async fn an_action_saved_in_settings_reaches_the_next_page_menu() {
    let (root, d) = (tempfile::tempdir().expect("t"), dir());
    write_config(d.path(), ONE, 0o600);
    let app = server(root.path(), d.path(), None);
    let before = page(&app).await;
    assert!(
        before.contains("First") && !before.contains("Second"),
        "{before}"
    );
    save(&app, d.path(), TWO).await;
    gitterrace_testkit::wait::until("the menu lists the new action", || async {
        page(&app).await.contains("&#34;second&#34;")
    })
    .await;
}

/// A server started without actions mounts no run route and embeds no token, so the first
/// action needs a restart: the save says so where it was made, and the menu stays empty.
#[tokio::test(flavor = "multi_thread")]
async fn a_first_action_saved_in_settings_says_it_needs_a_restart() {
    let (root, d) = (tempfile::tempdir().expect("t"), dir());
    let app = server(root.path(), d.path(), None);
    assert!(!page(&app).await.contains("gitterrace-actions"));
    let reply = save(&app, d.path(), ONE).await;
    assert_eq!(reply["restart"], serde_json::json!(["actions"]), "{reply}");
    assert!(!page(&app).await.contains("gitterrace-actions"));
}

/// With actions on, a second action is live: the save asks for no restart.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_action_saved_in_settings_needs_no_restart() {
    let (root, d) = (tempfile::tempdir().expect("t"), dir());
    write_config(d.path(), ONE, 0o600);
    let app = server(root.path(), d.path(), None);
    let reply = save(&app, d.path(), TWO).await;
    assert_eq!(reply["restart"], serde_json::json!([]), "{reply}");
}

/// The security invariant: on a non-loopback bind the run route stays absent after a save
/// adds an action, whatever the save says.
#[tokio::test(flavor = "multi_thread")]
async fn the_run_route_stays_absent_off_loopback_after_a_save() {
    let (root, d) = (tempfile::tempdir().expect("t"), dir());
    write_config(d.path(), ONE, 0o600);
    let app = server(
        root.path(),
        d.path(),
        Some("192.168.1.5".parse().expect("ip")),
    );
    write_config(d.path(), TWO, 0o600);
    // Past two reload polls: a reload must not mount the route.
    gitterrace_testkit::wait::pause(std::time::Duration::from_secs(2)).await;
    let mut q = super::Req::new("POST", crate::actions::RUN_PATH, Some("x"));
    q.body = r#"{"action":"second","kind":"page","locator":{"repo":"r"}}"#.to_owned();
    assert_eq!(super::send(&app, q).await.0, 404);
}

/// Guard class: a file that does not exist yet read as a broken one. With no actions.toml (and
/// with an empty one) the actions tab opens on the empty form, not on a parse error, and an
/// empty save is refused by no rule. The second instance this catches: config.toml the same way.
#[tokio::test(flavor = "multi_thread")]
async fn a_missing_or_empty_settings_file_opens_the_empty_form() {
    for (f, name, text) in [
        ("actions", "actions.toml", None),
        ("actions", "actions.toml", Some("")),
        ("config", "config.toml", None),
        ("config", "config.toml", Some("")),
    ] {
        let (root, d) = (tempfile::tempdir().expect("t"), dir());
        if let Some(t) = text {
            std::fs::write(d.path().join(name), t).expect("w");
        }
        let app = server(root.path(), d.path(), None);
        let (s, html) = super::page(&app, &format!("/_settings?f={f}")).await;
        let case = format!(
            "{name} {}",
            if text.is_some() { "empty" } else { "missing" }
        );
        assert_eq!(s, 200, "{case}: {html}");
        assert!(
            !html.contains("id=\"formoff\""),
            "{case}: the form is off: {html}"
        );
        if f == "actions" {
            let shown = "version = 1\n";
            assert!(
                html.contains("version = 1"),
                "{case}: no start line in Plain"
            );
            let reply = save(&app, d.path(), shown).await;
            assert!(
                reply["errors"].as_array().is_none_or(Vec::is_empty),
                "{case}: {reply}"
            );
            let home = std::path::PathBuf::new();
            let text = std::fs::read_to_string(d.path().join(name)).expect("saved");
            gitterrace_actions::Config::parse(&text, &home).expect("the saved file loads");
        }
    }
}
