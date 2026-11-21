use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// The letters `nav.js` binds as single page keys are exactly the page keys the actions
/// loader refuses, so an action key never also fires a page command.
#[test]
fn reserved_keys_match_nav_js() {
    let nav = include_str!("../../static/nav.js");
    let mut bound: Vec<char> = nav
        .split("k === \"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .filter_map(|k| {
            let mut c = k.chars();
            c.next().filter(|ch| ch.is_ascii_alphanumeric() && c.next().is_none())
        })
        .collect();
    bound.sort_unstable();
    bound.dedup();
    let mut reserved: Vec<char> = gitterrace_actions::RESERVED_KEYS
        .iter()
        .filter(|(_, who)| who.starts_with("the page's"))
        .map(|(k, _)| *k)
        .collect();
    reserved.sort_unstable();
    assert!(bound.len() >= 6, "nav.js key table not found: {bound:?}");
    assert_eq!(bound, reserved, "nav.js page keys and RESERVED_KEYS drifted");
}

/// A shell action that records its stdin and environment next to the config, then
/// prints a first line. Placeholder `{dir}` is the config folder.
pub(crate) const ECHO: &str = r#"version = 1
[[action]]
id = "echo"
label = "Echo"
kinds = ["line", "commit", "page", "file"]
shell = true
command = 'cat > "{dir}/stdin.json"; env > "{dir}/env.txt"; printf "%s\n" "$GITTERRACE_TITLE"'
timeout = "5s"

[[action]]
id = "slow"
label = "Slow"
kinds = ["page", "commit"]
shell = true
command = 'cat >/dev/null; sh -c "echo \$\$ > \"{dir}/grandchild.tmp\"; mv \"{dir}/grandchild.tmp\" \"{dir}/grandchild.pid\"; sleep 30" & while [ ! -e "{dir}/grandchild.pid" ]; do sleep 0.01; done; sleep 30'
timeout = "2s"

[[action]]
id = "nap1"
label = "Nap 1"
kinds = ["page"]
shell = true
command = 'cat >/dev/null; touch "{dir}/nap1.up"; while [ ! -e "{dir}/release" ]; do sleep 0.02; done'
timeout = "120s"
[[action]]
id = "nap2"
label = "Nap 2"
kinds = ["page"]
shell = true
command = 'cat >/dev/null; touch "{dir}/nap2.up"; while [ ! -e "{dir}/release" ]; do sleep 0.02; done'
timeout = "120s"
[[action]]
id = "nap3"
label = "Nap 3"
kinds = ["page"]
shell = true
command = 'cat >/dev/null; touch "{dir}/nap3.up"; while [ ! -e "{dir}/release" ]; do sleep 0.02; done'
timeout = "120s"
[[action]]
id = "nap4"
label = "Nap 4"
kinds = ["page"]
shell = true
command = 'cat >/dev/null; touch "{dir}/nap4.up"; while [ ! -e "{dir}/release" ]; do sleep 0.02; done'
timeout = "120s"
"#;

/// A private folder holding `actions.toml` built from `template`.
pub(crate) fn config_dir(template: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tmp");
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).expect("chmod");
    write_config(d.path(), template, 0o600);
    d
}

#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "`{rev}` and `{dir}` are our own placeholders, not format arguments"
)]
pub(crate) fn write_config(dir: &Path, template: &str, mode: u32) {
    let p = dir.join("actions.toml");
    let body = template.replace("{dir}", &dir.to_string_lossy());
    std::fs::write(&p, body).expect("write");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

/// The `]` and `[` keys move focus between file headers, which are focusable, and the `?`
/// sheet lists them.
#[test]
fn bracket_keys_focus_the_file_headers() {
    let nav = include_str!("../../static/nav.js");
    assert!(nav.contains("k === \"]\"") && nav.contains("k === \"[\""));
    assert!(nav.contains("section.file > header") && nav.contains("to.focus()"));
    assert!(nav.contains("Next / previous changed file"));
}
